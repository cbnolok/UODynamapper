//! What each dress-layer item takes off in Dynamapper's paperdolls, measured when `paperdoll_ec.uddp` is packed and
//! stored in it (`udd_assets::paperdoll_ec::PaperdollEcDressDecisions`). The client has no such rule.
//!
//! The Enhanced Client draws the shirt, trousers, skirt, sleeves and chest pieces beneath a robe, and their 3D frames do
//! not line up with the robe's: their edges show past it, at the collar, down the sides and at the hem. Taking them off
//! fits a robe, not a dress-layer item that leaves them in view, as epaulets, a tabard or a jacket do. So for each
//! dress-layer item, on each wearer body it has a frame for, and for each garment of the slots that may be taken off
//! ([`HIDEABLE_SLOTS`]) the client draws beneath the item, the packer draws the body wearing the garment and the item,
//! in the client's draw order, then without the garment, and counts the texels that vanish where the item is not
//! opaque. Within [`EDGE_BAND_TEXELS`] of the item they are the edges showing past it; beyond, the garment was in plain
//! view. The garment is kept when what vanishes beyond the band reaches [`LARGE_REMOVED_AREA_TEXELS`]; the decision
//! stores the garments kept, and Dynamapper takes off every other garment of those slots the client draws beneath the
//! item.
//!
//! Format 2 decided a slot at a time, after its largest garment (the most opaque texels in its frame for the body), so
//! some garments were drawn otherwise than alone they would be: on the EC 2024 data, of the 61,120 garments drawn
//! beneath the dress-layer items, it took off 458 in plain view (mostly the amazon armour's shoulders) and kept 2,287
//! whose edges alone show. Deciding each garment draws every one of them as measured; the summary still counts what the
//! slot rule would do, and the report names each garment it would draw otherwise. The client's own rules settle the
//! garments it leaves out under the item or draws over it, so they are not measured; a garment drawn beneath the item
//! only in another outfit, with its draw order changed by the other items, is taken off too.

use std::collections::HashMap;

use color_eyre::eyre::{self, WrapErr};
use rayon::prelude::*;
use udd_assets::paperdoll_ec::{layer_mask, PaperdollEcDressDecision, PaperdollEcDressDecisions, PaperdollEcKeptGarment};
use uocf::enhanced::animationframe::FrameEntry;
use uocf::enhanced::hues::EcHueTables;
use uocf::enhanced::paperdoll::{
    alpha_blend_at, body_frame, item_frame, item_layer, plan, slot_layer, FrameRef, FrameSets, PaperdollDefinitions,
    PlacedFrame, WearableRules, Wearer, WornItem, LAYER_COUNT, SLOT_NAMES,
};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tileart::TileArtEntry;

/// The wearable rules the decisions are measured with: Dynamapper's bundled copy of the client's `wearables.xml`. A
/// user's edited copy changes the draw order Dynamapper's paperdoll applies them to, not the decisions.
pub const BUNDLED_WEARABLE_RULES: &str =
    include_str!("../../../../dynamapper/assets/runtime_specs/paperdoll/WearableRules.kdl");
/// The bodies the decisions are measured on, with the names the log gives them: the player races, male and female.
pub const WEARER_BODIES: [(&str, u32); 6] = [
    ("human male", 400),
    ("human female", 401),
    ("elf male", 605),
    ("elf female", 606),
    ("gargoyle male", 666),
    ("gargoyle female", 667),
];
/// The slot of the items measured.
pub const DRESS_SLOT: &str = "dress";
/// The slots a dress-layer item may take off: what the client draws beneath a robe.
pub const HIDEABLE_SLOTS: [&str; 6] = ["torso", "legs", "skirt", "arms", "chest", "abovechest"];
/// How far from the dress-layer item, in texels across or down (the larger), a vanished texel still counts as an edge
/// showing past it. At 16 the robes' edges fall inside it: on the cloth and armour outfits measured when the rule was
/// set (docs/agent_notes/ec_paperdoll_robe_override.md), what vanishes beyond it is at most 911 texels for a robe, and
/// at least 3,033 for a dress-layer item that leaves a garment in view.
pub const EDGE_BAND_TEXELS: usize = 16;
/// Texels that may vanish beyond the edge band for the slot to be taken off: about a 40 x 40 patch at full size, 28 x
/// 28 points in the window, between the two groups above.
pub const LARGE_REMOVED_AREA_TEXELS: usize = 1600;
/// Everything is measured unhued: a hue changes colours, not where a frame is opaque.
const NO_HUE: u16 = 0;
/// A tileart record's `string_dict_off` is one past the index of its name in `string_dictionary.uop` (EC 2024 data).
const TILEART_NAME_STRING_SHIFT: usize = 1;
const RGBA_BYTES: usize = 4;
const ALPHA_BYTE: usize = 3;
const OPAQUE: u8 = u8::MAX;
/// Digits between the separators of a logged count.
const DIGIT_GROUP: usize = 3;
/// Kept garments a decision's log line names; it counts the rest.
const LOGGED_KEPT_GARMENTS: usize = 8;

/// The name of a `tileart.uop` record in `string_dictionary.uop`.
pub fn tileart_name(record: &TileArtEntry, strings: &UoStringDictionary) -> Option<String> {
    let index = (record.string_dict_off as usize).checked_sub(TILEART_NAME_STRING_SHIFT)?;
    strings.get_string(index).filter(|name| !name.is_empty()).map(str::to_string)
}

