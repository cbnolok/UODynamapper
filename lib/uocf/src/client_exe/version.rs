//! The version resource of a PE image: the `VS_FIXEDFILEINFO` of the first `RT_VERSION` resource
//! (`VS_VERSIONINFO`), its file and product versions. Information only: no patch decision reads it (everything is
//! located by structure).
//!
//! The walk: the resource directory (data directory 2) → the `RT_VERSION` (16) type entry → its first name entry →
//! its first language entry → the data entry (RVA, size) → `VS_VERSIONINFO`: `wLength`, `wValueLength`, `wType`, the
//! UTF-16 key `VS_VERSION_INFO` and its NUL, padding to 32 bits, then the `VS_FIXEDFILEINFO` value.

use crate::client_exe::pe::PeImage;

/// `RT_VERSION`.
const RT_VERSION: u32 = 16;
/// `IMAGE_RESOURCE_DIRECTORY`: the counts of named and id entries, then the 8-byte entries.
const DIRECTORY_NAMED_COUNT_OFFSET: usize = 12;
const DIRECTORY_ID_COUNT_OFFSET: usize = 14;
const DIRECTORY_HEADER_SIZE: usize = 16;
const DIRECTORY_ENTRY_SIZE: usize = 8;
const ENTRY_OFFSET_OFFSET: usize = 4;
/// The high bit of an entry's offset marks a subdirectory; the rest is the offset from the resource directory.
const ENTRY_SUBDIRECTORY: u32 = 0x8000_0000;
/// `IMAGE_RESOURCE_DATA_ENTRY`: the data's RVA, then its size.
const DATA_ENTRY_SIZE_OFFSET: usize = 4;
/// `VS_VERSIONINFO`: `wLength`, `wValueLength`, `wType`, then the key.
const VERSION_INFO_VALUE_LENGTH_OFFSET: usize = 2;
#[cfg(test)]
const VERSION_INFO_TYPE_OFFSET: usize = 4;
const VERSION_INFO_KEY_OFFSET: usize = 6;
const VERSION_INFO_KEY: &str = "VS_VERSION_INFO";
const VERSION_INFO_ALIGNMENT: usize = 4;
/// `VS_FIXEDFILEINFO`: the signature, the structure version, then the file and product versions (MS, LS dwords).
const FIXED_FILE_INFO_SIGNATURE: u32 = 0xfeef_04bd;
const FIXED_FILE_INFO_SIZE: usize = 52;
const FIXED_FILE_VERSION_OFFSET: usize = 8;
const FIXED_PRODUCT_VERSION_OFFSET: usize = 16;
/// The number of parts of a version, `a.b.c.d`.
pub const VERSION_PARTS: usize = 4;
const VERSION_SEPARATOR: char = '.';

/// A four-part version, `a.b.c.d`. Serialized as its string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VersionNumber(pub [u16; VERSION_PARTS]);

impl VersionNumber {
    /// From the `MS` and `LS` dwords of `VS_FIXEDFILEINFO`.
    fn from_dwords(ms: u32, ls: u32) -> Self {
        let high = |dword: u32| (dword >> u16::BITS) as u16;
        let low = |dword: u32| dword as u16;
        Self([high(ms), low(ms), high(ls), low(ls)])
    }
}

impl std::fmt::Display for VersionNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c, d] = self.0;
        write!(f, "{a}{VERSION_SEPARATOR}{b}{VERSION_SEPARATOR}{c}{VERSION_SEPARATOR}{d}")
    }
}

impl TryFrom<String> for VersionNumber {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let parts: Vec<u16> = text
            .split(VERSION_SEPARATOR)
            .map(|part| part.trim().parse::<u16>().map_err(|_| format!("version `{text}`: expected a.b.c.d")))
            .collect::<Result<_, _>>()?;
        let parts: [u16; VERSION_PARTS] = parts.try_into().map_err(|_| format!("version `{text}`: expected a.b.c.d"))?;
        Ok(Self(parts))
    }
}

impl From<VersionNumber> for String {
    fn from(version: VersionNumber) -> Self {
        version.to_string()
    }
}

/// The file and product versions of an executable's version resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ExeVersion {
    pub file: VersionNumber,
    pub product: VersionNumber,
}

impl std::fmt::Display for ExeVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.file == self.product {
            write!(f, "{}", self.file)
        } else {
            write!(f, "file {}, product {}", self.file, self.product)
        }
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at.checked_add(size_of::<u16>())?)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(size_of::<u32>())?)?.try_into().ok()?))
}

