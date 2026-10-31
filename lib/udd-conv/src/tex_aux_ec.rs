//! Build-time packer for `tex_aux_ec.uddp`.
//!
//! Packs image-bearing auxiliary texture refs from tileart entries — those whose
//! `EcMaterialStableRole` is not `Base` or `SecondaryBase` and whose logical family a
//! runtime consumer reads ([`TEX_AUX_EC_PACKED_FAMILIES`]) — into a UDDP keyed
//! by virtual path hash. Each payload is raw pixel bytes (RGBA8888 or BC7) stored
//! at the virtual path `textures/{texture_id:08x}`.
//!
//! TileArtLegacy (and TileArtEnhanced) refs are skipped: they belong to the EC's legacy-art mode,
//! which is not converted. Classic art comes from the Classic Client files (`tex_art_cc`).
//!
//! Package layout:
//! - `metadata/slots.bin`: flat sorted array of `TexAuxEcSlotRecord`.
//! - `textures/{texture_id:08x}`: raw pixel bytes per `texture_id`.
//! - the client's `Data\SystemTextures\missing.tga` under `TEX_AUX_EC_MISSING_TEXTURE_ID`;
//! - `metadata/absent_sources.bin`: referenced ids the EC archive lacks, which the client
//!   replaces with `missing.tga`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bytemuck::cast_slice;
use color_eyre::eyre::{self, WrapErr};
use log::info;
use rayon::prelude::*;

use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::tex_aux_ec::{
    encode_absent_sources, texture_entry_path, TexAuxEcAbsentSourceRecord, TexAuxEcSlotRecord,
    TEX_AUX_EC_ABSENT_SOURCES_ENTRY_PATH, TEX_AUX_EC_MISSING_TEXTURE_ID, TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH,
};
use udd_assets::tilemeta::{
    EcMaterialLogicalFamily, EcMaterialPhysicalPackage, EcMaterialStableRole, TileMetaItemTextureRef, TileMetaPackage,
};
use udd_container::{
    AddFileRequest, AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder,
};
use uocf::enhanced::textures::Textures;

use crate::bc7::{
    encode_for_vram_with_bc7_rdo_and_stage_progress, preferred_bc7_encoder_backend,
    ImageExtent, RawImageFormat, VramTextureEncoding,
};
use crate::package_progress::build_and_write_package_with_progress;
use crate::source_paths::find_first_existing_file;
use crate::upscale_pipeline::{apply_upscale_passes_owned, UpscalePass};
use image_postprocess::upscaling::UpscaleFilter;

/// Logical families a runtime consumer reads from `tex_aux_ec` (dynapper's EC item liquid, static
/// terrain surface and sprite hue mask all take block-0 WorldArt refs). Refs of other families are
/// counted in [`TexAuxEcPackSummary::excluded_unread_family`] and not packed.
pub const TEX_AUX_EC_PACKED_FAMILIES: [EcMaterialLogicalFamily; 1] = [EcMaterialLogicalFamily::WorldArt];

/// Archive holding the client's system textures.
const SYSTEM_TEXTURES_UOP: &str = "SystemTextures.uop";
/// UOP entry of the image the client loads when a texture request returns no data
/// (`Data\SystemTextures\missing.tga`). The `.tga` extension selects the decoder, as in the client.
pub const EC_MISSING_TEXTURE_UOP_ENTRY: &str = "data/systemtextures/missing.tga";

/// Options controlling texture format and upscaling for `pack_tex_aux_ec`.
pub struct TexAuxEcPackOptions {
    /// Payload compression applied to each texture entry.
    pub compression: CompressionFlag,
    /// Pixel format for texture payloads.
    pub pixel_format: PagePixelFormat,
    /// BC7 RDO lambda (0.0 = no RDO). Only used when `pixel_format == Bc7`.
    pub bc7_rdo_lambda: f32,
    /// BC7 RDO lookback block count. Only used when `pixel_format == Bc7`.
    pub bc7_rdo_lookback_blocks: usize,
    /// Simple upscale filter applied before format encoding. `None` = no upscaling.
    pub upscale: UpscaleFilter,
    /// Explicit upscale pass chain. Takes precedence over `upscale` when non-empty.
    pub upscale_passes: Vec<UpscalePass>,
}

