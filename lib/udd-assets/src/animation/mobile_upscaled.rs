//! The 2x frames a mobile animation package may carry beside its own, made ahead of time by the converter
//! (`--prescale`): the SPAN 2x canvas Dynamapper's `mobile_upscale` makes at run time from each chosen frame, so the
//! renderer draws them without running the network. A package without the entry has none; its frames upscale at run
//! time as before.
//!
//! The 1x frames, their pages and records stay as they are, so the strides (which hash the 1x frames) are unaffected.
//! The 2x frames have pages of their own (`upscaled_pages/`), in the package's page format, and a table
//! (`metadata/upscaled_frames.bin`) giving, per 1x frame record that has one, where its 2x canvas lies. A 2x canvas
//! is the frame with a [`UPSCALED_FRAME_BORDER`] on each side, times [`UPSCALED_FRAME_SCALE`]: the halo past the
//! frame's edge included, its anchor `(anchor + border) * scale`.

use std::io::{Cursor, Read};

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};
use udd_container::UddpReader;

use crate::animation::mobile_pages::{ClipPages, PageLayout, PageSet, StoredPage, StoredPageCache};
use crate::animation::mobile_strides::{PageMemo, StoredFramePixels};
use crate::common::{ensure_manifest_version, extract_atlas_subrect_rgba, read_path_entry, read_path_entry_cow};
use crate::tex_art_cc::PagePixelFormat;

pub const UPSCALED_FRAMES_ENTRY_PATH: &str = "metadata/upscaled_frames.bin";
pub const UPSCALED_FRAMES_MAGIC: [u8; 4] = *b"MOUP";
pub const MOBILE_UPSCALED_FRAMES_VERSION: u32 = 1;
/// The folder of the 2x pages; every entry below it belongs to the 2x frames.
pub const UPSCALED_PAGES_DIR: &str = "upscaled_pages";
/// The scale of the 2x frames, and the transparent border each side of a frame that its 2x canvas holds, in 1x
/// pixels: Dynamapper's runtime upscale's (uot-gpu-compute `UPSCALE_OUTPUT_SCALE`, `UPSCALE_BORDER`).
pub const UPSCALED_FRAME_SCALE: u32 = 2;
pub const UPSCALED_FRAME_BORDER: u32 = 2;

const HEADER_BYTES: usize = 4 + 4 + 1 + 1 + 1 + 1 + 4 + 4 + 2 + 4 + 4;
const PAGE_RECORD_BYTES: usize = 8;
const FRAME_RECORD_BYTES: usize = 16;

/// The entry of 2x page `page_index`.
pub fn upscaled_page_entry_path(page_index: u32, fmt: PagePixelFormat) -> String {
    format!("{UPSCALED_PAGES_DIR}/{page_index:05}.{}", fmt.extension())
}

/// A 2x page: the part of the atlas its frames use, as the 1x pages record it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpscaledPageRecord {
    pub used_width: u32,
    pub used_height: u32,
}

/// The 2x canvas of one 1x frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpscaledFrameRecord {
    /// The 1x frame record's index among the package's frame records.
    pub frame: u32,
    pub page: u32,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// `metadata/upscaled_frames.bin`: the 2x pages and the 2x canvas of each 1x frame that has one, sorted by frame.
#[derive(Debug, Clone, PartialEq)]
pub struct UpscaledFrameTable {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    pub pixel_format: PagePixelFormat,
    pub pages: Vec<UpscaledPageRecord>,
    pub frames: Vec<UpscaledFrameRecord>,
}

impl UpscaledFrameTable {
    /// The 2x canvas of 1x frame record `frame`, if it has one.
    pub fn frame(&self, frame: u32) -> Option<&UpscaledFrameRecord> {
        self.frames.binary_search_by_key(&frame, |record| record.frame).ok().map(|index| &self.frames[index])
    }

    /// The size of the 2x canvas of a 1x frame of `width` by `height`.
    pub fn canvas_size(width: u16, height: u16) -> (u32, u32) {
        let side = |side: u16| (u32::from(side) + 2 * UPSCALED_FRAME_BORDER) * UPSCALED_FRAME_SCALE;
        (side(width), side(height))
    }

