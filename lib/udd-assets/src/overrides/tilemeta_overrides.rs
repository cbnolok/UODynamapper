//! `TilemetaOverrides.kdl`: the developers' corrections to the Enhanced tables of `tilemeta.uddp`, which Enhanced,
//! KR-like and UOD-Painterly read. The converter applies them after the conversion pass, and `udd-cli tool
//! apply-tilemeta-overrides` applies them to a packed `tilemeta.uddp`; the package records their digest and the values
//! they replaced ([`TileMetaOverridesRecord`]).
//!
//! ```kdl
//! // Why the entry is there: every entry has a reason in the comments above it.
//! items 0x1D97 0x1D98 "0x3236..0x323F" {
//!     add "Surface" "Rotated"   // TaeFlag names, or bit numbers: add 9
//!     remove "LightSource"
//!     kr-light                  // the item's light as the Kingdom Reborn tile data defines it
//! }
//! ```
//!
//! `items` takes item ids (integers, or strings holding a decimal or `0x` hexadecimal id) and inclusive ranges
//! (strings, `"0x3236..0x323F"`: KDL v1 has no bare range syntax). Its operations run in order on each item:
//! - `add` / `remove` set or clear the [`TaeFlag`]s named, by name or bit number, in `TileMetaItemTile::flags`.
//! - `kr-light` copies the item's light record from the Kingdom Reborn section of the package into the Enhanced
//!   tables.
//!
//! The file is read strictly: an unknown node, operation, property, flag name or bit, an empty or reversed range,
//! an id listed twice in one entry and an entry without operations are errors naming the line. Applying it fails
//! for an id outside the item table and for `kr-light` on an item the Kingdom Reborn data gives no light; an operation
//! that changes none of its entry's items is a warning naming the entry.

use std::fmt::Write as _;

use color_eyre::eyre;
use knuffel::ast::{Literal, Node, Value};
use knuffel::span::LineSpan;
use uocf::enhanced::tileart::TaeFlag;
use uot_kdl::strict::{error_at, value_u32, NodeSchema};

use crate::tilemeta::{set_item_light, TileMetaItemLight, TileMetaItemTile, TileMetaOverridesRecord};

pub const TILEMETA_OVERRIDES_FILE_NAME: &str = "TilemetaOverrides.kdl";
const ITEMS_NODE: &str = "items";
const ADD_NODE: &str = "add";
const REMOVE_NODE: &str = "remove";
const KR_LIGHT_NODE: &str = "kr-light";
/// Separator of an inclusive id range: `"0x3236..0x323F"`.
const RANGE_SEPARATOR: &str = "..";
const HEX_PREFIX: &str = "0x";
const HEX_RADIX: u32 = 16;
const DECIMAL_RADIX: u32 = 10;
/// Seed of the digest of the normalised overrides ([`TilemetaOverrides::digest`]).
const DIGEST_SEED: u64 = 0;
/// The most ids a warning lists before it counts the rest.
const WARNING_LISTED_IDS: usize = 8;

pub const TILEMETA_OVERRIDES_SCHEMA: &[NodeSchema] = &[NodeSchema {
    name: ITEMS_NODE,
    properties: &[],
    children: &[NodeSchema::leaf(ADD_NODE, &[]), NodeSchema::leaf(REMOVE_NODE, &[]), NodeSchema::leaf(KR_LIGHT_NODE, &[])],
}];

/// One operation of an entry, run on each of its items in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilemetaOverrideOp {
    /// Sets these `TileMetaItemTile::flags` bits.
    Add(TaeFlag),
    /// Clears these `TileMetaItemTile::flags` bits.
    Remove(TaeFlag),
    /// Copies the item's Kingdom Reborn light record into the Enhanced tables.
    KrLight,
}

impl TilemetaOverrideOp {
    fn describe(&self) -> String {
        match self {
            Self::Add(flags) => format!("{ADD_NODE} {}", flag_names(*flags)),
            Self::Remove(flags) => format!("{REMOVE_NODE} {}", flag_names(*flags)),
            Self::KrLight => KR_LIGHT_NODE.to_string(),
        }
    }

