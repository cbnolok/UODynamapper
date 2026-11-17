//! Applying `TilemetaOverrides.kdl` (`udd_assets::tilemeta_overrides`) to a packed `tilemeta.uddp`: its Enhanced item
//! flags and lights, with the Kingdom Reborn lights the package's own Kingdom Reborn section holds. The packers apply
//! the bundled file once they have nested that section ([`crate::kr_sections`]); `udd-cli tool
//! apply-tilemeta-overrides` applies a file to a packed package. A package the overrides were applied to before is
//! restored to the values its record keeps first, so the result depends only on the file applied.

use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use log::{info, warn};

use udd_assets::tilemeta::{
    encode_item_lights, TileMetaPackage, TILEMETA_ITEM_ENTRY_PATH, TILEMETA_ITEM_LIGHT_ENTRY_PATH,
    TILEMETA_OVERRIDES_ENTRY_PATH,
};
use udd_assets::tilemeta_overrides::TilemetaOverrides;
use udd_container::{
    unpack_codec, unpack_offset40, unpack_type, AddFileRequest, Codec, CompressionFlag, DataType, FileKey, LookupMode,
    UddpBuilder, UddpReader, xxh64_virtual_path,
};

use crate::conversion_tables::TILEMETA_OVERRIDES_KDL;

/// The suffix of a package being written, renamed over the package once complete.
const PARTIAL_SUFFIX: &str = ".partial";
/// How the overrides' record entry is stored when the package has none yet: as the tables beside it.
const RECORD_COMPRESSION: CompressionFlag = CompressionFlag::ZstdNoDict;

/// What [`apply_tilemeta_overrides`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TilemetaOverridesSummary {
    /// `TilemetaOverrides::digest` of the file applied.
    pub digest: u64,
    /// Items whose flags differ from the clients' tables.
    pub flag_items: usize,
    /// Items whose light differs from the clients' tables.
    pub light_items: usize,
    /// The operations that changed none of their items.
    pub warnings: Vec<String>,
}

impl std::fmt::Display for TilemetaOverridesSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "digest {:016x}, flags of {} items, lights of {} items, {} warnings", self.digest, self.flag_items,
            self.light_items, self.warnings.len())
    }
}

/// Applies the bundled `TilemetaOverrides.kdl` (`crate::conversion_tables::TILEMETA_OVERRIDES_KDL`) to `package` in
/// place; see [`apply_tilemeta_overrides`].
pub fn apply_bundled_tilemeta_overrides(package: &Path) -> eyre::Result<Option<TilemetaOverridesSummary>> {
    let overrides = uocf::utils::bundled_files::resolve(None, TILEMETA_OVERRIDES_KDL)?;
    apply_tilemeta_overrides(package, package, &overrides)
}

