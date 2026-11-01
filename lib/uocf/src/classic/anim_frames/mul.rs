//! `anim*.mul` entries (one body, action and direction) and the `anim*.idx` / `anim*.mul` pair writer.
//!
//! An entry (`UOClassicAnim::GetMulFrame`): the 256-word palette, a `u32` frame count, one `u32` offset per frame
//! (relative to the end of the palette; 0 is no frame, which the client skips), then the frames
//! ([`ClassicFrame`]). Every shipped entry (UO Gold to the 7.0.103 Classic Client) has its frames in offset order,
//! back to back from the end of the offset table, each ending with its marker, and nothing after the last.
//!
//! An index entry is 12 bytes: `u32` lookup, length, extra (0 in the shipped animation indexes); a lookup of
//! `0xFFFFFFFF` (negative for the client) or a length of 0 is an empty slot. The shipped `anim*.mul` files are not
//! in index order and have gaps (entries replaced by patches), so [`write_mul_entries`] copies the data file and
//! appends each changed entry at its end: everything else keeps its bytes, and an unchanged write is a copy.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

crate::eyre_imports!();

use super::{read_palette, write_palette, ClassicFrame, ClassicPalette, PALETTE_BYTES, RLE_END_MARKER};
use crate::enhanced::animationframe::package::EntryChange;
use crate::utils::output_file::{refuse_same_file, write_through_temp};

/// Bytes of an index entry.
pub const INDEX_ENTRY_BYTES: usize = 3 * U32_BYTES;
/// The lookup of an empty slot.
pub const EMPTY_LOOKUP: u32 = u32::MAX;
/// The frame offset of a missing frame.
pub const MISSING_FRAME_OFFSET: u32 = 0;
/// Most frames an entry may hold (as the uocf decoder, `classic::anim`).
pub const MAX_FRAMES_PER_ENTRY: usize = 1000;
/// The highest end of an entry in the data file: the client reads the lookup as a signed `i32`.
pub const MAX_DATA_END: u64 = i32::MAX as u64;

const U32_BYTES: usize = size_of::<u32>();
const FRAME_COUNT_BYTES: usize = U32_BYTES;

/// One entry: the palette and its frames (`None` for an offset of 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicMulEntry {
    pub palette: ClassicPalette,
    pub frames: Vec<Option<ClassicFrame>>,
}

impl ClassicMulEntry {
    /// Reads an entry (each frame at its offset, as the client).
    pub fn parse(data: &[u8]) -> eyre::Result<Self> {
        let palette = read_palette(data)?;
        let read_u32 = |offset: usize| {
            data.get(offset..offset + U32_BYTES)
                .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
                .ok_or_else(|| eyre!("the entry ends at byte {} before byte {}", data.len(), offset + U32_BYTES))
        };
        let count = read_u32(PALETTE_BYTES)? as usize;
        eyre::ensure!(count <= MAX_FRAMES_PER_ENTRY, "{count} frames: at most {MAX_FRAMES_PER_ENTRY}");
        let mut frames = Vec::with_capacity(count);
        for index in 0..count {
            let offset = read_u32(PALETTE_BYTES + FRAME_COUNT_BYTES + index * U32_BYTES)?;
            if offset == MISSING_FRAME_OFFSET {
                frames.push(None);
                continue;
            }
            let start = PALETTE_BYTES + offset as usize;
            let bytes = data.get(start..).filter(|bytes| !bytes.is_empty()).ok_or_else(|| eyre!("frame {index} offset {offset} is past the entry"))?;
            let (frame, _) = ClassicFrame::decode(bytes).wrap_err_with(|| format!("frame {index}"))?;
            frames.push(Some(frame));
        }
        Ok(Self { palette, frames })
    }

    /// The entry as the shipped encoder writes it. An empty frame (width or height 0, none shipped) is its header
    /// and the end marker.
    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        eyre::ensure!(
            self.frames.len() <= MAX_FRAMES_PER_ENTRY,
            "{} frames: at most {MAX_FRAMES_PER_ENTRY}",
            self.frames.len()
        );
        let mut frames = Vec::new();
        let mut offsets = Vec::with_capacity(self.frames.len());
        let table_bytes = FRAME_COUNT_BYTES + self.frames.len() * U32_BYTES;
        for (index, frame) in self.frames.iter().enumerate() {
            let Some(frame) = frame else {
                offsets.push(MISSING_FRAME_OFFSET);
                continue;
            };
            offsets.push(u32::try_from(table_bytes + frames.len()).map_err(|_| eyre!("frame {index} offset overflows"))?);
            frame.encode(&mut frames).wrap_err_with(|| format!("frame {index}"))?;
            if frame.is_empty() {
                frames.extend_from_slice(&RLE_END_MARKER.to_le_bytes());
            }
        }
        let mut out = Vec::with_capacity(PALETTE_BYTES + table_bytes + frames.len());
        write_palette(&self.palette, &mut out);
        out.extend_from_slice(&(self.frames.len() as u32).to_le_bytes());
        for offset in offsets {
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.extend_from_slice(&frames);
        Ok(out)
    }
}

