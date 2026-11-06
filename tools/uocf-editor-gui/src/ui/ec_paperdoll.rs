//! The Enhanced Client's paperdoll images: the body and worn-item frames of `Paperdoll.uop`, found and ordered as
//! `ECPaperdollTexture::Build` (UOSA 0x0050b340) does. The rule and its addresses are in
//! `scripts/dev-helpers/ec-research/ghidra-paperdoll/results/uosa_paperdoll.md`.

use std::collections::HashMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use knuffel::Decode;
use uocf::enhanced::animationframe::{AnimationFrame, FrameEntry};
use uocf::enhanced::hues::{hue_bitmap_hash, EcHueLookup, EcHueTable, EcHueTables, ShadowPaletteStats};
use uocf::enhanced::mobile_body::{DEFAULT_GENDER, DEFAULT_RACE, MOBILE_DEFINITION_BASE};
use uocf::enhanced::tileart::{PropertyKey, TaeAppearanceFilter, TileArtEntry};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::{LoadMode, UopPackage};

use crate::app::TileArtFileEntry;

pub(crate) const PAPERDOLL_UOP_NAME: &str = "Paperdoll.uop";
/// The two frame sets, the male (asset 0x40) and the female (asset 0x41); every frame of a set has the set's
/// index as its action.
const FRAME_SET_PATHS: [&str; GENDER_SET_COUNT] = ["build/paperdoll/000001/00.bin", "build/paperdoll/000001/01.bin"];
const GENDER_SET_COUNT: usize = 2;
/// The male set: gender 0 picks it, any other gender the female one.
pub(crate) const MALE_SET: usize = 0;
pub(crate) const FEMALE_SET: usize = 1;
/// `FindPaperdollFrame` (0x0050b780) falls back to this frame of the male set, the human male body.
const FALLBACK_BODY_FRAME: u32 = 400;
/// `FindLayerFrame` (0x0050b850) follows an `Appearance` chain this many levels deep.
const MAX_APPEARANCE_DEPTH: u32 = 5;
/// Equipment layers of a mobile.
pub(crate) const LAYER_COUNT: usize = 26;
/// The slot names of `wearables.xml`, indexed by layer: the string table at UOSA 0x00db9ba0.
pub(crate) const SLOT_NAMES: [&str; LAYER_COUNT] = [
    "drag", "righthand", "lefthand", "feet", "legs", "torso", "head", "hands", "finger1", "talisman", "neck", "hair",
    "waist", "chest", "lwrist", "face", "facialhair", "abovechest", "ears", "arms", "cape", "backpack", "dress",
    "skirt", "feetlegs", "riding",
];
/// Layers the paperdoll's layer list (0x0050b690) skips: layer 0, the backpack and the mount.
const SKIPPED_LAYERS: [usize; 3] = [0, 0x15, 0x19];
/// The draw order `BuildEquipmentLayerOrder` (0x00468810) builds for the paperdoll, direction 9.
const PAPERDOLL_DIRECTION: &str = "paperdoll";
/// `GetRace` values (props1 key 9; 1, human, when absent) of the race names of `wearables.xml`.
const RACES: [(&str, u32); 3] = [("human", 1), ("elf", 2), ("gargoyle", 3)];
/// The item list that matches any item.
const ANY_ITEM: &str = "any";
const WEARABLE_RULES_FILE_NAME: &str = "WearableRules.kdl";
/// `wearables.xml` of `MainMisc.uop`, as dynamapper reads it.
const WEARABLE_RULES_KDL: &str =
    include_str!("../../../../dynamapper/assets/runtime_specs/paperdoll/WearableRules.kdl");
const PRIORITY_PROPERTY: &str = "priority";
const ITEMS_PROPERTY: &str = "items";
/// `draworder "<direction>" { slot "<slot>" priority=N }` and
/// `wearable "<slot>" items="..." { override priority=N [race=] [direction=] { when "<slot>" items="..." } }`.
const WEARABLE_RULES_SCHEMA: &[uot_kdl::strict::NodeSchema] = &[
    uot_kdl::strict::NodeSchema {
        name: "draworder",
        properties: &[],
        children: &[uot_kdl::strict::NodeSchema::leaf("slot", &[PRIORITY_PROPERTY])],
    },
    uot_kdl::strict::NodeSchema {
        name: "wearable",
        properties: &[ITEMS_PROPERTY],
        children: &[uot_kdl::strict::NodeSchema {
            name: "override",
            properties: &[PRIORITY_PROPERTY, "race", "direction"],
            children: &[uot_kdl::strict::NodeSchema::leaf("when", &[ITEMS_PROPERTY])],
        }],
    },
];

/// The set of a gender: 0 the male one, any other the female one (`ECPaperdollTexture::Build`).
pub(crate) fn gender_set(gender: u32) -> usize {
    if gender == DEFAULT_GENDER { MALE_SET } else { FEMALE_SET }
}

/// A frame of a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameRef {
    pub set: usize,
    pub entry: FrameEntry,
}

/// A decoded frame and where it goes: its pixel (0, 0) is drawn at `(x, y)` from the common anchor.
pub(crate) struct PlacedFrame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The male and female frame sets of `Paperdoll.uop`.
pub(crate) struct FrameSets {
    sets: [AnimationFrame; GENDER_SET_COUNT],
}