    /// The anchor of the 2x canvas of a 1x frame anchored at `anchor`, on one axis.
    pub fn canvas_anchor(anchor: i32) -> i32 {
        (anchor + UPSCALED_FRAME_BORDER as i32) * UPSCALED_FRAME_SCALE as i32
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut frames = self.frames.clone();
        frames.sort_by_key(|record| record.frame);
        let mut bytes = Vec::with_capacity(HEADER_BYTES + self.pages.len() * PAGE_RECORD_BYTES
            + frames.len() * FRAME_RECORD_BYTES);
        bytes.extend_from_slice(&UPSCALED_FRAMES_MAGIC);
        bytes.extend_from_slice(&MOBILE_UPSCALED_FRAMES_VERSION.to_le_bytes());
        bytes.push(UPSCALED_FRAME_SCALE as u8);
        bytes.push(UPSCALED_FRAME_BORDER as u8);
        bytes.push(self.pixel_format as u8);
        bytes.push(0);
        bytes.extend_from_slice(&self.atlas_width.to_le_bytes());
        bytes.extend_from_slice(&self.atlas_height.to_le_bytes());
        bytes.extend_from_slice(&self.gutter.to_le_bytes());
        bytes.extend_from_slice(&(self.pages.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(frames.len() as u32).to_le_bytes());
        for page in &self.pages {
            bytes.extend_from_slice(&page.used_width.to_le_bytes());
            bytes.extend_from_slice(&page.used_height.to_le_bytes());
        }
        for record in &frames {
            bytes.extend_from_slice(&record.frame.to_le_bytes());
            bytes.extend_from_slice(&record.page.to_le_bytes());
            for value in [record.x, record.y, record.width, record.height] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }

    pub fn parse(bytes: &[u8]) -> eyre::Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let mut magic = [0u8; 4];
        cursor.read_exact(&mut magic)?;
        eyre::ensure!(magic == UPSCALED_FRAMES_MAGIC, "invalid mobile upscaled frames magic");
        let version = cursor.read_u32::<LittleEndian>()?;
        ensure_manifest_version("mobile upscaled frames", version, MOBILE_UPSCALED_FRAMES_VERSION)?;
        let (scale, border) = (cursor.read_u8()?, cursor.read_u8()?);
        eyre::ensure!(u32::from(scale) == UPSCALED_FRAME_SCALE && u32::from(border) == UPSCALED_FRAME_BORDER,
            "mobile upscaled frames at {scale}x with a border of {border}: only {UPSCALED_FRAME_SCALE}x with \
             {UPSCALED_FRAME_BORDER} reads");
        let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?)
            .ok_or_else(|| eyre::eyre!("invalid mobile upscaled frames pixel format"))?;
        let _reserved = cursor.read_u8()?;
        let atlas_width = cursor.read_u32::<LittleEndian>()?;
        let atlas_height = cursor.read_u32::<LittleEndian>()?;
        let gutter = cursor.read_u16::<LittleEndian>()?;
        let page_count = cursor.read_u32::<LittleEndian>()? as usize;
        let frame_count = cursor.read_u32::<LittleEndian>()? as usize;
        let expected = HEADER_BYTES + page_count * PAGE_RECORD_BYTES + frame_count * FRAME_RECORD_BYTES;
        eyre::ensure!(bytes.len() == expected, "mobile upscaled frames: {} bytes, expected {expected}", bytes.len());
        let pages = (0..page_count).map(|_| Ok(UpscaledPageRecord {
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
        })).collect::<eyre::Result<Vec<_>>>()?;
        let frames = (0..frame_count).map(|_| Ok(UpscaledFrameRecord {
            frame: cursor.read_u32::<LittleEndian>()?,
            page: cursor.read_u32::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
        })).collect::<eyre::Result<Vec<_>>>()?;
        eyre::ensure!(frames.windows(2).all(|pair| pair[0].frame < pair[1].frame),
            "mobile upscaled frames: the records are not sorted by frame");
        eyre::ensure!(frames.iter().all(|record| (record.page as usize) < pages.len()),
            "mobile upscaled frames: a record names a page past the {} there are", pages.len());
        Ok(Self { atlas_width, atlas_height, gutter, pixel_format, pages, frames })
    }

    /// The table of `package`, `None` without the entry.
    pub fn read(package: &UddpReader) -> eyre::Result<Option<Self>> {
        if package.find_by_path_hash(udd_container::xxh64_virtual_path(UPSCALED_FRAMES_ENTRY_PATH)).is_none() {
            return Ok(None);
        }
        Self::parse(&read_path_entry_cow(package, UPSCALED_FRAMES_ENTRY_PATH)?)
            .wrap_err_with(|| format!("read {UPSCALED_FRAMES_ENTRY_PATH}"))
            .map(Some)
    }