/// One index entry as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MulIndexEntry {
    pub lookup: u32,
    pub length: u32,
    pub extra: u32,
}

impl MulIndexEntry {
    /// No data: an [`EMPTY_LOOKUP`] lookup (negative for the client) or a length of 0.
    pub fn is_empty(&self) -> bool {
        self.lookup == EMPTY_LOOKUP || self.length == 0
    }

    fn read(idx: &[u8], index: u32) -> eyre::Result<Self> {
        let start = index as usize * INDEX_ENTRY_BYTES;
        let bytes = idx
            .get(start..start + INDEX_ENTRY_BYTES)
            .ok_or_else(|| eyre!("index {index} is past the {} entries of the index", idx.len() / INDEX_ENTRY_BYTES))?;
        let field = |n: usize| u32::from_le_bytes(bytes[n * U32_BYTES..(n + 1) * U32_BYTES].try_into().unwrap());
        Ok(Self { lookup: field(0), length: field(1), extra: field(2) })
    }

    fn write(&self, idx: &mut [u8], index: u32) {
        let start = index as usize * INDEX_ENTRY_BYTES;
        for (n, value) in [self.lookup, self.length, self.extra].into_iter().enumerate() {
            idx[start + n * U32_BYTES..start + (n + 1) * U32_BYTES].copy_from_slice(&value.to_le_bytes());
        }
    }
}

