//! Packs `effects_ec.uddp` from an Enhanced Client directory: the effect definition trees of
//! `EffectDefinitionCollection.uop`, `tileart.uop` and `AnimationSequence.uop`, the Chaos and NIF systems they
//! reach (compiled with `particle-sim`) and the textures of those systems.
//!
//! The layout and the rules (closure, names, mips, texture formats) are documented in the `udd-assets` README
//! (`effects_ec.uddp`); the records and manifest encoders come from `udd_assets::effects_ec`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr, bail, ensure, eyre};
use gamebryo_nif::{Link, MipFlag, NiPixelData, NiSourceTexture, NifFile};
use log::info;
use particle_sim::chaos::compile::compile_chaos_source;
use particle_sim::chaos::{ChaosParticleType, ChaosSystemDesc};
use particle_sim::codec::{decode_chaos_desc, decode_nif_desc, encode_chaos_desc, encode_nif_desc};
use particle_sim::nif::compile::decode_pixel_level;
use particle_sim::nif::desc::NifTextureSource;
use particle_sim::nif::{NifCompileOutput, compile as compile_nif};
use particle_sim::NifEffectDesc;
use udd_assets::effects_ec::{
    COLLECTION_ENTRY_PATH, COLLECTION_MAGIC, CollectionRecord, DEF_BRANCHES_ENTRY_PATH, DEF_BRANCHES_MAGIC,
    DEF_LISTS_ENTRY_PATH, DEF_LISTS_MAGIC, DEF_NODES_ENTRY_PATH, DEF_NODES_MAGIC, DEF_PARAMS_ENTRY_PATH,
    DEF_PARAMS_MAGIC, DEP_SYSTEM_BIT, DEPS_ENTRY_PATH, DEPS_MAGIC, DefBranchRecord, DefNodeRecord, NODE_FLAG_HUED,
    NODE_FLAG_LOOPING, NODE_FLAG_PROPERTY_MODE, NODE_FLAG_WAIT_ALIGNED, NONE, PACKAGE_ENTRY_PATH, PACKAGE_MAGIC,
    PackageRecord, ParticleNode, RGBA8_BYTES_PER_PIXEL, SEQUENCE_FRAMES_ENTRY_PATH, SEQUENCE_FRAMES_MAGIC,
    STRINGS_ENTRY_PATH, SYSTEM_FLAG_COMPILE_FAILED, SYSTEM_FLAG_COMPILE_WARNINGS, SYSTEM_FLAG_DEFINITION_ROOT,
    SYSTEM_FLAG_MISSING, SYSTEM_KIND_CHAOS, SYSTEM_KIND_NIF, SYSTEMS_ENTRY_PATH, SYSTEMS_MAGIC,
    SequenceFrameRecord, SystemRecord, TEXTURE_FLAG_AUTHORED_MIPS, TEXTURE_FLAG_DECODE_FAILED,
    TEXTURE_FLAG_EMBEDDED, TEXTURE_FLAG_MISSING, TEXTURE_FLAG_OPAQUE, TEXTURE_FLAG_UNDECODED, TEXTURE_FORMAT_BC1,
    TEXTURE_FORMAT_BC3, TEXTURE_FORMAT_BC7, TEXTURE_FORMAT_NONE, TEXTURE_FORMAT_RGBA8, TEXTURE_SOURCE_DDS,
    TEXTURE_SOURCE_NIF_FILE, TEXTURE_SOURCE_NIF_PIXEL_DATA, TEXTURE_SOURCE_TGA, TEXTURES_ENTRY_PATH, TEXTURES_MAGIC,
    TILEART_ENTRY_PATH, TILEART_MAGIC, TextureRecord, TileArtRecord, BC_BLOCK_EDGE, earthquake_params,
    encode_manifest, encode_strings, mip_extent, normalize_resource_name, particle_params, position_offset_params,
    property_flag_params, system_entry_path, texture_entry_path, texture_payload_len,
};
use udd_container::{AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};
use uocf::animation_sequence::AnimationSequence;
use uocf::enhanced::effect_definition::{
    EFFECT_DEFINITION_COLLECTION_UOP_NAME, EffectDefinition, EffectDefinitionCollection, EffectEntryKey, EffectIssue,
    EffectIssueKind, EffectIssueSeverity, EffectMapEntry, EffectValidationContext, PropertyFlagMode,
    validate_effect_definitions,
};
use uocf::enhanced::effect_texture::{EFFECT_TEXTURE_UOP_NAME, EffectTexturePackage};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tileart::TileArtEntry;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::{LoadMode, UopPackage};

use crate::bc7::{ImageExtent, RawImageFormat, encode_to_bc7, preferred_bc7_encoder_backend};
use crate::package_progress::build_and_write_package_with_progress;
use crate::progress::{ProgressBar, ProgressStyle};
use crate::source_paths::find_first_existing_file;

/// The client's string dictionary (particle names of the definitions).
const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";
/// Tile records, with the effects of world objects.
const TILEART_UOP_NAME: &str = "tileart.uop";
/// Mobile animation sequences, with the frame effects.
const ANIMATION_SEQUENCE_UOP_NAMES: [&str; 2] = ["AnimationSequence.uop", "animationsequence.uop"];
/// Highest body type probed in `AnimationSequence.uop` (its entries are named by body type only).
const MAX_SEQUENCE_BODY_TYPE: u32 = u16::MAX as u32;

/// Extension of a Chaos script name.
const CHAOS_EXTENSION: &str = ".ems";
/// Extension of a Gamebryo scene name.
const NIF_EXTENSION: &str = ".nif";
/// Extension of a Targa texture name.
const TGA_EXTENSION: &str = ".tga";
/// Extension of a DirectDraw Surface texture name.
const DDS_EXTENSION: &str = ".dds";
/// Separator of an embedded texture name: `<nif name>#<texture index>`.
const EMBEDDED_TEXTURE_SEPARATOR: char = '#';
/// Extensions of the `EffectTexture.uop` resources the closure can include (others are never played).
const RESOURCE_EXTENSIONS: [&str; 4] = [CHAOS_EXTENSION, NIF_EXTENSION, TGA_EXTENSION, DDS_EXTENSION];

/// Largest texture edge: wgpu's default `max_texture_dimension_2d` (the record also stores it as u16).
const MAX_TEXTURE_EDGE: u32 = 8192;
/// Largest number of mip levels a record stores.
const MAX_MIP_LEVELS: usize = u8::MAX as usize;
/// Alpha of an opaque pixel.
const OPAQUE_ALPHA: u8 = u8::MAX;
/// Index of the alpha byte of an RGBA8 pixel.
const ALPHA_CHANNEL: usize = 3;

/// Size of the TGA header.
const TGA_HEADER_SIZE: usize = 18;
/// TGA image types read by `NiTGAReader::ReadHeader` (`NiTGAReader.h:40-45`).
const TGA_TYPE_MAP: u8 = 1;
const TGA_TYPE_RGB: u8 = 2;
const TGA_TYPE_MONO: u8 = 3;
const TGA_TYPE_RLE_MAP: u8 = 9;
const TGA_TYPE_RLE_RGB: u8 = 10;
const TGA_TYPE_RLE_MONO: u8 = 11;
/// TGA attribute bit: rows stored top first (clear: bottom first, flipped by `NiTGAReader`).
const TGA_ATTR_TOP_ORIGIN: u8 = 0x20;
/// TGA RLE packet bit: one pixel repeated (clear: raw pixels).
const TGA_RLE_RUN_BIT: u8 = 0x80;
/// Repeat count of an RLE run packet is `tag - TGA_RLE_RUN_BIAS`.
const TGA_RLE_RUN_BIAS: usize = 127;
/// Pixel and colour map entry sizes, in bits.
const TGA_BITS_8: u8 = 8;
const TGA_BITS_15: u8 = 15;
const TGA_BITS_16: u8 = 16;
const TGA_BITS_24: u8 = 24;
const TGA_BITS_32: u8 = 32;
/// `NiTGAReader::ReadFile`: a colour map that would reach this many entries is rejected (`MAXCOLORS`).
const TGA_MAX_COLORS: usize = 4096;

/// DDS file magic.
const DDS_MAGIC: &[u8; 4] = b"DDS ";
/// Bytes of the magic and the `DDS_HEADER`.
const DDS_HEADER_END: usize = 128;
/// Bytes of the `DDS_HEADER_DXT10` extension that follows when the FourCC is `DX10`.
const DDS_DXT10_HEADER_SIZE: usize = 20;
/// File offsets of the `DDS_HEADER` fields read here.
const DDS_MIP_COUNT_OFFSET: usize = 28;
const DDS_FOURCC_OFFSET: usize = 84;
const DDS_CAPS_OFFSET: usize = 108;
const DDS_FOURCC_DX10: &[u8; 4] = b"DX10";
/// `DDSCAPS_COMPLEX` and `DDSCAPS_MIPMAP`: `NiDDSReader::ReadHeader` keeps the file's levels only when both are set
/// and the count is not 1 (`NiDDSReader.cpp:313-323`).
const DDSCAPS_COMPLEX: u32 = 0x8;
const DDSCAPS_MIPMAP: u32 = 0x40_0000;

/// Whether textures that only Chaos scripts (or NIFs with `MIP_DEFAULT`) reference get mipmaps.
///
/// Gamebryo 2.6 `NiTexture::ms_bMipmapByDefault` is `false` (`CoreLibs/NiMain/NiTexture.cpp:27`), and the DX9
/// renderer mipmaps a source texture only when its format preferences say `YES`, or `MIP_DEFAULT` with that
/// default set (`NiDX9Renderer/NiDX9SourceTextureData.cpp:375-378`); `FormatPrefs` default to `MIP_DEFAULT`
/// (`NiTexture.inl:31`). Whether UOSA.exe calls `NiTexture::SetMipmapByDefault` is not verified.
const MIPMAP_BY_DEFAULT: bool = false;

/// Severity of a packer finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingSeverity {
    Info,
    Warning,
    /// Fails the build with [`EffectsEcPackOptions::strict`].
    Error,
}

/// One packer finding (one line of the report).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: FindingSeverity,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            FindingSeverity::Info => "info",
            FindingSeverity::Warning => "warning",
            FindingSeverity::Error => "error",
        };
        write!(f, "{severity}: {}", self.message)
    }
}

/// Options of [`pack_effects_ec`].
#[derive(Debug, Clone)]
pub struct EffectsEcPackOptions {
    /// Encode uncompressed sources whose size is a multiple of 4 to BC7 (no RDO); otherwise they stay RGBA8.
    pub bc7: bool,
    /// Compression of the manifests and texture payloads (system blobs use `Auto`, with the type dictionary).
    pub compression: CompressionFlag,
    /// Fail when a finding has severity [`FindingSeverity::Error`] instead of writing a flagged record.
    pub strict: bool,
    /// Where to write every finding, one line each (written before the strict check).
    pub report: Option<PathBuf>,
}

impl Default for EffectsEcPackOptions {
    fn default() -> Self {
        Self { bc7: false, compression: CompressionFlag::ZstdNoDict, strict: false, report: None }
    }
}