    fn normalized(&self) -> String {
        match self {
            Self::Add(flags) => format!("{ADD_NODE} {:#X}", flags.bits()),
            Self::Remove(flags) => format!("{REMOVE_NODE} {:#X}", flags.bits()),
            Self::KrLight => KR_LIGHT_NODE.to_string(),
        }
    }
}

/// One `items` node: its items, in file order, and its operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TilemetaOverrideEntry {
    /// 1-based line of the `items` node.
    pub line: usize,
    pub ids: Vec<u32>,
    pub ops: Vec<TilemetaOverrideOp>,
}

impl TilemetaOverrideEntry {
    /// `items 0x1D97 0x1D98` with the ids sorted and runs written as ranges, at most `WARNING_LISTED_IDS` of them.
    fn describe(&self) -> String {
        let ranges = id_ranges(&self.ids);
        let mut text = ITEMS_NODE.to_string();
        for range in ranges.iter().take(WARNING_LISTED_IDS) {
            text.push(' ');
            text.push_str(range);
        }
        if ranges.len() > WARNING_LISTED_IDS {
            let _ = write!(text, " and {} more", ranges.len() - WARNING_LISTED_IDS);
        }
        text
    }
}

/// The parsed overrides file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TilemetaOverrides {
    /// The file name errors and warnings start with.
    pub source_name: String,
    pub entries: Vec<TilemetaOverrideEntry>,
}

/// The outcome of [`TilemetaOverrides::apply`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TilemetaOverridesApplied {
    /// The digest and the values the overrides replaced, for `TILEMETA_OVERRIDES_ENTRY_PATH`.
    pub record: TileMetaOverridesRecord,
    /// One line per operation that changed none of its entry's items.
    pub warnings: Vec<String>,
}

impl TilemetaOverrides {
    /// Reads the file strictly (see the module documentation).
    pub fn parse(source_name: &str, text: &str) -> eyre::Result<Self> {
        Self::parse_entries(source_name, text)
            .map(|entries| Self { source_name: source_name.to_string(), entries })
            .map_err(|error| uot_kdl::strict::in_file(source_name, error))
    }

    fn parse_entries(source_name: &str, text: &str) -> eyre::Result<Vec<TilemetaOverrideEntry>> {
        let document = uot_kdl::parse_ast_line_span(source_name, text)
            .map_err(|error| uot_kdl::strict::describe_error(text, &error))?;
        uot_kdl::strict::check_document(&document, TILEMETA_OVERRIDES_SCHEMA)?;
        document.nodes.iter().map(|node| parse_entry(node)).collect()
    }

    /// The overrides in a canonical text: one line per entry, its ids sorted with runs as ranges, its operations in
    /// order with their flags as one hexadecimal mask. Comments, layout, flag names against bit numbers and id order
    /// do not change it.
    pub fn normalized(&self) -> String {
        let mut text = String::new();
        for entry in &self.entries {
            text.push_str(ITEMS_NODE);
            for range in id_ranges(&entry.ids) {
                text.push(' ');
                text.push_str(&range);
            }
            text.push_str(" {");
            for op in &entry.ops {
                text.push_str(" ");
                text.push_str(&op.normalized());
                text.push(';');
            }
            text.push_str(" }\n");
        }
        text
    }

    /// The xxh64 of [`Self::normalized`], recorded in the package the overrides were applied to.
    pub fn digest(&self) -> u64 {
        xxhash_rust::xxh64::xxh64(self.normalized().as_bytes(), DIGEST_SEED)
    }

