//! The parts of a 32-bit PE image the patchers need: sections (to map virtual addresses to file offsets), the
//! checksum field and its algorithm, and the Authenticode (security) directory.

crate::eyre_imports!();

/// File offset of `e_lfanew`, the offset of the PE signature.
const E_LFANEW_OFFSET: usize = 0x3c;
const PE_SIGNATURE: &[u8; 4] = b"PE\0\0";
/// The COFF file header follows the signature.
const COFF_HEADER_OFFSET: usize = PE_SIGNATURE.len();
const COFF_MACHINE_OFFSET: usize = 0;
/// COFF machine of an x86 (i386) image, the machine of every client build.
pub const MACHINE_I386: u16 = 0x14c;
const COFF_SECTION_COUNT_OFFSET: usize = 2;
const COFF_OPTIONAL_HEADER_SIZE_OFFSET: usize = 16;
const COFF_HEADER_SIZE: usize = 20;
/// Optional header magic of a PE32 (32-bit) image; the clients are 32-bit.
const PE32_MAGIC: u16 = 0x10b;
/// Optional header magic of a PE32+ (64-bit) image: refused, with its own message (no OSI client is 64-bit).
const PE32_PLUS_MAGIC: u16 = 0x20b;
const OPTIONAL_IMAGE_BASE_OFFSET: usize = 28;
/// The checksum field, from the start of the optional header (same in PE32 and PE32+).
pub const OPTIONAL_CHECKSUM_OFFSET: usize = 64;
const OPTIONAL_DIRECTORY_COUNT_OFFSET: usize = 92;
const OPTIONAL_DIRECTORIES_OFFSET: usize = 96;
const DATA_DIRECTORY_SIZE: usize = 8;
/// Index of the security (Authenticode certificate table) data directory; its address is a file offset.
const SECURITY_DIRECTORY: usize = 4;
const SECTION_HEADER_SIZE: usize = 40;
const SECTION_NAME_SIZE: usize = 8;
const SECTION_VIRTUAL_SIZE_OFFSET: usize = 8;
const SECTION_VIRTUAL_ADDRESS_OFFSET: usize = 12;
const SECTION_RAW_SIZE_OFFSET: usize = 16;
const SECTION_RAW_OFFSET_OFFSET: usize = 20;
const SECTION_CHARACTERISTICS_OFFSET: usize = 36;
/// Section characteristics (`IMAGE_SCN_*`).
pub const SECTION_CODE: u32 = 0x0000_0020;
pub const SECTION_INITIALIZED_DATA: u32 = 0x0000_0040;
pub const SECTION_EXECUTE: u32 = 0x2000_0000;
pub const SECTION_READ: u32 = 0x4000_0000;
pub const SECTION_WRITE: u32 = 0x8000_0000;
/// Mask of the low 16 bits, the checksum fold.
const LOW_16_BITS: u64 = 0xffff;
const LOW_32_BITS: u64 = 0xffff_ffff;

/// A section header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeSection {
    pub name: String,
    /// Virtual address (image base + RVA).
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    /// `IMAGE_SCN_*` flags ([`SECTION_CODE`], [`SECTION_READ`], ...).
    pub characteristics: u32,
}

impl PeSection {
    /// The file bytes of the section.
    pub fn raw_range(&self) -> std::ops::Range<usize> {
        self.raw_offset as usize..self.raw_offset as usize + self.raw_size as usize
    }

    /// Code: contains code or is executable.
    pub fn is_code(&self) -> bool {
        self.characteristics & (SECTION_CODE | SECTION_EXECUTE) != 0
    }

    /// Initialized data that is not code (`.data`, `.rdata`, ...).
    pub fn is_data(&self) -> bool {
        self.characteristics & SECTION_INITIALIZED_DATA != 0 && !self.is_code()
    }

    pub fn is_readable(&self) -> bool {
        self.characteristics & SECTION_READ != 0
    }

    pub fn is_writable(&self) -> bool {
        self.characteristics & SECTION_WRITE != 0
    }
}