/// The largest garment of a slot on one body: the one format 2 decided the slot after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LargestGarment {
    pub body: u32,
    pub layer: usize,
    pub item_id: u32,
    /// Opaque texels of its frame for the body.
    pub opaque: usize,
    /// Garments of the slot with a frame for the body.
    pub garments: usize,
}

/// Texels that vanish without one garment drawn beneath a dress-layer item, where the item is not opaque: within
/// the edge band of the item, and beyond it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GarmentMeasure {
    pub item_id: u32,
    pub near_edge: usize,
    pub beyond_edge: usize,
}

impl GarmentMeasure {
    /// Whether the garment is in plain view beyond the item's edges: the item keeps it.
    pub fn shows(&self) -> bool {
        self.beyond_edge >= LARGE_REMOVED_AREA_TEXELS
    }
}

/// What one slot measured under a dress-layer item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotMeasure {
    /// No item of the slot has a frame for the body: nothing to take off.
    NoGarment,
    /// The client draws no garment of the slot beneath the item and leaves out one at least.
    HiddenByClient,
    /// The client draws every garment of the slot over the item, or leaves the item out with them.
    NotBeneath,
    /// Every garment of the slot the client draws beneath the item, largest first.
    Measured(Vec<GarmentMeasure>),
}

impl SlotMeasure {
    /// The garments measured: none unless [`Self::Measured`].
    pub fn garments(&self) -> &[GarmentMeasure] {
        match self {
            Self::Measured(garments) => garments,
            _ => &[],
        }
    }

    /// The garments that show beyond the edge band: those the item keeps.
    pub fn shown(&self) -> impl Iterator<Item = &GarmentMeasure> {
        self.garments().iter().filter(|garment| garment.shows())
    }

    /// Whether format 2's slot rule takes the slot off: its largest garment does not show beyond the edge band, or the
    /// client leaves its garments out beneath the item.
    pub fn slot_rule_hides(&self) -> bool {
        match self {
            Self::NoGarment | Self::NotBeneath => false,
            Self::HiddenByClient => true,
            Self::Measured(garments) => garments.first().is_none_or(|largest| !largest.shows()),
        }
    }
}

/// What one dress-layer item takes off on one body, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DressDecision {
    pub item_id: u32,
    pub name: Option<String>,
    pub body_name: &'static str,
    pub body: u32,
    /// Per [`HIDEABLE_SLOTS`] entry, its layer and what it measured.
    pub slots: Vec<(usize, SlotMeasure)>,
}

impl DressDecision {
    /// The decision to store: the garments kept, ascending by id.
    pub fn record(&self) -> eyre::Result<PaperdollEcDressDecision> {
        let mut kept = self
            .kept()
            .map(|garment| {
                let beyond_edge = u32::try_from(garment.beyond_edge)
                    .wrap_err_with(|| format!("garment {} beyond the edge", garment.item_id))?;
                Ok(PaperdollEcKeptGarment { item_id: garment.item_id, beyond_edge })
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        kept.sort_by_key(|garment| garment.item_id);
        Ok(PaperdollEcDressDecision {
            item_id: self.item_id,
            body: u16::try_from(self.body).wrap_err_with(|| format!("body {}", self.body))?,
            kept,
        })
    }

    /// Every garment measured beneath the item.
    pub fn measured(&self) -> impl Iterator<Item = &GarmentMeasure> + '_ {
        self.slots.iter().flat_map(|(_, measure)| measure.garments())
    }

    /// The garments the item keeps: those that show beyond the edge band.
    pub fn kept(&self) -> impl Iterator<Item = &GarmentMeasure> + '_ {
        self.slots.iter().flat_map(|(_, measure)| measure.shown())
    }

    /// The decision in one line: `0x279C Hakama-Shita, human male: takes off 31 of the 33 garments drawn beneath it;
    /// keeps 0x1539 Long Pants (20,104 texels beyond the edge), ...`, naming the first [`LOGGED_KEPT_GARMENTS`] kept,
    /// the most in view first, and counting the rest. `name` names a garment.
    pub fn describe(&self, name: impl Fn(u32) -> Option<String>) -> String {
        let measured = self.measured().count();
        let mut kept: Vec<&GarmentMeasure> = self.kept().collect();
        kept.sort_by_key(|garment| (std::cmp::Reverse(garment.beyond_edge), garment.item_id));
        let noun = if measured == 1 { "garment" } else { "garments" };
        let mut line = format!(
            "{}, {}: takes off {} of the {measured} {noun} drawn beneath it",
            item_label(self.item_id, self.name.as_deref()),
            self.body_name,
            measured - kept.len()
        );
        if kept.is_empty() {
            line.push_str("; keeps none");
            return line;
        }
        let named: Vec<String> = kept
            .iter()
            .take(LOGGED_KEPT_GARMENTS)
            .map(|garment| {
                let label = item_label(garment.item_id, name(garment.item_id).as_deref());
                format!("{label} ({} texels beyond the edge)", grouped(garment.beyond_edge))
            })
            .collect();
        line = format!("{line}; keeps {}", named.join(", "));
        if kept.len() > LOGGED_KEPT_GARMENTS {
            line = format!("{line} and {} more", kept.len() - LOGGED_KEPT_GARMENTS);
        }
        line
    }
}

