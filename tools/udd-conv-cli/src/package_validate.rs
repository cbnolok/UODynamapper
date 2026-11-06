//! `udd-cli tool validate`: checks that packages open with the readers in this build.
//!
//! For every package it checks:
//! - the container: header, format version, index, dictionary table and package hash;
//! - every entry: it decodes through its own logical key, with its codec and dictionary,
//!   to exactly the declared raw size;
//! - the package's own reader: the `udd-assets` reader for its kind parses its manifests,
//!   which also checks the manifest format versions.
//!
//! The kind comes from the file name (`tex_land_ec.uddp`, `map0.uddp`, `terrain_meta.bin`).
//! A file with an unknown name is offered to every reader, and the first one that accepts it
//! names the kind.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use log::info;
use rayon::prelude::*;
use udd_assets::map_metadata::decode_map_package_metadata;
use udd_assets::terrain_meta::TerrainMetaPackage;
use udd_assets::tex_aux_ec::TexAuxEcPackage;
use udd_assets::{
    EffectsEcPackage, GumpsPackage, HuesPackage, MobileAnimCcPackage, MobileAnimEcPackage,
    TexArtCcPackage, TexArtEcPackage, TexLandCcPackage, TexLandEcPackage, TileMetaPackage,
    WorldLightsPackage,
};
use udd_container::{
    reconstruct_stored_size, unpack_codec, unpack_offset40, unpack_type, Codec, DataType, FileKey,
    FileRecord, LookupMode, UddpReader,
};
use uocf::classic::statics::StaticTile;

/// File extension of UDDP packages.
const UDDP_EXTENSION: &str = "uddp";
/// The terrain-layer sidecar, a flat binary file read by its own reader.
const TERRAIN_META_FILE_NAME: &str = "terrain_meta.bin";
/// Side of one packed map chunk, in tiles.
const MAP_CHUNK_TILE_DIM: usize = 32;
/// One packed map texel: tile id (u16) plus height and flags (u16).
const MAP_PACKED_TEXEL_BYTES: usize = 4;
/// Decoded size of every map chunk entry.
const MAP_CHUNK_BYTES: usize = MAP_CHUNK_TILE_DIM * MAP_CHUNK_TILE_DIM * MAP_PACKED_TEXEL_BYTES;
/// Entry failures listed per package; the rest are only counted.
const MAX_LISTED_ENTRY_FAILURES: usize = 16;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

/// Package kinds, each read by its own reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageKind {
    TexArtCc,
    TexArtEc,
    TexLandCc,
    TexLandEc,
    TexAuxEc,
    TileMeta,
    Hues,
    WorldLights,
    MobileAnimCc,
    MobileAnimEc,
    EffectsEc,
    Gumps,
    Map(u32),
    Statics(u32),
    TerrainMeta,
}

/// UDDP kinds offered, in order, to a package whose file name names no kind.
const SNIFFED_KINDS: [PackageKind; 12] = [
    PackageKind::TileMeta,
    PackageKind::Hues,
    PackageKind::WorldLights,
    PackageKind::TexArtCc,
    PackageKind::TexArtEc,
    PackageKind::TexLandCc,
    PackageKind::TexLandEc,
    PackageKind::TexAuxEc,
    PackageKind::MobileAnimCc,
    PackageKind::MobileAnimEc,
    PackageKind::EffectsEc,
    PackageKind::Gumps,
];

impl PackageKind {
    /// The kind named by a file name; a suffix after the first dot is ignored
    /// (`tex_land_ec.rebuilt.uddp` is a `tex_land_ec` package).
    pub fn from_file_name(file_name: &str) -> Option<Self> {
        if file_name == TERRAIN_META_FILE_NAME {
            return Some(Self::TerrainMeta);
        }
        let base = file_name.split('.').next().unwrap_or(file_name);
        let kind = match base {
            "tex_art_cc" => Self::TexArtCc,
            "tex_art_ec" => Self::TexArtEc,
            "tex_land_cc" => Self::TexLandCc,
            "tex_land_ec" => Self::TexLandEc,
            "tex_aux_ec" => Self::TexAuxEc,
            "tilemeta" => Self::TileMeta,
            "hues" => Self::Hues,
            "world_lights" => Self::WorldLights,
            "mobile_anim_cc" => Self::MobileAnimCc,
            "mobile_anim_ec" => Self::MobileAnimEc,
            _ if base == "effects_ec" || base.starts_with("effects_ec_") => Self::EffectsEc,
            _ if base == "gumps" || base.starts_with("gumps_") => Self::Gumps,
            _ => {
                if let Some(map_id) = numbered_suffix(base, "map") {
                    Self::Map(map_id)
                } else if let Some(map_id) = numbered_suffix(base, "statics") {
                    Self::Statics(map_id)
                } else {
                    return None;
                }
            }
        };
        Some(kind)
    }

