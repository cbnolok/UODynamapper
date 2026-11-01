//! Writing Classic Client sounds: the `soundidx.mul`/`sound.mul` pair ([`SoundMulFiles`]), `soundLegacyMUL.uop`, and
//! slots made from WAV or MP3 files ([`ClassicSoundEditor`]).
//!
//! - `sound.mul` as shipped (UO Gold, Mondain's Legacy, 7.0.20): the slots back to back in index order, no gaps;
//!   index entry `i` is `(offset, length, i + 1)`, an empty one `(-1, -1, i + 1)`. [`SoundMulFiles`] keeps every
//!   index entry's third field and the fields of empty entries as read, and lays the slots out that way, so an
//!   unchanged pair is written back byte for byte. A slot added past the end of the index extends it with
//!   `(-1, -1, i + 1)` entries.
//! - `soundLegacyMUL.uop`: one uncompressed entry per slot, `build/soundlegacymul/%08d.dat`. A replaced entry keeps
//!   its compression flag and an added one is uncompressed, as every shipped one; both get a fresh timestamp data
//!   block header.
//! - A slot is the 40-byte header and mono 16-bit PCM, which the client plays at 22,048 Hz. A replaced slot keeps
//!   its header, its name field overwritten when a new name is given (the name, a NUL, and the old bytes after it:
//!   the shipped name fields hold leftovers after the NUL) and bytes 32..40 kept. A new slot's header is the name
//!   and zeros: no client reads bytes 0..40 (`Voice_SetData` skips them), so zeros are safe.
//! - Slot ids go up to [`MAX_SOUND_ID`], the highest id the client plays.
//! - Files: 16-bit PCM WAV, mono, at 22,048 Hz, taken as they are; 22,050 Hz (the nominal rate the PCM was made at)
//!   is taken too, with a note that the client plays it 0.01% slower. Other rates are refused: the client would play
//!   them at the wrong speed. With `convert`, any WAV or MP3 [`crate::audio::convert`] decodes at those rates,
//!   downmixed to mono. There is no resampling.
//! - Verdata patches are not applied or written: the editor works on the base files.

use std::path::{Path, PathBuf};

use crate::audio::convert::{decode_file, downmix_to_mono, MONO};
use crate::audio::wav::{PCM_WAV_BITS_PER_SAMPLE, WAVE_FORMAT_PCM};
use crate::audio::{probe_audio, AudioContent};
use crate::classic::sound::{
    sound_uop_entry_name, BITS_PER_SAMPLE, MAX_SOUND_ID, NOMINAL_SAMPLE_RATE, SAMPLE_RATE, SOUND_HEADER_BYTES,
    SOUND_IDX_FILE_NAME, SOUND_MUL_FILE_NAME, SOUND_NAME_BYTES, SOUND_UOP_FILE_NAME,
};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{FileReplacement, LoadMode, NewEntryCompression, UopPackage};
use crate::utils::path::find_file_ignoring_case;

crate::eyre_imports!();

/// The index fields of an empty slot.
pub const EMPTY_INDEX_FIELD: u32 = u32::MAX;
pub const SOUND_INDEX_ENTRY_BYTES: usize = 12;
const INDEX_FIELD_BYTES: usize = size_of::<u32>();
/// The longest name a header holds with its NUL.
pub const SOUND_NAME_MAX_LEN: usize = SOUND_NAME_BYTES - 1;
/// Bytes 32..40 of a new slot's header: no client reads them.
pub const NEW_SOUND_HEADER_TAIL: [u8; SOUND_HEADER_BYTES - SOUND_NAME_BYTES] = [0; SOUND_HEADER_BYTES - SOUND_NAME_BYTES];
/// The rates a file's PCM is taken at: the rate the client plays, and the nominal one (with a note).
pub const CLASSIC_PCM_RATES: [u32; 2] = [SAMPLE_RATE, NOMINAL_SAMPLE_RATE];
/// An added `soundLegacyMUL.uop` entry's compression, as every shipped one.
pub const NEW_SLOT_COMPRESSION: CompressionFlag = CompressionFlag::None;

