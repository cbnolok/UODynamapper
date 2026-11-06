//! `palette.mul`, `animinfo.mul` and `sjis2uni.mul`, read as the 2024 Classic client reads them, strictly, and
//! written back byte for byte.
//!
//! Traced in the 2024 Classic client (`UOClassicDataFiles::Palette_Load`, `AnimInfo_Load`,
//! `scripts/dev-helpers/ec-research/ghidra-classic-text-tables`):
//!
//! - **`palette.mul`** ([`PaletteFile`]): 256 RGB triples, one `fread` of [`PALETTE_BYTES`]; the client's palette on
//!   an 8-bit desktop.
//! - **`animinfo.mul`** ([`AnimInfoFile`]): two-byte entries, the client reads [`ANIMINFO_CLIENT_ENTRIES`]. An
//!   entry with a zero byte is set to [`ANIMINFO_DEFAULT`] and the file reported corrupted; without the file every
//!   entry is the default. The 7.0.20 and later files have 2000 entries, UO Gold's and ML's 1000 (for those
//!   clients); every shipped entry is the default.
//! - **`sjis2uni.mul`** ([`ShiftJisToUnicodeFile`]): [`SJIS_ENTRIES`] little-endian 16-bit code units, the Unicode
//!   character of each Shift-JIS code (0 for none), for the Japanese auto-translation; the client looks in
//!   `language\` first, the shipped clients have it in their folder.
//!
//! Strict: a file of another size, and an `animinfo.mul` entry with a zero byte, are errors.

crate::eyre_imports!();
use std::path::Path;

use crate::utils::output_file::write_through_temp;

pub const PALETTE_FILE: &str = "palette.mul";
pub const ANIMINFO_FILE: &str = "animinfo.mul";
pub const SJIS_FILE: &str = "sjis2uni.mul";
/// Colours of `palette.mul`.
pub const PALETTE_COLORS: usize = 256;
const RGB_BYTES: usize = 3;
pub const PALETTE_BYTES: usize = PALETTE_COLORS * RGB_BYTES;
/// Bytes of an `animinfo.mul` entry.
pub const ANIMINFO_ENTRY_BYTES: usize = 2;
/// Entries the 2024 client reads.
pub const ANIMINFO_CLIENT_ENTRIES: usize = 2000;
/// What the client uses for a missing or corrupted entry.
pub const ANIMINFO_DEFAULT: [u8; ANIMINFO_ENTRY_BYTES] = [4, 2];
/// Shift-JIS codes of `sjis2uni.mul`: one per 16-bit value.
pub const SJIS_ENTRIES: usize = 1 << u16::BITS;
const SJIS_ENTRY_BYTES: usize = size_of::<u16>();

fn read(path: &Path) -> eyre::Result<Vec<u8>> {
    std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))
}

fn save(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    write_through_temp(path, |temp| std::fs::write(temp, bytes).wrap_err_with(|| format!("write {}", temp.display())))
}

/// `palette.mul`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteFile {
    pub colors: [[u8; RGB_BYTES]; PALETTE_COLORS],
}

impl PaletteFile {
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        if bytes.len() != PALETTE_BYTES {
            eyre::bail!("{PALETTE_FILE} has {} bytes, not {PALETTE_BYTES} ({PALETTE_COLORS} RGB colours)", bytes.len());
        }
        Ok(Self { colors: std::array::from_fn(|color| bytes[color * RGB_BYTES..][..RGB_BYTES].try_into().expect("three bytes")) })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        Self::from_bytes(&read(path)?).wrap_err_with(|| format!("{}", path.display()))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.colors.concat()
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        save(path, &self.to_bytes())
    }
}

/// `animinfo.mul`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimInfoFile {
    pub entries: Vec<[u8; ANIMINFO_ENTRY_BYTES]>,
}

