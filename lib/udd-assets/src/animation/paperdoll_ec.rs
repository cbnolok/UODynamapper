//! `paperdoll_ec.uddp`: the frames of the Enhanced Client's paperdoll (the male and female sets of `Paperdoll.uop`),
//! decoded losslessly, with the hue masks and palette statistics hueing them needs, so `uocf::enhanced::paperdoll`
//! draws a paperdoll from the package with the colours it draws from the client files (`FrameSets::decode`). The hue
//! tables come from `hues.uddp`, the single hue source (`HuesPackage::ec_hue_tables_of`).
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
//! - `metadata/dress_decisions.v3.bin`: magic `PDDD`, `u32` version, `u32` slots (bit n: layer n of
//!   `uocf::enhanced::paperdoll`, the layers the decisions take garments off), `u32` count, then per decision,
//!   ascending by (item id, body): `u32` item id, `u16` body, `u16` kept count, then per kept garment, ascending by
//!   id, `u32` item id and `u32` texels beyond the edge. What the packer measured each dress-layer item to take off
//!   on each wearer body ([`PaperdollEcDressDecisions`]); not client data.
//!
//! Only frames the paperdoll can find are stored: those whose action is their set's index
//! (`UOAnimations::FindFrameEntry`), the first of a repeated number.

use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};
use udd_container::UddpReader;
use uocf::enhanced::animationframe::{AnimationBounds, HUE_MASK_HUEABLE};
use uocf::enhanced::hues::{EcHueTables, ShadowPaletteStats};
use uocf::enhanced::paperdoll::{
    plan, FrameRef, Paperdoll, PaperdollDefinitions, PaperdollFrames, PlacedFrame, WearableRules, WornItem,
    GENDER_SET_COUNT, LAYER_COUNT,
};

use crate::common::{ensure_manifest_version, read_path_entry, read_path_entry_cow};
pub use crate::mobile_anim_ec::{FRAME_HUE_ALL, FRAME_HUE_MASKED, FRAME_HUE_NONE};

pub const FRAMES_ENTRY_PATH: &str = "metadata/frames.bin";
/// The dress-layer decisions, per garment since format 3 (format 2 stored a mask of slots at
/// `metadata/dress_hides.bin`).
pub const DRESS_DECISIONS_ENTRY_PATH: &str = "metadata/dress_decisions.v3.bin";
pub const FRAMES_MAGIC: [u8; 4] = *b"PDFR";
pub const DRESS_DECISIONS_MAGIC: [u8; 4] = *b"PDDD";
/// Version of the manifests, written by the converter. 2 added the dress-layer decisions; 3 decides them per garment
/// (`DRESS_DECISIONS_ENTRY_PATH`) and leaves the hue tables to `hues.uddp` (format 2 carried those of `Hues.uop` at
/// `metadata/hue_tables.bin`).
pub const PAPERDOLL_EC_METADATA_VERSION: u32 = 3;
const RGBA_CHANNELS: usize = 4;
const ALPHA_CHANNEL: usize = 3;
/// Bytes of a dress decision before its kept garments: item id, body and kept count.
const DRESS_DECISION_HEADER_BYTES: usize =
    std::mem::size_of::<u32>() + std::mem::size_of::<u16>() + std::mem::size_of::<u16>();
/// Bytes of a kept garment: item id and texels beyond the edge.
const KEPT_GARMENT_BYTES: usize = std::mem::size_of::<u32>() + std::mem::size_of::<u32>();

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

/// A garment a dress-layer item keeps beneath it: it shows beyond the item's edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperdollEcKeptGarment {
    pub item_id: u32,
    /// Texels of the garment that vanish without it beyond the edge band of the item: what of it is in plain view.
    pub beyond_edge: u32,
}

/// What a dress-layer item takes off on one wearer body: every garment of the decisions' slots the client draws
/// beneath it, except the garments it keeps, which show beyond its edges; the others would show only their edges past
/// it. The packer measures it (`udd_conv::paperdoll_ec_dress`); the client has no such rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperdollEcDressDecision {
    pub item_id: u32,
    /// The wearer's body: the number of the body frame the paperdoll draws.
    pub body: u16,
    /// Ascending by item id.
    pub kept: Vec<PaperdollEcKeptGarment>,
}

