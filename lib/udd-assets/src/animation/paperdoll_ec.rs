//! `paperdoll_ec.uddp`: the frames of the Enhanced Client's paperdoll (the male and female sets of `Paperdoll.uop`),
//! decoded losslessly, with what hueing them needs, so `uocf::enhanced::paperdoll` draws a paperdoll from the
//! package with the colours it draws from the client files (`FrameSets::decode`).
//!
//! Path-addressed entries:
//! - `metadata/frames.bin`: magic `PDFR`, `u32` version, `u32` set count; per set its header box (`i16` x0, y0, x1,
//!   y1) and the shadow statistics of its palette (`u8` min, mean and max inverse luminance, `u8` reserved 0); `u32`
//!   frame count; then per frame, ascending by (set, number): `u8` set, `u8` hue mode (`FRAME_HUE_*`), `u16` frame
//!   number, `i16` x and y (where pixel (0, 0) goes from the anchor), `u16` width and height.
//! - `frames/SS/NNNNN.rgba8888` (`DataType::Paperdoll`): the straight-alpha RGBA of frame NNNNN of set SS, as
//!   `AnimationFrame::decode_frame` gives it; absent for an empty frame.
//! - `frames/SS/NNNNN.r8` (`DataType::Paperdoll`): the hue mask of a `FRAME_HUE_MASKED` frame, `HUE_MASK_HUEABLE`
//!   on the pixels whose palette colour takes the hue, else 0.
//! - `metadata/hue_tables.bin`: magic `PDHT`, `u32` version, `u32` count, then per table a `u16` bitmap number and
//!   256 RGBA entries: the tables of `Hues.uop` (`EcHueTables`).
//! - `metadata/dress_hides.bin`: magic `PDDH`, `u32` version, `u32` count, then per record, ascending by (item id,
//!   body): `u32` item id, `u16` body, `u32` hidden layers (bit n: layer n of `uocf::enhanced::paperdoll`). What
//!   the packer measured each dress-layer item to hide on each wearer body ([`PaperdollEcDressHides`]); not client
//!   data.
//!
//! Only frames the paperdoll can find are stored: those whose action is their set's index
//! (`UOAnimations::FindFrameEntry`), the first of a repeated number.

use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};
use udd_container::UddpReader;
use uocf::enhanced::animationframe::{AnimationBounds, HUE_MASK_HUEABLE};
use uocf::enhanced::hues::{EcHueTable, EcHueTables, ShadowPaletteStats, HUE_TABLE_ENTRIES};
use uocf::enhanced::paperdoll::{
    plan, FrameRef, Paperdoll, PaperdollDefinitions, PaperdollFrames, PlacedFrame, WearableRules, WornItem,
    GENDER_SET_COUNT, LAYER_COUNT,
};

use crate::common::{ensure_manifest_version, read_path_entry, read_path_entry_cow};
pub use crate::mobile_anim_ec::{FRAME_HUE_ALL, FRAME_HUE_MASKED, FRAME_HUE_NONE};

pub const FRAMES_ENTRY_PATH: &str = "metadata/frames.bin";
pub const HUE_TABLES_ENTRY_PATH: &str = "metadata/hue_tables.bin";
pub const DRESS_HIDES_ENTRY_PATH: &str = "metadata/dress_hides.bin";
pub const FRAMES_MAGIC: [u8; 4] = *b"PDFR";
pub const HUE_TABLES_MAGIC: [u8; 4] = *b"PDHT";
pub const DRESS_HIDES_MAGIC: [u8; 4] = *b"PDDH";
/// Version of the three manifests, written by the converter. 2 adds `metadata/dress_hides.bin`.
pub const PAPERDOLL_EC_METADATA_VERSION: u32 = 2;
const RGBA_CHANNELS: usize = 4;
const ALPHA_CHANNEL: usize = 3;
/// Bytes of a table entry: its bitmap number, then its RGBA entries.
const HUE_TABLE_RECORD_BYTES: usize = std::mem::size_of::<u16>() + HUE_TABLE_ENTRIES * RGBA_CHANNELS;
/// Bytes of a dress record: item id, body and hidden layers.
const DRESS_HIDES_RECORD_BYTES: usize = std::mem::size_of::<u32>() + std::mem::size_of::<u16>() + std::mem::size_of::<u32>();