    /// Applies the overrides to the Enhanced `items` (indexed by id) and `lights` (sorted by unique id), taking the
    /// Kingdom Reborn lights from `kr_lights` (sorted by unique id). Checks every entry before changing anything: on an
    /// error the tables are left as they were.
    pub fn apply(
        &self,
        items: &mut [TileMetaItemTile],
        lights: &mut Vec<TileMetaItemLight>,
        kr_lights: &[TileMetaItemLight],
    ) -> eyre::Result<TilemetaOverridesApplied> {
        let kr_light = |id: u32| kr_lights.binary_search_by_key(&id, |light| light.tile_id).ok().map(|index| kr_lights[index]);
        for entry in &self.entries {
            let at = format!("{}: line {}", self.source_name, entry.line);
            for &id in &entry.ids {
                eyre::ensure!((id as usize) < items.len(),
                    "{at}: item {id:#06X} is outside the item table (ids below {:#06X})", items.len());
                eyre::ensure!(!entry.ops.contains(&TilemetaOverrideOp::KrLight) || kr_light(id).is_some(),
                    "{at}: {KR_LIGHT_NODE}: the Kingdom Reborn tile data gives item {id:#06X} no light");
            }
        }

        let mut original_flags = std::collections::BTreeMap::new();
        let mut original_lights = std::collections::BTreeMap::new();
        let mut warnings = Vec::new();
        for entry in &self.entries {
            for op in &entry.ops {
                let mut changed = false;
                for &id in &entry.ids {
                    let item = &mut items[id as usize];
                    match op {
                        TilemetaOverrideOp::Add(flags) | TilemetaOverrideOp::Remove(flags) => {
                            let new_flags = match op {
                                TilemetaOverrideOp::Add(_) => item.flags | flags.bits(),
                                _ => item.flags & !flags.bits(),
                            };
                            if new_flags != item.flags {
                                original_flags.entry(id).or_insert(item.flags);
                                item.flags = new_flags;
                                changed = true;
                            }
                        }
                        TilemetaOverrideOp::KrLight => {
                            let light = kr_light(id).expect("checked above");
                            let current = lights.binary_search_by_key(&id, |light| light.tile_id).ok().map(|index| lights[index]);
                            if current != Some(light) {
                                original_lights.entry(id).or_insert(current);
                                set_item_light(lights, id, Some(light));
                                changed = true;
                            }
                        }
                    }
                }
                if !changed {
                    warnings.push(format!("{}: line {}: `{}` in `{}` changes none of its items",
                        self.source_name, entry.line, op.describe(), entry.describe()));
                }
            }
        }
        Ok(TilemetaOverridesApplied {
            record: TileMetaOverridesRecord {
                digest: self.digest(),
                original_flags: original_flags.into_iter().collect(),
                original_lights: original_lights.into_iter().collect(),
            },
            warnings,
        })
    }
}

fn parse_entry(node: &Node<LineSpan>) -> eyre::Result<TilemetaOverrideEntry> {
    let line = node.node_name.span().0.line + 1;
    eyre::ensure!(!node.arguments.is_empty(),
        error_at(node.node_name.span(), format!("`{ITEMS_NODE}` takes item ids or ranges, found none")));
    let mut ids = Vec::new();
    for value in &node.arguments {
        for id in parse_ids(value)? {
            if ids.contains(&id) {
                return Err(error_at(value.literal.span(), format!("`{ITEMS_NODE}`: item {id:#06X} listed twice")));
            }
            ids.push(id);
        }
    }
    let mut ops = Vec::new();
    for child in node.children() {
        let name: &str = &child.node_name;
        uot_kdl::strict::reject_children(child)?;
        let op = match name {
            ADD_NODE => TilemetaOverrideOp::Add(parse_flags(child)?),
            REMOVE_NODE => TilemetaOverrideOp::Remove(parse_flags(child)?),
            KR_LIGHT_NODE => {
                uot_kdl::strict::arguments::<0>(child)?;
                TilemetaOverrideOp::KrLight
            }
            _ => unreachable!("checked against the schema"),
        };
        ops.push(op);
    }
    eyre::ensure!(!ops.is_empty(), error_at(node.node_name.span(),
        format!("`{ITEMS_NODE}` has no operations: expected `{ADD_NODE}`, `{REMOVE_NODE}` or `{KR_LIGHT_NODE}`")));
    Ok(TilemetaOverrideEntry { line, ids, ops })
}