/// One `soundidx.mul` entry as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundIndexEntry {
    pub offset: u32,
    pub length: u32,
    pub extra: u32,
}

impl SoundIndexEntry {
    pub fn is_empty(&self) -> bool {
        self.offset == EMPTY_INDEX_FIELD || self.length == EMPTY_INDEX_FIELD
    }

    /// The entry of slot `slot_id` the shipped files would have when empty.
    fn empty(slot_id: usize) -> Self {
        Self { offset: EMPTY_INDEX_FIELD, length: EMPTY_INDEX_FIELD, extra: slot_id as u32 + 1 }
    }
}

/// A `soundidx.mul`/`sound.mul` pair in memory: the index entries and each slot's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundMulFiles {
    entries: Vec<SoundIndexEntry>,
    slots: Vec<Option<Vec<u8>>>,
}

impl SoundMulFiles {
    pub fn load(idx_path: &Path, mul_path: &Path) -> eyre::Result<Self> {
        let read = |path: &Path| std::fs::read(path).wrap_err_with(|| format!("Failed to read {}", path.display()));
        Self::parse(&read(idx_path)?, &read(mul_path)?).wrap_err_with(|| format!("Failed to read {}", idx_path.display()))
    }

    /// Strict: the index is whole entries, and every slot lies inside `mul`.
    pub fn parse(idx: &[u8], mul: &[u8]) -> eyre::Result<Self> {
        if idx.len() % SOUND_INDEX_ENTRY_BYTES != 0 {
            eyre::bail!("{} bytes are not whole {SOUND_INDEX_ENTRY_BYTES}-byte index entries", idx.len());
        }
        let field = |entry: &[u8], index: usize| {
            u32::from_le_bytes(entry[index * INDEX_FIELD_BYTES..(index + 1) * INDEX_FIELD_BYTES].try_into().expect("four bytes"))
        };
        let mut entries = Vec::new();
        let mut slots = Vec::new();
        for (slot_id, raw) in idx.chunks_exact(SOUND_INDEX_ENTRY_BYTES).enumerate() {
            let entry = SoundIndexEntry { offset: field(raw, 0), length: field(raw, 1), extra: field(raw, 2) };
            let slot = if entry.is_empty() {
                None
            } else {
                let (start, length) = (entry.offset as usize, entry.length as usize);
                let bytes = start.checked_add(length).and_then(|end| mul.get(start..end)).ok_or_else(|| {
                    eyre!("slot {slot_id} (offset {start}, length {length}) lies outside the {} bytes of the data file", mul.len())
                })?;
                Some(bytes.to_vec())
            };
            entries.push(entry);
            slots.push(slot);
        }
        Ok(Self { entries, slots })
    }

    /// The index entries.
    pub fn slot_count(&self) -> usize {
        self.entries.len()
    }

    pub fn slot(&self, slot_id: u32) -> Option<&[u8]> {
        self.slots.get(slot_id as usize).and_then(|slot| slot.as_deref())
    }

    pub fn entry(&self, slot_id: u32) -> Option<SoundIndexEntry> {
        self.entries.get(slot_id as usize).copied()
    }

    /// Sets the bytes of a slot, extending the index with empty entries up to it.
    pub fn set_slot(&mut self, slot_id: u32, bytes: Vec<u8>) {
        let slot_id = slot_id as usize;
        while self.entries.len() <= slot_id {
            self.entries.push(SoundIndexEntry::empty(self.entries.len()));
            self.slots.push(None);
        }
        self.slots[slot_id] = Some(bytes);
    }

    /// Empties a slot; returns whether it held bytes.
    pub fn clear_slot(&mut self, slot_id: u32) -> bool {
        let Some(slot) = self.slots.get_mut(slot_id as usize) else { return false };
        let had = slot.take().is_some();
        if had {
            let extra = self.entries[slot_id as usize].extra;
            self.entries[slot_id as usize] = SoundIndexEntry { extra, ..SoundIndexEntry::empty(slot_id as usize) };
        }
        had
    }