/// The headers of a PE32 image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeImage {
    /// COFF machine ([`MACHINE_I386`] for the clients).
    pub machine: u16,
    pub image_base: u32,
    /// File offset of the checksum field.
    pub checksum_offset: usize,
    /// The checksum the file stores.
    pub stored_checksum: u32,
    /// File offset and size of the Authenticode certificate table; size 0 when the image is not signed.
    pub security_directory: (u32, u32),
    pub sections: Vec<PeSection>,
}

fn read_u16(bytes: &[u8], offset: usize) -> eyre::Result<u16> {
    let slice = bytes.get(offset..offset + size_of::<u16>()).ok_or_else(|| eyre!("the PE ends at {offset:#x}"))?;
    Ok(u16::from_le_bytes(slice.try_into().expect("u16 slice")))
}

pub(crate) fn read_u32(bytes: &[u8], offset: usize) -> eyre::Result<u32> {
    let slice = bytes.get(offset..offset + size_of::<u32>()).ok_or_else(|| eyre!("the PE ends at {offset:#x}"))?;
    Ok(u32::from_le_bytes(slice.try_into().expect("u32 slice")))
}

impl PeImage {
    /// Parses the headers of a PE32 image.
    pub fn parse(bytes: &[u8]) -> eyre::Result<Self> {
        let pe = read_u32(bytes, E_LFANEW_OFFSET)? as usize;
        eyre::ensure!(bytes.get(pe..pe + PE_SIGNATURE.len()) == Some(PE_SIGNATURE), "not a PE image");
        let coff = pe + COFF_HEADER_OFFSET;
        let machine = read_u16(bytes, coff + COFF_MACHINE_OFFSET)?;
        let section_count = read_u16(bytes, coff + COFF_SECTION_COUNT_OFFSET)?;
        let optional_size = read_u16(bytes, coff + COFF_OPTIONAL_HEADER_SIZE_OFFSET)? as usize;
        let optional = coff + COFF_HEADER_SIZE;
        let magic = read_u16(bytes, optional)?;
        eyre::ensure!(
            magic != PE32_PLUS_MAGIC,
            "a 64-bit (PE32+) image: not supported, every OSI client (UOSA, UOKR, client.exe) is 32-bit"
        );
        eyre::ensure!(magic == PE32_MAGIC, "optional header magic {magic:#x}: not a 32-bit (PE32) image");
        let image_base = read_u32(bytes, optional + OPTIONAL_IMAGE_BASE_OFFSET)?;
        let checksum_offset = optional + OPTIONAL_CHECKSUM_OFFSET;
        let stored_checksum = read_u32(bytes, checksum_offset)?;
        let directory_count = read_u32(bytes, optional + OPTIONAL_DIRECTORY_COUNT_OFFSET)? as usize;
        let security_directory = if directory_count > SECURITY_DIRECTORY {
            let entry = optional + OPTIONAL_DIRECTORIES_OFFSET + DATA_DIRECTORY_SIZE * SECURITY_DIRECTORY;
            (read_u32(bytes, entry)?, read_u32(bytes, entry + size_of::<u32>())?)
        } else {
            (0, 0)
        };
        let mut sections = Vec::with_capacity(usize::from(section_count));
        for index in 0..usize::from(section_count) {
            let header = optional + optional_size + SECTION_HEADER_SIZE * index;
            let name = bytes
                .get(header..header + SECTION_NAME_SIZE)
                .ok_or_else(|| eyre!("the PE ends inside section header {index}"))?;
            let name = String::from_utf8_lossy(name).trim_end_matches('\0').to_owned();
            sections.push(PeSection {
                name,
                virtual_address: image_base.wrapping_add(read_u32(bytes, header + SECTION_VIRTUAL_ADDRESS_OFFSET)?),
                virtual_size: read_u32(bytes, header + SECTION_VIRTUAL_SIZE_OFFSET)?,
                raw_offset: read_u32(bytes, header + SECTION_RAW_OFFSET_OFFSET)?,
                raw_size: read_u32(bytes, header + SECTION_RAW_SIZE_OFFSET)?,
                characteristics: read_u32(bytes, header + SECTION_CHARACTERISTICS_OFFSET)?,
            });
        }
        Ok(Self { machine, image_base, checksum_offset, stored_checksum, security_directory, sections })
    }

    /// True when the image carries an Authenticode signature (a non-empty security directory).
    pub fn is_signed(&self) -> bool {
        self.security_directory.1 != 0
    }