/// The ids of one `items` argument: an integer, or a string holding an id or an inclusive range.
fn parse_ids(value: &Value<LineSpan>) -> eyre::Result<Vec<u32>> {
    let span = value.literal.span();
    let text = match &*value.literal {
        Literal::Int(_) => return value_u32(value, ITEMS_NODE).map(|id| vec![id]),
        Literal::String(text) => text.trim(),
        _ => return Err(error_at(span, format!("`{ITEMS_NODE}`: expected an item id or a range like \"0x3236..0x323F\""))),
    };
    let parse_id = |part: &str| {
        parse_number(part.trim()).ok_or_else(|| error_at(span,
            format!("`{ITEMS_NODE}`: `{part}` is not an item id; expected 0x1D97, 7575 or a range like \"0x3236..0x323F\"")))
    };
    match text.split_once(RANGE_SEPARATOR) {
        Some((start, end)) => {
            let (start, end) = (parse_id(start)?, parse_id(end)?);
            eyre::ensure!(start <= end,
                error_at(span, format!("`{ITEMS_NODE}`: range {text} ends before it starts")));
            Ok((start..=end).collect())
        }
        None => parse_id(text).map(|id| vec![id]),
    }
}

/// A decimal or `0x` hexadecimal number.
fn parse_number(text: &str) -> Option<u32> {
    match text.strip_prefix(HEX_PREFIX).or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, HEX_RADIX).ok(),
        None => u32::from_str_radix(text, DECIMAL_RADIX).ok(),
    }
}

/// The flags of an `add` or `remove`: TaeFlag names (strings) or bit numbers (integers), at least one.
fn parse_flags(node: &Node<LineSpan>) -> eyre::Result<TaeFlag> {
    let name: &str = &node.node_name;
    eyre::ensure!(!node.arguments.is_empty(),
        error_at(node.node_name.span(), format!("`{name}` takes flag names or bit numbers, found none")));
    let mut flags = TaeFlag::empty();
    for value in &node.arguments {
        let span = value.literal.span();
        let flag = match &*value.literal {
            Literal::String(flag_name) => TaeFlag::from_name(flag_name)
                .filter(|flag| !flag.is_empty())
                .ok_or_else(|| error_at(span, format!("`{name}`: unknown flag `{flag_name}`; expected one of {}",
                    TaeFlag::all().iter_names().map(|(name, _)| name).collect::<Vec<_>>().join(", "))))?,
            Literal::Int(_) => {
                let bit = value_u32(value, name)?;
                1u64.checked_shl(bit)
                    .and_then(TaeFlag::from_bits)
                    .ok_or_else(|| error_at(span, format!("`{name}`: bit {bit} is not a TaeFlag; defined bits are 0 to {}",
                        TaeFlag::all().bits().ilog2())))?
            }
            _ => return Err(error_at(span, format!("`{name}`: expected a flag name or a bit number"))),
        };
        flags |= flag;
    }
    Ok(flags)
}

/// `Surface | Rotated`.
fn flag_names(flags: TaeFlag) -> String {
    flags.iter_names().map(|(name, _)| format!("\"{name}\"")).collect::<Vec<_>>().join(" ")
}

