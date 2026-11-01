//! Replacing and adding whole entries of any client UOP package ([`UopEntryEditor`]), with the new file checked by
//! its content.
//!
//! - **Kind check.** The file is classified by its bytes ([`classify_content`]). Its kind must be one the entry
//!   may hold: the kind its name's extension declares ([`declared_format`]), text for a text extension
//!   ([`TEXT_EXTENSIONS`]), or the kind of the (non-empty) entry it replaces; a TGA with trailing bytes counts as a
//!   TGA and a BMP with a wrong size field as a BMP. An unnamed entry of plain binary takes only plain binary (the
//!   shipped 18-byte placeholders named `.tga` take a TGA or another placeholder); Windows-1252 text counts as
//!   text ([`entry_kind`]). A mismatch is refused under
//!   every policy. An entry added without a name or a known extension has nothing to
//!   check against ([`EntryCheck::Unchecked`]).
//! - **Parse check.** Where a strict reader exists for the entry ([`PARSE_CHECKS`]), the file must parse. On a
//!   replace, the reader is the first one (and client dialect) the current entry parses under, so a reader is
//!   never applied to an entry it does not describe, and the file keeps what the reader requires of the entry it
//!   replaces ([`ParseCheck::keeps`]: a GameData table its columns); on an add, the file must parse under one of
//!   the readers of its package and kind, when there are any. A reader for one entry name or extension
//!   ([`ParseCheck::name`], [`ParseCheck::extensions`]: the Interface Lua scripts) applies to a target by hash
//!   whose name it is, or which replaces an entry that parses with it.
//! - **Caller checks.** A crate above uocf adds the readers uocf does not have ([`UopEntryEditor::with_check`]:
//!   the CLI and the inspector add `gamebryo-nif`'s NIF reader); they are tried before the built-in ones, and
//!   a replacement or addition is checked with them the same way.
//! - **Audio.** MP3 and WAV files also pass the `Audio.uop` format checks ([`crate::audio::validate`]) under the
//!   caller's [`FormatPolicy`].
//! - **Storage.** A replaced entry keeps its compression flag; an added one gets the flag most entries of its kind
//!   in the package have ([`UopEntryEditor::family_compression`]), else the package's majority. Edited entries
//!   get a fresh timestamp data block header (`uop_entries::put_entry`); the others keep their bytes and place.
//!   [`UopEntryEditor::save_as`] writes a copy to an absolute path other than the source.

use std::path::{Path, PathBuf};

use crate::audio::validate::{enhanced_format_report, kind_mismatch, FormatPolicy};
use crate::audio::{probe_audio, AudioContent};
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::entry_text::windows_1252_text;
use crate::enhanced::image_census::{classify_content, declared_format, name_extension, ContentFormat};
use crate::enhanced::uop_entries::{put_entry, save_package_as};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{LoadMode, NewEntryCompression, UopPackage};

crate::eyre_imports!();

/// Extensions of the shipped text entries no magic identifies: they take text (`classify_content`'s
/// [`ContentFormat::Text`]).
pub const TEXT_EXTENSIONS: [&str; 6] = ["csv", "lua", "txt", "ems", "psh", "vsh"];
const HEX_PREFIX: &str = "0x";
const HEX_RADIX: u32 = 16;
/// Hex digits of an entry hash written without the prefix.
const HASH_HEX_DIGITS: usize = 16;
/// The reader [`EntryCheck::Parsed`] names for the `Audio.uop` format checks.
pub const AUDIO_READER: &str = "audio format checks";
const MP3_EXTENSION: &str = "mp3";
const WAV_EXTENSION: &str = "wav";
/// The client dialects a parse check tries, in order.
const DIALECTS: [ClientDialect; 2] = [ClientDialect::Enhanced, ClientDialect::KingdomReborn];

/// An entry by name or by hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryTarget {
    Name(String),
    Hash(u64),
}

impl EntryTarget {
    /// A hash written as `0x` and hex digits, or as 16 hex digits; anything else is a name.
    pub fn parse(text: &str) -> eyre::Result<Self> {
        if let Some(digits) = text.strip_prefix(HEX_PREFIX) {
            return Ok(Self::Hash(u64::from_str_radix(digits, HEX_RADIX).wrap_err_with(|| format!("bad entry hash {text}"))?));
        }
        if text.len() == HASH_HEX_DIGITS && text.chars().all(|digit| digit.is_ascii_hexdigit()) {
            return Ok(Self::Hash(u64::from_str_radix(text, HEX_RADIX)?));
        }
        Ok(Self::Name(text.to_owned()))
    }

    pub fn hash(&self) -> u64 {
        match self {
            Self::Name(name) => hash_file_name_single(name),
            Self::Hash(hash) => *hash,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Name(name) => Some(name),
            Self::Hash(_) => None,
        }
    }

    /// The name, or the hash in hex.
    pub fn label(&self) -> String {
        match self {
            Self::Name(name) => name.clone(),
            Self::Hash(hash) => format!("{hash:016X}"),
        }
    }
}

/// Where an entry's file must parse.
#[derive(Clone, Copy)]
pub struct ParseCheck {
    pub label: &'static str,
    /// Package file stems (lowercase, digits removed: `facet0x.uop` is `facetx`), or empty for any package.
    pub packages: &'static [&'static str],
    /// The content kinds it reads.
    pub kinds: &'static [ContentFormat],
    /// Only the entry of this name, when set (a target by hash when the name hashes to it).
    pub name: Option<&'static str>,
    /// Only entries named with one of these extensions, when not empty. A target by hash has no name: the check
    /// applies when it replaces an entry (that parses with it), not to an entry added by hash.
    pub extensions: &'static [&'static str],
    /// The reader reads both client dialects ([`DIALECTS`]); else it is called with the default one.
    pub dialects: bool,
    pub check: fn(&[u8], ClientDialect) -> eyre::Result<()>,
    /// What a replacement must keep of the entry it replaces, called with the current payload and the file
    /// after both parse (a GameData table keeps its columns: [`crate::enhanced::game_data::keeps_shape`]).
    pub keeps: Option<fn(&[u8], &[u8]) -> eyre::Result<()>>,
}

/// The reader label of the Classic Client's `animationlegacyframe` entries.
pub const CLASSIC_ANIMATION_FRAMES_READER: &str = "Classic animation frames (animationlegacyframe)";
/// The [`package_key`] of `AnimationFrame*.uop`.
const ANIMATION_FRAME_PACKAGE_KEY: &str = "animationframe";

