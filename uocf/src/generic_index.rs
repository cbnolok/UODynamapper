#![allow(dead_code)]

crate::eyre_imports!();
use byteorder::{LittleEndian, ReadBytesExt};
use std::fs::File;
use std::io::{prelude::*, Cursor};
use std::path::PathBuf;

use super::utils::math::*;

#[derive(Clone, Debug, Default)]
pub struct IndexElement {
    lookup: u32, // Position of the element in the related file.
    size: u32,   // Size of the element in bytes.
    extra: u32,  // Extra data, used only by some files.
}
impl IndexElement {
    const INVALID_LOOKUP: u32 = 0xFFFFFFFF;
    const PACKED_SIZE: u32 = 4 + 4 + 4;

    pub fn lookup(&self) -> Option<u32> {
        if self.lookup == Self::INVALID_LOOKUP || self.extra == Self::INVALID_LOOKUP {
            return None;
        }
        Some(self.lookup)
    }

    pub fn len(&self) -> Option<u32> {
        if self.lookup == Self::INVALID_LOOKUP || self.extra == Self::INVALID_LOOKUP {
            return None;
        }
        Some(self.size)
    }

    pub fn extra(&self) -> Option<u32> {
        if self.lookup == Self::INVALID_LOOKUP || self.extra == Self::INVALID_LOOKUP {
            return None;
        }
        Some(self.extra)
    }
}

pub struct IndexFile {
    file_data: Vec<IndexElement>,
}

impl IndexFile {
    pub fn element_count(&self) -> usize {
        self.file_data.len()
    }

    pub fn element(&self, element_index: usize) -> eyre::Result<&IndexElement> {
        if element_index >= self.file_data.len() {
            return Err(eyre!(
                "IndexFile: requested element with out of range index ({element_index})."
                    .to_owned()
            ));
        }
        Ok(&self.file_data[element_index])
    }

    pub fn load(file_path: PathBuf) -> eyre::Result<IndexFile> {
        let file_name = file_path
            .file_name()
            .expect("Provided file path without filename.")
            .to_string_lossy();
        let file_path = file_path
            .canonicalize()
            .wrap_err_with(|| format!("Check {file_name} path"))?;

        let mut file_handle = File::open(&file_path)
            .wrap_err_with(|| format!("Open index mul file at '{file_name}'"))?;
        let file_metadata = file_handle
            .metadata()
            .wrap_err("Get {file_name} metadata")?;
        let file_size = downcast_ceil_usize(file_metadata.len());

        let index_element_qty = file_size / IndexElement::PACKED_SIZE as usize;
        let mut index_file = IndexFile {
            file_data: vec![IndexElement::default(); index_element_qty],
        };

        let mut index_file_rdr = {
            let mut rdr_buf = vec![0; file_size];
            file_handle
                .read_exact(rdr_buf.as_mut())
                .wrap_err("Read index file")?;
            Cursor::new(rdr_buf)
        };

        let strerr_base = "Reading index data for element ";
        let mut i_elem = 0;
        for elem in index_file.file_data.iter_mut() {
            elem.lookup = index_file_rdr
                .read_u32::<LittleEndian>()
                .wrap_err_with(|| format!("{}0x{:x}: Reading {}", strerr_base, i_elem, "lookup"))?;

            elem.size = index_file_rdr
                .read_u32::<LittleEndian>()
                .wrap_err_with(|| format!("{}0x{:x}: Reading {}", strerr_base, i_elem, "size"))?;

            elem.extra = index_file_rdr
                .read_u32::<LittleEndian>()
                .wrap_err_with(|| format!("{}0x{:x}: Reading {}", strerr_base, i_elem, "extra"))?;
            i_elem += 1;
        }
        println!(
            "Loaded {i_elem} (0x{:x}) Index Elements from '{file_name}'.",
            i_elem
        );

        /*  Some index file sizes are not multiple of 12, so there are cases of idx files with trailing, unused (?), small data.
        assert_eq!(
            index_file_rdr.get_ref().len() as u64,
            index_file_rdr.position()
        ); // Consumed the whole file
        */

        Ok(index_file)
    }
}
