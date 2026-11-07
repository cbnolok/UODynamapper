//! What Dynamapper's paperdolls take off beneath a robe. The client draws the shirt, trousers, skirt, sleeves and chest
//! pieces beneath a robe, and their 3D frames do not line up with the robe's: their edges show past it. The packer
//! measured, for each dress-layer item on each player body, each garment of those slots the client draws beneath it,
//! and stored the garments that show beyond its edges in paperdoll_ec.uddp
//! (`udd_assets::paperdoll_ec::PaperdollEcDressDecisions`, measured by `udd_conv::paperdoll_ec_dress`): every other
//! garment of those slots drawn beneath the item is taken off. `PaperdollOverrides.kdl`
//! (`udd_assets::paperdoll_overrides`) is the user's manual layer over those decisions: a cover's garment rules beat
//! its slot rules, which beat the stored decision for its items. Only what the client draws beneath the
//! covering item is taken off; a taken-off item counts as not worn, so the client's rules it would set off, as plate
//! legs hiding the shoes, do not apply. The gump paperdolls apply them too, on the frame the 3D paperdoll draws for the profile's body: the gump art
//! shows the armour's edges past a robe as well, and the decisions take off no garment the gumps show in plain view
//! (docs/agent_notes/ec_paperdoll_robe_override.md). The uocf editor keeps the client's rules.

use color_eyre::eyre;
use udd_assets::paperdoll_ec::PaperdollEcFrameRecord;
pub use udd_assets::paperdoll_overrides::{PaperdollOverrides, PAPERDOLL_OVERRIDES_FILE_NAME};
use uocf::enhanced::animationframe::FrameEntry;
use uocf::enhanced::paperdoll::{
    item_frame, item_layer, plan, PaperdollDefinitions, PaperdollFrames, PaperdollPlan, WearableRules, Wearer, WornItem,
};

/// The draw order of Dynamapper's paperdolls: the client's rules (`WearableRules.kdl`, shared with the uocf editor),
/// then the user's overrides over the decisions paperdoll_ec.uddp stores.
#[derive(Debug, Default, Clone)]
pub struct PaperdollOrder {
    pub rules: WearableRules,
    pub overrides: PaperdollOverrides,
}

impl PaperdollOrder {
    /// Whether the drawn item `covering` takes off `garment`, drawn beneath it (each its layer and id), on the body
    /// frame `body_frame` (its number; `None` without one): the overrides' decision ([`PaperdollOverrides::decide`]),
    /// else `stored`, the decision the package stores for it on the drawn body.
    pub fn takes_off(
        &self,
        covering: (usize, u32),
        garment: (usize, u32),
        body_frame: Option<u32>,
        stored: impl FnOnce() -> bool,
    ) -> bool {
        match self.overrides.decide(covering, body_frame, garment) {
            Some(decision) => decision.takes_off,
            None => stored(),
        }
    }
}

/// A garment taken off beneath a covering item: each its layer and id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TakenOff {
    pub garment: (usize, u32),
    pub covering: (usize, u32),
}

/// The items taken off of `drawn`, the (layer, item id) pairs the client draws, bottom first: each item drawn beneath
/// one that `takes_off` it (the covering item, then the item beneath), once, with the first item over it that does.
pub fn taken_off(drawn: &[(usize, u32)], takes_off: impl Fn((usize, u32), (usize, u32)) -> bool) -> Vec<TakenOff> {
    let mut taken: Vec<TakenOff> = Vec::new();
    for (position, &covering) in drawn.iter().enumerate() {
        for &garment in &drawn[..position] {
            if !taken.iter().any(|taken| taken.garment == garment) && takes_off(covering, garment) {
                taken.push(TakenOff { garment, covering });
            }
        }
    }
    taken
}

/// The number of a paperdoll frame: the stored decisions name a body by the number of the frame drawn for it.
pub trait FrameNumber {
    fn frame_number(&self) -> u16;
}

impl FrameNumber for FrameEntry {
    fn frame_number(&self) -> u16 {
        self.frame_number
    }
}

impl FrameNumber for PaperdollEcFrameRecord {
    fn frame_number(&self) -> u16 {
        self.frame_number
    }
}

/// Whether the packer measures `garment` beneath the dress-layer items on `body` (`udd_conv::paperdoll_ec_dress`): it
/// has a `tileart.uop` Layer and a 3D frame for the body. A stored decision speaks for those garments alone; the
/// paperdolls leave the others on, as the client draws them (an item worn on its tiledata layer, or whose gump has no
/// 3D frame for the body).
pub fn measured_garment<'a, S, T>(sets: &S, definitions: T, body: u32, garment: u32) -> bool
where
    S: PaperdollFrames,
    T: PaperdollDefinitions<'a>,
{
    item_layer(definitions, garment, None).is_some()
        && item_frame(sets, definitions, &Wearer::new(definitions, body), garment).is_ok()
}