    pub fn name(self) -> String {
        match self {
            Self::TexArtCc => "tex_art_cc".into(),
            Self::TexArtEc => "tex_art_ec".into(),
            Self::TexLandCc => "tex_land_cc".into(),
            Self::TexLandEc => "tex_land_ec".into(),
            Self::TexAuxEc => "tex_aux_ec".into(),
            Self::TileMeta => "tilemeta".into(),
            Self::Hues => "hues".into(),
            Self::WorldLights => "world_lights".into(),
            Self::MobileAnimCc => "mobile_anim_cc".into(),
            Self::MobileAnimEc => "mobile_anim_ec".into(),
            Self::EffectsEc => "effects_ec".into(),
            Self::Gumps => "gumps".into(),
            Self::Map(map_id) => format!("map{map_id}"),
            Self::Statics(map_id) => format!("statics{map_id}"),
            Self::TerrainMeta => "terrain_meta".into(),
        }
    }
}

fn numbered_suffix(base: &str, prefix: &str) -> Option<u32> {
    let digits = base.strip_prefix(prefix)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// What `validate` found in one package.
#[derive(Debug, Default)]
pub struct PackageReport {
    pub path: PathBuf,
    pub kind: Option<PackageKind>,
    /// The kind was found by offering the package to every reader.
    pub kind_sniffed: bool,
    pub entry_count: usize,
    pub raw_bytes: u64,
    pub stored_bytes: u64,
    pub codec_counts: BTreeMap<&'static str, usize>,
    pub dictionary_count: usize,
    pub reader_summary: Option<String>,
    pub failures: Vec<String>,
}

impl PackageReport {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    fn kind_name(&self) -> String {
        match self.kind {
            Some(kind) if self.kind_sniffed => format!("{} (by content)", kind.name()),
            Some(kind) => kind.name(),
            None => "unknown".into(),
        }
    }
}

fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::None => "None",
        Codec::ZstdNoDict => "ZstdNoDict",
        Codec::ZstdTypeDict => "ZstdTypeDict",
    }
}

/// Validate each package (directories are searched recursively), log a per-package summary
/// and fail when any package fails.
pub fn run_validate(inputs: &[PathBuf]) -> eyre::Result<()> {
    let paths = collect_package_paths(inputs)?;
    eyre::ensure!(!paths.is_empty(), "no packages found (looked for *.{UDDP_EXTENSION} and {TERRAIN_META_FILE_NAME})");

    let mut failed = 0usize;
    for path in &paths {
        let report = validate_package(path);
        log_report(&report);
        if !report.passed() {
            failed += 1;
        }
    }

    info!("Validated {} packages: {} passed, {} failed", paths.len(), paths.len() - failed, failed);
    eyre::ensure!(failed == 0, "{failed} of {} packages failed validation", paths.len());
    Ok(())
}

fn log_report(report: &PackageReport) {
    let status = if report.passed() { "OK  " } else { "FAIL" };
    info!("{status} {} [{}]", report.path.display(), report.kind_name());
    if report.entry_count > 0 {
        let codecs = report
            .codec_counts
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(", ");
        info!(
            "     entries={}, raw={:.1} MiB, stored={:.1} MiB, codecs: {}, dictionaries={}",
            report.entry_count,
            report.raw_bytes as f64 / BYTES_PER_MIB,
            report.stored_bytes as f64 / BYTES_PER_MIB,
            codecs,
            report.dictionary_count
        );
    }
    if let Some(summary) = &report.reader_summary {
        info!("     reader: {summary}");
    }
    for failure in &report.failures {
        info!("     error: {failure}");
    }
}