impl Default for TexAuxEcPackOptions {
    fn default() -> Self {
        Self {
            compression: CompressionFlag::ZstdNoDict,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale: UpscaleFilter::None,
            upscale_passes: Vec::new(),
        }
    }
}

/// Breakdown of how refs were classified during packing.
#[derive(Debug, Clone, Copy, Default)]
pub struct TexAuxEcPackSummary {
    /// Total `TileMetaItemTextureRef` rows considered.
    pub refs_considered: u64,
    /// Primary-selected refs: the image `tex_art_ec` packs for the owner.
    pub excluded_primary: u64,
    /// Non-base refs of a family no runtime consumer reads (TileArtLegacy, TileArtEnhanced); see
    /// [`TEX_AUX_EC_PACKED_FAMILIES`].
    pub excluded_unread_family: u64,
    /// Refs with no resolvable EC client path or unknown physical package.
    pub excluded_no_path: u64,
    /// Distinct source ids the EC archive does not contain. They are listed in
    /// `metadata/absent_sources.bin`; the client draws `missing.tga` for them.
    pub excluded_source_absent: u64,
    /// Distinct source ids whose file is present but could not be read or decoded to RGBA8888.
    pub excluded_decode_error: u64,
    /// Refs that are duplicate texture_ids already queued from a prior owner tile.
    pub excluded_duplicate_id: u64,
    /// Refs successfully packed as auxiliary image entries.
    pub included: u64,
}

/// Entry for deferred parallel decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QualifiedRef {
    texture_id: u32,
    logical_family: u8,
    stable_role: u8,
    physical_package: EcMaterialPhysicalPackage,
}