    /// The index and data files: the slots back to back in index order.
    pub fn to_bytes(&self) -> eyre::Result<(Vec<u8>, Vec<u8>)> {
        let mut idx = Vec::with_capacity(self.entries.len() * SOUND_INDEX_ENTRY_BYTES);
        let mut mul = Vec::new();
        for (slot_id, (entry, slot)) in self.entries.iter().zip(&self.slots).enumerate() {
            let entry = match slot {
                Some(bytes) => {
                    let offset = u32::try_from(mul.len()).ok().filter(|offset| *offset != EMPTY_INDEX_FIELD);
                    let length = u32::try_from(bytes.len()).ok().filter(|length| *length != EMPTY_INDEX_FIELD);
                    let (Some(offset), Some(length)) = (offset, length) else {
                        eyre::bail!("slot {slot_id}: the data file outgrows the 32-bit offsets of the index");
                    };
                    mul.extend_from_slice(bytes);
                    SoundIndexEntry { offset, length, extra: entry.extra }
                }
                None => *entry,
            };
            for field in [entry.offset, entry.length, entry.extra] {
                idx.extend_from_slice(&field.to_le_bytes());
            }
        }
        Ok((idx, mul))
    }

    pub fn save(&self, idx_path: &Path, mul_path: &Path) -> eyre::Result<()> {
        let (idx, mul) = self.to_bytes()?;
        std::fs::write(idx_path, idx).wrap_err_with(|| format!("Failed to write {}", idx_path.display()))?;
        std::fs::write(mul_path, mul).wrap_err_with(|| format!("Failed to write {}", mul_path.display()))
    }
}

/// Checks a sound name: printable ASCII, 1 to [`SOUND_NAME_MAX_LEN`] bytes.
pub fn check_sound_name(name: &str) -> eyre::Result<()> {
    if name.is_empty() || name.len() > SOUND_NAME_MAX_LEN {
        eyre::bail!("the sound name `{name}` must have 1 to {SOUND_NAME_MAX_LEN} characters");
    }
    if let Some(bad) = name.chars().find(|c| !(' '..='~').contains(c)) {
        eyre::bail!("the sound name `{name}`: `{}` is not printable ASCII", bad.escape_default());
    }
    Ok(())
}

/// A slot header: `name` (when given), a NUL and the rest of `base`; without `base`, the name and zeros.
pub fn sound_header(name: Option<&str>, base: Option<&[u8]>) -> eyre::Result<[u8; SOUND_HEADER_BYTES]> {
    let mut header = [0u8; SOUND_HEADER_BYTES];
    match base {
        Some(base) if base.len() == SOUND_HEADER_BYTES => header.copy_from_slice(base),
        Some(base) => eyre::bail!("a sound header has {SOUND_HEADER_BYTES} bytes, not {}", base.len()),
        None => header[SOUND_NAME_BYTES..].copy_from_slice(&NEW_SOUND_HEADER_TAIL),
    }
    match name {
        Some(name) => {
            check_sound_name(name)?;
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[name.len()] = 0;
        }
        None if base.is_none() => eyre::bail!("a new sound slot needs a name"),
        None => {}
    }
    Ok(header)
}

/// The note for PCM taken at the nominal rate, which the client plays at [`SAMPLE_RATE`].
fn rate_note(rate: u32) -> Option<String> {
    (rate == NOMINAL_SAMPLE_RATE).then(|| format!("the file is {rate} Hz; the client plays every sound at {SAMPLE_RATE} Hz (0.01% slower)"))
}