/// Packages named by the inputs: files as given, directories searched recursively for
/// `*.uddp` and `terrain_meta.bin`, in path order.
pub fn collect_package_paths(inputs: &[PathBuf]) -> eyre::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for input in inputs {
        if input.is_dir() {
            collect_directory_packages(input, &mut paths)?;
        } else {
            eyre::ensure!(input.is_file(), "{} does not exist", input.display());
            paths.push(input.clone());
        }
    }
    Ok(paths)
}

fn collect_directory_packages(directory: &Path, paths: &mut Vec<PathBuf>) -> eyre::Result<()> {
    let mut entries = fs::read_dir(directory)
        .wrap_err_with(|| format!("read {}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .wrap_err_with(|| format!("read {}", directory.display()))?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_directory_packages(&path, paths)?;
        } else if is_package_file(&path) {
            paths.push(path);
        }
    }
    Ok(())
}

fn is_package_file(path: &Path) -> bool {
    let is_uddp = path.extension().and_then(|ext| ext.to_str()) == Some(UDDP_EXTENSION);
    let is_terrain_meta = path.file_name().and_then(|name| name.to_str()) == Some(TERRAIN_META_FILE_NAME);
    is_uddp || is_terrain_meta
}

/// Validate one package; every problem is recorded in the report, never returned.
pub fn validate_package(path: &Path) -> PackageReport {
    let mut report = PackageReport {
        path: path.to_path_buf(),
        kind: path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(PackageKind::from_file_name),
        ..PackageReport::default()
    };

    if report.kind == Some(PackageKind::TerrainMeta) {
        match TerrainMetaPackage::load(path) {
            Ok(_) => report.reader_summary = Some("terrain_meta ok".into()),
            Err(error) => report.failures.push(format!("terrain_meta reader: {error:#}")),
        }
        return report;
    }

    let package = match UddpReader::load(path) {
        Ok(package) => package,
        Err(error) => {
            report.failures.push(format!("container: {error}"));
            return report;
        }
    };

    check_container(&package, &mut report);

    if report.kind.is_none() {
        report.kind = SNIFFED_KINDS
            .into_iter()
            .find(|kind| run_package_reader(*kind, &package).is_ok());
        report.kind_sniffed = report.kind.is_some();
    }
    match report.kind {
        Some(kind) => match run_package_reader(kind, &package) {
            Ok(summary) => report.reader_summary = Some(summary),
            Err(error) => report.failures.push(format!("{} reader: {error:#}", kind.name())),
        },
        None => report
            .failures
            .push("no reader in this build accepts the package, and its file name names no kind".into()),
    }
    report
}

/// Container checks: package hash, dictionary table, and every entry decoded through its key.
fn check_container(package: &UddpReader, report: &mut PackageReport) {
    let stored_hash = package.stored_package_hash64();
    let computed_hash = package.computed_package_hash64();
    if stored_hash != computed_hash {
        report.failures.push(format!(
            "package hash: stored 0x{stored_hash:016x}, computed 0x{computed_hash:016x}"
        ));
    }

    let dictionaries = package.dictionary_records();
    report.dictionary_count = dictionaries.len();
    for (data_type, codec, size) in &dictionaries {
        // The dictionary table names the codec a dictionary serves; only ZstdTypeDict uses one.
        if *codec != Codec::ZstdTypeDict {
            report.failures.push(format!(
                "dictionary for type {data_type} is tagged {}; only ZstdTypeDict entries use dictionaries",
                codec_name(*codec)
            ));
        }
        match package.dictionary_for_type(*data_type) {
            Some(bytes) if bytes.len() == *size as usize => {}
            _ => report
                .failures
                .push(format!("dictionary for type {data_type} ({size} bytes) lies outside the package")),
        }
    }

    let records = package.records();
    report.entry_count = records.len();
    for record in &records {
        *report
            .codec_counts
            .entry(codec_name(unpack_codec(record.locator.meta32)))
            .or_default() += 1;
    }

    let blob_offset = package.header().blob_offset;
    let lookup_mode = package.lookup_mode();
    let outcomes = records
        .par_iter()
        .map(|record| check_entry(package, lookup_mode, blob_offset, record))
        .collect::<Vec<_>>();

    let mut entry_failures = 0usize;
    for outcome in outcomes {
        match outcome {
            Ok((raw, stored)) => {
                report.raw_bytes += raw;
                report.stored_bytes += stored;
            }
            Err(failure) => {
                entry_failures += 1;
                if entry_failures <= MAX_LISTED_ENTRY_FAILURES {
                    report.failures.push(failure);
                }
            }
        }
    }
    if entry_failures > MAX_LISTED_ENTRY_FAILURES {
        report.failures.push(format!(
            "{} more entries failed to decode",
            entry_failures - MAX_LISTED_ENTRY_FAILURES
        ));
    }
}