/// What [`pack_effects_ec`] wrote and found.
#[derive(Debug, Clone, Default)]
pub struct EffectsEcPackSummary {
    pub collection_entries: usize,
    pub tileart_records: usize,
    pub sequence_frames: usize,
    pub nodes: usize,
    pub chaos_systems: usize,
    pub nif_systems: usize,
    pub systems_missing: usize,
    pub systems_failed: usize,
    pub systems_with_warnings: usize,
    pub textures: usize,
    /// Textures with a payload, per `TEXTURE_FORMAT_*`.
    pub textures_by_format: BTreeMap<u8, usize>,
    pub textures_missing: usize,
    pub textures_failed: usize,
    pub textures_generated_mips: usize,
    pub textures_authored_mips: usize,
    /// Named `EffectTexture.uop` resources that nothing reaches.
    pub unreferenced_resources: usize,
    /// Bytes of the system blobs and texture payloads before package compression.
    pub system_bytes: usize,
    pub texture_bytes: usize,
    pub findings: Vec<Finding>,
}

impl EffectsEcPackSummary {
    pub fn count(&self, severity: FindingSeverity) -> usize {
        self.findings.iter().filter(|finding| finding.severity == severity).count()
    }
}

impl fmt::Display for EffectsEcPackSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let format_name = |format: u8| match format {
            TEXTURE_FORMAT_RGBA8 => "RGBA8",
            TEXTURE_FORMAT_BC7 => "BC7",
            TEXTURE_FORMAT_BC1 => "BC1",
            TEXTURE_FORMAT_BC3 => "BC3",
            _ => "?",
        };
        let formats =
            self.textures_by_format.iter().map(|(&format, count)| format!("{count} {}", format_name(format)));
        write!(
            f,
            "{} collection entries, {} tiles, {} sequence frames, {} definition nodes; {} systems ({} Chaos, {} NIF; \
             {} missing, {} failed, {} with warnings; {} KiB of blobs); {} textures ({}; {} missing, {} failed; {} \
             with generated mips, {} with authored mips; {} KiB); {} unreferenced resources; findings: {} errors, {} \
             warnings, {} info",
            self.collection_entries,
            self.tileart_records,
            self.sequence_frames,
            self.nodes,
            self.chaos_systems + self.nif_systems,
            self.chaos_systems,
            self.nif_systems,
            self.systems_missing,
            self.systems_failed,
            self.systems_with_warnings,
            self.system_bytes / KIB,
            self.textures,
            formats.collect::<Vec<_>>().join(", "),
            self.textures_missing,
            self.textures_failed,
            self.textures_generated_mips,
            self.textures_authored_mips,
            self.texture_bytes / KIB,
            self.unreferenced_resources,
            self.count(FindingSeverity::Error),
            self.count(FindingSeverity::Warning),
            self.count(FindingSeverity::Info),
        )
    }
}

/// Bytes per KiB (summary only).
const KIB: usize = 1024;

#[derive(Default)]
struct Report {
    findings: Vec<Finding>,
}

impl Report {
    fn add(&mut self, severity: FindingSeverity, message: impl Into<String>) {
        self.findings.push(Finding { severity, message: message.into() });
    }

    fn error(&mut self, message: impl Into<String>) {
        self.add(FindingSeverity::Error, message);
    }

    fn warning(&mut self, message: impl Into<String>) {
        self.add(FindingSeverity::Warning, message);
    }

    fn info(&mut self, message: impl Into<String>) {
        self.add(FindingSeverity::Info, message);
    }
}

/// Packs `effects_ec.uddp` from the first source directory holding each client file.
pub fn pack_effects_ec(
    source_dirs: &[PathBuf],
    output: &Path,
    options: &EffectsEcPackOptions,
) -> eyre::Result<EffectsEcPackSummary> {
    let package = build_checked(source_dirs, options)?;
    info!("effects_ec: writing {}", output.display());
    build_and_write_package_with_progress(package.builder, output, |_| {})?;
    Ok(package.summary)
}

/// [`pack_effects_ec`] into memory: the package bytes.
pub fn pack_effects_ec_to_bytes(
    source_dirs: &[PathBuf],
    options: &EffectsEcPackOptions,
) -> eyre::Result<(Vec<u8>, EffectsEcPackSummary)> {
    let package = build_checked(source_dirs, options)?;
    Ok((package.builder.into_bytes()?, package.summary))
}

/// Builds the package, writes the report and applies [`EffectsEcPackOptions::strict`].
fn build_checked(source_dirs: &[PathBuf], options: &EffectsEcPackOptions) -> eyre::Result<BuiltPackage> {
    let package = build_effects_ec(source_dirs, options)?;
    if let Some(path) = &options.report {
        let lines: String = package.summary.findings.iter().map(|finding| format!("{finding}\n")).collect();
        std::fs::write(path, lines).wrap_err_with(|| format!("write {}", path.display()))?;
    }
    let errors = package.summary.count(FindingSeverity::Error);
    if options.strict && errors != 0 {
        let first = package.summary.findings.iter().filter(|f| f.severity == FindingSeverity::Error).take(10);
        let lines = first.map(ToString::to_string).collect::<Vec<_>>().join("\n");
        bail!("{errors} error findings with --strict; nothing written. First ones:\n{lines}");
    }
    Ok(package)
}

/// A built package not yet written.
struct BuiltPackage {
    builder: UddpBuilder,
    summary: EffectsEcPackSummary,
}

/// The client files the packer reads.
struct Sources {
    effect_files: EffectTexturePackage,
    strings: UoStringDictionary,
    collection: EffectDefinitionCollection,
    tileart: PathBuf,
    sequences: Option<PathBuf>,
}

impl Sources {
    fn load(source_dirs: &[PathBuf]) -> eyre::Result<Self> {
        let find = |name: &str| {
            find_first_existing_file(source_dirs, &[name]).ok_or_else(|| eyre!("{name} not found in {source_dirs:?}"))
        };
        let effect_texture = find(EFFECT_TEXTURE_UOP_NAME)?;
        let client_dir = effect_texture.parent().map(Path::to_path_buf).unwrap_or_default();
        let effect_files = EffectTexturePackage::load_with_names(&client_dir)
            .wrap_err_with(|| format!("load {}", effect_texture.display()))?;
        let strings_path = find(STRING_DICTIONARY_UOP_NAME)?;
        let strings =
            UoStringDictionary::load(&strings_path).wrap_err_with(|| format!("load {}", strings_path.display()))?;
        let collection_path = find(EFFECT_DEFINITION_COLLECTION_UOP_NAME)?;
        let collection = EffectDefinitionCollection::load(&collection_path)
            .wrap_err_with(|| format!("load {}", collection_path.display()))?;
        Ok(Self {
            effect_files,
            strings,
            collection,
            tileart: find(TILEART_UOP_NAME)?,
            sequences: find_first_existing_file(source_dirs, &ANIMATION_SEQUENCE_UOP_NAMES),
        })
    }
}

/// The definition lists of every entry table, sorted by key.
struct DefinitionSources {
    collection: Vec<(u32, Vec<EffectDefinition>)>,
    tileart: Vec<(u32, Vec<EffectDefinition>)>,
    /// `(sequence_id, action, frame)`.
    frames: Vec<((u32, u16, u16), Vec<EffectDefinition>)>,
    /// Particle files not in `EffectTexture.uop`, by normalized name: the definitions naming them
    /// (`collection entry 9011 [0]`, `tile 6686 [0]`, ...). Reported once per file, not per definition.
    missing_particle_files: BTreeMap<String, BTreeSet<String>>,
}

/// Where a validation issue is: `<prefix> entry <id> [<path>]`, or `<prefix> [<path>]` for a bare list.
fn issue_location(prefix: &str, issue: &EffectIssue) -> String {
    let mut location = match issue.entry {
        Some(EffectEntryKey::Id(id)) => format!("{prefix} entry {id}"),
        Some(EffectEntryKey::Hash(hash)) => format!("{prefix} entry {hash:016X}"),
        None => prefix.to_owned(),
    };
    if !issue.path.is_empty() {
        location.push_str(&format!(" [{}]", issue.path));
    }
    location
}

fn load_definitions(sources: &Sources, report: &mut Report) -> eyre::Result<DefinitionSources> {
    let file_exists = |name: &str| sources.effect_files.contains(name);
    let context = EffectValidationContext { strings: Some(&sources.strings), particle_file_exists: Some(&file_exists) };
    let mut missing_particle_files: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut validation_findings = |issues: Vec<EffectIssue>, prefix: &str| {
        for issue in issues {
            // A client data gap, not a packer error: collected and reported once per file with the systems.
            if let EffectIssueKind::MissingParticleFile { name } = &issue.kind {
                let referrers = missing_particle_files.entry(normalize_resource_name(name)).or_default();
                referrers.insert(issue_location(prefix, &issue));
                continue;
            }
            let severity = match issue.severity() {
                EffectIssueSeverity::Error => FindingSeverity::Error,
                EffectIssueSeverity::Warning => FindingSeverity::Warning,
            };
            report.add(severity, format!("{prefix} {issue}"));
        }
    };

    validation_findings(sources.collection.validate(&context), "collection");
    let mut collection = Vec::new();
    for entry in &sources.collection.entries {
        if let EffectEntryKey::Id(id) = entry.key {
            collection.push((id, entry.definitions.clone()));
        }
    }
    collection.sort_by_key(|(id, _)| *id);

    let mut tileart: BTreeMap<u32, Vec<EffectDefinition>> = BTreeMap::new();
    let package = UopPackage::load(&sources.tileart).wrap_err_with(|| format!("load {}", sources.tileart.display()))?;
    let mut tile_findings = Vec::new();
    for file in package.iter_files() {
        let entry = match TileArtEntry::parse_raw(file) {
            Ok(entry) => entry,
            Err(error) => {
                tile_findings.push(Finding {
                    severity: FindingSeverity::Error,
                    message: format!("tileart entry {:016X} does not parse: {error}", file.filename_hash()),
                });
                continue;
            }
        };
        if entry.effects.is_empty() {
            continue;
        }
        if tileart.contains_key(&entry.tile_id) {
            tile_findings.push(Finding {
                severity: FindingSeverity::Warning,
                message: format!("tile {}: repeated tileart record with effects; the first is kept", entry.tile_id),
            });
            continue;
        }
        tileart.insert(entry.tile_id, entry.effects);
    }
    for (tile_id, definitions) in &tileart {
        validation_findings(validate_effect_definitions(definitions, &context), &format!("tile {tile_id}"));
    }

    let mut frames = Vec::new();
    if let Some(path) = &sources.sequences {
        let mut package = UopPackage::load_with_mode(path, LoadMode::Lazy)
            .wrap_err_with(|| format!("load {}", path.display()))?;
        for sequence_id in 0..=MAX_SEQUENCE_BODY_TYPE {
            let hash = hash_file_name_single(&AnimationSequence::entry_path(sequence_id));
            if package.get_file_by_hash(hash).is_none() {
                continue;
            }
            let Some(data) = package.unpack_file_by_hash_cached(hash)? else {
                continue;
            };
            let sequence = match AnimationSequence::parse(&data) {
                Ok(sequence) => sequence,
                Err(error) => {
                    tile_findings.push(Finding {
                        severity: FindingSeverity::Error,
                        message: format!("sequence {sequence_id} does not parse: {error}"),
                    });
                    continue;
                }
            };
            for (&action, record) in &sequence.actions {
                for (&frame, definitions) in &record.frame_effects {
                    if definitions.is_empty() {
                        continue;
                    }
                    let what = format!("sequence {sequence_id} action {action} frame {frame}");
                    let (Ok(action16), Ok(frame16)) = (u16::try_from(action), u16::try_from(frame)) else {
                        tile_findings.push(Finding {
                            severity: FindingSeverity::Error,
                            message: format!("{what}: action or frame above u16; dropped"),
                        });
                        continue;
                    };
                    if let Some(alias) = record.alias {
                        tile_findings.push(Finding {
                            severity: FindingSeverity::Info,
                            message: format!(
                                "{what}: alias record (of {alias}) with effects; stored under its own action {action}, \
                                 where the client looks frame effects up"
                            ),
                        });
                    }
                    frames.push(((sequence_id, action16, frame16), definitions.clone()));
                }
            }
        }
    } else {
        tile_findings.push(Finding {
            severity: FindingSeverity::Warning,
            message: "AnimationSequence.uop not found: no sequence frame effects".to_owned(),
        });
    }
    frames.sort_by_key(|(key, _)| *key);
    for ((sequence_id, action, frame), definitions) in &frames {
        validation_findings(
            validate_effect_definitions(definitions, &context),
            &format!("sequence {sequence_id} action {action} frame {frame}"),
        );
    }
    drop(validation_findings);
    report.findings.extend(tile_findings);
    for entry in &sources.collection.entries {
        if let EffectEntryKey::Hash(hash) = entry.key {
            report.warning(format!(
                "collection entry {hash:016X} ({} definitions) has no id name: unreachable by id, dropped",
                entry.definitions.len()
            ));
        }
    }
    Ok(DefinitionSources { collection, tileart: tileart.into_iter().collect(), frames, missing_particle_files })
}