impl AnimInfoFile {
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        if bytes.len() % ANIMINFO_ENTRY_BYTES != 0 {
            eyre::bail!("{ANIMINFO_FILE} has {} bytes, not whole {ANIMINFO_ENTRY_BYTES}-byte entries", bytes.len());
        }
        let entries: Vec<[u8; ANIMINFO_ENTRY_BYTES]> =
            bytes.chunks_exact(ANIMINFO_ENTRY_BYTES).map(|entry| entry.try_into().expect("two bytes")).collect();
        Self::check(&entries)?;
        Ok(Self { entries })
    }

    fn check(entries: &[[u8; ANIMINFO_ENTRY_BYTES]]) -> eyre::Result<()> {
        if let Some(index) = entries.iter().position(|entry| entry.contains(&0)) {
            eyre::bail!("{ANIMINFO_FILE} entry {index} holds a zero byte (the client reports the file corrupted and uses {ANIMINFO_DEFAULT:?})");
        }
        if entries.len() > ANIMINFO_CLIENT_ENTRIES {
            eyre::bail!("{ANIMINFO_FILE} has {} entries; the client reads {ANIMINFO_CLIENT_ENTRIES}", entries.len());
        }
        Ok(())
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        Self::from_bytes(&read(path)?).wrap_err_with(|| format!("{}", path.display()))
    }

    /// Entry `index` as the 2024 client holds it: the default past the file's end.
    pub fn client_entry(&self, index: usize) -> Option<[u8; ANIMINFO_ENTRY_BYTES]> {
        (index < ANIMINFO_CLIENT_ENTRIES).then(|| self.entries.get(index).copied().unwrap_or(ANIMINFO_DEFAULT))
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        Self::check(&self.entries)?;
        Ok(self.entries.concat())
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        save(path, &self.to_bytes()?)
    }
}

/// `sjis2uni.mul`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShiftJisToUnicodeFile {
    /// The UTF-16 code unit of each Shift-JIS code, 0 for none.
    pub code_units: Vec<u16>,
}

impl ShiftJisToUnicodeFile {
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        if bytes.len() != SJIS_ENTRIES * SJIS_ENTRY_BYTES {
            eyre::bail!("{SJIS_FILE} has {} bytes, not {SJIS_ENTRIES} 16-bit entries", bytes.len());
        }
        Ok(Self { code_units: bytes.chunks_exact(SJIS_ENTRY_BYTES).map(|unit| u16::from_le_bytes([unit[0], unit[1]])).collect() })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        Self::from_bytes(&read(path)?).wrap_err_with(|| format!("{}", path.display()))
    }

    /// The character of a Shift-JIS code, `None` for 0 or a lone surrogate.
    pub fn character(&self, sjis: u16) -> Option<char> {
        let unit = *self.code_units.get(usize::from(sjis))?;
        (unit != 0).then(|| char::from_u32(u32::from(unit))).flatten()
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        if self.code_units.len() != SJIS_ENTRIES {
            eyre::bail!("{SJIS_FILE} needs {SJIS_ENTRIES} entries, not {}", self.code_units.len());
        }
        Ok(self.code_units.iter().flat_map(|unit| unit.to_le_bytes()).collect())
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        save(path, &self.to_bytes()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_binaries_write_back_and_reject_other_sizes() {
        let palette: Vec<u8> = (0..PALETTE_BYTES).map(|byte| byte as u8).collect();
        let file = PaletteFile::from_bytes(&palette).unwrap();
        assert_eq!((file.colors[1], file.to_bytes()), ([3, 4, 5], palette.clone()));
        assert!(PaletteFile::from_bytes(&palette[1..]).is_err());

        let mut anim = AnimInfoFile::from_bytes(&[4, 2, 5, 1]).unwrap();
        assert_eq!((anim.client_entry(1), anim.client_entry(2), anim.client_entry(ANIMINFO_CLIENT_ENTRIES)), (Some([5, 1]), Some(ANIMINFO_DEFAULT), None));
        assert_eq!(anim.to_bytes().unwrap(), [4, 2, 5, 1]);
        assert!(AnimInfoFile::from_bytes(&[4, 0]).is_err());
        assert!(AnimInfoFile::from_bytes(&[4]).is_err());
        anim.entries[0] = [0, 2];
        assert!(anim.to_bytes().is_err());

        let mut sjis = vec![0u8; SJIS_ENTRIES * SJIS_ENTRY_BYTES];
        sjis[0x41 * SJIS_ENTRY_BYTES] = b'A';
        let file = ShiftJisToUnicodeFile::from_bytes(&sjis).unwrap();
        assert_eq!((file.character(0x41), file.character(0x42)), (Some('A'), None));
        assert_eq!(file.to_bytes().unwrap(), sjis);
        assert!(ShiftJisToUnicodeFile::from_bytes(&sjis[2..]).is_err());
    }
}