fn ok<T>(result: eyre::Result<T>) -> eyre::Result<()> {
    result.map(|_| ())
}

/// The strict readers a file of an entry is checked with (see the module documentation), the specific ones first:
/// a hue bitmap is checked as a hue bitmap before it is checked as a BMP.
pub const PARSE_CHECKS: &[ParseCheck] = &[
    ParseCheck {
        label: "Chaos script",
        packages: &["effecttexture"],
        kinds: &[ContentFormat::Text],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::chaos_script::ChaosScript::parse_bytes(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "sound table",
        packages: &["audio"],
        kinds: &[ContentFormat::Text],
        name: Some(crate::enhanced::audio::SOUND_TABLE_ENTRY),
        extensions: &[],
        dialects: false,
        check: |bytes, _| {
            use crate::enhanced::audio::{parse_table, SOUND_TABLE_ENTRY, SOUND_TABLE_HEADER};
            ok(parse_table(SOUND_TABLE_ENTRY, std::str::from_utf8(bytes)?, SOUND_TABLE_HEADER, false))
        },
        keeps: None,
    },
    ParseCheck {
        label: "music table",
        packages: &["audio"],
        kinds: &[ContentFormat::Text],
        name: Some(crate::enhanced::audio::MUSIC_TABLE_ENTRY),
        extensions: &[],
        dialects: false,
        check: |bytes, _| {
            use crate::enhanced::audio::{parse_table, MUSIC_TABLE_ENTRY, MUSIC_TABLE_HEADER};
            ok(parse_table(MUSIC_TABLE_ENTRY, std::str::from_utf8(bytes)?, MUSIC_TABLE_HEADER, true))
        },
        keeps: None,
    },
    ParseCheck {
        label: "Kingdom Reborn sound table",
        packages: &["audio"],
        kinds: &[ContentFormat::Text],
        name: Some(crate::enhanced::audio::SOUND_TABLE_ENTRY),
        extensions: &[],
        dialects: false,
        check: |bytes, _| {
            use crate::enhanced::{audio::SOUND_TABLE_ENTRY, kr_audio::KrSoundTable};
            ok(KrSoundTable::parse(SOUND_TABLE_ENTRY, std::str::from_utf8(bytes)?))
        },
        keeps: None,
    },
    ParseCheck {
        label: "Kingdom Reborn effects table",
        packages: &["audio"],
        kinds: &[ContentFormat::Text],
        name: Some(crate::enhanced::kr_audio::KR_EFFECTS_TABLE_ENTRY),
        extensions: &[],
        dialects: false,
        check: |bytes, _| {
            use crate::enhanced::kr_audio::{KrEffectsTable, KR_EFFECTS_TABLE_ENTRY};
            ok(KrEffectsTable::parse(KR_EFFECTS_TABLE_ENTRY, std::str::from_utf8(bytes)?))
        },
        keeps: None,
    },
    ParseCheck {
        label: "legacy terrain map",
        packages: &["gamedata"],
        kinds: &[ContentFormat::Text],
        name: Some(crate::enhanced::legacy_terrain_map::UOP_ENTRY_PATH),
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::legacy_terrain_map::check_canonical(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "GameData table",
        packages: &["gamedata"],
        kinds: &[ContentFormat::Text],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::game_data::GameDataTable::parse("the table", bytes)),
        keeps: Some(crate::enhanced::game_data::keeps_shape),
    },
    ParseCheck {
        label: "legacy terrain document",
        packages: &["legacyterrain"],
        kinds: &[ContentFormat::Xml],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::legacy_terrain::LegacyTerrainDocument::parse(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "Lua script",
        packages: &["interface"],
        // Two EC scripts are UTF-16, which LuaPlus reads.
        kinds: &[ContentFormat::Text, ContentFormat::Utf16Text],
        name: None,
        extensions: &["lua"],
        dialects: false,
        check: |bytes, _| crate::enhanced::syntax_check::check_lua(bytes),
        keeps: None,
    },
    ParseCheck {
        label: "TinyXML document",
        packages: &["interface", "mainmisc"],
        kinds: &[ContentFormat::Xml],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| crate::enhanced::syntax_check::check_xml(bytes),
        keeps: None,
    },
    ParseCheck {
        label: "UTF-16 TinyXML document",
        packages: &["interface", "mainmisc"],
        kinds: &[ContentFormat::Utf16Text],
        name: None,
        extensions: &["xml"],
        dialects: false,
        check: |bytes, _| crate::enhanced::syntax_check::check_xml(bytes),
        keeps: None,
    },
    ParseCheck {
        label: "hue bitmap",
        packages: &["hues"],
        kinds: &[ContentFormat::Bmp],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::hues::EcHueTable::from_bitmap(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "tileart record",
        packages: &["tileart"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::tileart::TileArtEntry::parse_bytes(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "animation definition",
        packages: &["animationdefinition"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: true,
        check: |bytes, dialect| ok(crate::enhanced::animation_definition::AnimationDefinition::parse_dialect(bytes, dialect)),
        keeps: None,
    },
    ParseCheck {
        label: "animation sequence",
        packages: &["animationsequence"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: true,
        check: |bytes, dialect| ok(crate::animation_sequence::AnimationSequence::parse_dialect(bytes, dialect)),
        keeps: None,
    },
    // Before the Enhanced reader, which also reads the Classic Client's container: a Classic entry is checked as
    // Classic frames, and an Enhanced entry, which does not parse as one, with the Enhanced reader.
    ParseCheck {
        label: CLASSIC_ANIMATION_FRAMES_READER,
        packages: &[ANIMATION_FRAME_PACKAGE_KEY],
        kinds: &[ContentFormat::Amou],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::classic::anim_frames::uop::ClassicUopEntry::parse(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "animation frames (AMOU)",
        packages: &[],
        kinds: &[ContentFormat::Amou],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::animationframe::AnimationFrame::load(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "effect definition collection",
        packages: &["effectdefinitioncollection"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: true,
        check: |bytes, dialect| {
            ok(crate::enhanced::effect_definition::parse_effect_definition_collection_dialect(bytes, dialect))
        },
        keeps: None,
    },
    ParseCheck {
        label: "terrain definition",
        packages: &["terraindefinition"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: true,
        check: |bytes, dialect| ok(crate::enhanced::terrain_definition::TerrainDefinitionEntry::from_bytes_dialect(bytes, dialect)),
        keeps: None,
    },
    ParseCheck {
        label: "facet definition",
        packages: &["facets"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::facet_definition::FacetDefinition::from_bytes(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "facet sector",
        packages: &["facet", "facetx"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: true,
        check: |bytes, dialect| ok(crate::enhanced::facet_decoder::decode_sector(bytes, dialect)),
        keeps: None,
    },
    ParseCheck {
        label: "facet data map",
        packages: &["facet", "facetx"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::facet_sector::FacetGeometry::from_data_map_payload(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "housing pieces",
        packages: &["multicollection"],
        kinds: &[ContentFormat::Binary],
        name: Some("build/multicollection/housing.bin"),
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::housing::HousingPieces::from_payload_bytes(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "multi",
        packages: &["multicollection"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: true,
        check: |bytes, dialect| ok(crate::enhanced::multis::MultiItem::from_payload_bytes_dialect(0, String::new(), bytes, dialect)),
        keeps: None,
    },
    ParseCheck {
        label: "waypoints",
        packages: &["waypoint"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::waypoints::WaypointsPackage::from_payload_bytes(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "string dictionary",
        packages: &["string_dictionary"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::string_dictionary::UoStringDictionary::from_payload_bytes(bytes, "entry")),
        keeps: None,
    },
    ParseCheck {
        label: "localized strings",
        packages: &["localizedstrings"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::localized_strings::LocalizedStringTable::from_payload_bytes(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "Classic gump",
        packages: &["gumpartlegacymul"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::classic::gump::decode_uop_gump(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "static chunk",
        packages: &["staticchunk"],
        kinds: &[ContentFormat::Binary],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::kr::facet::static_chunk::StaticChunk::decode(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "DDS header",
        packages: &[],
        kinds: &[ContentFormat::Dds],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::dds_writer::DdsDescription::parse(bytes)),
        keeps: None,
    },
    ParseCheck {
        label: "font table directory",
        packages: &[],
        kinds: &[ContentFormat::Font],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| check_font(bytes),
        keeps: None,
    },
    ParseCheck {
        label: "PNG, BMP or JPEG decode",
        packages: &[],
        kinds: &[ContentFormat::Png, ContentFormat::Bmp, ContentFormat::Jpeg],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(image::load_from_memory(bytes).map_err(Into::into)),
        keeps: None,
    },
    ParseCheck {
        label: "TGA decode",
        packages: &[],
        kinds: &[ContentFormat::Tga],
        name: None,
        extensions: &[],
        dialects: false,
        check: |bytes, _| ok(image::load_from_memory_with_format(bytes, image::ImageFormat::Tga).map_err(Into::into)),
        keeps: None,
    },
];

/// An sfnt font's table directory, or each font's of a collection: every table lies inside the file.
fn check_font(bytes: &[u8]) -> eyre::Result<()> {
    const COLLECTION_MAGIC: &[u8] = b"ttcf";
    const COLLECTION_COUNT_OFFSET: usize = 8;
    const COLLECTION_OFFSETS_START: usize = 12;
    const TABLE_COUNT_OFFSET: usize = 4;
    const DIRECTORY_HEADER_BYTES: usize = 12;
    const TABLE_RECORD_BYTES: usize = 16;
    const TABLE_OFFSET_FIELD: usize = 8;
    const TABLE_LENGTH_FIELD: usize = 12;
    let u16_at = |offset: usize| bytes.get(offset..offset + size_of::<u16>()).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize);
    let u32_at = |offset: usize| bytes.get(offset..offset + size_of::<u32>()).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let directories: Vec<usize> = if bytes.starts_with(COLLECTION_MAGIC) {
        let count = u32_at(COLLECTION_COUNT_OFFSET).ok_or_else(|| eyre!("font collection: no font count"))?;
        (0..count)
            .map(|index| u32_at(COLLECTION_OFFSETS_START + index * size_of::<u32>()).ok_or_else(|| eyre!("font collection: font {index} has no offset")))
            .collect::<eyre::Result<_>>()?
    } else {
        vec![0]
    };
    for start in directories {
        let tables = u16_at(start + TABLE_COUNT_OFFSET).ok_or_else(|| eyre!("font at {start}: no table count"))?;
        for index in 0..tables {
            let record = start + DIRECTORY_HEADER_BYTES + index * TABLE_RECORD_BYTES;
            let (Some(offset), Some(length)) = (u32_at(record + TABLE_OFFSET_FIELD), u32_at(record + TABLE_LENGTH_FIELD)) else {
                eyre::bail!("font at {start}: table record {index} is cut");
            };
            if offset.checked_add(length).is_none_or(|end| end > bytes.len()) {
                eyre::bail!("font at {start}: table {index} ({offset}+{length}) ends past the file ({} bytes)", bytes.len());
            }
        }
    }
    Ok(())
}

/// The extension the audio checks give an entry of `kind` by hash, for an MP3 or WAV.
fn audio_extension(kind: ContentFormat) -> Option<&'static str> {
    match kind {
        ContentFormat::Mp3 => Some(MP3_EXTENSION),
        ContentFormat::Wav => Some(WAV_EXTENSION),
        _ => None,
    }
}

/// A package's key in [`ParseCheck::packages`]: the lowercase file stem without digits.
pub fn package_key(path: &Path) -> String {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    stem.chars().filter(|c| !c.is_ascii_digit()).collect()
}

/// The kind a content counts as for the kind check.
pub fn kind_family(format: ContentFormat) -> ContentFormat {
    match format {
        ContentFormat::TgaTrailing => ContentFormat::Tga,
        ContentFormat::BmpBadSize => ContentFormat::Bmp,
        other => other,
    }
}

/// What a file was checked with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryCheck {
    /// It parsed with this reader (and, for the readers with dialects, this dialect). MP3 and WAV entries count
    /// as parsed by [`AUDIO_READER`] when their name was known and the audio checks ran.
    Parsed { reader: &'static str, dialect: Option<ClientDialect> },
    /// Its kind matched; no reader applies.
    Kind(ContentFormat),
    /// Nothing to check against: an entry added by hash, or under a name of unknown extension.
    Unchecked(ContentFormat),
}

/// The kind a file counts as for the kind check: its [`kind_family`], and text for a file that is Windows-1252
/// text ([`windows_1252_text`]: the EC `GameData.uop` table with an `A0` byte), so that it takes its UTF-8 edit.
pub fn entry_kind(bytes: &[u8]) -> ContentFormat {
    match kind_family(classify_content(bytes).format) {
        ContentFormat::Binary if windows_1252_text(bytes).is_some() => ContentFormat::Text,
        kind => kind,
    }
}

/// What happened to an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryWrite {
    Replaced,
    /// The entry already held the file: it keeps its bytes and header.
    Unchanged,
    Added,
}

/// One replaced or added entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryEdit {
    pub hash: u64,
    pub name: Option<String>,
    pub write: EntryWrite,
    pub compression: CompressionFlag,
    pub kind: ContentFormat,
    pub check: EntryCheck,
    /// The audio format problems [`FormatPolicy::AllowUnverified`] let through.
    pub warnings: Vec<String>,
}

/// A UOP package being edited entry by entry.
pub struct UopEntryEditor {
    source: PathBuf,
    package: UopPackage,
    /// The caller's readers ([`Self::with_check`]), tried before [`PARSE_CHECKS`].
    caller_checks: Vec<ParseCheck>,
}

impl UopEntryEditor {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let source = path.as_ref().to_path_buf();
        let package = UopPackage::load_with_mode(&source, LoadMode::Lazy).wrap_err_with(|| format!("Failed to load {}", source.display()))?;
        Ok(Self { source, package, caller_checks: Vec::new() })
    }

    /// An editor over a loaded package (from `source`, which [`Self::save_as`] never overwrites).
    pub fn from_package(source: PathBuf, package: UopPackage) -> Self {
        Self { source, package, caller_checks: Vec::new() }
    }

    /// Adds a reader uocf does not have (a NIF parser): it checks the entries it applies to (its packages, kinds
    /// and name, as for [`PARSE_CHECKS`]) before the built-in readers do.
    pub fn with_check(mut self, check: ParseCheck) -> Self {
        self.caller_checks.push(check);
        self
    }

    /// [`Self::with_check`] for each of `checks`.
    pub fn with_checks(self, checks: impl IntoIterator<Item = ParseCheck>) -> Self {
        checks.into_iter().fold(self, Self::with_check)
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn package(&self) -> &UopPackage {
        &self.package
    }

    pub fn package_mut(&mut self) -> &mut UopPackage {
        &mut self.package
    }

    fn unpack(&self, hash: u64) -> eyre::Result<Option<Vec<u8>>> {
        self.package.unpack_file_by_hash(hash).wrap_err_with(|| format!("Failed to read entry {hash:016X}"))
    }

    /// Stores `payload` as the entry `hash`, without checks: an existing entry keeps its flag, a new one gets
    /// `compression`. The shared write path of the editors over this one.
    pub fn put_unchecked(&mut self, hash: u64, payload: &[u8], compression: CompressionFlag) -> eyre::Result<EntryWrite> {
        let current = self.unpack(hash)?;
        if current.as_deref() == Some(payload) {
            return Ok(EntryWrite::Unchanged);
        }
        put_entry(&mut self.package, hash, payload, compression)?;
        Ok(if current.is_some() { EntryWrite::Replaced } else { EntryWrite::Added })
    }

    /// The parse checks of this package that apply to the entry `target` of `kind`: the caller's, then uocf's. A
    /// check for one entry name applies to a target by hash when the name hashes to it.
    fn parse_checks(&self, target: &EntryTarget, kind: ContentFormat, replacing: bool) -> Vec<&ParseCheck> {
        let extension = target.name().and_then(name_extension);
        let key = package_key(&self.source);
        self.caller_checks
            .iter()
            .chain(PARSE_CHECKS)
            .filter(|check| check.packages.is_empty() || check.packages.contains(&key.as_str()))
            .filter(|check| check.kinds.contains(&kind))
            .filter(|check| check.name.is_none_or(|wanted| hash_file_name_single(wanted) == target.hash()))
            .filter(|check| {
                check.extensions.is_empty()
                    || match &extension {
                        Some(extension) => check.extensions.contains(&extension.as_str()),
                        None => target.name().is_none() && replacing,
                    }
            })
            .collect()
    }

    /// The first (reader, dialect) of `checks` that accepts `bytes`.
    fn first_parse<'a>(checks: &[&'a ParseCheck], bytes: &[u8]) -> Option<(&'a ParseCheck, ClientDialect)> {
        checks.iter().find_map(|check| {
            let dialects = if check.dialects { &DIALECTS[..] } else { &DIALECTS[..1] };
            dialects.iter().find(|&&dialect| (check.check)(bytes, dialect).is_ok()).map(|&dialect| (*check, dialect))
        })
    }

    /// Checks `file` for the entry `target` (current payload `current`): the kind, the parse, the audio formats.
    fn check(&self, target: &EntryTarget, current: Option<&[u8]>, file: &[u8], policy: FormatPolicy) -> eyre::Result<(EntryCheck, Vec<String>)> {
        let label = target.label();
        let kind = entry_kind(file);
        let current_kind = current.map(entry_kind);
        let name = target.name();
        let mut expected: Vec<ContentFormat> = name.and_then(declared_format).map(kind_family).into_iter().collect();
        let text_extension = name.and_then(name_extension).is_some_and(|extension| TEXT_EXTENSIONS.contains(&extension.as_str()));
        if expected.is_empty() && text_extension {
            expected.push(ContentFormat::Text);
        }
        match current_kind {
            None | Some(ContentFormat::Empty) => {}
            Some(current_kind) if !expected.contains(&current_kind) => expected.push(current_kind),
            Some(_) => {}
        }
        if !expected.is_empty() && !expected.contains(&kind) {
            let wanted: Vec<&str> = expected.iter().map(|format| format.label()).collect();
            eyre::bail!("{label}: the file is {}, the entry takes {}; a file of another kind is never stored", kind.label(), wanted.join(" or "));
        }

        let mut warnings = Vec::new();
        let mut audio_checked = false;
        if let Some(extension) = audio_extension(kind) {
            let content = probe_audio(file);
            if !matches!(content, AudioContent::Mpeg(_) | AudioContent::Wav(_)) {
                eyre::bail!("{label}: the file starts as {} but holds no {0} stream ({} by its content)", kind.label(), content.format().label());
            }
            // A target by hash is checked as an entry of its kind outside the legacy sound root.
            let synthetic;
            let audio_name = match name {
                Some(name) => name,
                None => {
                    synthetic = format!("{:016x}.{extension}", target.hash());
                    synthetic.as_str()
                }
            };
            if let Some(mismatch) = kind_mismatch(audio_name, &content) {
                eyre::bail!("{label}: {mismatch}");
            }
            warnings = policy.apply(audio_name, enhanced_format_report(audio_name, &content))?;
            audio_checked = true;
        }

        let checks = self.parse_checks(target, kind, current.is_some());
        let parsed = match current {
            Some(current) if current_kind == Some(kind) => Self::first_parse(&checks, current).map(|(check, dialect)| {
                (check.check)(file, dialect)
                    .wrap_err_with(|| format!("{label}: the file does not parse as a {} (the current entry does)", check.label))?;
                if let Some(keeps) = check.keeps {
                    keeps(current, file).wrap_err_with(|| format!("{label}: the file does not keep the {} it replaces", check.label))?;
                }
                Ok((check, dialect))
            }),
            Some(_) => None,
            None if checks.is_empty() => None,
            None => Some(Self::first_parse(&checks, file).ok_or_else(|| {
                let readers: Vec<&str> = checks.iter().map(|check| check.label).collect();
                eyre!("{label}: the file parses as none of {}", readers.join(", "))
            })),
        };
        let check = match parsed.transpose()? {
            Some((check, dialect)) => EntryCheck::Parsed { reader: check.label, dialect: check.dialects.then_some(dialect) },
            None if audio_checked => EntryCheck::Parsed { reader: AUDIO_READER, dialect: None },
            None if expected.is_empty() => EntryCheck::Unchecked(kind),
            None => EntryCheck::Kind(kind),
        };
        Ok((check, warnings))
    }

    /// Checks `file` as a replacement of the existing entry `target` without storing it: the checks of
    /// [`Self::replace`].
    pub fn check_replacement(&self, target: &EntryTarget, file: &[u8], policy: FormatPolicy) -> eyre::Result<EntryCheck> {
        let current = self.unpack(target.hash())?.ok_or_else(|| eyre!("{}: no entry {}", self.source.display(), target.label()))?;
        Ok(self.check(target, Some(&current), file, policy)?.0)
    }

    /// The flag most entries of `kind` in the package have; the package's majority when none is of that kind.
    /// Every entry is classified only when the package mixes flags.
    pub fn family_compression(&self, kind: ContentFormat) -> eyre::Result<CompressionFlag> {
        let files: Vec<(u64, CompressionFlag)> =
            self.package.iter_files().filter(|file| !file.is_empty_slot()).map(|file| (file.filename_hash(), file.compression())).collect();
        let uniform = files.first().map(|first| first.1).filter(|flag| files.iter().all(|file| file.1 == *flag));
        if let Some(flag) = uniform {
            return Ok(flag);
        }
        let (mut uncompressed, mut zlib) = (0usize, 0usize);
        for (hash, flag) in files {
            let Some(payload) = self.unpack(hash)? else { continue };
            if entry_kind(&payload) != kind {
                continue;
            }
            match flag {
                CompressionFlag::None => uncompressed += 1,
                CompressionFlag::Zlib => zlib += 1,
                CompressionFlag::Mythic | CompressionFlag::ZlibBwt => {}
            }
        }
        Ok(match (uncompressed, zlib) {
            (0, 0) => self.package.new_entry_compression(NewEntryCompression::LikePackage),
            (uncompressed, zlib) if uncompressed > zlib => CompressionFlag::None,
            _ => CompressionFlag::Zlib,
        })
    }

    /// Replaces the existing entry `target` with `file`, keeping its compression flag.
    pub fn replace(&mut self, target: &EntryTarget, file: &[u8], policy: FormatPolicy) -> eyre::Result<EntryEdit> {
        let hash = target.hash();
        let compression = self
            .package
            .get_file_by_hash(hash)
            .map(|entry| entry.compression())
            .ok_or_else(|| eyre!("{}: no entry {} to replace (add it instead)", self.source.display(), target.label()))?;
        let current = self.unpack(hash)?;
        let (check, warnings) = self.check(target, current.as_deref(), file, policy)?;
        let write = self.put_unchecked(hash, file, compression)?;
        let kind = entry_kind(file);
        Ok(EntryEdit { hash, name: target.name().map(str::to_owned), write, compression, kind, check, warnings })
    }

    /// Adds `file` as the new entry `target`, stored with `compression` or the family flag.
    pub fn add(&mut self, target: &EntryTarget, file: &[u8], compression: Option<CompressionFlag>, policy: FormatPolicy) -> eyre::Result<EntryEdit> {
        let hash = target.hash();
        if self.package.get_file_by_hash(hash).is_some() {
            eyre::bail!("{}: the entry {} exists (replace it instead)", self.source.display(), target.label());
        }
        if let Some(name) = target.name() {
            if name.contains('\\') || name.chars().any(|c| c.is_ascii_uppercase()) {
                eyre::bail!("`{name}`: entry names are lowercase with `/` separators (the name is hashed as written)");
            }
        }
        if file.is_empty() {
            eyre::bail!("{}: an entry needs a payload", target.label());
        }
        let (check, warnings) = self.check(target, None, file, policy)?;
        let kind = entry_kind(file);
        let compression = match compression {
            Some(compression) => compression,
            None => self.family_compression(kind)?,
        };
        let write = self.put_unchecked(hash, file, compression)?;
        Ok(EntryEdit { hash, name: target.name().map(str::to_owned), write, compression, kind, check, warnings })
    }

    /// Writes the edited package to `output`: an absolute path, not the source.
    pub fn save_as(&mut self, output: &Path) -> eyre::Result<()> {
        if !output.is_absolute() {
            eyre::bail!("the output must be an absolute path: {}", output.display());
        }
        save_package_as(&mut self.package, &self.source, output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{mp3, wav};

    const CSV_NAME: &str = "data/test/table.csv";
    const XML_NAME: &str = "data/test/doc.xml";
    const MP3_NAME: &str = "data/audio/sounds/test/a.mp3";
    const SCRIPT_NAME: &str = "data/effects/test.ems";
    /// A Chaos script the parser takes: a particle system with one property.
    const SCRIPT: &str = "particlesystem \"Test\" 1\n{\n\tminEmitTime = 1\n}\n";

    fn package_with(dir: &Path, file: &str, entries: &[(&str, &[u8], CompressionFlag)]) -> PathBuf {
        let mut package = UopPackage::new_default();
        for &(name, payload, flag) in entries {
            package.add_entry_by_name(name, payload, NewEntryCompression::Flag(flag)).unwrap();
        }
        let path = dir.join(file);
        package.finalize_and_save(&path).unwrap();
        path
    }

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uocf_entry_edit_{test}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn checks_kinds_and_keeps_flags() {
        let dir = temp_dir("kinds");
        let mp3 = mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 4);
        let path = package_with(
            &dir,
            "Test.uop",
            &[
                (CSV_NAME, b"a,b\r\n1,2\r\n", CompressionFlag::Zlib),
                (XML_NAME, b"<?xml version=\"1.0\"?><a/>", CompressionFlag::None),
                (MP3_NAME, &mp3, CompressionFlag::None),
                ("build/test/blob.bin", &[0, 1, 2, 3, 0xff], CompressionFlag::Zlib),
                ("data/test/latin.csv", b"Prized\xa0,1\r\n", CompressionFlag::Zlib),
            ],
        );
        let mut editor = UopEntryEditor::load(&path).unwrap();
        let policy = FormatPolicy::VerifiedOnly;
        let csv = EntryTarget::Name(CSV_NAME.to_owned());
        let put = editor.replace(&csv, b"a,b\r\n3,4\r\n", policy).unwrap();
        assert_eq!((put.write, put.compression, put.check.clone()), (EntryWrite::Replaced, CompressionFlag::Zlib, EntryCheck::Kind(ContentFormat::Text)));
        assert!(editor.replace(&csv, &mp3, policy).unwrap_err().to_string().contains("never stored"));
        let xml = EntryTarget::Name(XML_NAME.to_owned());
        assert!(editor.replace(&xml, b"plain text", policy).is_err());
        assert_eq!(editor.replace(&xml, b"<?xml version=\"1.0\"?><a/>", policy).unwrap().write, EntryWrite::Unchanged);
        let wav = wav::encode_pcm_wav(22_050, 1, &[0, 0]);
        let error = editor.replace(&EntryTarget::Name(MP3_NAME.to_owned()), &wav, FormatPolicy::AllowUnverified).unwrap_err();
        assert!(error.to_string().contains("never stored"), "{error}");
        let latin = EntryTarget::Hash(hash_file_name_single("data/test/latin.csv"));
        assert_eq!(editor.replace(&latin, b"Prized,1\r\n", policy).unwrap().check, EntryCheck::Kind(ContentFormat::Text), "Windows-1252 takes UTF-8");
        let blob = EntryTarget::parse(&format!("{:016X}", hash_file_name_single("build/test/blob.bin"))).unwrap();
        assert_eq!(EntryTarget::parse("0x1f").unwrap(), EntryTarget::Hash(0x1f));
        assert!(EntryTarget::parse("0xnothex").is_err());
        assert_eq!(EntryTarget::parse(CSV_NAME).unwrap(), csv);
        assert!(matches!(blob, EntryTarget::Hash(_)));
        assert!(editor.replace(&blob, b"now text", policy).is_err(), "binary takes binary");
        assert_eq!(editor.replace(&blob, &[9, 0, 9, 0xfe], policy).unwrap().check, EntryCheck::Kind(ContentFormat::Binary));

        let added = editor.add(&EntryTarget::Name("data/audio/sounds/test/b.mp3".to_owned()), &mp3, None, policy).unwrap();
        assert_eq!((added.write, added.compression), (EntryWrite::Added, CompressionFlag::None), "like the other MP3");
        let added = editor.add(&EntryTarget::Name("data/test/other.csv".to_owned()), b"x\r\n", None, policy).unwrap();
        assert_eq!(added.compression, CompressionFlag::Zlib, "like the other text");
        assert!(editor.add(&EntryTarget::Name("data/test/other.csv".to_owned()), b"x", None, policy).is_err(), "exists");
        assert!(editor.add(&EntryTarget::Name("data/test/Upper.csv".to_owned()), b"x", None, policy).is_err());
        let unchecked = editor.add(&EntryTarget::Hash(1), b"anything", None, policy).unwrap();
        assert_eq!(unchecked.check, EntryCheck::Unchecked(ContentFormat::Text));

        let output = dir.join("Test.edited.uop");
        assert!(editor.save_as(&path).is_err());
        assert!(editor.save_as(Path::new("relative.uop")).is_err());
        editor.save_as(&output).unwrap();
        let saved = UopPackage::load(&output).unwrap();
        assert_eq!(saved.unpack_file_by_hash(csv.hash()).unwrap().unwrap(), b"a,b\r\n3,4\r\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_with_the_reader_the_current_entry_parses_under() {
        let dir = temp_dir("parse");
        let script = SCRIPT.as_bytes();
        crate::enhanced::chaos_script::ChaosScript::parse_bytes(script).expect("the test script parses");
        let path = package_with(&dir, "EffectTexture.uop", &[(SCRIPT_NAME, script, CompressionFlag::Zlib), ("data/effects/notes.txt", b"not a { script", CompressionFlag::Zlib)]);
        let mut editor = UopEntryEditor::load(&path).unwrap();
        let policy = FormatPolicy::VerifiedOnly;
        let target = EntryTarget::Name(SCRIPT_NAME.to_owned());
        let put = editor.replace(&target, b"particlesystem \"Other\" 1\n{\n\tminEmitTime = 2\n}\n", policy).unwrap();
        assert!(matches!(put.check, EntryCheck::Parsed { reader: "Chaos script", .. }), "{:?}", put.check);
        assert!(editor.replace(&target, b"particlesystem \"x\"\r\n{}", policy).unwrap_err().to_string().contains("does not parse"));
        let notes = EntryTarget::Name("data/effects/notes.txt".to_owned());
        assert_eq!(editor.replace(&notes, b"still { not a script", policy).unwrap().check, EntryCheck::Kind(ContentFormat::Text));
        assert!(editor.add(&EntryTarget::Name("data/effects/new.ems".to_owned()), b"particlesystem \"x\"\r\n{}", None, policy).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// A JPEG in an RGB image of `side` pixels square.
    fn jpeg(side: u32) -> Vec<u8> {
        const RGB_CHANNELS: u32 = 3;
        let pixels = vec![0x80; (side * side * RGB_CHANNELS) as usize];
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes).encode(&pixels, side, side, image::ExtendedColorType::Rgb8).unwrap();
        bytes
    }

    #[test]
    fn checks_with_the_callers_readers() {
        const NAME: &str = "data/test/model.nif";
        const NIF_HEADER: &[u8] = b"Gamebryo File Format, Version 20.0.0.4\n";
        /// A stand-in reader: a NIF must end in `ok`.
        const ENDS_IN_OK: ParseCheck = ParseCheck {
            label: "test NIF reader",
            packages: &[],
            kinds: &[ContentFormat::Nif],
            name: None,
            extensions: &[],
            dialects: false,
            check: |bytes, _| if bytes.ends_with(b"ok") { Ok(()) } else { Err(eyre!("does not end in ok")) },
            keeps: None,
        };
        let nif = |tail: &[u8]| [NIF_HEADER, tail].concat();
        let dir = temp_dir("caller_checks");
        let path = package_with(&dir, "Test.uop", &[(NAME, &nif(b"ok"), CompressionFlag::None)]);
        let target = EntryTarget::Name(NAME.to_owned());
        let policy = FormatPolicy::VerifiedOnly;
        let mut plain = UopEntryEditor::load(&path).unwrap();
        assert_eq!(plain.replace(&target, &nif(b"bad"), policy).unwrap().check, EntryCheck::Kind(ContentFormat::Nif));
        let mut editor = UopEntryEditor::load(&path).unwrap().with_checks([ENDS_IN_OK]);
        let put = editor.replace(&target, &nif(b"also ok"), policy).unwrap();
        assert_eq!(put.check, EntryCheck::Parsed { reader: "test NIF reader", dialect: None });
        assert!(editor.replace(&target, &nif(b"bad"), policy).unwrap_err().to_string().contains("test NIF reader"));
        assert!(editor.add(&EntryTarget::Name("data/test/other.nif".to_owned()), &nif(b"bad"), None, policy).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn game_data_tables_keep_their_columns() {
        const NAME: &str = "data/gamedata/weaponability.csv";
        let dir = temp_dir("game_data");
        let path = package_with(&dir, "GameData.uop", &[(NAME, b"objectId,primaryAbility\r\n3568,ArmorIgnore\r\n", CompressionFlag::Zlib)]);
        let mut editor = UopEntryEditor::load(&path).unwrap();
        let target = EntryTarget::Name(NAME.to_owned());
        let policy = FormatPolicy::VerifiedOnly;
        let put = editor.replace(&target, b"objectId,primaryAbility\r\n3568,Disarm\r\n3569,Dismount\r\n", policy).unwrap();
        assert_eq!(put.check, EntryCheck::Parsed { reader: "GameData table", dialect: None });
        let error = format!("{:#}", editor.replace(&target, b"objectId,primaryAbility\r\nsword,Disarm\r\n", policy).unwrap_err());
        assert!(error.contains("does not keep the GameData table") && error.contains("field 1 (objectId)"), "{error}");
        assert!(editor.replace(&target, b"objectId,primary\r\n1,\"open\r\n", policy).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_legacy_terrain_map_stays_in_the_shipped_format() {
        const NAME: &str = crate::enhanced::legacy_terrain_map::UOP_ENTRY_PATH;
        let dir = temp_dir("legacy_terrain_map");
        let path = package_with(&dir, "GameData.uop", &[(NAME, b"legacyId,newId,newSubType\r\n1,0,1\r\n", CompressionFlag::Zlib)]);
        let mut editor = UopEntryEditor::load(&path).unwrap();
        let target = EntryTarget::Name(NAME.to_owned());
        let policy = FormatPolicy::VerifiedOnly;
        let put = editor.replace(&target, b"legacyId,newId,newSubType\r\n1,0,1\r\n2,4,0\r\n", policy).unwrap();
        assert_eq!(put.check, EntryCheck::Parsed { reader: "legacy terrain map", dialect: None });
        let error = format!("{:#}", editor.replace(&target, b"legacyId,newId,newSubType\n1,0,1\n", policy).unwrap_err());
        assert!(error.contains("not in the shipped format at line 1"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audio_checks_apply_by_hash_and_to_mp3s_without_frames() {
        /// An ID3v2.3 tag with no frames: "ID3", version 3.0, no flags, a size of 0.
        const EMPTY_ID3V2: [u8; 10] = [b'I', b'D', b'3', 3, 0, 0, 0, 0, 0, 0];
        const NOT_FRAMES: [u8; 64] = [1; 64];
        let dir = temp_dir("audio_by_hash");
        let mp3 = mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 4);
        let path = package_with(&dir, "Audio.uop", &[(MP3_NAME, &mp3, CompressionFlag::None)]);
        let mut editor = UopEntryEditor::load(&path).unwrap();
        let policy = FormatPolicy::VerifiedOnly;
        let by_name = EntryTarget::Name(MP3_NAME.to_owned());
        let by_hash = EntryTarget::Hash(hash_file_name_single(MP3_NAME));
        // The first frame past the window mssmp3.asi scans after the tag.
        let late = [&EMPTY_ID3V2[..], &vec![0; crate::audio::validate::MILES_MP3_SYNC_WINDOW + 1], &mp3].concat();
        let no_frames = [&EMPTY_ID3V2[..], &NOT_FRAMES].concat();
        for target in [&by_name, &by_hash] {
            let error = format!("{:#}", editor.replace(target, &late, policy).unwrap_err());
            assert!(error.contains("bytes in (mssmp3.asi scans"), "{target:?}: {error}");
            let error = format!("{:#}", editor.replace(target, &no_frames, policy).unwrap_err());
            assert!(error.contains("holds no MP3 stream"), "{target:?}: {error}");
        }
        let put = editor.replace(&by_hash, &mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 2), policy).unwrap();
        assert_eq!(put.check, EntryCheck::Parsed { reader: AUDIO_READER, dialect: None });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn named_parse_checks_apply_by_hash() {
        use crate::enhanced::audio::SOUND_TABLE_ENTRY;
        use crate::enhanced::legacy_terrain_map::UOP_ENTRY_PATH;
        const BAD_TABLE: &[u8] = b"not,a,sound,table\r\nx,y\r\n";
        let dir = temp_dir("named_by_hash");
        let audio = package_with(&dir, "Audio.uop", &[(SOUND_TABLE_ENTRY, b"#ID,FilePath\r\n0,Sounds\\a.mp3\r\n", CompressionFlag::Zlib)]);
        let mut editor = UopEntryEditor::load(&audio).unwrap();
        let target = EntryTarget::Hash(hash_file_name_single(SOUND_TABLE_ENTRY));
        let error = format!("{:#}", editor.replace(&target, BAD_TABLE, FormatPolicy::VerifiedOnly).unwrap_err());
        assert!(error.contains("does not parse as a sound table"), "{error}");
        let game_data = package_with(&dir, "GameData.uop", &[(UOP_ENTRY_PATH, b"legacyId,newId,newSubType\r\n1,0,1\r\n", CompressionFlag::Zlib)]);
        let mut editor = UopEntryEditor::load(&game_data).unwrap();
        let target = EntryTarget::Hash(hash_file_name_single(UOP_ENTRY_PATH));
        let error = format!("{:#}", editor.replace(&target, b"legacyId,newId,newSubType\n1,0,1\n", FormatPolicy::VerifiedOnly).unwrap_err());
        assert!(error.contains("not in the shipped format"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checks_interface_lua_and_xml_by_name_and_by_hash() {
        const LUA_NAME: &str = "data/interface/default/source/test.lua";
        const XML_NAME: &str = "data/interface/default/source/test.xml";
        const NOTE_NAME: &str = "data/interface/default/source/note.txt";
        let dir = temp_dir("interface_syntax");
        let path = package_with(
            &dir,
            "Interface.uop",
            &[
                (LUA_NAME, b"for i = 1, 2 do\r\n\tcontinue\r\nend\r\n", CompressionFlag::Zlib),
                (XML_NAME, b"<Interface a=\"1\"b=\"2\"></Interface>", CompressionFlag::Zlib),
            ],
        );
        let mut editor = UopEntryEditor::load(&path).unwrap();
        let policy = FormatPolicy::VerifiedOnly;
        for target in [EntryTarget::Name(LUA_NAME.to_owned()), EntryTarget::Hash(hash_file_name_single(LUA_NAME))] {
            let put = editor.replace(&target, b"x = 1 & 3\r\n", policy).unwrap();
            assert_eq!(put.check, EntryCheck::Parsed { reader: "Lua script", dialect: None });
            let error = format!("{:#}", editor.replace(&target, b"x = 1\r\nif x then\r\n", policy).unwrap_err());
            assert!(error.contains("does not parse as a Lua script") && error.contains("line 3, column 1"), "{error}");
        }
        for target in [EntryTarget::Name(XML_NAME.to_owned()), EntryTarget::Hash(hash_file_name_single(XML_NAME))] {
            let error = format!("{:#}", editor.replace(&target, b"<Interface>\n<Window></Interface>", policy).unwrap_err());
            assert!(error.contains("does not parse as a TinyXML document") && error.contains("line 2, column"), "{error}");
        }
        // Text of another extension, or added by hash, is no script.
        let note = editor.add(&EntryTarget::Name(NOTE_NAME.to_owned()), b"not ( lua", None, policy).unwrap();
        assert_eq!(note.check, EntryCheck::Kind(ContentFormat::Text));
        let added = editor.add(&EntryTarget::Hash(hash_file_name_single("data/interface/x.txt")), b"not ( lua", None, policy).unwrap();
        assert_eq!(added.check, EntryCheck::Unchecked(ContentFormat::Text));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checks_classic_animation_frames_by_name_and_by_hash() {
        use crate::classic::anim_frames::uop::{ClassicUopEntry, ClassicUopFrame, FIRST_FRAME_NUMBER};
        use crate::classic::anim_frames::{ClassicFrame, PALETTE_COLOURS};
        use crate::classic::animationframe_cc::AnimationFrameCc;
        const BODY: u32 = 400;
        const ACTION: u16 = 3;
        const WHITE: u16 = 0x7FFF;
        const END_OF_FRAME: [u8; 4] = [0xFF, 0x7F, 0xFF, 0x7F];
        let entry = |colour: u16| {
            let mut palette = [0u16; PALETTE_COLOURS];
            palette[1] = colour;
            let mut entry = ClassicUopEntry::new(BODY, palette);
            let frame = ClassicFrame { center_x: 1, center_y: -2, width: 2, height: 1, pixels: vec![Some(1), None] };
            entry.frames.push(ClassicUopFrame { action: ACTION, frame_number: FIRST_FRAME_NUMBER, frame });
            entry.to_bytes().unwrap()
        };
        let name = AnimationFrameCc::animationframe_path(BODY, ACTION);
        let dir = temp_dir("classic_frames");
        let path = package_with(&dir, "AnimationFrame1.uop", &[(&name, &entry(WHITE), CompressionFlag::None)]);
        let mut editor = UopEntryEditor::load(&path).unwrap();
        let policy = FormatPolicy::VerifiedOnly;
        // The end of the frame overwritten: the runs read past the frame.
        let mut broken = entry(WHITE);
        let end = broken.windows(END_OF_FRAME.len()).rposition(|window| window == END_OF_FRAME).unwrap();
        broken[end..end + END_OF_FRAME.len()].fill(0);
        for target in [EntryTarget::Name(name.clone()), EntryTarget::Hash(AnimationFrameCc::animationframe_hash(BODY, ACTION))] {
            let put = editor.replace(&target, &entry(1), policy).unwrap();
            assert_eq!(put.check, EntryCheck::Parsed { reader: CLASSIC_ANIMATION_FRAMES_READER, dialect: None });
            let error = format!("{:#}", editor.replace(&target, &broken, policy).unwrap_err());
            assert!(error.contains(&format!("does not parse as a {CLASSIC_ANIMATION_FRAMES_READER}")), "{error}");
        }
        let added = editor.add(&EntryTarget::Name(AnimationFrameCc::animationframe_path(BODY, ACTION + 1)), &entry(WHITE), None, policy).unwrap();
        assert_eq!(added.check, EntryCheck::Parsed { reader: CLASSIC_ANIMATION_FRAMES_READER, dialect: None });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decodes_replaced_jpegs() {
        const NAME: &str = "data/test/picture.jpg";
        const SIDE: u32 = 8;
        const CUT_BYTES: usize = 16;
        let dir = temp_dir("jpeg");
        let path = package_with(&dir, "Test.uop", &[(NAME, &jpeg(SIDE), CompressionFlag::None)]);
        let mut editor = UopEntryEditor::load(&path).unwrap();
        let target = EntryTarget::Name(NAME.to_owned());
        let put = editor.replace(&target, &jpeg(SIDE * 2), FormatPolicy::VerifiedOnly).unwrap();
        assert_eq!(put.check, EntryCheck::Parsed { reader: "PNG, BMP or JPEG decode", dialect: None });
        let cut = jpeg(SIDE);
        let error = editor.replace(&target, &cut[..cut.len() - CUT_BYTES], FormatPolicy::VerifiedOnly).unwrap_err();
        assert!(error.to_string().contains("does not parse"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
