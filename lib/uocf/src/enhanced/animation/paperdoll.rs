//! The Enhanced Client's paperdoll images: the body and worn-item frames of `Paperdoll.uop`, found and ordered as
//! `ECPaperdollTexture::Build` (UOSA 0x0050b340) does. The rule and its addresses are in
//! `scripts/dev-helpers/ec-research/ghidra-paperdoll/results/uosa_paperdoll.md`.
//!
//! The lookup reads the definition records through [`PaperdollDefinitions`] and the frames through
//! [`PaperdollFrames`], so the editor (over `tileart.uop` and `Paperdoll.uop`) and dynamapper (over `tilemeta.uddp`
//! and `paperdoll_ec.uddp`) find the same frames in the same order.

use std::collections::HashMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use knuffel::Decode;

use crate::enhanced::animationframe::{AnimationFrame, FrameEntry};
use crate::enhanced::hues::{hue_bitmap_hash, EcHueLookup, EcHueTable, EcHueTables, ShadowPaletteStats};
use crate::enhanced::mobile_body::{DEFAULT_GENDER, DEFAULT_RACE, MOBILE_DEFINITION_BASE};
use crate::enhanced::tileart::{tileart_entry_hash, PropertyKey, TaeAppearanceFilter, TaeBodyAppearance, TileArtEntry};
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{LoadMode, UopPackage};

pub const PAPERDOLL_UOP_NAME: &str = "Paperdoll.uop";
/// The two frame sets, the male (asset 0x40) and the female (asset 0x41); every frame of a set has the set's
/// index as its action.
pub const FRAME_SET_PATHS: [&str; GENDER_SET_COUNT] =
    ["build/paperdoll/000001/00.bin", "build/paperdoll/000001/01.bin"];
pub const GENDER_SET_COUNT: usize = 2;
/// The male set: gender 0 picks it, any other gender the female one.
pub const MALE_SET: usize = 0;
pub const FEMALE_SET: usize = 1;
/// `FindPaperdollFrame` (0x0050b780) falls back to this frame of the male set, the human male body.
pub const FALLBACK_BODY_FRAME: u32 = 400;
/// `FindLayerFrame` (0x0050b850) follows an `Appearance` chain this many levels deep.
const MAX_APPEARANCE_DEPTH: u32 = 5;
/// Equipment layers of a mobile.
pub const LAYER_COUNT: usize = 26;
/// The slot names of `wearables.xml`, indexed by layer: the string table at UOSA 0x00db9ba0.
pub const SLOT_NAMES: [&str; LAYER_COUNT] = [
    "drag", "righthand", "lefthand", "feet", "legs", "torso", "head", "hands", "finger1", "talisman", "neck", "hair",
    "waist", "chest", "lwrist", "face", "facialhair", "abovechest", "ears", "arms", "cape", "backpack", "dress",
    "skirt", "feetlegs", "riding",
];
/// Layers the paperdoll's layer list (0x0050b690) skips: layer 0, the backpack and the mount.
pub const SKIPPED_LAYERS: [usize; 3] = [0, 0x15, 0x19];
/// The draw order `BuildEquipmentLayerOrder` (0x00468810) builds for the paperdoll, direction 9.
pub const PAPERDOLL_DIRECTION: &str = "paperdoll";
/// `GetRace` values (props1 key 9; 1, human, when absent) of the race names of `wearables.xml`.
pub const RACES: [(&str, u32); 3] = [("human", 1), ("elf", 2), ("gargoyle", 3)];
/// The item list that matches any item.
const ANY_ITEM: &str = "any";
pub const WEARABLE_RULES_FILE_NAME: &str = "WearableRules.kdl";
const PRIORITY_PROPERTY: &str = "priority";
const ITEMS_PROPERTY: &str = "items";
/// `draworder "<direction>" { slot "<slot>" priority=N }` and
/// `wearable "<slot>" items="..." { override priority=N [race=] [direction=] { when "<slot>" items="..." } }`.
pub const WEARABLE_RULES_SCHEMA: &[uot_kdl::strict::NodeSchema] = &[
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
/// The bytes of an RGBA texel.
const RGBA_BYTES: usize = 4;
const ALPHA_BYTE: usize = 3;
const OPAQUE: u16 = u8::MAX as u16;

/// The set of a gender: 0 the male one, any other the female one (`ECPaperdollTexture::Build`).
pub fn gender_set(gender: u32) -> usize {
    if gender == DEFAULT_GENDER { MALE_SET } else { FEMALE_SET }
}

/// One appearance filter of a definition, as `UOObjectDefinitionBase::GetAppearance` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceFilter<'a> {
    /// Subtype 0: the appearance of the row whose body type is the wearer's.
    ByOwnerBodyType(&'a [TaeBodyAppearance]),
    /// Subtype 1: `appearance` when the wearer has `gender`.
    ByOwnerGender { gender: u8, appearance: u32 },
    /// Subtype 2, the animation filter, which the paperdoll skips.
    Animation,
}

impl<'a> From<&'a TaeAppearanceFilter> for AppearanceFilter<'a> {
    fn from(filter: &'a TaeAppearanceFilter) -> Self {
        match filter {
            TaeAppearanceFilter::ByOwnerBodyType(rows) => Self::ByOwnerBodyType(rows),
            TaeAppearanceFilter::ByOwnerGender { gender, appearance } => {
                Self::ByOwnerGender { gender: *gender, appearance: *appearance }
            }
            TaeAppearanceFilter::AnimationByOwnerBodyType(_) => Self::Animation,
        }
    }
}