/// The definition pools being built. Particle system indices are patched in once the system table is sorted.
#[derive(Default)]
struct DefinitionTables {
    nodes: Vec<DefNodeRecord>,
    lists: Vec<u32>,
    branches: Vec<DefBranchRecord>,
    params: Vec<u32>,
    /// `(params index of the system word, normalized system name)` of every Particle node with a usable name.
    particle_systems: Vec<(usize, String)>,
}

impl DefinitionTables {
    /// Flattens a list; returns `(first, count)` in `lists`.
    fn list(&mut self, definitions: &[EffectDefinition], strings: &UoStringDictionary, report: &mut Report) -> (u32, u32) {
        let nodes: Vec<u32> = definitions.iter().map(|definition| self.node(definition, strings, report)).collect();
        let first = pool_index(self.lists.len());
        self.lists.extend_from_slice(&nodes);
        (first, pool_index(nodes.len()))
    }

    /// Flattens keyed or weighted lists into consecutive branches; returns `(first, count)` in `branches`.
    fn branches(
        &mut self,
        entries: &[(u32, &[EffectDefinition])],
        strings: &UoStringDictionary,
        report: &mut Report,
    ) -> (u32, u32) {
        let records: Vec<DefBranchRecord> = entries
            .iter()
            .map(|&(key, definitions)| {
                let (list_first, list_count) = self.list(definitions, strings, report);
                DefBranchRecord { key, list_first, list_count }
            })
            .collect();
        let first = pool_index(self.branches.len());
        self.branches.extend_from_slice(&records);
        (first, pool_index(records.len()))
    }

    fn push_params(&mut self, words: &[u32]) -> u32 {
        let first = pool_index(self.params.len());
        self.params.extend_from_slice(words);
        first
    }

    /// A client `std::map` of key to list: ascending keys, the last list of a repeated key (validation reports
    /// the repeat).
    fn map_entries(entries: &[EffectMapEntry]) -> Vec<(u32, &[EffectDefinition])> {
        let map: BTreeMap<u32, &[EffectDefinition]> =
            entries.iter().map(|entry| (entry.key, entry.children.as_slice())).collect();
        map.into_iter().collect()
    }

    fn node(&mut self, definition: &EffectDefinition, strings: &UoStringDictionary, report: &mut Report) -> u32 {
        let index = pool_index(self.nodes.len());
        self.nodes.push(DefNodeRecord::default());
        let type_key = u8::try_from(definition.type_key()).expect("client effect type keys fit u8");
        let mut record = DefNodeRecord { type_key, flags: 0, reserved: 0, param: NONE, first: 0, count: 0 };
        match definition {
            EffectDefinition::Particle(particle) => {
                let name = particle.name(strings).map(str::to_owned);
                let system_name = name.as_deref().and_then(|name| {
                    let normalized = normalize_resource_name(name);
                    if system_kind(&normalized).is_some() {
                        Some(normalized)
                    } else {
                        report.error(format!("particle name {name:?} is neither {CHAOS_EXTENSION} nor {NIF_EXTENSION}"));
                        None
                    }
                });
                let node = ParticleNode {
                    system: None,
                    offset_x: particle.offset_x,
                    offset_y: particle.offset_y,
                    camera_offset: particle.camera_offset,
                    scale: particle.scale,
                    duration: particle.duration,
                    attach_mode: particle.attach_mode,
                    field_3c: particle.field_3c,
                    looping: particle.looping,
                    hued: particle.hued,
                };
                record.param = self.push_params(&particle_params(&node));
                if let Some(system_name) = system_name {
                    self.particle_systems.push((record.param as usize, system_name));
                }
                if particle.looping {
                    record.flags |= NODE_FLAG_LOOPING;
                }
                if particle.hued {
                    record.flags |= NODE_FLAG_HUED;
                }
            }
            EffectDefinition::Sound(sound) => {
                record.param = self.push_params(&[sound.sound_id, sound.value_08.to_bits()]);
            }
            EffectDefinition::RoundRobin(round_robin) => {
                (record.first, record.count) = self.list(&round_robin.children, strings, report);
            }
            EffectDefinition::PositionOffset(offset) => {
                let offsets = offset.offsets.map(|direction| (direction.x, direction.y));
                record.param = self.push_params(&position_offset_params(&offsets));
            }
            EffectDefinition::Wait(wait) => {
                (record.first, record.count) = self.list(&wait.children, strings, report);
                record.param = self.push_params(&[wait.min_delay_ms, wait.max_delay_ms]);
                if wait.aligned {
                    record.flags |= NODE_FLAG_WAIT_ALIGNED;
                }
            }
            EffectDefinition::Lod(lod) => {
                (record.first, record.count) = self.branches(&Self::map_entries(&lod.levels), strings, report);
            }
            EffectDefinition::PropertyFlag(property_flag) => {
                let (default_first, default_count) = self.list(&property_flag.default_children, strings, report);
                (record.first, record.count) =
                    self.branches(&Self::map_entries(&property_flag.cases), strings, report);
                record.param = self.push_params(&property_flag_params(
                    property_flag.mask,
                    property_flag.property,
                    default_first,
                    default_count,
                ));
                if property_flag.mode == PropertyFlagMode::Property {
                    record.flags |= NODE_FLAG_PROPERTY_MODE;
                }
            }
            EffectDefinition::Earthquake(earthquake) => {
                record.param =
                    self.push_params(&earthquake_params(earthquake.amplitude, earthquake.duration, earthquake.frequency));
            }
            EffectDefinition::Random(random) => {
                let entries: Vec<(u32, &[EffectDefinition])> =
                    random.entries.iter().map(|entry| (entry.weight.to_bits(), entry.children.as_slice())).collect();
                (record.first, record.count) = self.branches(&entries, strings, report);
            }
            EffectDefinition::UserPreference(preference) => {
                (record.first, record.count) = self.list(&preference.children, strings, report);
                record.param = self.push_params(&[preference.mask]);
            }
        }
        self.nodes[index as usize] = record;
        index
    }
}

/// A pool length or index as stored (u32).
fn pool_index(value: usize) -> u32 {
    u32::try_from(value).expect("effects_ec pools fit u32")
}

/// The system kind of a normalized name, by extension.
fn system_kind(name: &str) -> Option<u8> {
    if name.ends_with(CHAOS_EXTENSION) {
        Some(SYSTEM_KIND_CHAOS)
    } else if name.ends_with(NIF_EXTENSION) {
        Some(SYSTEM_KIND_NIF)
    } else {
        None
    }
}

/// A dependency of a system, by normalized name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DepName {
    System(String),
    Texture(String),
}

/// A system being built.
#[derive(Default)]
struct SystemBuild {
    kind: u8,
    flags: u8,
    /// The encoded desc.
    payload: Option<Vec<u8>>,
    /// Direct dependencies, first use first, no repeats.
    deps: Vec<DepName>,
}

impl SystemBuild {
    fn add_dep(&mut self, dep: DepName) {
        if !self.deps.contains(&dep) {
            self.deps.push(dep);
        }
    }
}

/// The pixels of a `NiPixelData`: embedded in a NIF system, or the object of a `.nif` texture file.
enum PixelDataPixels {
    /// RGBA8 levels as `NiPixelData` holds them, level 0 first.
    Decoded { levels: Vec<(u32, u32, Vec<u8>)> },
    /// A pixel format not decoded (`NifTextureSource::Undecoded`).
    Undecoded { format: u32 },
}

impl PixelDataPixels {
    /// Every decodable level of `pixels`, or `Undecoded` when level 0 does not decode.
    fn of(pixels: &NiPixelData) -> Self {
        let levels = pixel_data_levels(pixels);
        if levels.is_empty() {
            Self::Undecoded { format: pixels.pixel_format.format.0 }
        } else {
            Self::Decoded { levels }
        }
    }
}

/// A texture being collected: who uses it and how.
#[derive(Default)]
struct TextureUse {
    /// `MipFlag` values of the `NiSourceTexture`s naming it; Chaos scripts count as `MIP_DEFAULT`.
    mip_flags: BTreeSet<u32>,
    embedded: Option<PixelDataPixels>,
    /// Systems using it (report only).
    users: BTreeSet<String>,
}

/// The name of embedded texture `index` of NIF `nif_name`.
pub fn embedded_texture_name(nif_name: &str, index: usize) -> String {
    format!("{nif_name}{EMBEDDED_TEXTURE_SEPARATOR}{index}")
}

/// Rewrites every texture source that is not external to `External { "<nif name>#<index>" }`, the name under which
/// the package stores the pixels (or the `UNDECODED` record). `nif_name` is the normalized system name.
pub fn externalize_embedded_textures(desc: &mut NifEffectDesc, nif_name: &str) {
    for texture in &mut desc.textures {
        if !matches!(texture.source, NifTextureSource::External { .. }) {
            texture.source = NifTextureSource::External { file_name: embedded_texture_name(nif_name, texture.index) };
        }
    }
}

/// Compiles a NIF system the way the package stores it: `particle_sim::nif::compile` named by the normalized
/// system name, embedded textures externalized.
pub fn compile_nif_for_package(nif: &NifFile, nif_name: &str) -> NifCompileOutput {
    let mut output = compile_nif(nif, nif_name);
    externalize_embedded_textures(&mut output.desc, nif_name);
    output
}