/// The entries of the resource directory at file offset `directory`: `(name or id, offset)`.
fn directory_entries(bytes: &[u8], directory: usize) -> Option<Vec<(u32, u32)>> {
    let count = usize::from(u16_at(bytes, directory + DIRECTORY_NAMED_COUNT_OFFSET)?)
        + usize::from(u16_at(bytes, directory + DIRECTORY_ID_COUNT_OFFSET)?);
    (0..count)
        .map(|index| {
            let entry = directory + DIRECTORY_HEADER_SIZE + DIRECTORY_ENTRY_SIZE * index;
            Some((u32_at(bytes, entry)?, u32_at(bytes, entry + ENTRY_OFFSET_OFFSET)?))
        })
        .collect()
}

/// The subdirectory an entry offset names, as a file offset.
fn subdirectory(root: usize, offset: u32) -> Option<usize> {
    (offset & ENTRY_SUBDIRECTORY != 0).then(|| root + (offset & !ENTRY_SUBDIRECTORY) as usize)
}

/// The bytes of the first `RT_VERSION` resource.
fn version_resource<'a>(bytes: &'a [u8], pe: &PeImage) -> Option<&'a [u8]> {
    let (rva, size) = pe.resource_directory;
    if size == 0 {
        return None;
    }
    let root = pe.va_to_offset(pe.image_base.wrapping_add(rva))?;
    let (_, types) = directory_entries(bytes, root)?.into_iter().find(|(id, _)| *id == RT_VERSION)?;
    let names = subdirectory(root, types)?;
    let (_, languages) = *directory_entries(bytes, names)?.first()?;
    let languages = subdirectory(root, languages)?;
    let (_, data_entry) = *directory_entries(bytes, languages)?.first()?;
    if data_entry & ENTRY_SUBDIRECTORY != 0 {
        return None;
    }
    let data_entry = root + data_entry as usize;
    let data_rva = u32_at(bytes, data_entry)?;
    let data_size = u32_at(bytes, data_entry + DATA_ENTRY_SIZE_OFFSET)? as usize;
    let data = pe.va_to_offset(pe.image_base.wrapping_add(data_rva))?;
    bytes.get(data..data.checked_add(data_size)?)
}