/// An item in a log line: `0x279C Hakama-Shita`.
pub fn item_label(item_id: u32, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("0x{item_id:04X} {name}"),
        None => format!("0x{item_id:04X}"),
    }
}

/// A count with thousands separators: `20,104`.
fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / DIGIT_GROUP);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % DIGIT_GROUP == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Everything the measurement found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DressMeasurement {
    /// The dress-layer items, ascending.
    pub dress_items: Vec<u32>,
    /// One per dress-layer item and body it has a frame for, ascending by (item, body order of [`WEARER_BODIES`]).
    pub decisions: Vec<DressDecision>,
    /// Per body measured and slot, the largest garment: what format 2's slot rule followed.
    pub largest: Vec<LargestGarment>,
    /// (item, body name) of the dress-layer items with no frame for a body.
    pub without_frame: Vec<(u32, &'static str)>,
    /// The bodies left out, and why.
    pub skipped_bodies: Vec<String>,
}

/// Counts of [`DressMeasurement::decisions`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DressCounts {
    /// Decisions keeping none of the garments measured beneath the item, some, and all of them.
    pub keep_none: usize,
    pub keep_some: usize,
    pub keep_all: usize,
    /// Decisions with no garment measured: the client draws none beneath the item.
    pub nothing_measured: usize,
    /// Garments measured beneath the items, one per item, body and garment.
    pub garments_measured: usize,
    /// Of those, the garments kept: in plain view beyond the edge band. The others are taken off.
    pub kept: usize,
    /// Of those, the garments in plain view that format 2's slot rule takes off with their slot.
    pub slot_rule_taken_off_in_view: usize,
    /// Of those, the garments whose edges alone show that format 2's slot rule keeps with their slot.
    pub slot_rule_kept_edges_only: usize,
}

impl DressMeasurement {
    /// The decisions to store: the slots they take garments off and the decisions ascending by (item id, body).
    pub fn records(&self) -> eyre::Result<PaperdollEcDressDecisions> {
        let slots = HIDEABLE_SLOTS.iter().map(|slot| slot_layer(slot)).collect::<eyre::Result<Vec<_>>>()?;
        let mut decisions = self.decisions.iter().map(DressDecision::record).collect::<eyre::Result<Vec<_>>>()?;
        decisions.sort_by_key(|decision| (decision.item_id, decision.body));
        Ok(PaperdollEcDressDecisions { slots: layer_mask(slots), decisions })
    }

    pub fn counts(&self) -> DressCounts {
        let mut counts = DressCounts::default();
        for decision in &self.decisions {
            let measured = decision.measured().count();
            let kept = decision.kept().count();
            match (measured, kept) {
                (0, _) => counts.nothing_measured += 1,
                (_, 0) => counts.keep_none += 1,
                _ if kept == measured => counts.keep_all += 1,
                _ => counts.keep_some += 1,
            }
            counts.garments_measured += measured;
            counts.kept += kept;
            for (_, measure) in &decision.slots {
                let shown = measure.shown().count();
                if measure.slot_rule_hides() {
                    counts.slot_rule_taken_off_in_view += shown;
                } else {
                    counts.slot_rule_kept_edges_only += measure.garments().len() - shown;
                }
            }
        }
        counts
    }

    /// The summary line: items, bodies, decisions and garments, and what format 2's slot rule would do.
    pub fn summary(&self) -> String {
        let counts = self.counts();
        format!(
            "{} dress-layer items measured on {} bodies: {} decisions keep none of the garments drawn beneath the \
             item, {} some, {} all, {} measure none; {} item and body pairs without a frame; of the {} garments drawn \
             beneath them, {} show beyond the edge and are kept, {} are taken off; the slot rule of format 2 (the \
             largest garment decides its slot) would take off {} in plain view and keep {} whose edges alone show",
            self.dress_items.len(),
            WEARER_BODIES.len() - self.skipped_bodies.len(),
            counts.keep_none,
            counts.keep_some,
            counts.keep_all,
            counts.nothing_measured,
            self.without_frame.len(),
            grouped(counts.garments_measured),
            grouped(counts.kept),
            grouped(counts.garments_measured - counts.kept),
            grouped(counts.slot_rule_taken_off_in_view),
            grouped(counts.slot_rule_kept_edges_only)
        )
    }

