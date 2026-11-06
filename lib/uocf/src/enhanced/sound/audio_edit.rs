//! Replacing and adding `Audio.uop` entries ([`AudioPackageEditor`]), and the table rows a new id needs.
//!
//! - Names follow the tables (`crate::enhanced::audio`), normalised as the client does (`\` to `/`, lowercase,
//!   nothing trimmed): a sound id's MP3 is its row's path, its legacy WAV the path under `legacysounds/`, a music
//!   id's MP3 its row's path. For Kingdom Reborn a sound id is the server's id, followed through
//!   `audio_effects.csv` to its sound structure ([`crate::enhanced::kr_audio`]).
//! - The Enhanced Client plays a sound only through a table row, so a playable sound is an MP3 entry and a row: a
//!   new id gets its row in `audio_sounds.csv` or `audio_music.csv` (written back with [`write_table`]), and an MP3
//!   added by name must be one a row already names (a shipped row naming no entry). For Kingdom Reborn a new server
//!   id gets a sound structure in `audio_sounds.csv` and its `audio_effects.csv` row
//!   ([`crate::enhanced::kr_audio::KrAudioTables::with_sound`]). A `legacysounds/` WAV can still be replaced or
//!   added, with a note that the Enhanced Client never plays it.
//! - Every file is checked against what the client's decoder (Miles 7.2c) plays ([`enhanced_format_report`])
//!   under a [`FormatPolicy`]. With [`AudioImportOptions::convert`] a WAV entry takes any file [`to_pcm16_wav`]
//!   converts (WAV of another sample format, MP3); an MP3 entry takes MP3 files only: there is no encoder.
//! - A replaced entry keeps its compression flag; an added one is stored like the shipped entries of its kind: MP3
//!   uncompressed, WAV and tables zlib-compressed ([`family_compression`]). Both get a fresh timestamp data block
//!   header (`UopPackage::edit_data_block_header`); untouched entries keep theirs and their place in the file.
//! - [`AudioPackageEditor::save_as`] writes a copy, to an absolute path other than the source.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::audio::convert::to_pcm16_wav;
use crate::audio::mp3::MpegLayer;
use crate::audio::validate::{enhanced_format_report, expected_format, kind_mismatch, FormatPolicy};
use crate::audio::{probe_audio, AudioContent, AudioFormat};
use crate::enhanced::audio::{
    check_table_path, legacy_sound_entry_name, read_tables, table_path_entry_name, write_table, AudioKind, AudioTableRow, AudioTables,
    AUDIO_ENTRY_ROOT, LEGACY_SOUNDS_ROOT, MAX_SERVER_SOUND_ID, MUSIC_TABLE_ENTRY, MUSIC_TABLE_HEADER, SOUND_TABLE_ENTRY,
    SOUND_TABLE_HEADER,
};
use crate::enhanced::kr_audio::{KrAudioTables, KR_EFFECTS_TABLE_ENTRY};
use crate::enhanced::uop_entries::save_package_as;
use crate::uop_container::file::{CompressionFlag, UopFile};
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{FileReplacement, NewEntryCompression, UopPackage};

crate::eyre_imports!();

/// How a file becomes an entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AudioImportOptions {
    pub policy: FormatPolicy,
    /// Convert a file a WAV entry cannot take as it is to a 16-bit PCM WAV of its rate and channels.
    pub convert: bool,
}

/// Which existing entry to replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioTarget {
    /// A table id's entry.
    Id(AudioKind, u32),
    /// An entry name (`data/audio/...`, lowercase, `/` separators).
    Name(String),
}

/// Which new entry to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewAudio {
    /// A table id's entry. An id without a row needs `path` (a sound table path for [`AudioKind::Sound`] and
    /// [`AudioKind::LegacySound`], a music one for [`AudioKind::Music`]) and gets a row; `loops` sets a new music
    /// row's flag. For an id with a row, `path` and `loops` must match it when given.
    Id { kind: AudioKind, id: u32, path: Option<String>, loops: Option<bool> },
    /// An entry by name, without a table row.
    Name(String),
}

/// What happened to an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioWrite {
    Replaced,
    /// The entry already held the file: it keeps its bytes and header.
    Unchanged,
    Added,
}

/// One replaced or added entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioPut {
    pub name: String,
    pub hash: u64,
    pub write: AudioWrite,
    pub compression: CompressionFlag,
    /// The file was converted to a 16-bit PCM WAV.
    pub converted: bool,
    /// The format problems [`FormatPolicy::AllowUnverified`] let through.
    pub warnings: Vec<String>,
    /// The tables that got a row for the id (Kingdom Reborn's: both).
    pub tables_with_new_rows: Vec<&'static str>,
}

