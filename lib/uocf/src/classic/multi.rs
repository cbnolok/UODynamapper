//! Decoding for Classic UO multi files (`multi.mul` and `multi.idx`).

crate::eyre_imports!();
use crate::classic::generic_index::IndexFile;
use crate::classic::verdata::{VerFileId, Verdata};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const LEGACY_MULTI_PART_RECORD_SIZE: usize = 12;
const EXTENDED_MULTI_PART_RECORD_SIZE: usize = 16;
const MAX_CLASSIC_STATIC_ITEM_ID: u16 = 0x3FFF;

#[derive(Debug, Clone, Copy)]
pub struct MultiPart {
    pub item_id: u16,
    pub x: i16,
    pub y: i16,
    pub z: i16,
    pub flags: u32,
}

pub struct MultiMap {
    index_file: IndexFile,
    mul_path: std::path::PathBuf,
    verdata: Option<Arc<Verdata>>,
}

impl MultiMap {
    pub fn load(client_path: &Path) -> eyre::Result<Self> {
        let idx_path = find_multi_index_path(client_path);
        let mul_path = client_path.join("multi.mul");

        let Some(idx_path) = idx_path else {
            eyre::bail!(
                "multi.idx/multiidx.mul or multi.mul not found in {}",
                client_path.display()
            );
        };
        if !mul_path.exists() {
            eyre::bail!("multi.mul not found in {}", client_path.display());
        }

        let index = IndexFile::load(idx_path)?;
        Ok(Self {
            index_file: index,
            mul_path,
            verdata: None,
        })
    }

    pub fn with_verdata(mut self, verdata: Arc<Verdata>) -> Self {
        self.verdata = Some(verdata);
        self
    }

    pub fn get_parts(&self, multi_id: u32) -> eyre::Result<Vec<MultiPart>> {
        if let Some(verdata) = &self.verdata {
            if let Some(bytes) = verdata.read_patch(VerFileId::Multi, multi_id as i32)? {
                return parse_multi_parts(&bytes);
            }
        }

        let file = File::open(&self.mul_path)?;
        let mut reader = BufReader::new(file);
        self.read_parts_from_reader(multi_id, &mut reader)
    }

    pub fn load_all_parts(&self) -> eyre::Result<Vec<Vec<MultiPart>>> {
        let file = File::open(&self.mul_path)?;
        let mut reader = BufReader::new(file);
        let mut definitions = Vec::with_capacity(self.index_file.element_count());

        for multi_id in 0..self.index_file.element_count() {
            definitions.push(self.read_parts_from_reader(multi_id as u32, &mut reader)?);
        }

        Ok(definitions)
    }

    pub fn max_id(&self) -> u32 {
        self.index_file.element_count() as u32
    }

    fn read_parts_from_reader(
        &self,
        multi_id: u32,
        reader: &mut BufReader<File>,
    ) -> eyre::Result<Vec<MultiPart>> {
        let entry = self
            .index_file
            .element(multi_id as usize)
            .wrap_err_with(|| format!("Multi ID {} not found in index", multi_id))?;

        let index_patch = self
            .verdata
            .as_ref()
            .and_then(|verdata| verdata.index_patch(VerFileId::MultiIdx, multi_id as i32));
        let index_values = index_patch.or_else(|| {
            Some((entry.lookup()?, entry.len()?, entry.extra().unwrap_or(0)))
        });
        let Some((lookup, length, _extra)) = index_values else {
            return Ok(Vec::new());
        };

        reader.seek(SeekFrom::Start(lookup as u64))?;
        let mut bytes = vec![0u8; length as usize];
        reader.read_exact(&mut bytes)?;

        parse_multi_parts(&bytes)
    }
}

fn parse_multi_parts(bytes: &[u8]) -> eyre::Result<Vec<MultiPart>> {
    parse_multi_parts_with_stride(bytes, choose_multi_part_record_size(bytes))
}

fn parse_multi_parts_with_stride(bytes: &[u8], stride: usize) -> eyre::Result<Vec<MultiPart>> {
    let count = bytes.len() / stride;
    let mut parts = Vec::with_capacity(count);

    for i in 0..count {
        let base = i * stride;
        parts.push(MultiPart {
            item_id: u16::from_le_bytes([bytes[base], bytes[base + 1]]),
            x: i16::from_le_bytes([bytes[base + 2], bytes[base + 3]]),
            y: i16::from_le_bytes([bytes[base + 4], bytes[base + 5]]),
            z: i16::from_le_bytes([bytes[base + 6], bytes[base + 7]]),
            flags: u32::from_le_bytes([
                bytes[base + 8],
                bytes[base + 9],
                bytes[base + 10],
                bytes[base + 11],
            ]),
        });
    }

    Ok(parts)
}

