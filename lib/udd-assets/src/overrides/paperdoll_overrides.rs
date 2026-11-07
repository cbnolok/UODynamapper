//! `PaperdollOverrides.kdl`: the user's manual layer over the dress-layer decisions paperdoll_ec.uddp stores
//! ([`crate::paperdoll_ec::PaperdollEcDressDecisions`]), which decide what Dynamapper's paperdolls take off beneath a
//! robe. A cover decides for its covering items, on its bodies: a garment rule (`show items=`, `hide items=`) for the
//! garments it names, else its slot rule (`hide "<slot>"`) for the rest; garment rules beat slot rules, which beat the
//! stored decisions, and among the covers that apply the last one in the file decides.
//!
//! [`PaperdollOverrides::decode`] reads the file strictly; [`export_dress_decisions`] writes the stored decisions as
//! covers that decide the same; [`set_garment_rule`] edits a file in place for one garment, keeping the user's comments
//! and other covers as written.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::ops::Range;

use color_eyre::eyre;
use knuffel::ast::{Literal, Node};
use knuffel::span::Span;
use knuffel::Decode;
use uocf::enhanced::paperdoll::{slot_layer, LAYER_COUNT, SLOT_NAMES};

use crate::paperdoll_ec::{layer_mask, PaperdollEcDressDecisions};

pub const PAPERDOLL_OVERRIDES_FILE_NAME: &str = "PaperdollOverrides.kdl";
/// The slot of the dress-layer items, the covering items of the stored decisions.
pub const DRESS_SLOT: &str = "dress";
const COVER_NODE: &str = "cover";
const HIDE_NODE: &str = "hide";
const SHOW_NODE: &str = "show";
const EXCEPT_NODE: &str = "except";
const ITEMS_PROPERTY: &str = "items";
const BODIES_PROPERTY: &str = "bodies";
const ANY_ITEM: &str = "any";
const ID_SEPARATOR: char = ',';
/// How a cover's rules are indented, as in the bundled file.
const RULE_INDENT: &str = "    ";
/// Ends the comment above a cover the paperdoll window writes, so the cover's heading goes with it.
const PAPERDOLL_WINDOW_MARK: &str = "chosen in the paperdoll window";

/// `cover "<slot>" items="any"|"<ids>" [bodies="<body frames>"] { hide "<slot>"; hide items="<ids>";
/// show items="<ids>"; except items="<ids>" }`, decoded by `OverridesKdl`.
pub const PAPERDOLL_OVERRIDES_SCHEMA: &[uot_kdl::strict::NodeSchema] = &[uot_kdl::strict::NodeSchema {
    name: COVER_NODE,
    properties: &[ITEMS_PROPERTY, BODIES_PROPERTY],
    children: &[
        uot_kdl::strict::NodeSchema::leaf(HIDE_NODE, &[ITEMS_PROPERTY]),
        uot_kdl::strict::NodeSchema::leaf(SHOW_NODE, &[ITEMS_PROPERTY]),
        uot_kdl::strict::NodeSchema::leaf(EXCEPT_NODE, &[ITEMS_PROPERTY]),
    ],
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
    #[knuffel(property)]
    bodies: Option<String>,
    #[knuffel(children(name = "hide"))]
    hides: Vec<HideKdl>,
    #[knuffel(children(name = "show"))]
    shows: Vec<ItemsKdl>,
    #[knuffel(children(name = "except"))]
    excepts: Vec<ItemsKdl>,
}

/// `hide "<slot>"` or `hide items="<ids>"`.
#[derive(Decode)]
struct HideKdl {
    #[knuffel(argument)]
    slot: Option<String>,
    #[knuffel(property)]
    items: Option<String>,
}

/// `show items="<ids>"` or `except items="<ids>"`.
#[derive(Decode)]
struct ItemsKdl {
    #[knuffel(property)]
    items: String,
}

/// While an item of `layer` among `items` is drawn on one of `bodies`, its rules decide what is taken off beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cover {
    layer: usize,
    /// The covering items; `None` is any.
    items: Option<Vec<u32>>,
    /// The body frames it applies on; `None` is every body.
    bodies: Option<Vec<u32>>,
    /// The slot rule, bit n set: layer n is taken off. `None` when the cover has garment rules alone and leaves the
    /// slots to the covers before it and the stored decision; a cover with no rule at all keeps everything.
    hidden: Option<u32>,
    /// Garments kept beneath the covering items, whatever the slot rules say.
    shown_items: Vec<u32>,
    /// Garments taken off beneath the covering items, whatever the slot rules say.
    hidden_items: Vec<u32>,
    /// Covering items the cover leaves alone.
    except: Vec<u32>,
    /// The 1-based line the cover starts on.
    line: usize,
}

impl Cover {
    fn applies(&self, covering: (usize, u32), body: Option<u32>) -> bool {
        self.layer == covering.0
            && self.items.as_ref().is_none_or(|items| items.contains(&covering.1))
            && !self.except.contains(&covering.1)
            && self.bodies.as_ref().is_none_or(|bodies| body.is_some_and(|body| bodies.contains(&body)))
    }

    /// The garment rule naming `garment`: whether it takes it off.
    fn garment_rule(&self, garment: u32) -> Option<bool> {
        if self.shown_items.contains(&garment) {
            Some(false)
        } else if self.hidden_items.contains(&garment) {
            Some(true)
        } else {
            None
        }
    }