/// One frame of a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperdollEcFrameRecord {
    pub set: u8,
    /// `FRAME_HUE_*`: which pixels take a hue.
    pub hue_mode: u8,
    pub frame_number: u16,
    /// Where pixel (0, 0) of the frame goes from the anchor: `x0, y0` of the frame box.
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

impl PaperdollEcFrameRecord {
    pub fn pixel_count(&self) -> usize {
        usize::from(self.width) * usize::from(self.height)
    }
}

/// What the package keeps of a set besides its frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperdollEcSet {
    /// The box of the `Paperdoll.uop` set header.
    pub bounds: AnimationBounds,
    /// What `SHADOW_HUE` normalizes against: the statistics of the set's palette.
    pub shadow: ShadowPaletteStats,
}

/// What a dress-layer item hides on one wearer body: the layers Dynamapper's 3D paperdoll takes off when the client
/// draws them beneath the item, because only their edges would show past it. The packer measures it
/// (`udd_conv::paperdoll_ec_dress`); the client has no such rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperdollEcDressHides {
    pub item_id: u32,
    /// The wearer's body: the number of the body frame the paperdoll draws.
    pub body: u16,
    /// Bit n set: layer n (`uocf::enhanced::paperdoll::SLOT_NAMES`) is taken off.
    pub hidden_layers: u32,
}

impl PaperdollEcDressHides {
    /// Whether layer `layer` is taken off.
    pub fn hides(&self, layer: usize) -> bool {
        layer < LAYER_COUNT && self.hidden_layers & (1 << layer) != 0
    }
}

/// The mask of `PaperdollEcDressHides::hidden_layers` with the given layers set.
pub fn hidden_layer_mask(layers: impl IntoIterator<Item = usize>) -> u32 {
    layers.into_iter().filter(|&layer| layer < LAYER_COUNT).fold(0, |mask, layer| mask | 1 << layer)
}

pub fn frame_entry_path(set: u8, frame_number: u16) -> String {
    format!("frames/{set:02}/{frame_number:05}.rgba8888")
}

pub fn frame_hue_mask_entry_path(set: u8, frame_number: u16) -> String {
    format!("frames/{set:02}/{frame_number:05}.r8")
}

/// `FRAME_HUE_*` of a decoded frame: which coded (alpha > 0) pixels its hue mask marks.
pub fn frame_hue_mode(rgba: &[u8], hue_mask: &[u8]) -> u8 {
    let mut hueable = false;
    let mut kept = false;
    for (pixel, &mask) in rgba.chunks_exact(RGBA_CHANNELS).zip(hue_mask) {
        if pixel[ALPHA_CHANNEL] == 0 {
            continue;
        }
        if mask == HUE_MASK_HUEABLE {
            hueable = true;
        } else {
            kept = true;
        }
        if hueable && kept {
            return FRAME_HUE_MASKED;
        }
    }
    if hueable { FRAME_HUE_ALL } else { FRAME_HUE_NONE }
}

/// The bytes of `metadata/frames.bin`.
pub fn encode_frames_manifest(sets: &[PaperdollEcSet], frames: &[PaperdollEcFrameRecord]) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&FRAMES_MAGIC);
    bytes.extend_from_slice(&PAPERDOLL_EC_METADATA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(sets.len())?.to_le_bytes());
    for set in sets {
        for value in [set.bounds.x0, set.bounds.y0, set.bounds.x1, set.bounds.y1] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[set.shadow.min_inverse, set.shadow.mean_inverse, set.shadow.max_inverse, 0]);
    }
    bytes.extend_from_slice(&u32::try_from(frames.len())?.to_le_bytes());
    for frame in frames {
        bytes.extend_from_slice(&[frame.set, frame.hue_mode]);
        bytes.extend_from_slice(&frame.frame_number.to_le_bytes());
        bytes.extend_from_slice(&frame.x.to_le_bytes());
        bytes.extend_from_slice(&frame.y.to_le_bytes());
        bytes.extend_from_slice(&frame.width.to_le_bytes());
        bytes.extend_from_slice(&frame.height.to_le_bytes());
    }
    Ok(bytes)
}

