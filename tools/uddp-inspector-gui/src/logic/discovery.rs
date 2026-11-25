use std::collections::HashMap;
use color_eyre::eyre;
use udd_assets::tex_art_cc::{MISSING_PAGE_INDEX, PagePixelFormat, SLOT_MANIFEST_ENTRY_PATH};
use udd_assets::{TexArtCcPackage, TexArtEcPackage, TexLandCcPackage, TexLandEcPackage};
use udd_container::{DataType, UddpReader, xxh64_virtual_path};

use crate::models::{AtlasPageInfo, AtlasPixelFormat, PaperdollFrameInfo, TexAuxEcSlotInfo, VirtualEntry, VirtualEntryData};

const KIND_CC_ART: &str = "CC Art";
const KIND_EC_ART: &str = "EC Art";
const KIND_EC_LAND: &str = "EC Land";
const KIND_CC_TEXMAPS: &str = "CC Texmaps";
const SLOT_MANIFEST_MAGIC_LEN: usize = 4;
/// Upscale factor and algorithm shown for land slots, which store neither.
const NOT_UPSCALED_FACTOR: u16 = 1;
const NOT_UPSCALED_ALGORITHM: u16 = 0;

pub fn slot_manifest_kind(magic: &[u8; 4]) -> Option<&'static str> {
    match magic {
        b"CASL" => Some(KIND_CC_ART),
        b"EASL" => Some(KIND_EC_ART),
        b"ELSL" => Some(KIND_EC_LAND),
        b"CTXS" => Some(KIND_CC_TEXMAPS),
        _ => None,
    }
}

/// An atlas package (art or land, Classic or Enhanced) read with its typed udd-assets reader.
pub enum AtlasPackage {
    CcArt(TexArtCcPackage),
    EcArt(TexArtEcPackage),
    EcLand(TexLandEcPackage),
    CcTexmaps(TexLandCcPackage),
}

/// Open `reader` as the atlas package its slot manifest names.
///
/// `None` when the package has no slot manifest of an atlas package; an error when it is an
/// atlas package the reader rejects, such as one packed in another format version.
pub fn load_atlas_package(reader: &UddpReader) -> Option<eyre::Result<AtlasPackage>> {
    let slot_manifest = reader.read_file_by_path_hash(xxh64_virtual_path(SLOT_MANIFEST_ENTRY_PATH)).ok()?;
    let magic: [u8; SLOT_MANIFEST_MAGIC_LEN] = slot_manifest.get(..SLOT_MANIFEST_MAGIC_LEN)?.try_into().ok()?;
    let reader = reader.clone();
    Some(match slot_manifest_kind(&magic)? {
        KIND_CC_ART => TexArtCcPackage::from_uddp_package(reader).map(AtlasPackage::CcArt),
        KIND_EC_ART => TexArtEcPackage::from_uddp_package(reader).map(AtlasPackage::EcArt),
        KIND_EC_LAND => TexLandEcPackage::from_uddp_package(reader).map(AtlasPackage::EcLand),
        _ => TexLandCcPackage::from_uddp_package(reader).map(AtlasPackage::CcTexmaps),
    })
}

