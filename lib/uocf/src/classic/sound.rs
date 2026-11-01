//! Classic Client sounds: the slots of `soundidx.mul`/`sound.mul` or of `soundLegacyMUL.uop`
//! (`build/soundlegacymul/%08d.dat`), patched by `verdata.mul`, and the `Sound.def` translations
//! ([`crate::classic::audio_def`]).
//!
//! A slot is a 40-byte header and raw mono 16-bit little-endian PCM, which the client plays at 22,048 Hz (its
//! DirectSound buffers are created at 0x5620 Hz; the PCM is nominally 22,050 Hz). The header is the sound's name,
//! NUL-terminated in 32 bytes, then 8 bytes no client reads (`Voice_SetData` copies from byte 40 on): build-tool
//! leftovers by their look (`30 00 00 00 98 fb 18 00` throughout `soundLegacyMUL.uop`, pointer-like values or
//! MSVC's `cc` fill in UO Gold's `sound.mul`). A slot of 40 bytes or fewer counts as missing, and the client plays
//! no id above [`MAX_SOUND_ID`]; `scripts/dev-helpers/ec-research/ghidra-audio/results/classic_audio.md` has the
//! code. The Enhanced Client's `data/audio/legacysounds/*.wav` hold the same PCM bytes as these slots,
//! from byte 40 on. [`ClassicSound`] keeps the header as stored, so a slot can be written back unchanged.

crate::eyre_imports!();

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::classic::audio_def::{DefTranslationTable, SOUND_DEF_FILE_NAME};
use crate::classic::generic_index::IndexFile;
use crate::classic::verdata::{VerFileId, Verdata};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{LoadMode, UopPackage};
use crate::utils::path::find_file_ignoring_case;

pub const SOUND_NAME_BYTES: usize = 32;
/// The name and the 8 unknown bytes before the PCM.
pub const SOUND_HEADER_BYTES: usize = 40;
pub const WAV_HEADER_BYTES: usize = crate::audio::wav::PCM_WAV_HEADER_SIZE;
/// The rate the client plays every slot at.
pub const SAMPLE_RATE: u32 = 22_048;
/// The rate the PCM was made at, which files often carry.
pub const NOMINAL_SAMPLE_RATE: u32 = 22_050;
/// The highest id `PlaySound` plays (0x683).
pub const MAX_SOUND_ID: u32 = 1667;
pub const CHANNELS: u16 = 1;
pub const BITS_PER_SAMPLE: u16 = 16;
const BITS_PER_BYTE: u16 = 8;

pub const SOUND_IDX_FILE_NAME: &str = "soundidx.mul";
pub const SOUND_MUL_FILE_NAME: &str = "sound.mul";
pub const SOUND_UOP_FILE_NAME: &str = "soundLegacyMUL.uop";
/// The entry name of slot `id` in `soundLegacyMUL.uop`: `build/soundlegacymul/` and the id in 8 digits.
pub const SOUND_UOP_ENTRY_PREFIX: &str = "build/soundlegacymul/";
pub const SOUND_UOP_ENTRY_EXTENSION: &str = ".dat";
/// Sound ids are 16-bit in the protocol: the UOP entries searched by id.
pub const SOUND_UOP_SLOT_LIMIT: u32 = 1 << u16::BITS;

/// The `soundLegacyMUL.uop` entry name of a slot.
pub fn sound_uop_entry_name(slot_id: u32) -> String {
    format!("{SOUND_UOP_ENTRY_PREFIX}{slot_id:08}{SOUND_UOP_ENTRY_EXTENSION}")
}

/// Where a slot was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundSlotSource {
    Mul,
    /// A `soundLegacyMUL.uop` entry, stored with this compression.
    Uop(CompressionFlag),
    Verdata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicSound {
    pub slot_id: u32,
    pub name: String,
    /// The slot's first [`SOUND_HEADER_BYTES`] bytes as stored: the name field and the 8 unknown bytes.
    pub header: Vec<u8>,
    pub pcm_data: Vec<u8>,
    pub source: SoundSlotSource,
}

impl ClassicSound {
    pub fn wav_bytes(&self) -> Vec<u8> {
        encode_wav(&self.pcm_data)
    }

    pub fn duration_seconds(&self) -> f64 {
        let frame_bytes = f64::from(CHANNELS * BITS_PER_SAMPLE / BITS_PER_BYTE);
        self.pcm_data.len() as f64 / frame_bytes / f64::from(SAMPLE_RATE)
    }

