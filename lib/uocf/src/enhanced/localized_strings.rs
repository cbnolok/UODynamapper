use std::path::Path;

use color_eyre::eyre::{self, WrapErr};

use crate::classic::cliloc::Cliloc;
use crate::uop_container::package::UopPackage;

pub const LOCALIZED_STRINGS_UOP_NAME: &str = "localizedstrings.uop";

#[derive(Debug, Clone)]
pub struct LocalizedStringsFile {
    pub filename_hash: u64,
    pub byte_len: u32,
    pub strings: Cliloc,
}

#[derive(Debug, Clone)]
pub struct LocalizedStringsPackage {
    pub files: Vec<LocalizedStringsFile>,
}

impl LocalizedStringsPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UopPackage::load(path)?;
        Self::from_package(&package)
    }

    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
        let mut files = Vec::new();
        for file in package.iter_files() {
            if !file.has_size() {
                continue;
            }
            let bytes = file.unpack().wrap_err_with(|| {
                format!("failed to unpack localized strings file {:016X}", file.filename_hash())
            })?;
            let strings = Cliloc::from_file_bytes(&bytes).wrap_err_with(|| {
                format!("failed to parse localized strings file {:016X}", file.filename_hash())
            })?;
            files.push(LocalizedStringsFile {
                filename_hash: file.filename_hash(),
                byte_len: file.decompressed_size(),
                strings,
            });
        }
        files.sort_by_key(|file| file.filename_hash);
        Ok(Self { files })
    }

    pub fn len(&self) -> usize {
        self.files.iter().map(|file| file.strings.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};

    #[test]
    fn parses_localized_payload_with_cliloc_layout() {
        let mut bytes = Vec::new();
        bytes.write_i32::<LittleEndian>(0).unwrap();
        bytes.write_i16::<LittleEndian>(0).unwrap();
        bytes.write_i32::<LittleEndian>(200).unwrap();
        bytes.write_u8(1).unwrap();
        bytes.write_i16::<LittleEndian>(4).unwrap();
        bytes.extend_from_slice(b"text");

        let strings = Cliloc::from_file_bytes(&bytes).unwrap();

        assert_eq!(strings.get(200), Some("text"));
    }
}
