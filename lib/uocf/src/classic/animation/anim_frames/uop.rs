//! The Classic Client's `AnimationFrame*.uop` entries, `build/animationlegacyframe/%06d/%02d.bin` (one body and
//! action): the AMOU container of the Enhanced Client ([`AmouFile`]) holding Classic frames.
//!
//! - The container: the 40-byte header, a colour table, the 16-byte frame table sorted by (action, frame number)
//!   (the client binary-searches it: `UOClassicAnim::FindFrameEntry`), the frame data. The client reads only the
//!   frame count (+0x20), the table offset (+0x24) and each entry's key and data offset; the header box, the colour
//!   table and the entry boxes are leftovers of the EC conversion that match no Classic frame. A rewrite keeps them
//!   from the entry it replaces; a new entry has no colour table, entry boxes around the frames and the header box
//!   of the EC entries ([`AmouFile::frame_union_bounds`]).
//! - Each frame's data: the 256-word palette (the same in every frame of a shipped entry), then a Classic frame
//!   ([`ClassicFrame`]); an empty frame (width or height 0) is the palette and an 8-byte zero header.
//! - Frame numbers count from 1 over the five stored directions: frame `i` of direction `d` is number
//!   `d * frames_per_direction + i + 1`.

use std::path::Path;

crate::eyre_imports!();

use super::{read_palette, write_palette, ClassicFrame, ClassicPalette, PALETTE_BYTES};
use crate::classic::animationframe_cc::AnimationFrameCc;
use crate::enhanced::animationframe::encode::AMOU_VERSION;
use crate::enhanced::animationframe::package::{write_payload, EntryChange};
use crate::enhanced::animationframe::{AmouFile, AmouFrameRecord, AnimationBounds};
use crate::uop_container::package::UopPackage;

/// The number of the first frame of an entry.
pub const FIRST_FRAME_NUMBER: u16 = 1;

/// One frame of an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicUopFrame {
    pub action: u16,
    pub frame_number: u16,
    pub frame: ClassicFrame,
}

/// An entry as its palette and frames, plus the container fields the client does not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicUopEntry {
    /// Header, colour table, entry boxes and tail as stored; its records' data is not used by [`Self::to_bytes`].
    pub container: AmouFile,
    pub palette: ClassicPalette,
    /// In table order.
    pub frames: Vec<ClassicUopFrame>,
}

impl ClassicUopEntry {
    /// A new entry without frames: version [`AMOU_VERSION`], no colour table.
    pub fn new(animation_id: u32, palette: ClassicPalette) -> Self {
        let container = AmouFile {
            version: AMOU_VERSION,
            animation_id,
            bounds: AnimationBounds::default(),
            colours: Vec::new(),
            frames: Vec::new(),
            tail: Vec::new(),
        };
        Self { container, palette, frames: Vec::new() }
    }

    /// Reads an entry. Fails when its frames have different palettes or a frame has bytes after its end.
    pub fn parse(payload: &[u8]) -> eyre::Result<Self> {
        let (container, _) = AmouFile::parse(payload)?;
        let mut palette = None;
        let mut frames = Vec::with_capacity(container.frames.len());
        for record in &container.frames {
            let context = || format!("action {} frame {}", record.action, record.frame_number);
            let frame_palette = read_palette(&record.data).wrap_err_with(context)?;
            match palette {
                None => palette = Some(frame_palette),
                Some(palette) => {
                    eyre::ensure!(palette == frame_palette, "{}: another palette than the entry's first frame", context())
                }
            }
            let (frame, read) = ClassicFrame::decode(&record.data[PALETTE_BYTES..]).wrap_err_with(context)?;
            let left = record.data.len() - PALETTE_BYTES - read;
            eyre::ensure!(left == 0, "{}: {left} bytes after the frame", context());
            frames.push(ClassicUopFrame { action: record.action, frame_number: record.frame_number, frame });
        }
        let palette = palette.unwrap_or([0; super::PALETTE_COLOURS]);
        Ok(Self { container, palette, frames })
    }