/// The table rows a new id gets.
enum NewRows {
    Enhanced(AudioKind, AudioTableRow),
    /// Kingdom Reborn's tables with the structure and its effects row.
    KingdomReborn(Box<KrAudioTables>),
}

/// A table entry written for a new row, and the entry it replaced (`None` when it was added).
struct TableWrite {
    entry: &'static str,
    hash: u64,
    previous: Option<UopFile>,
}

/// How an added entry of this content is stored, as the shipped entries are: MP3 (and anything else)
/// uncompressed, WAV and text zlib-compressed.
pub fn family_compression(content: &AudioContent) -> CompressionFlag {
    match content.format() {
        AudioFormat::Wav | AudioFormat::Text => CompressionFlag::Zlib,
        _ => CompressionFlag::None,
    }
}

/// Checks a name given for an entry: under `data/audio/`, lowercase, `/` separators, as every shipped name.
pub fn check_entry_name(name: &str) -> eyre::Result<()> {
    if !name.starts_with(AUDIO_ENTRY_ROOT) || name.len() <= AUDIO_ENTRY_ROOT.len() {
        eyre::bail!("`{name}`: Audio.uop entry names start with `{AUDIO_ENTRY_ROOT}`");
    }
    if name.chars().any(|c| c.is_ascii_uppercase() || c == '\\') {
        eyre::bail!("`{name}`: Audio.uop entry names are lowercase with `/` separators (the name is hashed as written)");
    }
    Ok(())
}

/// The entry name of a table row for `kind`.
fn row_entry_name(kind: AudioKind, row: &AudioTableRow) -> eyre::Result<String> {
    match kind {
        AudioKind::Sound | AudioKind::Music => Ok(row.entry_name()),
        AudioKind::LegacySound => legacy_sound_entry_name(&row.path)
            .ok_or_else(|| eyre!("sound {}: the path `{}` has no legacy sound name", row.id, row.path)),
    }
}

/// An `Audio.uop` being edited.
pub struct AudioPackageEditor {
    source: PathBuf,
    package: UopPackage,
    tables: AudioTables,
}

impl AudioPackageEditor {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let source = path.as_ref().to_path_buf();
        let package = UopPackage::load(&source).wrap_err_with(|| format!("Failed to load {}", source.display()))?;
        let tables = read_tables(&package, &source)?;
        Ok(Self { source, package, tables })
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn package(&self) -> &UopPackage {
        &self.package
    }

    /// The package's tables, with the rows added so far.
    pub fn tables(&self) -> &AudioTables {
        &self.tables
    }

    fn row(&self, kind: AudioKind, id: u32) -> Option<&AudioTableRow> {
        match (&self.tables, kind) {
            (AudioTables::Enhanced { music, .. }, AudioKind::Music) => music.get(&id),
            (AudioTables::Enhanced { sounds, .. }, _) => sounds.get(&id),
            _ => None,
        }
    }

    /// The entry name of a replacement target.
    pub fn target_name(&self, target: &AudioTarget) -> eyre::Result<String> {
        match target {
            AudioTarget::Name(name) => {
                check_entry_name(name)?;
                Ok(name.clone())
            }
            AudioTarget::Id(kind, id) => {
                if let Some(name) = self.kingdom_reborn_name(*kind, *id)? {
                    return Ok(name);
                }
                let row = self.row(*kind, *id).ok_or_else(|| self.missing_row_error(*kind, *id))?;
                row_entry_name(*kind, row)
            }
        }
    }

    /// For Kingdom Reborn's tables, the entry a server sound id plays; `Ok(None)` for other tables.
    fn kingdom_reborn_name(&self, kind: AudioKind, id: u32) -> eyre::Result<Option<String>> {
        let AudioTables::KingdomReborn(kr) = &self.tables else {
            return Ok(None);
        };
        if kind != AudioKind::Sound {
            eyre::bail!("{}: Kingdom Reborn's tables have sound ids only here; name the entry instead", self.source.display());
        }
        match kr.sound_for_server_id(id) {
            Some((_, Some(sound))) => Ok(Some(sound.entry_name())),
            Some((effect, None)) => eyre::bail!(
                "{}: audio_effects.csv line {}: server id {id} (ID# {}) names `{}`, which is no sound structure",
                self.source.display(),
                effect.line,
                effect.id,
                effect.structure
            ),
            None => eyre::bail!("{}: audio_effects.csv has no row for server id {id} (ID# {id} + 1)", self.source.display()),
        }
    }

    /// Whether a table row names the entry `name`, so the client can play it.
    fn row_names(&self, name: &str) -> bool {
        match &self.tables {
            AudioTables::Enhanced { sounds, music } => sounds.values().chain(music.values()).any(|row| row.entry_name() == name),
            AudioTables::KingdomReborn(kr) => kr.sounds.sounds().any(|row| row.entry_name() == name),
            AudioTables::Missing => false,
        }
    }