/// What the client draws for `body` wearing `items` ([`plan`] with `order`'s rules), without what is taken off: the
/// items [`taken_off`] by [`PaperdollOrder::takes_off`], where `stored` gives the package's decision for the body frame
/// drawn (its number), a covering item and a garment beneath it (its layer and id), are left out and the rest planned
/// again, until none is. Returns the plan and every item taken off, in the order taken.
pub fn plan_taking_off<'a, S, T>(
    sets: &S,
    definitions: T,
    order: &PaperdollOrder,
    body: u32,
    body_hue: u16,
    items: &[WornItem],
    stored: impl Fn(u32, u32, (usize, u32)) -> bool,
) -> eyre::Result<(PaperdollPlan<S::Frame>, Vec<TakenOff>)>
where
    S: PaperdollFrames,
    S::Frame: FrameNumber,
    T: PaperdollDefinitions<'a>,
{
    let mut worn = items.to_vec();
    let mut taken_in_all = Vec::new();
    loop {
        let planned = plan(sets, definitions, &order.rules, body, body_hue, &worn)?;
        // The body's frame comes first.
        let body_frame = planned
            .frames
            .first()
            .filter(|frame| frame.item.is_none())
            .map(|frame| u32::from(frame.frame.entry.frame_number()));
        let drawn: Vec<(usize, u32)> = planned.frames.iter().filter_map(|frame| frame.item).collect();
        let taken = taken_off(&drawn, |covering, garment| {
            order.takes_off(covering, garment, body_frame, || {
                body_frame.is_some_and(|body| stored(body, covering.1, garment))
            })
        });
        if taken.is_empty() {
            return Ok((planned, taken_in_all));
        }
        // Each pass leaves out one item at least, so the loop ends.
        worn.retain(|item| !taken.iter().any(|taken| taken.garment.1 == item.item_id));
        taken_in_all.extend(taken);
    }
}