/// Pack auxiliary EC texture refs from `tilemeta_path` into `output`.
///
/// Source images are looked up from the EC source directories using the
/// `physical_package` field on each `TileMetaItemTextureRef` to select the
/// right UOP archive. Only refs with a decodable RGBA image and a non-base
/// stable role are included.
pub fn pack_tex_aux_ec(
    source_dirs: &[PathBuf],
    tilemeta_path: &Path,
    output: &Path,
    options: &TexAuxEcPackOptions,
) -> eyre::Result<TexAuxEcPackSummary> {
    info!(
        "Building tex_aux_ec.uddp from tilemeta '{}' -> '{}'",
        tilemeta_path.display(),
        output.display()
    );

    let tilemeta = TileMetaPackage::load(tilemeta_path)
        .wrap_err("load tilemeta for tex_aux_ec packing")?;

    // Only the archive of the packed families is needed: WorldArt refs resolve in Texture.uop.
    let texture_uop_path = find_first_existing_file(source_dirs, &["Texture.uop"]);
    let world_textures = texture_uop_path
        .as_ref()
        .map(|p| Textures::new(p, None, None).wrap_err_with(|| format!("load {}", p.display())))
        .transpose()?;
    let missing_texture = load_missing_texture(source_dirs)?;

    let mut summary = TexAuxEcPackSummary::default();
    let qualifying_refs = select_qualifying_refs(
        tilemeta.item_tiles().iter().flat_map(|item_tile| tilemeta.item_texture_refs(item_tile.tile_id)),
        &mut summary,
    )?;

    info!(
        "tex_aux_ec: {} refs considered, {} qualifying unique texture ids before decode",
        summary.refs_considered,
        qualifying_refs.len()
    );

    struct DecodedEntry {
        texture_id: u32,
        logical_family: u8,
        stable_role: u8,
        width: u32,
        height: u32,
        pixel_format: PagePixelFormat,
        data: Vec<u8>,
    }

    let upscale_passes: Vec<UpscalePass> = if !options.upscale_passes.is_empty() {
        options.upscale_passes.clone()
    } else if options.upscale != UpscaleFilter::None {
        vec![UpscalePass::from(options.upscale)]
    } else {
        Vec::new()
    };

    enum Decoded {
        Entry(DecodedEntry),
        SourceAbsent(QualifiedRef),
        DecodeError,
    }
    let encode = |texture_id: u32, logical_family: u8, stable_role: u8, image: RawRgba| {
        encode_image(image, &upscale_passes, options).map(|(width, height, pixel_format, data)| DecodedEntry {
            texture_id, logical_family, stable_role, width, height, pixel_format, data,
        })
    };

    // Decode (and optionally upscale + BC7-encode) images in parallel.
    let decoded: Vec<Decoded> = qualifying_refs
        .par_iter()
        .map(|q| {
            let source: Option<&Textures> = match q.physical_package {
                EcMaterialPhysicalPackage::Texture => world_textures.as_ref(),
                _ => None,
            };
            let file = match source.map(|source| source.get_from_id(q.texture_id)) {
                Some(Ok(Some(file))) => file,
                Some(Ok(None)) => return Decoded::SourceAbsent(*q),
                // No archive for this package, or the entry could not be read.
                _ => return Decoded::DecodeError,
            };
            decode_raw_rgba(&file).ok()
                .and_then(|image| encode(q.texture_id, q.logical_family, q.stable_role, image))
                .map_or(Decoded::DecodeError, Decoded::Entry)
        })
        .collect();

    let mut slot_records: Vec<TexAuxEcSlotRecord> = Vec::new();
    let mut image_entries: Vec<(u32, u32, u32, PagePixelFormat, Vec<u8>)> = Vec::new();
    let mut absent_sources: Vec<TexAuxEcAbsentSourceRecord> = Vec::new();

    let mut push_entry = |entry: DecodedEntry| {
        slot_records.push(TexAuxEcSlotRecord {
            texture_id: entry.texture_id,
            width: entry.width as u16,
            height: entry.height as u16,
            logical_family: entry.logical_family,
            stable_role: entry.stable_role,
            pixel_format: entry.pixel_format as u8,
            _pad: 0,
        });
        image_entries.push((entry.texture_id, entry.width, entry.height, entry.pixel_format, entry.data));
    };
    for decoded in decoded {
        match decoded {
            Decoded::Entry(entry) => {
                summary.included += 1;
                push_entry(entry);
            }
            Decoded::SourceAbsent(q) => {
                summary.excluded_source_absent += 1;
                absent_sources.push(TexAuxEcAbsentSourceRecord {
                    texture_id: q.texture_id,
                    logical_family: q.logical_family,
                    physical_package: q.physical_package as u8,
                    _pad: [0; 2],
                });
            }
            Decoded::DecodeError => summary.excluded_decode_error += 1,
        }
    }
    // The client's fallback image, stored under the reserved key; not counted in `included`.
    push_entry(
        encode(
            TEX_AUX_EC_MISSING_TEXTURE_ID,
            EcMaterialLogicalFamily::SystemTextures as u8,
            EcMaterialStableRole::Unknown as u8,
            missing_texture,
        )
        .ok_or_else(|| eyre::eyre!("encode {EC_MISSING_TEXTURE_UOP_ENTRY}"))?,
    );

    // Sort by texture_id for binary-search at runtime.
    slot_records.sort_by_key(|slot| slot.texture_id);
    image_entries.sort_by_key(|(tid, _, _, _, _)| *tid);

    info!(
        "tex_aux_ec: {} included plus the missing-texture fallback; excluded: primary {}, unread family {}, no path {}, \
         source absent {} (listed for the fallback), decode error {}, duplicates {}",
        summary.included,
        summary.excluded_primary,
        summary.excluded_unread_family,
        summary.excluded_no_path,
        summary.excluded_source_absent,
        summary.excluded_decode_error,
        summary.excluded_duplicate_id,
    );

    // Build UDDP.
    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    let slot_bytes = cast_slice::<TexAuxEcSlotRecord, u8>(&slot_records);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: slot_bytes,
    })?;

    let absent_bytes = encode_absent_sources(&absent_sources);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(TEX_AUX_EC_ABSENT_SOURCES_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &absent_bytes,
    })?;

    for (texture_id, width, height, _pixel_format, data) in image_entries {
        let vpath = texture_entry_path(texture_id);
        package.add_owned_file(AddOwnedFileRequest {
            data_type: DataType::Texture as u8,
            compression: options.compression,
            width,
            height,
            virtual_path: Some(vpath),
            path_hash64: None,
            id: None,
            data,
        })?;
    }

    build_and_write_package_with_progress(package, output, &mut |_| {})?;

    info!(
        "tex_aux_ec: wrote {} auxiliary texture slots to '{}'",
        summary.included,
        output.display()
    );

    Ok(summary)
}