/// The Classic PCM of a file, whether it was converted, and the note on its rate (see the module documentation).
pub fn classic_pcm_from_file(file: &[u8], convert: bool) -> eyre::Result<(Vec<u8>, bool, Option<String>)> {
    let content = probe_audio(file);
    if let AudioContent::Wav(analysis) = &content {
        let taken = analysis.format.filter(|format| {
            format.format_tag == WAVE_FORMAT_PCM
                && format.bits_per_sample == PCM_WAV_BITS_PER_SAMPLE
                && format.channels == MONO
                && CLASSIC_PCM_RATES.contains(&format.sample_rate)
        });
        if let (Some(_), Some(data), false) = (taken, analysis.data(file), analysis.data_truncated) {
            if data.len() % (usize::from(BITS_PER_SAMPLE) / u8::BITS as usize) != 0 {
                eyre::bail!("the WAV data is not whole 16-bit samples");
            }
            if data.is_empty() {
                eyre::bail!("the WAV holds no samples");
            }
            return Ok((data.to_vec(), false, taken.and_then(|format| rate_note(format.sample_rate))));
        }
    }
    let wanted = format!("16-bit PCM mono WAV at {SAMPLE_RATE} Hz (or {NOMINAL_SAMPLE_RATE} Hz)");
    if !convert {
        eyre::bail!("the file is {}, not {wanted}: convert it (--convert) or with an external tool", content.detail());
    }
    let decoded = downmix_to_mono(&decode_file(file)?);
    if !CLASSIC_PCM_RATES.contains(&decoded.sample_rate) {
        eyre::bail!(
            "the file is {} Hz: the client plays every sound at {SAMPLE_RATE} Hz, so it would play at the wrong speed, and \
             there is no resampler here (use `ffmpeg -ar {SAMPLE_RATE}` or sox)",
            decoded.sample_rate
        );
    }
    if decoded.samples.is_empty() {
        eyre::bail!("the file holds no samples");
    }
    Ok((decoded.pcm_le_bytes(), true, rate_note(decoded.sample_rate)))
}

/// What happened to a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicSoundWrite {
    Replaced,
    /// The slot already held these bytes.
    Unchanged,
    Added,
}

/// One written slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicSoundPut {
    pub slot_id: u32,
    pub name: String,
    pub write: ClassicSoundWrite,
    pub converted: bool,
    pub pcm_bytes: usize,
    pub warnings: Vec<String>,
}

enum ClassicSoundFiles {
    Mul { files: SoundMulFiles, idx_name: String, mul_name: String },
    Uop { package: UopPackage, file_name: String },
}

/// The sound files of a Classic client folder being edited: the `sound.mul` pair when present (as
/// [`crate::classic::sound::SoundMap`] prefers it), else `soundLegacyMUL.uop`.
pub struct ClassicSoundEditor {
    source_dir: PathBuf,
    files: ClassicSoundFiles,
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

impl ClassicSoundEditor {
    pub fn load(client_path: &Path) -> eyre::Result<Self> {
        let idx = find_file_ignoring_case(client_path, SOUND_IDX_FILE_NAME);
        let mul = find_file_ignoring_case(client_path, SOUND_MUL_FILE_NAME);
        let files = match (idx, mul) {
            (Some(idx), Some(mul)) => {
                ClassicSoundFiles::Mul { files: SoundMulFiles::load(&idx, &mul)?, idx_name: file_name_of(&idx), mul_name: file_name_of(&mul) }
            }
            _ => {
                let path = find_file_ignoring_case(client_path, SOUND_UOP_FILE_NAME).ok_or_else(|| {
                    eyre!("Neither {SOUND_IDX_FILE_NAME}/{SOUND_MUL_FILE_NAME} nor {SOUND_UOP_FILE_NAME} found in {}", client_path.display())
                })?;
                let package = UopPackage::load_with_mode(&path, LoadMode::Lazy).wrap_err_with(|| format!("Failed to load {}", path.display()))?;
                ClassicSoundFiles::Uop { package, file_name: file_name_of(&path) }
            }
        };
        Ok(Self { source_dir: client_path.to_path_buf(), files })
    }

    /// The files written: `sound.mul` and `soundidx.mul`, or `soundLegacyMUL.uop`, as named in the source folder.
    pub fn file_names(&self) -> Vec<&str> {
        match &self.files {
            ClassicSoundFiles::Mul { idx_name, mul_name, .. } => vec![idx_name, mul_name],
            ClassicSoundFiles::Uop { file_name, .. } => vec![file_name],
        }
    }