#[cfg(test)]
mod measure;

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::paperdoll::{slot_layer, FrameSets, SortedTileArt, MALE_SET};
    use uocf::enhanced::tileart::TileArtEntry;

    const BODY: u32 = 400;
    const ROBE: u32 = 0x1F03;
    const SHIRT: u32 = 0x1517;
    const SASH: u32 = 5441;
    const PLATE_LEGS: u32 = 0x1411;
    const SHOES: u32 = 0x170F;
    const BUNDLED_OVERRIDES: &str = include_str!("../../../../assets/runtime_specs/paperdoll/PaperdollOverrides.kdl");
    const BUNDLED_RULES: &str = include_str!("../../../../assets/runtime_specs/paperdoll/WearableRules.kdl");

    fn overrides(content: &str) -> PaperdollOverrides {
        PaperdollOverrides::decode(PAPERDOLL_OVERRIDES_FILE_NAME, content).unwrap()
    }

    fn slot(name: &str) -> usize {
        slot_layer(name).unwrap()
    }

    /// The bundled file holds comments only: the stored decisions apply as measured.
    #[test]
    fn the_bundled_overrides_leave_the_stored_decisions() {
        assert_eq!(overrides(BUNDLED_OVERRIDES), PaperdollOverrides::default());
    }

    /// Whether `order` takes `garment` off beneath `covering` on the dress layer of body frame `BODY`, with `stored`
    /// the stored decision.
    fn takes_off(order: &PaperdollOrder, covering: u32, garment: (&str, u32), stored: bool) -> bool {
        order.takes_off((slot("dress"), covering), (slot(garment.0), garment.1), Some(BODY), || stored)
    }

    /// The overrides decide before the stored decision, which holds where they decide nothing: a garment rule for the
    /// garment, else a slot rule, of the covers that apply on the body drawn.
    #[test]
    fn the_overrides_decide_before_the_stored_decision() {
        let order = PaperdollOrder {
            rules: WearableRules::default(),
            overrides: overrides(
                "cover \"dress\" items=\"0x1F03\" {\n    hide \"legs\"\n    show items=\"0x1411\"\n}\n\
                 cover \"dress\" items=\"5441\" bodies=\"401\" {\n    hide \"torso\"\n}\n",
            ),
        };
        let shirt = ("torso", SHIRT);
        let plate_legs = ("legs", PLATE_LEGS);
        assert!(!takes_off(&order, ROBE, plate_legs, true), "a garment rule");
        assert!(takes_off(&order, ROBE, ("legs", 0x1539), false), "a slot rule");
        assert!(!takes_off(&order, ROBE, shirt, true), "a slot the cover keeps");
        assert!(takes_off(&order, SASH, shirt, true) && !takes_off(&order, SASH, shirt, false), "another body");
        assert!(takes_off(&PaperdollOrder::default(), ROBE, shirt, true));
        assert!(!takes_off(&PaperdollOrder::default(), ROBE, shirt, false));
    }

    /// Only what is drawn beneath the covering item is taken off, once, by the first item over it that takes it off.
    #[test]
    fn only_what_is_drawn_beneath_is_taken_off() {
        const PANTS: u32 = 0x1539;
        let [torso, legs, dress, abovechest] = ["torso", "legs", "dress", "abovechest"].map(slot);
        let robe = |covering: (usize, u32), garment: (usize, u32)| covering.0 == dress && garment.1 != PANTS;
        let drawn = [(legs, PANTS), (torso, SHIRT), (dress, ROBE), (abovechest, SASH)];
        let taken = |garment, covering| TakenOff { garment, covering };
        assert_eq!(taken_off(&drawn, robe), [taken((torso, SHIRT), (dress, ROBE))], "the sash is drawn over the robe");
        assert_eq!(taken_off(&drawn[..2], robe), [], "no robe");
        let both = |covering: (usize, u32), garment: (usize, u32)| covering.0 != legs && garment.0 == legs;
        assert_eq!(taken_off(&drawn, both), [taken((legs, PANTS), (torso, SHIRT))]);
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
    /// the robe's stored decision on the body drawn, the plate legs no longer hide the shoes, which show under the robe.
    /// A cover with no hides keeps what the client draws.
    #[test]
    fn what_a_robe_takes_off_no_longer_hides_the_shoes() {
        let (sets, records) = robe_outfit();
        let rules = WearableRules::decode(BUNDLED_RULES).unwrap();
        let items: Vec<WornItem> = [SHIRT, PLATE_LEGS, SHOES, ROBE]
            .iter()
            .map(|&item_id| WornItem { item_id, hue: 0, tiledata_layer: None })
            .collect();
        let [torso, legs, feet, dress] = ["torso", "legs", "feet", "dress"].map(slot);
        // The robe's decision on the body: its slots are the torso and legs, and it keeps nothing.
        let stored = |body: u32, covering: u32, (layer, _): (usize, u32)| {
            body == BODY && covering == ROBE && (layer == torso || layer == legs)
        };
        let planned = |order: &PaperdollOrder| {
            let (plan, taken) = plan_taking_off(&sets, SortedTileArt(&records), order, BODY, 0, &items, stored).unwrap();
            let drawn: Vec<(usize, u32)> = plan.frames.iter().filter_map(|frame| frame.item).collect();
            (drawn, taken)
        };
        let order = PaperdollOrder { rules, overrides: PaperdollOverrides::default() };
        let taken = |garment| TakenOff { garment, covering: (dress, ROBE) };
        assert_eq!(planned(&order), (vec![(feet, SHOES), (dress, ROBE)], vec![taken((torso, SHIRT)), taken((legs, PLATE_LEGS))]));
        let keep = PaperdollOrder { overrides: overrides("cover \"dress\" items=\"0x1F03\"\n"), ..order };
        assert_eq!(planned(&keep), (vec![(torso, SHIRT), (legs, PLATE_LEGS), (dress, ROBE)], vec![]));
    }

    /// The packer measures a garment with a `tileart.uop` Layer and a frame for the body; the stored decisions speak
    /// for those alone.
    #[test]
    fn a_garment_is_measured_with_a_layer_and_a_frame() {
        use uocf::enhanced::tileart::{PropertyKey, TaeProp};
        let (sets, records) = robe_outfit();
        let measured = |records: &[TileArtEntry], garment| measured_garment(&sets, SortedTileArt(records), BODY, garment);
        assert!(measured(&records, SHIRT));
        // The shirt worn on its tiledata layer: its record has no Layer.
        let mut no_layer = records.clone();
        no_layer.iter_mut().filter(|record| record.tile_id == SHIRT).for_each(|record| record.prop_vector1.clear());
        assert!(!measured(&no_layer, SHIRT));
        // A sash with a Layer and no frame.
        let mut sash = records.clone();
        sash.push(TileArtEntry {
            tile_id: SASH,
            prop_vector1: vec![TaeProp { id: PropertyKey::Layer as u8, val: slot("abovechest") as u32 }],
            ..TileArtEntry::default()
        });
        sash.sort_by_key(|record| record.tile_id);
        assert!(!measured(&sash, SASH));
    }
}
