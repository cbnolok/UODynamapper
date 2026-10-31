//! Replacing and adding whole entries of any client UOP package ([`UopEntryEditor`]), with the new file checked by
//! its content.
//!
//! - **Kind check.** The file is classified by its bytes ([`classify_content`]). Its kind must be one the entry
//!   may hold: the kind its name's extension declares ([`declared_format`]), text for a text extension
//!   ([`TEXT_EXTENSIONS`]), or the kind of the (non-empty) entry it replaces; a TGA with trailing bytes counts as a
//!   TGA and a BMP with a wrong size field as a BMP. An unnamed entry of plain binary takes only plain binary (the
//!   shipped 18-byte placeholders named `.tga` take a TGA or another placeholder). A mismatch is refused under
//!   every policy. An entry added without a name or a known extension has nothing to
//!   check against ([`EntryCheck::Unchecked`]).
//! - **Parse check.** Where a strict reader exists for the entry ([`PARSE_CHECKS`]), the file must parse. On a
//!   replace, the reader is the first one (and client dialect) the current entry parses under, so a reader is
//!   never applied to an entry it does not describe; on an add, the file must parse under one of the readers of
//!   its package and kind, when there are any.
//! - **Audio.** MP3 and WAV files also pass the `Audio.uop` format checks ([`crate::audio::validate`]) under the
//!   caller's [`FormatPolicy`].
//! - **Storage.** A replaced entry keeps its compression flag; an added one gets the flag most entries of its kind
//!   in the package have ([`UopEntryEditor::family_compression`]), else the package's majority. Edited entries
//!   get a fresh timestamp data block header (`uop_entries::put_entry`); the others keep their bytes and place.
//!   [`UopEntryEditor::save_as`] writes a copy to an absolute path other than the source.

use std::path::{Path, PathBuf};