impl PaperdollEcDressDecision {
    /// The kept garment `item_id`, if kept.
    pub fn kept_garment(&self, item_id: u32) -> Option<&PaperdollEcKeptGarment> {
        let index = self.kept.binary_search_by_key(&item_id, |garment| garment.item_id).ok()?;
        Some(&self.kept[index])
    }
}

/// The dress-layer decisions of a package: the slots they take garments off and one decision per item and body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaperdollEcDressDecisions {
    /// Bit n set: layer n (`uocf::enhanced::paperdoll::SLOT_NAMES`) is one the decisions take garments off.
    pub slots: u32,
    /// Ascending by (item id, body).
    pub decisions: Vec<PaperdollEcDressDecision>,
}

impl PaperdollEcDressDecisions {
    /// Whether the decisions take garments of layer `layer` off.
    pub fn covers(&self, layer: usize) -> bool {
        layer < LAYER_COUNT && self.slots & (1 << layer) != 0
    }

    /// The decision for dress-layer item `item_id` on `body` (the number of the body frame drawn); `None` when the
    /// packer measured nothing for them.
    pub fn on(&self, item_id: u32, body: u32) -> Option<&PaperdollEcDressDecision> {
        let body = u16::try_from(body).ok()?;
        let index =
            self.decisions.binary_search_by_key(&(item_id, body), |decision| (decision.item_id, decision.body)).ok()?;
        Some(&self.decisions[index])
    }

    /// Whether dress-layer item `item_id` takes off garment `garment`, worn on `layer` and drawn beneath it, on `body`:
    /// its decision there covers the layer and does not keep the garment. Without a decision, nothing is taken off.
    pub fn takes_off(&self, item_id: u32, body: u32, layer: usize, garment: u32) -> bool {
        self.covers(layer) && self.on(item_id, body).is_some_and(|decision| decision.kept_garment(garment).is_none())
    }
}