    pub fn section(&self, name: &str) -> Option<&PeSection> {
        self.sections.iter().find(|section| section.name == name)
    }

    /// The section that stores virtual address `va` in the file.
    pub fn section_of_va(&self, va: u32) -> Option<&PeSection> {
        self.sections.iter().find(|section| va.checked_sub(section.virtual_address).is_some_and(|delta| delta < section.raw_size))
    }

    /// The file offset of virtual address `va`, when a section stores it in the file.
    pub fn va_to_offset(&self, va: u32) -> Option<usize> {
        self.sections.iter().find_map(|section| {
            let delta = va.checked_sub(section.virtual_address)?;
            (delta < section.raw_size).then(|| section.raw_offset as usize + delta as usize)
        })
    }

    /// The virtual address of file offset `offset`, when it lies in a section.
    pub fn offset_to_va(&self, offset: usize) -> Option<u32> {
        self.sections.iter().find_map(|section| {
            section
                .raw_range()
                .contains(&offset)
                .then(|| section.virtual_address + (offset - section.raw_offset as usize) as u32)
        })
    }
}

/// The PE checksum of `bytes` (`CheckSumMappedFile`): the 16-bit one's-complement-style sum of the little-endian
/// u32 words, the checksum field at `checksum_offset` skipped and a trailing partial word zero padded, plus the
/// file length.
pub fn pe_checksum(bytes: &[u8], checksum_offset: usize) -> u32 {
    let mut sum: u64 = 0;
    for (index, word) in bytes.chunks(size_of::<u32>()).enumerate() {
        if index * size_of::<u32>() == checksum_offset {
            continue;
        }
        let mut padded = [0u8; size_of::<u32>()];
        padded[..word.len()].copy_from_slice(word);
        sum += u64::from(u32::from_le_bytes(padded));
        sum = (sum & LOW_32_BITS) + (sum >> u32::BITS);
    }
    sum = (sum & LOW_16_BITS) + (sum >> u16::BITS);
    sum += sum >> u16::BITS;
    sum &= LOW_16_BITS;
    (sum + bytes.len() as u64) as u32
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Image base of the synthetic images.
    pub const TEST_IMAGE_BASE: u32 = 0x0040_0000;
    /// `e_lfanew` of the synthetic images.
    const TEST_PE_OFFSET: usize = 0x80;
    const TEST_OPTIONAL_SIZE: usize = 0xe0;
    const TEST_DIRECTORY_COUNT: u32 = 16;
    /// Raw offset and RVA alignment of the synthetic sections.
    pub const TEST_SECTION_ALIGNMENT: u32 = 0x1000;

    /// The characteristics of a synthetic section: `.text` is code, `.data` writable data, any other read-only data.
    fn synthetic_characteristics(name: &str) -> u32 {
        match name {
            ".text" => SECTION_CODE | SECTION_EXECUTE | SECTION_READ,
            ".data" => SECTION_INITIALIZED_DATA | SECTION_READ | SECTION_WRITE,
            _ => SECTION_INITIALIZED_DATA | SECTION_READ,
        }
    }

    /// A PE32 image with the given sections `(name, contents)`, each at its own aligned RVA equal to its raw
    /// offset; `signed` sets a security directory.
    pub fn synthetic_pe(sections: &[(&str, Vec<u8>)], signed: bool) -> Vec<u8> {
        let mut bytes = vec![0u8; TEST_SECTION_ALIGNMENT as usize];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[E_LFANEW_OFFSET..E_LFANEW_OFFSET + 4].copy_from_slice(&(TEST_PE_OFFSET as u32).to_le_bytes());
        bytes[TEST_PE_OFFSET..TEST_PE_OFFSET + 4].copy_from_slice(PE_SIGNATURE);
        let coff = TEST_PE_OFFSET + COFF_HEADER_OFFSET;
        bytes[coff + COFF_MACHINE_OFFSET..][..2].copy_from_slice(&MACHINE_I386.to_le_bytes());
        bytes[coff + COFF_SECTION_COUNT_OFFSET..][..2].copy_from_slice(&(sections.len() as u16).to_le_bytes());
        bytes[coff + COFF_OPTIONAL_HEADER_SIZE_OFFSET..][..2].copy_from_slice(&(TEST_OPTIONAL_SIZE as u16).to_le_bytes());
        let optional = coff + COFF_HEADER_SIZE;
        bytes[optional..optional + 2].copy_from_slice(&PE32_MAGIC.to_le_bytes());
        bytes[optional + OPTIONAL_IMAGE_BASE_OFFSET..][..4].copy_from_slice(&TEST_IMAGE_BASE.to_le_bytes());
        bytes[optional + OPTIONAL_DIRECTORY_COUNT_OFFSET..][..4].copy_from_slice(&TEST_DIRECTORY_COUNT.to_le_bytes());
        if signed {
            let entry = optional + OPTIONAL_DIRECTORIES_OFFSET + DATA_DIRECTORY_SIZE * SECURITY_DIRECTORY;
            bytes[entry + 4..entry + 8].copy_from_slice(&1u32.to_le_bytes());
        }
        let mut raw_offset = TEST_SECTION_ALIGNMENT;
        for (index, (name, contents)) in sections.iter().enumerate() {
            let header = optional + TEST_OPTIONAL_SIZE + SECTION_HEADER_SIZE * index;
            bytes[header..header + name.len()].copy_from_slice(name.as_bytes());
            let size = contents.len() as u32;
            for (field, value) in [
                (SECTION_VIRTUAL_SIZE_OFFSET, size),
                (SECTION_VIRTUAL_ADDRESS_OFFSET, raw_offset),
                (SECTION_RAW_SIZE_OFFSET, size),
                (SECTION_RAW_OFFSET_OFFSET, raw_offset),
                (SECTION_CHARACTERISTICS_OFFSET, synthetic_characteristics(name)),
            ] {
                bytes[header + field..header + field + 4].copy_from_slice(&value.to_le_bytes());
            }
            bytes.resize(raw_offset as usize, 0);
            bytes.extend_from_slice(contents);
            raw_offset = (bytes.len() as u32).next_multiple_of(TEST_SECTION_ALIGNMENT);
        }
        bytes
    }

    #[test]
    fn headers_and_addresses_parse() {
        const TEXT: &str = ".text";
        let image = synthetic_pe(&[(TEXT, vec![1; 16]), (".data", vec![2; 8])], false);
        let pe = PeImage::parse(&image).unwrap();
        assert_eq!(pe.checksum_offset, TEST_PE_OFFSET + COFF_HEADER_OFFSET + COFF_HEADER_SIZE + OPTIONAL_CHECKSUM_OFFSET);
        assert!(!pe.is_signed());
        let data = pe.section(".data").unwrap();
        assert_eq!(pe.va_to_offset(data.virtual_address + 4), Some(data.raw_offset as usize + 4));
        assert_eq!(pe.offset_to_va(data.raw_offset as usize + 4), Some(data.virtual_address + 4));
        assert_eq!(pe.va_to_offset(data.virtual_address + data.raw_size), None);
        assert!(PeImage::parse(&synthetic_pe(&[(TEXT, vec![0; 4])], true)).unwrap().is_signed());
        assert!(pe.section(TEXT).unwrap().is_code() && !pe.section(TEXT).unwrap().is_data());
        assert!(data.is_data() && data.is_readable());

        let mut wide = image.clone();
        let magic = TEST_PE_OFFSET + COFF_HEADER_OFFSET + COFF_HEADER_SIZE;
        wide[magic..magic + 2].copy_from_slice(&PE32_PLUS_MAGIC.to_le_bytes());
        let error = PeImage::parse(&wide).unwrap_err().to_string();
        assert!(error.contains("64-bit"), "{error}");
    }

    #[test]
    fn checksum_skips_its_field_and_adds_the_length() {
        // Words 1 and 0xffff_ffff fold to 1; the checksum field (word 1) is skipped; a 2-byte tail pads to a word.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0xdead_beefu32.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&[2, 0]);
        // 1 + 0xffffffff = 0x1_0000_0000 -> folds to 1; + 2 = 3; 16-bit fold keeps 3; + length 14.
        assert_eq!(pe_checksum(&bytes, size_of::<u32>()), 3 + bytes.len() as u32);
    }
}