/// Compiles a Chaos system the way the package stores it, or the error lines.
pub fn compile_chaos_for_package(source: &[u8]) -> Result<(ChaosSystemDesc, Vec<String>), Vec<String>> {
    let report = compile_chaos_source(source);
    let warnings: Vec<String> =
        report.issues.iter().filter(|issue| !issue.is_error()).map(ToString::to_string).collect();
    let errors: Vec<String> = report.errors().map(ToString::to_string).collect();
    match report.desc {
        Some(desc) if errors.is_empty() => Ok((desc, warnings)),
        _ if errors.is_empty() => Err(vec!["no description produced".to_owned()]),
        _ => Err(errors),
    }
}

/// The systems and textures the definitions reach.
#[derive(Default)]
struct Closure {
    systems: BTreeMap<String, SystemBuild>,
    textures: BTreeMap<String, TextureUse>,
    /// The Chaos systems whose MESH sequences name each `.nif` (report only).
    mesh_users: BTreeMap<String, BTreeSet<String>>,
}

impl Closure {
    fn texture(&mut self, name: String, user: &str, mip_flag: MipFlag) -> &mut TextureUse {
        let texture = self.textures.entry(name).or_default();
        texture.mip_flags.insert(mip_flag.0);
        texture.users.insert(user.to_owned());
        texture
    }

    fn build_chaos(&mut self, name: &str, source: &[u8], system: &mut SystemBuild, report: &mut Report) -> Vec<String> {
        let (desc, warnings) = match compile_chaos_for_package(source) {
            Ok(compiled) => compiled,
            Err(errors) => {
                system.flags |= SYSTEM_FLAG_COMPILE_FAILED;
                for error in errors {
                    report.error(format!("system {name}: {error}"));
                }
                return Vec::new();
            }
        };
        if !warnings.is_empty() {
            system.flags |= SYSTEM_FLAG_COMPILE_WARNINGS;
            for warning in warnings {
                report.info(format!("system {name}: {warning}"));
            }
        }
        let mut meshes = Vec::new();
        for sequence in &desc.sequences {
            if sequence.resource.is_empty() {
                continue;
            }
            let resource = normalize_resource_name(&sequence.resource);
            match sequence.particle_type {
                ChaosParticleType::Point | ChaosParticleType::Axial => {
                    self.texture(resource.clone(), name, MipFlag::MIP_DEFAULT);
                    system.add_dep(DepName::Texture(resource));
                }
                ChaosParticleType::Mesh => {
                    if system_kind(&resource) == Some(SYSTEM_KIND_NIF) {
                        system.add_dep(DepName::System(resource.clone()));
                        meshes.push(resource);
                    } else {
                        report.error(format!("system {name}: MESH sequence {} names {resource:?}", sequence.name));
                    }
                }
                ChaosParticleType::Emitter => {
                    report.warning(format!(
                        "system {name}: EMITTER sequence {} ({resource}) is not simulated; not packed",
                        sequence.name
                    ));
                }
                _ => {}
            }
        }
        let payload = encode_chaos_desc(&desc);
        let decoded = decode_chaos_desc(&payload).map_err(|error| error.to_string());
        if decoded.as_ref() != Ok(&desc) {
            report.error(format!("system {name}: codec round trip differs ({:?})", decoded.err()));
            system.flags |= SYSTEM_FLAG_COMPILE_FAILED;
            return meshes;
        }
        system.payload = Some(payload);
        meshes
    }

    fn build_nif(&mut self, name: &str, source: &[u8], system: &mut SystemBuild, report: &mut Report) {
        let nif = match NifFile::read(source) {
            Ok(nif) => nif,
            Err(error) => {
                system.flags |= SYSTEM_FLAG_COMPILE_FAILED;
                report.error(format!("system {name}: does not parse: {error}"));
                return;
            }
        };
        let output = compile_nif(&nif, name);
        if !output.warnings.is_empty() {
            system.flags |= SYSTEM_FLAG_COMPILE_WARNINGS;
            for warning in &output.warnings {
                report.info(format!("system {name}: {warning}"));
            }
        }
        for (texture, &block) in output.desc.textures.iter().zip(&output.texture_blocks) {
            let source_texture = nif.blocks[block].get::<NiSourceTexture>().expect("compiled texture block");
            let mip_flag = source_texture.format_prefs.mip_mapped;
            match &texture.source {
                NifTextureSource::External { file_name } => {
                    let texture_name = normalize_resource_name(file_name);
                    self.texture(texture_name.clone(), name, mip_flag);
                    system.add_dep(DepName::Texture(texture_name));
                }
                source => {
                    let texture_name = embedded_texture_name(name, texture.index);
                    let pixels = match source {
                        NifTextureSource::Undecoded { format } => PixelDataPixels::Undecoded { format: *format },
                        _ => {
                            let data = nif.get(source_texture.src_pixel_data).expect("embedded pixel data");
                            PixelDataPixels::Decoded { levels: pixel_data_levels(data) }
                        }
                    };
                    self.texture(texture_name.clone(), name, mip_flag).embedded = Some(pixels);
                    system.add_dep(DepName::Texture(texture_name));
                }
            }
        }
        let mut desc = output.desc;
        externalize_embedded_textures(&mut desc, name);
        let payload = encode_nif_desc(&desc);
        let decoded = decode_nif_desc(&payload).map_err(|error| error.to_string());
        if decoded.as_ref() != Ok(&desc) {
            report.error(format!("system {name}: codec round trip differs ({:?})", decoded.err()));
            system.flags |= SYSTEM_FLAG_COMPILE_FAILED;
            return;
        }
        system.payload = Some(payload);
    }

    /// Builds every system named by the definitions and, transitively, the NIFs of Chaos MESH sequences.
    fn build_systems(&mut self, roots: &BTreeSet<String>, files: &EffectTexturePackage, report: &mut Report) -> eyre::Result<()> {
        let mut queue: Vec<(String, bool)> = roots.iter().rev().map(|name| (name.clone(), true)).collect();
        let bar = progress_bar(queue.len(), "compiling effect systems");
        while let Some((name, root)) = queue.pop() {
            if let Some(system) = self.systems.get_mut(&name) {
                if root {
                    system.flags |= SYSTEM_FLAG_DEFINITION_ROOT;
                }
                bar.inc(1);
                continue;
            }
            let kind = system_kind(&name).expect("system names have a system extension");
            let mut system = SystemBuild { kind, flags: if root { SYSTEM_FLAG_DEFINITION_ROOT } else { 0 }, ..Default::default() };
            match files.read(&name).wrap_err_with(|| format!("read {name}"))? {
                // Reported by `report_missing_files`, once per file.
                None => system.flags |= SYSTEM_FLAG_MISSING,
                Some(source) if kind == SYSTEM_KIND_CHAOS => {
                    let meshes = self.build_chaos(&name, &source, &mut system, report);
                    for mesh in meshes.into_iter().rev() {
                        self.mesh_users.entry(mesh.clone()).or_default().insert(name.clone());
                        if !self.systems.contains_key(&mesh) {
                            bar.set_length(bar.length().unwrap_or_default() + 1);
                            queue.push((mesh, false));
                        }
                    }
                }
                Some(source) => self.build_nif(&name, &source, &mut system, report),
            }
            self.systems.insert(name, system);
            bar.inc(1);
        }
        bar.finish_with_message(format!("effect systems compiled ({})", self.systems.len()));
        Ok(())
    }

    /// One warning per particle file missing from `EffectTexture.uop`, with every definition and Chaos MESH
    /// sequence naming it. A client data gap (the client plays nothing for it), so never an error; the system
    /// record keeps `SYSTEM_FLAG_MISSING`. `definition_referrers` holds the files the validation found missing.
    fn report_missing_files(&self, definition_referrers: &BTreeMap<String, BTreeSet<String>>, report: &mut Report) {
        let missing_systems =
            self.systems.iter().filter(|(_, system)| system.flags & SYSTEM_FLAG_MISSING != 0).map(|(name, _)| name);
        // Names the validation found missing that have no system (neither `.ems` nor `.nif`) are reported too.
        let unbuilt = definition_referrers.keys().filter(|name| !self.systems.contains_key(*name));
        let names: BTreeSet<&String> = missing_systems.chain(unbuilt).collect();
        for name in names {
            let definitions = definition_referrers.get(name).into_iter().flatten().cloned();
            let meshes = self.mesh_users.get(name).into_iter().flatten().map(|user| format!("system {user} MESH"));
            let referrers: Vec<String> = definitions.chain(meshes).collect();
            report.warning(format!(
                "particle file {name}: not in {EFFECT_TEXTURE_UOP_NAME}, the client plays nothing; named by {} ({})",
                referrers.len(),
                referrers.join(", ")
            ));
        }
    }
}

/// Every decodable level of `NiPixelData`, level 0 first (stops at the first level that does not decode).
fn pixel_data_levels(pixels: &NiPixelData) -> Vec<(u32, u32, Vec<u8>)> {
    (0..pixels.mipmaps.len()).map_while(|level| decode_pixel_level(pixels, level)).collect()
}

fn progress_bar(length: usize, message: &str) -> ProgressBar {
    let bar = ProgressBar::new(length as u64);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
            .expect("progress template")
            .progress_chars("#>-"),
    );
    bar.set_message(message.to_owned());
    bar
}

/// A decoded source texture.
enum SourcePixels {
    /// RGBA8 levels, level 0 first. `generatable`: the SDK would generate a chain for it (an uncompressed source).
    Rgba { levels: Vec<Vec<u8>>, generatable: bool },
    /// BC1 or BC3 blocks of `mip_count` levels, concatenated (a DXT1 / DXT5 DDS passed through).
    Blocks { format: u8, data: Vec<u8>, mip_count: u32 },
}

struct SourceImage {
    width: u32,
    height: u32,
    pixels: SourcePixels,
    opaque: bool,
    source: u8,
}

fn is_opaque(rgba: &[u8]) -> bool {
    rgba.chunks_exact(RGBA8_BYTES_PER_PIXEL).all(|pixel| pixel[ALPHA_CHANNEL] == OPAQUE_ALPHA)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + size_of::<u32>()].try_into().expect("u32 bytes"))
}