fn read_index(path: &Path) -> eyre::Result<Vec<u8>> {
    let idx = std::fs::read(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    eyre::ensure!(
        idx.len() % INDEX_ENTRY_BYTES == 0,
        "{} is {} bytes, not a whole number of {INDEX_ENTRY_BYTES}-byte entries",
        path.display(),
        idx.len()
    );
    Ok(idx)
}

fn read_range(file: &mut std::fs::File, entry: MulIndexEntry, path: &Path) -> eyre::Result<Vec<u8>> {
    let mut bytes = vec![0u8; entry.length as usize];
    file.seek(SeekFrom::Start(u64::from(entry.lookup)))
        .and_then(|_| file.read_exact(&mut bytes))
        .wrap_err_with(|| format!("failed to read {} bytes at {} of {}", entry.length, entry.lookup, path.display()))?;
    Ok(bytes)
}

/// The bytes of entry `index` of the pair, `None` for an empty slot.
pub fn read_mul_entry(idx_path: &Path, mul_path: &Path, index: u32) -> eyre::Result<Option<Vec<u8>>> {
    let entry = MulIndexEntry::read(&read_index(idx_path)?, index)?;
    if entry.is_empty() {
        return Ok(None);
    }
    let mut mul = std::fs::File::open(mul_path).wrap_err_with(|| format!("failed to open {}", mul_path.display()))?;
    read_range(&mut mul, entry, mul_path).map(Some)
}

/// Writes copies of the `idx_source` / `mul_source` pair to `idx_output` / `mul_output` with each `(index,
/// payload)` (checked with [`ClassicMulEntry::parse`]) as that entry: an entry that already holds the payload is
/// left alone; any other is appended to the data file and its index entry points there (the extra field kept).
/// The other entries and every other byte stay as they are. The outputs must be two new paths, neither source;
/// each is written through a temporary file. Returns what happened to each entry.
pub fn write_mul_entries(
    idx_source: &Path,
    mul_source: &Path,
    idx_output: &Path,
    mul_output: &Path,
    entries: &[(u32, Vec<u8>)],
) -> eyre::Result<Vec<EntryChange>> {
    let sources = [idx_source, mul_source];
    refuse_same_file(sources, idx_output, "source animation file")?;
    refuse_same_file(sources, mul_output, "source animation file")?;
    // A missing output is no file yet, so the two outputs only compare by path then.
    if idx_output.exists() {
        refuse_same_file([idx_output], mul_output, "index output")?;
    }
    eyre::ensure!(idx_output != mul_output, "the index and data outputs are one path: {}", idx_output.display());
    for (index, payload) in entries {
        ClassicMulEntry::parse(payload).wrap_err_with(|| format!("entry {index} is not an animation entry"))?;
    }
    if let Some(index) = entries.iter().enumerate().find_map(|(n, (index, _))| entries[..n].iter().any(|(other, _)| other == index).then_some(index)) {
        eyre::bail!("entry {index} is given twice");
    }

    let mut idx = read_index(idx_source)?;
    let mut mul = std::fs::File::open(mul_source).wrap_err_with(|| format!("failed to open {}", mul_source.display()))?;
    let mut end = mul.metadata().wrap_err_with(|| format!("failed to read {}", mul_source.display()))?.len();
    let mut changes = Vec::with_capacity(entries.len());
    let mut appended: Vec<&[u8]> = Vec::new();
    for (index, payload) in entries {
        let entry = MulIndexEntry::read(&idx, *index)?;
        let change = if entry.is_empty() {
            EntryChange::Added
        } else if read_range(&mut mul, entry, mul_source)? == *payload {
            changes.push(EntryChange::Unchanged);
            continue;
        } else {
            EntryChange::Replaced
        };
        let length = u32::try_from(payload.len()).map_err(|_| eyre!("entry {index} is {} bytes", payload.len()))?;
        let new_end = end + u64::from(length);
        eyre::ensure!(new_end <= MAX_DATA_END, "the data file would pass {MAX_DATA_END} bytes");
        let extra = if entry.lookup == EMPTY_LOOKUP { 0 } else { entry.extra };
        MulIndexEntry { lookup: end as u32, length, extra }.write(&mut idx, *index);
        appended.push(payload);
        end = new_end;
        changes.push(change);
    }
    drop(mul);

    write_through_temp(mul_output, |temp| {
        std::fs::copy(mul_source, temp).wrap_err_with(|| format!("failed to copy {} to {}", mul_source.display(), temp.display()))?;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(temp)
            .wrap_err_with(|| format!("failed to open {}", temp.display()))?;
        for payload in &appended {
            file.write_all(payload).wrap_err_with(|| format!("failed to write {}", temp.display()))?;
        }
        file.sync_all().wrap_err_with(|| format!("failed to write {}", temp.display()))
    })?;
    write_through_temp(idx_output, |temp| std::fs::write(temp, &idx).wrap_err_with(|| format!("failed to write {}", temp.display())))?;
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> ClassicMulEntry {
        let mut palette = [0u16; super::super::PALETTE_COLOURS];
        palette[1] = 0x7C00;
        let frame = ClassicFrame { center_x: 0, center_y: -1, width: 1, height: 1, pixels: vec![Some(1)] };
        ClassicMulEntry { palette, frames: vec![Some(frame), None, Some(ClassicFrame::empty(0, 0))] }
    }

    #[test]
    fn entries_round_trip() {
        let entry = entry();
        let bytes = entry.to_bytes().unwrap();
        assert_eq!(ClassicMulEntry::parse(&bytes).unwrap(), entry);
        let offset = |n: usize| u32::from_le_bytes(bytes[PALETTE_BYTES + 4 + 4 * n..][..4].try_into().unwrap());
        assert_eq!((offset(0), offset(1)), (4 + 3 * 4, MISSING_FRAME_OFFSET));
    }

    #[test]
    fn the_pair_writer_appends_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("uocf-anim-mul-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = entry().to_bytes().unwrap();
        let mut idx = Vec::new();
        for value in [0u32, first.len() as u32, 0, EMPTY_LOOKUP, 0, 0] {
            idx.extend_from_slice(&value.to_le_bytes());
        }
        let (idx_path, mul_path) = (dir.join("anim.idx"), dir.join("anim.mul"));
        std::fs::write(&idx_path, &idx).unwrap();
        std::fs::write(&mul_path, &first).unwrap();
        let (idx_out, mul_out) = (dir.join("out.idx"), dir.join("out.mul"));

        let changes = write_mul_entries(&idx_path, &mul_path, &idx_out, &mul_out, &[(0, first.clone())]).unwrap();
        assert_eq!(changes, [EntryChange::Unchanged]);
        assert_eq!(std::fs::read(&idx_out).unwrap(), idx);
        assert_eq!(std::fs::read(&mul_out).unwrap(), first);

        let mut second = entry();
        second.frames.pop();
        let second = second.to_bytes().unwrap();
        std::fs::remove_file(&idx_out).unwrap();
        let changes = write_mul_entries(&idx_path, &mul_path, &idx_out, &mul_out, &[(1, second.clone())]).unwrap();
        assert_eq!(changes, [EntryChange::Added]);
        assert_eq!(read_mul_entry(&idx_out, &mul_out, 1).unwrap(), Some(second));
        assert_eq!(read_mul_entry(&idx_out, &mul_out, 0).unwrap(), Some(first));
        assert!(write_mul_entries(&idx_path, &mul_path, &idx_path, &mul_out, &[]).is_err(), "never overwrites");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