/// The bytes of `metadata/hue_tables.bin`.
pub fn encode_hue_tables(tables: &EcHueTables) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(tables.tables.len() * HUE_TABLE_RECORD_BYTES);
    bytes.extend_from_slice(&HUE_TABLES_MAGIC);
    bytes.extend_from_slice(&PAPERDOLL_EC_METADATA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(tables.tables.len())?.to_le_bytes());
    for (number, table) in &tables.tables {
        bytes.extend_from_slice(&number.to_le_bytes());
        for entry in &table.entries {
            bytes.extend_from_slice(entry);
        }
    }
    Ok(bytes)
}

/// The bytes of `metadata/dress_hides.bin`; `records` ascending by (item id, body).
pub fn encode_dress_hides(records: &[PaperdollEcDressHides]) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(records.len() * DRESS_HIDES_RECORD_BYTES);
    bytes.extend_from_slice(&DRESS_HIDES_MAGIC);
    bytes.extend_from_slice(&PAPERDOLL_EC_METADATA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(records.len())?.to_le_bytes());
    for record in records {
        bytes.extend_from_slice(&record.item_id.to_le_bytes());
        bytes.extend_from_slice(&record.body.to_le_bytes());
        bytes.extend_from_slice(&record.hidden_layers.to_le_bytes());
    }
    Ok(bytes)
}

fn read_magic(cursor: &mut Cursor<&[u8]>, expected: [u8; 4], entry_path: &str) -> eyre::Result<()> {
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic).wrap_err_with(|| format!("{entry_path}: truncated"))?;
    eyre::ensure!(magic == expected, "{entry_path}: magic {magic:?}, expected {expected:?}");
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version(entry_path, version, PAPERDOLL_EC_METADATA_VERSION)
}

fn ensure_consumed(cursor: &Cursor<&[u8]>, entry_path: &str) -> eyre::Result<()> {
    let trailing = cursor.get_ref().len() as u64 - cursor.position();
    eyre::ensure!(trailing == 0, "{entry_path}: {trailing} trailing bytes");
    Ok(())
}

fn parse_frames_manifest(bytes: &[u8]) -> eyre::Result<(Vec<PaperdollEcSet>, Vec<PaperdollEcFrameRecord>)> {
    let mut cursor = Cursor::new(bytes);
    read_magic(&mut cursor, FRAMES_MAGIC, FRAMES_ENTRY_PATH)?;
    let set_count = cursor.read_u32::<LittleEndian>()? as usize;
    eyre::ensure!(set_count == GENDER_SET_COUNT, "{FRAMES_ENTRY_PATH}: {set_count} sets, expected {GENDER_SET_COUNT}");
    let mut sets = Vec::with_capacity(set_count);
    for _ in 0..set_count {
        let mut bounds = [0i16; 4];
        cursor.read_i16_into::<LittleEndian>(&mut bounds)?;
        let mut shadow = [0u8; 4];
        cursor.read_exact(&mut shadow)?;
        sets.push(PaperdollEcSet {
            bounds: AnimationBounds { x0: bounds[0], y0: bounds[1], x1: bounds[2], y1: bounds[3] },
            shadow: ShadowPaletteStats { min_inverse: shadow[0], mean_inverse: shadow[1], max_inverse: shadow[2] },
        });
    }
    let frame_count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut frames: Vec<PaperdollEcFrameRecord> = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        let frame = PaperdollEcFrameRecord {
            set: cursor.read_u8()?,
            hue_mode: cursor.read_u8()?,
            frame_number: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_i16::<LittleEndian>()?,
            y: cursor.read_i16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
        };
        eyre::ensure!(usize::from(frame.set) < set_count, "{FRAMES_ENTRY_PATH}: frame of set {}", frame.set);
        eyre::ensure!(
            matches!(frame.hue_mode, FRAME_HUE_NONE | FRAME_HUE_ALL | FRAME_HUE_MASKED),
            "{FRAMES_ENTRY_PATH}: frame {} of set {} has hue mode {}",
            frame.frame_number,
            frame.set,
            frame.hue_mode
        );
        if let Some(last) = frames.last() {
            eyre::ensure!(
                (last.set, last.frame_number) < (frame.set, frame.frame_number),
                "{FRAMES_ENTRY_PATH}: frame {} of set {} after frame {} of set {}",
                frame.frame_number,
                frame.set,
                last.frame_number,
                last.set
            );
        }
        frames.push(frame);
    }
    ensure_consumed(&cursor, FRAMES_ENTRY_PATH)?;
    Ok((sets, frames))
}