/// Decodes a Targa file as Gamebryo 2.6 `NiTGAReader` does (`CoreLibs/NiMain/NiTGAReader.cpp`), to RGBA8 rows, top
/// row first:
/// - types 1/9 (colour mapped), 2/10 (true colour), 3/11 (greyscale), RLE runs may cross rows (`LoadRLERow`);
/// - 8-bit greyscale expands to R = G = B with alpha 255 (`Unpack8BitSourceRowGray`, RGB24 destination, `:176-189`);
/// - 15/16-bit pixels: 5-5-5 without bit replication, the top bit ignored, alpha 255 (`Unpack16BitSourceRowRGB`);
/// - 24-bit BGR, alpha 255; 32-bit BGRA (`Unpack24BitSourceRow`, `Unpack32BitSourceRow`): the destination is RGBA32
///   only for 32-bit pixels (`ReadHeader`, `:453-466`);
/// - colour map entries of 8 (greyscale), 15/16, 24 and 32 bits; alpha only from 32-bit entries (`GetColormap`);
///   4-bit colour-mapped pixels (`Unpack4BitSourceRowPal`) are not supported;
/// - rows flipped unless attribute bit 0x20 is set; the right-to-left bit is ignored (`ReadHeader`).
fn decode_tga(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if bytes.len() < TGA_HEADER_SIZE {
        return Err(format!("{} bytes, shorter than the TGA header", bytes.len()));
    }
    let id_length = usize::from(bytes[0]);
    let color_map_type = bytes[1];
    let image_type = bytes[2];
    let first_index = usize::from(read_u16(bytes, 3));
    let palette_length = usize::from(read_u16(bytes, 5));
    let entry_bits = bytes[7];
    let width = usize::from(read_u16(bytes, 12));
    let height = usize::from(read_u16(bytes, 14));
    let pixel_bits = bytes[16];
    let attributes = bytes[17];
    let (color_mapped, rle) = match image_type {
        TGA_TYPE_MAP => (true, false),
        TGA_TYPE_RGB | TGA_TYPE_MONO => (false, false),
        TGA_TYPE_RLE_MAP => (true, true),
        TGA_TYPE_RLE_RGB | TGA_TYPE_RLE_MONO => (false, true),
        other => return Err(format!("TGA image type {other}")),
    };
    let bytes_per_pixel = match pixel_bits {
        TGA_BITS_8 => 1,
        TGA_BITS_15 | TGA_BITS_16 => 2,
        TGA_BITS_24 => 3,
        TGA_BITS_32 => 4,
        other => return Err(format!("{other}-bit TGA pixels")),
    };
    if width == 0 || height == 0 {
        return Err(format!("empty TGA {width}x{height}"));
    }
    let mut cursor = TGA_HEADER_SIZE + id_length;
    let mut take = |len: usize| -> Result<&[u8], String> {
        let slice = bytes.get(cursor..cursor + len).ok_or_else(|| "truncated TGA".to_owned())?;
        cursor += len;
        Ok(slice)
    };
    let mut palette: Vec<[u8; 4]> = Vec::new();
    if color_map_type != 0 {
        if first_index + palette_length + 1 >= TGA_MAX_COLORS {
            return Err(format!("TGA colour map {first_index}+{palette_length} too large"));
        }
        palette = match entry_bits {
            TGA_BITS_8 => take(palette_length)?.iter().map(|&grey| [grey, grey, grey, OPAQUE_ALPHA]).collect(),
            TGA_BITS_15 | TGA_BITS_16 => take(palette_length * 2)?.chunks_exact(2).map(|p| rgb555(p[0], p[1])).collect(),
            TGA_BITS_24 => take(palette_length * 3)?.chunks_exact(3).map(|p| [p[2], p[1], p[0], OPAQUE_ALPHA]).collect(),
            TGA_BITS_32 => take(palette_length * 4)?.chunks_exact(4).map(|p| [p[2], p[1], p[0], p[3]]).collect(),
            // `GetColormap` reads nothing for other entry sizes.
            _ => Vec::new(),
        };
    } else if color_mapped {
        return Err("colour-mapped TGA without a colour map".to_owned());
    }
    // Colour map entries are indexed from the first index: 8-bit indices wrap (`Unpack8BitSourceRowPal` stores
    // `(unsigned char)(index - first)`), 16-bit ones do not (`Unpack16BitSourceRowIndexed`).
    let palette_entry = |entry: Option<usize>| -> Result<[u8; 4], String> {
        entry
            .and_then(|entry| palette.get(entry).copied())
            .ok_or_else(|| format!("TGA colour map entry {entry:?} outside the colour map"))
    };
    let unpack = |raw: &[u8]| -> Result<[u8; 4], String> {
        Ok(match raw.len() {
            1 if color_mapped => palette_entry(Some(usize::from(raw[0].wrapping_sub(first_index as u8))))?,
            1 => [raw[0], raw[0], raw[0], OPAQUE_ALPHA],
            2 if color_mapped => {
                palette_entry((usize::from(raw[0]) | (usize::from(raw[1]) << u8::BITS)).checked_sub(first_index))?
            }
            2 => rgb555(raw[0], raw[1]),
            3 => [raw[2], raw[1], raw[0], OPAQUE_ALPHA],
            _ => [raw[2], raw[1], raw[0], raw[3]],
        })
    };
    let row_bytes = width * RGBA8_BYTES_PER_PIXEL;
    let mut rgba = vec![0u8; row_bytes * height];
    let mut run_left = 0usize;
    let mut run_pixel: Option<Vec<u8>> = None;
    for row in 0..height {
        let dest_row = if attributes & TGA_ATTR_TOP_ORIGIN != 0 { row } else { height - 1 - row };
        let dest = &mut rgba[dest_row * row_bytes..(dest_row + 1) * row_bytes];
        let mut column = 0;
        while column < width {
            let raw: Vec<u8> = if rle {
                if run_left == 0 {
                    let tag = take(1)?[0];
                    if tag & TGA_RLE_RUN_BIT != 0 {
                        run_left = usize::from(tag) - TGA_RLE_RUN_BIAS;
                        run_pixel = Some(take(bytes_per_pixel)?.to_vec());
                    } else {
                        run_left = usize::from(tag) + 1;
                        run_pixel = None;
                    }
                }
                run_left -= 1;
                match &run_pixel {
                    Some(pixel) => pixel.clone(),
                    None => take(bytes_per_pixel)?.to_vec(),
                }
            } else {
                take(bytes_per_pixel)?.to_vec()
            };
            let pixel = unpack(&raw)?;
            dest[column * RGBA8_BYTES_PER_PIXEL..(column + 1) * RGBA8_BYTES_PER_PIXEL].copy_from_slice(&pixel);
            column += 1;
        }
    }
    Ok((width as u32, height as u32, rgba))
}

/// A 15/16-bit pixel or colour map entry as `NiTGAReader` expands it (low 3 bits zero, alpha 255).
fn rgb555(low: u8, high: u8) -> [u8; 4] {
    const RED_MASK: u8 = 0x7C;
    const GREEN_HIGH_MASK: u8 = 0x03;
    const GREEN_LOW_MASK: u8 = 0xE0;
    const BLUE_MASK: u8 = 0x1F;
    [
        (high & RED_MASK) << 1,
        ((high & GREEN_HIGH_MASK) << 6) + ((low & GREEN_LOW_MASK) >> 2),
        (low & BLUE_MASK) << 3,
        OPAQUE_ALPHA,
    ]
}

/// Decodes a DDS file. DXT1 / DXT5 with a 4-aligned size are passed through as BC1 / BC3 blocks; anything else is
/// decoded to RGBA8. The levels kept follow `NiDDSReader::ReadHeader`: the file's mip count when `DDSCAPS_COMPLEX`
/// and `DDSCAPS_MIPMAP` are set and the count is not 1, else level 0 only.
fn decode_dds(bytes: &[u8]) -> Result<SourceImage, String> {
    if bytes.len() < DDS_HEADER_END || &bytes[..DDS_MAGIC.len()] != DDS_MAGIC {
        return Err("not a DDS file".to_owned());
    }
    let caps = read_u32(bytes, DDS_CAPS_OFFSET);
    let file_mips = read_u32(bytes, DDS_MIP_COUNT_OFFSET);
    let levels = if caps & DDSCAPS_COMPLEX != 0 && caps & DDSCAPS_MIPMAP != 0 && file_mips != 1 {
        file_mips.max(1)
    } else {
        1
    };
    let mut decoder = dds::Decoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let size = decoder.main_size();
    let (width, height) = (size.width, size.height);
    let format = decoder.format();
    let mut rgba_levels = Vec::new();
    for _ in 0..levels {
        let size = decoder.surface_info().ok_or_else(|| "fewer surfaces than mip levels".to_owned())?.size();
        let len = dds::ColorFormat::RGBA_U8.buffer_size(size).ok_or_else(|| "DDS too large".to_owned())?;
        let mut rgba = vec![0u8; len];
        let view = dds::ImageViewMut::new(&mut rgba, size, dds::ColorFormat::RGBA_U8)
            .ok_or_else(|| "DDS buffer".to_owned())?;
        decoder.read_surface(view).map_err(|error| error.to_string())?;
        rgba_levels.push(rgba);
    }
    let opaque = is_opaque(&rgba_levels[0]);
    let block_format = match format {
        dds::Format::BC1_UNORM => Some(TEXTURE_FORMAT_BC1),
        dds::Format::BC3_UNORM => Some(TEXTURE_FORMAT_BC3),
        _ => None,
    };
    let aligned = width % BC_BLOCK_EDGE == 0 && height % BC_BLOCK_EDGE == 0;
    let pixels = match block_format {
        Some(format) if aligned => {
            let offset = DDS_HEADER_END
                + if &bytes[DDS_FOURCC_OFFSET..DDS_FOURCC_OFFSET + DDS_FOURCC_DX10.len()] == DDS_FOURCC_DX10 {
                    DDS_DXT10_HEADER_SIZE
                } else {
                    0
                };
            let len = texture_payload_len(format, width, height, levels).expect("block format");
            let data = bytes.get(offset..offset + len).ok_or_else(|| "truncated DDS blocks".to_owned())?.to_vec();
            SourcePixels::Blocks { format, data, mip_count: levels }
        }
        // A compressed source keeps its levels: the SDK generates chains only for RGB(A) destinations.
        Some(_) => SourcePixels::Rgba { levels: rgba_levels, generatable: false },
        None => SourcePixels::Rgba { levels: rgba_levels, generatable: true },
    };
    Ok(SourceImage { width, height, pixels, opaque, source: TEXTURE_SOURCE_DDS })
}

/// The full mip chain of a power-of-two RGBA8 image as `NiDevImageConverter::GenerateMipmapLevels` builds it
/// (`CoreLibs/NiMain/NiDevImageConverter.cpp:1108-1290`): each level from the previous one by a truncating 2x2 box
/// (`(a + b + c + d) >> 2`), or `(a + b) >> 1` when the previous level is one pixel wide or high, down to 1x1.
fn generate_mips(width: u32, height: u32, level0: Vec<u8>) -> Vec<Vec<u8>> {
    let mut levels = vec![level0];
    let (mut fine_width, mut fine_height) = (width as usize, height as usize);
    while fine_width > 1 || fine_height > 1 {
        let fine = levels.last().expect("level 0");
        let (coarse_width, coarse_height) = ((fine_width / 2).max(1), (fine_height / 2).max(1));
        let pixel = |x: usize, y: usize| &fine[(y * fine_width + x) * RGBA8_BYTES_PER_PIXEL..][..RGBA8_BYTES_PER_PIXEL];
        let mut coarse = Vec::with_capacity(coarse_width * coarse_height * RGBA8_BYTES_PER_PIXEL);
        for y in 0..coarse_height {
            for x in 0..coarse_width {
                for channel in 0..RGBA8_BYTES_PER_PIXEL {
                    let value = |x: usize, y: usize| u32::from(pixel(x, y)[channel]);
                    let average = if fine_width == 1 {
                        (value(0, 2 * y) + value(0, 2 * y + 1)) >> 1
                    } else if fine_height == 1 {
                        (value(2 * x, 0) + value(2 * x + 1, 0)) >> 1
                    } else {
                        (value(2 * x, 2 * y) + value(2 * x + 1, 2 * y) + value(2 * x, 2 * y + 1) + value(2 * x + 1, 2 * y + 1))
                            >> 2
                    };
                    coarse.push(average as u8);
                }
            }
        }
        levels.push(coarse);
        (fine_width, fine_height) = (coarse_width, coarse_height);
    }
    levels
}