fn entry_label(key: FileKey) -> String {
    match key {
        FileKey::PathHash(hash) => format!("entry 0x{hash:016x}"),
        FileKey::Id(id) => format!("entry {id}"),
    }
}

/// Decode one entry through its own key; returns its raw and stored sizes.
fn check_entry(
    package: &UddpReader,
    lookup_mode: LookupMode,
    blob_offset: u64,
    record: &FileRecord,
) -> Result<(u64, u64), String> {
    let locator = &record.locator;
    let label = entry_label(record.key);
    let codec = unpack_codec(locator.meta32);
    let stored = reconstruct_stored_size(locator.raw_size, locator.meta32, locator.pos64);
    let offset = unpack_offset40(locator.pos64);
    if offset < blob_offset {
        return Err(format!("{label}: payload offset {offset} lies before the blob region at {blob_offset}"));
    }
    if codec == Codec::ZstdTypeDict && package.dictionary_for_type(unpack_type(locator.meta32)).is_none() {
        return Err(format!(
            "{label}: {} needs the type {} dictionary, which the package lacks",
            codec_name(codec),
            unpack_type(locator.meta32)
        ));
    }

    let decoded: Result<Cow<'_, [u8]>, _> = match (record.key, lookup_mode) {
        (FileKey::PathHash(hash), _) => package.read_file_by_path_hash_cow(hash),
        (FileKey::Id(id), LookupMode::SparseId) => package.read_file_by_sparse_id_cow(id),
        (FileKey::Id(id), _) => package.read_file_by_dense_id_cow(id),
    };
    let decoded = decoded.map_err(|error| format!("{label} ({}): {error}", codec_name(codec)))?;
    if decoded.len() != locator.raw_size as usize {
        return Err(format!(
            "{label} ({}): decoded {} bytes, the index declares {}",
            codec_name(codec),
            decoded.len(),
            locator.raw_size
        ));
    }
    Ok((u64::from(locator.raw_size), u64::from(stored)))
}

/// Parse the package with the reader for its kind; returns a one-line summary.
fn run_package_reader(kind: PackageKind, package: &UddpReader) -> eyre::Result<String> {
    let summary = match kind {
        PackageKind::TexArtCc => {
            let reader = TexArtCcPackage::from_uddp_package(package.clone())?;
            format!("slots={}, pages={}", reader.slots().len(), reader.pages().len())
        }
        PackageKind::TexArtEc => {
            let reader = TexArtEcPackage::from_uddp_package(package.clone())?;
            format!("slots={}, pages={}", reader.slots().len(), reader.pages().len())
        }
        PackageKind::TexLandCc => {
            let reader = TexLandCcPackage::from_uddp_package(package.clone())?;
            format!("slots={}, pages={}", reader.slots().len(), reader.pages().len())
        }
        PackageKind::TexLandEc => {
            let reader = TexLandEcPackage::from_uddp_package(package.clone())?;
            format!(
                "slots={}, pages={}, terrain_provenance={}",
                reader.slots().len(),
                reader.pages().len(),
                reader.terrain_provenance().len()
            )
        }
        PackageKind::TexAuxEc => {
            let reader = TexAuxEcPackage::from_uddp_package(package.clone())?;
            format!("slots={}", reader.slots().len())
        }
        PackageKind::TileMeta => {
            let reader = TileMetaPackage::from_uddp_package(package.clone())?;
            format!(
                "land_tiles={}, item_tiles={}",
                reader.land_tiles().len(),
                reader.item_tiles().len()
            )
        }
        PackageKind::Hues => {
            let reader = HuesPackage::from_uddp_package(package.clone())?;
            format!("slots={}", reader.slots().len())
        }
        PackageKind::WorldLights => {
            let reader = WorldLightsPackage::from_uddp_package(package.clone())?;
            format!("slots={}", reader.slots().len())
        }
        PackageKind::MobileAnimCc => {
            let reader = MobileAnimCcPackage::from_uddp_package(package.clone())?;
            format!(
                "pages={}, animations={}, frames={}",
                reader.pages().len(),
                reader.animations().len(),
                reader.frames().len()
            )
        }
        PackageKind::MobileAnimEc => {
            let reader = MobileAnimEcPackage::from_uddp_package(package.clone())?;
            format!(
                "pages={}, animations={}, frames={}",
                reader.pages().len(),
                reader.animations().len(),
                reader.frames().len()
            )
        }
        PackageKind::EffectsEc => {
            let reader = EffectsEcPackage::from_reader(package.clone())?;
            format!("textures={}", reader.textures().len())
        }
        PackageKind::Gumps => {
            GumpsPackage::from_uddp_package(package.clone())?;
            "gumps ok".into()
        }
        PackageKind::Map(map_id) => check_map_package(package, map_id)?,
        PackageKind::Statics(_) => check_statics_package(package)?,
        PackageKind::TerrainMeta => eyre::bail!("terrain_meta is not a UDDP package"),
    };
    Ok(summary)
}