impl AtlasPackage {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::CcArt(_) => KIND_CC_ART,
            Self::EcArt(_) => KIND_EC_ART,
            Self::EcLand(_) => KIND_EC_LAND,
            Self::CcTexmaps(_) => KIND_CC_TEXMAPS,
        }
    }

    pub fn page_infos(&self) -> HashMap<u32, AtlasPageInfo> {
        macro_rules! pages {
            ($package:expr) => {{
                let (atlas_width, atlas_height) = ($package.atlas_width(), $package.atlas_height());
                $package
                    .pages()
                    .iter()
                    .map(|page| {
                        let info = AtlasPageInfo {
                            atlas_width,
                            atlas_height,
                            used_width: page.used_width,
                            used_height: page.used_height,
                            pixel_format: atlas_pixel_format(page.pixel_format),
                        };
                        (page.page_index, info)
                    })
                    .collect()
            }};
        }
        match self {
            Self::CcArt(package) => pages!(package),
            Self::EcArt(package) => pages!(package),
            Self::EcLand(package) => pages!(package),
            Self::CcTexmaps(package) => pages!(package),
        }
    }

    /// One entry per present slot.
    pub fn virtual_entries(&self) -> Vec<VirtualEntry> {
        let kind = self.kind();
        let slots: Vec<AtlasSlot> = match self {
            Self::CcArt(package) => package
                .slots()
                .iter()
                .filter(|slot| slot.is_present())
                .map(|slot| AtlasSlot {
                    id: slot.art_id,
                    page_index: slot.page_index,
                    page_tile_index: slot.page_tile_index,
                    flags: slot.flags,
                    x: slot.x,
                    y: slot.y,
                    width: slot.width,
                    height: slot.height,
                    upscale: Some((slot.upscale_factor.max(NOT_UPSCALED_FACTOR), slot.upscale_algorithm)),
                })
                .collect(),
            Self::EcArt(package) => package
                .slots()
                .iter()
                .filter(|slot| slot.is_present())
                .map(|slot| AtlasSlot {
                    id: slot.art_id,
                    page_index: slot.page_index,
                    page_tile_index: slot.page_tile_index,
                    flags: slot.flags,
                    x: slot.x,
                    y: slot.y,
                    width: slot.width,
                    height: slot.height,
                    upscale: Some((slot.upscale_factor.max(NOT_UPSCALED_FACTOR), slot.upscale_algorithm)),
                })
                .collect(),
            Self::EcLand(package) => package
                .slots()
                .iter()
                .filter(|slot| slot.is_present())
                .map(|slot| AtlasSlot {
                    id: slot.art_id,
                    page_index: slot.page_index,
                    page_tile_index: slot.page_tile_index,
                    flags: slot.flags,
                    x: slot.x,
                    y: slot.y,
                    width: slot.width,
                    height: slot.height,
                    upscale: None,
                })
                .collect(),
            Self::CcTexmaps(package) => package
                .slots()
                .iter()
                .filter(|slot| slot.is_present())
                .map(|slot| AtlasSlot {
                    id: slot.id,
                    page_index: slot.page_index,
                    page_tile_index: slot.page_tile_index,
                    flags: slot.flags,
                    x: slot.x,
                    y: slot.y,
                    width: slot.width,
                    height: slot.height,
                    upscale: None,
                })
                .collect(),
        };
        let data_type = match self {
            Self::CcArt(_) | Self::EcArt(_) => DataType::Art,
            Self::EcLand(_) | Self::CcTexmaps(_) => DataType::Texture,
        };
        slots.into_iter().map(|slot| atlas_virtual_entry(kind, data_type, slot)).collect()
    }
}

/// A present slot of any atlas package; `upscale` is `(factor, algorithm)` for art slots.
#[derive(Debug, Clone, Copy)]
struct AtlasSlot {
    id: u32,
    page_index: u32,
    page_tile_index: u16,
    flags: u16,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    upscale: Option<(u16, u16)>,
}

fn atlas_pixel_format(format: PagePixelFormat) -> AtlasPixelFormat {
    match format {
        PagePixelFormat::Rgba8888 => AtlasPixelFormat::Rgba8888,
        PagePixelFormat::Bc7 => AtlasPixelFormat::Bc7,
    }
}

fn atlas_virtual_entry(kind: &str, data_type: DataType, slot: AtlasSlot) -> VirtualEntry {
    let AtlasSlot { id, page_index, page_tile_index, flags, x, y, width, height, upscale } = slot;
    let (upscale_factor, upscale_algorithm) = upscale.unwrap_or((NOT_UPSCALED_FACTOR, NOT_UPSCALED_ALGORITHM));
    let is_empty = page_index == MISSING_PAGE_INDEX || width == 0 || height == 0;
    let summary = if is_empty {
        "empty".to_string()
    } else if upscale.is_some() {
        let logical_width = width as f32 / f32::from(upscale_factor);
        let logical_height = height as f32 / f32::from(upscale_factor);
        format!(
            "{}x{} at {},{}; logical {:.1}x{:.1}; upscale {} {}x",
            width,
            height,
            x,
            y,
            logical_width,
            logical_height,
            upscale_algorithm_name(upscale_algorithm),
            upscale_factor
        )
    } else {
        format!("{}x{} at {},{}", width, height, x, y)
    };
    let location = if page_index == MISSING_PAGE_INDEX {
        "missing page".to_string()
    } else {
        format!("page {}", page_index)
    };
    VirtualEntry {
        id,
        _data_type: data_type as u8,
        kind: kind.to_string(),
        summary,
        location,
        data: VirtualEntryData::AtlasRect {
            page_index,
            page_tile_idx: page_tile_index,
            x,
            y,
            width,
            height,
            flags,
            upscale_factor,
            upscale_algorithm,
        },
    }
}

