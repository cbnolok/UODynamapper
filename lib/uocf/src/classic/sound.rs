#![allow(dead_code)]
//! Classic Client sound codec for `soundidx.mul` and `sound.mul`.

crate::eyre_imports!();

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::classic::generic_index::IndexFile;
use crate::classic::verdata::{VerFileId, Verdata};

pub const SOUND_NAME_BYTES: usize = 32;
pub const WAV_HEADER_BYTES: usize = 44;
pub const SAMPLE_RATE: u32 = 22_050;
pub const CHANNELS: u16 = 1;
pub const BITS_PER_SAMPLE: u16 = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicSound {
    pub slot_id: u32,
    pub name: String,
    pub pcm_data: Vec<u8>,
}

impl ClassicSound {
    pub fn wav_bytes(&self) -> Vec<u8> {
        encode_wav(&self.pcm_data)
    }

    pub fn duration_seconds(&self) -> f64 {
        let bytes_per_sample = (BITS_PER_SAMPLE / 8) as f64;
        self.pcm_data.len() as f64 / SAMPLE_RATE as f64 / CHANNELS as f64 / bytes_per_sample
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundLookup {
    pub sound: ClassicSound,
    pub requested_id: u32,
    pub translated: bool,
}

#[derive(Clone)]
pub struct SoundMap {
    client_path: PathBuf,
    idx_file: IndexFile,
    mul_data: Vec<u8>,
    translations: HashMap<u32, u32>,
    verdata: Option<Arc<Verdata>>,
}

impl SoundMap {
    pub fn load(client_path: impl AsRef<Path>) -> eyre::Result<Self> {
        let client_path = client_path.as_ref().to_path_buf();
        let idx_path = client_path.join("soundidx.mul");
        let mul_path = client_path.join("sound.mul");

        let idx_file = IndexFile::load(idx_path)?;

        let mut mul_file = File::open(&mul_path)
            .wrap_err_with(|| format!("Failed to open {}", mul_path.display()))?;
        let mut mul_data = Vec::new();
        mul_file
            .read_to_end(&mut mul_data)
            .wrap_err_with(|| format!("Failed to read {}", mul_path.display()))?;

        let translations = load_sound_def_translations(&client_path.join("Sound.def"))?;

        Ok(Self {
            client_path,
            idx_file,
            mul_data,
            translations,
            verdata: None,
        })
    }

    pub fn with_verdata(mut self, verdata: Arc<Verdata>) -> Self {
        self.verdata = Some(verdata);
        self
    }

    pub fn slot_count(&self) -> usize {
        self.idx_file.element_count()
    }

    pub fn translation_count(&self) -> usize {
        self.translations.len()
    }

    pub fn read_slot(&self, slot_id: u32) -> eyre::Result<Option<ClassicSound>> {
        if let Some(verdata) = &self.verdata {
            if let Some(bytes) = verdata.read_patch(VerFileId::Sound, slot_id as i32)? {
                return Ok(parse_sound_slot(slot_id, &bytes));
            }
        }

        let entry = match self.idx_file.element(slot_id as usize) {
            Ok(entry) => entry,
            Err(_) => return Ok(None),
        };

        let index_patch = self
            .verdata
            .as_ref()
            .and_then(|verdata| verdata.index_patch(VerFileId::SoundIdx, slot_id as i32));
        let index_values = index_patch.or_else(|| {
            Some((entry.lookup()?, entry.len()?, entry.extra().unwrap_or(0)))
        });
        let Some((lookup, length, _extra)) = index_values else {
            return Ok(None);
        };

        if length as usize <= SOUND_NAME_BYTES {
            return Ok(None);
        }

        let lookup = lookup as usize;
        let length = length as usize;
        let end = lookup
            .checked_add(length)
            .ok_or_else(|| eyre!("Sound slot {} range overflows usize.", slot_id))?;

        if end > self.mul_data.len() {
            eyre::bail!(
                "Sound slot {} points outside sound.mul (offset {}, length {}, file length {}).",
                slot_id,
                lookup,
                length,
                self.mul_data.len()
            );
        }

        Ok(parse_sound_slot(slot_id, &self.mul_data[lookup..end]))
    }

    pub fn read_id(&self, sound_id: u32) -> eyre::Result<Option<SoundLookup>> {
        if let Some(sound) = self.read_slot(sound_id)? {
            return Ok(Some(SoundLookup {
                sound,
                requested_id: sound_id,
                translated: false,
            }));
        }

        let Some(&slot_id) = self.translations.get(&sound_id) else {
            return Ok(None);
        };

        Ok(self.read_slot(slot_id)?.map(|sound| SoundLookup {
            sound,
            requested_id: sound_id,
            translated: true,
        }))
    }
}

pub fn encode_wav(pcm_data: &[u8]) -> Vec<u8> {
    let mut wav = Vec::with_capacity(WAV_HEADER_BYTES + pcm_data.len());
    let data_len = pcm_data.len() as u32;
    let byte_rate = SAMPLE_RATE * CHANNELS as u32 * BITS_PER_SAMPLE as u32 / 8;
    let block_align = CHANNELS * BITS_PER_SAMPLE / 8;

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(data_len + 36).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm_data);

    wav
}

fn decode_sound_name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim_end().to_string()
}