/// The checks `dynamapper` makes when it opens a map package, plus the chunk sizes.
fn check_map_package(package: &UddpReader, map_id: u32) -> eyre::Result<String> {
    let records = package.records();
    let record = records.last().ok_or_else(|| eyre::eyre!("map package has no records"))?;
    eyre::ensure!(
        unpack_type(record.locator.meta32) == DataType::Metadata as u8,
        "map package is missing its final metadata record"
    );
    let FileKey::Id(id) = record.key else {
        eyre::bail!("map package metadata record is not id addressed");
    };
    let expected_id = records.len() as u32 - 1;
    eyre::ensure!(id == expected_id, "map metadata record id {id}, expected the final id {expected_id}");
    let bytes = package
        .read_file_by_dense_id(id)
        .map_err(|error| eyre::eyre!("read map metadata: {error}"))?;
    let metadata = decode_map_package_metadata(&bytes).map_err(|error| eyre::eyre!("map metadata: {error}"))?;
    eyre::ensure!(metadata.map_id == map_id, "map metadata is for map {}, the file name says map {map_id}", metadata.map_id);
    eyre::ensure!(
        metadata.chunk_count as usize + 1 == records.len(),
        "map metadata declares {} chunks, the package has {} records",
        metadata.chunk_count,
        records.len()
    );
    let wrong_size = records[..records.len() - 1]
        .iter()
        .filter(|record| record.locator.raw_size as usize != MAP_CHUNK_BYTES)
        .count();
    eyre::ensure!(wrong_size == 0, "{wrong_size} map chunks are not {MAP_CHUNK_BYTES} bytes");
    Ok(format!(
        "map {} {}x{} tiles, chunks={}",
        metadata.map_id, metadata.width_tiles, metadata.height_tiles, metadata.chunk_count
    ))
}