    /// The cover as the paperdoll window writes it: garment rules alone, for one covering item on one body.
    fn is_window_cover(&self, covering: (usize, u32), body: u32) -> bool {
        self.layer == covering.0
            && self.items.as_deref() == Some(&[covering.1])
            && self.bodies.as_deref() == Some(&[body])
            && self.except.is_empty()
            && self.hidden.is_none()
    }
}

/// Which rule of a cover decides a garment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideRule {
    /// `show items=` or `hide items=` names the garment.
    Garment,
    /// The cover's `hide "<slot>"` nodes, or its having no rule (keep everything), decide the garment's slot.
    Slot,
}

/// What the overrides decide for a garment beneath a covering item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverrideDecision {
    pub takes_off: bool,
    pub rule: OverrideRule,
    /// The 1-based line of the deciding cover.
    pub line: usize,
}

/// The user's paperdoll overrides. The default has none: the stored decisions alone.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PaperdollOverrides {
    covers: Vec<Cover>,
}

/// A decimal or `0x` hexadecimal number.
fn parse_number(text: &str) -> Option<u32> {
    const HEX_RADIX: u32 = 16;
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, HEX_RADIX).ok(),
        None => text.parse().ok(),
    }
}

/// A comma-separated list of item ids; `at` starts the error message.
fn parse_item_ids(at: &str, items: &str) -> eyre::Result<Vec<u32>> {
    items
        .split(ID_SEPARATOR)
        .map(|part| {
            parse_number(part).ok_or_else(|| {
                eyre::eyre!("{at}: items: `{}` is not an item id; expected ids like 0x1F03,7939", part.trim())
            })
        })
        .collect()
}

/// A comma-separated list of body frame numbers; `at` starts the error message.
fn parse_bodies(at: &str, bodies: &str) -> eyre::Result<Vec<u32>> {
    bodies
        .split(ID_SEPARATOR)
        .map(|part| {
            parse_number(part).ok_or_else(|| {
                eyre::eyre!("{at}: bodies: `{}` is not a body; expected body frame numbers like 400,401", part.trim())
            })
        })
        .collect()
}

fn layer(at: &str, slot: &str) -> eyre::Result<usize> {
    slot_layer(slot).map_err(|_| eyre::eyre!("{at}: unknown slot `{slot}`; expected one of {SLOT_NAMES:?}"))
}

/// An item id as the files write it.
fn item_text(item_id: u32) -> String {
    format!("0x{item_id:04X}")
}

/// `name (0x1F03)`, or the id alone.
fn named(item_id: u32, name: Option<&str>) -> String {
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("{name} ({})", item_text(item_id)),
        None => item_text(item_id),
    }
}

/// A name fit for a one-line comment.
fn comment_text(name: &str) -> String {
    name.chars().map(|character| if character.is_control() { ' ' } else { character }).collect::<String>().trim().to_string()
}

impl PaperdollOverrides {
    pub fn decode(source_name: &str, text: &str) -> eyre::Result<Self> {
        let decoded: OverridesKdl = uot_kdl::strict::decode(source_name, text, PAPERDOLL_OVERRIDES_SCHEMA)?;
        // Every top-level node is a cover, in file order.
        let document = uot_kdl::parse_ast_line_span(source_name, text)
            .map_err(|error| uot_kdl::strict::in_file(source_name, uot_kdl::strict::describe_error(text, &error)))?;
        let lines = document.nodes.iter().map(|node| node.span().0.line + 1);
        let mut covers = Vec::with_capacity(decoded.covers.len());
        for (cover, line) in decoded.covers.into_iter().zip(lines) {
            let at = format!("{source_name}: line {line}");
            let cover_layer = layer(&at, &cover.slot)?;
            let items = if cover.items.trim().eq_ignore_ascii_case(ANY_ITEM) {
                None
            } else {
                Some(parse_item_ids(&at, &cover.items)?)
            };
            let bodies = cover.bodies.as_deref().map(|bodies| parse_bodies(&at, bodies)).transpose()?;
            let mut hidden_slots = Vec::new();
            let mut hidden_items = Vec::new();
            for hide in &cover.hides {
                match (&hide.slot, &hide.items) {
                    (Some(slot), None) => {
                        let hidden_layer = layer(&at, slot)?;
                        eyre::ensure!(hidden_layer != cover_layer, "{at}: `cover \"{}\"` hides its own slot", cover.slot);
                        hidden_slots.push(hidden_layer);
                    }
                    (None, Some(items)) => hidden_items.extend(parse_item_ids(&at, items)?),
                    (Some(_), Some(_)) => eyre::bail!("{at}: `hide` takes a slot or items=, not both"),
                    (None, None) => eyre::bail!("{at}: `hide` needs a slot or items="),
                }
            }
            let mut shown_items = Vec::new();
            for show in &cover.shows {
                shown_items.extend(parse_item_ids(&at, &show.items)?);
            }
            if let Some(both) = shown_items.iter().find(|item| hidden_items.contains(item)) {
                eyre::bail!("{at}: item {} is both shown and hidden", item_text(*both));
            }
            let mut except = Vec::new();
            for node in &cover.excepts {
                except.extend(parse_item_ids(&at, &node.items)?);
            }
            let garment_rules_only = hidden_slots.is_empty() && !(shown_items.is_empty() && hidden_items.is_empty());
            let hidden = (!garment_rules_only).then(|| layer_mask(hidden_slots));
            covers.push(Cover { layer: cover_layer, items, bodies, hidden, shown_items, hidden_items, except, line });
        }
        Ok(Self { covers })
    }