fn parse_hue_tables(bytes: &[u8]) -> eyre::Result<EcHueTables> {
    let mut cursor = Cursor::new(bytes);
    read_magic(&mut cursor, HUE_TABLES_MAGIC, HUE_TABLES_ENTRY_PATH)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut tables = EcHueTables::default();
    for _ in 0..count {
        let number = cursor.read_u16::<LittleEndian>()?;
        let mut table = EcHueTable { entries: [[0; RGBA_CHANNELS]; HUE_TABLE_ENTRIES] };
        for entry in &mut table.entries {
            cursor.read_exact(entry).wrap_err_with(|| format!("{HUE_TABLES_ENTRY_PATH}: truncated in table {number}"))?;
        }
        eyre::ensure!(
            tables.tables.insert(number, table).is_none(),
            "{HUE_TABLES_ENTRY_PATH}: table {number} repeats"
        );
    }
    ensure_consumed(&cursor, HUE_TABLES_ENTRY_PATH)?;
    Ok(tables)
}

fn parse_dress_hides(bytes: &[u8]) -> eyre::Result<Vec<PaperdollEcDressHides>> {
    let mut cursor = Cursor::new(bytes);
    read_magic(&mut cursor, DRESS_HIDES_MAGIC, DRESS_HIDES_ENTRY_PATH)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut records: Vec<PaperdollEcDressHides> = Vec::with_capacity(count);
    for _ in 0..count {
        let record = PaperdollEcDressHides {
            item_id: cursor.read_u32::<LittleEndian>()?,
            body: cursor.read_u16::<LittleEndian>()?,
            hidden_layers: cursor.read_u32::<LittleEndian>()?,
        };
        eyre::ensure!(
            record.hidden_layers >> LAYER_COUNT == 0,
            "{DRESS_HIDES_ENTRY_PATH}: item {} on body {} hides a layer past {LAYER_COUNT}: {:#x}",
            record.item_id,
            record.body,
            record.hidden_layers
        );
        if let Some(last) = records.last() {
            eyre::ensure!(
                (last.item_id, last.body) < (record.item_id, record.body),
                "{DRESS_HIDES_ENTRY_PATH}: item {} on body {} after item {} on body {}",
                record.item_id,
                record.body,
                last.item_id,
                last.body
            );
        }
        records.push(record);
    }
    ensure_consumed(&cursor, DRESS_HIDES_ENTRY_PATH)?;
    Ok(records)
}

pub struct PaperdollEcPackage {
    package: UddpReader,
    sets: Vec<PaperdollEcSet>,
    frames: Vec<PaperdollEcFrameRecord>,
    hues: EcHueTables,
    dress_hides: Vec<PaperdollEcDressHides>,
}