impl FrameSets {
    pub(crate) fn load(path: &Path) -> eyre::Result<Self> {
        let package = UopPackage::load_with_mode(path, LoadMode::Lazy)
            .wrap_err_with(|| format!("cannot open {}", path.display()))?;
        let load = |name: &str| -> eyre::Result<AnimationFrame> {
            let payload = package
                .unpack_file_by_hash(hash_file_name_single(name))?
                .ok_or_else(|| eyre::eyre!("{} has no {name}", path.display()))?;
            AnimationFrame::load(&payload).wrap_err_with(|| format!("cannot read {name}"))
        };
        Ok(Self { sets: [load(FRAME_SET_PATHS[MALE_SET])?, load(FRAME_SET_PATHS[FEMALE_SET])?] })
    }

    #[cfg(test)]
    fn from_sets(sets: [AnimationFrame; GENDER_SET_COUNT]) -> Self {
        Self { sets }
    }

    /// `UOAnimations::FindFrameEntry` (0x00430f60): the frame (action, number), the action being the set's. It
    /// compares 32 bits, so a number past the 16-bit frame numbers finds nothing.
    pub(crate) fn find(&self, set: usize, number: u32) -> Option<FrameRef> {
        let number = u16::try_from(number).ok()?;
        let action = set as u16;
        let entry = self.sets[set].frames.iter().find(|entry| entry.action == action && entry.frame_number == number)?;
        Some(FrameRef { set, entry: *entry })
    }

    /// `FindPaperdollFrame` (0x0050b780): the body's frame in its gender's set, else frame 400 of the male set.
    /// The second value is whether the fallback was taken.
    pub(crate) fn body_frame(&self, wearer: &Wearer) -> Option<(FrameRef, bool)> {
        let body = strip_definition_base(wearer.body);
        match self.find(gender_set(wearer.gender), body) {
            Some(frame) => Some((frame, false)),
            None => self.find(MALE_SET, FALLBACK_BODY_FRAME).map(|frame| (frame, true)),
        }
    }

    /// Decodes a frame with a server hue: the hue's table recolours the palette's hueable colours, as
    /// `UOAnimations::BuildFramePalette` does.
    pub(crate) fn decode(&self, frame: FrameRef, hue: u16, hues: &EcHueTables) -> eyre::Result<PlacedFrame> {
        let set = &self.sets[frame.set];
        let mut decoded = set.decode_frame(&frame.entry)?;
        hues.apply(hue, &mut decoded.data, &decoded.hue_mask, &ShadowPaletteStats::from_palette(&set.colours));
        Ok(PlacedFrame {
            x: i32::from(frame.entry.init_coords_x),
            y: i32::from(frame.entry.init_coords_y),
            width: u32::from(decoded.width),
            height: u32::from(decoded.height),
            rgba: decoded.data,
        })
    }
}

/// `tileart.uop` records by tile id: `UOTileArt::GetObjectDefinition`.
#[derive(Clone, Copy)]
pub(crate) struct Definitions<'a>(pub &'a [TileArtFileEntry]);

impl<'a> Definitions<'a> {
    /// The record of `tile_id`; the entries are sorted by tile id.
    pub(crate) fn get(&self, tile_id: u32) -> Option<&'a TileArtEntry> {
        let index = self.0.binary_search_by_key(&tile_id, |file| file.entry.tile_id).ok()?;
        Some(&self.0[index].entry)
    }

    /// The record a property value names: value + 10,000,000.
    fn get_named(&self, value: u32) -> Option<&'a TileArtEntry> {
        self.get(value.checked_add(MOBILE_DEFINITION_BASE)?)
    }
}

/// The mobile wearing the items: its server body and its definition, the creature placeholder
/// `10,000,000 + body`.
pub(crate) struct Wearer<'a> {
    pub body: u32,
    pub definition: Option<&'a TileArtEntry>,
    /// `GetGender` (props1 key 10, 0 when absent).
    pub gender: u32,
    /// `GetRace` (props1 key 9, 1 when absent).
    pub race: u32,
}

impl<'a> Wearer<'a> {
    pub(crate) fn new(definitions: Definitions<'a>, body: u32) -> Self {
        let definition = definitions.get_named(strip_definition_base(body));
        Self {
            body,
            definition,
            gender: definition.map_or(DEFAULT_GENDER, gender_of),
            race: definition.and_then(|definition| definition.property1(PropertyKey::Race)).unwrap_or(DEFAULT_RACE),
        }
    }
}

fn gender_of(definition: &TileArtEntry) -> u32 {
    definition.property1(PropertyKey::Gender).unwrap_or(DEFAULT_GENDER)
}

fn strip_definition_base(id: u32) -> u32 {
    if id > MOBILE_DEFINITION_BASE { id - MOBILE_DEFINITION_BASE } else { id }
}

/// `UOObjectDefinitionBase::GetAppearance` (0x00842bb2) for a wearer: the Appearance property (8), else the tile
/// id, unless an appearance filter applies first (`AppearanceFilterByOwnerBodyType::Apply` 0x0084b3c4 matches the
/// wearer's Appearance, else its tile id, against a row's body type; `AppearanceFilterByOwnerGender::Apply`
/// 0x0084b485 its gender). Animation filters (subtype 2) are skipped.
pub(crate) fn appearance(definition: &TileArtEntry, owner: Option<&TileArtEntry>) -> u32 {
    let own = definition.property1(PropertyKey::Appearance).unwrap_or(definition.tile_id);
    let Some(owner) = owner else { return own };
    for filter in &definition.appearance_vector {
        let applied = match filter {
            TaeAppearanceFilter::ByOwnerBodyType(rows) => {
                let key = owner.property1(PropertyKey::Appearance).unwrap_or(owner.tile_id);
                rows.iter().find(|row| row.body_type == key).map(|row| row.appearance)
            }
            TaeAppearanceFilter::ByOwnerGender { gender, appearance } => {
                (gender_of(owner) == u32::from(*gender)).then_some(*appearance)
            }
            TaeAppearanceFilter::AnimationByOwnerBodyType(_) => None,
        };
        if let Some(appearance) = applied {
            return appearance;
        }
    }
    own
}

