//! An index file and its data file (`*idx.mul` / `*.mul`) as entries, written back as the shipped files are laid
//! out, for the Classic files whose payloads need no other layout: `multi.mul`, `light.mul`, `texmaps.mul`,
//! `skills.mul`.
//!
//! Measured on UO Gold, Mondain's Legacy, 7.0.20, the 2024 Classic client and Classic fp: the present entries'
//! payloads follow each other from offset 0, with nothing between them or after the last one, in id order but in
//! `skills.mul` from 7.0.20 on (55 to 57 were appended as 57, 56, 55), so the order read is kept. An
//! absent entry's 12-byte record differs by file and era, so it is kept as read:
//!
//! | File | Absent entry record (lookup, size, extra) |
//! |---|---|
//! | `multi.idx` | `(-1, -1, -1)` |
//! | `lightidx.mul` | `(-1, 0, 0)` |
//! | `texidx.mul` (Gold) | `(-1, 0, 0)` |
//! | `texidx.mul` (Mondain's Legacy) | `(-1, 0, 0)`, then `(-1, -1, 0)` at 4095 and `(0, -1, 0)` from 4096 on |
//! | `texidx.mul` (7.0.20, 2024, fp) | `(0, 0, 0)` |
//!
//! Mondain's Legacy's `texidx.mul` also ends with 4 bytes that are no whole record; they are kept too.
//! [`IndexedMul::to_bytes`] lays the payloads out again in the order read, then the added ones in id order, so an
//! unchanged file comes back byte for byte and an edited entry moves only the payloads after it.

crate::eyre_imports!();
use std::path::Path;

use crate::utils::output_file::write_through_temp;

/// Bytes of one index record: lookup, size and extra, each a little-endian `u32`.
pub const INDEX_RECORD_BYTES: usize = 12;
/// The lookup, size or extra value that marks an absent entry.
pub const ABSENT: u32 = u32::MAX;

/// One index slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexedEntry {
    /// A payload, with the index record's third field (dimensions, a size flag, or unused).
    Present { extra: u32, payload: Vec<u8> },
    /// No payload: the record as the file holds it.
    Absent { lookup: u32, size: u32, extra: u32 },
}

impl IndexedEntry {
    pub fn payload(&self) -> Option<&[u8]> {
        match self {
            Self::Present { payload, .. } => Some(payload),
            Self::Absent { .. } => None,
        }
    }

    pub fn extra(&self) -> Option<u32> {
        match self {
            Self::Present { extra, .. } => Some(*extra),
            Self::Absent { .. } => None,
        }
    }
}

/// The entries of an index and data file pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedMul {
    pub entries: Vec<IndexedEntry>,
    /// Index bytes after the last whole record.
    pub index_tail: Vec<u8>,
    /// Data bytes after the last payload (none in the shipped files).
    pub data_tail: Vec<u8>,
    /// The ids of the payloads read, in data order; [`Self::to_bytes`] writes the present ones in this order, then
    /// the others in id order.
    pub data_order: Vec<usize>,
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four bytes"))
}

impl IndexedMul {
    /// Reads the pair. A record is absent when its lookup, size or extra is `-1` or its size is 0; a present
    /// record must lie inside the data.
    pub fn from_bytes(index: &[u8], data: &[u8]) -> eyre::Result<Self> {
        let count = index.len() / INDEX_RECORD_BYTES;
        let mut entries = Vec::with_capacity(count);
        let mut data_end = 0usize;
        let mut lookups = Vec::new();
        for id in 0..count {
            let base = id * INDEX_RECORD_BYTES;
            let (lookup, size, extra) = (read_u32(index, base), read_u32(index, base + 4), read_u32(index, base + 8));
            if lookup == ABSENT || size == ABSENT || extra == ABSENT || size == 0 {
                entries.push(IndexedEntry::Absent { lookup, size, extra });
                continue;
            }
            let (start, len) = (lookup as usize, size as usize);
            let end = start.checked_add(len).filter(|&end| end <= data.len()).ok_or_else(|| {
                eyre!("index entry {id}: {len} bytes at {start} lie past the data's {} bytes", data.len())
            })?;
            data_end = data_end.max(end);
            lookups.push((start, id));
            entries.push(IndexedEntry::Present { extra, payload: data[start..end].to_vec() });
        }
        lookups.sort_unstable();
        let data_order = lookups.into_iter().map(|(_, id)| id).collect();
        Ok(Self { entries, index_tail: index[count * INDEX_RECORD_BYTES..].to_vec(), data_tail: data[data_end..].to_vec(), data_order })
    }