/// A definition record (`UOTileArt::GetObjectDefinition`) as the paperdoll reads it: its tile id, its
/// `prop_vector1` values and its appearance filters.
pub trait PaperdollDefinition<'a>: Copy + 'a {
    fn tile_id(self) -> u32;
    /// The value of `key` in `prop_vector1`, as the client's definition getters read it.
    fn property(self, key: PropertyKey) -> Option<u32>;
    /// The appearance filters, in record order.
    fn appearance_filters(self) -> impl Iterator<Item = AppearanceFilter<'a>> + 'a;
}

impl<'a> PaperdollDefinition<'a> for &'a TileArtEntry {
    fn tile_id(self) -> u32 {
        self.tile_id
    }

    fn property(self, key: PropertyKey) -> Option<u32> {
        self.property1(key)
    }

    fn appearance_filters(self) -> impl Iterator<Item = AppearanceFilter<'a>> + 'a {
        self.appearance_vector.iter().map(AppearanceFilter::from)
    }
}

/// The definition records by tile id: `UOTileArt::GetObjectDefinition`.
pub trait PaperdollDefinitions<'a>: Copy {
    type Definition: PaperdollDefinition<'a>;
    /// The record of `tile_id`.
    fn get(self, tile_id: u32) -> Option<Self::Definition>;

    /// The record a property value names: value + 10,000,000.
    fn get_named(self, value: u32) -> Option<Self::Definition> {
        self.get(value.checked_add(MOBILE_DEFINITION_BASE)?)
    }
}

/// `tileart.uop` records sorted by their stored tile id, found by it: records built in tests, which store their
/// entry name's id. A client file is read with [`NamedTileArt`].
#[derive(Clone, Copy)]
pub struct SortedTileArt<'a>(pub &'a [TileArtEntry]);

impl<'a> PaperdollDefinitions<'a> for SortedTileArt<'a> {
    type Definition = &'a TileArtEntry;

    fn get(self, tile_id: u32) -> Option<&'a TileArtEntry> {
        let index = self.0.binary_search_by_key(&tile_id, |entry| entry.tile_id).ok()?;
        Some(&self.0[index])
    }
}

/// The entry ids the paperdoll looks up: a 16-bit item id, or 10,000,000 plus a 16-bit body or property value.
pub const ENTRY_ID_SPAN: u32 = 1 << u16::BITS;

/// The records of a `tileart.uop`, found by the id of their entry name (`build/tileart/%08d.bin`) as the client
/// finds them. The tile id stored in a record is only copied (`GetTileId`), and a few client records store another
/// id than their name's (43957..=43963 store 43956, 42091 stores 41516).
#[derive(Debug, Default)]
pub struct NamedTileArt {
    /// (entry id, record), ascending by entry id.
    pub records: Vec<(u32, TileArtEntry)>,
    /// Records that could not be parsed.
    pub unreadable: usize,
    /// Records named outside the ids the paperdoll looks up.
    pub unnamed: usize,
    /// (entry id, stored tile id) of the records that store another id than their name's, ascending.
    pub renamed: Vec<(u32, u32)>,
}