/// Statics chunks are arrays of `StaticTile`.
fn check_statics_package(package: &UddpReader) -> eyre::Result<String> {
    let records = package.records();
    let tile_size = std::mem::size_of::<StaticTile>();
    let ragged = records
        .iter()
        .filter(|record| record.locator.raw_size as usize % tile_size != 0)
        .count();
    eyre::ensure!(ragged == 0, "{ragged} statics chunks are not a whole number of {tile_size}-byte statics");
    let statics: usize = records.iter().map(|record| record.locator.raw_size as usize / tile_size).sum();
    Ok(format!("chunks={}, statics={statics}", records.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_container::{AddFileRequest, CompressionFlag, UddpBuilder};

    #[test]
    fn kinds_come_from_file_names() {
        assert_eq!(PackageKind::from_file_name("tex_land_ec.uddp"), Some(PackageKind::TexLandEc));
        assert_eq!(PackageKind::from_file_name("tex_land_ec.rebuilt.uddp"), Some(PackageKind::TexLandEc));
        assert_eq!(PackageKind::from_file_name("effects_ec_bc7.uddp"), Some(PackageKind::EffectsEc));
        assert_eq!(PackageKind::from_file_name("gumps_cc.uddp"), Some(PackageKind::Gumps));
        assert_eq!(PackageKind::from_file_name("map5.uddp"), Some(PackageKind::Map(5)));
        assert_eq!(PackageKind::from_file_name("statics0.uddp"), Some(PackageKind::Statics(0)));
        assert_eq!(PackageKind::from_file_name(TERRAIN_META_FILE_NAME), Some(PackageKind::TerrainMeta));
        assert_eq!(PackageKind::from_file_name("mapx.uddp"), None);
        assert_eq!(PackageKind::from_file_name("facet00.dds"), None);
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("udd_validate_{name}_{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn write_statics_package(path: &Path, chunk_bytes: &[&[u8]]) {
        let mut builder = UddpBuilder::new(LookupMode::DenseId);
        for (id, data) in chunk_bytes.iter().enumerate() {
            builder
                .add_file(AddFileRequest {
                    data_type: DataType::Static as u8,
                    compression: CompressionFlag::Auto,
                    width: 0,
                    height: 0,
                    virtual_path: None,
                    path_hash64: None,
                    id: Some(id as u32),
                    data,
                })
                .expect("add chunk");
        }
        fs::write(path, builder.build().expect("build package")).expect("write package");
    }

    #[test]
    fn a_sound_package_passes_and_every_entry_is_counted() {
        let dir = scratch_dir("sound");
        let path = dir.join("statics0.uddp");
        let tile = [0u8; std::mem::size_of::<StaticTile>()];
        let two_tiles = [tile, tile].concat();
        write_statics_package(&path, &[&tile, &two_tiles, &[]]);

        let report = validate_package(&path);

        assert!(report.passed(), "{:?}", report.failures);
        assert_eq!(report.kind, Some(PackageKind::Statics(0)));
        assert_eq!(report.entry_count, 3);
        assert_eq!(report.raw_bytes, (3 * tile.len()) as u64);
        assert_eq!(report.reader_summary.as_deref(), Some("chunks=3, statics=3"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_flipped_payload_byte_fails_the_package_hash() {
        let dir = scratch_dir("hash");
        let path = dir.join("statics0.uddp");
        let tile = [0u8; std::mem::size_of::<StaticTile>()];
        write_statics_package(&path, &[&tile]);
        let mut bytes = fs::read(&path).expect("read package");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        fs::write(&path, bytes).expect("write package");

        let report = validate_package(&path);

        assert!(!report.passed());
        assert!(report.failures.iter().any(|failure| failure.starts_with("package hash")), "{:?}", report.failures);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_wrong_reader_for_the_file_name_fails() {
        let dir = scratch_dir("wrong_reader");
        let path = dir.join("tex_land_ec.uddp");
        let tile = [0u8; std::mem::size_of::<StaticTile>()];
        write_statics_package(&path, &[&tile]);

        let report = validate_package(&path);

        assert!(!report.passed());
        assert!(report.failures.iter().any(|failure| failure.starts_with("tex_land_ec reader")), "{:?}", report.failures);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_ragged_statics_chunk_fails() {
        let dir = scratch_dir("ragged");
        let path = dir.join("statics1.uddp");
        write_statics_package(&path, &[&[0u8; 3]]);

        let report = validate_package(&path);

        assert!(!report.passed());
        assert!(report.failures.iter().any(|failure| failure.contains("not a whole number")), "{:?}", report.failures);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_truncated_package_fails_in_the_container() {
        let dir = scratch_dir("truncated");
        let path = dir.join("statics0.uddp");
        let tile = [0u8; std::mem::size_of::<StaticTile>()];
        write_statics_package(&path, &[&tile]);
        let bytes = fs::read(&path).expect("read package");
        fs::write(&path, &bytes[..bytes.len() / 2]).expect("write package");

        let report = validate_package(&path);

        assert!(!report.passed(), "a half package must fail");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn directories_are_searched_for_packages_only() {
        let dir = scratch_dir("collect");
        let nested = dir.join("nested");
        fs::create_dir_all(&nested).expect("create nested dir");
        fs::write(dir.join("facet00.dds"), b"").expect("write dds");
        fs::write(dir.join(TERRAIN_META_FILE_NAME), b"").expect("write terrain meta");
        fs::write(nested.join("hues.uddp"), b"").expect("write hues");

        let paths = collect_package_paths(&[dir.clone()]).expect("collect");

        assert_eq!(paths, vec![nested.join("hues.uddp"), dir.join(TERRAIN_META_FILE_NAME)]);
        fs::remove_dir_all(&dir).ok();
    }
}
