//! `udd-cli tool validate`: checks that packages open with the readers in this build.
//!
//! For every package it checks:
//! - the container: header, format version, index, dictionary table and package hash;
//! - every entry: it decodes through its own logical key, with its codec and dictionary,
//!   to exactly the declared raw size;
//! - the package's own reader: the `udd-assets` reader for its kind parses its manifests,
//!   which also checks the manifest format versions.
//!
//! A gumps package that shares gumps with `gumps_cc` (`pack-ec-gumps --share-with`) is also checked against the
//! `gumps_cc.uddp` beside it: every shared gump's hash against that package's image. Without one beside it, the report
//! notes that the package depends on it.
//!
//! The kind comes from the file name (`tex_land_ec.uddp`, `map0.uddp`).
//! A file with an unknown name is offered to every reader, and the first one that accepts it
//! names the kind.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use log::info;
use rayon::prelude::*;
use udd_assets::gumps::gump_image_hash;
use udd_assets::map_metadata::decode_map_package_metadata;
use udd_assets::tex_aux_ec::TexAuxEcPackage;
use udd_assets::{
    EffectsEcPackage, GumpsPackage, HuesPackage, MobileAnimCcPackage, MobileAnimEcPackage,
    PaperdollEcPackage, TexArtCcPackage, TexArtEcPackage, TexLandCcPackage, TexLandEcPackage, TileMetaPackage,
    WorldLightsPackage,
};
use udd_container::{
    reconstruct_stored_size, unpack_codec, unpack_offset40, unpack_type, Codec, DataType, FileKey,
    FileRecord, LookupMode, UddpReader,
};
use uocf::classic::statics::StaticTile;

/// File extension of UDDP packages.
const UDDP_EXTENSION: &str = "uddp";
/// Side of one packed map chunk, in tiles.
const MAP_CHUNK_TILE_DIM: usize = 32;
/// One packed map texel: tile id (u16) plus height and flags (u16).
const MAP_PACKED_TEXEL_BYTES: usize = 4;
/// Decoded size of every map chunk entry.
const MAP_CHUNK_BYTES: usize = MAP_CHUNK_TILE_DIM * MAP_CHUNK_TILE_DIM * MAP_PACKED_TEXEL_BYTES;
/// Entry failures listed per package; the rest are only counted.
const MAX_LISTED_ENTRY_FAILURES: usize = 16;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;
/// The package a gumps package that shares gumps draws them from, looked for beside it.
const SHARED_CLASSIC_GUMPS_FILE_NAME: &str = "gumps_cc.uddp";

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
    PaperdollEc,
    Gumps,
    Map(u32),
    Statics(u32),
}

/// UDDP kinds offered, in order, to a package whose file name names no kind.
const SNIFFED_KINDS: [PackageKind; 13] = [
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
    PackageKind::PaperdollEc,
    PackageKind::Gumps,
];

impl PackageKind {
    /// The kind named by a file name; a suffix after the first dot is ignored
    /// (`tex_land_ec.rebuilt.uddp` is a `tex_land_ec` package).
    pub fn from_file_name(file_name: &str) -> Option<Self> {
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
            "paperdoll_ec" => Self::PaperdollEc,
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
            Self::PaperdollEc => "paperdoll_ec".into(),
            Self::Gumps => "gumps".into(),
            Self::Map(map_id) => format!("map{map_id}"),
            Self::Statics(map_id) => format!("statics{map_id}"),
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
    /// What the package depends on or was checked against, which isn't a failure.
    pub notes: Vec<String>,
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
    let counts = validate_inputs(inputs, log_report)?;
    info!("{}", counts.summary_line());
    counts.ensure_passed()
}

/// How many packages one `validate` run checked, and how many of them failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidateCounts {
    pub packages: usize,
    pub failed: usize,
}

impl ValidateCounts {
    /// The line `validate` ends with.
    pub fn summary_line(&self) -> String {
        format!(
            "Validated {} packages: {} passed, {} failed",
            self.packages,
            self.packages - self.failed,
            self.failed
        )
    }

    /// Fails when any package failed.
    pub fn ensure_passed(&self) -> eyre::Result<()> {
        eyre::ensure!(self.failed == 0, "{} of {} packages failed validation", self.failed, self.packages);
        Ok(())
    }
}