impl NamedTileArt {
    pub fn read(package: &UopPackage) -> Self {
        let names: HashMap<u64, u32> = (0..ENTRY_ID_SPAN)
            .chain(MOBILE_DEFINITION_BASE..MOBILE_DEFINITION_BASE + ENTRY_ID_SPAN)
            .map(|id| (tileart_entry_hash(id), id))
            .collect();
        let mut named = Self::default();
        for file in package.iter_files() {
            let Ok(record) = TileArtEntry::parse_raw(file) else {
                named.unreadable += 1;
                continue;
            };
            let entry_id = if tileart_entry_hash(record.tile_id) == file.filename_hash() {
                record.tile_id
            } else if let Some(&entry_id) = names.get(&file.filename_hash()) {
                named.renamed.push((entry_id, record.tile_id));
                entry_id
            } else {
                named.unnamed += 1;
                continue;
            };
            named.records.push((entry_id, record));
        }
        named.records.sort_unstable_by_key(|&(entry_id, _)| entry_id);
        named.renamed.sort_unstable();
        named
    }
}

impl<'a> PaperdollDefinitions<'a> for &'a NamedTileArt {
    type Definition = &'a TileArtEntry;

    fn get(self, entry_id: u32) -> Option<&'a TileArtEntry> {
        let index = self.records.binary_search_by_key(&entry_id, |&(id, _)| id).ok()?;
        Some(&self.records[index].1)
    }
}

/// Frames by (set, number): `UOAnimations::FindFrameEntry` (0x00430f60) over the two sets.
pub trait PaperdollFrames {
    type Frame: Copy;
    /// The frame (action, number) of `set`, the action being the set's index.
    fn find_in_set(&self, set: usize, number: u16) -> Option<Self::Frame>;
}

/// A frame of a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRef<F = FrameEntry> {
    pub set: usize,
    pub entry: F,
}

/// `UOAnimations::FindFrameEntry` (0x00430f60): it compares 32 bits, so a number past the 16-bit frame numbers finds
/// nothing.
pub fn find<S: PaperdollFrames>(sets: &S, set: usize, number: u32) -> Option<FrameRef<S::Frame>> {
    let number = u16::try_from(number).ok()?;
    sets.find_in_set(set, number).map(|entry| FrameRef { set, entry })
}

/// The mobile wearing the items: its server body and its definition, the creature placeholder
/// `10,000,000 + body`.
#[derive(Debug, Clone, Copy)]
pub struct Wearer<D> {
    pub body: u32,
    pub definition: Option<D>,
    /// `GetGender` (props1 key 10, 0 when absent).
    pub gender: u32,
    /// `GetRace` (props1 key 9, 1 when absent).
    pub race: u32,
}

impl<'a, D: PaperdollDefinition<'a>> Wearer<D> {
    pub fn new<S: PaperdollDefinitions<'a, Definition = D>>(definitions: S, body: u32) -> Self {
        let definition = definitions.get_named(strip_definition_base(body));
        Self {
            body,
            definition,
            gender: definition.map_or(DEFAULT_GENDER, gender_of),
            race: definition.and_then(|definition| definition.property(PropertyKey::Race)).unwrap_or(DEFAULT_RACE),
        }
    }
}

fn gender_of<'a>(definition: impl PaperdollDefinition<'a>) -> u32 {
    definition.property(PropertyKey::Gender).unwrap_or(DEFAULT_GENDER)
}

/// The body number of a body or a definition id: id - 10,000,000 above that.
pub fn strip_definition_base(id: u32) -> u32 {
    if id > MOBILE_DEFINITION_BASE { id - MOBILE_DEFINITION_BASE } else { id }
}

/// `FindPaperdollFrame` (0x0050b780): the body's frame in its gender's set, else frame 400 of the male set.
/// The second value is whether the fallback was taken.
pub fn body_frame<S: PaperdollFrames, D>(sets: &S, wearer: &Wearer<D>) -> Option<(FrameRef<S::Frame>, bool)> {
    let body = strip_definition_base(wearer.body);
    match find(sets, gender_set(wearer.gender), body) {
        Some(frame) => Some((frame, false)),
        None => find(sets, MALE_SET, FALLBACK_BODY_FRAME).map(|frame| (frame, true)),
    }
}