    /// The slot bytes as stored: header and PCM.
    pub fn slot_bytes(&self) -> Vec<u8> {
        [self.header.as_slice(), self.pcm_data.as_slice()].concat()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundLookup {
    pub sound: ClassicSound,
    pub requested_id: u32,
    pub translated: bool,
    /// For a translated id, the `Sound.def` targets the client picks from at random on every play.
    pub targets: Vec<u32>,
}

#[derive(Clone)]
enum SoundStore {
    Mul { idx_file: IndexFile, mul_data: Vec<u8> },
    Uop { package: Arc<UopPackage>, slot_hashes: HashMap<u32, u64> },
}

#[derive(Clone)]
pub struct SoundMap {
    client_path: PathBuf,
    store: SoundStore,
    translations: DefTranslationTable,
    verdata: Option<Arc<Verdata>>,
}

impl SoundMap {
    /// `soundidx.mul` and `sound.mul` when both exist, otherwise `soundLegacyMUL.uop` (file names matched ignoring
    /// case), and `Sound.def` when present, parsed strictly.
    pub fn load(client_path: impl AsRef<Path>) -> eyre::Result<Self> {
        let client_path = client_path.as_ref().to_path_buf();
        let idx_path = find_file_ignoring_case(&client_path, SOUND_IDX_FILE_NAME);
        let mul_path = find_file_ignoring_case(&client_path, SOUND_MUL_FILE_NAME);
        let store = match (idx_path, mul_path) {
            (Some(idx_path), Some(mul_path)) => {
                let idx_file = IndexFile::load(idx_path)?;
                let mut mul_data = Vec::new();
                File::open(&mul_path)
                    .and_then(|mut file| file.read_to_end(&mut mul_data))
                    .wrap_err_with(|| format!("Failed to read {}", mul_path.display()))?;
                SoundStore::Mul { idx_file, mul_data }
            }
            _ => {
                let uop_path = find_file_ignoring_case(&client_path, SOUND_UOP_FILE_NAME).ok_or_else(|| {
                    eyre!(
                        "Neither {SOUND_IDX_FILE_NAME}/{SOUND_MUL_FILE_NAME} nor {SOUND_UOP_FILE_NAME} found in {}",
                        client_path.display()
                    )
                })?;
                let package = UopPackage::load_with_mode(&uop_path, LoadMode::Lazy)
                    .wrap_err_with(|| format!("Failed to load {}", uop_path.display()))?;
                let slot_hashes = (0..SOUND_UOP_SLOT_LIMIT)
                    .map(|slot_id| (slot_id, hash_file_name_single(&sound_uop_entry_name(slot_id))))
                    .filter(|(_, hash)| package.get_file_by_hash(*hash).is_some())
                    .collect();
                SoundStore::Uop { package: Arc::new(package), slot_hashes }
            }
        };
        let translations = match find_file_ignoring_case(&client_path, SOUND_DEF_FILE_NAME) {
            Some(path) => DefTranslationTable::load(&path)?.unwrap_or_default(),
            None => DefTranslationTable::default(),
        };
        Ok(Self { client_path, store, translations, verdata: None })
    }

    /// The client folder has `sound.mul` or `soundLegacyMUL.uop`, which [`Self::load`] reads.
    pub fn has_sound_files(client_path: &Path) -> bool {
        [SOUND_MUL_FILE_NAME, SOUND_UOP_FILE_NAME].iter().any(|name| find_file_ignoring_case(client_path, name).is_some())
    }

    pub fn with_verdata(mut self, verdata: Arc<Verdata>) -> Self {
        self.verdata = Some(verdata);
        self
    }

    pub fn client_path(&self) -> &Path {
        &self.client_path
    }

    /// The slots files hold: `soundLegacyMUL.uop` or `sound.mul`.
    pub fn source_label(&self) -> &'static str {
        match self.store {
            SoundStore::Mul { .. } => SOUND_MUL_FILE_NAME,
            SoundStore::Uop { .. } => SOUND_UOP_FILE_NAME,
        }
    }

    /// The slot ids to scan: the index length, or one past the highest UOP entry.
    pub fn slot_count(&self) -> usize {
        match &self.store {
            SoundStore::Mul { idx_file, .. } => idx_file.element_count(),
            SoundStore::Uop { slot_hashes, .. } => slot_hashes.keys().max().map_or(0, |&max| max as usize + 1),
        }
    }

    pub fn translation_count(&self) -> usize {
        self.translations.len()
    }

    pub fn translations(&self) -> &DefTranslationTable {
        &self.translations
    }

    /// The slot as stored (header and PCM), with its source; `None` for an empty or missing slot.
    pub fn read_slot_bytes(&self, slot_id: u32) -> eyre::Result<Option<(Vec<u8>, SoundSlotSource)>> {
        if let Some(verdata) = &self.verdata {
            if let Some(bytes) = verdata.read_patch(VerFileId::Sound, slot_id as i32)? {
                return Ok(Some((bytes, SoundSlotSource::Verdata)));
            }
        }
        match &self.store {
            SoundStore::Mul { idx_file, mul_data } => {
                let Ok(entry) = idx_file.element(slot_id as usize) else {
                    return Ok(None);
                };
                let index_patch = self.verdata.as_ref().and_then(|verdata| verdata.index_patch(VerFileId::SoundIdx, slot_id as i32));
                let Some((lookup, length, _extra)) =
                    index_patch.or_else(|| Some((entry.lookup()?, entry.len()?, entry.extra().unwrap_or(0))))
                else {
                    return Ok(None);
                };
                let (lookup, length) = (lookup as usize, length as usize);
                let end = lookup.checked_add(length).ok_or_else(|| eyre!("Sound slot {slot_id} range overflows usize."))?;
                if end > mul_data.len() {
                    eyre::bail!(
                        "Sound slot {slot_id} points outside {SOUND_MUL_FILE_NAME} (offset {lookup}, length {length}, file length {}).",
                        mul_data.len()
                    );
                }
                Ok(Some((mul_data[lookup..end].to_vec(), SoundSlotSource::Mul)))
            }
            SoundStore::Uop { package, slot_hashes } => {
                let Some(&hash) = slot_hashes.get(&slot_id) else {
                    return Ok(None);
                };
                let compression = package.get_file_by_hash(hash).map(|file| file.compression()).unwrap_or(CompressionFlag::None);
                let bytes = package
                    .unpack_file_by_hash(hash)
                    .wrap_err_with(|| format!("Failed to read {}", sound_uop_entry_name(slot_id)))?;
                Ok(bytes.map(|bytes| (bytes, SoundSlotSource::Uop(compression))))
            }
        }
    }

    pub fn read_slot(&self, slot_id: u32) -> eyre::Result<Option<ClassicSound>> {
        Ok(self.read_slot_bytes(slot_id)?.and_then(|(bytes, source)| parse_sound_slot(slot_id, &bytes, source)))
    }

    /// The slot of `sound_id`; when it is missing, the first of its `Sound.def` targets (as the client reads them)
    /// that holds a sound. The client picks one target at random on every play: [`SoundLookup::targets`] lists them.
    pub fn read_id(&self, sound_id: u32) -> eyre::Result<Option<SoundLookup>> {
        if let Some(sound) = self.read_slot(sound_id)? {
            return Ok(Some(SoundLookup { sound, requested_id: sound_id, translated: false, targets: Vec::new() }));
        }
        let Some(targets) = self.translations.get(sound_id).and_then(|translation| translation.client_targets()) else {
            return Ok(None);
        };
        for &slot_id in &targets {
            if let Some(sound) = self.read_slot(slot_id)? {
                return Ok(Some(SoundLookup { sound, requested_id: sound_id, translated: true, targets }));
            }
        }
        Ok(None)
    }

    /// How the client uses `Sound.def`: which lines apply (their id's slot is missing), which never do, and what it
    /// reads differently from the text ([`DefTranslationTable::client_notes`]).
    pub fn translation_notes(&self) -> eyre::Result<Vec<String>> {
        let mut applied = Vec::new();
        let mut never = 0usize;
        for translation in self.translations.iter().filter(|translation| translation.client_ignored.is_none()) {
            if self.read_slot(translation.id)?.is_some() {
                never += 1;
            } else {
                applied.push(translation.id);
            }
        }
        let mut notes = vec![format!(
            "{SOUND_DEF_FILE_NAME}: lines apply only to ids without a sound of their own: {} apply (ids {applied:?}), {never} \
             never apply",
            applied.len()
        )];
        notes.extend(self.translations.client_notes().into_iter().map(|note| format!("{SOUND_DEF_FILE_NAME} {note}")));
        Ok(notes)
    }
}

/// A PCM WAV file of Classic sound PCM.
pub fn encode_wav(pcm_data: &[u8]) -> Vec<u8> {
    crate::audio::wav::encode_pcm_wav(SAMPLE_RATE, CHANNELS, pcm_data)
}

fn decode_sound_name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim_end().to_string()
}