/// The ids sorted, each run of consecutive ids written as one `0xA..0xB` range.
fn id_ranges(ids: &[u32]) -> Vec<String> {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < sorted.len() {
        let start = sorted[index];
        let mut end = start;
        while index + 1 < sorted.len() && sorted[index + 1] == end + 1 {
            index += 1;
            end = sorted[index];
        }
        ranges.push(if start == end { format!("{start:#06X}") } else { format!("{start:#06X}{RANGE_SEPARATOR}{end:#06X}") });
        index += 1;
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    const NAME: &str = "TilemetaOverrides.kdl";

    fn parse(text: &str) -> TilemetaOverrides {
        TilemetaOverrides::parse(NAME, text).expect("parse")
    }

    fn parse_error(text: &str) -> String {
        format!("{:#}", TilemetaOverrides::parse(NAME, text).expect_err("parse error"))
    }

    fn items(count: u32) -> Vec<TileMetaItemTile> {
        (0..count).map(|tile_id| TileMetaItemTile { tile_id, ..TileMetaItemTile::zeroed() }).collect()
    }

    fn light(tile_id: u32, light_texture_entry: u32) -> TileMetaItemLight {
        TileMetaItemLight { tile_id, light_texture_entry, scale: 2.0, offset_x: -5.0, offset_y: -4.0 }
    }

    #[test]
    fn reads_ids_ranges_flag_names_and_bits() {
        let overrides = parse(
            "// a reason\nitems 0x10 \"0x3..0x5\" \"7\" 1 {\n    add \"Surface\" 34\n    remove \"LightSource\"\n    kr-light\n}\n",
        );
        assert_eq!(overrides.entries, vec![TilemetaOverrideEntry {
            line: 2,
            ids: vec![0x10, 3, 4, 5, 7, 1],
            ops: vec![
                TilemetaOverrideOp::Add(TaeFlag::Surface | TaeFlag::Rotated),
                TilemetaOverrideOp::Remove(TaeFlag::LightSource),
                TilemetaOverrideOp::KrLight,
            ],
        }]);
        assert_eq!(overrides.normalized(), "items 0x0001 0x0003..0x0005 0x0007 0x0010 { add 0x400000200; remove 0x800000; kr-light; }\n");
    }

    #[test]
    fn digest_ignores_comments_layout_names_and_id_order() {
        let named = parse("// why\nitems \"0x3..0x5\" { add \"Surface\"; }\n");
        let numbered = parse("items 5 4 \"3\" {\n    // other words\n    add 9\n}\n");
        assert_eq!(named.digest(), numbered.digest());
        assert_ne!(named.digest(), parse("items \"0x3..0x5\" { add \"Bridge\"; }\n").digest());
    }

    #[test]
    fn the_bundled_file_parses() {
        let text = include_str!("../../../../dynamapper/assets/cc_ec_convtables/TilemetaOverrides.kdl");
        assert!(!parse(text).entries.is_empty());
    }

    #[test]
    fn rejects_what_it_does_not_know() {
        assert!(parse_error("item 1 { add \"Surface\"; }").contains("line 1, column 1: unknown node `item` at the top level"));
        assert!(parse_error("items 1 { set \"Surface\"; }").contains("unknown node `set` in `items`"));
        assert!(parse_error("items 1 kind=2 { add \"Surface\"; }").contains("`items` takes no properties"));
        assert!(parse_error("items 1 { add \"Surfaces\"; }").contains("unknown flag `Surfaces`"));
        assert!(parse_error("items 1 { add \"None\"; }").contains("unknown flag `None`"));
        assert!(parse_error("items 1 { add 41; }").contains("bit 41 is not a TaeFlag; defined bits are 0 to 40"));
        assert!(parse_error("items 1 { add 64; }").contains("bit 64 is not a TaeFlag"));
        assert!(parse_error("items 1 { add; }").contains("`add` takes flag names or bit numbers, found none"));
        assert!(parse_error("items 1 { remove 1.5; }").contains("expected a flag name or a bit number"));
        assert!(parse_error("items 1 { kr-light 1; }").contains("`kr-light` takes 0 arguments"));
        assert!(parse_error("items 1 { kr-light { add 9; }; }").contains("`kr-light` takes no children"));
        assert!(parse_error("items { add 9; }").contains("`items` takes item ids or ranges, found none"));
        assert!(parse_error("items 1\n").contains("line 1, column 1: `items` has no operations"));
        assert!(parse_error("items \"0x5..0x3\" { add 9; }").contains("range 0x5..0x3 ends before it starts"));
        assert!(parse_error("items \"0xZZ\" { add 9; }").contains("`0xZZ` is not an item id"));
        assert!(parse_error("items 4 \"0x3..0x5\" { add 9; }").contains("item 0x0004 listed twice"));
        assert!(parse_error("item 1 { add 9; }").starts_with(NAME));
    }

    #[test]
    fn applies_flags_and_kr_lights_and_records_what_it_replaced() {
        let mut table = items(8);
        table[3].flags = TaeFlag::LightSource.bits();
        let mut lights = vec![light(5, 9)];
        let kr_lights = [light(3, 1), light(5, 1)];
        let overrides = parse(
            "items 3 \"0x4..0x5\" { add \"Surface\"; }\nitems 3 5 { kr-light; }\nitems 3 { remove \"LightSource\"; }\n",
        );
        let applied = overrides.apply(&mut table, &mut lights, &kr_lights).expect("apply");
        assert!(applied.warnings.is_empty(), "{:?}", applied.warnings);
        assert_eq!(table[3].flags, TaeFlag::Surface.bits());
        assert_eq!(table[4].flags, TaeFlag::Surface.bits());
        assert_eq!(table[6].flags, 0);
        assert_eq!(lights, vec![light(3, 1), light(5, 1)]);
        assert_eq!(applied.record.digest, overrides.digest());
        assert_eq!(applied.record.original_flags, vec![(3, TaeFlag::LightSource.bits()), (4, 0), (5, 0)]);
        assert_eq!(applied.record.original_lights, vec![(3, None), (5, Some(light(5, 9)))]);

        // The record restores the tables as they were, and survives its entry's bytes.
        let bytes = applied.record.to_bytes();
        let record = TileMetaOverridesRecord::from_bytes(&bytes).expect("record");
        assert_eq!(record, applied.record);
        assert!(TileMetaOverridesRecord::from_bytes(&bytes[..bytes.len() - 1]).is_err(), "truncated");
        assert!(TileMetaOverridesRecord::from_bytes(&[bytes.as_slice(), &[0]].concat()).is_err(), "trailing bytes");
        record.restore(&mut table, &mut lights).expect("restore");
        assert_eq!(table[3].flags, TaeFlag::LightSource.bits());
        assert_eq!(table[5].flags, 0);
        assert_eq!(lights, vec![light(5, 9)]);
    }

    #[test]
    fn warns_for_an_operation_that_changes_nothing() {
        let mut table = items(4);
        table[1].flags = TaeFlag::Surface.bits();
        let mut lights = vec![light(2, 1)];
        let overrides = parse("items 1 { add \"Surface\"; remove \"Bridge\"; }\nitems 2 3 { add 9; }\nitems 2 { kr-light; }\n");
        let applied = overrides.apply(&mut table, &mut lights, &[light(2, 1)]).expect("apply");
        assert_eq!(applied.warnings, vec![
            format!("{NAME}: line 1: `add \"Surface\"` in `items 0x0001` changes none of its items"),
            format!("{NAME}: line 1: `remove \"Bridge\"` in `items 0x0001` changes none of its items"),
            format!("{NAME}: line 3: `kr-light` in `items 0x0002` changes none of its items"),
        ]);
        assert_eq!(applied.record.original_flags, vec![(2, 0), (3, 0)]);
        assert!(applied.record.original_lights.is_empty());
    }

    #[test]
    fn refuses_ids_outside_the_table_and_kr_lights_the_kr_data_lacks() {
        let mut table = items(4);
        let mut lights = Vec::new();
        let error = parse("items 1 { add 9; }\nitems \"2..4\" { add 9; }\n")
            .apply(&mut table, &mut lights, &[]).expect_err("outside");
        assert_eq!(format!("{error}"), format!("{NAME}: line 2: item 0x0004 is outside the item table (ids below 0x0004)"));
        assert_eq!(table[1].flags, 0, "nothing is applied when an entry fails");
        let error = parse("items 1 { kr-light; }\n").apply(&mut table, &mut lights, &[light(2, 1)]).expect_err("no light");
        assert!(format!("{error}").contains("the Kingdom Reborn tile data gives item 0x0001 no light"));
    }
}