/// `UOObjectDefinitionBase::GetAppearance` (0x00842bb2) for a wearer: the Appearance property (8), else the tile
/// id, unless an appearance filter applies first (`AppearanceFilterByOwnerBodyType::Apply` 0x0084b3c4 matches the
/// wearer's Appearance, else its tile id, against a row's body type; `AppearanceFilterByOwnerGender::Apply`
/// 0x0084b485 its gender). Animation filters (subtype 2) are skipped.
pub fn appearance<'a, D: PaperdollDefinition<'a>>(definition: D, owner: Option<D>) -> u32 {
    let own = definition.property(PropertyKey::Appearance).unwrap_or(definition.tile_id());
    let Some(owner) = owner else { return own };
    for filter in definition.appearance_filters() {
        let applied = match filter {
            AppearanceFilter::ByOwnerBodyType(rows) => {
                let key = owner.property(PropertyKey::Appearance).unwrap_or(owner.tile_id());
                rows.iter().find(|row| row.body_type == key).map(|row| row.appearance)
            }
            AppearanceFilter::ByOwnerGender { gender, appearance } => {
                (gender_of(owner) == u32::from(gender)).then_some(appearance)
            }
            AppearanceFilter::Animation => None,
        };
        if let Some(appearance) = applied {
            return appearance;
        }
    }
    own
}

/// `FindLayerFrame` (0x0050b850): a definition with an Appearance property tries the definition it names first, up
/// to `MAX_APPEARANCE_DEPTH` levels; then the frame numbered by its `GetAppearance`.
pub fn find_layer_frame<'a, S: PaperdollFrames, T: PaperdollDefinitions<'a>>(
    sets: &S,
    definitions: T,
    set: usize,
    depth: u32,
    definition: T::Definition,
    owner: Option<T::Definition>,
) -> Option<FrameRef<S::Frame>> {
    if depth + 1 > MAX_APPEARANCE_DEPTH {
        return None;
    }
    let named = definition.property(PropertyKey::Appearance).and_then(|value| definitions.get_named(value));
    if let Some(found) = named.and_then(|named| find_layer_frame(sets, definitions, set, depth + 1, named, owner)) {
        return Some(found);
    }
    find(sets, set, appearance(definition, owner))
}

/// Why a worn item has no frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemMiss {
    /// `tileart.uop` has no record of the item.
    NoDefinition,
    /// The item's Paperdoll property (11) names a record that does not exist.
    NoPaperdollDefinition(u32),
    /// Neither set has a frame for it.
    NoFrame,
}

impl ItemMiss {
    pub fn describe(self) -> String {
        match self {
            Self::NoDefinition => "no tileart.uop record".to_string(),
            Self::NoPaperdollDefinition(value) => {
                format!("its Paperdoll property names {}, which has no record", u64::from(value) + u64::from(MOBILE_DEFINITION_BASE))
            }
            Self::NoFrame => format!("no frame in either set of {PAPERDOLL_UOP_NAME}"),
        }
    }
}

/// `ECPaperdollTexture::GetLayerFrame` (0x0050ba30): the frame of a worn item. A Paperdoll property (11) on the
/// item's record swaps in the record it names; a miss in the female set is retried in the male one.
pub fn item_frame<'a, S: PaperdollFrames, T: PaperdollDefinitions<'a>>(
    sets: &S,
    definitions: T,
    wearer: &Wearer<T::Definition>,
    item_id: u32,
) -> Result<FrameRef<S::Frame>, ItemMiss> {
    let item = definitions.get(item_id).ok_or(ItemMiss::NoDefinition)?;
    let definition = match item.property(PropertyKey::Paperdoll) {
        Some(value) => definitions.get_named(value).ok_or(ItemMiss::NoPaperdollDefinition(value))?,
        None => item,
    };
    let set = gender_set(wearer.gender);
    find_layer_frame(sets, definitions, set, 0, definition, wearer.definition)
        .or_else(|| find_layer_frame(sets, definitions, MALE_SET, 0, definition, wearer.definition))
        .ok_or(ItemMiss::NoFrame)
}

/// The layer an item is worn on: its record's Layer property (6), else `fallback` (the tiledata layer).
pub fn item_layer<'a, T: PaperdollDefinitions<'a>>(definitions: T, item_id: u32, fallback: Option<u8>) -> Option<usize> {
    let layer = definitions
        .get(item_id)
        .and_then(|definition| definition.property(PropertyKey::Layer))
        .or(fallback.map(u32::from))?;
    usize::try_from(layer).ok().filter(|&layer| layer < LAYER_COUNT)
}

/// An item to put on the paperdoll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WornItem {
    pub item_id: u32,
    pub hue: u16,
    /// Its tiledata layer, used when its `tileart.uop` record has no Layer property.
    pub tiledata_layer: Option<u8>,
}

/// A frame to draw and the hue it is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedFrame<F> {
    pub frame: FrameRef<F>,
    pub hue: u16,
    /// The worn item's layer and id; `None` for the body.
    pub item: Option<(usize, u32)>,
}