/// Which pixels of a `paperdoll_ec.uddp` frame take a hue (`FRAME_HUE_*`).
pub fn paperdoll_hue_mode_name(hue_mode: u8) -> &'static str {
    use udd_assets::paperdoll_ec::{FRAME_HUE_ALL, FRAME_HUE_MASKED};
    match hue_mode {
        FRAME_HUE_MASKED => "partly hued",
        FRAME_HUE_ALL => "hued",
        _ => "not hued",
    }
}

/// The frames of a `paperdoll_ec.uddp`, male set first, each previewed unhued.
pub fn detect_paperdoll_frame_virtual_entries(package: &udd_assets::PaperdollEcPackage) -> Vec<VirtualEntry> {
    use udd_assets::paperdoll_ec::frame_entry_path;
    use uocf::enhanced::paperdoll::MALE_SET;
    package
        .frames()
        .iter()
        .map(|record| {
            let set = if usize::from(record.set) == MALE_SET { "male" } else { "female" };
            let hue = paperdoll_hue_mode_name(record.hue_mode);
            VirtualEntry {
                id: u32::from(record.frame_number),
                _data_type: DataType::Paperdoll as u8,
                kind: format!("Paperdoll Frame ({set})"),
                summary: format!("{}x{} rgba8888 | {hue}", record.width, record.height),
                location: frame_entry_path(record.set, record.frame_number),
                data: VirtualEntryData::PaperdollFrame(PaperdollFrameInfo {
                    record: *record,
                    path: frame_entry_path(record.set, record.frame_number),
                }),
            }
        })
        .collect()
}

/// The slots of a `tex_aux_ec.uddp`, by texture id: the tile textures, then the reserved keys (the atmosphere maps,
/// the light textures and the missing-texture image).
pub fn detect_tex_aux_ec_virtual_entries(package: &udd_assets::tex_aux_ec::TexAuxEcPackage) -> Vec<VirtualEntry> {
    use udd_assets::tex_aux_ec::{is_tex_aux_ec_reserved_key, texture_entry_path, SlotInfo, TEX_AUX_EC_MISSING_TEXTURE_ID};
    package
        .slots()
        .iter()
        .map(|record| {
            let slot = SlotInfo::from(record);
            let kind = match slot.texture_id {
                TEX_AUX_EC_MISSING_TEXTURE_ID => "Missing Texture",
                key if is_tex_aux_ec_reserved_key(key) => "Terrain Texture",
                _ => "Aux Texture",
            };
            let content = tex_aux_ec_slot_content(&slot);
            let alias = if slot.is_missing_source() { " | missing source: the missing-texture image" } else { "" };
            VirtualEntry {
                id: slot.texture_id,
                _data_type: DataType::Texture as u8,
                kind: kind.to_string(),
                summary: format!("{}x{} {} | {content}{alias}", slot.width, slot.height, tex_aux_ec_format_name(&slot)),
                location: texture_entry_path(slot.payload_texture_id),
                data: VirtualEntryData::TexAuxEc(TexAuxEcSlotInfo {
                    path: texture_entry_path(slot.payload_texture_id),
                    content,
                    slot,
                }),
            }
        })
        .collect()
}

