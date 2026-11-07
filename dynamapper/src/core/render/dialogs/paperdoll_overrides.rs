//! What Dynamapper's paperdolls take off beneath a robe. The client draws the shirt, trousers, skirt, sleeves and chest
//! pieces beneath a robe, and their 3D frames do not line up with the robe's: their edges show past it. The packer
//! measured, for each dress-layer item on each player body, the slots whose garments show only at its edges, and
//! stored them in paperdoll_ec.uddp (`udd_assets::paperdoll_ec::PaperdollEcDressHides`, measured by
//! `udd_conv::paperdoll_ec_dress`). `PaperdollOverrides.kdl` is the user's manual layer over those decisions: a cover
//! replaces the stored decision for its items. Only what the client draws beneath the covering item is taken off; a
//! taken-off item counts as not worn, so the client's rules it would set off, as plate legs hiding the shoes, do not
//! apply. The gump paperdolls apply them too, on the frame the 3D paperdoll draws for the profile's body: the gump art
//! shows the armour's edges past a robe as well, and the decisions take off no garment the gumps show in plain view
//! (docs/agent_notes/ec_paperdoll_robe_override.md). The uocf editor keeps the client's rules.

use color_eyre::eyre;
use knuffel::Decode;
use uocf::enhanced::paperdoll::{
    plan, slot_layer, PaperdollDefinitions, PaperdollFrames, PaperdollPlan, WearableRules, WornItem, LAYER_COUNT,
    SLOT_NAMES,
};

pub const PAPERDOLL_OVERRIDES_FILE_NAME: &str = "PaperdollOverrides.kdl";
const ITEMS_PROPERTY: &str = "items";
const ANY_ITEM: &str = "any";

/// `cover "<slot>" items="any"|"<ids>" { hide "<slot>"; except items="<ids>" }`, decoded by `OverridesKdl`.
pub const PAPERDOLL_OVERRIDES_SCHEMA: &[uot_kdl::strict::NodeSchema] = &[uot_kdl::strict::NodeSchema {
    name: "cover",
    properties: &[ITEMS_PROPERTY],
    children: &[uot_kdl::strict::NodeSchema::leaf("hide", &[]), uot_kdl::strict::NodeSchema::leaf("except", &[ITEMS_PROPERTY])],
}];

#[derive(Decode)]
struct OverridesKdl {
    #[knuffel(children(name = "cover"))]
    covers: Vec<CoverKdl>,
}

#[derive(Decode)]
struct CoverKdl {
    #[knuffel(argument)]
    slot: String,
    #[knuffel(property)]
    items: String,
    #[knuffel(children(name = "hide"))]
    hides: Vec<HideKdl>,
    #[knuffel(children(name = "except"))]
    excepts: Vec<ExceptKdl>,
}

#[derive(Decode)]
struct HideKdl {
    #[knuffel(argument)]
    slot: String,
}

#[derive(Decode)]
struct ExceptKdl {
    #[knuffel(property)]
    items: String,
}

/// While an item of `layer` among `items` is drawn, the `hidden` layers drawn beneath it are taken off.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cover {
    layer: usize,
    /// The covering items; `None` is any.
    items: Option<Vec<u32>>,
    /// Bit n set: layer n is taken off.
    hidden: u32,
    /// Covering items the cover leaves alone.
    except: Vec<u32>,
}

impl Cover {
    fn covers_with(&self, item_id: u32) -> bool {
        self.items.as_ref().is_none_or(|items| items.contains(&item_id)) && !self.except.contains(&item_id)
    }
}

/// The user's paperdoll overrides. The default has none: the stored decisions alone.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PaperdollOverrides {
    covers: Vec<Cover>,
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

/// A comma-separated list of item ids.
fn parse_item_ids(source_name: &str, items: &str) -> eyre::Result<Vec<u32>> {
    items
        .split(',')
        .map(|part| {
            parse_item_id(part).ok_or_else(|| {
                eyre::eyre!(
                    "{source_name}: items: `{}` is not an item id; expected ids like 0x1F03,7939",
                    part.trim()
                )
            })
        })
        .collect()
}

fn layer(source_name: &str, slot: &str) -> eyre::Result<usize> {
    slot_layer(slot).map_err(|_| eyre::eyre!("{source_name}: unknown slot `{slot}`; expected one of {SLOT_NAMES:?}"))
}

/// The mask with bit n set for each layer n of `layers`.
pub fn layer_mask(layers: impl IntoIterator<Item = usize>) -> u32 {
    layers.into_iter().filter(|&layer| layer < LAYER_COUNT).fold(0, |mask, layer| mask | 1 << layer)
}