/// `FindLayerFrame` (0x0050b850): a definition with an Appearance property tries the definition it names first, up
/// to `MAX_APPEARANCE_DEPTH` levels; then the frame numbered by its `GetAppearance`.
fn find_layer_frame(
    sets: &FrameSets,
    definitions: Definitions,
    set: usize,
    depth: u32,
    definition: &TileArtEntry,
    owner: Option<&TileArtEntry>,
) -> Option<FrameRef> {
    if depth + 1 > MAX_APPEARANCE_DEPTH {
        return None;
    }
    let named = definition.property1(PropertyKey::Appearance).and_then(|value| definitions.get_named(value));
    if let Some(found) = named.and_then(|named| find_layer_frame(sets, definitions, set, depth + 1, named, owner)) {
        return Some(found);
    }
    sets.find(set, appearance(definition, owner))
}

/// Why a worn item has no frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemMiss {
    /// `tileart.uop` has no record of the item.
    NoDefinition,
    /// The item's Paperdoll property (11) names a record that does not exist.
    NoPaperdollDefinition(u32),
    /// Neither set has a frame for it.
    NoFrame,
}

/// `ECPaperdollTexture::GetLayerFrame` (0x0050ba30): the frame of a worn item. A Paperdoll property (11) on the
/// item's record swaps in the record it names; a miss in the female set is retried in the male one.
pub(crate) fn item_frame(
    sets: &FrameSets,
    definitions: Definitions,
    wearer: &Wearer,
    item_id: u32,
) -> Result<FrameRef, ItemMiss> {
    let item = definitions.get(item_id).ok_or(ItemMiss::NoDefinition)?;
    let definition = match item.property1(PropertyKey::Paperdoll) {
        Some(value) => definitions.get_named(value).ok_or(ItemMiss::NoPaperdollDefinition(value))?,
        None => item,
    };
    let set = gender_set(wearer.gender);
    find_layer_frame(sets, definitions, set, 0, definition, wearer.definition)
        .or_else(|| find_layer_frame(sets, definitions, MALE_SET, 0, definition, wearer.definition))
        .ok_or(ItemMiss::NoFrame)
}

/// The layer an item is worn on: its record's Layer property (6), else `fallback` (the tiledata layer).
pub(crate) fn item_layer(definitions: Definitions, item_id: u32, fallback: Option<u8>) -> Option<usize> {
    let layer = definitions
        .get(item_id)
        .and_then(|definition| definition.property1(PropertyKey::Layer))
        .or(fallback.map(u32::from))?;
    usize::try_from(layer).ok().filter(|&layer| layer < LAYER_COUNT)
}

/// The tables of the given server hues, read from `hues.uop`.
pub(crate) fn hue_tables(package: &UopPackage, hues: impl IntoIterator<Item = u16>) -> eyre::Result<EcHueTables> {
    let mut tables = EcHueTables::default();
    for hue in hues {
        let EcHueLookup::Bitmap(number) = EcHueLookup::from_hue(hue) else { continue };
        if tables.tables.contains_key(&number) {
            continue;
        }
        if let Some(bitmap) = package.unpack_file_by_hash(hue_bitmap_hash(number))? {
            tables.tables.insert(number, EcHueTable::from_bitmap(&bitmap).wrap_err_with(|| format!("hue {hue}"))?);
        }
    }
    Ok(tables)
}

/// An item to put on the paperdoll.
pub(crate) struct WornItem {
    pub item_id: u32,
    pub hue: u16,
    /// Its tiledata layer, used when its `tileart.uop` record has no Layer property.
    pub tiledata_layer: Option<u8>,
}

/// The frames to draw, bottom first, and what the client would leave out.
pub(crate) struct Paperdoll {
    pub frames: Vec<PlacedFrame>,
    pub notes: Vec<String>,
}

/// What the client draws for a body wearing `items`: the body, then the worn layers in their paperdoll order.
pub(crate) fn build(
    sets: &FrameSets,
    definitions: Definitions,
    rules: &WearableRules,
    hues: &EcHueTables,
    body: u32,
    body_hue: u16,
    items: &[WornItem],
) -> eyre::Result<Paperdoll> {
    let wearer = Wearer::new(definitions, body);
    let mut notes = Vec::new();
    let (body_frame, fell_back) = sets
        .body_frame(&wearer)
        .ok_or_else(|| eyre::eyre!("{PAPERDOLL_UOP_NAME} has no frame {body} nor the fallback {FALLBACK_BODY_FRAME}"))?;
    if fell_back {
        notes.push(format!("body {body} has no frame: the male {FALLBACK_BODY_FRAME} is drawn, as the client does"));
    }
    let mut frames = vec![sets.decode(body_frame, body_hue, hues)?];

    let mut worn = [None; LAYER_COUNT];
    let mut worn_hues = [0u16; LAYER_COUNT];
    for item in items {
        let layer = item_layer(definitions, item.item_id, item.tiledata_layer)
            .ok_or_else(|| eyre::eyre!("item {} has no layer in tileart.uop or tiledata", item.item_id))?;
        if let Some(other) = worn[layer] {
            eyre::bail!("items {other} and {} are both worn on layer {layer} ({})", item.item_id, SLOT_NAMES[layer]);
        }
        worn[layer] = Some(item.item_id);
        worn_hues[layer] = item.hue;
    }
    let order = rules.paperdoll_order(&worn, wearer.race);
    for (layer, item) in worn.iter().enumerate() {
        if let Some(item) = item
            && !order.contains(&layer)
        {
            notes.push(format!("item {item} on {}: not drawn by the paperdoll draw order", SLOT_NAMES[layer]));
        }
    }
    for layer in order {
        let Some(item) = worn[layer] else { continue };
        match item_frame(sets, definitions, &wearer, item) {
            Ok(frame) => frames.push(sets.decode(frame, worn_hues[layer], hues)?),
            Err(miss) => notes.push(format!("item {item} on {}: {}", SLOT_NAMES[layer], miss.describe())),
        }
    }
    Ok(Paperdoll { frames, notes })
}