fn parse_sound_slot(slot_id: u32, bytes: &[u8]) -> Option<ClassicSound> {
    if bytes.len() <= SOUND_NAME_BYTES {
        return None;
    }

    Some(ClassicSound {
        slot_id,
        name: decode_sound_name(&bytes[..SOUND_NAME_BYTES]),
        pcm_data: bytes[SOUND_NAME_BYTES..].to_vec(),
    })
}

fn load_sound_def_translations(path: &Path) -> eyre::Result<HashMap<u32, u32>> {
    if !path.is_file() {
        return Ok(HashMap::new());
    }

    let text = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("Failed to read {}", path.display()))?;
    let mut translations = HashMap::new();

    for line in text.lines() {
        if let Some((from, to)) = parse_sound_def_translation(line) {
            translations.insert(from, to);
        }
    }

    Ok(translations)
}

fn parse_sound_def_translation(line: &str) -> Option<(u32, u32)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let mut first_digits = String::new();
    for ch in line.chars() {
        if ch.is_ascii_digit() {
            first_digits.push(ch);
        } else {
            break;
        }
    }

    let open = line.find('{')?;
    let close = line[open + 1..].find('}')? + open + 1;
    let from = first_digits.parse().ok()?;
    let to = line[open + 1..close].trim().parse().ok()?;

    Some((from, to))
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
        assert_eq!(&wav[44..], &pcm);
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
        let _ = std::fs::remove_file(dir.join("soundidx.mul"));
        let _ = std::fs::remove_file(dir.join("sound.mul"));
        let _ = std::fs::remove_file(dir.join("Sound.def"));
        let _ = std::fs::remove_dir(&dir);

        result.unwrap();
    }

    fn write_sound_pair_and_read(dir: &Path) -> std::io::Result<()> {
        let mut payload = [0u8; SOUND_NAME_BYTES].to_vec();
        payload[..5].copy_from_slice(b"clang");
        payload.extend_from_slice(&[1u8, 2, 3, 4]);

        let mut idx = std::fs::File::create(dir.join("soundidx.mul"))?;
        idx.write_all(&0u32.to_le_bytes())?;
        idx.write_all(&(payload.len() as u32).to_le_bytes())?;
        idx.write_all(&1u32.to_le_bytes())?;
        std::fs::write(dir.join("sound.mul"), payload)?;
        std::fs::write(dir.join("Sound.def"), "7 {0} 0\n")?;

        let sounds = SoundMap::load(dir).unwrap();
        let slot = sounds.read_slot(0).unwrap().unwrap();
        assert_eq!(slot.name, "clang");
        assert_eq!(slot.pcm_data, [1u8, 2, 3, 4]);

        let lookup = sounds.read_id(7).unwrap().unwrap();
        assert!(lookup.translated);
        assert_eq!(lookup.requested_id, 7);
        assert_eq!(lookup.sound.slot_id, 0);

        Ok(())
    }
}