    /// The 2x canvas of `record` as its page stores it (RGBA8, or the BC7 blocks of the canvas padded to whole
    /// blocks), the page read through `pages` and the package's page cache `cache`.
    pub(crate) fn read_stored(&self, package: &UddpReader, record: &UpscaledFrameRecord, cache: &StoredPageCache,
        pages: &mut ClipPages) -> eyre::Result<StoredFramePixels> {
        let page = *self.pages.get(record.page as usize)
            .ok_or_else(|| eyre::eyre!("missing mobile upscaled page {}", record.page))?;
        let layout = PageLayout { pixel_format: self.pixel_format, atlas_width: self.atlas_width,
            atlas_height: self.atlas_height, used_width: page.used_width, used_height: page.used_height };
        let stored = pages.page(cache, (PageSet::Upscaled, record.page), || {
            StoredPage::read(package, &upscaled_page_entry_path(record.page, self.pixel_format), layout)
        })?;
        stored.frame(layout, record.x, record.y, record.width, record.height)
    }

    /// The 2x canvas of `record` as straight-alpha RGBA8.
    pub(crate) fn read_rgba(&self, package: &UddpReader, record: &UpscaledFrameRecord, pages: &mut PageMemo)
        -> eyre::Result<Vec<u8>> {
        let (page, bytes) = self.page_bytes(package, record, pages)?;
        extract_atlas_subrect_rgba(bytes, self.pixel_format, self.atlas_width, self.atlas_height, page.used_width,
            page.used_height, record.x, record.y, record.width, record.height)
    }

    fn page_bytes<'m>(&self, package: &UddpReader, record: &UpscaledFrameRecord, pages: &'m mut PageMemo)
        -> eyre::Result<(UpscaledPageRecord, &'m [u8])> {
        let page = *self.pages.get(record.page as usize)
            .ok_or_else(|| eyre::eyre!("missing mobile upscaled page {}", record.page))?;
        let bytes = pages.page(record.page, |index| {
            read_path_entry(package, &upscaled_page_entry_path(index, self.pixel_format))
                .wrap_err_with(|| format!("unpack mobile upscaled page {index}"))
        })?;
        Ok((page, bytes))
    }
}

/// The index of `record` among `records`, which holds it.
pub(crate) fn record_index<T>(records: &[T], record: &T) -> u32 {
    let offset = (record as *const T as usize).wrapping_sub(records.as_ptr() as usize);
    let index = offset / std::mem::size_of::<T>().max(1);
    debug_assert!(index < records.len(), "the record is not among the records");
    index as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> UpscaledFrameTable {
        UpscaledFrameTable {
            atlas_width: 2048,
            atlas_height: 2048,
            gutter: 4,
            pixel_format: PagePixelFormat::Bc7,
            pages: vec![UpscaledPageRecord { used_width: 512, used_height: 256 }],
            frames: vec![
                UpscaledFrameRecord { frame: 3, page: 0, x: 0, y: 0, width: 40, height: 52 },
                UpscaledFrameRecord { frame: 9, page: 0, x: 48, y: 0, width: 36, height: 60 },
            ],
        }
    }

    #[test]
    fn the_table_reads_back_as_written() {
        let table = table();
        assert_eq!(UpscaledFrameTable::parse(&table.encode()).unwrap(), table);
        assert_eq!(table.frame(9).map(|record| record.x), Some(48));
        assert_eq!(table.frame(4), None);
    }

    #[test]
    fn a_table_out_of_order_or_past_its_pages_does_not_read() {
        let mut swapped = table();
        swapped.frames.swap(0, 1);
        let mut bytes = swapped.encode();
        // `encode` sorts; put the records out of order by hand.
        let records = HEADER_BYTES + PAGE_RECORD_BYTES;
        let (first, second) = bytes[records..].split_at_mut(FRAME_RECORD_BYTES);
        first.swap_with_slice(second);
        assert!(UpscaledFrameTable::parse(&bytes).unwrap_err().to_string().contains("not sorted"));
        let mut past = table();
        past.frames[1].page = 1;
        assert!(UpscaledFrameTable::parse(&past.encode()).unwrap_err().to_string().contains("past the 1"));
    }

    #[test]
    fn the_canvas_is_the_frame_with_its_border_at_2x() {
        assert_eq!(UpscaledFrameTable::canvas_size(16, 20), (40, 48));
        assert_eq!(UpscaledFrameTable::canvas_anchor(-3), -2);
        assert_eq!(UpscaledFrameTable::canvas_anchor(10), 24);
    }

    #[test]
    fn a_record_is_found_by_its_place() {
        let records = [1u64, 2, 3];
        assert_eq!(record_index(&records, &records[2]), 2);
    }
}
