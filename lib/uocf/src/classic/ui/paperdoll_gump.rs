//! The Classic client's paperdoll gump of a worn item, as `DrawPaperdollGumpLayers` (0x0051a4a0) chooses it: the
//! paperdoll body, the anim (tiledata's, or tileart's appearance), `Equipconv.def`, the female gump and `gump.def`.
//! The rule and its addresses are in `docs/client_research/ghidra-cc-tileart/results/cc_tileart.md` section 3.1.
//!
//! The tileart records are read through [`PaperdollDefinitions`], the client's tables through
//! [`ClassicPaperdollTables`], so tests and dynamapper (over `tilemeta.uddp`) run the same rule.

use crate::classic::tables::def_files::NONE;
use crate::enhanced::paperdoll::{animation_appearance, appearance, PaperdollDefinitions};

/// An item's gump is its anim plus this (`EquipConvGump` 0x005624a0).
pub const GUMP_ITEM_BASE: u32 = 50_000;
/// A woman's gump is the man's plus this, when that gump exists.
pub const GUMP_FEMALE_OFFSET: u32 = 10_000;
/// A `gump.def` target below this takes the female step again.
pub const GUMP_DEF_FEMALE_LIMIT: u32 = 60_000;
/// The `Equipconv.def` gump column that numbers the gump by the converted id.
pub const EQUIPCONV_GUMP_FROM_CONVERTED: i32 = NONE;
/// The `Equipconv.def` gump column that numbers the gump by the anim, as no row does.
pub const EQUIPCONV_GUMP_FROM_ANIM: i32 = 0;

pub const HUMAN_MALE_BODY: u32 = 400;
pub const HUMAN_FEMALE_BODY: u32 = 401;
pub const ELF_MALE_BODY: u32 = 605;
pub const ELF_FEMALE_BODY: u32 = 606;
pub const GARGOYLE_MALE_BODY: u32 = 666;
pub const GARGOYLE_FEMALE_BODY: u32 = 667;
/// The body the paperdoll draws as itself, a man's, and with UOP frames in the shipped `mobtypes.txt`.
pub const BODY_1253: u32 = 1253;
/// Bodies the paperdoll draws as a human man and woman.
const HUMAN_MALE_ALIAS_BODY: u32 = 0x2e8;
const HUMAN_FEMALE_ALIAS_BODY: u32 = 0x2e9;
/// The body the paperdoll draws as a human man or woman by the mobile's female bit.
const GENDERED_HUMAN_BODY: u32 = 987;
/// `GetRace` values the paperdoll maps other bodies by.
pub const RACE_HUMAN: u32 = 1;
pub const RACE_ELF: u32 = 2;
pub const RACE_GARGOYLE: u32 = 3;

/// The body a paperdoll is drawn for, one of seven, and whether it is a woman's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperdollBody {
    pub body: u32,
    pub female: bool,
}

/// The paperdoll body of `body` (after `body.def`): 400, 401, 605, 606, 666, 667 and 1253 are themselves, two aliases
/// and 987 are human, and any other body goes by `race` and `female` (the mobile's race and female bit). `None` for
/// another body of another race.
pub fn paperdoll_body(body: u32, race: u32, female: bool) -> Option<PaperdollBody> {
    let pick = |male, woman| PaperdollBody { body: if female { woman } else { male }, female };
    Some(match body {
        HUMAN_MALE_BODY | HUMAN_MALE_ALIAS_BODY => PaperdollBody { body: HUMAN_MALE_BODY, female: false },
        HUMAN_FEMALE_BODY | HUMAN_FEMALE_ALIAS_BODY => PaperdollBody { body: HUMAN_FEMALE_BODY, female: true },
        ELF_MALE_BODY | GARGOYLE_MALE_BODY | BODY_1253 => PaperdollBody { body, female: false },
        ELF_FEMALE_BODY | GARGOYLE_FEMALE_BODY => PaperdollBody { body, female: true },
        GENDERED_HUMAN_BODY => pick(HUMAN_MALE_BODY, HUMAN_FEMALE_BODY),
        _ => match race {
            RACE_HUMAN => pick(HUMAN_MALE_BODY, HUMAN_FEMALE_BODY),
            RACE_ELF => pick(ELF_MALE_BODY, ELF_FEMALE_BODY),
            RACE_GARGOYLE => pick(GARGOYLE_MALE_BODY, GARGOYLE_FEMALE_BODY),
            _ => return None,
        },
    })
}