    /// The slot as stored; `None` when empty.
    pub fn slot_bytes(&self, slot_id: u32) -> eyre::Result<Option<Vec<u8>>> {
        match &self.files {
            ClassicSoundFiles::Mul { files, .. } => Ok(files.slot(slot_id).map(<[u8]>::to_vec)),
            ClassicSoundFiles::Uop { package, .. } => package
                .unpack_file_by_hash(hash_file_name_single(&sound_uop_entry_name(slot_id)))
                .wrap_err_with(|| format!("Failed to read {}", sound_uop_entry_name(slot_id))),
        }
    }

    /// Stores a whole slot (header and PCM).
    pub fn put_slot_bytes(&mut self, slot_id: u32, bytes: &[u8]) -> eyre::Result<ClassicSoundWrite> {
        if slot_id > MAX_SOUND_ID {
            eyre::bail!("sound slot {slot_id}: the client plays no sound id above {MAX_SOUND_ID}");
        }
        if bytes.len() <= SOUND_HEADER_BYTES {
            eyre::bail!("a sound slot is the {SOUND_HEADER_BYTES}-byte header and PCM; {} bytes hold no PCM", bytes.len());
        }
        let current = self.slot_bytes(slot_id)?;
        if current.as_deref() == Some(bytes) {
            return Ok(ClassicSoundWrite::Unchanged);
        }
        let write = if current.is_some() { ClassicSoundWrite::Replaced } else { ClassicSoundWrite::Added };
        match &mut self.files {
            ClassicSoundFiles::Mul { files, .. } => files.set_slot(slot_id, bytes.to_vec()),
            ClassicSoundFiles::Uop { package, .. } => {
                let hash = hash_file_name_single(&sound_uop_entry_name(slot_id));
                match package.get_file_by_hash(hash).map(|file| file.compression()) {
                    Some(compression) => {
                        let replacement = package.replace_file(hash, bytes, compression)?;
                        debug_assert_eq!(replacement, FileReplacement::Replaced);
                    }
                    None => package.add_entry(hash, bytes, NewEntryCompression::Flag(NEW_SLOT_COMPRESSION))?,
                }
            }
        }
        Ok(write)
    }

    /// Stores the sound of a WAV or MP3 file in a slot (see the module documentation for the header and the file).
    pub fn put_sound(&mut self, slot_id: u32, file: &[u8], name: Option<&str>, convert: bool) -> eyre::Result<ClassicSoundPut> {
        let (pcm, converted, rate_note) = classic_pcm_from_file(file, convert)?;
        let current = self.slot_bytes(slot_id)?.filter(|bytes| bytes.len() >= SOUND_HEADER_BYTES);
        let header = sound_header(name, current.as_ref().map(|bytes| &bytes[..SOUND_HEADER_BYTES]))?;
        let mut warnings: Vec<String> = rate_note.into_iter().collect();
        if let ClassicSoundFiles::Mul { files, .. } = &self.files {
            if slot_id as usize >= files.slot_count() {
                warnings.push(format!("slot {slot_id} extends the index past its {} entries", files.slot_count()));
            }
        }
        let bytes = [header.as_slice(), pcm.as_slice()].concat();
        let write = self.put_slot_bytes(slot_id, &bytes)?;
        let name = crate::classic::sound::parse_sound_slot(slot_id, &bytes, crate::classic::sound::SoundSlotSource::Mul)
            .map(|sound| sound.name)
            .unwrap_or_default();
        Ok(ClassicSoundPut { slot_id, name, write, converted, pcm_bytes: pcm.len(), warnings })
    }