/// A worn item the client draws no frame for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissedItem {
    pub layer: usize,
    pub item_id: u32,
    pub miss: ItemMiss,
}

/// The frames to draw, bottom first, and what the client would leave out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperdollPlan<F> {
    pub frames: Vec<PlannedFrame<F>>,
    /// The worn items without a frame; each also has a note.
    pub misses: Vec<MissedItem>,
    pub notes: Vec<String>,
}

/// What the client draws for a body wearing `items`: the body, then the worn layers in their paperdoll order.
pub fn plan<'a, S: PaperdollFrames, T: PaperdollDefinitions<'a>>(
    sets: &S,
    definitions: T,
    rules: &WearableRules,
    body: u32,
    body_hue: u16,
    items: &[WornItem],
) -> eyre::Result<PaperdollPlan<S::Frame>> {
    let wearer = Wearer::new(definitions, body);
    let mut notes = Vec::new();
    let (body_frame, fell_back) = body_frame(sets, &wearer)
        .ok_or_else(|| eyre::eyre!("{PAPERDOLL_UOP_NAME} has no frame {body} nor the fallback {FALLBACK_BODY_FRAME}"))?;
    if fell_back {
        notes.push(format!("body {body} has no frame: the male {FALLBACK_BODY_FRAME} is drawn, as the client does"));
    }
    let mut frames = vec![PlannedFrame { frame: body_frame, hue: body_hue, item: None }];
    let mut misses = Vec::new();

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
            Ok(frame) => frames.push(PlannedFrame { frame, hue: worn_hues[layer], item: Some((layer, item)) }),
            Err(miss) => {
                notes.push(format!("item {item} on {}: {}", SLOT_NAMES[layer], miss.describe()));
                misses.push(MissedItem { layer, item_id: item, miss });
            }
        }
    }
    Ok(PaperdollPlan { frames, misses, notes })
}

/// A decoded frame and where it goes: its pixel (0, 0) is drawn at `(x, y)` from the common anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedFrame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The male and female frame sets of `Paperdoll.uop`.
pub struct FrameSets {
    sets: [AnimationFrame; GENDER_SET_COUNT],
}