/// Reads a `.nif` texture file as Gamebryo 2.6 `NiNIFImageReader::ReadFile` does
/// (`CoreLibs/NiMain/NiNIFImageReader.cpp:42-70`): the stream must hold exactly one top object, and it must be a
/// `NiPixelData`. Its stored levels and pixel format are used as they are: `NiDevImageConverter::ConvertPixelData`
/// generates a chain only for a single-level source (`NiDevImageConverter.cpp:453-470`).
fn read_nif_texture(bytes: &[u8]) -> Result<PixelDataPixels, String> {
    let nif = NifFile::read(bytes).map_err(|error| format!("NIF texture does not parse: {error}"))?;
    let [root] = nif.roots.as_slice() else {
        return Err(format!("NIF texture with {} top objects; NiNIFImageReader reads exactly one", nif.roots.len()));
    };
    let pixels = root
        .index()
        .and_then(|index| nif.blocks.get(index))
        .and_then(|block| block.get::<NiPixelData>())
        .ok_or_else(|| "the top object of the NIF texture is not a NiPixelData".to_owned())?;
    Ok(PixelDataPixels::of(pixels))
}

/// The image of `NiPixelData` pixels read from `source` (`TEXTURE_SOURCE_*`): every level when the sizes halve level
/// by level, else level 0 only. `None` when the pixel format is not decoded: the record is flagged `UNDECODED` and
/// the report warns.
fn pixel_data_image(
    name: &str,
    pixels: &PixelDataPixels,
    source: u8,
    record: &mut TextureRecord,
    report: &mut Report,
) -> Option<SourceImage> {
    record.source = source;
    let levels = match pixels {
        PixelDataPixels::Undecoded { format } => {
            record.flags |= TEXTURE_FLAG_UNDECODED;
            report.warning(format!("texture {name}: NiPixelData pixel format {format} is not decoded; no payload"));
            return None;
        }
        PixelDataPixels::Decoded { levels } => levels,
    };
    let (width, height, _) = levels[0];
    let consistent =
        levels.iter().enumerate().all(|(level, (w, h, _))| (*w, *h) == mip_extent(width, height, level as u32));
    let mut rgba: Vec<Vec<u8>> = levels.iter().map(|(_, _, rgba)| rgba.clone()).collect();
    if !consistent {
        report.warning(format!("texture {name}: NiPixelData mip sizes are not a halving chain; level 0 only"));
        rgba.truncate(1);
    }
    let opaque = is_opaque(&rgba[0]);
    Some(SourceImage { width, height, pixels: SourcePixels::Rgba { levels: rgba, generatable: true }, opaque, source })
}

/// A texture record and its payload.
struct BuiltTexture {
    record: TextureRecord,
    payload: Option<Vec<u8>>,
    generated_mips: bool,
}

/// Reads, decodes and encodes one texture.
fn build_texture(
    name: &str,
    texture: &TextureUse,
    files: &EffectTexturePackage,
    options: &EffectsEcPackOptions,
    report: &mut Report,
) -> eyre::Result<BuiltTexture> {
    let mut record = TextureRecord::default();
    let users = texture.users.iter().cloned().collect::<Vec<_>>().join(", ");
    let decoded: Result<SourceImage, String> = match &texture.embedded {
        Some(pixels) => {
            record.flags |= TEXTURE_FLAG_EMBEDDED;
            match pixel_data_image(name, pixels, TEXTURE_SOURCE_NIF_PIXEL_DATA, &mut record, report) {
                Some(image) => Ok(image),
                None => return Ok(BuiltTexture { record, payload: None, generated_mips: false }),
            }
        }
        None => match files.read(name).wrap_err_with(|| format!("read {name}"))? {
            None => {
                record.flags |= TEXTURE_FLAG_MISSING;
                report.warning(format!("texture {name} (used by {users}): not in {EFFECT_TEXTURE_UOP_NAME}"));
                return Ok(BuiltTexture { record, payload: None, generated_mips: false });
            }
            Some(bytes) if bytes.starts_with(DDS_MAGIC) => {
                record.source = TEXTURE_SOURCE_DDS;
                decode_dds(&bytes)
            }
            Some(bytes) if name.ends_with(TGA_EXTENSION) => {
                record.source = TEXTURE_SOURCE_TGA;
                decode_tga(&bytes).map(|(width, height, rgba)| SourceImage {
                    width,
                    height,
                    opaque: is_opaque(&rgba),
                    pixels: SourcePixels::Rgba { levels: vec![rgba], generatable: true },
                    source: TEXTURE_SOURCE_TGA,
                })
            }
            Some(bytes) if name.ends_with(NIF_EXTENSION) => {
                record.source = TEXTURE_SOURCE_NIF_FILE;
                match read_nif_texture(&bytes) {
                    Ok(pixels) => match pixel_data_image(name, &pixels, TEXTURE_SOURCE_NIF_FILE, &mut record, report) {
                        Some(image) => Ok(image),
                        None => return Ok(BuiltTexture { record, payload: None, generated_mips: false }),
                    },
                    Err(error) => Err(error),
                }
            }
            Some(_) => Err(format!("neither a DDS, a {TGA_EXTENSION} nor a {NIF_EXTENSION} file")),
        },
    };
    let image = match decoded {
        Ok(image) if image.width == 0 || image.height == 0 || image.width > MAX_TEXTURE_EDGE || image.height > MAX_TEXTURE_EDGE => {
            Err(format!("size {}x{} outside 1..={MAX_TEXTURE_EDGE}", image.width, image.height))
        }
        other => other,
    };
    let image = match image {
        Ok(image) => image,
        Err(error) => {
            record.flags |= TEXTURE_FLAG_DECODE_FAILED;
            report.error(format!("texture {name} (used by {users}): {error}; no payload"));
            return Ok(BuiltTexture { record, payload: None, generated_mips: false });
        }
    };
    record.source = image.source;
    record.width = image.width as u16;
    record.height = image.height as u16;
    if image.opaque {
        record.flags |= TEXTURE_FLAG_OPAQUE;
    }

    let yes = texture.mip_flags.contains(&MipFlag::YES.0);
    let no = texture.mip_flags.contains(&MipFlag::NO.0);
    let default = texture.mip_flags.contains(&MipFlag::MIP_DEFAULT.0);
    let wants_mips = yes || (default && MIPMAP_BY_DEFAULT);
    if wants_mips && (no || (default && !MIPMAP_BY_DEFAULT)) {
        report.info(format!("texture {name} (used by {users}): conflicting mipmap preferences; mipmapped"));
    }
    let (width, height) = (image.width, image.height);
    let mut generated_mips = false;
    let (format, mip_count, payload) = match image.pixels {
        SourcePixels::Blocks { format, data, mip_count } => (format, mip_count, data),
        SourcePixels::Rgba { mut levels, generatable } => {
            if levels.len() == 1 && wants_mips && generatable && width.is_power_of_two() && height.is_power_of_two() {
                levels = generate_mips(width, height, levels.pop().expect("level 0"));
                generated_mips = true;
            }
            let mip_count = levels.len() as u32;
            if options.bc7 && width % BC_BLOCK_EDGE == 0 && height % BC_BLOCK_EDGE == 0 {
                let mut blocks = Vec::new();
                for (level, rgba) in levels.iter().enumerate() {
                    let (w, h) = mip_extent(width, height, level as u32);
                    let extent = ImageExtent::new(w, h).map_err(|error| eyre!("{name}: {error:?}"))?;
                    let encoded = encode_to_bc7(rgba, extent, RawImageFormat::Rgba8888, preferred_bc7_encoder_backend())
                        .map_err(|error| eyre!("{name}: BC7 encode: {error:?}"))?;
                    blocks.extend_from_slice(encoded.blocks());
                }
                (TEXTURE_FORMAT_BC7, mip_count, blocks)
            } else {
                (TEXTURE_FORMAT_RGBA8, mip_count, levels.concat())
            }
        }
    };
    ensure!((mip_count as usize) <= MAX_MIP_LEVELS, "texture {name}: {mip_count} mip levels");
    if mip_count > 1 && !generated_mips {
        record.flags |= TEXTURE_FLAG_AUTHORED_MIPS;
    }
    ensure!(
        texture_payload_len(format, width, height, mip_count) == Some(payload.len()),
        "texture {name}: payload of {} bytes does not match {width}x{height} format {format} with {mip_count} levels",
        payload.len()
    );
    record.format = format;
    record.mip_count = mip_count as u8;
    Ok(BuiltTexture { record, payload: Some(payload), generated_mips })
}