/// The mask with bit n set for each layer n of `layers` below `LAYER_COUNT`.
pub fn layer_mask(layers: impl IntoIterator<Item = usize>) -> u32 {
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

/// The bytes of `DRESS_DECISIONS_ENTRY_PATH`; the decisions ascending by (item id, body), their kept garments by id.
pub fn encode_dress_decisions(decisions: &PaperdollEcDressDecisions) -> eyre::Result<Vec<u8>> {
    let kept: usize = decisions.decisions.iter().map(|decision| decision.kept.len()).sum();
    let mut bytes =
        Vec::with_capacity(decisions.decisions.len() * DRESS_DECISION_HEADER_BYTES + kept * KEPT_GARMENT_BYTES);
    bytes.extend_from_slice(&DRESS_DECISIONS_MAGIC);
    bytes.extend_from_slice(&PAPERDOLL_EC_METADATA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&decisions.slots.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(decisions.decisions.len())?.to_le_bytes());
    for decision in &decisions.decisions {
        bytes.extend_from_slice(&decision.item_id.to_le_bytes());
        bytes.extend_from_slice(&decision.body.to_le_bytes());
        bytes.extend_from_slice(&u16::try_from(decision.kept.len())?.to_le_bytes());
        for garment in &decision.kept {
            bytes.extend_from_slice(&garment.item_id.to_le_bytes());
            bytes.extend_from_slice(&garment.beyond_edge.to_le_bytes());
        }
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

fn parse_dress_decisions(bytes: &[u8]) -> eyre::Result<PaperdollEcDressDecisions> {
    let mut cursor = Cursor::new(bytes);
    read_magic(&mut cursor, DRESS_DECISIONS_MAGIC, DRESS_DECISIONS_ENTRY_PATH)?;
    let slots = cursor.read_u32::<LittleEndian>()?;
    eyre::ensure!(slots >> LAYER_COUNT == 0, "{DRESS_DECISIONS_ENTRY_PATH}: a slot past {LAYER_COUNT}: {slots:#x}");
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut decisions: Vec<PaperdollEcDressDecision> = Vec::with_capacity(count);
    for _ in 0..count {
        let item_id = cursor.read_u32::<LittleEndian>()?;
        let body = cursor.read_u16::<LittleEndian>()?;
        let kept_count = cursor.read_u16::<LittleEndian>()?;
        let truncated = || format!("{DRESS_DECISIONS_ENTRY_PATH}: truncated in item {item_id} on body {body}");
        let mut kept: Vec<PaperdollEcKeptGarment> = Vec::with_capacity(usize::from(kept_count));
        for _ in 0..kept_count {
            let garment = PaperdollEcKeptGarment {
                item_id: cursor.read_u32::<LittleEndian>().wrap_err_with(truncated)?,
                beyond_edge: cursor.read_u32::<LittleEndian>().wrap_err_with(truncated)?,
            };
            if let Some(last) = kept.last() {
                eyre::ensure!(
                    last.item_id < garment.item_id,
                    "{DRESS_DECISIONS_ENTRY_PATH}: item {item_id} on body {body} keeps garment {} after {}",
                    garment.item_id,
                    last.item_id
                );
            }
            kept.push(garment);
        }
        if let Some(last) = decisions.last() {
            eyre::ensure!(
                (last.item_id, last.body) < (item_id, body),
                "{DRESS_DECISIONS_ENTRY_PATH}: item {item_id} on body {body} after item {} on body {}",
                last.item_id,
                last.body
            );
        }
        decisions.push(PaperdollEcDressDecision { item_id, body, kept });
    }
    ensure_consumed(&cursor, DRESS_DECISIONS_ENTRY_PATH)?;
    Ok(PaperdollEcDressDecisions { slots, decisions })
}

pub struct PaperdollEcPackage {
    package: UddpReader,
    sets: Vec<PaperdollEcSet>,
    frames: Vec<PaperdollEcFrameRecord>,
    dress: PaperdollEcDressDecisions,
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
        let dress = parse_dress_decisions(&read_path_entry_cow(&package, DRESS_DECISIONS_ENTRY_PATH)?)?;
        Ok(Self { package, sets, frames, dress })
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

    /// What each dress-layer item takes off on each body measured.
    pub fn dress_decisions(&self) -> &PaperdollEcDressDecisions {
        &self.dress
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

    /// A frame decoded and hued with `hues` (the tables of `hues.uddp`: `HuesPackage::ec_hue_tables_of`), placed from
    /// the anchor: `FrameSets::decode` of the client files.
    pub fn decode(
        &self,
        frame: FrameRef<PaperdollEcFrameRecord>,
        hue: u16,
        hues: &EcHueTables,
    ) -> eyre::Result<PlacedFrame> {
        let record = frame.entry;
        let mut rgba = self.read_frame_rgba(&record)?;
        let mask = self.read_frame_hue_mask(&record, &rgba)?;
        let set = self.sets.get(frame.set).ok_or_else(|| eyre::eyre!("no frame set {}", frame.set))?;
        hues.apply(hue, &mut rgba, &mask, &set.shadow);
        Ok(PlacedFrame {
            x: i32::from(record.x),
            y: i32::from(record.y),
            width: u32::from(record.width),
            height: u32::from(record.height),
            rgba,
        })
    }

    /// What the client draws for a body wearing `items`, hued with `hues`: `uocf::enhanced::paperdoll::build` over the
    /// package.
    pub fn build<'a, T: PaperdollDefinitions<'a>>(
        &self,
        definitions: T,
        rules: &WearableRules,
        hues: &EcHueTables,
        body: u32,
        body_hue: u16,
        items: &[WornItem],
    ) -> eyre::Result<Paperdoll> {
        let plan = plan(self, definitions, rules, body, body_hue, items)?;
        let frames = plan
            .frames
            .iter()
            .map(|planned| self.decode(planned.frame, planned.hue, hues))
            .collect::<eyre::Result<_>>()?;
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

    const ROBE: u32 = 7939;
    const HUMAN_MALE: u16 = 400;
    const HUMAN_FEMALE: u16 = 401;
    const TORSO_LAYER: usize = 5;
    const LEGS_LAYER: usize = 4;

    const SHIRT: u32 = 0x1517;
    const PLATE_TUNIC: u32 = 0x1415;
    const PLATE_LEGS: u32 = 0x1411;
    const WAIST_LAYER: usize = 12;
    const PLATE_TUNIC_BEYOND: u32 = 5210;

    fn decision(item_id: u32, body: u16, kept: &[(u32, u32)]) -> PaperdollEcDressDecision {
        let kept = kept.iter().map(|&(item_id, beyond_edge)| PaperdollEcKeptGarment { item_id, beyond_edge }).collect();
        PaperdollEcDressDecision { item_id, body, kept }
    }

    fn decisions(decisions: Vec<PaperdollEcDressDecision>) -> PaperdollEcDressDecisions {
        PaperdollEcDressDecisions { slots: layer_mask([TORSO_LAYER, LEGS_LAYER]), decisions }
    }

    /// A robe keeps the plate tunic beneath it on the male body and takes off the shirt and the plate legs; it takes
    /// off nothing of a slot the decisions do not cover, nor on a body it has no decision for.
    #[test]
    fn the_dress_decisions_round_trip_and_take_off_what_they_do_not_keep() {
        let stored = decisions(vec![
            decision(ROBE, HUMAN_MALE, &[(PLATE_TUNIC, PLATE_TUNIC_BEYOND)]),
            decision(ROBE, HUMAN_FEMALE, &[]),
        ]);
        let parsed = parse_dress_decisions(&encode_dress_decisions(&stored).unwrap()).unwrap();
        assert_eq!(parsed, stored);
        let male = u32::from(HUMAN_MALE);
        assert!(parsed.takes_off(ROBE, male, TORSO_LAYER, SHIRT));
        assert!(parsed.takes_off(ROBE, male, LEGS_LAYER, PLATE_LEGS));
        assert!(!parsed.takes_off(ROBE, male, TORSO_LAYER, PLATE_TUNIC), "kept");
        assert_eq!(parsed.on(ROBE, male).unwrap().kept_garment(PLATE_TUNIC).unwrap().beyond_edge, PLATE_TUNIC_BEYOND);
        assert!(!parsed.takes_off(ROBE, male, WAIST_LAYER, SHIRT), "a slot not covered");
        assert!(!parsed.takes_off(ROBE + 1, male, TORSO_LAYER, SHIRT), "no decision");
        assert!(!parsed.covers(LAYER_COUNT), "past the layers");
        assert!(parsed.takes_off(ROBE, u32::from(HUMAN_FEMALE), TORSO_LAYER, PLATE_TUNIC));
    }

    #[test]
    fn unsorted_dress_decisions_slots_past_the_layers_and_an_older_version_are_refused() {
        let unsorted = decisions(vec![decision(ROBE, HUMAN_FEMALE, &[]), decision(ROBE, HUMAN_MALE, &[])]);
        assert!(parse_dress_decisions(&encode_dress_decisions(&unsorted).unwrap()).is_err());
        let unsorted_kept = decisions(vec![decision(ROBE, HUMAN_MALE, &[(SHIRT, 0), (PLATE_TUNIC, 0)])]);
        assert!(parse_dress_decisions(&encode_dress_decisions(&unsorted_kept).unwrap()).is_err());
        let past = PaperdollEcDressDecisions { slots: 1 << LAYER_COUNT, decisions: Vec::new() };
        assert!(parse_dress_decisions(&encode_dress_decisions(&past).unwrap()).is_err());
        let mut truncated = encode_dress_decisions(&decisions(vec![decision(ROBE, HUMAN_MALE, &[(SHIRT, 0)])])).unwrap();
        truncated.pop();
        assert!(parse_dress_decisions(&truncated).is_err());
        let mut older = encode_dress_decisions(&decisions(Vec::new())).unwrap();
        older[VERSION_RANGE].copy_from_slice(&(PAPERDOLL_EC_METADATA_VERSION - 1).to_le_bytes());
        let error = parse_dress_decisions(&older).unwrap_err().to_string();
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