    fn missing_row_error(&self, kind: AudioKind, id: u32) -> eyre::Report {
        let table = if kind == AudioKind::Music { MUSIC_TABLE_ENTRY } else { SOUND_TABLE_ENTRY };
        match self.tables {
            AudioTables::Enhanced { .. } => eyre!("{}: {table} has no row for id {id}", self.source.display()),
            _ => eyre!("{}: the package has no Enhanced Client tables; name the entry instead", self.source.display()),
        }
    }

    /// The file as the entry `name` will hold it, with whether it was converted and the warnings.
    fn prepare(&self, name: &str, file: &[u8], options: AudioImportOptions) -> eyre::Result<(Vec<u8>, AudioContent, bool, Vec<String>)> {
        let mut payload = file.to_vec();
        let mut content = probe_audio(&payload);
        let mut converted = false;
        if options.convert && expected_format(name) == Some(AudioFormat::Wav) && !enhanced_format_report(name, &content).is_verified() {
            payload = to_pcm16_wav(file, false).wrap_err_with(|| format!("Failed to convert the file for {name}"))?;
            content = probe_audio(&payload);
            converted = true;
        }
        if let Some(mismatch) = kind_mismatch(name, &content) {
            let hint = if expected_format(name) == Some(AudioFormat::Mpeg(MpegLayer::L3)) {
                " There is no MP3 encoder here: encode the file with LAME or ffmpeg (CBR) first."
            } else if options.convert {
                ""
            } else {
                " --convert turns an MP3 into a WAV."
            };
            eyre::bail!("{name}: {mismatch}; a file of another kind is never stored under the name.{hint}");
        }
        let warnings = options.policy.apply(name, enhanced_format_report(name, &content))?;
        Ok((payload, content, converted, warnings))
    }

    /// Replaces an existing entry with `file`, keeping its compression flag.
    pub fn replace(&mut self, target: &AudioTarget, file: &[u8], options: AudioImportOptions) -> eyre::Result<AudioPut> {
        let name = self.target_name(target)?;
        let hash = hash_file_name_single(&name);
        let compression = self
            .package
            .get_file_by_hash(hash)
            .map(|entry| entry.compression())
            .ok_or_else(|| eyre!("{}: no entry `{name}` to replace (add it instead)", self.source.display()))?;
        let (payload, _, converted, warnings) = self.prepare(&name, file, options)?;
        let write = match self.package.replace_file(hash, &payload, compression).wrap_err_with(|| format!("Failed to replace {name}"))? {
            FileReplacement::Replaced => AudioWrite::Replaced,
            FileReplacement::Unchanged => AudioWrite::Unchanged,
        };
        Ok(AudioPut { name, hash, write, compression, converted, warnings, tables_with_new_rows: Vec::new() })
    }

    /// Adds a new entry holding `file`, and the table row its id needs: both or, when either fails, neither.
    pub fn add(&mut self, new: &NewAudio, file: &[u8], options: AudioImportOptions) -> eyre::Result<AudioPut> {
        let (name, new_row) = match new {
            NewAudio::Name(name) => {
                check_entry_name(name)?;
                let mp3 = expected_format(name) == Some(AudioFormat::Mpeg(MpegLayer::L3));
                if mp3 && !name.starts_with(LEGACY_SOUNDS_ROOT) && !self.row_names(name) {
                    eyre::bail!(
                        "{}: no table row names `{name}`, so the client would never play it; add it by id with a table path \
                         (the entry and its row together)",
                        self.source.display()
                    );
                }
                (name.clone(), None)
            }
            NewAudio::Id { kind, id, path, loops } => self.new_entry_name(*kind, *id, path.as_deref(), *loops)?,
        };
        let hash = hash_file_name_single(&name);
        if self.package.get_file_by_hash(hash).is_some() {
            eyre::bail!("{}: the entry `{name}` exists (replace it instead)", self.source.display());
        }
        let (payload, content, converted, warnings) = self.prepare(&name, file, options)?;
        let compression = family_compression(&content);
        // The tables are written first, and put back when the entry cannot be added.
        let tables = match &new_row {
            Some(rows) => self.write_tables_with_rows(rows)?,
            None => Vec::new(),
        };
        if let Err(error) = self.package.add_entry(hash, &payload, NewEntryCompression::Flag(compression)) {
            self.restore_tables(tables);
            return Err(error).wrap_err_with(|| format!("Failed to add {name}"));
        }
        let tables_with_new_rows = tables.iter().map(|table| table.entry).collect();
        match new_row {
            Some(NewRows::Enhanced(kind, row)) => {
                self.table_rows_mut(kind).insert(row.id, row);
            }
            Some(NewRows::KingdomReborn(kr)) => self.tables = AudioTables::KingdomReborn(kr),
            None => {}
        }
        Ok(AudioPut { name, hash, write: AudioWrite::Added, compression, converted, warnings, tables_with_new_rows })
    }