    /// Writes the files into `output_dir` (an absolute folder other than the source, created when missing);
    /// returns their paths.
    pub fn save_to(&mut self, output_dir: &Path) -> eyre::Result<Vec<PathBuf>> {
        if !output_dir.is_absolute() {
            eyre::bail!("the output folder must be an absolute path: {}", output_dir.display());
        }
        std::fs::create_dir_all(output_dir).wrap_err_with(|| format!("Failed to create {}", output_dir.display()))?;
        let canonical = |path: &Path| std::fs::canonicalize(path).wrap_err_with(|| format!("Failed to resolve {}", path.display()));
        if canonical(output_dir)? == canonical(&self.source_dir)? {
            eyre::bail!("refusing to overwrite the sound files of {}: write to another folder", self.source_dir.display());
        }
        match &mut self.files {
            ClassicSoundFiles::Mul { files, idx_name, mul_name } => {
                let (idx, mul) = (output_dir.join(&*idx_name), output_dir.join(&*mul_name));
                files.save(&idx, &mul)?;
                Ok(vec![idx, mul])
            }
            ClassicSoundFiles::Uop { package, file_name } => {
                let path = output_dir.join(&*file_name);
                package.finalize_and_save(&path).wrap_err_with(|| format!("Failed to write {}", path.display()))?;
                Ok(vec![path])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav;

    fn index(entries: &[(u32, u32, u32)]) -> Vec<u8> {
        entries.iter().flat_map(|&(offset, length, extra)| [offset, length, extra]).flat_map(u32::to_le_bytes).collect()
    }

    fn slot(name: &str, pcm: &[u8]) -> Vec<u8> {
        [sound_header(Some(name), None).unwrap().as_slice(), pcm].concat()
    }

    #[test]
    fn mul_pair_round_trips_and_grows() {
        let (a, b) = (slot("a.wav", &[1, 0]), slot("b.wav", &[2, 0, 3, 0]));
        let mul = [a.as_slice(), b.as_slice()].concat();
        let idx = index(&[(0, a.len() as u32, 1), (EMPTY_INDEX_FIELD, EMPTY_INDEX_FIELD, 2), (a.len() as u32, b.len() as u32, 3)]);
        let mut files = SoundMulFiles::parse(&idx, &mul).unwrap();
        assert_eq!(files.to_bytes().unwrap(), (idx.clone(), mul.clone()));
        assert_eq!(files.slot(2), Some(&b[..]));

        let c = slot("c.wav", &[4, 0]);
        files.set_slot(1, c.clone());
        files.set_slot(4, a.clone());
        assert!(files.clear_slot(2) && !files.clear_slot(2));
        let (idx2, mul2) = files.to_bytes().unwrap();
        let expected_idx = index(&[
            (0, a.len() as u32, 1),
            (a.len() as u32, c.len() as u32, 2),
            (EMPTY_INDEX_FIELD, EMPTY_INDEX_FIELD, 3),
            (EMPTY_INDEX_FIELD, EMPTY_INDEX_FIELD, 4),
            ((a.len() + c.len()) as u32, a.len() as u32, 5),
        ]);
        assert_eq!((idx2, mul2), (expected_idx, [a.as_slice(), c.as_slice(), a.as_slice()].concat()));

        assert!(SoundMulFiles::parse(&idx[..5], &mul).is_err());
        assert!(SoundMulFiles::parse(&index(&[(1, mul.len() as u32, 1)]), &mul).unwrap_err().to_string().contains("outside"));
    }

    #[test]
    fn headers_keep_their_leftovers() {
        let mut base = [0xccu8; SOUND_HEADER_BYTES];
        base[..8].copy_from_slice(b"old.wav\0");
        let renamed = sound_header(Some("dog.wav"), Some(&base)).unwrap();
        assert_eq!(&renamed[..8], b"dog.wav\0");
        assert_eq!(&renamed[8..], &base[8..]);
        assert_eq!(sound_header(None, Some(&base)).unwrap(), base);
        let new = sound_header(Some("x.wav"), None).unwrap();
        assert_eq!(&new[..6], b"x.wav\0");
        assert!(new[6..].iter().all(|&byte| byte == 0));
        assert!(sound_header(None, None).is_err());
        assert!(sound_header(Some(&"n".repeat(SOUND_NAME_BYTES)), None).is_err());
        assert!(sound_header(Some("caf\u{e9}"), None).is_err());
    }

    #[test]
    fn takes_classic_pcm_and_converts_on_request() {
        let pcm = [1u8, 0, 2, 0];
        assert_eq!(classic_pcm_from_file(&wav::encode_pcm_wav(SAMPLE_RATE, 1, &pcm), false).unwrap(), (pcm.to_vec(), false, None));
        let (nominal, _, note) = classic_pcm_from_file(&wav::encode_pcm_wav(NOMINAL_SAMPLE_RATE, 1, &pcm), false).unwrap();
        assert_eq!(nominal, pcm);
        assert!(note.unwrap().contains("plays every sound at 22048 Hz"));
        let stereo = wav::encode_pcm_wav(SAMPLE_RATE, 2, &[2, 0, 4, 0]);
        assert!(classic_pcm_from_file(&stereo, false).unwrap_err().to_string().contains("--convert"));
        assert_eq!(classic_pcm_from_file(&stereo, true).unwrap(), (vec![3, 0], true, None));
        let fast = wav::encode_pcm_wav(44_100, 1, &pcm);
        assert!(classic_pcm_from_file(&fast, true).unwrap_err().to_string().contains("no resampler"));
    }

    #[test]
    fn editor_writes_both_stores_to_another_folder() {
        let dir = std::env::temp_dir().join(format!("uocf_sound_edit_{}", std::process::id()));
        let (mul_dir, uop_dir, out) = (dir.join("mul"), dir.join("uop"), dir.join("out"));
        for folder in [&mul_dir, &uop_dir] {
            std::fs::create_dir_all(folder).unwrap();
        }
        let a = slot("a.wav", &[1, 0]);
        std::fs::write(mul_dir.join(SOUND_IDX_FILE_NAME), index(&[(0, a.len() as u32, 1)])).unwrap();
        std::fs::write(mul_dir.join(SOUND_MUL_FILE_NAME), &a).unwrap();
        let mut package = UopPackage::new_default();
        package.add_entry_by_name(&sound_uop_entry_name(0), &a, NewEntryCompression::Flag(CompressionFlag::Zlib)).unwrap();
        package.finalize_and_save(uop_dir.join(SOUND_UOP_FILE_NAME)).unwrap();

        let file = wav::encode_pcm_wav(SAMPLE_RATE, 1, &[9, 0]);
        for (source, written) in [(&mul_dir, vec![SOUND_IDX_FILE_NAME, SOUND_MUL_FILE_NAME]), (&uop_dir, vec![SOUND_UOP_FILE_NAME])] {
            let mut editor = ClassicSoundEditor::load(source).unwrap();
            assert_eq!(editor.file_names(), written);
            assert_eq!(editor.put_slot_bytes(0, &a).unwrap(), ClassicSoundWrite::Unchanged);
            let replaced = editor.put_sound(0, &file, None, false).unwrap();
            assert_eq!((replaced.write, replaced.name.as_str()), (ClassicSoundWrite::Replaced, "a.wav"));
            let added = editor.put_sound(3, &file, Some("new.wav"), false).unwrap();
            assert_eq!(added.write, ClassicSoundWrite::Added);
            assert!(editor.put_sound(MAX_SOUND_ID + 1, &file, Some("x.wav"), false).unwrap_err().to_string().contains("above 1667"));
            assert!(editor.save_to(source).is_err(), "never the source folder");
            assert!(editor.save_to(Path::new("relative")).is_err());
            editor.save_to(&out).unwrap();

            let saved = crate::classic::sound::SoundMap::load(&out).unwrap();
            assert_eq!(saved.read_slot(0).unwrap().unwrap().pcm_data, [9, 0]);
            assert_eq!(saved.read_slot(3).unwrap().unwrap().name, "new.wav");
            if source == &uop_dir {
                let package = UopPackage::load(out.join(SOUND_UOP_FILE_NAME)).unwrap();
                let compression = |slot| package.get_file_by_hash(hash_file_name_single(&sound_uop_entry_name(slot))).unwrap().compression();
                assert_eq!((compression(0), compression(3)), (CompressionFlag::Zlib, NEW_SLOT_COMPRESSION));
            }
            let _ = std::fs::remove_dir_all(&out);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