/// What a `tex_aux_ec` slot holds: the missing-texture image, a `TerrainTexture.uop` entry (an atmosphere map or a
/// light texture) or a tile texture of its family and role.
fn tex_aux_ec_slot_content(slot: &udd_assets::tex_aux_ec::SlotInfo) -> String {
    use udd_assets::tex_aux_ec::{
        is_tex_aux_ec_reserved_key, EC_DAY_MAP_TERRAIN_TEXTURE_ENTRY, EC_NIGHT_MAP_TERRAIN_TEXTURE_ENTRY,
        EC_NIGHT_SIGHT_MAP_TERRAIN_TEXTURE_ENTRY, TEX_AUX_EC_MISSING_TEXTURE_ID, TEX_AUX_EC_TERRAIN_TEXTURE_KEY_BASE,
    };
    use udd_assets::tilemeta::{EcMaterialLogicalFamily, EcMaterialStableRole};
    let role = EcMaterialStableRole::from_u8(slot.stable_role).as_str();
    match slot.texture_id {
        TEX_AUX_EC_MISSING_TEXTURE_ID => "the client's missing-texture image".to_string(),
        key if is_tex_aux_ec_reserved_key(key) => {
            let entry = key & !TEX_AUX_EC_TERRAIN_TEXTURE_KEY_BASE;
            let name = match entry {
                EC_DAY_MAP_TERRAIN_TEXTURE_ENTRY => "day map".to_string(),
                EC_NIGHT_MAP_TERRAIN_TEXTURE_ENTRY => "night map".to_string(),
                EC_NIGHT_SIGHT_MAP_TERRAIN_TEXTURE_ENTRY => "night-sight map".to_string(),
                _ => format!("{role} texture"),
            };
            format!("{name}, TerrainTexture.uop entry {entry}")
        }
        _ => format!("{:?} {role}", EcMaterialLogicalFamily::from_u8(slot.logical_family)),
    }
}

/// The stored pixel format of a `tex_aux_ec` slot.
pub(crate) fn tex_aux_ec_format_name(slot: &udd_assets::tex_aux_ec::SlotInfo) -> &'static str {
    if slot.pixel_format == PagePixelFormat::Bc7 as u8 { "bc7" } else { "rgba8888" }
}