    /// What the overrides decide for `garment` (its layer and id) drawn beneath `covering` (its layer and id) on `body`
    /// (the number of the body frame drawn; `None` when no frame is: then a cover with bodies does not apply). Among
    /// the covers that apply, the last one with a garment rule naming the garment decides; else the last one with a
    /// slot rule (a cover with no rule keeps everything); else `None`, and the stored decision holds. An item a cover
    /// excepts falls back to the covers before it.
    pub fn decide(&self, covering: (usize, u32), body: Option<u32>, garment: (usize, u32)) -> Option<OverrideDecision> {
        let applying = || self.covers.iter().rev().filter(move |cover| cover.applies(covering, body));
        let by_garment = applying().find_map(|cover| {
            let takes_off = cover.garment_rule(garment.1)?;
            Some(OverrideDecision { takes_off, rule: OverrideRule::Garment, line: cover.line })
        });
        by_garment.or_else(|| {
            applying().find_map(|cover| {
                let hidden = cover.hidden?;
                let takes_off = garment.0 < LAYER_COUNT && hidden & 1 << garment.0 != 0;
                Some(OverrideDecision { takes_off, rule: OverrideRule::Slot, line: cover.line })
            })
        })
    }
}

/// The stored decisions as `PaperdollOverrides.kdl` covers that decide the same garments the same way: for each
/// dress-layer item, one cover for each set of bodies on which it keeps the same garments, hiding the decisions' slots
/// and showing the garments kept, one per line. `name` gives an item's name for the comments; `source` names the
/// package in the heading.
pub fn export_dress_decisions(
    decisions: &PaperdollEcDressDecisions,
    source: &str,
    name: impl Fn(u32) -> Option<String>,
) -> String {
    // Per item, ascending: the kept garments and the bodies that keep them, in the order met.
    let mut items: BTreeMap<u32, Vec<(Vec<u32>, Vec<u16>)>> = BTreeMap::new();
    for decision in &decisions.decisions {
        let kept: Vec<u32> = decision.kept.iter().map(|garment| garment.item_id).collect();
        let groups = items.entry(decision.item_id).or_default();
        match groups.iter_mut().find(|(group_kept, _)| *group_kept == kept) {
            Some((_, bodies)) => bodies.push(decision.body),
            None => groups.push((kept, vec![decision.body])),
        }
    }
    let hidden_slots: Vec<&str> =
        (0..LAYER_COUNT).filter(|&layer| decisions.covers(layer)).map(|layer| SLOT_NAMES[layer]).collect();
    let mut text = String::new();
    let _ = writeln!(text, "// What {source} decides beneath each dress-layer item, written as {PAPERDOLL_OVERRIDES_FILE_NAME} covers");
    text.push_str(
        "// by `udd-cli tool export-kdl`. Each cover decides one item on the bodies listed (the number of the body frame\n\
         // drawn: 400 and 401 human, 605 and 606 elf, 666 and 667 gargoyle, male then female). Beneath it the garments of\n\
         // the hidden slots are taken off, except the garments shown, which the packer measured to show in plain view.\n\
         // Used as your PaperdollOverrides.kdl, these covers decide every garment the package decides as it does, with or\n\
         // without the package's decisions; edit them as any cover (the bundled PaperdollOverrides.kdl gives the syntax).\n\
         // Unlike the stored decisions, a hidden slot also takes off the garments the packer could not measure: those\n\
         // without a tileart Layer, or without a 3D frame for the body.\n",
    );
    let _ = writeln!(text, "// {} dress-layer items, {} decisions.", items.len(), decisions.decisions.len());
    for (item_id, groups) in &items {
        let _ = writeln!(text, "\n// {}", comment_text(&named(*item_id, name(*item_id).as_deref())));
        for (kept, bodies) in groups {
            let bodies: Vec<String> = bodies.iter().map(u16::to_string).collect();
            let _ = writeln!(
                text,
                "{COVER_NODE} \"{DRESS_SLOT}\" {ITEMS_PROPERTY}=\"{}\" {BODIES_PROPERTY}=\"{}\" {{",
                item_text(*item_id),
                bodies.join(",")
            );
            for slot in &hidden_slots {
                let _ = writeln!(text, "{RULE_INDENT}{HIDE_NODE} \"{slot}\"");
            }
            for &garment in kept {
                let _ = write!(text, "{RULE_INDENT}{SHOW_NODE} {ITEMS_PROPERTY}=\"{}\"", item_text(garment));
                match name(garment) {
                    Some(garment_name) => {
                        let _ = writeln!(text, " // {}", comment_text(&garment_name));
                    }
                    None => text.push('\n'),
                }
            }
            text.push_str("}\n");
        }
    }
    text
}

/// What the paperdoll window writes for a garment beneath a covering item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GarmentChoice {
    /// `show items=`: kept beneath the covering item, whatever the slot rules and the stored decision say.
    Show,
    /// `hide items=`: taken off beneath it.
    Hide,
    /// No garment rule: the covers' slot rules, else the stored decision.
    Measured,
}

/// A choice for `garment` beneath `covering` (each its layer and id, and its name for the comments) on `body` (the
/// number of the body frame drawn).
#[derive(Debug, Clone, Copy)]
pub struct GarmentRuleEdit<'a> {
    pub covering: (usize, u32),
    pub covering_name: Option<&'a str>,
    pub body: u32,
    pub garment: (usize, u32),
    pub garment_name: Option<&'a str>,
    pub choice: GarmentChoice,
}

/// A text edit: the bytes of `range` replaced by `text`.
struct TextEdit {
    range: Range<usize>,
    text: String,
}