/// An `Equipconv.def` row's converted id and gump column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquipConvGump {
    pub converted: i32,
    pub gump: i32,
}

/// A `gump.def` row: the gump drawn for a missing one, and its hue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GumpDefTarget {
    pub target: u32,
    pub hue: u16,
}

/// The Classic client's tables the paperdoll reads.
pub trait ClassicPaperdollTables {
    /// Whether `body` has the `mobtypes.txt` flag 0x10000 (`BodyUsesUopFrames` 0x005978a0).
    fn uses_uop_frames(&self, body: u32) -> bool;
    /// The `Equipconv.def` row the client uses for `equipment` on `body`.
    fn equip_conv(&self, body: u32, equipment: u32) -> Option<EquipConvGump>;
    /// The `gump.def` row of `gump`.
    fn gump_def(&self, gump: u32) -> Option<GumpDefTarget>;
}

/// The anim the paperdoll numbers `item_id`'s gump by on `body`: on a body with UOP frames, tileart's appearance
/// (`GetAppearance`, with the placeholder `10,000,000 + body` as the owner); on another, `tiledata_anim`, unless an
/// animation filter of the item matches the placeholder (`GetAnimationAppearance` not 0), which switches to the
/// appearance. A missing item or placeholder record gives 0 for both values.
pub fn item_anim<'a, S: PaperdollDefinitions<'a>>(
    definitions: S,
    tables: &impl ClassicPaperdollTables,
    body: u32,
    item_id: u32,
    tiledata_anim: u32,
) -> u32 {
    let records = definitions.get(item_id).zip(definitions.get_named(body));
    let tileart = |read: fn(S::Definition, Option<S::Definition>) -> u32| {
        records.map_or(0, |(item, owner)| read(item, Some(owner)))
    };
    if tables.uses_uop_frames(body) || tileart(animation_appearance) != 0 {
        tileart(appearance)
    } else {
        tiledata_anim
    }
}

/// `EquipConvGump` (0x005624a0): the gump column of the `Equipconv.def` row of `anim` on `body` when above 0, the
/// converted id + 50,000 when it is [`EQUIPCONV_GUMP_FROM_CONVERTED`], else (0, another negative value, or no row)
/// the anim + 50,000.
pub fn equip_conv_gump(tables: &impl ClassicPaperdollTables, body: u32, anim: u32) -> u32 {
    let from = |id: u32| id.saturating_add(GUMP_ITEM_BASE);
    match tables.equip_conv(body, anim) {
        Some(row) if row.gump > EQUIPCONV_GUMP_FROM_ANIM => row.gump.unsigned_abs(),
        Some(row) if row.gump == EQUIPCONV_GUMP_FROM_CONVERTED => from(row.converted.unsigned_abs()),
        _ => from(anim),
    }
}