/// The frames drawn in order on a canvas just big enough for them all: width, height and RGBA.
pub(crate) fn compose(frames: &[PlacedFrame]) -> (u32, u32, Vec<u8>) {
    let min_x = frames.iter().map(|frame| frame.x).min().unwrap_or(0);
    let min_y = frames.iter().map(|frame| frame.y).min().unwrap_or(0);
    let max_x = frames.iter().map(|frame| frame.x + frame.width as i32).max().unwrap_or(0);
    let max_y = frames.iter().map(|frame| frame.y + frame.height as i32).max().unwrap_or(0);
    let width = (max_x - min_x).max(1) as u32;
    let height = (max_y - min_y).max(1) as u32;
    let mut canvas = vec![0u8; width as usize * height as usize * 4];
    for frame in frames {
        super::gumps::alpha_blend_at(
            &mut canvas,
            width,
            frame.x - min_x,
            frame.y - min_y,
            frame.width,
            frame.height,
            &frame.rgba,
        );
    }
    (width, height, canvas)
}

impl ItemMiss {
    fn describe(self) -> String {
        match self {
            Self::NoDefinition => "no tileart.uop record".to_string(),
            Self::NoPaperdollDefinition(value) => {
                format!("its Paperdoll property names {}, which has no record", u64::from(value) + u64::from(MOBILE_DEFINITION_BASE))
            }
            Self::NoFrame => format!("no frame in either set of {PAPERDOLL_UOP_NAME}"),
        }
    }
}

/// The Derived column of an item row: its layer, and its frame once the frame sets are loaded.
pub(crate) fn item_detail(
    sets: Option<&FrameSets>,
    definitions: Definitions,
    body: u32,
    item_id: u32,
    tiledata_layer: Option<u8>,
) -> String {
    let layer = match item_layer(definitions, item_id, tiledata_layer) {
        Some(layer) => format!("layer={layer} {}", SLOT_NAMES[layer]),
        None => "no layer".to_string(),
    };
    let Some(sets) = sets else { return format!("{layer} (Render to find the frame)") };
    match item_frame(sets, definitions, &Wearer::new(definitions, body), item_id) {
        Ok(frame) => format!(
            "{layer} frame={} {}",
            frame.entry.frame_number,
            if frame.set == MALE_SET { "male" } else { "female" }
        ),
        Err(miss) => format!("{layer}: {}", miss.describe()),
    }
}

/// An item list of `wearables.xml`: `None` for "any".
type ItemKey = Option<u32>;

/// One `override` of a `wearable`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Override {
    priority: i32,
    /// `GetRace` value it is limited to.
    race: Option<u32>,
    /// It applies to the paperdoll: no direction, or the paperdoll's.
    paperdoll: bool,
    /// (layer, item) pairs; any one worn sets the priority. None: always.
    conditions: Vec<(usize, ItemKey)>,
}

/// The paperdoll draw order of `wearables.xml` (`WearableRules.kdl`): the priorities of the paperdoll `draworder`
/// and the `wearable` overrides, keyed by (layer, item).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WearableRules {
    priorities: [i32; LAYER_COUNT],
    overrides: HashMap<(usize, ItemKey), Vec<Override>>,
}

#[derive(Decode)]
struct WearableRulesKdl {
    #[knuffel(children(name = "draworder"))]
    draw_orders: Vec<DrawOrderKdl>,
    #[knuffel(children(name = "wearable"))]
    wearables: Vec<WearableKdl>,
}

#[derive(Decode)]
struct DrawOrderKdl {
    #[knuffel(argument)]
    direction: String,
    #[knuffel(children(name = "slot"))]
    slots: Vec<DrawOrderSlotKdl>,
}

#[derive(Decode)]
struct DrawOrderSlotKdl {
    #[knuffel(argument)]
    slot: String,
    #[knuffel(property)]
    priority: i32,
}

#[derive(Decode)]
struct WearableKdl {
    #[knuffel(argument)]
    slot: String,
    #[knuffel(property)]
    items: String,
    #[knuffel(children(name = "override"))]
    overrides: Vec<OverrideKdl>,
}

#[derive(Decode)]
struct OverrideKdl {
    #[knuffel(property)]
    priority: i32,
    #[knuffel(property)]
    race: Option<String>,
    #[knuffel(property)]
    direction: Option<String>,
    #[knuffel(children(name = "when"))]
    conditions: Vec<WhenKdl>,
}

#[derive(Decode)]
struct WhenKdl {
    #[knuffel(argument)]
    slot: String,
    #[knuffel(property)]
    items: String,
}