    /// The entry name of a new id entry, and the rows to add when the id has none.
    fn new_entry_name(&self, kind: AudioKind, id: u32, path: Option<&str>, loops: Option<bool>) -> eyre::Result<(String, Option<NewRows>)> {
        if let AudioTables::KingdomReborn(kr) = &self.tables {
            return self.new_kingdom_reborn_entry_name(kr, kind, id, path, loops);
        }
        if !matches!(self.tables, AudioTables::Enhanced { .. }) {
            return Err(self.missing_row_error(kind, id));
        }
        let music = kind == AudioKind::Music;
        if loops.is_some() && !music {
            eyre::bail!("the loop flag is for music rows only");
        }
        if let Some(row) = self.row(kind, id) {
            if path.is_some_and(|path| path != row.path) {
                eyre::bail!("id {id} has the row `{}` on line {}: give no path, or that one", row.path, row.line);
            }
            if loops.is_some() && loops != row.loops {
                eyre::bail!("music id {id} has the loop flag {:?} on line {}", row.loops, row.line);
            }
            return Ok((row_entry_name(kind, row)?, None));
        }
        let path = path.ok_or_else(|| eyre!("id {id} has no table row: give the path of a new one"))?;
        if !music && id > MAX_SERVER_SOUND_ID {
            eyre::bail!("sound id {id}: a server sends sound ids up to {MAX_SERVER_SOUND_ID}");
        }
        check_table_path(path, music)?;
        let row = AudioTableRow { id, path: path.to_owned(), loops: music.then_some(loops.unwrap_or(false)), line: 0 };
        let rows = NewRows::Enhanced(kind, row.clone());
        let name = row_entry_name(kind, &row)?;
        if kind == AudioKind::LegacySound {
            let mp3 = row_entry_name(AudioKind::Sound, &row)?;
            if self.package.get_file_by_hash(hash_file_name_single(&mp3)).is_none() {
                eyre::bail!(
                    "legacy sound id {id} has no table row, and a new row would name `{mp3}`, which the package does not \
                     hold: the Enhanced Client plays only the MP3 a row names, never the legacysounds/ WAV, so the row \
                     would play nothing. Add the MP3 first (sound id {id} with this table path), then the WAV by legacy \
                     sound id {id}, which takes that row"
                );
            }
        }
        Ok((name, Some(rows)))
    }

    /// [`Self::new_entry_name`] for Kingdom Reborn's tables: a server id with an `audio_effects.csv` row plays its
    /// structure's file (a path given must name it); one without gets a structure and its row, for `path`.
    fn new_kingdom_reborn_entry_name(
        &self,
        kr: &KrAudioTables,
        kind: AudioKind,
        id: u32,
        path: Option<&str>,
        loops: Option<bool>,
    ) -> eyre::Result<(String, Option<NewRows>)> {
        if loops.is_some() {
            eyre::bail!("{}: the loop flag is for the Enhanced Client's music rows only", self.source.display());
        }
        if kind != AudioKind::Sound || kr.effects.as_ref().and_then(|effects| effects.by_server_id(id)).is_some() {
            let name = self.kingdom_reborn_name(kind, id)?.expect("Kingdom Reborn's tables");
            if let Some(path) = path.filter(|path| table_path_entry_name(path) != name) {
                eyre::bail!("{}: server id {id} plays `{name}`: give no path, or one naming that file, not `{path}`", self.source.display());
            }
            return Ok((name, None));
        }
        let path = path.ok_or_else(|| {
            eyre!(
                "{}: {KR_EFFECTS_TABLE_ENTRY} has no row for server id {id} (ID# {id} + 1): give the path of a new sound \
                 structure",
                self.source.display()
            )
        })?;
        if id > MAX_SERVER_SOUND_ID {
            eyre::bail!("sound id {id}: a server sends sound ids up to {MAX_SERVER_SOUND_ID}");
        }
        let tables = kr.with_sound(&self.source.display().to_string(), id, path)?;
        Ok((table_path_entry_name(path), Some(NewRows::KingdomReborn(Box::new(tables)))))
    }

    fn table_rows_mut(&mut self, kind: AudioKind) -> &mut BTreeMap<u32, AudioTableRow> {
        let AudioTables::Enhanced { sounds, music } = &mut self.tables else { unreachable!("checked by new_entry_name") };
        if kind == AudioKind::Music { music } else { sounds }
    }