impl FrameSets {
    pub fn load(path: &Path) -> eyre::Result<Self> {
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

    pub fn from_sets(sets: [AnimationFrame; GENDER_SET_COUNT]) -> Self {
        Self { sets }
    }

    /// The `AnimationFrame` file of a set.
    pub fn set(&self, set: usize) -> &AnimationFrame {
        &self.sets[set]
    }

    /// Decodes a frame with a server hue: the hue's table recolours the palette's hueable colours, as
    /// `UOAnimations::BuildFramePalette` does.
    pub fn decode(&self, frame: FrameRef, hue: u16, hues: &EcHueTables) -> eyre::Result<PlacedFrame> {
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

impl PaperdollFrames for FrameSets {
    type Frame = FrameEntry;

    fn find_in_set(&self, set: usize, number: u16) -> Option<FrameEntry> {
        let action = set as u16;
        self.sets[set].frames.iter().find(|entry| entry.action == action && entry.frame_number == number).copied()
    }
}

/// The frames to draw, bottom first, decoded, and what the client would leave out.
pub struct Paperdoll {
    pub frames: Vec<PlacedFrame>,
    pub notes: Vec<String>,
}

/// What the client draws for a body wearing `items` ([`plan`]), decoded from `Paperdoll.uop`.
pub fn build<'a, T: PaperdollDefinitions<'a>>(
    sets: &FrameSets,
    definitions: T,
    rules: &WearableRules,
    hues: &EcHueTables,
    body: u32,
    body_hue: u16,
    items: &[WornItem],
) -> eyre::Result<Paperdoll> {
    let plan = plan(sets, definitions, rules, body, body_hue, items)?;
    let frames = plan.frames.iter().map(|planned| sets.decode(planned.frame, planned.hue, hues)).collect::<eyre::Result<_>>()?;
    Ok(Paperdoll { frames, notes: plan.notes })
}

/// The tables of the given server hues, read from `hues.uop`.
pub fn hue_tables(package: &UopPackage, hues: impl IntoIterator<Item = u16>) -> eyre::Result<EcHueTables> {
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

/// The frames drawn in order on a canvas just big enough for them all: width, height and RGBA.
pub fn compose(frames: &[PlacedFrame]) -> (u32, u32, Vec<u8>) {
    let min_x = frames.iter().map(|frame| frame.x).min().unwrap_or(0);
    let min_y = frames.iter().map(|frame| frame.y).min().unwrap_or(0);
    let max_x = frames.iter().map(|frame| frame.x + frame.width as i32).max().unwrap_or(0);
    let max_y = frames.iter().map(|frame| frame.y + frame.height as i32).max().unwrap_or(0);
    let width = (max_x - min_x).max(1) as u32;
    let height = (max_y - min_y).max(1) as u32;
    (width, height, compose_on(width, height, -min_x, -min_y, frames))
}

/// The frames drawn in order on a `width` x `height` canvas whose anchor is at `(anchor_x, anchor_y)`: RGBA. A frame
/// texel outside the canvas is not drawn.
pub fn compose_on(width: u32, height: u32, anchor_x: i32, anchor_y: i32, frames: &[PlacedFrame]) -> Vec<u8> {
    let mut canvas = vec![0u8; width as usize * height as usize * RGBA_BYTES];
    for frame in frames {
        alpha_blend_at(&mut canvas, width, frame.x + anchor_x, frame.y + anchor_y, frame.width, frame.height, &frame.rgba);
    }
    canvas
}

/// Blends a straight-alpha `layer` over `canvas` at `(dst_x, dst_y)`: an opaque texel is copied, a partly
/// transparent one blended over what is there, so over a transparent canvas the colour comes out premultiplied.
/// Texels left of or above the canvas, or past its end, are not drawn.
pub fn alpha_blend_at(
    canvas: &mut [u8],
    canvas_width: u32,
    dst_x: i32,
    dst_y: i32,
    layer_width: u32,
    layer_height: u32,
    layer: &[u8],
) {
    for y in 0..layer_height as usize {
        for x in 0..layer_width as usize {
            let canvas_x = x as i32 + dst_x;
            let canvas_y = y as i32 + dst_y;
            if canvas_x < 0 || canvas_y < 0 {
                continue;
            }

            let src_i = (y * layer_width as usize + x) * RGBA_BYTES;
            let src_a = u16::from(layer[src_i + ALPHA_BYTE]);
            if src_a == 0 {
                continue;
            }

            let dst_i = (canvas_y as usize * canvas_width as usize + canvas_x as usize) * RGBA_BYTES;
            if dst_i + RGBA_BYTES > canvas.len() {
                continue;
            }

            if src_a == OPAQUE {
                canvas[dst_i..dst_i + RGBA_BYTES].copy_from_slice(&layer[src_i..src_i + RGBA_BYTES]);
                continue;
            }

            let inv_a = OPAQUE - src_a;
            for channel in 0..ALPHA_BYTE {
                canvas[dst_i + channel] =
                    ((u16::from(layer[src_i + channel]) * src_a + u16::from(canvas[dst_i + channel]) * inv_a) / OPAQUE)
                        as u8;
            }
            canvas[dst_i + ALPHA_BYTE] = (src_a + (u16::from(canvas[dst_i + ALPHA_BYTE]) * inv_a) / OPAQUE) as u8;
        }
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

impl Override {
    /// It applies to a wearer of `race` wearing `worn`.
    fn applies(&self, worn: &[Option<u32>; LAYER_COUNT], race: u32) -> bool {
        let fits = self.paperdoll && self.race.is_none_or(|required| required == race);
        let worn_condition = self.conditions.is_empty()
            || self.conditions.iter().any(|&(other, wanted)| match wanted {
                Some(wanted) => worn[other] == Some(wanted),
                None => worn[other].is_some(),
            });
        fits && worn_condition
    }
}

/// The paperdoll draw order of `wearables.xml` (`WearableRules.kdl`): the priorities of the paperdoll `draworder`
/// and the `wearable` overrides, keyed by (layer, item).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WearableRules {
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

/// The layer of a slot name.
pub fn slot_layer(slot: &str) -> eyre::Result<usize> {
    SLOT_NAMES
        .iter()
        .position(|name| *name == slot)
        .ok_or_else(|| eyre::eyre!("{WEARABLE_RULES_FILE_NAME}: unknown slot `{slot}`; expected one of {SLOT_NAMES:?}"))
}

/// A decimal or `0x` hexadecimal item id.
fn parse_item_id(text: &str) -> Option<u32> {
    const HEX_RADIX: u32 = 16;
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, HEX_RADIX).ok(),
        None => text.parse().ok(),
    }
}

fn item_keys(items: &str) -> eyre::Result<Vec<ItemKey>> {
    if items.trim().eq_ignore_ascii_case(ANY_ITEM) {
        return Ok(vec![None]);
    }
    items
        .split(',')
        .map(|part| {
            parse_item_id(part)
                .map(Some)
                .ok_or_else(|| eyre::eyre!("{WEARABLE_RULES_FILE_NAME}: `{}` is not an item id or `any`", part.trim()))
        })
        .collect()
}

impl WearableRules {
    pub fn decode(text: &str) -> eyre::Result<Self> {
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

    /// The priority of a slot in the paperdoll `draworder`, before any override.
    pub fn base_priority(&self, layer: usize) -> i32 {
        self.priorities[layer]
    }

    /// `BuildEquipmentLayerOrder` (0x00468810, the +0x160 order) and the paperdoll's layer list (0x0050b690): the
    /// worn layers in drawing order. Each worn layer takes the rule of its item, else the rule for any item; the last
    /// of its overrides whose race fits and one of whose conditions is worn sets its priority; a priority below 1
    /// hides the layer. `worn` holds the item id of each layer.
    pub fn paperdoll_order(&self, worn: &[Option<u32>; LAYER_COUNT], race: u32) -> Vec<usize> {
        let mut priorities = self.priorities;
        for (layer, item) in worn.iter().enumerate() {
            let Some(item) = *item else { continue };
            let Some(overrides) = self.overrides.get(&(layer, Some(item))).or_else(|| self.overrides.get(&(layer, None)))
            else {
                continue;
            };
            for rule in overrides {
                if rule.applies(worn, race) {
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
    use crate::enhanced::animationframe::AnimationBounds;
    use crate::enhanced::tileart::TaeProp;

    const HUMAN_MALE: u32 = 400;
    const HUMAN_FEMALE: u32 = 401;
    const FEMALE: u32 = 1;
    const ROBE: u32 = 7939;
    const LEGS: u32 = 5137;
    const SHIRT: u32 = 5399;
    const ELF_RACE: u32 = 2;
    /// `wearables.xml` of `MainMisc.uop`, as dynamapper reads it.
    const WEARABLE_RULES_KDL: &str =
        include_str!("../../../../../dynamapper/assets/runtime_specs/paperdoll/WearableRules.kdl");

    fn definition(tile_id: u32, properties: &[(PropertyKey, u32)]) -> TileArtEntry {
        TileArtEntry {
            tile_id,
            prop_vector1: properties.iter().map(|&(key, val)| TaeProp { id: key as u8, val }).collect(),
            ..TileArtEntry::default()
        }
    }

    fn entries(mut definitions: Vec<TileArtEntry>) -> Vec<TileArtEntry> {
        definitions.sort_by_key(|definition| definition.tile_id);
        definitions
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
        let definitions = SortedTileArt(&files);
        let female = body_frame(&sets, &Wearer::new(definitions, HUMAN_FEMALE)).unwrap();
        assert_eq!((female.0.set, female.0.entry.frame_number, female.1), (FEMALE_SET, 401, false));
        let unknown = body_frame(&sets, &Wearer::new(definitions, 999)).unwrap();
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
        let definitions = SortedTileArt(&files);
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
        let definitions = SortedTileArt(&files);
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
        let rules = WearableRules::decode(WEARABLE_RULES_KDL).unwrap();
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

    /// An opaque texel is copied, a half-transparent one is blended over the canvas (premultiplied over a transparent
    /// one), a transparent one adds nothing, and a texel off the canvas is dropped.
    #[test]
    fn frames_blend_by_alpha_at_their_anchor() {
        const RED: [u8; 4] = [200, 0, 0, 255];
        const HALF_WHITE: [u8; 4] = [255, 255, 255, 128];
        const PREMULTIPLIED_HALF_WHITE: [u8; 4] = [128, 128, 128, 128];
        const TRANSPARENT_PINK: [u8; 4] = [255, 0, 255, 0];
        let frames = [
            PlacedFrame { x: -1, y: 0, width: 2, height: 1, rgba: [RED, RED].concat() },
            PlacedFrame { x: -2, y: 0, width: 3, height: 1, rgba: [RED, HALF_WHITE, TRANSPARENT_PINK].concat() },
        ];
        let canvas = compose_on(2, 1, 1, 0, &frames);
        assert_eq!(canvas, [[227, 128, 128, 255], RED].concat());
        let (width, height, union) = compose(&frames[1..]);
        assert_eq!((width, height), (3, 1));
        assert_eq!(union, [RED, PREMULTIPLIED_HALF_WHITE, [0; 4]].concat());
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

    /// The file `name` of `dir`, whatever the case of its name.
    fn client_file(dir: &Path, name: &str) -> std::path::PathBuf {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().eq_ignore_ascii_case(name))
            .unwrap_or_else(|| panic!("no {name} in {}", dir.display()))
            .path()
    }

    fn real_client() -> (std::path::PathBuf, NamedTileArt, FrameSets) {
        let dir = std::path::PathBuf::from(std::env::var_os(EC_CLIENT_DIR_ENV).expect("set UO_EC_CLIENT_DIR"));
        let package = UopPackage::load(client_file(&dir, TILEART_UOP_NAME)).unwrap();
        let named = NamedTileArt::read(&package);
        let sets = FrameSets::load(&client_file(&dir, PAPERDOLL_UOP_NAME)).unwrap();
        (dir, named, sets)
    }

    /// Every record of a shipped `tileart.uop` is named after an id the paperdoll looks up, and the records that
    /// store another id are found by their name.
    #[test]
    #[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
    fn shipped_records_are_found_by_their_entry_name() {
        // EC 2024: entry 42091 stores the tile id 41516, which entry 41516 stores too.
        const RENAMED_ENTRY: u32 = 42091;
        const RENAMED_STORED_ID: u32 = 41516;
        let (_, named, _) = real_client();
        assert_eq!((named.unreadable, named.unnamed), (0, 0), "{:?}", named.renamed);
        assert!(named.renamed.contains(&(RENAMED_ENTRY, RENAMED_STORED_ID)), "{:?}", named.renamed);
        assert_eq!((&named).get(RENAMED_ENTRY).map(|record| record.tile_id), Some(RENAMED_STORED_ID));
        assert_eq!((&named).get(RENAMED_STORED_ID).map(|record| record.tile_id), Some(RENAMED_STORED_ID));
    }

    #[test]
    #[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
    fn shipped_bodies_and_wearables_find_their_frames() {
        let (_, named, sets) = real_client();
        let definitions = &named;
        let male = Wearer::new(definitions, HUMAN_MALE);
        let female = Wearer::new(definitions, HUMAN_FEMALE);
        assert_eq!((male.gender, female.gender), (DEFAULT_GENDER, FEMALE));
        let body = |wearer: &Wearer<&TileArtEntry>| {
            body_frame(&sets, wearer).map(|(frame, fell_back)| (frame.set, frame.entry.frame_number, fell_back))
        };
        assert_eq!(body(&male), Some((MALE_SET, HUMAN_MALE as u16, false)));
        assert_eq!(body(&female), Some((FEMALE_SET, HUMAN_FEMALE as u16, false)));

        let wearables: Vec<u32> = named
            .records
            .iter()
            .map(|&(entry_id, _)| entry_id)
            .filter(|&entry_id| entry_id < MOBILE_DEFINITION_BASE)
            .filter(|&entry_id| item_layer(definitions, entry_id, None).is_some_and(|layer| !SKIPPED_LAYERS.contains(&layer)))
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
        let (dir, named, sets) = real_client();
        let definitions = &named;
        let hues_path = client_file(&dir, crate::enhanced::hues::HUES_UOP_NAME);
        let hues = hue_tables(&UopPackage::load_with_mode(&hues_path, LoadMode::Lazy).unwrap(), [SKIN_HUE, DYED_HUE]).unwrap();
        assert_eq!(hues.tables.len(), 2);
        let outfit: Vec<WornItem> = [SHIRT, PANTS, SHOES, PLATE_CHEST, CLOAK, LONG_HAIR, KATANA]
            .into_iter()
            .map(|item_id| WornItem { item_id, hue: if item_id == CLOAK { DYED_HUE } else { 0 }, tiledata_layer: None })
            .collect();
        let rules = WearableRules::decode(WEARABLE_RULES_KDL).unwrap();
        let paperdoll = build(&sets, definitions, &rules, &hues, HUMAN_FEMALE, SKIN_HUE, &outfit).unwrap();
        // The plate chest hides the shirt (`torso` at 0 under any `chest`); everything else is drawn.
        assert_eq!(paperdoll.frames.len(), outfit.len(), "{:?}", paperdoll.notes);
        assert_eq!(paperdoll.notes.len(), 1, "{:?}", paperdoll.notes);
        let (width, height, rgba) = compose(&paperdoll.frames);
        let body = &paperdoll.frames[0];
        assert!(width >= body.width && height >= body.height);
        let path = std::env::temp_dir().join(format!("uocf-{}-ec_paperdoll.png", std::process::id()));
        assert!(path.is_absolute(), "{}", path.display());
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