impl PaperdollEcPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package =
            UddpReader::load(path.as_ref()).wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn load_in_memory(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load_in_memory(path.as_ref())
            .wrap_err_with(|| format!("load_in_memory {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        let (sets, frames) = parse_frames_manifest(&read_path_entry_cow(&package, FRAMES_ENTRY_PATH)?)?;
        let hues = parse_hue_tables(&read_path_entry_cow(&package, HUE_TABLES_ENTRY_PATH)?)?;
        let dress_hides = parse_dress_hides(&read_path_entry_cow(&package, DRESS_HIDES_ENTRY_PATH)?)?;
        Ok(Self { package, sets, frames, hues, dress_hides })
    }

    pub fn package(&self) -> &UddpReader {
        &self.package
    }

    pub fn sets(&self) -> &[PaperdollEcSet] {
        &self.sets
    }

    /// Every frame, ascending by (set, number).
    pub fn frames(&self) -> &[PaperdollEcFrameRecord] {
        &self.frames
    }

    pub fn hue_tables(&self) -> &EcHueTables {
        &self.hues
    }

    /// Every dress record, ascending by (item id, body).
    pub fn dress_hides(&self) -> &[PaperdollEcDressHides] {
        &self.dress_hides
    }

    /// What dress-layer item `item_id` hides on `body` (the number of the body frame drawn); `None` when the packer
    /// measured nothing for them.
    pub fn dress_hides_on(&self, item_id: u32, body: u32) -> Option<&PaperdollEcDressHides> {
        let body = u16::try_from(body).ok()?;
        let index = self.dress_hides.binary_search_by_key(&(item_id, body), |record| (record.item_id, record.body)).ok()?;
        Some(&self.dress_hides[index])
    }

    /// Frame `frame_number` of `set`.
    pub fn frame(&self, set: u8, frame_number: u16) -> Option<&PaperdollEcFrameRecord> {
        let index = self.frames.binary_search_by_key(&(set, frame_number), |frame| (frame.set, frame.frame_number)).ok()?;
        Some(&self.frames[index])
    }

    /// The straight-alpha RGBA of a frame, unhued.
    pub fn read_frame_rgba(&self, frame: &PaperdollEcFrameRecord) -> eyre::Result<Vec<u8>> {
        if frame.pixel_count() == 0 {
            return Ok(Vec::new());
        }
        let rgba = read_path_entry(&self.package, &frame_entry_path(frame.set, frame.frame_number))?;
        eyre::ensure!(
            rgba.len() == frame.pixel_count() * RGBA_CHANNELS,
            "frame {} of set {}: {} bytes for {}x{}",
            frame.frame_number,
            frame.set,
            rgba.len(),
            frame.width,
            frame.height
        );
        Ok(rgba)
    }

    /// The hue mask of a frame, one byte per pixel: `HUE_MASK_HUEABLE` where a pixel takes the hue, else 0.
    pub fn read_frame_hue_mask(&self, frame: &PaperdollEcFrameRecord, rgba: &[u8]) -> eyre::Result<Vec<u8>> {
        match frame.hue_mode {
            FRAME_HUE_NONE => Ok(vec![0; frame.pixel_count()]),
            FRAME_HUE_ALL => Ok(rgba
                .chunks_exact(RGBA_CHANNELS)
                .map(|pixel| if pixel[ALPHA_CHANNEL] != 0 { HUE_MASK_HUEABLE } else { 0 })
                .collect()),
            _ => {
                let mask = read_path_entry(&self.package, &frame_hue_mask_entry_path(frame.set, frame.frame_number))?;
                eyre::ensure!(
                    mask.len() == frame.pixel_count(),
                    "frame {} of set {}: hue mask of {} bytes for {}x{}",
                    frame.frame_number,
                    frame.set,
                    mask.len(),
                    frame.width,
                    frame.height
                );
                Ok(mask)
            }
        }
    }

    /// A frame decoded and hued, placed from the anchor: `FrameSets::decode` of the client files.
    pub fn decode(&self, frame: FrameRef<PaperdollEcFrameRecord>, hue: u16) -> eyre::Result<PlacedFrame> {
        let record = frame.entry;
        let mut rgba = self.read_frame_rgba(&record)?;
        let mask = self.read_frame_hue_mask(&record, &rgba)?;
        let set = self.sets.get(frame.set).ok_or_else(|| eyre::eyre!("no frame set {}", frame.set))?;
        self.hues.apply(hue, &mut rgba, &mask, &set.shadow);
        Ok(PlacedFrame {
            x: i32::from(record.x),
            y: i32::from(record.y),
            width: u32::from(record.width),
            height: u32::from(record.height),
            rgba,
        })
    }

    /// What the client draws for a body wearing `items`: `uocf::enhanced::paperdoll::build` over the package.
    pub fn build<'a, T: PaperdollDefinitions<'a>>(
        &self,
        definitions: T,
        rules: &WearableRules,
        body: u32,
        body_hue: u16,
        items: &[WornItem],
    ) -> eyre::Result<Paperdoll> {
        let plan = plan(self, definitions, rules, body, body_hue, items)?;
        let frames =
            plan.frames.iter().map(|planned| self.decode(planned.frame, planned.hue)).collect::<eyre::Result<_>>()?;
        Ok(Paperdoll { frames, notes: plan.notes })
    }
}

impl PaperdollFrames for PaperdollEcPackage {
    type Frame = PaperdollEcFrameRecord;

    fn find_in_set(&self, set: usize, number: u16) -> Option<PaperdollEcFrameRecord> {
        self.frame(u8::try_from(set).ok()?, number).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HUE_NUMBER: u16 = 38;
    const SET_X0: i16 = -256;
    const SET_Y0: i16 = -306;
    const SET_X1: i16 = 256;
    const SET_Y1: i16 = 206;
    const BODY_FRAME: u16 = 400;
    const ITEM_FRAME: u16 = 5399;
    const FRAME_X: i16 = -40;
    const FRAME_Y: i16 = -120;
    const FRAME_WIDTH: u16 = 3;
    const FRAME_HEIGHT: u16 = 2;
    const SHADOW: ShadowPaletteStats = ShadowPaletteStats { min_inverse: 3, mean_inverse: 90, max_inverse: 250 };
    /// Bytes of the version field after the magic.
    const VERSION_RANGE: std::ops::Range<usize> = 4..8;

    fn sets() -> Vec<PaperdollEcSet> {
        let bounds = AnimationBounds { x0: SET_X0, y0: SET_Y0, x1: SET_X1, y1: SET_Y1 };
        vec![PaperdollEcSet { bounds, shadow: SHADOW }; GENDER_SET_COUNT]
    }

    fn frame(set: u8, frame_number: u16, hue_mode: u8) -> PaperdollEcFrameRecord {
        PaperdollEcFrameRecord {
            set,
            hue_mode,
            frame_number,
            x: FRAME_X,
            y: FRAME_Y,
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
        }
    }

    #[test]
    fn the_frames_manifest_round_trips() {
        let frames = [
            frame(0, BODY_FRAME, FRAME_HUE_ALL),
            frame(0, ITEM_FRAME, FRAME_HUE_MASKED),
            frame(1, BODY_FRAME, FRAME_HUE_NONE),
        ];
        let bytes = encode_frames_manifest(&sets(), &frames).unwrap();
        assert_eq!(parse_frames_manifest(&bytes).unwrap(), (sets(), frames.to_vec()));
    }

    #[test]
    fn an_older_manifest_is_refused_with_a_repack_message() {
        let mut bytes = encode_frames_manifest(&sets(), &[frame(0, BODY_FRAME, FRAME_HUE_ALL)]).unwrap();
        bytes[VERSION_RANGE].copy_from_slice(&(PAPERDOLL_EC_METADATA_VERSION - 1).to_le_bytes());
        let error = parse_frames_manifest(&bytes).unwrap_err().to_string();
        assert!(error.contains("repack"), "{error}");
    }

    #[test]
    fn unsorted_frames_unknown_hue_modes_and_truncation_are_refused() {
        let unsorted = [frame(0, ITEM_FRAME, FRAME_HUE_ALL), frame(0, BODY_FRAME, FRAME_HUE_ALL)];
        assert!(parse_frames_manifest(&encode_frames_manifest(&sets(), &unsorted).unwrap()).is_err());
        let unknown_mode = [frame(0, BODY_FRAME, FRAME_HUE_MASKED + 1)];
        assert!(parse_frames_manifest(&encode_frames_manifest(&sets(), &unknown_mode).unwrap()).is_err());
        let mut truncated = encode_frames_manifest(&sets(), &[frame(0, BODY_FRAME, FRAME_HUE_ALL)]).unwrap();
        truncated.pop();
        assert!(parse_frames_manifest(&truncated).is_err());
    }

    #[test]
    fn the_hue_tables_round_trip() {
        let mut table = EcHueTable { entries: [[0; RGBA_CHANNELS]; HUE_TABLE_ENTRIES] };
        for (index, entry) in table.entries.iter_mut().enumerate() {
            let key = index as u8;
            *entry = [key, u8::MAX - key, key / 2, u8::MAX];
        }
        let mut tables = EcHueTables::default();
        tables.tables.insert(HUE_NUMBER, table);
        assert_eq!(parse_hue_tables(&encode_hue_tables(&tables).unwrap()).unwrap(), tables);
    }

    const ROBE: u32 = 7939;
    const HUMAN_MALE: u16 = 400;
    const HUMAN_FEMALE: u16 = 401;
    const TORSO_LAYER: usize = 5;
    const LEGS_LAYER: usize = 4;

    fn dress(item_id: u32, body: u16, layers: &[usize]) -> PaperdollEcDressHides {
        PaperdollEcDressHides { item_id, body, hidden_layers: hidden_layer_mask(layers.iter().copied()) }
    }

    #[test]
    fn the_dress_records_round_trip_and_are_found_by_item_and_body() {
        let records = [dress(ROBE, HUMAN_MALE, &[TORSO_LAYER, LEGS_LAYER]), dress(ROBE, HUMAN_FEMALE, &[])];
        let parsed = parse_dress_hides(&encode_dress_hides(&records).unwrap()).unwrap();
        assert_eq!(parsed, records);
        assert!(parsed[0].hides(TORSO_LAYER) && parsed[0].hides(LEGS_LAYER));
        assert!(!parsed[0].hides(LAYER_COUNT), "past the layers");
        assert_eq!(parsed[1].hidden_layers, 0);
    }

    #[test]
    fn unsorted_dress_records_masks_past_the_layers_and_an_older_version_are_refused() {
        let unsorted = [dress(ROBE, HUMAN_FEMALE, &[]), dress(ROBE, HUMAN_MALE, &[])];
        assert!(parse_dress_hides(&encode_dress_hides(&unsorted).unwrap()).is_err());
        let past = [PaperdollEcDressHides { item_id: ROBE, body: HUMAN_MALE, hidden_layers: 1 << LAYER_COUNT }];
        assert!(parse_dress_hides(&encode_dress_hides(&past).unwrap()).is_err());
        let mut older = encode_dress_hides(&[dress(ROBE, HUMAN_MALE, &[TORSO_LAYER])]).unwrap();
        older[VERSION_RANGE].copy_from_slice(&(PAPERDOLL_EC_METADATA_VERSION - 1).to_le_bytes());
        let error = parse_dress_hides(&older).unwrap_err().to_string();
        assert!(error.contains("repack"), "{error}");
    }

    #[test]
    fn hue_modes_follow_the_coded_pixels() {
        let opaque = [1, 2, 3, u8::MAX];
        let clear = [0; RGBA_CHANNELS];
        let rgba = [opaque, opaque, clear].concat();
        assert_eq!(frame_hue_mode(&rgba, &[HUE_MASK_HUEABLE, HUE_MASK_HUEABLE, 0]), FRAME_HUE_ALL);
        // A transparent pixel's mask does not count.
        assert_eq!(frame_hue_mode(&rgba, &[0, 0, HUE_MASK_HUEABLE]), FRAME_HUE_NONE);
        assert_eq!(frame_hue_mode(&rgba, &[HUE_MASK_HUEABLE, 0, 0]), FRAME_HUE_MASKED);
        assert_eq!(frame_hue_mode(&[], &[]), FRAME_HUE_NONE);
    }
}