use crate::audio::validate::{enhanced_format_problems, kind_mismatch, FormatPolicy};
use crate::audio::{probe_audio, AudioContent};
use crate::enhanced::client_dialect::ClientDialect;
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

    fn label(&self) -> String {
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
    /// Only the entry of this name, when set.
    pub name: Option<&'static str>,
    /// The reader reads both client dialects ([`DIALECTS`]); else it is called with the default one.
    pub dialects: bool,
    pub check: fn(&[u8], ClientDialect) -> eyre::Result<()>,
}

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
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::chaos_script::ChaosScript::parse_bytes(bytes)),
    },
    ParseCheck {
        label: "sound table",
        packages: &["audio"],
        kinds: &[ContentFormat::Text],
        name: Some(crate::enhanced::audio::SOUND_TABLE_ENTRY),
        dialects: false,
        check: |bytes, _| {
            use crate::enhanced::audio::{parse_table, SOUND_TABLE_ENTRY, SOUND_TABLE_HEADER};
            ok(parse_table(SOUND_TABLE_ENTRY, std::str::from_utf8(bytes)?, SOUND_TABLE_HEADER, false))
        },
    },
    ParseCheck {
        label: "music table",
        packages: &["audio"],
        kinds: &[ContentFormat::Text],
        name: Some(crate::enhanced::audio::MUSIC_TABLE_ENTRY),
        dialects: false,
        check: |bytes, _| {
            use crate::enhanced::audio::{parse_table, MUSIC_TABLE_ENTRY, MUSIC_TABLE_HEADER};
            ok(parse_table(MUSIC_TABLE_ENTRY, std::str::from_utf8(bytes)?, MUSIC_TABLE_HEADER, true))
        },
    },
    ParseCheck {
        label: "legacy terrain map",
        packages: &["gamedata"],
        kinds: &[ContentFormat::Text],
        name: Some(crate::enhanced::legacy_terrain_map::UOP_ENTRY_PATH),
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::legacy_terrain_map::LegacyTerrainMap::from_bytes(bytes)),
    },
    ParseCheck {
        label: "legacy terrain document",
        packages: &["legacyterrain"],
        kinds: &[ContentFormat::Xml],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::legacy_terrain::LegacyTerrainDocument::parse(bytes)),
    },
    ParseCheck {
        label: "hue bitmap",
        packages: &["hues"],
        kinds: &[ContentFormat::Bmp],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::hues::EcHueTable::from_bitmap(bytes)),
    },
    ParseCheck {
        label: "tileart record",
        packages: &["tileart"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::tileart::TileArtEntry::parse_bytes(bytes)),
    },
    ParseCheck {
        label: "animation definition",
        packages: &["animationdefinition"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: true,
        check: |bytes, dialect| ok(crate::enhanced::animation_definition::AnimationDefinition::parse_dialect(bytes, dialect)),
    },
    ParseCheck {
        label: "animation sequence",
        packages: &["animationsequence"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: true,
        check: |bytes, dialect| ok(crate::animation_sequence::AnimationSequence::parse_dialect(bytes, dialect)),
    },
    ParseCheck {
        label: "animation frames (AMOU)",
        packages: &[],
        kinds: &[ContentFormat::Amou],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::animationframe::AnimationFrame::load(bytes)),
    },
    ParseCheck {
        label: "effect definition collection",
        packages: &["effectdefinitioncollection"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: true,
        check: |bytes, dialect| {
            ok(crate::enhanced::effect_definition::parse_effect_definition_collection_dialect(bytes, dialect))
        },
    },
    ParseCheck {
        label: "terrain definition",
        packages: &["terraindefinition"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: true,
        check: |bytes, dialect| ok(crate::enhanced::terrain_definition::TerrainDefinitionEntry::from_bytes_dialect(bytes, dialect)),
    },
    ParseCheck {
        label: "facet definition",
        packages: &["facets"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::facet_definition::FacetDefinition::from_bytes(bytes)),
    },
    ParseCheck {
        label: "facet sector",
        packages: &["facet", "facetx"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: true,
        check: |bytes, dialect| ok(crate::enhanced::facet_decoder::decode_sector(bytes, dialect)),
    },
    ParseCheck {
        label: "facet data map",
        packages: &["facet", "facetx"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::facet_sector::FacetGeometry::from_data_map_payload(bytes)),
    },
    ParseCheck {
        label: "housing pieces",
        packages: &["multicollection"],
        kinds: &[ContentFormat::Binary],
        name: Some("build/multicollection/housing.bin"),
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::housing::HousingPieces::from_payload_bytes(bytes)),
    },
    ParseCheck {
        label: "multi",
        packages: &["multicollection"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: true,
        check: |bytes, dialect| ok(crate::enhanced::multis::MultiItem::from_payload_bytes_dialect(0, String::new(), bytes, dialect)),
    },
    ParseCheck {
        label: "waypoints",
        packages: &["waypoint"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::waypoints::WaypointsPackage::from_payload_bytes(bytes)),
    },
    ParseCheck {
        label: "string dictionary",
        packages: &["string_dictionary"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::string_dictionary::UoStringDictionary::from_payload_bytes(bytes, "entry")),
    },
    ParseCheck {
        label: "localized strings",
        packages: &["localizedstrings"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::localized_strings::LocalizedStringTable::from_payload_bytes(bytes)),
    },
    ParseCheck {
        label: "static chunk",
        packages: &["staticchunk"],
        kinds: &[ContentFormat::Binary],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::kr::facet::static_chunk::StaticChunk::decode(bytes)),
    },
    ParseCheck {
        label: "DDS header",
        packages: &[],
        kinds: &[ContentFormat::Dds],
        name: None,
        dialects: false,
        check: |bytes, _| ok(crate::enhanced::dds_writer::DdsDescription::parse(bytes)),
    },
    ParseCheck {
        label: "font table directory",
        packages: &[],
        kinds: &[ContentFormat::Font],
        name: None,
        dialects: false,
        check: |bytes, _| check_font(bytes),
    },
    ParseCheck {
        label: "PNG or BMP decode",
        packages: &[],
        kinds: &[ContentFormat::Png, ContentFormat::Bmp],
        name: None,
        dialects: false,
        check: |bytes, _| ok(image::load_from_memory(bytes).map_err(Into::into)),
    },
    ParseCheck {
        label: "TGA decode",
        packages: &[],
        kinds: &[ContentFormat::Tga],
        name: None,
        dialects: false,
        check: |bytes, _| ok(image::load_from_memory_with_format(bytes, image::ImageFormat::Tga).map_err(Into::into)),
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
}

impl UopEntryEditor {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let source = path.as_ref().to_path_buf();
        let package = UopPackage::load_with_mode(&source, LoadMode::Lazy).wrap_err_with(|| format!("Failed to load {}", source.display()))?;
        Ok(Self { source, package })
    }

    /// An editor over a loaded package (from `source`, which [`Self::save_as`] never overwrites).
    pub fn from_package(source: PathBuf, package: UopPackage) -> Self {
        Self { source, package }
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

    /// The parse checks of this package that apply to an entry `name` of `kind`.
    fn parse_checks(&self, name: Option<&str>, kind: ContentFormat) -> Vec<&'static ParseCheck> {
        let key = package_key(&self.source);
        PARSE_CHECKS
            .iter()
            .filter(|check| check.packages.is_empty() || check.packages.contains(&key.as_str()))
            .filter(|check| check.kinds.contains(&kind))
            .filter(|check| check.name.is_none_or(|wanted| name == Some(wanted)))
            .collect()
    }

    /// The first (reader, dialect) of `checks` that accepts `bytes`.
    fn first_parse(checks: &[&'static ParseCheck], bytes: &[u8]) -> Option<(&'static ParseCheck, ClientDialect)> {
        checks.iter().find_map(|check| {
            let dialects = if check.dialects { &DIALECTS[..] } else { &DIALECTS[..1] };
            dialects.iter().find(|&&dialect| (check.check)(bytes, dialect).is_ok()).map(|&dialect| (*check, dialect))
        })
    }

    /// Checks `file` for the entry `target` (current payload `current`): the kind, the parse, the audio formats.
    fn check(&self, target: &EntryTarget, current: Option<&[u8]>, file: &[u8], policy: FormatPolicy) -> eyre::Result<(EntryCheck, Vec<String>)> {
        let label = target.label();
        let kind = kind_family(classify_content(file).format);
        let current_kind = current.map(|bytes| kind_family(classify_content(bytes).format));
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
        if matches!(kind, ContentFormat::Mp3 | ContentFormat::Wav) {
            let content = probe_audio(file);
            if let (Some(name), AudioContent::Mpeg(_) | AudioContent::Wav(_)) = (name, &content) {
                if let Some(mismatch) = kind_mismatch(name, &content) {
                    eyre::bail!("{label}: {mismatch}");
                }
                warnings = policy.apply(name, enhanced_format_problems(name, &content))?;
                audio_checked = true;
            }
        }

        let checks = self.parse_checks(name, kind);
        let parsed = match current {
            Some(current) if current_kind == Some(kind) => Self::first_parse(&checks, current).map(|(check, dialect)| {
                (check.check)(file, dialect)
                    .map(|()| (check, dialect))
                    .wrap_err_with(|| format!("{label}: the file does not parse as a {} (the current entry does)", check.label))
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
            if kind_family(classify_content(&payload).format) != kind {
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
        let kind = kind_family(classify_content(file).format);
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
        let kind = kind_family(classify_content(file).format);
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
}