/// A slot of more than [`SOUND_HEADER_BYTES`] bytes.
pub fn parse_sound_slot(slot_id: u32, bytes: &[u8], source: SoundSlotSource) -> Option<ClassicSound> {
    if bytes.len() <= SOUND_HEADER_BYTES {
        return None;
    }
    Some(ClassicSound {
        slot_id,
        name: decode_sound_name(&bytes[..SOUND_NAME_BYTES]),
        header: bytes[..SOUND_HEADER_BYTES].to_vec(),
        pcm_data: bytes[SOUND_HEADER_BYTES..].to_vec(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn wav_header_matches_classic_sound_pcm() {
        let pcm = [1u8, 2, 3, 4];
        let wav = encode_wav(&pcm);

        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 40);
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(u16::from_le_bytes(wav[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), CHANNELS);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), SAMPLE_RATE);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), pcm.len() as u32);
        assert_eq!(&wav[WAV_HEADER_BYTES..], &pcm);
    }

    #[test]
    fn slot_header_is_kept_and_pcm_starts_after_it() {
        let mut slot = vec![0u8; SOUND_HEADER_BYTES];
        slot[..5].copy_from_slice(b"clang");
        slot[SOUND_NAME_BYTES..].copy_from_slice(&[0x30, 0, 0, 0, 0x98, 0xfb, 0x18, 0]);
        slot.extend_from_slice(&[1, 2]);
        let sound = parse_sound_slot(3, &slot, SoundSlotSource::Mul).unwrap();
        assert_eq!(sound.name, "clang");
        assert_eq!(sound.pcm_data, [1, 2]);
        assert_eq!(sound.slot_bytes(), slot);
        assert!(parse_sound_slot(3, &slot[..SOUND_HEADER_BYTES], SoundSlotSource::Mul).is_none());
    }

    #[test]
    fn sound_map_reads_slots_and_sound_def_translations() {
        let unique = format!(
            "uocf_sound_test_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).unwrap();

        let result = write_sound_pair_and_read(&dir);
        let _ = std::fs::remove_file(dir.join(SOUND_IDX_FILE_NAME));
        let _ = std::fs::remove_file(dir.join(SOUND_MUL_FILE_NAME));
        let _ = std::fs::remove_file(dir.join(SOUND_DEF_FILE_NAME));
        let _ = std::fs::remove_dir(&dir);

        result.unwrap();
    }

    fn write_sound_pair_and_read(dir: &Path) -> std::io::Result<()> {
        let mut payload = [0u8; SOUND_HEADER_BYTES].to_vec();
        payload[..5].copy_from_slice(b"clang");
        payload.extend_from_slice(&[1u8, 2, 3, 4]);

        let mut idx = std::fs::File::create(dir.join(SOUND_IDX_FILE_NAME))?;
        idx.write_all(&0u32.to_le_bytes())?;
        idx.write_all(&(payload.len() as u32).to_le_bytes())?;
        idx.write_all(&1u32.to_le_bytes())?;
        std::fs::write(dir.join(SOUND_MUL_FILE_NAME), payload)?;
        std::fs::write(dir.join(SOUND_DEF_FILE_NAME), "7 {-1, 0} 0\r\n8 {0 1\r\n")?;

        let sounds = SoundMap::load(dir).unwrap();
        let slot = sounds.read_slot(0).unwrap().unwrap();
        assert_eq!(slot.name, "clang");
        assert_eq!(slot.pcm_data, [1u8, 2, 3, 4]);
        assert_eq!(slot.source, SoundSlotSource::Mul);

        let lookup = sounds.read_id(7).unwrap().unwrap();
        assert!(lookup.translated);
        assert_eq!(lookup.requested_id, 7);
        assert_eq!(lookup.targets, [1, 0], "the client reads -1 as 1");
        assert_eq!(lookup.sound.slot_id, 0, "slot 1 is missing");
        assert!(sounds.read_id(8).unwrap().is_none(), "the client drops `8 {{0 1`");
        let notes = sounds.translation_notes().unwrap();
        assert!(notes[0].contains("1 apply (ids [7]), 0 never apply"), "{notes:?}");

        std::fs::write(dir.join(SOUND_DEF_FILE_NAME), "7 {0} 0 trailing\r\n")?;
        let error = SoundMap::load(dir).err().unwrap().to_string();
        assert!(error.contains(":1:9: expected the end of the line"), "{error}");

        Ok(())
    }
}