pub(crate) fn upscale_algorithm_name(code: u16) -> &'static str {
    match code {
        0 => "None",
        1 => "Nearest",
        2 => "Bilinear",
        3 => "CatmullRom",
        4 => "Lanczos3",
        5 => "SuperSai",
        6 => "FsrEasu",
        7 => "FsrEasuRcas",
        8 => "Depixelize",
        9 => "Nedi",
        10 => "TwoSai",
        11 => "SuperEagle",
        12 => "Lq",
        13 => "Hq",
        14 => "HqTrue",
        15 => "Epx",
        16 => "Xbr",
        17 => "Mmpx",
        18 => "SuperXbr",
        19 => "Cut1",
        20 => "Cut2",
        21 => "Cut3",
        22 => "ScaleFx",
        23 => "OmniScale",
        24 => "Jinc2",
        25 => "Jinc2Sharp",
        26 => "Jinc2Sharper",
        27 => "Jinc2Sharpest",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(width: u16, upscale: Option<(u16, u16)>) -> AtlasSlot {
        AtlasSlot {
            id: 42,
            page_index: 7,
            page_tile_index: 3,
            flags: 1,
            x: 11,
            y: 22,
            width,
            height: 44,
            upscale,
        }
    }

    #[test]
    fn slot_manifest_kind_recognizes_current_package_types() {
        assert_eq!(slot_manifest_kind(b"CASL"), Some(KIND_CC_ART));
        assert_eq!(slot_manifest_kind(b"EASL"), Some(KIND_EC_ART));
        assert_eq!(slot_manifest_kind(b"ELSL"), Some(KIND_EC_LAND));
        assert_eq!(slot_manifest_kind(b"CTXS"), Some(KIND_CC_TEXMAPS));
        assert_eq!(slot_manifest_kind(b"NOPE"), None);
    }

    #[test]
    fn atlas_entries_report_slot_rects() {
        let entry = atlas_virtual_entry(KIND_EC_LAND, DataType::Texture, slot(33, None));

        assert_eq!(entry.id, 42);
        assert_eq!(entry.kind, KIND_EC_LAND);
        assert_eq!(entry.summary, "33x44 at 11,22");
        assert_eq!(entry.location, "page 7");
        match &entry.data {
            VirtualEntryData::AtlasRect { page_index, x, y, width, height, upscale_factor, .. } => {
                assert_eq!((*page_index, *x, *y, *width, *height), (7, 11, 22, 33, 44));
                assert_eq!(*upscale_factor, NOT_UPSCALED_FACTOR);
            }
            other => panic!("expected atlas rect, got {other:?}"),
        }
    }

    #[test]
    fn atlas_entries_keep_present_slots_without_displayable_rects() {
        let entry = atlas_virtual_entry(KIND_EC_LAND, DataType::Texture, slot(0, None));

        assert_eq!(entry.summary, "empty");
    }

    #[test]
    fn atlas_entries_report_art_upscale_metadata() {
        let fsr_easu = 6;
        let entry = atlas_virtual_entry(KIND_EC_ART, DataType::Art, slot(33, Some((2, fsr_easu))));

        assert!(entry.summary.contains("logical 16.5x22.0"));
        assert!(entry.summary.contains("upscale FsrEasu 2x"));
    }

    /// A `paperdoll_ec.uddp` lists one entry per frame, which previews from its RGBA entry.
    #[test]
    fn paperdoll_frames_are_listed_with_their_entries() {
        use udd_assets::paperdoll_ec::{
            encode_dress_decisions, encode_frames_manifest, frame_entry_path, PaperdollEcDressDecisions,
            PaperdollEcFrameRecord, PaperdollEcSet, DRESS_DECISIONS_ENTRY_PATH, FRAMES_ENTRY_PATH, FRAME_HUE_MASKED,
        };
        use udd_container::{AddFileRequest, CompressionFlag, LookupMode, UddpBuilder};
        use uocf::enhanced::paperdoll::GENDER_SET_COUNT;
        const FEMALE_SET: u8 = 1;
        const FRAME_NUMBER: u16 = 401;
        const RGBA_CHANNELS: usize = 4;
        let record = PaperdollEcFrameRecord {
            set: FEMALE_SET,
            hue_mode: FRAME_HUE_MASKED,
            frame_number: FRAME_NUMBER,
            x: -10,
            y: -20,
            width: 2,
            height: 3,
        };
        let sets = vec![PaperdollEcSet { bounds: Default::default(), shadow: Default::default() }; GENDER_SET_COUNT];
        let manifest = encode_frames_manifest(&sets, &[record]).unwrap();
        let dress_decisions = encode_dress_decisions(&PaperdollEcDressDecisions::default()).unwrap();
        let rgba = vec![u8::MAX; record.pixel_count() * RGBA_CHANNELS];
        let frame_path = frame_entry_path(record.set, record.frame_number);
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (path, data, data_type) in [
            (FRAMES_ENTRY_PATH, manifest.as_slice(), DataType::Metadata),
            (DRESS_DECISIONS_ENTRY_PATH, dress_decisions.as_slice(), DataType::Metadata),
            (frame_path.as_str(), rgba.as_slice(), DataType::Paperdoll),
        ] {
            builder
                .add_file(AddFileRequest {
                    data_type: data_type as u8,
                    compression: CompressionFlag::None,
                    width: 0,
                    height: 0,
                    virtual_path: Some(path),
                    path_hash64: None,
                    id: None,
                    data,
                })
                .unwrap();
        }
        let package =
            udd_assets::PaperdollEcPackage::from_uddp_package(UddpReader::open(builder.build().unwrap()).unwrap())
                .unwrap();

        let entries = detect_paperdoll_frame_virtual_entries(&package);

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.id, u32::from(FRAME_NUMBER));
        assert_eq!(entry.kind, "Paperdoll Frame (female)");
        assert_eq!(entry.summary, "2x3 rgba8888 | partly hued");
        match &entry.data {
            VirtualEntryData::PaperdollFrame(info) => {
                assert_eq!(info.record, record);
                assert_eq!(package.read_frame_rgba(&info.record).unwrap(), rgba);
                assert_eq!(info.path, frame_path);
            }
            other => panic!("expected a paperdoll frame, got {other:?}"),
        }
    }

    /// A `tex_aux_ec` package of `slots` (sorted), each non-alias slot's payload `width` by `height` RGBA texels.
    fn tex_aux_ec_test_package(slots: &[udd_assets::tex_aux_ec::TexAuxEcSlotRecord]) -> udd_assets::tex_aux_ec::TexAuxEcPackage {
        use udd_assets::tex_aux_ec::*;
        use udd_container::{AddFileRequest, CompressionFlag, LookupMode, UddpBuilder};
        const RGBA_BYTES: usize = 4;
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        let manifest = encode_tex_aux_ec_slots(slots);
        let no_extents: &[u8] = &[];
        for (path, data) in [(TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH, manifest.as_slice()), (TEX_AUX_EC_WATER_EXTENTS_ENTRY_PATH, no_extents)] {
            builder.add_file(AddFileRequest {
                data_type: DataType::Metadata as u8, compression: CompressionFlag::None, width: 0, height: 0,
                virtual_path: Some(path), path_hash64: None, id: None, data,
            }).expect("add metadata");
        }
        for slot in slots.iter().filter(|slot| !slot.is_missing_source()) {
            let path = texture_entry_path(slot.texture_id);
            let data = vec![u8::MAX; usize::from(slot.width) * usize::from(slot.height) * RGBA_BYTES];
            builder.add_file(AddFileRequest {
                data_type: DataType::Texture as u8, compression: CompressionFlag::None,
                width: slot.width.into(), height: slot.height.into(), virtual_path: Some(&path),
                path_hash64: None, id: None, data: &data,
            }).expect("add payload");
        }
        TexAuxEcPackage::from_uddp_package(UddpReader::open(builder.build().expect("build")).expect("open"))
            .expect("load tex_aux_ec")
    }

    #[test]
    fn tex_aux_ec_entries_name_each_slot_and_an_alias_points_at_the_missing_texture() {
        use udd_assets::tex_aux_ec::*;
        use udd_assets::tilemeta::{EcMaterialLogicalFamily, EcMaterialStableRole};
        const TILE_TEXTURE: u32 = 20_002_472;
        const ABSENT_TEXTURE: u32 = 20_004_039;
        const LIGHT_ENTRY: u32 = 40;
        const SIDE: u16 = 2;
        const LIGHT_WIDTH: u16 = 3;
        const LIGHT_HEIGHT: u16 = 1;
        let record = |texture_id, width, height, family: EcMaterialLogicalFamily, role: EcMaterialStableRole| {
            TexAuxEcSlotRecord {
                texture_id, width, height, logical_family: family as u8, stable_role: role as u8, ..Default::default()
            }
        };
        let tile = record(TILE_TEXTURE, SIDE, SIDE, EcMaterialLogicalFamily::WorldArt, EcMaterialStableRole::Noise);
        let alias = TexAuxEcSlotRecord {
            texture_id: ABSENT_TEXTURE, flags: TEX_AUX_EC_SLOT_FLAG_MISSING_SOURCE, ..tile
        };
        let atmosphere = EC_ATMOSPHERE_MAP_TERRAIN_TEXTURE_ENTRIES.map(|entry| record(
            tex_aux_ec_terrain_texture_key(entry), SIDE, SIDE, EcMaterialLogicalFamily::Textures, EcMaterialStableRole::Unknown,
        ));
        let light = record(tex_aux_ec_terrain_texture_key(LIGHT_ENTRY), LIGHT_WIDTH, LIGHT_HEIGHT,
            EcMaterialLogicalFamily::Textures, EcMaterialStableRole::Light);
        let missing = record(TEX_AUX_EC_MISSING_TEXTURE_ID, SIDE, SIDE,
            EcMaterialLogicalFamily::SystemTextures, EcMaterialStableRole::Unknown);
        let slots = [&[tile, alias][..], &atmosphere, &[light, missing]].concat();
        let package = tex_aux_ec_test_package(&slots);

        let entries = detect_tex_aux_ec_virtual_entries(&package);
        let summary = |id: u32| {
            let entry = entries.iter().find(|entry| entry.id == id).expect("listed slot");
            (entry.kind.as_str(), entry.summary.clone(), entry.location.clone())
        };
        assert_eq!(entries.len(), slots.len());
        assert_eq!(summary(TILE_TEXTURE), ("Aux Texture", "2x2 rgba8888 | WorldArt noise".into(), texture_entry_path(TILE_TEXTURE)));
        let (_, alias_summary, alias_location) = summary(ABSENT_TEXTURE);
        assert!(alias_summary.ends_with("missing source: the missing-texture image"), "{alias_summary}");
        assert_eq!(alias_location, texture_entry_path(TEX_AUX_EC_MISSING_TEXTURE_ID));
        let night = tex_aux_ec_terrain_texture_key(EC_NIGHT_MAP_TERRAIN_TEXTURE_ENTRY);
        assert_eq!(summary(night).1, "2x2 rgba8888 | night map, TerrainTexture.uop entry 21");
        let light_key = tex_aux_ec_terrain_texture_key(LIGHT_ENTRY);
        assert_eq!(summary(light_key), ("Terrain Texture", "3x1 rgba8888 | light texture, TerrainTexture.uop entry 40".into(),
            texture_entry_path(light_key)));
        assert_eq!(summary(TEX_AUX_EC_MISSING_TEXTURE_ID).0, "Missing Texture");
    }
}