impl PaperdollOverrides {
    pub fn decode(source_name: &str, text: &str) -> eyre::Result<Self> {
        let decoded: OverridesKdl = uot_kdl::strict::decode(source_name, text, PAPERDOLL_OVERRIDES_SCHEMA)?;
        let mut covers = Vec::with_capacity(decoded.covers.len());
        for cover in decoded.covers {
            let cover_layer = layer(source_name, &cover.slot)?;
            let items = if cover.items.trim().eq_ignore_ascii_case(ANY_ITEM) {
                None
            } else {
                Some(parse_item_ids(source_name, &cover.items)?)
            };
            let mut hidden = Vec::with_capacity(cover.hides.len());
            for hide in &cover.hides {
                let hidden_layer = layer(source_name, &hide.slot)?;
                eyre::ensure!(hidden_layer != cover_layer, "{source_name}: `cover \"{}\"` hides its own slot", cover.slot);
                hidden.push(hidden_layer);
            }
            let mut except = Vec::new();
            for node in &cover.excepts {
                except.extend(parse_item_ids(source_name, &node.items)?);
            }
            covers.push(Cover { layer: cover_layer, items, hidden: layer_mask(hidden), except });
        }
        Ok(Self { covers })
    }

    /// The layers item `item_id`, worn on `layer`, takes off beneath it by the last cover that applies to it (bit n:
    /// layer n); `None` when none does, and the stored decision holds. An item a cover excepts falls back to the covers
    /// before it, then to the stored decision; a cover that hides nothing keeps everything beneath its items.
    pub fn hidden_layers(&self, layer: usize, item_id: u32) -> Option<u32> {
        self.covers.iter().rev().find(|cover| cover.layer == layer && cover.covers_with(item_id)).map(|cover| cover.hidden)
    }
}

/// The draw order of Dynamapper's paperdolls: the client's rules (`WearableRules.kdl`, shared with the uocf editor),
/// then the user's overrides over the decisions paperdoll_ec.uddp stores.
#[derive(Debug, Default, Clone)]
pub struct PaperdollOrder {
    pub rules: WearableRules,
    pub overrides: PaperdollOverrides,
}

impl PaperdollOrder {
    /// The layers the drawn item `item_id` of `layer` takes off beneath it: the overrides' cover, else `stored`, the
    /// decision the package stores for it on the drawn body (bit n: layer n).
    pub fn hidden_layers(&self, layer: usize, item_id: u32, stored: impl FnOnce() -> Option<u32>) -> u32 {
        self.overrides.hidden_layers(layer, item_id).or_else(stored).unwrap_or(0)
    }
}

/// The items taken off of `drawn`, the (layer, item id) pairs the client draws, bottom first: each item drawn beneath
/// one whose `hides` (its layer and id) has the bit of its layer.
pub fn taken_off(drawn: &[(usize, u32)], hides: impl Fn(usize, u32) -> u32) -> Vec<u32> {
    let mut taken = Vec::new();
    for (position, &(layer, item_id)) in drawn.iter().enumerate() {
        let hidden = hides(layer, item_id);
        if hidden == 0 {
            continue;
        }
        for &(beneath, beneath_item) in &drawn[..position] {
            if hidden & 1 << beneath != 0 && !taken.contains(&beneath_item) {
                taken.push(beneath_item);
            }
        }
    }
    taken
}

/// What the client draws for `body` wearing `items` ([`plan`] with `order`'s rules), without what is taken off: the
/// items [`taken_off`] by [`PaperdollOrder::hidden_layers`], where `stored` gives the package's decision for the body
/// frame drawn and an item, are left out and the rest planned again, until none is.
pub fn plan_taking_off<'a, S: PaperdollFrames, T: PaperdollDefinitions<'a>>(
    sets: &S,
    definitions: T,
    order: &PaperdollOrder,
    body: u32,
    body_hue: u16,
    items: &[WornItem],
    stored: impl Fn(&S::Frame, u32) -> Option<u32>,
) -> eyre::Result<PaperdollPlan<S::Frame>> {
    let mut worn = items.to_vec();
    loop {
        let planned = plan(sets, definitions, &order.rules, body, body_hue, &worn)?;
        // The body's frame comes first.
        let body_frame = planned.frames.first().filter(|frame| frame.item.is_none()).map(|frame| frame.frame.entry);
        let drawn: Vec<(usize, u32)> = planned.frames.iter().filter_map(|frame| frame.item).collect();
        let taken = taken_off(&drawn, |layer, item_id| {
            order.hidden_layers(layer, item_id, || body_frame.as_ref().and_then(|frame| stored(frame, item_id)))
        });
        if taken.is_empty() {
            return Ok(planned);
        }
        // Each pass leaves out one item at least, so the loop ends.
        worn.retain(|item| !taken.contains(&item.item_id));
    }
}