/// Classifies tilemeta refs and returns the distinct ids to decode, in first-seen order.
///
/// Deduplication is by texture_id: the same texture may appear for multiple owner tile ids, but
/// one image is packed per texture_id.
fn select_qualifying_refs<'a>(
    refs: impl IntoIterator<Item = &'a TileMetaItemTextureRef>,
    summary: &mut TexAuxEcPackSummary,
) -> eyre::Result<Vec<QualifiedRef>> {
    let mut qualifying_refs: Vec<QualifiedRef> = Vec::new();
    let mut seen_ids: HashMap<u32, ()> = HashMap::new();
    for tex_ref in refs {
        summary.refs_considered += 1;
        let role = tex_ref.stable_role();
        if tex_ref.is_primary_selected() {
            summary.excluded_primary += 1;
            continue;
        }
        if !TEX_AUX_EC_PACKED_FAMILIES.contains(&tex_ref.logical_family()) {
            summary.excluded_unread_family += 1;
            continue;
        }
        let package = tex_ref.physical_package();
        let texture_id = tex_ref.texture_id;
        if package == EcMaterialPhysicalPackage::Unknown || texture_id == 0 {
            summary.excluded_no_path += 1;
            continue;
        }
        eyre::ensure!(
            texture_id != TEX_AUX_EC_MISSING_TEXTURE_ID,
            "texture id {texture_id} collides with the reserved missing-texture key"
        );
        if seen_ids.insert(texture_id, ()).is_some() {
            summary.excluded_duplicate_id += 1;
            continue;
        }
        qualifying_refs.push(QualifiedRef {
            texture_id,
            logical_family: tex_ref.texture_type,
            stable_role: tex_ref.stable_role,
            physical_package: package,
        });
    }
    Ok(qualifying_refs)
}

/// Decodes the client's `missing.tga` from `SystemTextures.uop`, by its `.tga` extension as the client does.
fn load_missing_texture(source_dirs: &[PathBuf]) -> eyre::Result<RawRgba> {
    let path = find_first_existing_file(source_dirs, &[SYSTEM_TEXTURES_UOP])
        .ok_or_else(|| eyre::eyre!("{SYSTEM_TEXTURES_UOP} not found in the EC source dirs; it holds the missing-texture fallback"))?;
    let textures = Textures::new(&path, None, None).wrap_err_with(|| format!("load {}", path.display()))?;
    textures
        .get_from_name(EC_MISSING_TEXTURE_UOP_ENTRY)?
        .ok_or_else(|| eyre::eyre!("{} has no {EC_MISSING_TEXTURE_UOP_ENTRY}", path.display()))
        .and_then(|file| decode_raw_rgba(&file))
        .wrap_err_with(|| format!("decode {EC_MISSING_TEXTURE_UOP_ENTRY}"))
}