/// The gump and hue of a worn item of anim `anim` and hue `hue` on `body`, `has_gump` saying which gumps exist:
/// [`equip_conv_gump`], plus [`GUMP_FEMALE_OFFSET`] for a woman when that gump exists; a gump that does not exist and
/// has a `gump.def` row becomes its target, with its hue when `hue` is 0, and a target below
/// [`GUMP_DEF_FEMALE_LIMIT`] takes the female step again. The backpack styles of the player's own layer 21 are not
/// drawn by this rule.
pub fn item_gump(
    tables: &impl ClassicPaperdollTables,
    body: PaperdollBody,
    anim: u32,
    hue: u16,
    has_gump: impl Fn(u32) -> bool,
) -> (u32, u16) {
    let female_step = |gump: u32| {
        let female = gump.saturating_add(GUMP_FEMALE_OFFSET);
        if body.female && has_gump(female) { female } else { gump }
    };
    let gump = female_step(equip_conv_gump(tables, body.body, anim));
    if has_gump(gump) {
        return (gump, hue);
    }
    let Some(row) = tables.gump_def(gump) else { return (gump, hue) };
    let hue = if hue == 0 { row.hue } else { hue };
    let target = if row.target < GUMP_DEF_FEMALE_LIMIT { female_step(row.target) } else { row.target };
    (target, hue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::mobile_body::MOBILE_DEFINITION_BASE;
    use crate::enhanced::paperdoll::SortedTileArt;
    use crate::enhanced::tileart::{PropertyKey, TaeAppearanceFilter, TaeBodyAppearance, TaeProp, TileArtEntry};
    use std::collections::{BTreeMap, BTreeSet};

    /// A whip with a man's and a woman's appearance and an animation filter for both, as the shipped 41610 has.
    const WHIP: u32 = 41610;
    const WHIP_TILEDATA_ANIM: u32 = 1486;
    const WHIP_MALE_APPEARANCE: u32 = 1490;
    const WHIP_FEMALE_APPEARANCE: u32 = 1487;
    /// An item with an Appearance property and no animation filter.
    const SHIRT: u32 = 5399;
    const SHIRT_TILEDATA_ANIM: u32 = 434;
    const SHIRT_APPEARANCE: u32 = 4340;
    /// A gargoyle item whose appearance is its gargoyle filter row.
    const GARGOYLE_ITEM: u32 = 16_486;
    const GARGOYLE_ITEM_TILEDATA_ANIM: u32 = 400;
    const GARGOYLE_APPEARANCE: u32 = 1_900;
    const ITEM_HUE: u16 = 33;
    const GUMP_DEF_HUE: u16 = 1_153;

    fn placeholder(body: u32) -> u32 {
        MOBILE_DEFINITION_BASE + body
    }

    fn rows(pairs: &[(u32, u32)]) -> Vec<TaeBodyAppearance> {
        pairs.iter().map(|&(body, appearance)| TaeBodyAppearance { body_type: placeholder(body), appearance }).collect()
    }

    fn records() -> Vec<TileArtEntry> {
        let mut records = vec![
            TileArtEntry {
                tile_id: SHIRT,
                prop_vector1: vec![TaeProp { id: PropertyKey::Appearance as u8, val: SHIRT_APPEARANCE }],
                ..TileArtEntry::default()
            },
            TileArtEntry {
                tile_id: GARGOYLE_ITEM,
                appearance_vector: vec![TaeAppearanceFilter::ByOwnerBodyType(rows(&[(
                    GARGOYLE_MALE_BODY,
                    GARGOYLE_APPEARANCE,
                )]))],
                ..TileArtEntry::default()
            },
            TileArtEntry {
                tile_id: WHIP,
                appearance_vector: vec![
                    TaeAppearanceFilter::ByOwnerBodyType(rows(&[
                        (HUMAN_MALE_BODY, WHIP_MALE_APPEARANCE),
                        (HUMAN_FEMALE_BODY, WHIP_FEMALE_APPEARANCE),
                    ])),
                    TaeAppearanceFilter::AnimationByOwnerBodyType(rows(&[
                        (HUMAN_MALE_BODY, WHIP_TILEDATA_ANIM),
                        (HUMAN_FEMALE_BODY, WHIP_TILEDATA_ANIM),
                    ])),
                ],
                ..TileArtEntry::default()
            },
        ];
        records.extend(
            [HUMAN_MALE_BODY, HUMAN_FEMALE_BODY, GARGOYLE_MALE_BODY, GARGOYLE_FEMALE_BODY]
                .map(|body| TileArtEntry { tile_id: placeholder(body), ..TileArtEntry::default() }),
        );
        records
    }

    #[derive(Default)]
    struct Tables {
        uop_bodies: BTreeSet<u32>,
        equip_conv: BTreeMap<(u32, u32), EquipConvGump>,
        gump_def: BTreeMap<u32, GumpDefTarget>,
    }

    impl Tables {
        /// Gargoyles with UOP frames, as the shipped `mobtypes.txt` has them.
        fn shipped() -> Self {
            Self { uop_bodies: [GARGOYLE_MALE_BODY, GARGOYLE_FEMALE_BODY].into(), ..Self::default() }
        }
    }

    impl ClassicPaperdollTables for Tables {
        fn uses_uop_frames(&self, body: u32) -> bool {
            self.uop_bodies.contains(&body)
        }

        fn equip_conv(&self, body: u32, equipment: u32) -> Option<EquipConvGump> {
            self.equip_conv.get(&(body, equipment)).copied()
        }

        fn gump_def(&self, gump: u32) -> Option<GumpDefTarget> {
            self.gump_def.get(&gump).copied()
        }
    }

    fn anim(tables: &Tables, body: u32, item: u32, tiledata_anim: u32) -> u32 {
        let records = records();
        item_anim(SortedTileArt(&records), tables, body, item, tiledata_anim)
    }

    fn body(body: u32) -> PaperdollBody {
        paperdoll_body(body, RACE_HUMAN, false).unwrap()
    }

    /// The human whip's animation filter matches the man, so his paperdoll takes tileart's appearance.
    #[test]
    fn a_human_whip_takes_the_tileart_appearance() {
        let tables = Tables::shipped();
        assert_eq!(anim(&tables, HUMAN_MALE_BODY, WHIP, WHIP_TILEDATA_ANIM), WHIP_MALE_APPEARANCE);
        assert_eq!(anim(&tables, HUMAN_FEMALE_BODY, WHIP, WHIP_TILEDATA_ANIM), WHIP_FEMALE_APPEARANCE);
        assert_eq!(item_gump(&tables, body(HUMAN_MALE_BODY), WHIP_MALE_APPEARANCE, 0, |_| true).0,
            WHIP_MALE_APPEARANCE + GUMP_ITEM_BASE);
    }

    /// Without an animation filter a human item takes the tiledata anim, whatever its Appearance property.
    #[test]
    fn a_plain_human_item_takes_the_tiledata_anim() {
        let tables = Tables::shipped();
        assert_eq!(anim(&tables, HUMAN_MALE_BODY, SHIRT, SHIRT_TILEDATA_ANIM), SHIRT_TILEDATA_ANIM);
        // Without a record of the item, the same.
        assert_eq!(anim(&tables, HUMAN_MALE_BODY, SHIRT + 1, SHIRT_TILEDATA_ANIM), SHIRT_TILEDATA_ANIM);
    }

    /// A body with UOP frames always takes tileart's appearance; with no records, 0.
    #[test]
    fn a_gargoyle_item_takes_the_tileart_appearance() {
        let tables = Tables::shipped();
        assert_eq!(anim(&tables, GARGOYLE_MALE_BODY, GARGOYLE_ITEM, GARGOYLE_ITEM_TILEDATA_ANIM), GARGOYLE_APPEARANCE);
        assert_eq!(anim(&tables, GARGOYLE_MALE_BODY, SHIRT, SHIRT_TILEDATA_ANIM), SHIRT_APPEARANCE);
        // The woman's placeholder matches no row: the item's own id.
        assert_eq!(anim(&tables, GARGOYLE_FEMALE_BODY, GARGOYLE_ITEM, GARGOYLE_ITEM_TILEDATA_ANIM), GARGOYLE_ITEM);
        assert_eq!(anim(&tables, GARGOYLE_MALE_BODY, SHIRT + 1, SHIRT_TILEDATA_ANIM), 0);
    }

    /// A woman takes the gump 10,000 above the man's when it exists, else the man's.
    #[test]
    fn a_woman_falls_back_to_the_male_gump() {
        let tables = Tables::default();
        let woman = body(HUMAN_FEMALE_BODY);
        let male_gump = SHIRT_TILEDATA_ANIM + GUMP_ITEM_BASE;
        let female_gump = male_gump + GUMP_FEMALE_OFFSET;
        assert_eq!(item_gump(&tables, woman, SHIRT_TILEDATA_ANIM, ITEM_HUE, |_| true), (female_gump, ITEM_HUE));
        assert_eq!(item_gump(&tables, woman, SHIRT_TILEDATA_ANIM, ITEM_HUE, |gump| gump == male_gump),
            (male_gump, ITEM_HUE));
        // A man never takes the female gump.
        assert_eq!(item_gump(&tables, body(HUMAN_MALE_BODY), SHIRT_TILEDATA_ANIM, 0, |_| true).0, male_gump);
    }

    /// An `Equipconv.def` row gives its gump, the converted id + 50,000 for -1, and the anim + 50,000 for 0.
    #[test]
    fn equipconv_rows_override_the_gump() {
        const GUMP: i32 = 61_250;
        const CONVERTED: i32 = 1_250;
        let mut tables = Tables::default();
        tables.equip_conv.insert((HUMAN_FEMALE_BODY, SHIRT_TILEDATA_ANIM),
            EquipConvGump { converted: CONVERTED, gump: GUMP });
        tables.equip_conv.insert((ELF_FEMALE_BODY, SHIRT_TILEDATA_ANIM),
            EquipConvGump { converted: CONVERTED, gump: EQUIPCONV_GUMP_FROM_CONVERTED });
        tables.equip_conv.insert((ELF_MALE_BODY, SHIRT_TILEDATA_ANIM),
            EquipConvGump { converted: CONVERTED, gump: EQUIPCONV_GUMP_FROM_ANIM });
        assert_eq!(equip_conv_gump(&tables, HUMAN_FEMALE_BODY, SHIRT_TILEDATA_ANIM), GUMP as u32);
        assert_eq!(equip_conv_gump(&tables, ELF_FEMALE_BODY, SHIRT_TILEDATA_ANIM), CONVERTED as u32 + GUMP_ITEM_BASE);
        assert_eq!(equip_conv_gump(&tables, ELF_MALE_BODY, SHIRT_TILEDATA_ANIM), SHIRT_TILEDATA_ANIM + GUMP_ITEM_BASE);
        assert_eq!(equip_conv_gump(&tables, HUMAN_MALE_BODY, SHIRT_TILEDATA_ANIM),
            SHIRT_TILEDATA_ANIM + GUMP_ITEM_BASE);
    }

    /// A missing gump with a `gump.def` row draws its target, with its hue when the item has none, and a woman takes
    /// the target's female gump below 60,000; a gump that exists ignores its row.
    #[test]
    fn gump_def_replaces_a_missing_gump() {
        const MISSING_ANIM: u32 = 759;
        const TARGET: u32 = 50_468;
        let missing = MISSING_ANIM + GUMP_ITEM_BASE;
        let mut tables = Tables::default();
        tables.gump_def.insert(missing, GumpDefTarget { target: TARGET, hue: GUMP_DEF_HUE });
        tables.gump_def.insert(missing + GUMP_FEMALE_OFFSET, GumpDefTarget { target: TARGET, hue: GUMP_DEF_HUE });
        let present = |gump: u32| gump != missing && gump != missing + GUMP_FEMALE_OFFSET;
        let man = body(HUMAN_MALE_BODY);
        assert_eq!(item_gump(&tables, man, MISSING_ANIM, 0, present), (TARGET, GUMP_DEF_HUE));
        assert_eq!(item_gump(&tables, man, MISSING_ANIM, ITEM_HUE, present), (TARGET, ITEM_HUE));
        assert_eq!(item_gump(&tables, body(HUMAN_FEMALE_BODY), MISSING_ANIM, 0, present),
            (TARGET + GUMP_FEMALE_OFFSET, GUMP_DEF_HUE));
        assert_eq!(item_gump(&tables, man, MISSING_ANIM, 0, |_| true), (missing, 0));
    }

    #[test]
    fn other_bodies_take_a_paperdoll_body_by_race() {
        assert_eq!(paperdoll_body(HUMAN_FEMALE_ALIAS_BODY, RACE_GARGOYLE, false),
            Some(PaperdollBody { body: HUMAN_FEMALE_BODY, female: true }));
        assert_eq!(paperdoll_body(GENDERED_HUMAN_BODY, RACE_ELF, true),
            Some(PaperdollBody { body: HUMAN_FEMALE_BODY, female: true }));
        assert_eq!(paperdoll_body(HUMAN_MALE_BODY + 2, RACE_GARGOYLE, true),
            Some(PaperdollBody { body: GARGOYLE_FEMALE_BODY, female: true }));
        assert_eq!(paperdoll_body(HUMAN_MALE_BODY + 2, RACE_GARGOYLE + 1, true), None);
    }
}