    /// The entry bytes: the frames sorted by (action, frame number), each with the palette; entry boxes kept from
    /// the container for keys it has, else the frame box around the anchor. A new entry (no container frames) gets
    /// the header box of the EC entries. Fails on a repeated key or a frame that does not encode.
    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut file = self.container.clone();
        let new_entry = file.frames.is_empty();
        let mut records = Vec::with_capacity(self.frames.len());
        for frame in &self.frames {
            let context = || format!("action {} frame {}", frame.action, frame.frame_number);
            let mut data = Vec::with_capacity(PALETTE_BYTES);
            write_palette(&self.palette, &mut data);
            frame.frame.encode(&mut data).wrap_err_with(context)?;
            let kept = self
                .container
                .frames
                .iter()
                .find(|record| (record.action, record.frame_number) == (frame.action, frame.frame_number));
            let (x0, y0, x1, y1) = match kept {
                Some(record) => (record.x0, record.y0, record.x1, record.y1),
                None => frame_box(&frame.frame).wrap_err_with(context)?,
            };
            records.push(AmouFrameRecord { action: frame.action, frame_number: frame.frame_number, x0, y0, x1, y1, data });
        }
        records.sort_by_key(|record| (record.action, record.frame_number));
        if let Some(pair) = records.windows(2).find(|pair| (pair[0].action, pair[0].frame_number) == (pair[1].action, pair[1].frame_number)) {
            eyre::bail!("action {} frame {} repeats", pair[0].action, pair[0].frame_number);
        }
        file.frames = records;
        if new_entry {
            file.bounds = file.frame_union_bounds();
        }
        file.to_bytes()
    }
}

/// The box of a frame relative to its anchor, as run offsets count: `(-anchor x, -anchor y - height)` to that plus
/// the size.
pub fn frame_box(frame: &ClassicFrame) -> eyre::Result<(i16, i16, i16, i16)> {
    let x0 = -i32::from(frame.center_x);
    let y0 = -i32::from(frame.center_y) - i32::from(frame.height);
    let x1 = x0 + i32::from(frame.width);
    let y1 = y0 + i32::from(frame.height);
    let fit = |value: i32| i16::try_from(value).map_err(|_| eyre!("frame box value {value} does not fit an i16"));
    Ok((fit(x0)?, fit(y0)?, fit(x1)?, fit(y1)?))
}

/// The payload of the `body` / `action` entry, if the package has it.
pub fn read_uop_entry(package: &UopPackage, body: u32, action: u16) -> eyre::Result<Option<Vec<u8>>> {
    let path = AnimationFrameCc::animationframe_path(body, action);
    package
        .unpack_file_by_hash(AnimationFrameCc::animationframe_hash(body, action))
        .wrap_err_with(|| format!("failed to unpack {path}"))
}

/// Writes a copy of `source` to `output` with `payload` (checked with [`ClassicUopEntry::parse`]) as the `body` /
/// `action` entry, replaced keeping its compression or added. `output` must not be `source`.
pub fn write_uop_entry(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    body: u32,
    action: u16,
    payload: &[u8],
) -> eyre::Result<EntryChange> {
    ClassicUopEntry::parse(payload).wrap_err("the payload is not a Classic animationlegacyframe entry")?;
    write_payload(source, output, AnimationFrameCc::animationframe_hash(body, action), payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_entry_round_trips() {
        let mut palette = [0u16; super::super::PALETTE_COLOURS];
        palette[1] = 0x7FFF;
        let mut entry = ClassicUopEntry::new(400, palette);
        let frame = ClassicFrame { center_x: 1, center_y: -2, width: 2, height: 1, pixels: vec![Some(1), None] };
        entry.frames.push(ClassicUopFrame { action: 0, frame_number: 2, frame: ClassicFrame::empty(0, 0) });
        entry.frames.push(ClassicUopFrame { action: 0, frame_number: FIRST_FRAME_NUMBER, frame });
        let bytes = entry.to_bytes().unwrap();
        let parsed = ClassicUopEntry::parse(&bytes).unwrap();
        assert_eq!(parsed.palette, palette);
        assert_eq!(parsed.frames.iter().map(|frame| frame.frame_number).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(parsed.frames[0].frame, entry.frames[1].frame);
        assert_eq!((parsed.container.frames[0].x0, parsed.container.frames[0].y1), (-1, 2));
        assert!(parsed.container.colours.is_empty());
        // A parsed entry writes itself back.
        assert_eq!(parsed.to_bytes().unwrap(), bytes);
    }
}