/// A decoded RGBA8888 image: width, height and row-major pixel bytes.
struct RawRgba {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Decodes a source file to [`RawRgba`] with the decoder its format (extension) selects.
fn decode_raw_rgba(file: &uocf::enhanced::textures::TextureFile) -> eyre::Result<RawRgba> {
    let image = file.decode_to_rgba8()?;
    Ok(RawRgba { width: image.width(), height: image.height(), rgba: image.into_raw() })
}

/// Upscales and encodes one RGBA image to the package pixel format: `(width, height, format, bytes)`.
fn encode_image(
    image: RawRgba,
    upscale_passes: &[UpscalePass],
    options: &TexAuxEcPackOptions,
) -> Option<(u32, u32, PagePixelFormat, Vec<u8>)> {
    let RawRgba { width, height, rgba } = image;
    let (width, height, rgba) = if !upscale_passes.is_empty() {
        let (w, h, r, _, _) = apply_upscale_passes_owned(width, height, rgba, upscale_passes);
        (w, h, r)
    } else {
        (width, height, rgba)
    };
    if options.pixel_format != PagePixelFormat::Bc7 {
        return Some((width, height, PagePixelFormat::Rgba8888, rgba));
    }
    let extent = ImageExtent::new(width, height).ok()?;
    let vram = encode_for_vram_with_bc7_rdo_and_stage_progress(
        &rgba,
        extent,
        RawImageFormat::Rgba8888,
        VramTextureEncoding::Bc7(preferred_bc7_encoder_backend()),
        options.bc7_rdo_lambda,
        options.bc7_rdo_lookback_blocks,
        |_, _| {},
    )
    .ok()?;
    Some((width, height, PagePixelFormat::Bc7, vram.to_container_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASK_ID: u32 = 20_002_472;
    const LEGACY_ID: u32 = 4_039;
    const BASE_ID: u32 = 1_364;
    const SECONDARY_ID: u32 = 2_000_131;

    fn texture_ref(
        texture_id: u32,
        family: EcMaterialLogicalFamily,
        package: EcMaterialPhysicalPackage,
        role: EcMaterialStableRole,
    ) -> TileMetaItemTextureRef {
        TileMetaItemTextureRef {
            texture_id,
            texture_type: family as u8,
            physical_package: package as u8,
            stable_role: role as u8,
            ..TileMetaItemTextureRef::default()
        }
    }

    #[test]
    fn only_packed_families_qualify_and_skipped_refs_are_counted() -> eyre::Result<()> {
        use EcMaterialLogicalFamily::{TileArtEnhanced, TileArtLegacy, WorldArt};
        use EcMaterialPhysicalPackage::{LegacyTexture, Texture, Unknown};
        use EcMaterialStableRole::{Base, HueMask, SecondaryBase};
        let primary = TileMetaItemTextureRef {
            flags: udd_assets::tilemeta::TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ..texture_ref(BASE_ID, WorldArt, Texture, Base)
        };
        let refs = [
            texture_ref(MASK_ID, WorldArt, Texture, HueMask),
            texture_ref(MASK_ID, WorldArt, Texture, HueMask),
            primary,
            // Terrain RGB B is a colour image, yet only the primary image belongs to tex_art_ec.
            texture_ref(SECONDARY_ID, WorldArt, Texture, SecondaryBase),
            texture_ref(LEGACY_ID, TileArtLegacy, LegacyTexture, Base),
            texture_ref(LEGACY_ID, TileArtEnhanced, Unknown, Base),
            texture_ref(0, WorldArt, Texture, HueMask),
        ];
        let mut summary = TexAuxEcPackSummary::default();
        let selected = select_qualifying_refs(&refs, &mut summary)?;
        let qualified = |texture_id, role: EcMaterialStableRole| QualifiedRef {
            texture_id, logical_family: WorldArt as u8, stable_role: role as u8, physical_package: Texture,
        };
        assert_eq!(selected, [qualified(MASK_ID, HueMask), qualified(SECONDARY_ID, SecondaryBase)]);
        assert_eq!(summary.refs_considered, refs.len() as u64);
        assert_eq!(summary.excluded_duplicate_id, 1);
        assert_eq!(summary.excluded_primary, 1);
        assert_eq!(summary.excluded_unread_family, 2, "legacy and enhanced refs are counted, not packed");
        assert_eq!(summary.excluded_no_path, 1);
        Ok(())
    }

    #[test]
    fn a_ref_on_the_reserved_key_is_rejected() {
        let refs = [texture_ref(TEX_AUX_EC_MISSING_TEXTURE_ID, EcMaterialLogicalFamily::WorldArt,
            EcMaterialPhysicalPackage::Texture, EcMaterialStableRole::HueMask)];
        assert!(select_qualifying_refs(&refs, &mut TexAuxEcPackSummary::default()).is_err());
    }
}