    /// Every finding, one line each, `info: ...` or `warning: ...` as the Enhanced effects report: the summary, each
    /// decision, each slot not measured and each garment measured.
    pub fn report_lines(&self, name: impl Fn(u32) -> Option<String>) -> Vec<String> {
        let label = |item_id: u32| item_label(item_id, name(item_id).as_deref());
        let body_name =
            |body: u32| WEARER_BODIES.iter().find(|&&(_, number)| number == body).map_or("?", |&(name, _)| name);
        let mut lines = vec![format!("info: {}", self.summary())];
        lines.extend(self.skipped_bodies.iter().map(|skipped| format!("warning: body {skipped} is not measured")));
        for largest in &self.largest {
            lines.push(format!(
                "info: largest {} garment on {}: {} ({} opaque texels, of {} garments with a frame)",
                SLOT_NAMES[largest.layer],
                body_name(largest.body),
                label(largest.item_id),
                grouped(largest.opaque),
                largest.garments
            ));
        }
        for decision in &self.decisions {
            lines.push(format!("info: {}", decision.describe(&name)));
            let pair = format!("{}, {}", item_label(decision.item_id, decision.name.as_deref()), decision.body_name);
            for (layer, measure) in &decision.slots {
                let slot = SLOT_NAMES[*layer];
                let garments = match measure {
                    SlotMeasure::NoGarment => {
                        lines.push(format!("info: {pair}, {slot}: no garment has a frame for this body"));
                        continue;
                    }
                    SlotMeasure::HiddenByClient => {
                        lines.push(format!("info: {pair}, {slot}: the client leaves its garments out beneath it"));
                        continue;
                    }
                    SlotMeasure::NotBeneath => {
                        lines.push(format!("info: {pair}, {slot}: the client draws its garments over it"));
                        continue;
                    }
                    SlotMeasure::Measured(garments) => garments,
                };
                for garment in garments {
                    let verdict = if garment.shows() { "kept" } else { "taken off" };
                    // Format 2 decided the slot after its largest garment, and drew this one otherwise.
                    let slot_rule = match (garment.shows(), measure.slot_rule_hides()) {
                        (true, true) => "; the slot rule took it off, in plain view",
                        (false, false) => "; the slot rule kept it, its edges alone showing",
                        _ => "",
                    };
                    lines.push(format!(
                        "info: {pair}, {slot}: {}: {verdict}: {} texels vanish near the edge, {} beyond{slot_rule}",
                        label(garment.item_id),
                        grouped(garment.near_edge),
                        grouped(garment.beyond_edge)
                    ));
                }
            }
        }
        for &(item_id, body) in &self.without_frame {
            lines.push(format!("info: {}, {body}: no frame, nothing to measure", label(item_id)));
        }
        lines
    }
}

/// A frame's place in the frame cache: its set and number.
type FrameKey = (usize, u16);

fn frame_key(frame: FrameRef<FrameEntry>) -> FrameKey {
    (frame.set, frame.entry.frame_number)
}

/// The frames the measurement draws, decoded unhued once.
struct FrameCache<'s> {
    sets: &'s FrameSets,
    hues: EcHueTables,
    frames: HashMap<FrameKey, PlacedFrame>,
}

impl FrameCache<'_> {
    fn decode(&mut self, frame: FrameRef<FrameEntry>) -> eyre::Result<&PlacedFrame> {
        let key = frame_key(frame);
        if !self.frames.contains_key(&key) {
            let decoded = self
                .sets
                .decode(frame, NO_HUE, &self.hues)
                .wrap_err_with(|| format!("frame {} of set {}", key.1, key.0))?;
            self.frames.insert(key, decoded);
        }
        Ok(&self.frames[&key])
    }

    fn get(&self, frame: FrameRef<FrameEntry>) -> eyre::Result<&PlacedFrame> {
        let key = frame_key(frame);
        self.frames.get(&key).ok_or_else(|| eyre::eyre!("frame {} of set {} was not decoded", key.1, key.0))
    }
}

fn opaque_texels(frame: &PlacedFrame) -> usize {
    frame.rgba.chunks_exact(RGBA_BYTES).filter(|texel| texel[ALPHA_BYTE] == OPAQUE).count()
}

/// A canvas around frames placed from the anchor.
struct Canvas {
    x0: i32,
    y0: i32,
    width: usize,
    height: usize,
}

impl Canvas {
    fn around<'f>(frames: impl IntoIterator<Item = &'f PlacedFrame>) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for frame in frames {
            x0 = x0.min(frame.x);
            y0 = y0.min(frame.y);
            x1 = x1.max(frame.x + frame.width as i32);
            y1 = y1.max(frame.y + frame.height as i32);
        }
        if x0 >= x1 || y0 >= y1 {
            return Self { x0: 0, y0: 0, width: 0, height: 0 };
        }
        Self { x0, y0, width: (x1 - x0) as usize, height: (y1 - y0) as usize }
    }

    fn of(frame: &PlacedFrame) -> Self {
        Self { x0: frame.x, y0: frame.y, width: frame.width as usize, height: frame.height as usize }
    }

    /// The frames drawn in order, cut to the canvas: RGBA.
    fn compose(&self, frames: &[&PlacedFrame]) -> Vec<u8> {
        let mut canvas = vec![0u8; self.width * self.height * RGBA_BYTES];
        for frame in frames {
            // The rows and columns of the frame on the canvas; `alpha_blend_at` draws one row at a time.
            let left = (self.x0 - frame.x).max(0) as usize;
            let right = (self.x0 + self.width as i32 - frame.x).clamp(0, frame.width as i32) as usize;
            let top = (self.y0 - frame.y).max(0) as usize;
            let bottom = (self.y0 + self.height as i32 - frame.y).clamp(0, frame.height as i32) as usize;
            if left >= right {
                continue;
            }
            for row in top..bottom {
                let start = (row * frame.width as usize + left) * RGBA_BYTES;
                alpha_blend_at(
                    &mut canvas,
                    self.width as u32,
                    frame.x + left as i32 - self.x0,
                    frame.y + row as i32 - self.y0,
                    (right - left) as u32,
                    1,
                    &frame.rgba[start..start + (right - left) * RGBA_BYTES],
                );
            }
        }
        canvas
    }

    /// The index on this canvas of texel `index` of `window`, which lies within it.
    fn index_of(&self, window: &Canvas, index: usize) -> usize {
        let x = (window.x0 - self.x0) as usize + index % window.width;
        let y = (window.y0 - self.y0) as usize + index / window.width;
        y * self.width + x
    }
}