fn choose_multi_part_record_size(bytes: &[u8]) -> usize {
    let supports_legacy = bytes.len() >= LEGACY_MULTI_PART_RECORD_SIZE
        && bytes.len() % LEGACY_MULTI_PART_RECORD_SIZE == 0;
    let supports_extended = bytes.len() >= EXTENDED_MULTI_PART_RECORD_SIZE
        && bytes.len() % EXTENDED_MULTI_PART_RECORD_SIZE == 0;

    if supports_extended && !supports_legacy {
        return EXTENDED_MULTI_PART_RECORD_SIZE;
    }

    if supports_extended && supports_legacy {
        let legacy_score = multi_part_record_plausibility_score(bytes, LEGACY_MULTI_PART_RECORD_SIZE);
        let extended_score = multi_part_record_plausibility_score(bytes, EXTENDED_MULTI_PART_RECORD_SIZE);
        if extended_score < legacy_score {
            return EXTENDED_MULTI_PART_RECORD_SIZE;
        }
    }

    LEGACY_MULTI_PART_RECORD_SIZE
}

fn multi_part_record_plausibility_score(bytes: &[u8], stride: usize) -> u32 {
    let mut score = 0u32;
    for i in 0..(bytes.len() / stride) {
        let base = i * stride;
        let item_id = u16::from_le_bytes([bytes[base], bytes[base + 1]]);
        let x = i16::from_le_bytes([bytes[base + 2], bytes[base + 3]]) as i32;
        let y = i16::from_le_bytes([bytes[base + 4], bytes[base + 5]]) as i32;
        let z = i16::from_le_bytes([bytes[base + 6], bytes[base + 7]]) as i32;

        if item_id > MAX_CLASSIC_STATIC_ITEM_ID {
            score += 4;
        }
        if x.abs() > 512 {
            score += 2;
        }
        if y.abs() > 512 {
            score += 2;
        }
        if z.abs() > 128 {
            score += 2;
        }
    }

    score
}

fn find_multi_index_path(client_path: &Path) -> Option<PathBuf> {
    ["multi.idx", "multiidx.mul"]
        .iter()
        .map(|file_name| client_path.join(file_name))
        .find(|path| path.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_record(bytes: &mut Vec<u8>, item_id: u16, x: i16, y: i16, z: i16, flags: u32) {
        bytes.extend_from_slice(&item_id.to_le_bytes());
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes.extend_from_slice(&z.to_le_bytes());
        bytes.extend_from_slice(&flags.to_le_bytes());
    }

    fn push_extended_record(
        bytes: &mut Vec<u8>,
        item_id: u16,
        x: i16,
        y: i16,
        z: i16,
        flags: u32,
    ) {
        push_record(bytes, item_id, x, y, z, flags);
        bytes.extend_from_slice(&[0xFE, 0xFF, 0x7F, 0x7F]);
    }

    #[test]
    fn parses_extended_multi_records_with_sixteen_byte_stride() {
        let mut bytes = Vec::new();
        push_extended_record(&mut bytes, 0x0123, -1, 2, 3, 1);
        push_extended_record(&mut bytes, 0x0456, 4, -5, 6, 0x100);

        let parts = parse_multi_parts(&bytes).expect("parse extended multi parts");

        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].item_id, 0x0123);
        assert_eq!((parts[0].x, parts[0].y, parts[0].z, parts[0].flags), (-1, 2, 3, 1));
        assert_eq!(parts[1].item_id, 0x0456);
        assert_eq!((parts[1].x, parts[1].y, parts[1].z, parts[1].flags), (4, -5, 6, 0x100));
    }

    #[test]
    fn prefers_extended_stride_for_ambiguous_but_implausible_legacy_payloads() {
        let mut bytes = Vec::new();
        push_extended_record(&mut bytes, 0x0100, 0, 0, 0, 1);
        push_extended_record(&mut bytes, 0x0101, 1, 0, 0, 1);
        push_extended_record(&mut bytes, 0x0102, 2, 0, 0, 1);

        assert_eq!(bytes.len(), 48);
        assert_eq!(choose_multi_part_record_size(&bytes), EXTENDED_MULTI_PART_RECORD_SIZE);
        let parts = parse_multi_parts(&bytes).expect("parse ambiguous extended multi parts");

        assert_eq!(parts.len(), 3);
        assert_eq!(parts[2].item_id, 0x0102);
        assert_eq!(parts[2].x, 2);
    }
}