/// The range to delete for the node spanning `span` (which ends with its terminator: a newline, a comment and its
/// newline, or `;`): its whole lines, with a comment ending the last one, when the node is alone on them; else the node
/// and the blanks after it.
fn node_removal(text: &str, span: &Span) -> Range<usize> {
    let line_start = text[..span.0].rfind('\n').map_or(0, |newline| newline + 1);
    let alone = text[line_start..span.0].trim().is_empty();
    if text[span.0..span.1].ends_with('\n') || span.1 == text.len() {
        return if alone { line_start..span.1 } else { span.0..span.1 };
    }
    let line_end = text[span.1..].find('\n').map_or(text.len(), |newline| span.1 + newline + 1);
    let rest = text[span.1..line_end].trim();
    if alone && (rest.is_empty() || rest.starts_with("//")) {
        return line_start..line_end;
    }
    let after = &text[span.1..];
    span.0..span.1 + after.len() - after.trim_start_matches([' ', '\t']).len()
}

/// The line before `start`, when it is a comment ending with [`PAPERDOLL_WINDOW_MARK`]: the heading of a cover the
/// paperdoll window wrote, with the blank line the window writes before it.
fn window_heading(text: &str, start: usize) -> Option<Range<usize>> {
    let before = text[..start].strip_suffix('\n')?;
    let heading_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    let heading = before[heading_start..].trim();
    if !(heading.starts_with("//") && heading.ends_with(PAPERDOLL_WINDOW_MARK)) {
        return None;
    }
    let blank_before = text[..heading_start]
        .strip_suffix('\n')
        .is_some_and(|earlier| earlier.is_empty() || earlier.ends_with('\n'));
    Some(heading_start - usize::from(blank_before)..start)
}

/// The string value of property `name` and its span.
fn string_property<'a>(node: &'a Node<Span>, name: &str) -> Option<(&'a str, Span)> {
    let (_, value) = node.properties.iter().find(|(key, _)| key.as_ref() == name)?;
    match &*value.literal {
        Literal::String(text) => Some((text, value.literal.span().clone())),
        _ => None,
    }
}

/// `text`, a `PaperdollOverrides.kdl`, edited so the garment of `edit` is shown, hidden or left to the slot rules and
/// the stored decision beneath its covering item on its body, keeping every comment and every other cover as written.
/// Showing or hiding takes the garment out of the window's covers for that item on that body (garment rules alone) and
/// writes the rule in the last of them, when no other cover that applies names the garment after it, else in such a
/// cover added at the end: the choice decides and the user's covers for other items and bodies stay. Back to measured
/// takes the garment out of every cover that applies, the user's too. A cover left with no rule goes: with garment
/// rules alone it decided no slot. Fails on a file the overrides do not read.
pub fn set_garment_rule(source_name: &str, text: &str, edit: &GarmentRuleEdit) -> eyre::Result<String> {
    let overrides = PaperdollOverrides::decode(source_name, text)?;
    let document = uot_kdl::parse_ast(source_name, text)
        .map_err(|error| uot_kdl::strict::in_file(source_name, uot_kdl::strict::describe_error(text, &error)))?;
    let (covering, body, garment) = (edit.covering, edit.body, edit.garment.1);
    let measured = edit.choice == GarmentChoice::Measured;
    // The covers the garment's rules go from.
    let clears = |cover: &Cover| match measured {
        true => cover.applies(covering, Some(body)),
        false => cover.is_window_cover(covering, body),
    };
    let last_naming_other = overrides.covers.iter().rposition(|cover| {
        cover.applies(covering, Some(body)) && !clears(cover) && cover.garment_rule(garment).is_some()
    });
    let target = (!measured)
        .then(|| overrides.covers.iter().rposition(|cover| cover.is_window_cover(covering, body)))
        .flatten()
        .filter(|&index| last_naming_other.is_none_or(|other| index > other))
        .filter(|&index| document.nodes[index].children.is_some());

    let mut edits = Vec::new();
    for (index, (cover, node)) in overrides.covers.iter().zip(&document.nodes).enumerate() {
        if !clears(cover) || cover.garment_rule(garment).is_none() {
            continue;
        }
        let children: Vec<_> = node.children().collect();
        let mut removed = 0;
        let mut node_edits = Vec::new();
        for child in &children {
            let name: &str = &child.node_name;
            if name != SHOW_NODE && name != HIDE_NODE {
                continue;
            }
            let Some((items, span)) = string_property(child, ITEMS_PROPERTY) else { continue };
            let parts: Vec<&str> = items.split(ID_SEPARATOR).map(str::trim).collect();
            let kept: Vec<&str> = parts.iter().copied().filter(|part| parse_number(part) != Some(garment)).collect();
            if kept.len() == parts.len() {
                continue;
            }
            if kept.is_empty() {
                removed += 1;
                node_edits.push(TextEdit { range: node_removal(text, child.span()), text: String::new() });
            } else {
                node_edits.push(TextEdit { range: span.0..span.1, text: format!("\"{}\"", kept.join(",")) });
            }
        }
        // A cover left with excepts alone, or nothing, would keep everything: it goes. Not the cover the rule goes to.
        if removed == rule_count(&children) && target != Some(index) {
            let mut range = node_removal(text, node.span());
            if let Some(heading) = window_heading(text, range.start) {
                range.start = heading.start;
            }
            edits.push(TextEdit { range, text: String::new() });
        } else {
            edits.extend(node_edits);
        }
    }

    let rule_node = match edit.choice {
        GarmentChoice::Show => Some(SHOW_NODE),
        GarmentChoice::Hide => Some(HIDE_NODE),
        GarmentChoice::Measured => None,
    };
    if let Some(rule_node) = rule_node {
        let rule = format!("{rule_node} {ITEMS_PROPERTY}=\"{}\"", item_text(garment));
        let comment = edit.garment_name.map(comment_text).filter(|name| !name.is_empty());
        match target {
            Some(index) => edits.extend(insert_rule(text, &document.nodes[index], &rule, comment.as_deref())),
            None => {
                let mut cover = String::new();
                if !text.is_empty() && !text.ends_with('\n') {
                    cover.push('\n');
                }
                let covering_text = comment_text(&named(covering.1, edit.covering_name));
                let _ = writeln!(cover, "\n// {covering_text} on body {body}, {PAPERDOLL_WINDOW_MARK}");
                let _ = writeln!(
                    cover,
                    "{COVER_NODE} \"{}\" {ITEMS_PROPERTY}=\"{}\" {BODIES_PROPERTY}=\"{body}\" {{",
                    SLOT_NAMES[covering.0],
                    item_text(covering.1)
                );
                let _ = write!(cover, "{RULE_INDENT}{rule}");
                match &comment {
                    Some(comment) => {
                        let _ = writeln!(cover, " // {comment}");
                    }
                    None => cover.push('\n'),
                }
                cover.push_str("}\n");
                edits.push(TextEdit { range: text.len()..text.len(), text: cover });
            }
        }
    }

    // From the end, so the ranges before stay valid; a removal ending where an insertion starts goes first.
    edits.sort_by_key(|edit| std::cmp::Reverse((edit.range.start, edit.range.end)));
    let mut edited = text.to_string();
    for TextEdit { range, text } in edits {
        edited.replace_range(range, &text);
    }

    let reread = PaperdollOverrides::decode(source_name, &edited)?;
    let decided = reread.decide(covering, Some(body), edit.garment);
    let holds = match edit.choice {
        GarmentChoice::Show => decided.is_some_and(|d| d.rule == OverrideRule::Garment && !d.takes_off),
        GarmentChoice::Hide => decided.is_some_and(|d| d.rule == OverrideRule::Garment && d.takes_off),
        GarmentChoice::Measured => decided.is_none_or(|d| d.rule == OverrideRule::Slot),
    };
    eyre::ensure!(holds, "{source_name}: the edited file does not decide {} as chosen", item_text(garment));
    Ok(edited)
}