fn slot_layer(slot: &str) -> eyre::Result<usize> {
    SLOT_NAMES
        .iter()
        .position(|name| *name == slot)
        .ok_or_else(|| eyre::eyre!("{WEARABLE_RULES_FILE_NAME}: unknown slot `{slot}`; expected one of {SLOT_NAMES:?}"))
}

fn item_keys(items: &str) -> eyre::Result<Vec<ItemKey>> {
    if items.trim().eq_ignore_ascii_case(ANY_ITEM) {
        return Ok(vec![None]);
    }
    items
        .split(',')
        .map(|part| {
            super::parse_entry_id(part)
                .and_then(|id| u32::try_from(id).ok())
                .map(Some)
                .ok_or_else(|| eyre::eyre!("{WEARABLE_RULES_FILE_NAME}: `{}` is not an item id or `any`", part.trim()))
        })
        .collect()
}

impl WearableRules {
    /// The rules shipped with dynamapper, decoded once.
    pub(crate) fn embedded() -> Result<&'static Self, &'static str> {
        static RULES: std::sync::OnceLock<Result<WearableRules, String>> = std::sync::OnceLock::new();
        RULES
            .get_or_init(|| Self::decode(WEARABLE_RULES_KDL).map_err(|error| format!("{error:#}")))
            .as_ref()
            .map_err(String::as_str)
    }

    pub(crate) fn decode(text: &str) -> eyre::Result<Self> {
        let decoded: WearableRulesKdl = uot_kdl::strict::decode(WEARABLE_RULES_FILE_NAME, text, WEARABLE_RULES_SCHEMA)?;
        let mut rules = Self::default();
        let draw_order = decoded
            .draw_orders
            .iter()
            .find(|order| order.direction == PAPERDOLL_DIRECTION)
            .ok_or_else(|| eyre::eyre!("{WEARABLE_RULES_FILE_NAME}: no draworder \"{PAPERDOLL_DIRECTION}\""))?;
        for slot in &draw_order.slots {
            rules.priorities[slot_layer(&slot.slot)?] = slot.priority;
        }
        for wearable in decoded.wearables {
            let layer = slot_layer(&wearable.slot)?;
            let mut overrides = Vec::with_capacity(wearable.overrides.len());
            for node in wearable.overrides {
                let race = match node.race.as_deref() {
                    None => None,
                    Some(name) => Some(RACES.iter().find(|(race, _)| *race == name).map(|&(_, value)| value).ok_or_else(
                        || eyre::eyre!("{WEARABLE_RULES_FILE_NAME}: unknown race `{name}`; expected one of {RACES:?}"),
                    )?),
                };
                let mut conditions = Vec::new();
                for condition in node.conditions {
                    // The client's loader (0x00468230) reads only the attributes named by the slot table: any other
                    // slot, as the shipped `belowchest`, is no condition, and an override left with none always
                    // applies.
                    let Ok(condition_layer) = slot_layer(&condition.slot) else {
                        log::debug!("{WEARABLE_RULES_FILE_NAME}: `when \"{}\"` is not a slot; ignored", condition.slot);
                        continue;
                    };
                    conditions.extend(item_keys(&condition.items)?.into_iter().map(|item| (condition_layer, item)));
                }
                overrides.push(Override {
                    priority: node.priority,
                    race,
                    paperdoll: node.direction.as_deref().is_none_or(|direction| direction == PAPERDOLL_DIRECTION),
                    conditions,
                });
            }
            for item in item_keys(&wearable.items)? {
                // A second rule for the same item adds nothing, as `std::map::insert` (the loader is not traced).
                rules.overrides.entry((layer, item)).or_insert_with(|| overrides.clone());
            }
        }
        Ok(rules)
    }