/// Validate each package `inputs` names or holds (directories are searched recursively), handing each report to
/// `on_report` as soon as it is made. Fails when no package is found.
pub fn validate_inputs(
    inputs: &[PathBuf],
    mut on_report: impl FnMut(&PackageReport),
) -> eyre::Result<ValidateCounts> {
    let paths = collect_package_paths(inputs)?;
    eyre::ensure!(!paths.is_empty(), "no packages found (looked for *.{UDDP_EXTENSION})");

    let mut failed = 0usize;
    for path in &paths {
        let report = validate_package(path);
        on_report(&report);
        if !report.passed() {
            failed += 1;
        }
    }
    Ok(ValidateCounts { packages: paths.len(), failed })
}

/// What a line of a package report says, for a caller that shows each kind its own way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportLineKind {
    /// The status line of a package that passed.
    Passed,
    /// The status line of a package that failed.
    Failed,
    /// Entry counts, sizes, codecs and the reader's summary.
    Detail,
    /// What the package depends on or was checked against.
    Note,
    /// A failure.
    Error,
}

/// The lines `validate` logs for one package: its status line, then its entry, reader, note and error lines.
pub fn report_lines(report: &PackageReport) -> Vec<(ReportLineKind, String)> {
    let (status_kind, status) =
        if report.passed() { (ReportLineKind::Passed, "OK  ") } else { (ReportLineKind::Failed, "FAIL") };
    let mut lines = vec![(status_kind, format!("{status} {} [{}]", report.path.display(), report.kind_name()))];
    if report.entry_count > 0 {
        let codecs = report
            .codec_counts
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push((
            ReportLineKind::Detail,
            format!(
                "     entries={}, raw={:.1} MiB, stored={:.1} MiB, codecs: {}, dictionaries={}",
                report.entry_count,
                report.raw_bytes as f64 / BYTES_PER_MIB,
                report.stored_bytes as f64 / BYTES_PER_MIB,
                codecs,
                report.dictionary_count
            ),
        ));
    }
    if let Some(summary) = &report.reader_summary {
        lines.push((ReportLineKind::Detail, format!("     reader: {summary}")));
    }
    lines.extend(report.notes.iter().map(|note| (ReportLineKind::Note, format!("     note: {note}"))));
    lines.extend(report.failures.iter().map(|failure| (ReportLineKind::Error, format!("     error: {failure}"))));
    lines
}

fn log_report(report: &PackageReport) {
    for (_, line) in report_lines(report) {
        info!("{line}");
    }
}

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
    path.extension().and_then(|ext| ext.to_str()) == Some(UDDP_EXTENSION)
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
            Ok(summary) => {
                report.reader_summary = Some(summary);
                if kind == PackageKind::Gumps {
                    check_shared_classic_gumps(path, &package, &mut report);
                }
            }
            Err(error) => report.failures.push(format!("{} reader: {error:#}", kind.name())),
        },
        None => report
            .failures
            .push("no reader in this build accepts the package, and its file name names no kind".into()),
    }
    report
}

/// A gumps package that shares gumps with `gumps_cc`: each shared gump's listed hash against the image of the
/// `gumps_cc.uddp` beside it, or a note that the package depends on one when none is there.
fn check_shared_classic_gumps(path: &Path, package: &UddpReader, report: &mut PackageReport) {
    let gumps = match GumpsPackage::from_uddp_package(package.clone()) {
        Ok(gumps) => gumps,
        Err(error) => return report.failures.push(format!("gumps reader: {error:#}")),
    };
    let shared_count = gumps.shared_classic_gump_count();
    if shared_count == 0 {
        return;
    }
    let classic_path = path.with_file_name(SHARED_CLASSIC_GUMPS_FILE_NAME);
    if path.file_name() == classic_path.file_name() {
        return report.failures.push(format!(
            "{SHARED_CLASSIC_GUMPS_FILE_NAME} shares {shared_count} gumps with {SHARED_CLASSIC_GUMPS_FILE_NAME}: only a \
             gumps_ec may share, and a gumps_cc must be standalone"
        ));
    }
    if !classic_path.is_file() {
        return report.notes.push(format!(
            "shares {shared_count} gumps with {SHARED_CLASSIC_GUMPS_FILE_NAME}, and none is beside it: it depends on \
             one (or the Classic client files) to draw them"
        ));
    }
    let classic = match GumpsPackage::load(&classic_path) {
        Ok(classic) => classic,
        Err(error) => {
            return report.failures.push(format!("shared gumps: {}: {error:#}", classic_path.display()));
        }
    };
    let shared = gumps.shared_classic_gumps().collect::<Vec<_>>();
    let differing = shared
        .par_iter()
        .filter_map(|shared| {
            let problem = match classic.read_single_gump_image(shared.gump_id) {
                Ok(Some((_, image))) if gump_image_hash(&image) == shared.image_hash => return None,
                Ok(Some(_)) => "draws it differently".to_string(),
                Ok(None) => "has no single gump with its id".to_string(),
                Err(error) => format!("does not decode it: {error:#}"),
            };
            Some(format!("shared gump {}: {SHARED_CLASSIC_GUMPS_FILE_NAME} {problem}", shared.gump_id))
        })
        .collect::<Vec<_>>();
    if differing.is_empty() {
        return report.notes.push(format!("{shared_count} shared gumps match {}", classic_path.display()));
    }
    let differing_count = differing.len();
    report.failures.extend(differing.into_iter().take(MAX_LISTED_ENTRY_FAILURES));
    report.failures.push(format!(
        "{differing_count} of {shared_count} shared gumps differ: {} is not the {SHARED_CLASSIC_GUMPS_FILE_NAME} this \
         package was packed against",
        classic_path.display()
    ));
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
                "land_tiles={}, item_tiles={}, item_definitions={}",
                reader.land_tiles().len(),
                reader.item_tiles().len(),
                reader.item_definitions().entry_ids().count()
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
        PackageKind::PaperdollEc => check_paperdoll_ec_package(package)?,
        PackageKind::Gumps => {
            GumpsPackage::from_uddp_package(package.clone())?;
            "gumps ok".into()
        }
        PackageKind::Map(map_id) => check_map_package(package, map_id)?,
        PackageKind::Statics(_) => check_statics_package(package)?,
    };
    Ok(summary)
}