    /// Stores the text of the tables with `rows` added (the tables held are left as they are); returns what
    /// [`Self::restore_tables`] needs to undo it. A table that cannot be stored puts back those stored before it.
    fn write_tables_with_rows(&mut self, rows: &NewRows) -> eyre::Result<Vec<TableWrite>> {
        let texts = match rows {
            NewRows::Enhanced(kind, row) => {
                let (entry, header, with_loop) = match kind {
                    AudioKind::Music => (MUSIC_TABLE_ENTRY, MUSIC_TABLE_HEADER, true),
                    AudioKind::Sound | AudioKind::LegacySound => (SOUND_TABLE_ENTRY, SOUND_TABLE_HEADER, false),
                };
                let mut table = self.table_rows_mut(*kind).clone();
                table.insert(row.id, row.clone());
                vec![(entry, write_table(header, &table, with_loop))]
            }
            NewRows::KingdomReborn(kr) => {
                let effects = kr.effects.as_ref().expect("KrAudioTables::with_sound adds the effects row");
                vec![(SOUND_TABLE_ENTRY, kr.sounds.to_text()), (KR_EFFECTS_TABLE_ENTRY, effects.to_text())]
            }
        };
        let mut written = Vec::new();
        for (entry, text) in texts {
            match self.write_table_text(entry, &text) {
                Ok(table) => written.push(table),
                Err(error) => {
                    self.restore_tables(written);
                    return Err(error);
                }
            }
        }
        Ok(written)
    }

    /// Stores `text` as the table entry `entry`, replaced keeping its flag or added zlib-compressed.
    fn write_table_text(&mut self, entry: &'static str, text: &str) -> eyre::Result<TableWrite> {
        let hash = hash_file_name_single(entry);
        let previous = self.package.get_file_by_hash(hash).cloned();
        match previous.as_ref().map(|file| file.compression()) {
            Some(compression) => {
                self.package.replace_file(hash, text.as_bytes(), compression).wrap_err_with(|| format!("Failed to write {entry}"))?;
            }
            None => self
                .package
                .add_entry(hash, text.as_bytes(), NewEntryCompression::Flag(family_compression(&AudioContent::Text)))
                .wrap_err_with(|| format!("Failed to add {entry}"))?,
        }
        Ok(TableWrite { entry, hash, previous })
    }

    /// Puts the table entries back as they were before [`Self::write_tables_with_rows`], the last written first.
    fn restore_tables(&mut self, tables: Vec<TableWrite>) {
        for table in tables.into_iter().rev() {
            match table.previous {
                Some(file) => *self.package.get_file_by_hash_mut(table.hash).expect("the table was just written") = file,
                None => {
                    self.package.remove_entry(table.hash);
                }
            }
        }
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

    const SOUND_TABLE: &str = "#ID,FilePath\r\n0,Sounds\\a\\b.mp3\r\n";
    const MUSIC_TABLE: &str = "#ID,FilePath,Loop\r\n0,Music/turfin.mp3,1\r\n";
    const SOUND_MP3: &str = "data/audio/sounds/a/b.mp3";
    const LEGACY_WAV: &str = "data/audio/legacysounds/a/b.wav";
    /// FILETIME of the edited entries.
    const EDIT_FILETIME: u64 = 0x01db_2a3b_4c5d_6e7f;

    fn mp3_file() -> Vec<u8> {
        mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 4)
    }