    /// `BuildEquipmentLayerOrder` (0x00468810, the +0x160 order) and the paperdoll's layer list (0x0050b690): the
    /// worn layers in drawing order. Each worn layer takes the rule of its item, else the rule for any item; the last
    /// of its overrides whose race fits and one of whose conditions is worn sets its priority; a priority below 1
    /// hides the layer. `worn` holds the item id of each layer.
    pub(crate) fn paperdoll_order(&self, worn: &[Option<u32>; LAYER_COUNT], race: u32) -> Vec<usize> {
        let mut priorities = self.priorities;
        for (layer, item) in worn.iter().enumerate() {
            let Some(item) = *item else { continue };
            let Some(overrides) = self.overrides.get(&(layer, Some(item))).or_else(|| self.overrides.get(&(layer, None)))
            else {
                continue;
            };
            for rule in overrides {
                let fits = rule.paperdoll && rule.race.is_none_or(|required| required == race);
                let worn_condition = rule.conditions.is_empty()
                    || rule.conditions.iter().any(|&(other, wanted)| match wanted {
                        Some(wanted) => worn[other] == Some(wanted),
                        None => worn[other].is_some(),
                    });
                if fits && worn_condition {
                    priorities[layer] = rule.priority;
                }
            }
        }
        let mut layers: Vec<usize> = (0..LAYER_COUNT)
            .filter(|&layer| worn[layer].is_some() && priorities[layer] > 0 && !SKIPPED_LAYERS.contains(&layer))
            .collect();
        layers.sort_by_key(|&layer| priorities[layer]);
        layers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::animationframe::AnimationBounds;
    use uocf::enhanced::tileart::{TaeBodyAppearance, TaeProp};

    const HUMAN_MALE: u32 = 400;
    const HUMAN_FEMALE: u32 = 401;
    const FEMALE: u32 = 1;
    const ROBE: u32 = 7939;
    const LEGS: u32 = 5137;
    const SHIRT: u32 = 5399;
    const ELF_RACE: u32 = 2;

    fn definition(tile_id: u32, properties: &[(PropertyKey, u32)]) -> TileArtEntry {
        TileArtEntry {
            tile_id,
            prop_vector1: properties.iter().map(|&(key, val)| TaeProp { id: key as u8, val }).collect(),
            ..TileArtEntry::default()
        }
    }

    fn entries(mut definitions: Vec<TileArtEntry>) -> Vec<TileArtFileEntry> {
        definitions.sort_by_key(|definition| definition.tile_id);
        definitions.into_iter().map(|entry| TileArtFileEntry { filename_hash: 0, entry }).collect()
    }

    /// A set whose frames are the given numbers, each an empty 0 x 0 box.
    fn set(action: u16, numbers: &[u16]) -> AnimationFrame {
        AnimationFrame {
            version: 0,
            total_size: 0,
            animation_id: 1,
            bounds: AnimationBounds::default(),
            colours_count: 0,
            colours_offset: 0,
            frames_count: numbers.len() as u32,
            frames_offset: 0,
            colours: Vec::new(),
            frames: numbers.iter().map(|&frame_number| FrameEntry { action, frame_number, ..FrameEntry::default() }).collect(),
            image_data: Vec::new().into(),
        }
    }

    fn found(frame: Result<FrameRef, ItemMiss>) -> Option<(usize, u16)> {
        frame.ok().map(|frame| (frame.set, frame.entry.frame_number))
    }

    #[test]
    fn the_body_takes_its_gender_set_else_the_male_400() {
        let sets = FrameSets::from_sets([set(0, &[400, 605]), set(1, &[401])]);
        let files = entries(vec![
            definition(MOBILE_DEFINITION_BASE + HUMAN_MALE, &[]),
            definition(MOBILE_DEFINITION_BASE + HUMAN_FEMALE, &[(PropertyKey::Gender, FEMALE)]),
        ]);
        let definitions = Definitions(&files);
        let female = sets.body_frame(&Wearer::new(definitions, HUMAN_FEMALE)).unwrap();
        assert_eq!((female.0.set, female.0.entry.frame_number, female.1), (FEMALE_SET, 401, false));
        let unknown = sets.body_frame(&Wearer::new(definitions, 999)).unwrap();
        assert_eq!((unknown.0.set, unknown.0.entry.frame_number, unknown.1), (MALE_SET, 400, true));
    }

    #[test]
    fn an_item_follows_paperdoll_appearance_and_owner_filters() {
        const NAMED_APPEARANCE: u32 = 496;
        const GENDER_APPEARANCE: u32 = 497;
        const BODY_APPEARANCE: u32 = 498;
        let sets = FrameSets::from_sets([
            set(0, &[NAMED_APPEARANCE as u16, BODY_APPEARANCE as u16]),
            set(1, &[GENDER_APPEARANCE as u16]),
        ]);
        let mut by_gender = definition(SHIRT, &[]);
        by_gender.appearance_vector =
            vec![TaeAppearanceFilter::ByOwnerGender { gender: FEMALE as u8, appearance: GENDER_APPEARANCE }];
        let mut by_body = definition(ROBE, &[]);
        by_body.appearance_vector = vec![TaeAppearanceFilter::ByOwnerBodyType(vec![TaeBodyAppearance {
            body_type: MOBILE_DEFINITION_BASE + HUMAN_MALE,
            appearance: BODY_APPEARANCE,
        }])];
        let files = entries(vec![
            definition(MOBILE_DEFINITION_BASE + HUMAN_MALE, &[]),
            definition(MOBILE_DEFINITION_BASE + HUMAN_FEMALE, &[(PropertyKey::Gender, FEMALE)]),
            // Paperdoll 11 swaps in the record 10,000,000 + value; its Appearance 8 numbers the frame.
            definition(LEGS, &[(PropertyKey::Paperdoll, 77)]),
            definition(MOBILE_DEFINITION_BASE + 77, &[(PropertyKey::Appearance, NAMED_APPEARANCE)]),
            by_gender,
            by_body,
        ]);
        let definitions = Definitions(&files);
        let male = Wearer::new(definitions, HUMAN_MALE);
        let female = Wearer::new(definitions, HUMAN_FEMALE);
        // The female set has no 496: the male set is tried with action 0.
        assert_eq!(found(item_frame(&sets, definitions, &female, LEGS)), Some((MALE_SET, NAMED_APPEARANCE as u16)));
        assert_eq!(found(item_frame(&sets, definitions, &female, SHIRT)), Some((FEMALE_SET, GENDER_APPEARANCE as u16)));
        assert_eq!(item_frame(&sets, definitions, &male, SHIRT), Err(ItemMiss::NoFrame));
        assert_eq!(found(item_frame(&sets, definitions, &male, ROBE)), Some((MALE_SET, BODY_APPEARANCE as u16)));
        assert_eq!(item_frame(&sets, definitions, &male, 1), Err(ItemMiss::NoDefinition));
    }

    #[test]
    fn an_appearance_chain_stops_after_five_levels() {
        const LOOP: u32 = 5;
        let sets = FrameSets::from_sets([set(0, &[]), set(1, &[])]);
        let files = entries(vec![definition(MOBILE_DEFINITION_BASE + LOOP, &[(PropertyKey::Appearance, LOOP)])]);
        let definitions = Definitions(&files);
        let looping = definitions.get(MOBILE_DEFINITION_BASE + LOOP).unwrap();
        assert_eq!(find_layer_frame(&sets, definitions, MALE_SET, 0, looping, None), None);
    }

    fn worn(items: &[(&str, u32)]) -> [Option<u32>; LAYER_COUNT] {
        let mut worn = [None; LAYER_COUNT];
        for &(slot, item) in items {
            worn[slot_layer(slot).unwrap()] = Some(item);
        }
        worn
    }

    #[test]
    fn the_embedded_rules_order_the_paperdoll() {
        let rules = WearableRules::embedded().unwrap();
        let human = DEFAULT_RACE;
        // The paperdoll draw order: legs 6000 under the torso 7000; the cape at 500, under everything worn.
        let order = rules.paperdoll_order(&worn(&[("torso", SHIRT), ("legs", 5400), ("cape", 5397)]), human);
        assert_eq!(order, vec![slot_layer("cape").unwrap(), slot_layer("legs").unwrap(), slot_layer("torso").unwrap()]);
        // A robe (7939) hides the legs, but plate legs (5137) stay, at 15000, under the robe's 18000.
        let robe = worn(&[("dress", ROBE), ("legs", 5400)]);
        assert_eq!(rules.paperdoll_order(&robe, human), vec![slot_layer("dress").unwrap()]);
        let plate = worn(&[("dress", ROBE), ("legs", LEGS)]);
        assert_eq!(rules.paperdoll_order(&plate, human), vec![slot_layer("legs").unwrap(), slot_layer("dress").unwrap()]);
        // The backpack and the mount are not drawn.
        let skipped = worn(&[("backpack", 3701), ("riding", 16016)]);
        assert!(rules.paperdoll_order(&skipped, human).is_empty());
    }

    #[test]
    fn overrides_need_their_race_and_the_paperdoll_direction() {
        let rules = WearableRules::decode(
            "draworder \"paperdoll\" { slot \"legs\" priority=6000; slot \"dress\" priority=18000; }\n\
             wearable \"legs\" items=\"any\" {\n\
                 override priority=0 race=\"elf\" { when \"dress\" items=\"any\"; }\n\
                 override priority=20000 direction=\"north\" { when \"dress\" items=\"any\"; }\n\
             }\n",
        )
        .unwrap();
        let dressed = worn(&[("dress", ROBE), ("legs", LEGS)]);
        let legs_then_dress = vec![slot_layer("legs").unwrap(), slot_layer("dress").unwrap()];
        assert_eq!(rules.paperdoll_order(&dressed, DEFAULT_RACE), legs_then_dress);
        assert_eq!(rules.paperdoll_order(&dressed, ELF_RACE), vec![slot_layer("dress").unwrap()]);
    }

    #[test]
    fn a_condition_on_an_unknown_slot_is_dropped_and_its_override_always_applies() {
        const SHOWN_UNDER_DRESS: i32 = 11900;
        let rules = WearableRules::decode(
            "draworder \"paperdoll\" { slot \"chest\" priority=12000; slot \"dress\" priority=18000; }\n\
             wearable \"chest\" items=\"any\" {\n\
                 override priority=0 { when \"dress\" items=\"any\"; }\n\
                 override priority=11900 { when \"belowchest\" items=\"8097\"; }\n\
             }\n",
        )
        .unwrap();
        let overrides = &rules.overrides[&(slot_layer("chest").unwrap(), None)];
        assert_eq!((overrides[1].priority, overrides[1].conditions.len()), (SHOWN_UNDER_DRESS, 0));
        // The later, unconditional override wins over the dress hiding the chest.
        let dressed = worn(&[("dress", ROBE), ("chest", PLATE_CHEST)]);
        assert_eq!(
            rules.paperdoll_order(&dressed, DEFAULT_RACE),
            vec![slot_layer("chest").unwrap(), slot_layer("dress").unwrap()]
        );
    }

    const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
    const TILEART_UOP_NAME: &str = "tileart.uop";
    const PANTS: u32 = 5433;
    const SHOES: u32 = 5903;
    const PLATE_CHEST: u32 = 5141;
    const CLOAK: u32 = 5397;
    const LONG_HAIR: u32 = 8252;
    const KATANA: u32 = 5119;
    const DYED_HUE: u16 = 38;
    const SKIN_HUE: u16 = 1002;
    /// The share of the items with a drawn Layer that must find a frame, for a man or a woman.
    const MIN_FOUND_PERCENT: usize = 85;
    const PERCENT: usize = 100;
    const SHOWN_MISSES: usize = 20;

    fn real_client() -> (std::path::PathBuf, Vec<TileArtFileEntry>, FrameSets) {
        let dir = std::path::PathBuf::from(std::env::var_os(EC_CLIENT_DIR_ENV).expect("set UO_EC_CLIENT_DIR"));
        let tileart = crate::app::find_client_file_case_insensitive(&dir, TILEART_UOP_NAME).expect("tileart.uop");
        let package = UopPackage::load(&tileart).unwrap();
        let mut files: Vec<TileArtFileEntry> = package
            .iter_files()
            .filter_map(|file| {
                Some(TileArtFileEntry { filename_hash: file.filename_hash(), entry: TileArtEntry::parse_raw(file).ok()? })
            })
            .collect();
        files.sort_by_key(|file| file.entry.tile_id);
        let paperdoll = crate::app::find_client_file_case_insensitive(&dir, PAPERDOLL_UOP_NAME).expect("Paperdoll.uop");
        let sets = FrameSets::load(&paperdoll).unwrap();
        (dir, files, sets)
    }

    #[test]
    #[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
    fn shipped_bodies_and_wearables_find_their_frames() {
        let (_, files, sets) = real_client();
        let definitions = Definitions(&files);
        let male = Wearer::new(definitions, HUMAN_MALE);
        let female = Wearer::new(definitions, HUMAN_FEMALE);
        assert_eq!((male.gender, female.gender), (DEFAULT_GENDER, FEMALE));
        let body = |wearer: &Wearer| sets.body_frame(wearer).map(|(frame, fell_back)| (frame.set, frame.entry.frame_number, fell_back));
        assert_eq!(body(&male), Some((MALE_SET, HUMAN_MALE as u16, false)));
        assert_eq!(body(&female), Some((FEMALE_SET, HUMAN_FEMALE as u16, false)));

        let wearables: Vec<u32> = files
            .iter()
            .filter(|file| file.entry.tile_id < MOBILE_DEFINITION_BASE)
            .filter(|file| item_layer(definitions, file.entry.tile_id, None).is_some_and(|layer| !SKIPPED_LAYERS.contains(&layer)))
            .map(|file| file.entry.tile_id)
            .collect();
        // EC 2024: 983 of 1125 for a man, 1054 for a woman. About 70 items have a frame in neither set (9801..9892,
        // the talismans 12120..12123 among them); a man misses the female-only pieces besides (7168..7181, and the
        // gargoyle pieces whose filters name only the gargoyle bodies 10,000,666 and 10,000,667).
        for wearer in [&male, &female] {
            let misses: Vec<(u32, ItemMiss)> = wearables
                .iter()
                .filter_map(|&item| item_frame(&sets, definitions, wearer, item).err().map(|miss| (item, miss)))
                .collect();
            let found = wearables.len() - misses.len();
            assert!(
                found * PERCENT >= wearables.len() * MIN_FOUND_PERCENT,
                "body {}: {found} of {} wearables have a frame; first misses {:?}",
                wearer.body,
                wearables.len(),
                &misses[..misses.len().min(SHOWN_MISSES)],
            );
            assert!(misses.iter().all(|(_, miss)| *miss == ItemMiss::NoFrame), "{misses:?}");
        }
        // The female set holds what a man misses: frame 927 of the female leather shorts (7168).
        const FEMALE_SHORTS: u32 = 7168;
        assert_eq!(item_frame(&sets, definitions, &male, FEMALE_SHORTS), Err(ItemMiss::NoFrame));
        assert_eq!(found(item_frame(&sets, definitions, &female, FEMALE_SHORTS)).map(|(set, _)| set), Some(FEMALE_SET));
        for item in [SHIRT, PANTS, SHOES, PLATE_CHEST, CLOAK, ROBE, LONG_HAIR, KATANA] {
            assert!(item_frame(&sets, definitions, &female, item).is_ok(), "item {item} has no frame for a woman");
        }
    }

    #[test]
    #[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and TMPDIR, and run with --ignored"]
    fn a_shipped_outfit_composes_over_the_body() {
        let (dir, files, sets) = real_client();
        let definitions = Definitions(&files);
        let hues_path = crate::app::find_client_file_case_insensitive(&dir, uocf::enhanced::hues::HUES_UOP_NAME).unwrap();
        let hues = hue_tables(&UopPackage::load_with_mode(&hues_path, LoadMode::Lazy).unwrap(), [SKIN_HUE, DYED_HUE]).unwrap();
        assert_eq!(hues.tables.len(), 2);
        let outfit: Vec<WornItem> = [SHIRT, PANTS, SHOES, PLATE_CHEST, CLOAK, LONG_HAIR, KATANA]
            .into_iter()
            .map(|item_id| WornItem { item_id, hue: if item_id == CLOAK { DYED_HUE } else { 0 }, tiledata_layer: None })
            .collect();
        let rules = WearableRules::embedded().unwrap();
        let paperdoll = build(&sets, definitions, rules, &hues, HUMAN_FEMALE, SKIN_HUE, &outfit).unwrap();
        // The plate chest hides the shirt (`torso` at 0 under any `chest`); everything else is drawn.
        assert_eq!(paperdoll.frames.len(), outfit.len(), "{:?}", paperdoll.notes);
        assert_eq!(paperdoll.notes.len(), 1, "{:?}", paperdoll.notes);
        let (width, height, rgba) = compose(&paperdoll.frames);
        let body = &paperdoll.frames[0];
        assert!(width >= body.width && height >= body.height);
        let path = crate::ui::test_support::output_path("ec_paperdoll.png");
        image::save_buffer(&path, &rgba, width, height, image::ExtendedColorType::Rgba8).unwrap();
        log::info!("wrote {}", path.display());
    }

    #[test]
    fn wearable_rules_reject_unknown_slots_races_and_items() {
        let slot = WearableRules::decode("draworder \"paperdoll\" { slot \"tail\" priority=1; }\n").unwrap_err();
        assert!(format!("{slot:#}").contains("unknown slot `tail`"), "{slot:#}");
        let race = WearableRules::decode(
            "draworder \"paperdoll\"\nwearable \"legs\" items=\"any\" { override priority=0 race=\"orc\"; }\n",
        )
        .unwrap_err();
        assert!(format!("{race:#}").contains("unknown race `orc`"), "{race:#}");
        let item = WearableRules::decode("draworder \"paperdoll\"\nwearable \"legs\" items=\"12,boots\"\n").unwrap_err();
        assert!(format!("{item:#}").contains("`boots` is not an item id"), "{item:#}");
        let order = WearableRules::decode("draworder \"north\"\n").unwrap_err();
        assert!(format!("{order:#}").contains("no draworder \"paperdoll\""), "{order:#}");
    }
}