/// Whether each texel is within `radius` texels (across or down, the larger) of a texel `rgba` touches: the touched
/// texels dilated by a square, one prefix sum per row, then per column.
fn near_touched(rgba: &[u8], width: usize, height: usize, radius: usize) -> Vec<bool> {
    let touched: Vec<bool> = rgba.chunks_exact(RGBA_BYTES).map(|texel| texel[ALPHA_BYTE] != 0).collect();
    let mut across = vec![false; touched.len()];
    let mut sums = vec![0usize; width.max(height) + 1];
    for y in 0..height {
        for x in 0..width {
            sums[x + 1] = sums[x] + usize::from(touched[y * width + x]);
        }
        for x in 0..width {
            across[y * width + x] = sums[(x + radius + 1).min(width)] > sums[x.saturating_sub(radius)];
        }
    }
    let mut near = vec![false; touched.len()];
    for x in 0..width {
        for y in 0..height {
            sums[y + 1] = sums[y] + usize::from(across[y * width + x]);
        }
        for y in 0..height {
            near[y * width + x] = sums[(y + radius + 1).min(height)] > sums[y.saturating_sub(radius)];
        }
    }
    near
}

/// A garment with a frame for a body.
#[derive(Clone, Copy)]
struct Garment {
    item_id: u32,
    frame: FrameRef<FrameEntry>,
}

/// A body the decisions are measured on.
struct MeasuredBody<D> {
    name: &'static str,
    body: u32,
    wearer: Wearer<D>,
    /// Per [`HIDEABLE_SLOTS`] entry, its layer and its garments with a frame for the body, largest first.
    garments: Vec<(usize, Vec<Garment>)>,
}

/// Measures what each dress-layer item among `item_ids` takes off on each of [`WEARER_BODIES`] it has a frame for. An
/// item's slot is its `tileart.uop` Layer property: every dress-layer item of the client has one. `name` names an
/// item in the decisions.
pub fn measure_dress_decisions<'a, T>(
    sets: &FrameSets,
    definitions: T,
    rules: &WearableRules,
    item_ids: &[u32],
    name: impl Fn(u32) -> Option<String>,
) -> eyre::Result<DressMeasurement>
where
    T: PaperdollDefinitions<'a> + Send + Sync,
    T::Definition: Send + Sync,
{
    let dress_layer = slot_layer(DRESS_SLOT)?;
    let hideable_layers = HIDEABLE_SLOTS.iter().map(|slot| slot_layer(slot)).collect::<eyre::Result<Vec<_>>>()?;
    let mut by_layer: HashMap<usize, Vec<u32>> = HashMap::new();
    for &item_id in item_ids {
        if let Some(layer) = item_layer(definitions, item_id, None) {
            by_layer.entry(layer).or_default().push(item_id);
        }
    }
    for items in by_layer.values_mut() {
        items.sort_unstable();
        items.dedup();
    }
    let mut measurement =
        DressMeasurement { dress_items: by_layer.remove(&dress_layer).unwrap_or_default(), ..Default::default() };

    let mut cache = FrameCache { sets, hues: EcHueTables::default(), frames: HashMap::new() };
    let mut bodies = Vec::new();
    for (body_name, body) in WEARER_BODIES {
        let wearer = Wearer::new(definitions, body);
        match body_frame(sets, &wearer) {
            Some((frame, false)) => {
                cache.decode(frame)?;
            }
            Some((_, true)) => {
                measurement.skipped_bodies.push(format!("{body_name} ({body}): no body frame of its own"));
                continue;
            }
            None => {
                measurement.skipped_bodies.push(format!("{body_name} ({body}): no body frame"));
                continue;
            }
        }
        let mut garments = Vec::with_capacity(hideable_layers.len());
        for &layer in &hideable_layers {
            let mut sized = Vec::new();
            for &item_id in by_layer.get(&layer).map(Vec::as_slice).unwrap_or_default() {
                let Ok(frame) = item_frame(sets, definitions, &wearer, item_id) else { continue };
                sized.push((opaque_texels(cache.decode(frame)?), Garment { item_id, frame }));
            }
            // Largest first; the lower id first among equals.
            sized.sort_by_key(|&(opaque, garment)| (std::cmp::Reverse(opaque), garment.item_id));
            if let Some(&(opaque, largest)) = sized.first() {
                measurement.largest.push(LargestGarment {
                    body,
                    layer,
                    item_id: largest.item_id,
                    opaque,
                    garments: sized.len(),
                });
            }
            garments.push((layer, sized.into_iter().map(|(_, garment)| garment).collect()));
        }
        bodies.push(MeasuredBody { name: body_name, body, wearer, garments });
    }

    let mut pairs = Vec::new();
    for &item_id in &measurement.dress_items {
        for body in &bodies {
            match item_frame(sets, definitions, &body.wearer, item_id) {
                Ok(frame) => {
                    cache.decode(frame)?;
                    pairs.push((item_id, body));
                }
                Err(_) => measurement.without_frame.push((item_id, body.name)),
            }
        }
    }

    let cache = &cache;
    let slots = pairs
        .par_iter()
        .map(|&(item_id, body)| measure_pair(cache, definitions, rules, dress_layer, item_id, body))
        .collect::<eyre::Result<Vec<_>>>()?;
    measurement.decisions = slots
        .into_iter()
        .zip(&pairs)
        .map(|(slots, &(item_id, body))| DressDecision {
            item_id,
            name: name(item_id),
            body_name: body.name,
            body: body.body,
            slots,
        })
        .collect();
    Ok(measurement)
}