/// The rules among a cover's children: `show` and `hide` nodes.
fn rule_count(children: &[&knuffel::ast::SpannedNode<Span>]) -> usize {
    children.iter().filter(|child| &**child.node_name == SHOW_NODE || &**child.node_name == HIDE_NODE).count()
}

/// An edit adding `rule` to the children of `cover`: as the last, on a line of its own with `comment`, when the closing
/// brace is on a line of its own; else first, right after the opening brace. `None` without children.
fn insert_rule(text: &str, cover: &Node<Span>, rule: &str, comment: Option<&str>) -> Option<TextEdit> {
    // The children's span runs from the opening brace to the closing one, included.
    let braces = cover.children.as_ref()?.span();
    let (open, close) = (braces.0 + 1, braces.1 - 1);
    let line_start = text[..close].rfind('\n').map_or(0, |newline| newline + 1);
    if line_start >= open && text[line_start..close].trim().is_empty() {
        let mut line = format!("{RULE_INDENT}{rule}");
        if let Some(comment) = comment {
            let _ = write!(line, " // {comment}");
        }
        line.push('\n');
        return Some(TextEdit { range: line_start..line_start, text: line });
    }
    Some(TextEdit { range: open..open, text: format!(" {rule};") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paperdoll_ec::{PaperdollEcDressDecision, PaperdollEcKeptGarment};

    const ROBE: u32 = 0x1F03;
    const EPAULETS: u32 = 39301;
    const SHIRT: u32 = 0x1517;
    const SASH: u32 = 5441;
    const PLATE_LEGS: u32 = 0x1411;
    const HUMAN_MALE: u32 = 400;
    const HUMAN_FEMALE: u32 = 401;
    const ELF_MALE: u32 = 605;

    fn overrides(content: &str) -> PaperdollOverrides {
        PaperdollOverrides::decode(PAPERDOLL_OVERRIDES_FILE_NAME, content).unwrap()
    }

    fn overrides_error(content: &str) -> String {
        format!("{:#}", PaperdollOverrides::decode(PAPERDOLL_OVERRIDES_FILE_NAME, content).unwrap_err())
    }

    fn slot(name: &str) -> usize {
        slot_layer(name).unwrap()
    }

    /// Whether the overrides take `garment` off beneath the dress-layer item `covering` on `body`; `None` when they
    /// leave it to the stored decision.
    fn takes_off(overrides: &PaperdollOverrides, covering: u32, body: u32, garment: (&str, u32)) -> Option<bool> {
        overrides.decide((slot("dress"), covering), Some(body), (slot(garment.0), garment.1)).map(|d| d.takes_off)
    }

    /// The last cover that applies to an item replaces the stored decision; an exception falls back to the covers
    /// before it, then to the stored decision; a cover without hides keeps everything.
    #[test]
    fn the_last_cover_that_applies_replaces_the_stored_decision() {
        let covers = overrides(
            "cover \"dress\" items=\"any\" {\n    hide \"torso\"\n    except items=\"39301\"\n}\n\
             cover \"dress\" items=\"0x1F03\" {\n    hide \"legs\"\n}\n\
             cover \"dress\" items=\"0x1F04\"\n",
        );
        let shirt = ("torso", SHIRT);
        let plate_legs = ("legs", PLATE_LEGS);
        let on = |covering, garment| takes_off(&covers, covering, HUMAN_MALE, garment);
        assert_eq!((on(ROBE, plate_legs), on(ROBE, shirt)), (Some(true), Some(false)));
        assert_eq!(on(ROBE + 1, shirt), Some(false), "a cover without hides");
        assert_eq!((on(SASH, shirt), on(SASH, plate_legs)), (Some(true), Some(false)), "the first cover");
        assert_eq!(on(EPAULETS, shirt), None, "an exception");
        let cape = covers.decide((slot("cape"), ROBE), Some(HUMAN_MALE), (slot("torso"), SHIRT));
        assert_eq!(cape, None, "another slot: the stored decision");
        assert_eq!(takes_off(&PaperdollOverrides::default(), ROBE, HUMAN_MALE, shirt), None);
    }

    /// Garment rules beat slot rules, which beat the stored decision; among the covers that apply, the last one with a
    /// garment rule for the garment decides it, and the last one with a slot rule the rest. A cover with garment rules
    /// alone decides no slot.
    #[test]
    fn garment_rules_beat_slot_rules() {
        const DOUBLET: u32 = 0x1F7B;
        let covers = overrides(
            "cover \"dress\" items=\"any\" {\n    hide \"torso\"\n    show items=\"0x1517\"\n}\n\
             cover \"dress\" items=\"0x1F03\" {\n    hide items=\"0x1517,0x1F7B\"\n}\n\
             cover \"dress\" items=\"0x1F03\" {\n    show items=\"0x1F7B\"\n}\n",
        );
        let on = |covering, garment| takes_off(&covers, covering, HUMAN_MALE, garment);
        assert_eq!(on(ROBE, ("torso", SHIRT)), Some(true), "the robe's hide beats the first cover's show");
        assert_eq!(on(ROBE, ("torso", DOUBLET)), Some(false), "the last garment rule");
        assert_eq!(on(ROBE, ("torso", SASH)), Some(true), "the first cover's slot rule");
        assert_eq!(on(ROBE, ("legs", PLATE_LEGS)), Some(false), "the first cover keeps the legs");
        assert_eq!(on(ROBE + 1, ("torso", SHIRT)), Some(false), "shown beneath any other item");
        let decided = covers.decide((slot("dress"), ROBE), Some(HUMAN_MALE), (slot("torso"), DOUBLET)).unwrap();
        assert_eq!((decided.rule, decided.line), (OverrideRule::Garment, 8));
        let garment_rules_alone = overrides("cover \"dress\" items=\"0x1F03\" {\n    hide items=\"0x1517\"\n}\n");
        assert_eq!(takes_off(&garment_rules_alone, ROBE, HUMAN_MALE, ("torso", SASH)), None);
    }

    /// A cover with bodies applies on those body frames alone, and on none when no body frame is drawn.
    #[test]
    fn a_cover_with_bodies_applies_on_those_bodies() {
        let covers = overrides("cover \"dress\" items=\"0x1F03\" bodies=\"400,605\" {\n    hide \"torso\"\n}\n");
        let shirt = ("torso", SHIRT);
        assert_eq!(takes_off(&covers, ROBE, HUMAN_MALE, shirt), Some(true));
        assert_eq!(takes_off(&covers, ROBE, ELF_MALE, shirt), Some(true));
        assert_eq!(takes_off(&covers, ROBE, HUMAN_FEMALE, shirt), None);
        assert_eq!(covers.decide((slot("dress"), ROBE), None, (slot("torso"), SHIRT)), None);
    }

    #[test]
    fn overrides_reject_unknown_nodes_slots_and_items() {
        assert_eq!(
            overrides_error("covers \"dress\" items=\"any\"\n"),
            "PaperdollOverrides.kdl: line 1, column 1: unknown node `covers` at the top level: expected `cover`"
        );
        assert_eq!(
            overrides_error("cover \"dress\" items=\"any\" {\n    hide \"torso\" when=\"always\"\n}\n"),
            "PaperdollOverrides.kdl: line 2, column 18: unknown property `when` of `hide`: expected `items`"
        );
        assert_eq!(
            overrides_error("cover \"dress\" items=\"any\" {\n    unhide \"torso\"\n}\n"),
            "PaperdollOverrides.kdl: line 2, column 5: unknown node `unhide` in `cover`: expected one of `hide`, `show`, \
             `except`"
        );
        assert!(overrides_error("cover \"robe\" items=\"any\"\n").contains("line 1: unknown slot `robe`"));
        assert!(overrides_error("cover \"dress\" items=\"any\" {\n    hide \"shirt\"\n}\n").contains("unknown slot `shirt`"));
        assert!(overrides_error("cover \"dress\" items=\"any\" {\n    hide \"dress\"\n}\n").contains("hides its own slot"));
        assert!(overrides_error("cover \"dress\" items=\"robes\"\n").contains("`robes` is not an item id"));
        assert!(overrides_error("cover \"dress\" items=\"any\" {\n    except items=\"any\"\n}\n").contains("`any` is not an item id"));
        assert!(overrides_error("cover \"dress\"\n").contains(ITEMS_PROPERTY));
        assert!(overrides_error("cover \"dress\" items=\"any\" bodies=\"human\"\n").contains("`human` is not a body"));
        assert!(overrides_error("cover \"dress\" items=\"any\" {\n    hide \"torso\" items=\"0x1517\"\n}\n")
            .contains("a slot or items=, not both"));
        assert!(overrides_error("cover \"dress\" items=\"any\" {\n    hide\n}\n").contains("needs a slot or items="));
        assert!(overrides_error("cover \"dress\" items=\"any\" {\n    show items=\"0x1517\"\n    hide items=\"5399\"\n}\n")
            .contains("item 0x1517 is both shown and hidden"));
        assert!(!overrides_error("cover \"dress\" items=\"any\" {\n    show \"torso\"\n}\n").is_empty());
    }

    fn kept(item_id: u32) -> PaperdollEcKeptGarment {
        const BEYOND_EDGE: u32 = 2000;
        PaperdollEcKeptGarment { item_id, beyond_edge: BEYOND_EDGE }
    }

    /// The robe keeps the shirt on the human male and female and nothing on the elf; the sash item keeps plate legs and
    /// a doublet. Ascending by item and body, as stored.
    fn stored() -> PaperdollEcDressDecisions {
        let decision = |item_id, body, garments: &[u32]| PaperdollEcDressDecision {
            item_id,
            body,
            kept: garments.iter().copied().map(kept).collect(),
        };
        PaperdollEcDressDecisions {
            slots: layer_mask([slot("torso"), slot("legs")]),
            decisions: vec![
                decision(SASH, HUMAN_MALE as u16, &[PLATE_LEGS, 0x1F7B]),
                decision(ROBE, HUMAN_MALE as u16, &[SHIRT]),
                decision(ROBE, HUMAN_FEMALE as u16, &[SHIRT]),
                decision(ROBE, ELF_MALE as u16, &[]),
            ],
        }
    }

    /// Used as the overrides, the exported covers decide every garment as the stored decisions do, without them: the
    /// garments kept stay, the others of the decisions' slots go, and nothing is decided where nothing is stored.
    #[test]
    fn the_exported_decisions_decide_as_the_stored_ones() {
        let stored = stored();
        let names = |item_id| (item_id == SHIRT).then(|| "fancy shirt".to_string());
        let text = export_dress_decisions(&stored, "paperdoll_ec.uddp", names);
        assert!(text.contains("cover \"dress\" items=\"0x1F03\" bodies=\"400,401\" {"), "{text}");
        assert!(text.contains("    show items=\"0x1517\" // fancy shirt\n"), "{text}");
        assert!(text.contains("cover \"dress\" items=\"0x1F03\" bodies=\"605\" {\n    hide \"legs\"\n    hide \"torso\"\n}"));
        let exported = overrides(&text);
        let garments = [("torso", SHIRT), ("torso", 0x1F7B), ("legs", PLATE_LEGS), ("legs", 0x1539), ("cape", 0x1515)];
        for covering in [ROBE, SASH, EPAULETS] {
            for body in [HUMAN_MALE, HUMAN_FEMALE, ELF_MALE, ELF_MALE + 1] {
                for (layer, garment) in garments {
                    let decided = takes_off(&exported, covering, body, (layer, garment)).unwrap_or(false);
                    let expected = stored.takes_off(covering, body, slot(layer), garment);
                    assert_eq!(decided, expected, "{covering:#x} on {body}: {layer} {garment:#x}");
                }
            }
        }
    }

    /// The exported covers of a real package decide every garment of the decisions' slots the item definitions
    /// know as the stored decisions do.
    #[test]
    #[ignore = "needs packages: set UDD_PAPERDOLL_EC and UDD_TILEMETA, and run with --ignored"]
    fn a_real_package_exports_its_decisions() {
        use uocf::enhanced::paperdoll::item_layer;
        let env_path = |name: &str| std::path::PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
        let package = crate::PaperdollEcPackage::load(env_path("UDD_PAPERDOLL_EC")).unwrap();
        let tilemeta = crate::tilemeta::TileMetaPackage::load(env_path("UDD_TILEMETA")).unwrap();
        let stored = package.dress_decisions();
        let name = |item_id| tilemeta.item_tile(item_id).map(|tile| tile.name_ascii().to_string());
        let exported = overrides(&export_dress_decisions(stored, "paperdoll_ec.uddp", name));
        let definitions = tilemeta.item_definitions();
        let garments: Vec<(usize, u32)> = definitions
            .entry_ids()
            .filter_map(|item_id| Some((item_layer(definitions, item_id, None)?, item_id)))
            .filter(|&(layer, _)| stored.covers(layer))
            .collect();
        let dress = slot(DRESS_SLOT);
        let mut compared = 0usize;
        for decision in &stored.decisions {
            let body = u32::from(decision.body);
            for &garment in &garments {
                let decided = exported.decide((dress, decision.item_id), Some(body), garment).map(|d| d.takes_off);
                assert_eq!(
                    decided,
                    Some(stored.takes_off(decision.item_id, body, garment.0, garment.1)),
                    "{:#x} on {body}: {garment:x?}",
                    decision.item_id
                );
                compared += 1;
            }
        }
        assert!(compared > 0);
        println!("{} decisions, {} garments each, {compared} compared", stored.decisions.len(), garments.len());
    }

    const SHIRT_NAME: &str = "fancy shirt";

    fn edit(covering: u32, garment: (&str, u32), choice: GarmentChoice) -> GarmentRuleEdit<'static> {
        GarmentRuleEdit {
            covering: (slot("dress"), covering),
            covering_name: Some("robe"),
            body: HUMAN_MALE,
            garment: (slot(garment.0), garment.1),
            garment_name: Some(SHIRT_NAME),
            choice,
        }
    }

    fn edited(text: &str, edit: &GarmentRuleEdit) -> String {
        set_garment_rule(PAPERDOLL_OVERRIDES_FILE_NAME, text, edit).unwrap()
    }

    /// The window's choices go to a cover of their own at the end, which later choices reuse; the user's comments and
    /// covers stay as written, and back to measured takes the window's cover away with its heading once it is empty.
    #[test]
    fn the_window_writes_its_choices_keeping_the_file() {
        const USER: &str = "// My robes.\ncover \"dress\" items=\"any\" {\n    hide \"torso\" // all shirts\n}\n";
        let shirt = ("torso", SHIRT);
        let shown = edited(USER, &edit(ROBE, shirt, GarmentChoice::Show));
        assert_eq!(
            shown,
            format!(
                "{USER}\n// robe (0x1F03) on body 400, {PAPERDOLL_WINDOW_MARK}\ncover \"dress\" items=\"0x1F03\" \
                 bodies=\"400\" {{\n    show items=\"0x1517\" // {SHIRT_NAME}\n}}\n"
            )
        );
        assert_eq!(takes_off(&overrides(&shown), ROBE, HUMAN_MALE, shirt), Some(false));

        let doublet = ("torso", 0x1F7B);
        let both = edited(&shown, &GarmentRuleEdit { garment_name: Some("doublet"), ..edit(ROBE, doublet, GarmentChoice::Hide) });
        assert!(both.ends_with(
            "    show items=\"0x1517\" // fancy shirt\n    hide items=\"0x1F7B\" // doublet\n}\n"
        ), "{both}");
        let hidden = edited(&both, &edit(ROBE, shirt, GarmentChoice::Hide));
        assert!(hidden.ends_with("    hide items=\"0x1F7B\" // doublet\n    hide items=\"0x1517\" // fancy shirt\n}\n"), "{hidden}");
        assert_eq!(takes_off(&overrides(&hidden), ROBE, HUMAN_MALE, shirt), Some(true));

        let measured = edited(&hidden, &edit(ROBE, shirt, GarmentChoice::Measured));
        let measured = edited(&measured, &edit(ROBE, doublet, GarmentChoice::Measured));
        assert_eq!(measured, USER, "the window's cover goes with its heading");
        let decided = overrides(&measured).decide((slot("dress"), ROBE), Some(HUMAN_MALE), (slot("torso"), SHIRT));
        assert_eq!(decided.map(|d| d.rule), Some(OverrideRule::Slot), "the user's slot rule");
    }

    /// Back to measured takes the garment out of every garment rule that applies, keeping the other ids and the covers
    /// that keep a rule; a cover left with no rule goes. A rule written later in a cover of the user's that is not the
    /// window's goes to a new cover, so it decides.
    #[test]
    fn back_to_measured_leaves_the_other_rules() {
        let user = "cover \"dress\" items=\"0x1F03\" {\n    hide \"legs\"\n    show items=\"0x1517, 0x1F7B\"\n}\n\
                    cover \"dress\" items=\"any\" { show items=\"0x1517\"; except items=\"0x1F04\"; }\n// end\n";
        let measured = edited(user, &edit(ROBE, ("torso", SHIRT), GarmentChoice::Measured));
        assert_eq!(measured, "cover \"dress\" items=\"0x1F03\" {\n    hide \"legs\"\n    show items=\"0x1F7B\"\n}\n// end\n");
        let decided = overrides(&measured).decide((slot("dress"), ROBE), Some(HUMAN_MALE), (slot("torso"), SHIRT));
        assert_eq!(decided.map(|d| (d.takes_off, d.rule)), Some((false, OverrideRule::Slot)));

        let one_line = "cover \"dress\" items=\"0x1F03\" bodies=\"400\" { show items=\"0x1F7B\"; }\n";
        let added = edited(one_line, &edit(ROBE, ("torso", SHIRT), GarmentChoice::Hide));
        assert_eq!(added, "cover \"dress\" items=\"0x1F03\" bodies=\"400\" { hide items=\"0x1517\"; show items=\"0x1F7B\"; }\n");
    }

    /// Showing or hiding for one item on one body leaves the user's rules for other items and bodies: when a cover of
    /// the user's that applies names the garment after the window's cover, the rule goes to a new cover after it.
    #[test]
    fn a_choice_leaves_the_users_other_rules() {
        let user = "cover \"dress\" items=\"0x1F03\" bodies=\"400\" {\n    show items=\"0x1F7B\"\n}\n\
                    cover \"dress\" items=\"any\" {\n    hide items=\"0x1517\"\n}\n";
        let shirt = ("torso", SHIRT);
        let shown = edited(user, &edit(ROBE, shirt, GarmentChoice::Show));
        assert!(shown.starts_with(user), "{shown}");
        let reread = overrides(&shown);
        assert_eq!(takes_off(&reread, ROBE, HUMAN_MALE, shirt), Some(false));
        assert_eq!(takes_off(&reread, SASH, HUMAN_MALE, shirt), Some(true), "other items");
        assert_eq!(takes_off(&reread, ROBE, HUMAN_FEMALE, shirt), Some(true), "other bodies");
    }

    /// A file the overrides do not read is not edited.
    #[test]
    fn an_invalid_file_is_not_edited() {
        let error = set_garment_rule(PAPERDOLL_OVERRIDES_FILE_NAME, "cover \"robe\" items=\"any\"\n", &edit(ROBE, ("torso", SHIRT), GarmentChoice::Show))
            .unwrap_err();
        assert!(format!("{error:#}").contains("unknown slot `robe`"));
    }
}