    /// A package with both tables, the MP3 of sound 0 and its legacy WAV (zlib), saved to a temporary folder.
    fn package_dir(test: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("uocf_audio_edit_{test}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut package = UopPackage::new_default();
        let flag = NewEntryCompression::Flag;
        package.add_entry_by_name(SOUND_TABLE_ENTRY, SOUND_TABLE.as_bytes(), flag(CompressionFlag::Zlib)).unwrap();
        package.add_entry_by_name(MUSIC_TABLE_ENTRY, MUSIC_TABLE.as_bytes(), flag(CompressionFlag::Zlib)).unwrap();
        package.add_entry_by_name(SOUND_MP3, &mp3_file(), flag(CompressionFlag::None)).unwrap();
        package.add_entry_by_name(LEGACY_WAV, &wav::encode_pcm_wav(22_050, 1, &[0, 0]), flag(CompressionFlag::Zlib)).unwrap();
        let path = dir.join("Audio.uop");
        package.finalize_and_save(&path).unwrap();
        (dir, path)
    }

    fn unpack(package: &UopPackage, name: &str) -> Option<Vec<u8>> {
        package.unpack_file_by_hash(hash_file_name_single(name)).unwrap()
    }

    #[test]
    fn replaces_keeping_the_flag_and_adds_ids_with_their_rows() {
        let (dir, path) = package_dir("replace_add");
        let mut editor = AudioPackageEditor::load(&path).unwrap();
        editor.package.set_edit_filetime(Some(EDIT_FILETIME));
        let options = AudioImportOptions::default();

        let same = editor.replace(&AudioTarget::Id(AudioKind::Sound, 0), &mp3_file(), options).unwrap();
        assert_eq!(same.write, AudioWrite::Unchanged);

        let louder = wav::encode_pcm_wav(22_050, 1, &[1, 0, 2, 0]);
        let put = editor.replace(&AudioTarget::Id(AudioKind::LegacySound, 0), &louder, options).unwrap();
        assert_eq!((put.name.as_str(), put.write, put.compression), (LEGACY_WAV, AudioWrite::Replaced, CompressionFlag::Zlib));

        let error = editor.replace(&AudioTarget::Id(AudioKind::Sound, 0), &louder, options).unwrap_err().to_string();
        assert!(error.contains("the name asks for MP3, the file is WAV"), "{error}");
        assert!(error.contains("no MP3 encoder"), "{error}");
        let allowed = AudioImportOptions { policy: FormatPolicy::AllowUnverified, ..options };
        assert!(editor.replace(&AudioTarget::Id(AudioKind::Sound, 0), &louder, allowed).is_err(), "never a WAV as an MP3");
        let ogg = NewAudio::Name("data/audio/sounds/a/b.ogg".to_owned());
        assert!(editor.add(&ogg, &mp3_file(), allowed).unwrap_err().to_string().contains("neither"));

        let new = NewAudio::Id { kind: AudioKind::Sound, id: 5, path: Some("Sounds\\custom\\Bell.mp3".to_owned()), loops: None };
        let added = editor.add(&new, &mp3_file(), options).unwrap();
        assert_eq!(added.name, "data/audio/sounds/custom/bell.mp3");
        assert_eq!((added.compression, added.tables_with_new_rows), (CompressionFlag::None, vec![SOUND_TABLE_ENTRY]));
        let legacy = NewAudio::Id { kind: AudioKind::LegacySound, id: 5, path: None, loops: None };
        let float_wav = wav::tests::wav_with_format(wav::WAVE_FORMAT_IEEE_FLOAT, 32, &0.5f32.to_le_bytes());
        assert!(editor.add(&legacy, &float_wav, options).is_err(), "float WAV needs --convert");
        let converted = editor.add(&legacy, &float_wav, AudioImportOptions { convert: true, ..options }).unwrap();
        assert_eq!((converted.name.as_str(), converted.converted, converted.compression), ("data/audio/legacysounds/custom/bell.wav", true, CompressionFlag::Zlib));
        let music = NewAudio::Id { kind: AudioKind::Music, id: 3, path: Some("Music/Custom.mp3".to_owned()), loops: Some(true) };
        editor.add(&music, &mp3_file(), options).unwrap();
        assert!(editor.add(&music, &mp3_file(), options).unwrap_err().to_string().contains("exists"));

        let output = dir.join("Audio.edited.uop");
        assert!(editor.save_as(&path).is_err(), "never the source");
        assert!(editor.save_as(Path::new("relative.uop")).is_err());
        editor.save_as(&output).unwrap();

        let saved = UopPackage::load(&output).unwrap();
        let sounds = String::from_utf8(unpack(&saved, SOUND_TABLE_ENTRY).unwrap()).unwrap();
        assert_eq!(sounds, "#ID,FilePath\r\n0,Sounds\\a\\b.mp3\r\n5,Sounds\\custom\\Bell.mp3\r\n");
        let music_table = String::from_utf8(unpack(&saved, MUSIC_TABLE_ENTRY).unwrap()).unwrap();
        assert_eq!(music_table, "#ID,FilePath,Loop\r\n0,Music/turfin.mp3,1\r\n3,Music/Custom.mp3,1\r\n");
        assert_eq!(unpack(&saved, LEGACY_WAV).unwrap(), louder);
        let converted_wav = unpack(&saved, "data/audio/legacysounds/custom/bell.wav").unwrap();
        assert_eq!(converted_wav, wav::encode_pcm_wav(22_050, 1, &16_384i16.to_le_bytes()));
        assert_eq!(saved.get_file_by_hash(hash_file_name_single(SOUND_MP3)).unwrap().compression(), CompressionFlag::None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checks_names_rows_and_policy() {
        let (dir, path) = package_dir("checks");
        let mut editor = AudioPackageEditor::load(&path).unwrap();
        let options = AudioImportOptions::default();
        assert!(editor.replace(&AudioTarget::Name("data/audio/Sounds/x.mp3".to_owned()), &mp3_file(), options).is_err());
        assert!(editor.replace(&AudioTarget::Id(AudioKind::Music, 9), &mp3_file(), options).unwrap_err().to_string().contains("no row for id 9"));
        let wrong_path = NewAudio::Id { kind: AudioKind::LegacySound, id: 0, path: Some("Sounds\\z.mp3".to_owned()), loops: None };
        assert!(editor.add(&wrong_path, &mp3_file(), options).unwrap_err().to_string().contains("has the row"));
        let no_path = NewAudio::Id { kind: AudioKind::Sound, id: 8, path: None, loops: None };
        assert!(editor.add(&no_path, &mp3_file(), options).unwrap_err().to_string().contains("give the path"));
        let too_high = NewAudio::Id { kind: AudioKind::Sound, id: MAX_SERVER_SOUND_ID + 1, path: Some("Sounds\\x.mp3".to_owned()), loops: None };
        assert!(editor.add(&too_high, &mp3_file(), options).unwrap_err().to_string().contains("up to 65534"));
        // A legacy WAV's new row would name an MP3 the package does not hold: the Enhanced Client would play nothing.
        let legacy_first = NewAudio::Id { kind: AudioKind::LegacySound, id: 7, path: Some("Sounds\\custom\\only.mp3".to_owned()), loops: None };
        let error = editor.add(&legacy_first, &wav::encode_pcm_wav(22_048, 1, &[0, 0]), options).unwrap_err().to_string();
        assert!(error.contains("plays only the MP3") && error.contains("Add the MP3 first"), "{error}");
        assert!(editor.row(AudioKind::Sound, 7).is_none());
        // When the entry cannot be added after its row's table was written, the table entry is put back as it was.
        let row = AudioTableRow { id: 9, path: "Sounds\\custom\\nine.mp3".to_owned(), loops: None, line: 0 };
        let table_hash = hash_file_name_single(SOUND_TABLE_ENTRY);
        let stamp = |file: &UopFile| (file.data().map(|bytes| bytes.to_vec()), file.data_block_header().map(|bytes| bytes.to_vec()), file.data_block_hash());
        let before = stamp(editor.package.get_file_by_hash(table_hash).unwrap());
        let tables = editor.write_tables_with_rows(&NewRows::Enhanced(AudioKind::Sound, row)).unwrap();
        assert!(unpack(&editor.package, SOUND_TABLE_ENTRY).unwrap().ends_with(b"9,Sounds\\custom\\nine.mp3\r\n"));
        editor.restore_tables(tables);
        assert_eq!(stamp(editor.package.get_file_by_hash(table_hash).unwrap()), before);
        assert!(editor.row(AudioKind::Sound, 9).is_none());
        let unnamed = NewAudio::Name("data/audio/sounds/custom/orphan.mp3".to_owned());
        assert!(editor.add(&unnamed, &mp3_file(), options).unwrap_err().to_string().contains("no table row names"));
        let legacy = NewAudio::Name("data/audio/legacysounds/custom/orphan.wav".to_owned());
        let added = editor.add(&legacy, &wav::encode_pcm_wav(22_048, 1, &[0, 0]), options).unwrap();
        assert!(added.warnings.iter().any(|warning| warning.contains("never plays")), "{:?}", added.warnings);

        let target = AudioTarget::Id(AudioKind::LegacySound, 0);
        let wav_48k = wav::encode_pcm_wav(48_000, 1, &[0, 0]);
        let put = editor.replace(&target, &wav_48k, options).unwrap();
        assert_eq!(put.warnings.len(), 2, "Miles plays any rate; the notes stay: {:?}", put.warnings);
        assert!(put.warnings.iter().any(|warning| warning.contains("never plays `data/audio/legacysounds/`")), "{:?}", put.warnings);
        // 2 bytes of 16-bit stereo: not whole frames.
        let partial = wav::encode_pcm_wav(22_050, 2, &[0, 0]);
        assert!(editor.replace(&target, &partial, options).unwrap_err().to_string().contains("--allow-unverified-format"));
        let allowed = AudioImportOptions { policy: FormatPolicy::AllowUnverified, ..options };
        let put = editor.replace(&target, &partial, allowed).unwrap();
        assert_eq!(put.warnings.len(), 3, "{:?}", put.warnings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn follows_kingdom_reborn_server_ids() {
        let dir = std::env::temp_dir().join(format!("uocf_audio_edit_kr_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sounds = "w,s,id,filename,rate,vol,pan,offset,r1,r2,,,polygroup,type,,status,usage,notes\r\n\
            ,,bell,Sounds\\Custom\\Bell.mp3,1.0-1.1,*,*,*,*,*,*,1,*,*,*,,,\r\n";
        let effects = ";,ID#,description,sound,notes\r\n,1,bell\r\n,2,l_none\r\n";
        let mut package = UopPackage::new_default();
        let flag = NewEntryCompression::Flag(CompressionFlag::Zlib);
        package.add_entry_by_name(SOUND_TABLE_ENTRY, sounds.as_bytes(), flag).unwrap();
        package.add_entry_by_name(crate::enhanced::kr_audio::KR_EFFECTS_TABLE_ENTRY, effects.as_bytes(), flag).unwrap();
        let path = dir.join("Audio.uop");
        package.finalize_and_save(&path).unwrap();
        let mut editor = AudioPackageEditor::load(&path).unwrap();
        let options = AudioImportOptions::default();
        let bell = NewAudio::Id { kind: AudioKind::Sound, id: 0, path: None, loops: None };
        assert_eq!(editor.add(&bell, &mp3_file(), options).unwrap().name, "data/audio/sounds/custom/bell.mp3", "server id 0 is ID# 1");
        let target = AudioTarget::Id(AudioKind::Sound, 0);
        assert_eq!(editor.replace(&target, &mp3_file(), options).unwrap().write, AudioWrite::Unchanged);
        let error = editor.target_name(&AudioTarget::Id(AudioKind::Sound, 1)).unwrap_err().to_string();
        assert!(error.contains("names `l_none`, which is no sound structure"), "{error}");
        assert!(editor.target_name(&AudioTarget::Id(AudioKind::Sound, 2)).unwrap_err().to_string().contains("no row for server id 2"));
        let with_path = NewAudio::Id { kind: AudioKind::Sound, id: 0, path: Some("Sounds\\x.mp3".to_owned()), loops: None };
        let error = editor.add(&with_path, &mp3_file(), options).unwrap_err().to_string();
        assert!(error.contains("server id 0 plays `data/audio/sounds/custom/bell.mp3`: give no path"), "{error}");

        // A server id without an effects row gets a structure and its row, both or neither.
        let no_path = NewAudio::Id { kind: AudioKind::Sound, id: 7, path: None, loops: None };
        assert!(editor.add(&no_path, &mp3_file(), options).unwrap_err().to_string().contains("give the path of a new sound structure"));
        let taken = NewAudio::Id { kind: AudioKind::Sound, id: 7, path: Some("Sounds\\Custom\\Bell.mp3".to_owned()), loops: None };
        let error = format!("{:#}", editor.add(&taken, &mp3_file(), options).unwrap_err());
        assert!(error.contains("the entry `data/audio/sounds/custom/bell.mp3` exists"), "{error}");
        let wav = NewAudio::Id { kind: AudioKind::Sound, id: 7, path: Some("Sounds\\Custom\\Horn.mp3".to_owned()), loops: None };
        assert!(editor.add(&wav, &wav::encode_pcm_wav(22_050, 1, &[0, 0]), options).is_err(), "a WAV for an MP3 entry");
        assert!(editor.tables() == AudioPackageEditor::load(&path).unwrap().tables(), "nothing written on failure");
        let unpacked = |editor: &AudioPackageEditor, name: &str| String::from_utf8(unpack(editor.package(), name).unwrap()).unwrap();
        assert_eq!(unpacked(&editor, SOUND_TABLE_ENTRY), sounds);
        let added = editor.add(&wav, &mp3_file(), options).unwrap();
        assert_eq!(added.name, "data/audio/sounds/custom/horn.mp3");
        assert_eq!(added.tables_with_new_rows, vec![SOUND_TABLE_ENTRY, crate::enhanced::kr_audio::KR_EFFECTS_TABLE_ENTRY]);
        assert_eq!(unpacked(&editor, SOUND_TABLE_ENTRY), format!("{sounds},,Custom\\Horn,Sounds\\Custom\\Horn.mp3,1.0-1.1,*,*,*,*,*,*,1,*,*,*\r\n"));
        assert_eq!(unpacked(&editor, crate::enhanced::kr_audio::KR_EFFECTS_TABLE_ENTRY), format!("{effects},8,Custom\\Horn\r\n"));
        assert_eq!(editor.target_name(&AudioTarget::Id(AudioKind::Sound, 7)).unwrap(), added.name, "server id 7 is ID# 8");
        let saved = dir.join("Edited.uop");
        editor.save_as(&saved).unwrap();
        let reloaded = AudioPackageEditor::load(&saved).unwrap();
        assert_eq!(reloaded.target_name(&AudioTarget::Id(AudioKind::Sound, 7)).unwrap(), added.name);
        let again = NewAudio::Id { kind: AudioKind::Sound, id: 7, path: Some("Sounds\\Custom\\Horn2.mp3".to_owned()), loops: None };
        let error = editor.add(&again, &mp3_file(), options).unwrap_err().to_string();
        assert!(error.contains("server id 7 plays `data/audio/sounds/custom/horn.mp3`"), "{error}");
        let looping = NewAudio::Id { kind: AudioKind::Sound, id: 9, path: Some("Sounds\\a.mp3".to_owned()), loops: Some(true) };
        assert!(editor.add(&looping, &mp3_file(), options).unwrap_err().to_string().contains("loop flag"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
