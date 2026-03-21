#![allow(dead_code)]

crate::eyre_imports!();
// use byteorder::{LittleEndian, ReadBytesExt};
use std::fs::File;
use std::io::prelude::*;
use std::path::PathBuf;

use super::utils::math::*;

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
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
        let mut file_data: Vec<IndexElement> = vec![IndexElement::default(); index_element_qty];
        
        file_handle
            .read_exact(bytemuck::cast_slice_mut(&mut file_data))
            .wrap_err("Read index file")?;

        // Handle endianness if strictly necessary, though UO data is always LE.
        // On LE systems, this is a no-op if optimized.
        #[cfg(target_endian = "big")]
        {
            for elem in file_data.iter_mut() {
                elem.lookup = elem.lookup.swap_bytes();
                elem.size = elem.size.swap_bytes();
                elem.extra = elem.extra.swap_bytes();
            }
        }

        let i_elem = file_data.len();
        let index_file = IndexFile { file_data };
        log::info!(
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