/// Reads every frame and hue mask of a `paperdoll_ec` package, which checks their sizes against the manifest.
fn check_paperdoll_ec_package(package: &UddpReader) -> eyre::Result<String> {
    let reader = PaperdollEcPackage::from_uddp_package(package.clone())?;
    let mut set_frames = vec![0usize; reader.sets().len()];
    let mut masked = 0;
    for frame in reader.frames() {
        let rgba = reader.read_frame_rgba(frame)?;
        reader.read_frame_hue_mask(frame, &rgba)?;
        set_frames[usize::from(frame.set)] += 1;
        masked += usize::from(frame.hue_mode == udd_assets::paperdoll_ec::FRAME_HUE_MASKED);
    }
    Ok(format!(
        "frames per set={set_frames:?}, hue_masks={masked}, hue_tables={}, dress_decisions={}",
        reader.hue_tables().tables.len(),
        reader.dress_hides().len()
    ))
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
        assert_eq!(PackageKind::from_file_name("paperdoll_ec.uddp"), Some(PackageKind::PaperdollEc));
        assert_eq!(PackageKind::from_file_name("map5.uddp"), Some(PackageKind::Map(5)));
        assert_eq!(PackageKind::from_file_name("statics0.uddp"), Some(PackageKind::Statics(0)));
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
        fs::write(nested.join("hues.uddp"), b"").expect("write hues");

        let paths = collect_package_paths(&[dir.clone()]).expect("collect");

        assert_eq!(paths, vec![nested.join("hues.uddp")]);
        fs::remove_dir_all(&dir).ok();
    }

    /// The report lines carry the status, entry, reader, note and error lines `validate` logs, in that order.
    #[test]
    fn report_lines_mark_each_line_with_its_kind() {
        let dir = scratch_dir("report_lines");
        let path = dir.join("statics0.uddp");
        let tile = [0u8; std::mem::size_of::<StaticTile>()];
        write_statics_package(&path, &[&tile]);
        let mut report = validate_package(&path);
        report.notes.push("a note".into());
        report.failures.push("a failure".into());

        let kinds: Vec<ReportLineKind> = report_lines(&report).into_iter().map(|(kind, _)| kind).collect();

        assert_eq!(
            kinds,
            [
                ReportLineKind::Failed,
                ReportLineKind::Detail,
                ReportLineKind::Detail,
                ReportLineKind::Note,
                ReportLineKind::Error,
            ]
        );
        let lines = report_lines(&report);
        assert_eq!(lines[0].1, format!("FAIL {} [statics0]", path.display()));
        assert_eq!(lines[3].1, "     note: a note");
        assert_eq!(lines[4].1, "     error: a failure");
        fs::remove_dir_all(&dir).ok();
    }

    /// `validate_inputs` hands over a report per package and counts the failures; an input without packages fails.
    #[test]
    fn validate_inputs_counts_passed_and_failed_packages() {
        let dir = scratch_dir("inputs");
        let tile = [0u8; std::mem::size_of::<StaticTile>()];
        write_statics_package(&dir.join("statics0.uddp"), &[&tile]);
        write_statics_package(&dir.join("statics1.uddp"), &[&[0u8; 3]]);
        let mut statuses = Vec::new();

        let counts = validate_inputs(&[dir.clone()], |report| statuses.push(report.passed())).expect("validate");

        assert_eq!(counts, ValidateCounts { packages: 2, failed: 1 });
        assert_eq!(statuses, [true, false]);
        assert_eq!(counts.summary_line(), "Validated 2 packages: 1 passed, 1 failed");
        assert!(counts.ensure_passed().is_err());
        let empty = dir.join("empty");
        fs::create_dir_all(&empty).expect("create empty dir");
        assert!(validate_inputs(&[empty], |_| {}).is_err());
        fs::remove_dir_all(&dir).ok();
    }

    /// Writes a lossless gumps package of `gumps` (id, one RGBA texel), sharing with `share_with` when given.
    fn write_gumps_package(path: &Path, gumps: &[(u32, [u8; 4])], share_with: Option<&Path>) {
        use udd_conv::single_gumps::{default_single_gump_compression, GumpPixelFormat, SingleGumpWriter};
        const SIDE: u32 = 1;
        let format = GumpPixelFormat::Rgba8888;
        let mut writer = SingleGumpWriter::new(format, default_single_gump_compression(format));
        if let Some(share_with) = share_with {
            let classic = GumpsPackage::load(share_with).expect("load the gumps_cc to share with");
            writer = writer.with_shared_classic_gumps(std::sync::Arc::new(classic));
        }
        let mut builder = UddpBuilder::new(LookupMode::SparseId);
        for (gump_id, rgba) in gumps {
            writer.push(&mut builder, *gump_id, SIDE, SIDE, rgba.to_vec()).expect("push gump");
        }
        writer.finish(&mut builder).expect("finish single gumps");
        udd_conv::gump_atlas::add_gump_atlas_files(&mut builder, Vec::new(), &Default::default())
            .expect("add the atlas manifests");
        fs::write(path, builder.build().expect("build package")).expect("write package");
    }

    /// A gumps_ec that shares gumps passes against the gumps_cc.uddp it was packed against, passes with a note when
    /// none is beside it, and fails, naming each differing gump, against another one.
    #[test]
    fn shared_gumps_are_checked_against_the_gumps_cc_beside_them() {
        const TWIN: u32 = 1000;
        const OTHER_TWIN: u32 = 1001;
        const ENHANCED_ONLY: u32 = 1002;
        const ORANGE: [u8; 4] = [248, 128, 8, 255];
        const GREEN: [u8; 4] = [8, 248, 128, 255];
        let dir = scratch_dir("shared_gumps");
        let classic_path = dir.join(SHARED_CLASSIC_GUMPS_FILE_NAME);
        let enhanced_path = dir.join("gumps_ec.uddp");
        write_gumps_package(&classic_path, &[(TWIN, ORANGE), (OTHER_TWIN, ORANGE)], None);
        write_gumps_package(
            &enhanced_path,
            &[(TWIN, ORANGE), (OTHER_TWIN, ORANGE), (ENHANCED_ONLY, GREEN)],
            Some(&classic_path),
        );

        let report = validate_package(&enhanced_path);
        assert!(report.passed(), "{:?}", report.failures);
        assert!(report.notes.iter().any(|note| note.starts_with("2 shared gumps match")), "{:?}", report.notes);
        let standalone = validate_package(&classic_path);
        assert!(standalone.passed() && standalone.notes.is_empty(), "{standalone:?}");

        fs::remove_file(&classic_path).expect("remove gumps_cc");
        let report = validate_package(&enhanced_path);
        assert!(report.passed(), "{:?}", report.failures);
        assert!(report.notes.iter().any(|note| note.contains("it depends on one")), "{:?}", report.notes);

        write_gumps_package(&classic_path, &[(TWIN, GREEN), (OTHER_TWIN, ORANGE)], None);
        let report = validate_package(&enhanced_path);
        assert_eq!(
            report.failures,
            [
                format!("shared gump {TWIN}: gumps_cc.uddp draws it differently"),
                format!(
                    "1 of 2 shared gumps differ: {} is not the gumps_cc.uddp this package was packed against",
                    classic_path.display()
                ),
            ]
        );
        fs::remove_dir_all(&dir).ok();
    }
}