/// Builds the package in memory.
fn build_effects_ec(source_dirs: &[PathBuf], options: &EffectsEcPackOptions) -> eyre::Result<BuiltPackage> {
    let sources = Sources::load(source_dirs)?;
    let mut report = Report::default();
    let definitions = load_definitions(&sources, &mut report)?;

    let mut tables = DefinitionTables::default();
    let mut collection = Vec::new();
    for (collection_id, list) in &definitions.collection {
        let (list_first, list_count) = tables.list(list, &sources.strings, &mut report);
        collection.push(CollectionRecord { collection_id: *collection_id, list_first, list_count });
    }
    let mut tileart = Vec::new();
    for (tile_id, list) in &definitions.tileart {
        let (list_first, list_count) = tables.list(list, &sources.strings, &mut report);
        tileart.push(TileArtRecord { tile_id: *tile_id, list_first, list_count });
    }
    let mut frames = Vec::new();
    for ((sequence_id, action_id, frame), list) in &definitions.frames {
        let (list_first, list_count) = tables.list(list, &sources.strings, &mut report);
        frames.push(SequenceFrameRecord { sequence_id: *sequence_id, action_id: *action_id, frame: *frame, list_first, list_count });
    }

    let roots: BTreeSet<String> = tables.particle_systems.iter().map(|(_, name)| name.clone()).collect();
    let mut closure = Closure::default();
    closure.build_systems(&roots, &sources.effect_files, &mut report)?;
    closure.report_missing_files(&definitions.missing_particle_files, &mut report);
    let system_index: HashMap<&str, u32> =
        closure.systems.keys().enumerate().map(|(index, name)| (name.as_str(), pool_index(index))).collect();
    for (slot, name) in &tables.particle_systems {
        tables.params[*slot] = system_index[name.as_str()];
    }

    let bar = progress_bar(closure.textures.len(), "decoding and encoding effect textures");
    let mut textures = Vec::with_capacity(closure.textures.len());
    for (name, texture) in &closure.textures {
        textures.push(build_texture(name, texture, &sources.effect_files, options, &mut report)?);
        bar.inc(1);
    }
    bar.finish_with_message(format!("effect textures encoded ({})", textures.len()));
    let texture_index: HashMap<&str, u32> =
        closure.textures.keys().enumerate().map(|(index, name)| (name.as_str(), pool_index(index))).collect();

    let mut strings: Vec<String> = Vec::new();
    let mut string_index: HashMap<String, u32> = HashMap::new();
    let mut intern = |name: &str| -> u32 {
        *string_index.entry(name.to_owned()).or_insert_with(|| {
            strings.push(name.to_owned());
            pool_index(strings.len() - 1)
        })
    };
    let mut deps = Vec::new();
    let mut system_records = Vec::with_capacity(closure.systems.len());
    for (name, system) in &closure.systems {
        let dep_first = pool_index(deps.len());
        for dep in &system.deps {
            deps.push(match dep {
                DepName::System(target) => system_index[target.as_str()] | DEP_SYSTEM_BIT,
                DepName::Texture(target) => texture_index[target.as_str()],
            });
        }
        let dep_count = u16::try_from(system.deps.len()).map_err(|_| eyre!("system {name}: too many dependencies"))?;
        system_records.push(SystemRecord { name: intern(name), kind: system.kind, flags: system.flags, dep_count, dep_first });
    }
    let mut texture_records = Vec::with_capacity(textures.len());
    for (name, texture) in closure.textures.keys().zip(&mut textures) {
        texture.record.name = intern(name);
        texture_records.push(texture.record);
    }

    let mut summary = EffectsEcPackSummary {
        collection_entries: collection.len(),
        tileart_records: tileart.len(),
        sequence_frames: frames.len(),
        nodes: tables.nodes.len(),
        textures: textures.len(),
        ..Default::default()
    };
    for system in closure.systems.values() {
        match system.kind {
            SYSTEM_KIND_CHAOS => summary.chaos_systems += 1,
            _ => summary.nif_systems += 1,
        }
        summary.systems_missing += usize::from(system.flags & SYSTEM_FLAG_MISSING != 0);
        summary.systems_failed += usize::from(system.flags & SYSTEM_FLAG_COMPILE_FAILED != 0);
        summary.systems_with_warnings += usize::from(system.flags & SYSTEM_FLAG_COMPILE_WARNINGS != 0);
        summary.system_bytes += system.payload.as_ref().map_or(0, Vec::len);
    }
    for texture in &textures {
        let record = texture.record;
        if record.format != TEXTURE_FORMAT_NONE {
            *summary.textures_by_format.entry(record.format).or_default() += 1;
        }
        summary.textures_missing += usize::from(record.flags & TEXTURE_FLAG_MISSING != 0);
        summary.textures_failed += usize::from(record.flags & (TEXTURE_FLAG_DECODE_FAILED | TEXTURE_FLAG_UNDECODED) != 0);
        summary.textures_generated_mips += usize::from(texture.generated_mips);
        summary.textures_authored_mips += usize::from(record.flags & TEXTURE_FLAG_AUTHORED_MIPS != 0);
        summary.texture_bytes += texture.payload.as_ref().map_or(0, Vec::len);
    }
    let mut unreferenced = BTreeSet::new();
    for entry in sources.effect_files.entries() {
        let Some(path) = entry.name.as_deref() else { continue };
        let name = normalize_resource_name(path);
        let is_resource = RESOURCE_EXTENSIONS.iter().any(|extension| name.ends_with(extension));
        if is_resource && !closure.systems.contains_key(&name) && !closure.textures.contains_key(&name) {
            unreferenced.insert(name);
        }
    }
    summary.unreferenced_resources = unreferenced.len();
    for name in unreferenced {
        report.info(format!("unreferenced resource {name}: nothing reaches it; not packed"));
    }
    summary.findings = report.findings;

    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    let mut add = |path: String, data_type: DataType, compression: CompressionFlag, (width, height): (u32, u32), data: Vec<u8>| {
        builder.add_owned_file(AddOwnedFileRequest {
            data_type: data_type as u8,
            compression,
            width,
            height,
            virtual_path: Some(path),
            path_hash64: None,
            id: None,
            data,
        })
    };
    let metadata = [
        (PACKAGE_ENTRY_PATH, encode_manifest(PACKAGE_MAGIC, &[PackageRecord::CURRENT])),
        (STRINGS_ENTRY_PATH, encode_strings(&strings)),
        (COLLECTION_ENTRY_PATH, encode_manifest(COLLECTION_MAGIC, &collection)),
        (TILEART_ENTRY_PATH, encode_manifest(TILEART_MAGIC, &tileart)),
        (SEQUENCE_FRAMES_ENTRY_PATH, encode_manifest(SEQUENCE_FRAMES_MAGIC, &frames)),
        (DEF_NODES_ENTRY_PATH, encode_manifest(DEF_NODES_MAGIC, &tables.nodes)),
        (DEF_LISTS_ENTRY_PATH, encode_manifest(DEF_LISTS_MAGIC, &tables.lists)),
        (DEF_BRANCHES_ENTRY_PATH, encode_manifest(DEF_BRANCHES_MAGIC, &tables.branches)),
        (DEF_PARAMS_ENTRY_PATH, encode_manifest(DEF_PARAMS_MAGIC, &tables.params)),
        (SYSTEMS_ENTRY_PATH, encode_manifest(SYSTEMS_MAGIC, &system_records)),
        (DEPS_ENTRY_PATH, encode_manifest(DEPS_MAGIC, &deps)),
        (TEXTURES_ENTRY_PATH, encode_manifest(TEXTURES_MAGIC, &texture_records)),
    ];
    for (path, data) in metadata {
        add(path.to_owned(), DataType::Metadata, options.compression, (0, 0), data)?;
    }
    for (index, system) in closure.systems.into_values().enumerate() {
        if let Some(payload) = system.payload {
            add(system_entry_path(index, system.kind), DataType::Effect, CompressionFlag::Auto, (0, 0), payload)?;
        }
    }
    for (index, texture) in textures.into_iter().enumerate() {
        if let Some(payload) = texture.payload {
            let size = (u32::from(texture.record.width), u32::from(texture.record.height));
            add(texture_entry_path(index), DataType::Texture, options.compression, size, payload)?;
        }
    }
    Ok(BuiltPackage { builder, summary })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamebryo_nif::{
        Block, NiNode, NiPixelDataMipmap, NiPixelFormat, NiPixelFormatComponent, PixelFormatFormat, Ref,
    };
    use uocf::enhanced::effect_definition::{LodEffect, ParticleEffect, SoundEffect};

    /// Bytes of an RGB24 pixel in a TGA file (B, G, R).
    const BGR: usize = 3;
    /// Header of a DDS file in these tests: `dwSize` of the header and pixel format, `DDSD_CAPS | DDSD_HEIGHT |
    /// DDSD_WIDTH | DDSD_PIXELFORMAT`, `DDPF_FOURCC`, `DDSCAPS_TEXTURE`.
    const DDS_HEADER_SIZE: u32 = 124;
    const DDS_PIXEL_FORMAT_SIZE: u32 = 32;
    const DDS_REQUIRED_FLAGS: u32 = 0x1007;
    const DDPF_FOURCC: u32 = 0x4;
    const DDSCAPS_TEXTURE: u32 = 0x1000;
    const DDS_SIZE_FIELD_OFFSET: usize = 4;
    const DDS_FLAGS_OFFSET: usize = 8;
    const DDS_HEIGHT_OFFSET: usize = 12;
    const DDS_WIDTH_OFFSET: usize = 16;
    const DDS_PIXEL_FORMAT_OFFSET: usize = 76;
    const DDS_PIXEL_FORMAT_FLAGS_OFFSET: usize = 80;
    /// A DXT1 block: colour 0 white, colour 1 black, every index 0 (all white, opaque).
    const WHITE_DXT1_BLOCK: [u8; 8] = [0xFF, 0xFF, 0, 0, 0, 0, 0, 0];

    fn tga_header(image_type: u8, width: u16, height: u16, pixel_bits: u8, attributes: u8) -> Vec<u8> {
        let mut header = vec![0u8; TGA_HEADER_SIZE];
        header[2] = image_type;
        header[12..14].copy_from_slice(&width.to_le_bytes());
        header[14..16].copy_from_slice(&height.to_le_bytes());
        header[16] = pixel_bits;
        header[17] = attributes;
        header
    }

    fn rgba(pixels: &[[u8; 4]]) -> Vec<u8> {
        pixels.concat()
    }

    #[test]
    fn tga_rows_are_flipped_unless_top_origin_and_rgb_gets_opaque_alpha() {
        let red = [0, 0, 255];
        let green = [0, 255, 0];
        let mut file = tga_header(TGA_TYPE_RGB, 1, 2, TGA_BITS_24, 0);
        file.extend([red, green].concat());
        let (width, height, pixels) = decode_tga(&file).unwrap();
        assert_eq!((width, height), (1, 2));
        assert_eq!(pixels, rgba(&[[0, 255, 0, 255], [255, 0, 0, 255]]), "bottom origin: the first row is the bottom");

        file[17] = TGA_ATTR_TOP_ORIGIN;
        assert_eq!(decode_tga(&file).unwrap().2, rgba(&[[255, 0, 0, 255], [0, 255, 0, 255]]));
    }

    #[test]
    fn tga_32_bit_keeps_alpha_and_greyscale_expands_to_opaque_grey() {
        let mut file = tga_header(TGA_TYPE_RGB, 1, 1, TGA_BITS_32, TGA_ATTR_TOP_ORIGIN);
        file.extend([10, 20, 30, 40]);
        assert_eq!(decode_tga(&file).unwrap().2, vec![30, 20, 10, 40]);

        let mut file = tga_header(TGA_TYPE_MONO, 2, 1, TGA_BITS_8, TGA_ATTR_TOP_ORIGIN);
        file.extend([7, 200]);
        assert_eq!(decode_tga(&file).unwrap().2, rgba(&[[7, 7, 7, 255], [200, 200, 200, 255]]));
    }

    #[test]
    fn tga_rle_runs_cross_rows() {
        let mut file = tga_header(TGA_TYPE_RLE_RGB, 3, 2, TGA_BITS_24, TGA_ATTR_TOP_ORIGIN);
        // A run of 4 blue pixels (crosses into row 1), then 2 raw pixels.
        file.push(TGA_RLE_RUN_BIT | 3);
        file.extend([255, 0, 0]);
        file.push(1);
        file.extend([0, 255, 0, 0, 0, 255]);
        let blue = [0, 0, 255, 255];
        let expected = rgba(&[blue, blue, blue, blue, [0, 255, 0, 255], [255, 0, 0, 255]]);
        assert_eq!(decode_tga(&file).unwrap().2, expected);
        assert_eq!(file.len(), TGA_HEADER_SIZE + 2 + BGR * 3);
    }

    #[test]
    fn tga_colour_map_entries_index_from_the_first_entry() {
        let mut file = tga_header(TGA_TYPE_MAP, 2, 1, TGA_BITS_8, TGA_ATTR_TOP_ORIGIN);
        file[1] = 1;
        file[3..5].copy_from_slice(&1u16.to_le_bytes());
        file[5..7].copy_from_slice(&2u16.to_le_bytes());
        file[7] = TGA_BITS_24;
        file.extend([1, 2, 3, 4, 5, 6]);
        file.extend([2, 1]);
        assert_eq!(decode_tga(&file).unwrap().2, rgba(&[[6, 5, 4, 255], [3, 2, 1, 255]]));

        let last = file.len() - 1;
        file[last] = 0;
        assert!(decode_tga(&file).is_err(), "8-bit index 0 wraps to entry 255, outside the map");

        let mut file = tga_header(TGA_TYPE_MAP, 1, 1, TGA_BITS_16, TGA_ATTR_TOP_ORIGIN);
        file[1] = 1;
        file[3..5].copy_from_slice(&1u16.to_le_bytes());
        file[5..7].copy_from_slice(&1u16.to_le_bytes());
        file[7] = TGA_BITS_32;
        file.extend([1, 2, 3, 4]);
        file.extend(1u16.to_le_bytes());
        assert_eq!(decode_tga(&file).unwrap().2, vec![3, 2, 1, 4], "32-bit entries keep alpha");
        let last = file.len() - 2;
        file[last] = 0;
        assert!(decode_tga(&file).is_err(), "16-bit index below the first entry");
    }

    #[test]
    fn rgb555_shifts_without_bit_replication() {
        assert_eq!(rgb555(0xFF, 0x7F), [0xF8, 0xF8, 0xF8, OPAQUE_ALPHA]);
        assert_eq!(rgb555(0x00, 0x80), [0, 0, 0, OPAQUE_ALPHA], "the top bit is ignored");
        assert_eq!(rgb555(0x1F, 0x00), [0, 0, 0xF8, OPAQUE_ALPHA]);
    }

    #[test]
    fn generated_mips_truncate_box_averages_down_to_one_pixel() {
        let level0 = rgba(&[[0, 0, 0, 255], [1, 0, 0, 255], [2, 0, 0, 255], [4, 255, 0, 0]]);
        let levels = generate_mips(2, 2, level0.clone());
        assert_eq!(levels, vec![level0, vec![1, 63, 0, 191]]);

        let row = rgba(&[[10, 0, 0, 0], [21, 0, 0, 0], [30, 0, 0, 0], [41, 0, 0, 0]]);
        let levels = generate_mips(4, 1, row);
        assert_eq!(levels[1], rgba(&[[15, 0, 0, 0], [35, 0, 0, 0]]), "pairs when one pixel high");
        assert_eq!(levels[2], vec![25, 0, 0, 0]);
        assert_eq!(levels.len(), 3);
    }

    fn dxt1_dds(width: u32, height: u32) -> Vec<u8> {
        let mut file = vec![0u8; DDS_HEADER_END];
        file[..DDS_MAGIC.len()].copy_from_slice(DDS_MAGIC);
        let mut put = |offset: usize, value: u32| file[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        put(DDS_SIZE_FIELD_OFFSET, DDS_HEADER_SIZE);
        put(DDS_FLAGS_OFFSET, DDS_REQUIRED_FLAGS);
        put(DDS_HEIGHT_OFFSET, height);
        put(DDS_WIDTH_OFFSET, width);
        put(DDS_PIXEL_FORMAT_OFFSET, DDS_PIXEL_FORMAT_SIZE);
        put(DDS_PIXEL_FORMAT_FLAGS_OFFSET, DDPF_FOURCC);
        put(DDS_CAPS_OFFSET, DDSCAPS_TEXTURE);
        file[DDS_FOURCC_OFFSET..DDS_FOURCC_OFFSET + 4].copy_from_slice(b"DXT1");
        file.extend(WHITE_DXT1_BLOCK);
        file
    }

    #[test]
    fn dds_dxt1_passes_through_when_aligned_and_decodes_otherwise() {
        let image = decode_dds(&dxt1_dds(4, 4)).unwrap();
        assert!(image.opaque);
        match image.pixels {
            SourcePixels::Blocks { format, data, mip_count } => {
                assert_eq!((format, data.as_slice(), mip_count), (TEXTURE_FORMAT_BC1, &WHITE_DXT1_BLOCK[..], 1));
            }
            SourcePixels::Rgba { .. } => panic!("an aligned DXT1 is passed through"),
        }

        let image = decode_dds(&dxt1_dds(2, 2)).unwrap();
        match image.pixels {
            SourcePixels::Rgba { levels, generatable } => {
                assert!(!generatable, "the SDK does not generate chains for compressed sources");
                assert_eq!(levels, vec![vec![u8::MAX; 2 * 2 * RGBA8_BYTES_PER_PIXEL]]);
            }
            SourcePixels::Blocks { .. } => panic!("a 2x2 DXT1 is decoded"),
        }
    }

    /// Bits per pixel of the synthetic `NiPixelData` (32-bit RGBA).
    const NIF_TEXTURE_BITS_PER_PIXEL: u8 = 32;
    /// Components of an `NiPixelFormat` (always stored, used or not).
    const NIF_PIXEL_FORMAT_COMPONENTS: usize = 4;

    /// A `.nif` texture file: `roots` top objects, the first a 32-bit RGBA `NiPixelData` holding a 2x2 level and
    /// a 1x1 level, the others `NiNode`s.
    fn nif_texture(roots: usize) -> Vec<u8> {
        let level0 = rgba(&[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 255, 0]]);
        let level1 = vec![10, 20, 30, 40];
        let mipmaps = vec![
            NiPixelDataMipmap { width: 2, height: 2, offset_in_bytes: 0 },
            NiPixelDataMipmap { width: 1, height: 1, offset_in_bytes: level0.len() as u32 },
        ];
        let pixels = [level0, level1].concat();
        let pixel_data = NiPixelData {
            pixel_format: NiPixelFormat {
                format: PixelFormatFormat::FORMAT_RGBA,
                bits_per_pixel: NIF_TEXTURE_BITS_PER_PIXEL,
                components: vec![NiPixelFormatComponent::default(); NIF_PIXEL_FORMAT_COMPONENTS],
                ..Default::default()
            },
            mipmap_levels: mipmaps.len() as u32,
            pixel_stride: RGBA8_BYTES_PER_PIXEL as u32,
            mipmaps,
            total_offset_in_bytes: pixels.len() as u32,
            pixels,
            ..Default::default()
        };
        let mut blocks = vec![Block::from(pixel_data)];
        blocks.extend((1..roots).map(|_| Block::from(NiNode::default())));
        let nif = NifFile { blocks, roots: (0..roots).map(Ref::new).collect(), ..Default::default() };
        nif.write().unwrap()
    }

    #[test]
    fn nif_texture_files_keep_their_levels_and_need_a_single_pixel_data_top_object() {
        let pixels = read_nif_texture(&nif_texture(1)).unwrap();
        let mut record = TextureRecord::default();
        let mut report = Report::default();
        let image = pixel_data_image("star.nif", &pixels, TEXTURE_SOURCE_NIF_FILE, &mut record, &mut report).unwrap();
        assert_eq!((image.width, image.height, image.source, image.opaque), (2, 2, TEXTURE_SOURCE_NIF_FILE, false));
        assert_eq!(record.source, TEXTURE_SOURCE_NIF_FILE);
        match image.pixels {
            SourcePixels::Rgba { levels, generatable } => {
                assert!(generatable, "uncompressed NiPixelData");
                assert_eq!(levels.len(), 2, "the stored levels are kept");
                assert_eq!(levels[1], vec![10, 20, 30, 40]);
            }
            SourcePixels::Blocks { .. } => panic!("NiPixelData decodes to RGBA8"),
        }
        assert!(report.findings.is_empty());

        assert!(read_nif_texture(&nif_texture(2)).is_err(), "NiNIFImageReader reads exactly one top object");
        let node_only = NifFile { blocks: vec![Block::from(NiNode::default())], roots: vec![Ref::new(0)], ..Default::default() };
        assert!(read_nif_texture(&node_only.write().unwrap()).is_err(), "the top object must be a NiPixelData");
    }

    #[test]
    fn missing_particle_files_are_one_warning_each_with_every_referrer() {
        let mut closure = Closure::default();
        let missing = SystemBuild { kind: SYSTEM_KIND_NIF, flags: SYSTEM_FLAG_MISSING, ..Default::default() };
        closure.systems.insert("lavafall.nif".to_owned(), missing);
        closure.systems.insert("fire.ems".to_owned(), SystemBuild { kind: SYSTEM_KIND_CHAOS, ..Default::default() });
        closure.mesh_users.entry("lavafall.nif".to_owned()).or_default().insert("fire.ems".to_owned());
        let mut referrers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        referrers.entry("lavafall.nif".to_owned()).or_default().extend(["tile 1 [0]".to_owned(), "tile 2 [0]".to_owned()]);
        let mut report = Report::default();
        closure.report_missing_files(&referrers, &mut report);
        assert_eq!(report.findings.len(), 1);
        let finding = &report.findings[0];
        assert_eq!(finding.severity, FindingSeverity::Warning);
        assert!(finding.message.ends_with("named by 3 (tile 1 [0], tile 2 [0], system fire.ems MESH)"), "{finding}");
    }

    fn dictionary(strings: &[&str]) -> UoStringDictionary {
        let mut payload = vec![0u8; size_of::<u64>()];
        payload.extend(u32::try_from(strings.len()).unwrap().to_le_bytes());
        payload.extend(0u32.to_le_bytes());
        for string in strings {
            payload.extend(u16::try_from(string.len()).unwrap().to_le_bytes());
            payload.extend(string.as_bytes());
        }
        UoStringDictionary::from_payload_bytes(&payload, "test").unwrap()
    }

    fn particle(name_index: u32) -> EffectDefinition {
        EffectDefinition::Particle(ParticleEffect {
            name_index,
            offset_x: 1.0,
            offset_y: 2.0,
            camera_offset: 3.0,
            scale: 1.0,
            attach_mode: 0,
            duration: 0.5,
            looping: true,
            field_3c: 0,
            hued: true,
        })
    }

    #[test]
    fn flattening_keeps_the_last_list_of_a_repeated_key_and_records_particle_slots() {
        let strings = dictionary(&["Fire.NIF", "smoke.ems", "readme.txt"]);
        let sound = |sound_id| EffectDefinition::Sound(SoundEffect { sound_id, value_08: 0.0 });
        let entry = |key, children| EffectMapEntry { key, children };
        let lod = EffectDefinition::Lod(LodEffect {
            levels: vec![entry(2, vec![sound(1)]), entry(1, vec![particle(0)]), entry(2, vec![sound(3), particle(1)])],
        });
        let mut tables = DefinitionTables::default();
        let mut report = Report::default();
        let (first, count) = tables.list(&[lod, particle(2)], &strings, &mut report);
        assert_eq!(count, 2);
        let roots = &tables.lists[first as usize..(first + count) as usize];

        let lod = tables.nodes[roots[0] as usize];
        let branches = &tables.branches[lod.first as usize..(lod.first + lod.count) as usize];
        assert_eq!(branches.iter().map(|branch| branch.key).collect::<Vec<_>>(), vec![1, 2]);
        let second = &tables.lists[branches[1].list_first as usize..][..branches[1].list_count as usize];
        assert_eq!(second.len(), 2, "the last list of key 2");

        let names: Vec<&str> = tables.particle_systems.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(names, vec!["fire.nif", "smoke.ems"], "the .txt particle has no system");
        let unnamed = tables.nodes[roots[1] as usize];
        assert_eq!(tables.params[unnamed.param as usize], NONE);
        assert_eq!(unnamed.flags, NODE_FLAG_LOOPING | NODE_FLAG_HUED);
    }
}