    pub fn load(index_path: &Path, data_path: &Path) -> eyre::Result<Self> {
        let index = std::fs::read(index_path).wrap_err_with(|| format!("read {}", index_path.display()))?;
        let data = std::fs::read(data_path).wrap_err_with(|| format!("read {}", data_path.display()))?;
        Self::from_bytes(&index, &data).wrap_err_with(|| format!("{} and {}", index_path.display(), data_path.display()))
    }

    /// The ids of the present entries in the order their payloads are written: [`Self::data_order`], then the
    /// others in id order.
    pub fn write_order(&self) -> Vec<usize> {
        let present = |id: &usize| self.entries.get(*id).is_some_and(|entry| entry.payload().is_some());
        let mut seen = vec![false; self.entries.len()];
        let mut order: Vec<usize> = self.data_order.iter().copied().filter(present).filter(|&id| !std::mem::replace(&mut seen[id], true)).collect();
        order.extend((0..self.entries.len()).filter(|id| present(id) && !seen[*id]));
        order
    }

    /// The index and the data: payloads from offset 0 in [`Self::write_order`], absent records as read.
    pub fn to_bytes(&self) -> eyre::Result<(Vec<u8>, Vec<u8>)> {
        let mut index = Vec::with_capacity(self.entries.len() * INDEX_RECORD_BYTES + self.index_tail.len());
        let mut data = Vec::new();
        let mut lookups = vec![0u32; self.entries.len()];
        for id in self.write_order() {
            let payload = self.entries[id].payload().expect("a present entry");
            lookups[id] = u32::try_from(data.len()).ok().filter(|&lookup| lookup != ABSENT).ok_or_else(|| {
                eyre!("entry {id}: {} bytes at {} do not fit an index record", payload.len(), data.len())
            })?;
            data.extend_from_slice(payload);
        }
        for (id, entry) in self.entries.iter().enumerate() {
            let (lookup, size, extra) = match entry {
                IndexedEntry::Present { extra, payload } => {
                    let size = u32::try_from(payload.len()).ok().filter(|&size| size != ABSENT && size != 0);
                    let Some(size) = size else {
                        eyre::bail!("entry {id}: {} bytes do not fit an index record", payload.len());
                    };
                    if *extra == ABSENT {
                        eyre::bail!("entry {id}: extra {ABSENT:#x} would mark it absent");
                    }
                    (lookups[id], size, *extra)
                }
                IndexedEntry::Absent { lookup, size, extra } => (*lookup, *size, *extra),
            };
            for field in [lookup, size, extra] {
                index.extend_from_slice(&field.to_le_bytes());
            }
        }
        index.extend_from_slice(&self.index_tail);
        data.extend_from_slice(&self.data_tail);
        Ok((index, data))
    }

    /// Writes both files, each through a temporary file in its folder.
    pub fn save(&self, index_path: &Path, data_path: &Path) -> eyre::Result<()> {
        let (index, data) = self.to_bytes()?;
        for (path, bytes) in [(data_path, &data), (index_path, &index)] {
            write_through_temp(path, |temp| std::fs::write(temp, bytes).wrap_err_with(|| format!("write {}", temp.display())))?;
        }
        Ok(())
    }