/// The versions of the image's version resource; `None` without one, or when it does not parse.
pub fn read_exe_version(bytes: &[u8], pe: &PeImage) -> Option<ExeVersion> {
    let info = version_resource(bytes, pe)?;
    let value_length = usize::from(u16_at(info, VERSION_INFO_VALUE_LENGTH_OFFSET)?);
    if value_length < FIXED_FILE_INFO_SIZE {
        return None;
    }
    let key: Vec<u16> = VERSION_INFO_KEY.encode_utf16().chain([0]).collect();
    let key_end = VERSION_INFO_KEY_OFFSET + key.len() * size_of::<u16>();
    let stored: Option<Vec<u16>> = (VERSION_INFO_KEY_OFFSET..key_end).step_by(size_of::<u16>()).map(|at| u16_at(info, at)).collect();
    if stored? != key {
        return None;
    }
    let fixed = key_end.next_multiple_of(VERSION_INFO_ALIGNMENT);
    if u32_at(info, fixed)? != FIXED_FILE_INFO_SIGNATURE {
        return None;
    }
    let version = |at: usize| Some(VersionNumber::from_dwords(u32_at(info, fixed + at)?, u32_at(info, fixed + at + size_of::<u32>())?));
    Some(ExeVersion { file: version(FIXED_FILE_VERSION_OFFSET)?, product: version(FIXED_PRODUCT_VERSION_OFFSET)? })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::client_exe::pe::tests::{synthetic_pe, test_directory_entry, TEST_SECTION_ALIGNMENT};
    use crate::client_exe::pe::RESOURCE_DIRECTORY;

    const FILE_VERSION: VersionNumber = VersionNumber([4, 0, 102, 3]);
    const PRODUCT_VERSION: VersionNumber = VersionNumber([4, 0, 102, 0]);
    /// `wType` of a binary value.
    const BINARY_VALUE: u16 = 0;
    /// The language id of the one language entry (en-US).
    const LANGUAGE_EN_US: u32 = 0x409;
    const NAME_ID: u32 = 1;

    fn directory(entries: &[(u32, u32)]) -> Vec<u8> {
        let mut bytes = vec![0u8; DIRECTORY_HEADER_SIZE];
        bytes[DIRECTORY_ID_COUNT_OFFSET..][..2].copy_from_slice(&(entries.len() as u16).to_le_bytes());
        for (id, offset) in entries {
            bytes.extend(id.to_le_bytes());
            bytes.extend(offset.to_le_bytes());
        }
        bytes
    }

    fn dwords(version: VersionNumber) -> [u32; 2] {
        let [a, b, c, d] = version.0.map(u32::from);
        [a << u16::BITS | b, c << u16::BITS | d]
    }

    /// A `VS_VERSIONINFO` with the fixed info of `file` and `product` and no children.
    fn version_info(file: VersionNumber, product: VersionNumber) -> Vec<u8> {
        let mut info = vec![0u8; VERSION_INFO_KEY_OFFSET];
        info[VERSION_INFO_VALUE_LENGTH_OFFSET..][..2].copy_from_slice(&(FIXED_FILE_INFO_SIZE as u16).to_le_bytes());
        info[VERSION_INFO_TYPE_OFFSET..][..2].copy_from_slice(&BINARY_VALUE.to_le_bytes());
        for unit in VERSION_INFO_KEY.encode_utf16().chain([0]) {
            info.extend(unit.to_le_bytes());
        }
        info.resize(info.len().next_multiple_of(VERSION_INFO_ALIGNMENT), 0);
        let mut fixed = vec![0u8; FIXED_FILE_INFO_SIZE];
        fixed[..4].copy_from_slice(&FIXED_FILE_INFO_SIGNATURE.to_le_bytes());
        for (at, version) in [(FIXED_FILE_VERSION_OFFSET, file), (FIXED_PRODUCT_VERSION_OFFSET, product)] {
            let [ms, ls] = dwords(version);
            fixed[at..at + 4].copy_from_slice(&ms.to_le_bytes());
            fixed[at + 4..at + 8].copy_from_slice(&ls.to_le_bytes());
        }
        info.extend(fixed);
        let length = info.len() as u16;
        info[..2].copy_from_slice(&length.to_le_bytes());
        info
    }

    /// A `.rsrc` section (the image's first, at RVA [`TEST_SECTION_ALIGNMENT`]) holding one `RT_VERSION` resource.
    pub fn version_rsrc(file: VersionNumber, product: VersionNumber) -> Vec<u8> {
        // Three one-entry directories (types, names, languages), the data entry, the data.
        let directory_size = DIRECTORY_HEADER_SIZE + DIRECTORY_ENTRY_SIZE;
        let (name_dir, language_dir, data_entry) = (directory_size, 2 * directory_size, 3 * directory_size);
        let data = data_entry + DIRECTORY_HEADER_SIZE;
        let mut rsrc = directory(&[(RT_VERSION, ENTRY_SUBDIRECTORY | name_dir as u32)]);
        rsrc.extend(directory(&[(NAME_ID, ENTRY_SUBDIRECTORY | language_dir as u32)]));
        rsrc.extend(directory(&[(LANGUAGE_EN_US, data_entry as u32)]));
        let info = version_info(file, product);
        rsrc.extend((TEST_SECTION_ALIGNMENT + data as u32).to_le_bytes());
        rsrc.extend((info.len() as u32).to_le_bytes());
        rsrc.resize(data, 0);
        rsrc.extend(info);
        rsrc
    }

    /// Points data directory 2 of a synthetic image at its first section, of `size` bytes.
    pub fn set_resource_directory(image: &mut [u8], size: usize) {
        let entry = test_directory_entry(RESOURCE_DIRECTORY);
        image[entry..entry + 4].copy_from_slice(&TEST_SECTION_ALIGNMENT.to_le_bytes());
        image[entry + 4..entry + 8].copy_from_slice(&(size as u32).to_le_bytes());
    }

    #[test]
    fn the_fixed_file_info_is_read() {
        let rsrc = version_rsrc(FILE_VERSION, PRODUCT_VERSION);
        let size = rsrc.len();
        let mut image = synthetic_pe(&[(".rsrc", rsrc), (".text", vec![0; 16])], false);
        let pe = PeImage::parse(&image).unwrap();
        assert_eq!(read_exe_version(&image, &pe), None, "no resource directory");
        set_resource_directory(&mut image, size);
        let pe = PeImage::parse(&image).unwrap();
        let version = read_exe_version(&image, &pe).unwrap();
        assert_eq!(version, ExeVersion { file: FILE_VERSION, product: PRODUCT_VERSION });
        assert_eq!(version.to_string(), "file 4.0.102.3, product 4.0.102.0");
        assert_eq!(serde_json::to_string(&version).unwrap(), r#"{"file":"4.0.102.3","product":"4.0.102.0"}"#);
        assert_eq!(serde_json::from_str::<ExeVersion>(r#"{"file":"4.0.102.3","product":"4.0.102.0"}"#).unwrap(), version);
        assert!(serde_json::from_str::<VersionNumber>(r#""4.0.x""#).is_err());

        // A broken signature reads as no version.
        let signature = image.windows(4).position(|window| window == FIXED_FILE_INFO_SIGNATURE.to_le_bytes()).unwrap();
        image[signature] ^= 1;
        let pe = PeImage::parse(&image).unwrap();
        assert_eq!(read_exe_version(&image, &pe), None);
    }
}