/// Applies the overrides file `overrides` to the `tilemeta.uddp` `input` and writes the result to `output` (which may
/// be `input`): its item flags and lights replaced, its record (`TILEMETA_OVERRIDES_ENTRY_PATH`) added or replaced,
/// every other entry copied unchanged, the Kingdom Reborn section still uncompressed. `None`, with a warning and
/// nothing written, when `input` has no Kingdom Reborn section: `kr-light` needs it, and a package without it is not
/// one the Enhanced styles' Kingdom Reborn data comes from.
pub fn apply_tilemeta_overrides(input: &Path, output: &Path, overrides: &Path)
    -> eyre::Result<Option<TilemetaOverridesSummary>> {
    let text = fs::read_to_string(overrides).wrap_err_with(|| format!("read {}", overrides.display()))?;
    let overrides = TilemetaOverrides::parse(&overrides.display().to_string(), &text)?;

    let package_bytes = fs::read(input).wrap_err_with(|| format!("read {}", input.display()))?;
    let tilemeta = UddpReader::open(package_bytes.clone())
        .map_err(eyre::Report::from)
        .and_then(TileMetaPackage::from_uddp_package)
        .wrap_err_with(|| format!("read {}", input.display()))?;
    let Some(kr) = tilemeta.kr_tilemeta()? else {
        warn!("{} holds no Kingdom Reborn tile data: the tilemeta overrides are not applied; repack it with its \
            Kingdom Reborn section", input.display());
        return Ok(None);
    };
    let mut items = tilemeta.item_tiles().to_vec();
    let mut lights = tilemeta.item_lights().to_vec();
    if let Some(previous) = tilemeta.overrides_record()? {
        previous.restore(&mut items, &mut lights)?;
    }
    let applied = overrides.apply(&mut items, &mut lights, kr.item_lights())?;
    for warning in &applied.warnings {
        warn!("{warning}");
    }
    drop(kr);
    drop(tilemeta);

    let replacements = [
        (TILEMETA_ITEM_ENTRY_PATH, bytemuck::cast_slice(&items).to_vec()),
        (TILEMETA_ITEM_LIGHT_ENTRY_PATH, encode_item_lights(&lights)),
        (TILEMETA_OVERRIDES_ENTRY_PATH, applied.record.to_bytes()),
    ];
    let reader = UddpReader::open(package_bytes).wrap_err_with(|| format!("read {}", input.display()))?;
    let bytes = rewrite_package(&reader, &replacements).wrap_err_with(|| format!("rewrite {}", input.display()))?;
    drop(reader);
    write_package(output, &bytes)?;

    let summary = TilemetaOverridesSummary {
        digest: applied.record.digest,
        flag_items: applied.record.original_flags.len(),
        light_items: applied.record.original_lights.len(),
        warnings: applied.warnings,
    };
    info!("Applied the tilemeta overrides {} to {}: {summary}", overrides.source_name, output.display());
    Ok(Some(summary))
}

/// `base` with the entries of `replacements` (virtual path, bytes) replaced, or added when `base` has none, and every
/// other entry re-added with the codec it is stored with.
fn rewrite_package(base: &UddpReader, replacements: &[(&str, Vec<u8>)]) -> eyre::Result<Vec<u8>> {
    eyre::ensure!(base.lookup_mode() == LookupMode::VirtualPathHash, "not a package of virtual paths");
    let mut builder = UddpBuilder::new(base.lookup_mode());
    let replacement = |hash: u64| replacements.iter().find(|(path, _)| xxh64_virtual_path(path) == hash);
    let mut records = base.records();
    records.sort_by_key(|record| unpack_offset40(record.locator.pos64));
    let mut stored = Vec::with_capacity(records.len());
    for record in records {
        let FileKey::PathHash(hash) = record.key else {
            eyre::bail!("an entry keyed by id in a package of virtual paths");
        };
        let compression = codec_compression(unpack_codec(record.locator.meta32));
        let data = match replacement(hash) {
            Some((_, bytes)) => bytes.clone(),
            None => base.read_file_by_path_hash(hash)?,
        };
        stored.push((hash, unpack_type(record.locator.meta32), compression, data));
    }
    for (path, bytes) in replacements {
        let hash = xxh64_virtual_path(path);
        if !stored.iter().any(|(stored_hash, ..)| *stored_hash == hash) {
            stored.push((hash, DataType::Metadata as u8, RECORD_COMPRESSION, bytes.clone()));
        }
    }
    for (hash, data_type, compression, data) in &stored {
        builder.add_file(AddFileRequest {
            data_type: *data_type,
            compression: *compression,
            width: 0,
            height: 0,
            virtual_path: None,
            path_hash64: Some(*hash),
            id: None,
            data,
        })?;
    }
    Ok(builder.build()?)
}

/// The builder's choice that stores an entry with `codec` again.
fn codec_compression(codec: Codec) -> CompressionFlag {
    match codec {
        Codec::None => CompressionFlag::None,
        Codec::ZstdNoDict => CompressionFlag::ZstdNoDict,
        Codec::ZstdTypeDict => CompressionFlag::ZstdDict,
    }
}

fn write_package(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    let mut partial = path.as_os_str().to_owned();
    partial.push(PARTIAL_SUFFIX);
    let partial = PathBuf::from(partial);
    fs::write(&partial, bytes).wrap_err_with(|| format!("write {}", partial.display()))?;
    fs::rename(&partial, path).wrap_err_with(|| format!("replace {}", path.display()))
}