#[cfg(test)]
mod measure;

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::animationframe::FrameEntry;
    use uocf::enhanced::paperdoll::{FrameSets, SortedTileArt, MALE_SET};
    use uocf::enhanced::tileart::TileArtEntry;

    const BODY: u32 = 400;
    const ROBE: u32 = 0x1F03;
    const EPAULETS: u32 = 39301;
    const SHIRT: u32 = 0x1517;
    const SASH: u32 = 5441;
    const PLATE_LEGS: u32 = 0x1411;
    const SHOES: u32 = 0x170F;
    const BUNDLED_OVERRIDES: &str = include_str!("../../../../assets/runtime_specs/paperdoll/PaperdollOverrides.kdl");
    const BUNDLED_RULES: &str = include_str!("../../../../assets/runtime_specs/paperdoll/WearableRules.kdl");

    fn overrides(content: &str) -> PaperdollOverrides {
        PaperdollOverrides::decode(PAPERDOLL_OVERRIDES_FILE_NAME, content).unwrap()
    }

    fn overrides_error(content: &str) -> String {
        format!("{:#}", PaperdollOverrides::decode(PAPERDOLL_OVERRIDES_FILE_NAME, content).unwrap_err())
    }

    fn slot(name: &str) -> usize {
        slot_layer(name).unwrap()
    }

    fn mask(slots: &[&str]) -> u32 {
        layer_mask(slots.iter().map(|name| slot(name)))
    }

    /// The bundled file holds comments only: the stored decisions apply as measured.
    #[test]
    fn the_bundled_overrides_leave_the_stored_decisions() {
        assert_eq!(overrides(BUNDLED_OVERRIDES), PaperdollOverrides::default());
    }

    /// The last cover that applies to an item replaces the stored decision; an exception falls back to the covers
    /// before it, then to the stored decision; a cover without hides keeps everything.
    #[test]
    fn the_last_cover_that_applies_replaces_the_stored_decision() {
        let order = PaperdollOrder {
            rules: WearableRules::default(),
            overrides: overrides(
                "cover \"dress\" items=\"any\" {\n    hide \"torso\"\n    except items=\"39301\"\n}\n\
                 cover \"dress\" items=\"0x1F03\" {\n    hide \"legs\"\n}\n\
                 cover \"dress\" items=\"0x1F04\"\n",
            ),
        };
        let dress = slot("dress");
        let stored = || Some(mask(&["torso", "legs", "arms"]));
        assert_eq!(order.hidden_layers(dress, ROBE, stored), mask(&["legs"]));
        assert_eq!(order.hidden_layers(dress, ROBE + 1, stored), 0, "a cover without hides");
        assert_eq!(order.hidden_layers(dress, SASH, stored), mask(&["torso"]), "the first cover");
        assert_eq!(order.hidden_layers(dress, EPAULETS, stored), mask(&["torso", "legs", "arms"]), "an exception");
        assert_eq!(order.hidden_layers(dress, EPAULETS, || None), 0, "nothing stored");
        assert_eq!(order.hidden_layers(slot("cape"), ROBE, stored), mask(&["torso", "legs", "arms"]), "another slot");
        assert_eq!(PaperdollOrder::default().hidden_layers(dress, ROBE, stored), mask(&["torso", "legs", "arms"]));
    }

    /// Only what is drawn beneath the covering item is taken off, once.
    #[test]
    fn only_what_is_drawn_beneath_is_taken_off() {
        const PANTS: u32 = 0x1539;
        let [torso, legs, dress, abovechest] = ["torso", "legs", "dress", "abovechest"].map(slot);
        let hides = |layer: usize, _item| if layer == dress { mask(&["torso", "legs", "abovechest"]) } else { 0 };
        let drawn = [(legs, PANTS), (torso, SHIRT), (dress, ROBE), (abovechest, SASH)];
        assert_eq!(taken_off(&drawn, hides), [PANTS, SHIRT], "the sash is drawn over the robe");
        assert_eq!(taken_off(&drawn[..2], hides), Vec::<u32>::new(), "no robe");
        let both = |layer: usize, _item| if layer == torso || layer == dress { mask(&["legs"]) } else { 0 };
        assert_eq!(taken_off(&drawn, both), [PANTS]);
    }

    /// A body, a robe, a shirt, plate legs and shoes, each a small frame numbered as its item, and their tileart
    /// layers.
    fn robe_outfit() -> (FrameSets, Vec<TileArtEntry>) {
        use uocf::enhanced::animationframe::{AmouFile, AnimationFrame, IndexedFrame, COVERAGE_SOLID};
        use uocf::enhanced::tileart::{PropertyKey, TaeProp};
        const PALETTE: [[u8; 4]; 2] = [[0, 0, 0, 0], [40, 80, 120, 0]];
        const SOLID_INDEX: u8 = 1;
        const SIZE: u16 = 4;
        let solid = || IndexedFrame {
            width: SIZE,
            height: SIZE,
            center_x: 0,
            center_y: 0,
            indices: vec![SOLID_INDEX; usize::from(SIZE * SIZE)],
            coverage: vec![COVERAGE_SOLID; usize::from(SIZE * SIZE)],
        };
        let action = MALE_SET as u16;
        let frames: Vec<(u16, u16, IndexedFrame)> =
            [BODY, ROBE, SHIRT, PLATE_LEGS, SHOES].iter().map(|&number| (action, number as u16, solid())).collect();
        let load = |file: AmouFile| AnimationFrame::load(&file.to_bytes().unwrap()).unwrap();
        let male = AmouFile::from_indexed_frames(BODY, PALETTE.to_vec(), &frames).unwrap();
        let empty = AmouFile::from_indexed_frames(BODY, PALETTE.to_vec(), &[]).unwrap();
        let worn = |item: u32, name: &str| TileArtEntry {
            tile_id: item,
            prop_vector1: vec![TaeProp { id: PropertyKey::Layer as u8, val: slot(name) as u32 }],
            ..TileArtEntry::default()
        };
        let mut records =
            vec![worn(ROBE, "dress"), worn(SHIRT, "torso"), worn(PLATE_LEGS, "legs"), worn(SHOES, "feet")];
        records.sort_by_key(|record| record.tile_id);
        (FrameSets::from_sets([load(male), load(empty)]), records)
    }

    /// Under a robe the client draws the shirt and plate legs, and hides the shoes under the plate legs. Taken off by
    /// the robe's stored decision, the plate legs no longer hide the shoes, which show under the robe. A cover with no
    /// hides keeps what the client draws.
    #[test]
    fn what_a_robe_takes_off_no_longer_hides_the_shoes() {
        let (sets, records) = robe_outfit();
        let rules = WearableRules::decode(BUNDLED_RULES).unwrap();
        let items: Vec<WornItem> = [SHIRT, PLATE_LEGS, SHOES, ROBE]
            .iter()
            .map(|&item_id| WornItem { item_id, hue: 0, tiledata_layer: None })
            .collect();
        let stored = |frame: &FrameEntry, item_id: u32| {
            (u32::from(frame.frame_number) == BODY && item_id == ROBE).then(|| mask(&["torso", "legs"]))
        };
        let drawn = |order: &PaperdollOrder| -> Vec<(usize, u32)> {
            plan_taking_off(&sets, SortedTileArt(&records), order, BODY, 0, &items, stored)
                .unwrap()
                .frames
                .iter()
                .filter_map(|frame| frame.item)
                .collect()
        };
        let [torso, legs, feet, dress] = ["torso", "legs", "feet", "dress"].map(slot);
        let order = PaperdollOrder { rules, overrides: PaperdollOverrides::default() };
        assert_eq!(drawn(&order), [(feet, SHOES), (dress, ROBE)]);
        let keep = PaperdollOrder { overrides: overrides("cover \"dress\" items=\"0x1F03\"\n"), ..order };
        assert_eq!(drawn(&keep), [(torso, SHIRT), (legs, PLATE_LEGS), (dress, ROBE)]);
    }

    #[test]
    fn overrides_reject_unknown_nodes_slots_and_items() {
        assert_eq!(
            overrides_error("covers \"dress\" items=\"any\"\n"),
            "PaperdollOverrides.kdl: line 1, column 1: unknown node `covers` at the top level: expected `cover`"
        );
        assert_eq!(
            overrides_error("cover \"dress\" items=\"any\" {\n    hide \"torso\" when=\"always\"\n}\n"),
            "PaperdollOverrides.kdl: line 2, column 18: `hide` takes no properties, found `when`"
        );
        assert_eq!(
            overrides_error("cover \"dress\" items=\"any\" {\n    show \"torso\"\n}\n"),
            "PaperdollOverrides.kdl: line 2, column 5: unknown node `show` in `cover`: expected `hide` or `except`"
        );
        assert!(overrides_error("cover \"robe\" items=\"any\"\n").contains("unknown slot `robe`"));
        assert!(overrides_error("cover \"dress\" items=\"any\" {\n    hide \"shirt\"\n}\n").contains("unknown slot `shirt`"));
        assert!(overrides_error("cover \"dress\" items=\"any\" {\n    hide \"dress\"\n}\n").contains("hides its own slot"));
        assert!(overrides_error("cover \"dress\" items=\"robes\"\n").contains("`robes` is not an item id"));
        assert!(overrides_error("cover \"dress\" items=\"any\" {\n    except items=\"any\"\n}\n").contains("`any` is not an item id"));
        assert!(overrides_error("cover \"dress\"\n").contains(ITEMS_PROPERTY));
    }
}