    /// The record an absent entry is written with when one is added or cleared: the file's most common one, or
    /// `(-1, 0, 0)` when it has none.
    pub fn absent_record(&self) -> IndexedEntry {
        let mut counts: Vec<((u32, u32, u32), usize)> = Vec::new();
        for entry in &self.entries {
            if let IndexedEntry::Absent { lookup, size, extra } = *entry {
                match counts.iter_mut().find(|(record, _)| *record == (lookup, size, extra)) {
                    Some((_, count)) => *count += 1,
                    None => counts.push(((lookup, size, extra), 1)),
                }
            }
        }
        let (lookup, size, extra) = counts.iter().max_by_key(|(_, count)| *count).map_or((ABSENT, 0, 0), |(record, _)| *record);
        IndexedEntry::Absent { lookup, size, extra }
    }

    /// Sets entry `id`, growing the index with absent entries when `id` is past its end.
    pub fn set(&mut self, id: usize, entry: IndexedEntry) {
        if id >= self.entries.len() {
            let absent = self.absent_record();
            self.entries.resize(id + 1, absent);
        }
        self.entries[id] = entry;
    }

    /// Clears entry `id` with the file's absent record.
    pub fn clear(&mut self, id: usize) {
        if id < self.entries.len() {
            self.entries[id] = self.absent_record();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(lookup: u32, size: u32, extra: u32) -> Vec<u8> {
        [lookup, size, extra].iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn keeps_absent_records_and_tails_and_lays_payloads_out_in_order() {
        let mut index = Vec::new();
        index.extend(record(0, 3, 7));
        index.extend(record(ABSENT, 0, 0));
        index.extend(record(0, ABSENT, 0));
        index.extend(record(3, 2, 9));
        index.extend([0, 0, 0, 0]);
        let data = [1, 2, 3, 4, 5];
        let file = IndexedMul::from_bytes(&index, &data).unwrap();
        assert_eq!(file.entries[0].payload(), Some(&[1, 2, 3][..]));
        assert_eq!(file.entries[2], IndexedEntry::Absent { lookup: 0, size: ABSENT, extra: 0 });
        assert_eq!(file.index_tail, vec![0; 4]);
        assert_eq!(file.to_bytes().unwrap(), (index.clone(), data.to_vec()));

        let mut edited = file.clone();
        edited.set(0, IndexedEntry::Present { extra: 7, payload: vec![8] });
        edited.set(5, IndexedEntry::Present { extra: 1, payload: vec![6, 6] });
        let (edited_index, edited_data) = edited.to_bytes().unwrap();
        assert_eq!(edited_data, vec![8, 4, 5, 6, 6]);
        let back = IndexedMul::from_bytes(&edited_index, &edited_data).unwrap();
        assert_eq!(back.entries[3].payload(), Some(&[4, 5][..]));
        // The added slot between the old end and the new entry takes the most common absent record.
        assert_eq!(back.entries.len(), 6);
        assert!(matches!(back.entries[4], IndexedEntry::Absent { .. }));
        edited.clear(3);
        assert!(edited.entries[3].payload().is_none());
    }

    #[test]
    fn keeps_the_data_order_read() {
        let mut index = Vec::new();
        index.extend(record(2, 1, 0));
        index.extend(record(0, 2, 0));
        let data = [5, 5, 4];
        let mut file = IndexedMul::from_bytes(&index, &data).unwrap();
        assert_eq!(file.data_order, vec![1, 0]);
        assert_eq!(file.to_bytes().unwrap(), (index, data.to_vec()));
        file.set(3, IndexedEntry::Present { extra: 0, payload: vec![9] });
        file.set(1, IndexedEntry::Present { extra: 0, payload: vec![6] });
        assert_eq!(file.write_order(), vec![1, 0, 3]);
        assert_eq!(file.to_bytes().unwrap().1, vec![6, 4, 9]);
    }

    #[test]
    fn rejects_a_payload_past_the_data() {
        assert!(IndexedMul::from_bytes(&record(2, 4, 0), &[0; 5]).is_err());
    }
}