/// Where the client draws a garment worn with a dress-layer item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    Beneath,
    Over,
    Hidden,
}

/// What each hideable slot measures under `dress` on `body`.
fn measure_pair<'a, T: PaperdollDefinitions<'a>>(
    cache: &FrameCache<'_>,
    definitions: T,
    rules: &WearableRules,
    dress_layer: usize,
    dress: u32,
    body: &MeasuredBody<T::Definition>,
) -> eyre::Result<Vec<(usize, SlotMeasure)>> {
    let placement = |layer: usize, garment: u32| {
        let mut worn = [None; LAYER_COUNT];
        worn[layer] = Some(garment);
        worn[dress_layer] = Some(dress);
        let order = rules.paperdoll_order(&worn, body.wearer.race);
        match (order.iter().position(|&drawn| drawn == layer), order.iter().position(|&drawn| drawn == dress_layer)) {
            (None, _) => Placement::Hidden,
            (Some(garment), Some(dress)) if garment < dress => Placement::Beneath,
            _ => Placement::Over,
        }
    };
    let mut measured = Vec::with_capacity(body.garments.len());
    for (layer, garments) in &body.garments {
        let placements: Vec<Placement> = garments.iter().map(|garment| placement(*layer, garment.item_id)).collect();
        let beneath: Vec<Garment> = garments
            .iter()
            .zip(&placements)
            .filter(|&(_, &placement)| placement == Placement::Beneath)
            .map(|(garment, _)| *garment)
            .collect();
        measured.push(match beneath.is_empty() {
            false => Ok(beneath),
            true if garments.is_empty() => Err(SlotMeasure::NoGarment),
            true if placements.contains(&Placement::Hidden) => Err(SlotMeasure::HiddenByClient),
            true => Err(SlotMeasure::NotBeneath),
        });
    }

    let worn = |items: &[u32]| -> Vec<WornItem> {
        items.iter().map(|&item_id| WornItem { item_id, hue: NO_HUE, tiledata_layer: None }).collect()
    };
    let draw = |items: &[u32]| -> eyre::Result<Vec<&PlacedFrame>> {
        let planned = plan(cache.sets, definitions, rules, body.body, NO_HUE, &worn(items))?;
        planned.frames.iter().map(|frame| cache.get(frame.frame)).collect()
    };
    let dress_frame = cache.get(
        item_frame(cache.sets, definitions, &body.wearer, dress)
            .map_err(|miss| eyre::eyre!("item {dress} on body {}: {}", body.body, miss.describe()))?,
    )?;
    let alone = draw(&[dress])?;
    let mut extent: Vec<&PlacedFrame> = alone.clone();
    for garments in measured.iter().filter_map(|measure| measure.as_ref().ok()) {
        for garment in garments {
            extent.push(cache.get(garment.frame)?);
        }
    }
    let canvas = Canvas::around(extent);
    let after = canvas.compose(&alone);
    let dress_alone = canvas.compose(&[dress_frame]);
    let near = near_touched(&dress_alone, canvas.width, canvas.height, EDGE_BAND_TEXELS);

    // Only the garment's texels differ with it worn: the client draws the body, and the item over the garment.
    let measure_garment = |garment: &Garment| -> eyre::Result<GarmentMeasure> {
        let window = Canvas::of(cache.get(garment.frame)?);
        let before = window.compose(&draw(&[garment.item_id, dress])?);
        let (mut near_edge, mut beyond_edge) = (0, 0);
        for (window_index, before) in before.chunks_exact(RGBA_BYTES).enumerate() {
            let index = canvas.index_of(&window, window_index);
            let texel = index * RGBA_BYTES;
            if before == &after[texel..texel + RGBA_BYTES] || dress_alone[texel + ALPHA_BYTE] == OPAQUE {
                continue;
            }
            if near[index] {
                near_edge += 1;
            } else {
                beyond_edge += 1;
            }
        }
        Ok(GarmentMeasure { item_id: garment.item_id, near_edge, beyond_edge })
    };
    let mut slots = Vec::with_capacity(measured.len());
    for ((layer, _), measure) in body.garments.iter().zip(measured) {
        let measure = match measure {
            Ok(garments) => {
                SlotMeasure::Measured(garments.iter().map(measure_garment).collect::<eyre::Result<Vec<_>>>()?)
            }
            Err(measure) => measure,
        };
        slots.push((*layer, measure));
    }
    Ok(slots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::animationframe::{AmouFile, AnimationFrame, IndexedFrame, COVERAGE_SOLID};
    use uocf::enhanced::paperdoll::{SortedTileArt, MALE_SET};
    use uocf::enhanced::tileart::{PropertyKey, TaeProp};

    const BODY: u16 = 400;
    /// A dress-layer item no wearable rule names.
    const DRESS: u16 = 7940;
    const SHIRT: u16 = 5399;
    /// A shirt smaller than [`SHIRT`], all of it under the item: measured after it.
    const SMALL_SHIRT: u16 = 5397;
    /// A torso garment smaller than [`SMALL_SHIRT`], all of it below the edge band: in plain view, so kept, where the
    /// slot rule of format 2 took it off with [`SHIRT`].
    const LOW_TUNIC: u16 = 5401;
    const LOW_TUNIC_HEIGHT: u16 = 45;
    const PANTS: u16 = 5433;
    /// Worn above the chest: the client leaves it out under a dress-layer item no rule names.
    const MANTLE: u16 = 7933;
    const PALETTE: [[u8; RGBA_BYTES]; 2] = [[0, 0, 0, 0], [40, 80, 120, 0]];
    const SOLID_INDEX: u8 = 1;
    const DRESS_WIDTH: u16 = 40;
    const DRESS_HEIGHT: u16 = 60;
    /// The shirt reaches this far past each side of the item: within the edge band.
    const SHIRT_MARGIN: u16 = 4;
    const SHIRT_HEIGHT: u16 = 50;
    /// The trousers reach this far below the item: far past the edge band.
    const PANTS_BELOW: u16 = 100;
    const BODY_SIZE: u16 = 8;

    /// A solid `width` x `height` frame whose pixel (0, 0) goes at `(x, y)` from the anchor.
    fn solid(width: u16, height: u16, x: i16, y: i16) -> IndexedFrame {
        let pixels = usize::from(width) * usize::from(height);
        IndexedFrame {
            width,
            height,
            center_x: -x,
            center_y: -y,
            indices: vec![SOLID_INDEX; pixels],
            coverage: vec![COVERAGE_SOLID; pixels],
        }
    }

    fn frame_sets() -> FrameSets {
        let action = MALE_SET as u16;
        let shirt_width = DRESS_WIDTH + 2 * SHIRT_MARGIN;
        let male = AmouFile::from_indexed_frames(
            u32::from(BODY),
            PALETTE.to_vec(),
            &[
                (action, BODY, solid(BODY_SIZE, BODY_SIZE, 0, 0)),
                (action, DRESS, solid(DRESS_WIDTH, DRESS_HEIGHT, 0, 0)),
                (action, SHIRT, solid(shirt_width, SHIRT_HEIGHT, -(SHIRT_MARGIN as i16), 0)),
                (action, SMALL_SHIRT, solid(DRESS_WIDTH, SHIRT_HEIGHT, 0, 0)),
                (action, LOW_TUNIC, solid(DRESS_WIDTH, LOW_TUNIC_HEIGHT, 0, (DRESS_HEIGHT as usize + EDGE_BAND_TEXELS) as i16)),
                (action, PANTS, solid(DRESS_WIDTH, DRESS_HEIGHT + PANTS_BELOW, 0, 0)),
                (action, MANTLE, solid(DRESS_WIDTH, DRESS_HEIGHT + PANTS_BELOW, 0, 0)),
            ],
        )
        .unwrap();
        let empty = AmouFile::from_indexed_frames(u32::from(BODY), PALETTE.to_vec(), &[]).unwrap();
        let load = |file: AmouFile| AnimationFrame::load(&file.to_bytes().unwrap()).unwrap();
        FrameSets::from_sets([load(male), load(empty)])
    }

    fn records() -> Vec<TileArtEntry> {
        let worn = |item: u16, slot: &str| TileArtEntry {
            tile_id: u32::from(item),
            prop_vector1: vec![TaeProp { id: PropertyKey::Layer as u8, val: slot_layer(slot).unwrap() as u32 }],
            ..TileArtEntry::default()
        };
        let mut records = vec![
            worn(DRESS, DRESS_SLOT),
            worn(SHIRT, "torso"),
            worn(SMALL_SHIRT, "torso"),
            worn(LOW_TUNIC, "torso"),
            worn(PANTS, "legs"),
            worn(MANTLE, "abovechest"),
        ];
        records.sort_by_key(|record| record.tile_id);
        records
    }

    fn rules() -> WearableRules {
        WearableRules::decode(BUNDLED_WEARABLE_RULES).unwrap()
    }

    /// The item takes off the shirts, whose edges show only beside it, and keeps the trousers, which reach far below
    /// it, and the low tunic, which the slot rule of format 2 took off with the shirts. Every garment drawn beneath
    /// the item is measured, the largest of its slot first; the mantle, which the client leaves out under the item,
    /// is not; the bodies without a frame of their own are left out.
    #[test]
    fn a_dress_takes_off_each_garment_that_shows_only_at_its_edges() {
        let sets = frame_sets();
        let records = records();
        let ids: Vec<u32> = records.iter().map(|record| record.tile_id).collect();
        let name = |item: u32| (item == u32::from(DRESS)).then(|| "robe".to_string());
        let measurement = measure_dress_decisions(&sets, SortedTileArt(&records), &rules(), &ids, name).unwrap();
        assert_eq!(measurement.dress_items, [u32::from(DRESS)]);
        assert_eq!(measurement.skipped_bodies.len(), WEARER_BODIES.len() - 1, "{:?}", measurement.skipped_bodies);
        let torso = slot_layer("torso").unwrap();
        let legs = slot_layer("legs").unwrap();
        let abovechest = slot_layer("abovechest").unwrap();
        let largest = |layer| measurement.largest.iter().find(|largest| largest.layer == layer).unwrap();
        assert_eq!((largest(torso).item_id, largest(torso).garments), (u32::from(SHIRT), 3));
        assert_eq!(largest(legs).item_id, u32::from(PANTS));

        let [decision] = measurement.decisions.as_slice() else { panic!("{:?}", measurement.decisions) };
        let measure = |layer| &decision.slots.iter().find(|&&(slot, _)| slot == layer).unwrap().1;
        let shirt_edges = 2 * usize::from(SHIRT_MARGIN) * usize::from(SHIRT_HEIGHT);
        let low_tunic = usize::from(DRESS_WIDTH) * usize::from(LOW_TUNIC_HEIGHT);
        let garment = |item: u16, near_edge, beyond_edge| GarmentMeasure { item_id: u32::from(item), near_edge, beyond_edge };
        assert_eq!(
            measure(torso),
            &SlotMeasure::Measured(vec![
                garment(SHIRT, shirt_edges, 0),
                garment(SMALL_SHIRT, 0, 0),
                garment(LOW_TUNIC, 0, low_tunic)
            ])
        );
        let below_band = (usize::from(PANTS_BELOW) - EDGE_BAND_TEXELS) * usize::from(DRESS_WIDTH);
        assert_eq!(
            measure(legs),
            &SlotMeasure::Measured(vec![garment(PANTS, EDGE_BAND_TEXELS * usize::from(DRESS_WIDTH), below_band)])
        );
        assert_eq!(measure(abovechest), &SlotMeasure::HiddenByClient);
        assert_eq!(measure(slot_layer("skirt").unwrap()), &SlotMeasure::NoGarment);
        assert!(measure(torso).slot_rule_hides() && !measure(legs).slot_rule_hides());

        let record = decision.record().unwrap();
        let kept = |item: u16, beyond_edge: usize| PaperdollEcKeptGarment {
            item_id: u32::from(item),
            beyond_edge: beyond_edge as u32,
        };
        assert_eq!(record.kept, [kept(LOW_TUNIC, low_tunic), kept(PANTS, below_band)]);
        let stored = measurement.records().unwrap();
        assert_eq!(stored.decisions, [record]);
        let body = u32::from(BODY);
        assert!(stored.takes_off(u32::from(DRESS), body, torso, u32::from(SHIRT)));
        assert!(!stored.takes_off(u32::from(DRESS), body, torso, u32::from(LOW_TUNIC)));
        assert!(!stored.takes_off(u32::from(DRESS), body, legs, u32::from(PANTS)));
        assert!(stored.takes_off(u32::from(DRESS), body, abovechest, u32::from(MANTLE)));
        assert_eq!(
            decision.describe(name),
            format!(
                "0x1F04 robe, human male: takes off 2 of the 4 garments drawn beneath it; keeps 0x1539 ({} texels \
                 beyond the edge), 0x1519 ({} texels beyond the edge)",
                grouped(below_band),
                grouped(low_tunic)
            )
        );
        assert_eq!(
            measurement.counts(),
            DressCounts {
                keep_none: 0,
                keep_some: 1,
                keep_all: 0,
                nothing_measured: 0,
                garments_measured: 4,
                kept: 2,
                slot_rule_taken_off_in_view: 1,
                slot_rule_kept_edges_only: 0
            }
        );
        let report = measurement.report_lines(name);
        assert!(report.iter().any(|line| line == &format!("info: {}", decision.describe(name))), "{report:#?}");
        for line in [
            format!(
                "info: 0x1F04 robe, human male, torso: 0x1517: taken off: {} texels vanish near the edge, 0 beyond",
                grouped(shirt_edges)
            ),
            format!(
                "info: 0x1F04 robe, human male, torso: 0x1519: kept: 0 texels vanish near the edge, {} beyond; the \
                 slot rule took it off, in plain view",
                grouped(low_tunic)
            ),
            "info: 0x1F04 robe, human male, abovechest: the client leaves its garments out beneath it".to_string(),
        ] {
            assert!(report.contains(&line), "{line}\n{report:#?}");
        }
    }

    /// A decision's log line names the garments kept most in view first, up to [`LOGGED_KEPT_GARMENTS`], and counts
    /// the rest.
    #[test]
    fn a_long_list_of_kept_garments_is_cut_short() {
        const MORE: usize = 2;
        const FIRST_GARMENT: u32 = 0x1000;
        let kept = LOGGED_KEPT_GARMENTS + MORE;
        let garments = (0..kept)
            .map(|index| GarmentMeasure {
                item_id: FIRST_GARMENT + index as u32,
                near_edge: 0,
                beyond_edge: LARGE_REMOVED_AREA_TEXELS + index,
            })
            .collect();
        let decision = DressDecision {
            item_id: u32::from(DRESS),
            name: None,
            body_name: WEARER_BODIES[0].0,
            body: WEARER_BODIES[0].1,
            slots: vec![(slot_layer("torso").unwrap(), SlotMeasure::Measured(garments))],
        };
        let line = decision.describe(|_| None);
        assert!(line.starts_with(&format!("0x1F04, human male: takes off 0 of the {kept} garments drawn beneath it; keeps 0x{:04X} (", FIRST_GARMENT + kept as u32 - 1)), "{line}");
        assert!(line.ends_with(&format!(" and {MORE} more")), "{line}");
        assert_eq!(line.matches("texels beyond the edge").count(), LOGGED_KEPT_GARMENTS, "{line}");
    }

    #[test]
    fn counts_are_grouped_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(911), "911");
        assert_eq!(grouped(20_104), "20,104");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }
}
