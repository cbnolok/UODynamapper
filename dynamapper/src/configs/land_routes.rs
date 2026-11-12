//! The route file, `runtime_specs/land/LandRoutes.kdl` (the user's design, 2026-11-12): which source and which id each
//! land id is drawn from, and which item art each static draws, in front of the automatic land fallback chain
//! (`texture_cache::land::routing`). A `routes` block always applies; one named `profile` (a season) may be chosen at
//! run time on top of it. The file is hand-written and read strictly (`uot_kdl::strict`): every error names its line.
//!
//! ```kdl
//! routes {
//!     land 0x0003 source="ec" id=0x0004   // land 3 drawn as the Enhanced land 4
//!     land 0x0010 0x0015 source="cc"      // land 0x10 to 0x15, each as itself, from Classic
//!     land 0x0020 source="kr" terrain=12  // the Kingdom Reborn terrain definition 12
//!     item 0x0CCA id=0x0CCB               // statics 0xCCA draw the item 0xCCB
//! }
//! profile "winter" {
//!     land 0x00C4 0x00C7 source="cc" id=0x011A
//! }
//! profile "desolation" {
//!     when player-x-below=0x1800          // only while the player stands west of x 0x1800
//!     item 0x0C37 0x0C38 id=0x1BAE
//! }
//! ```
//!
//! `land` and `item` take one id, or two: the first and last of a range. A land route's `source` defaults to the Land
//! Texture Source preference and its `id` to the routed id; `terrain` names an Enhanced-layout terrain definition in
//! place of `id`. An item route needs `id`. Within one block an id is routed once; the profile's route of an id
//! replaces the `routes` block's.
//!
//! A profile's `when` nodes (the user's design, 2026-11-12) limit where it applies: `when player-x-below=<x>
//! [map=<n>]` holds while the player's x is below `x` on the map `n`, or on every map without `map`. On each map the
//! `when` naming it decides, else the `when` without `map`; a profile with `when` nodes, none of which covers the
//! map, does not apply there. A profile without `when` applies everywhere. The Classic client applies desolation
//! only while the player's x is below 0x1800, on every map (`FUN_0045c5a0`); a custom map may need another line.

use color_eyre::eyre;
use uot_kdl::ast::{Literal, Node, SpannedNode, Value};
use uot_kdl::span::LineSpan;
use uot_kdl::strict::{self, NodeSchema};

use crate::configs::settings::LandTextureSource;

/// The folder of the route file under the runtime specs (`configs::runtime_specs`).
pub const LAND_ROUTES_SPECS_FOLDER: &str = "land";
pub const LAND_ROUTES_FILE_NAME: &str = "LandRoutes.kdl";

/// Land ids a route can name: the land route table's (`routing::LAND_ROUTE_TILE_COUNT`).
pub const LAND_ROUTE_ID_COUNT: usize = 16_384;
/// Item ids a route can name: every static graphic.
pub const ITEM_ROUTE_ID_COUNT: usize = 1 << u16::BITS;

const ROUTES_NODE: &str = "routes";
const PROFILE_NODE: &str = "profile";
const LAND_NODE: &str = "land";
const ITEM_NODE: &str = "item";
const SOURCE_PROPERTY: &str = "source";
const ID_PROPERTY: &str = "id";
const TERRAIN_PROPERTY: &str = "terrain";
const WHEN_NODE: &str = "when";
const PLAYER_X_BELOW_PROPERTY: &str = "player-x-below";
const MAP_PROPERTY: &str = "map";

/// Map ids a `when` can name: the player position's map (`UOVec4::m`).
pub const PROFILE_MAP_ID_COUNT: usize = 1 << u8::BITS;

const LAND_ROUTE_SCHEMA: NodeSchema = NodeSchema::leaf(LAND_NODE, &[SOURCE_PROPERTY, ID_PROPERTY, TERRAIN_PROPERTY]);
const ITEM_ROUTE_SCHEMA: NodeSchema = NodeSchema::leaf(ITEM_NODE, &[ID_PROPERTY]);

/// The nodes of the `routes` block.
const ROUTE_BLOCK_CHILDREN: &[NodeSchema] = &[LAND_ROUTE_SCHEMA, ITEM_ROUTE_SCHEMA];
/// The nodes of a profile: its routes and where it applies.
const PROFILE_CHILDREN: &[NodeSchema] = &[
    LAND_ROUTE_SCHEMA,
    ITEM_ROUTE_SCHEMA,
    NodeSchema::leaf(WHEN_NODE, &[PLAYER_X_BELOW_PROPERTY, MAP_PROPERTY]),
];

pub const LAND_ROUTES_SCHEMA: &[NodeSchema] = &[
    NodeSchema { name: ROUTES_NODE, properties: &[], children: ROUTE_BLOCK_CHILDREN },
    NodeSchema { name: PROFILE_NODE, properties: &[], children: PROFILE_CHILDREN },
];

/// What a land route draws, within its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandRouteTarget {
    /// The routed id itself.
    SameId,
    /// Another land id.
    Id(u16),
    /// An Enhanced-layout terrain definition (`TerrainDefinition` id), drawn as the first land id that has it.
    Terrain(u32),
}

/// One land id's route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandRoute {
    /// `None`: the Land Texture Source preference.
    pub source: Option<LandTextureSource>,
    pub target: LandRouteTarget,
    /// The line of the route in the file, for the warnings about it.
    pub line: usize,
}

impl LandRoute {
    /// The land id this route draws for `id`; `None` for a terrain definition, which the land cache resolves.
    pub fn target_id(&self, id: u32) -> Option<u32> {
        match self.target {
            LandRouteTarget::SameId => Some(id),
            LandRouteTarget::Id(target) => Some(u32::from(target)),
            LandRouteTarget::Terrain(_) => None,
        }
    }
}

/// A `land` node: the first and last id of its range and their route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandRouteRule {
    pub first: u16,
    pub last: u16,
    pub route: LandRoute,
}

/// An `item` node: the first and last id of its range and the item each draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemRouteRule {
    pub first: u16,
    pub last: u16,
    pub id: u16,
}

/// The routes of one block, `routes` or a profile, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouteBlock {
    pub land: Vec<LandRouteRule>,
    pub items: Vec<ItemRouteRule>,
}

impl RouteBlock {
    pub fn is_empty(&self) -> bool {
        self.land.is_empty() && self.items.is_empty()
    }
}

/// A `when` node: where its profile applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileCondition {
    /// `None`: every map no other `when` of the profile names.
    pub map: Option<u8>,
    /// The profile applies while the player's x is below this.
    pub player_x_below: u16,
    pub line: usize,
}

/// A named set of routes, chosen at run time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteProfile {
    pub name: String,
    /// Where the profile applies; empty: everywhere.
    pub conditions: Vec<ProfileCondition>,
    pub block: RouteBlock,
}

impl RouteProfile {
    /// Whether the profile applies with the player at `x` on `map`: everywhere without `when`; else under the `when`
    /// naming the map, or the `when` without a map when none names it, and nowhere on a map no `when` covers.
    pub fn applies_at(&self, map: u8, x: u16) -> bool {
        if self.conditions.is_empty() {
            return true;
        }
        let condition = self.conditions.iter().find(|condition| condition.map == Some(map))
            .or_else(|| self.conditions.iter().find(|condition| condition.map.is_none()));
        condition.is_some_and(|condition| x < condition.player_x_below)
    }
}

/// The route file as read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LandRoutesFile {
    pub routes: RouteBlock,
    pub profiles: Vec<RouteProfile>,
}

impl LandRoutesFile {
    pub fn profile(&self, name: &str) -> Option<&RouteProfile> {
        self.profiles.iter().find(|profile| profile.name == name)
    }

    /// The route of every land id, the profile's in front of the `routes` block's; `None` for an id neither routes.
    /// A profile the file lacks adds nothing.
    pub fn land_routes(&self, profile: Option<&str>) -> Vec<Option<LandRoute>> {
        let mut routes = vec![None; LAND_ROUTE_ID_COUNT];
        for block in self.blocks(profile) {
            for rule in &block.land {
                for id in rule.first..=rule.last {
                    routes[usize::from(id)] = Some(rule.route);
                }
            }
        }
        routes
    }

    /// The item each static graphic draws, the profile's in front of the `routes` block's; `None` when no item is
    /// routed.
    pub fn item_routes(&self, profile: Option<&str>) -> Option<Vec<u16>> {
        let blocks = self.blocks(profile).collect::<Vec<_>>();
        if blocks.iter().all(|block| block.items.is_empty()) {
            return None;
        }
        let mut items = (0..ITEM_ROUTE_ID_COUNT).map(|id| id as u16).collect::<Vec<_>>();
        for block in blocks {
            for rule in &block.items {
                for id in rule.first..=rule.last {
                    items[usize::from(id)] = rule.id;
                }
            }
        }
        Some(items)
    }

    /// The `routes` block, then the profile, if the file has it.
    fn blocks<'a>(&'a self, profile: Option<&str>) -> impl Iterator<Item = &'a RouteBlock> {
        let profile = profile.and_then(|name| self.profile(name)).map(|profile| &profile.block);
        std::iter::once(&self.routes).chain(profile)
    }

    /// Reads the route file's `text`; `source_name` starts every error.
    pub fn decode(source_name: &str, text: &str) -> eyre::Result<Self> {
        let document = uot_kdl::parse_ast_line_span(source_name, text)
            .map_err(|error| strict::in_file(source_name, strict::describe_error(text, &error)))?;
        decode_document(&document.nodes).map_err(|error| strict::in_file(source_name, error))
    }
}

fn decode_document(nodes: &[SpannedNode<LineSpan>]) -> eyre::Result<LandRoutesFile> {
    let mut file = LandRoutesFile::default();
    let mut routes_line = None;
    for node in nodes {
        let schema = strict::node_schema(node, None, LAND_ROUTES_SCHEMA)?;
        strict::check_properties(node, schema.properties)?;
        let line = line_of(node);
        match schema.name {
            ROUTES_NODE => {
                strict::arguments::<0>(node)?;
                if let Some(first) = routes_line {
                    return Err(strict::error_at(node.node_name.span(),
                        format!("a second `{ROUTES_NODE}` block (the first is on line {first})")));
                }
                routes_line = Some(line);
                // The `routes` schema has no `when`.
                (file.routes, _) = decode_block(node, ROUTE_BLOCK_CHILDREN)?;
            }
            _ => {
                let [name] = strict::arguments::<1>(node)?;
                let name_text = strict::value_string(name, "the profile name")?;
                if name_text.trim().is_empty() {
                    return Err(strict::error_at(name.literal.span(), "the profile name is empty"));
                }
                if file.profile(&name_text).is_some() {
                    return Err(strict::error_at(name.literal.span(), format!("a second profile \"{name_text}\"")));
                }
                let (block, conditions) = decode_block(node, PROFILE_CHILDREN)?;
                file.profiles.push(RouteProfile { name: name_text, conditions, block });
            }
        }
    }
    Ok(file)
}

/// The routes of a block and its `when` nodes, which only a profile's `children` allow.
fn decode_block(block_node: &Node<LineSpan>, children: &[NodeSchema])
    -> eyre::Result<(RouteBlock, Vec<ProfileCondition>)> {
    let block_name: &str = &block_node.node_name;
    let mut block = RouteBlock::default();
    let mut conditions = Vec::<ProfileCondition>::new();
    // The line routing each id, to name both lines of an id routed twice.
    let mut land_lines = vec![None; LAND_ROUTE_ID_COUNT];
    let mut item_lines = vec![None; ITEM_ROUTE_ID_COUNT];
    for node in block_node.children() {
        let schema = strict::node_schema(node, Some(block_name), children)?;
        strict::check_properties(node, schema.properties)?;
        strict::reject_children(node)?;
        let line = line_of(node);
        match schema.name {
            LAND_NODE => {
                let (first, last) = id_range(node, LAND_ROUTE_ID_COUNT)?;
                let route = decode_land_route(node, line)?;
                claim_ids(node, first, last, &mut land_lines)?;
                block.land.push(LandRouteRule { first, last, route });
            }
            ITEM_NODE => {
                let (first, last) = id_range(node, ITEM_ROUTE_ID_COUNT)?;
                let id = item_id(strict::required_property(node, ID_PROPERTY)?, ID_PROPERTY)?;
                claim_ids(node, first, last, &mut item_lines)?;
                block.items.push(ItemRouteRule { first, last, id });
            }
            _ => {
                let condition = decode_condition(node, line)?;
                if let Some(earlier) = conditions.iter().find(|earlier| earlier.map == condition.map) {
                    let maps = condition.map.map_or("without map=".to_string(), |map| format!("for map {map}"));
                    return Err(strict::error_at(node.node_name.span(),
                        format!("a second `{WHEN_NODE}` {maps} (the first is on line {})", earlier.line)));
                }
                conditions.push(condition);
            }
        }
    }
    Ok((block, conditions))
}

fn decode_condition(node: &Node<LineSpan>, line: usize) -> eyre::Result<ProfileCondition> {
    strict::arguments::<0>(node)?;
    let below = strict::required_property(node, PLAYER_X_BELOW_PROPERTY)?;
    let player_x_below = strict::value_u16(below, PLAYER_X_BELOW_PROPERTY)?;
    if player_x_below == 0 {
        return Err(strict::error_at(below.literal.span(), format!("{PLAYER_X_BELOW_PROPERTY}: no x is below 0")));
    }
    let map = strict::property(node, MAP_PROPERTY).map(|value| {
        let map = strict::value_u32(value, MAP_PROPERTY)?;
        u8::try_from(map).map_err(|_| strict::error_at(value.literal.span(),
            format!("{MAP_PROPERTY}: {map} is past the last map, {}", PROFILE_MAP_ID_COUNT - 1)))
    }).transpose()?;
    Ok(ProfileCondition { map, player_x_below, line })
}

fn decode_land_route(node: &Node<LineSpan>, line: usize) -> eyre::Result<LandRoute> {
    let source = strict::property(node, SOURCE_PROPERTY).map(land_source).transpose()?;
    let id = strict::property(node, ID_PROPERTY).map(|value| bounded_id(value, ID_PROPERTY, LAND_ROUTE_ID_COUNT))
        .transpose()?;
    let terrain = strict::property(node, TERRAIN_PROPERTY);
    let target = match (id, terrain) {
        (Some(_), Some(terrain)) => {
            return Err(strict::error_at(terrain.literal.span(), "`land` takes id= or terrain=, not both"));
        }
        (Some(id), None) => LandRouteTarget::Id(id),
        (None, Some(terrain)) => {
            if source == Some(LandTextureSource::Cc) {
                return Err(strict::error_at(terrain.literal.span(),
                    "terrain= names an Enhanced-layout terrain definition: source=\"ec\" or \"kr\", not \"cc\""));
            }
            LandRouteTarget::Terrain(strict::value_u32(terrain, TERRAIN_PROPERTY)?)
        }
        (None, None) => LandRouteTarget::SameId,
    };
    Ok(LandRoute { source, target, line })
}

/// The node's id, or its first and last id.
fn id_range(node: &Node<LineSpan>, count: usize) -> eyre::Result<(u16, u16)> {
    let node_name: &str = &node.node_name;
    match node.arguments.as_slice() {
        [id] => {
            let id = bounded_id(id, "the id", count)?;
            Ok((id, id))
        }
        [first, last] => {
            let (first_id, last_id) = (bounded_id(first, "the first id", count)?, bounded_id(last, "the last id", count)?);
            if last_id < first_id {
                return Err(strict::error_at(last.literal.span(),
                    format!("the range ends at {last_id:#06X}, before its first id {first_id:#06X}")));
            }
            Ok((first_id, last_id))
        }
        arguments => {
            let span = arguments.get(2).map_or(node.node_name.span(), |extra| extra.literal.span());
            Err(strict::error_at(span,
                format!("`{node_name}` takes an id, or the first and last id of a range; found {} arguments",
                    arguments.len())))
        }
    }
}

/// An id below `count`.
fn bounded_id(value: &Value<LineSpan>, field: &str, count: usize) -> eyre::Result<u16> {
    let id = strict::value_u32(value, field)?;
    if id as usize >= count {
        return Err(strict::error_at(value.literal.span(),
            format!("{field}: {id:#06X} is past the last id, {:#06X}", count - 1)));
    }
    Ok(id as u16)
}

fn item_id(value: &Value<LineSpan>, field: &str) -> eyre::Result<u16> {
    bounded_id(value, field, ITEM_ROUTE_ID_COUNT)
}

fn land_source(value: &Value<LineSpan>) -> eyre::Result<LandTextureSource> {
    const SOURCES: [(&str, LandTextureSource); 3] =
        [("cc", LandTextureSource::Cc), ("ec", LandTextureSource::Ec), ("kr", LandTextureSource::Kr)];
    let text = match &*value.literal {
        Literal::String(text) => text,
        _ => return Err(strict::error_at(value.literal.span(), "source: expected \"cc\", \"ec\" or \"kr\"")),
    };
    SOURCES.iter().find(|(name, _)| **name == **text).map(|&(_, source)| source).ok_or_else(|| {
        strict::error_at(value.literal.span(), format!("source: unknown source \"{text}\": expected \"cc\", \"ec\" or \"kr\""))
    })
}

/// Marks the node's ids as routed on its line, or names the line that routed one of them first.
fn claim_ids(node: &Node<LineSpan>, first: u16, last: u16, lines: &mut [Option<usize>]) -> eyre::Result<()> {
    let line = line_of(node);
    for id in first..=last {
        if let Some(earlier) = lines[usize::from(id)] {
            return Err(strict::error_at(node.node_name.span(),
                format!("{id:#06X} is routed twice in this block (also on line {earlier})")));
        }
        lines[usize::from(id)] = Some(line);
    }
    Ok(())
}

/// The 1-based line of a node.
fn line_of(node: &Node<LineSpan>) -> usize {
    node.node_name.span().0.line + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "LandRoutes.kdl";

    fn decode(text: &str) -> LandRoutesFile {
        LandRoutesFile::decode(FILE, text).unwrap_or_else(|error| panic!("{error:#}"))
    }

    fn decode_error(text: &str) -> String {
        format!("{:#}", LandRoutesFile::decode(FILE, text).expect_err("the file should not read"))
    }

    /// The error starts with the file and the line, as every route file error does.
    fn assert_error(text: &str, line: usize, needle: &str) {
        let error = decode_error(text);
        assert!(error.starts_with(&format!("{FILE}: line {line}, column ")), "{error}");
        assert!(error.contains(needle), "{error}");
    }

    #[test]
    fn the_documented_example_reads() {
        let file = decode(
            r#"
            routes {
                land 0x0003 source="ec" id=0x0004
                land 0x0010 0x0015 source="cc"
                land 0x0020 source="kr" terrain=12
                item 0x0CCA id=0x0CCB
            }
            profile "winter" {
                land 0x0003 0x0006 source="ec" id=0x011A
                item 0x0CE3 id=0x0CE4
            }
            "#,
        );
        assert_eq!(file.routes.land, vec![
            LandRouteRule { first: 3, last: 3, route: LandRoute {
                source: Some(LandTextureSource::Ec), target: LandRouteTarget::Id(4), line: 3 } },
            LandRouteRule { first: 0x10, last: 0x15, route: LandRoute {
                source: Some(LandTextureSource::Cc), target: LandRouteTarget::SameId, line: 4 } },
            LandRouteRule { first: 0x20, last: 0x20, route: LandRoute {
                source: Some(LandTextureSource::Kr), target: LandRouteTarget::Terrain(12), line: 5 } },
        ]);
        assert_eq!(file.routes.items, vec![ItemRouteRule { first: 0xCCA, last: 0xCCA, id: 0xCCB }]);
        assert_eq!(file.profiles.len(), 1);
        assert_eq!(file.profiles[0].name, "winter");
        assert_eq!(file.profiles[0].block.land[0].first, 3);
        assert_eq!(file.profiles[0].block.land[0].last, 6);
    }

    /// The bundled file routes nothing outside its season profiles, which hold the Classic client's tables.
    #[test]
    fn the_bundled_file_holds_the_client_seasons() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/runtime_specs")
            .join(LAND_ROUTES_SPECS_FOLDER).join(LAND_ROUTES_FILE_NAME);
        let file = decode(&std::fs::read_to_string(path).unwrap());
        assert!(file.routes.is_empty());
        let names = file.profiles.iter().map(|profile| profile.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["spring", "summer", "fall", "winter", "desolation"]);
        let routed = |name: &str| {
            let land = file.land_routes(Some(name)).iter().filter(|route| route.is_some()).count();
            let items = file.item_routes(Some(name))
                .map_or(0, |items| items.iter().enumerate().filter(|&(id, &item)| id != usize::from(item)).count());
            (land, items)
        };
        assert_eq!([routed("spring"), routed("summer"), routed("fall"), routed("winter"), routed("desolation")],
            [(0, 17), (0, 0), (0, 27), (31, 0), (0, 72)]);
        let winter = file.land_routes(Some("winter"));
        assert_eq!(winter[0xC4].unwrap().target_id(0xC4), Some(0x11A));
        assert_eq!(winter[0xC4].unwrap().source, None);
        assert_eq!(file.item_routes(Some("desolation")).unwrap()[0x1B7E], 0x1E34);
        // Desolation carries the client's rule: west of x 0x1800 on every map. The seasons apply everywhere.
        let desolation = file.profile("desolation").unwrap();
        assert_eq!(desolation.conditions.iter().map(|condition| (condition.map, condition.player_x_below))
            .collect::<Vec<_>>(), [(None, 0x1800)]);
        assert!(file.profiles.iter().filter(|profile| profile.name != "desolation")
            .all(|profile| profile.conditions.is_empty()));
    }

    #[test]
    fn when_nodes_read_with_their_lines() {
        let file = decode(
            r#"
            profile "desolation" {
                when player-x-below=0x1800
                item 0x0C37 id=0x1BAE
                when player-x-below=0x1000 map=0
                when player-x-below=900 map=5
            }
            "#,
        );
        let profile = file.profile("desolation").unwrap();
        assert_eq!(profile.conditions, vec![
            ProfileCondition { map: None, player_x_below: 0x1800, line: 3 },
            ProfileCondition { map: Some(0), player_x_below: 0x1000, line: 5 },
            ProfileCondition { map: Some(5), player_x_below: 900, line: 6 },
        ]);
        assert_eq!(profile.block.items, vec![ItemRouteRule { first: 0xC37, last: 0xC37, id: 0x1BAE }]);
    }

    /// The `when` naming the map decides, else the one without a map; a map none covers has no profile; a profile
    /// without `when` applies everywhere. The bound is strict, as the client's `x < 0x1800`.
    #[test]
    fn a_profile_applies_where_its_when_holds() {
        let file = decode(
            r#"
            profile "everywhere" {}
            profile "west" {
                when player-x-below=0x1800
            }
            profile "per-map" {
                when player-x-below=0x1800
                when player-x-below=0x1000 map=1
            }
            profile "map-two" {
                when player-x-below=100 map=2
            }
            "#,
        );
        let applies = |name, map, x| file.profile(name).unwrap().applies_at(map, x);
        assert!(applies("everywhere", 0, u16::MAX));
        assert!(applies("west", 0, 0x17FF) && !applies("west", 0, 0x1800) && !applies("west", 0, 0x1BFF));
        assert!(applies("west", 4, 0x17FF) && !applies("west", 4, 0x1800));
        assert!(applies("per-map", 0, 0x17FF) && applies("per-map", 1, 0x0FFF) && !applies("per-map", 1, 0x1000));
        assert!(applies("map-two", 2, 99) && !applies("map-two", 2, 100) && !applies("map-two", 0, 0));
    }

    #[test]
    fn when_errors_name_their_line() {
        assert_error("routes {\n    when player-x-below=10\n}\n", 2,
            "unknown node `when` in `routes`: expected `land` or `item`");
        assert_error("profile \"a\" {\n    season 3\n}\n", 2,
            "unknown node `season` in `profile`: expected one of `land`, `item`, `when`");
        assert_error("profile \"a\" {\n    when x=10\n}\n", 2,
            "unknown property `x` of `when`: expected `player-x-below` or `map`");
        assert_error("profile \"a\" {\n    when map=0\n}\n", 2, "`when`: missing player-x-below=");
        assert_error("profile \"a\" {\n    when 3 player-x-below=10\n}\n", 2, "`when` takes 0 arguments, found 1");
        assert_error("profile \"a\" {\n    when player-x-below=10 {\n        land 1\n    }\n}\n", 3,
            "`when` takes no children");
        assert_error("profile \"a\" {\n    when player-x-below=0\n}\n", 2, "player-x-below: no x is below 0");
        assert_error("profile \"a\" {\n    when player-x-below=0x10000\n}\n", 2, "player-x-below: not a u16");
        assert_error("profile \"a\" {\n    when player-x-below=\"10\"\n}\n", 2, "player-x-below: expected an integer");
        assert_error("profile \"a\" {\n    when player-x-below=10 map=256\n}\n", 2, "map: 256 is past the last map, 255");
        assert_error("profile \"a\" {\n    when player-x-below=10 map=-1\n}\n", 2, "map: not a u32");
        assert_error("profile \"a\" {\n    when player-x-below=10\n    when player-x-below=20\n}\n", 3,
            "a second `when` without map= (the first is on line 2)");
        assert_error("profile \"a\" {\n    when player-x-below=10 map=1\n    land 3\n    when player-x-below=20 map=1\n}\n",
            4, "a second `when` for map 1 (the first is on line 2)");
    }

    #[test]
    fn an_empty_file_routes_nothing() {
        let file = decode("// nothing yet\n");
        assert!(file.routes.is_empty() && file.profiles.is_empty());
        assert!(file.land_routes(None).iter().all(Option::is_none));
        assert_eq!(file.item_routes(None), None);
    }

    #[test]
    fn a_route_without_source_or_id_keeps_the_preference_and_the_id() {
        let file = decode("routes {\n    land 7\n}\n");
        let route = file.land_routes(None)[7].unwrap();
        assert_eq!((route.source, route.target, route.target_id(7)), (None, LandRouteTarget::SameId, Some(7)));
    }

    /// The profile's route of an id replaces the `routes` block's; ids it does not route keep theirs.
    #[test]
    fn the_profile_routes_in_front_of_the_routes_block() {
        let file = decode(
            r#"
            routes {
                land 1 3 source="cc"
                item 10 11 id=20
            }
            profile "winter" {
                land 2 source="ec" id=9
                item 11 id=30
            }
            "#,
        );
        let plain = file.land_routes(None);
        let winter = file.land_routes(Some("winter"));
        assert_eq!(plain[2].unwrap().source, Some(LandTextureSource::Cc));
        assert_eq!((winter[2].unwrap().source, winter[2].unwrap().target_id(2)), (Some(LandTextureSource::Ec), Some(9)));
        assert_eq!(winter[1], plain[1]);
        assert_eq!(winter[3], plain[3]);
        assert_eq!(winter[4], None);
        // A profile the file lacks adds nothing.
        assert_eq!(file.land_routes(Some("summer")), plain);

        let items = file.item_routes(Some("winter")).unwrap();
        assert_eq!((items[9], items[10], items[11], items[12]), (9, 20, 30, 12));
        assert_eq!(file.item_routes(None).unwrap()[11], 20);
    }

    #[test]
    fn a_profile_alone_routes_items() {
        let file = decode("profile \"fall\" {\n    item 0x0CD1 id=0x0CD2\n}\n");
        assert_eq!(file.item_routes(None), None);
        assert_eq!(file.item_routes(Some("fall")).unwrap()[0xCD1], 0xCD2);
    }

    #[test]
    fn syntax_errors_name_their_line() {
        let error = decode_error("routes {\n    land 3 source=\"ec\n}\n");
        assert!(error.starts_with(&format!("{FILE}: invalid KDL: line ")), "{error}");
    }

    #[test]
    fn unknown_nodes_and_properties_name_their_line_and_the_allowed_names() {
        assert_error("routes {}\nseason \"winter\" {}\n", 2, "unknown node `season` at the top level: expected `routes` or `profile`");
        assert_error("routes {\n    static 3 id=4\n}\n", 2, "unknown node `static` in `routes`: expected `land` or `item`");
        assert_error("routes {\n    land 3 graphic=4\n}\n", 2, "unknown property `graphic` of `land`: expected one of `source`, `id`, `terrain`");
        assert_error("routes {\n    item 3 source=\"ec\" id=4\n}\n", 2, "unknown property `source` of `item`: expected `id`");
        assert_error("routes name=\"x\" {\n}\n", 1, "`routes` takes no properties");
        assert_error("routes {\n    land 3 {\n        land 4\n    }\n}\n", 3, "`land` takes no children");
    }

    #[test]
    fn arguments_are_checked() {
        assert_error("routes {\n    land\n}\n", 2, "`land` takes an id, or the first and last id of a range; found 0 arguments");
        assert_error("routes {\n    land 1 2 3\n}\n", 2, "found 3 arguments");
        assert_error("routes {\n    land \"3\"\n}\n", 2, "the id: expected an integer");
        assert_error("routes {\n    land 5 2\n}\n", 2, "the range ends at 0x0002, before its first id 0x0005");
        assert_error("routes {\n    land 0x4000\n}\n", 2, "the id: 0x4000 is past the last id, 0x3FFF");
        assert_error("routes {\n    item 0x10000 id=1\n}\n", 2, "the id: 0x10000 is past the last id, 0xFFFF");
        assert_error("routes 1 {\n}\n", 1, "`routes` takes 0 arguments, found 1");
        assert_error("profile {\n}\n", 1, "`profile` takes 1 arguments, found 0");
        assert_error("profile 3 {\n}\n", 1, "the profile name: expected a string");
        assert_error("profile \" \" {\n}\n", 1, "the profile name is empty");
    }

    #[test]
    fn values_are_checked() {
        assert_error("routes {\n    land 3 source=\"uop\"\n}\n", 2, "source: unknown source \"uop\"");
        assert_error("routes {\n    land 3 source=2\n}\n", 2, "source: expected \"cc\", \"ec\" or \"kr\"");
        assert_error("routes {\n    land 3 id=0x4000\n}\n", 2, "id: 0x4000 is past the last id");
        assert_error("routes {\n    land 3 id=4 terrain=5\n}\n", 2, "`land` takes id= or terrain=, not both");
        assert_error("routes {\n    land 3 source=\"cc\" terrain=5\n}\n", 2, "not \"cc\"");
        assert_error("routes {\n    land 3 terrain=-1\n}\n", 2, "terrain: not a u32");
        assert_error("routes {\n    item 3\n}\n", 2, "`item`: missing id=");
        assert_error("routes {\n    item 3 id=0x10000\n}\n", 2, "id: 0x10000 is past the last id, 0xFFFF");
    }

    #[test]
    fn an_id_is_routed_once_per_block_and_names_are_unique() {
        assert_error("routes {\n    land 1 5\n    land 4\n}\n", 3, "0x0004 is routed twice in this block (also on line 2)");
        assert_error("routes {\n    item 9 id=1\n    item 8 9 id=2\n}\n", 3, "0x0009 is routed twice");
        assert_error("routes {}\nroutes {}\n", 2, "a second `routes` block (the first is on line 1)");
        assert_error("profile \"a\" {}\nprofile \"a\" {}\n", 2, "a second profile \"a\"");
        // A land id and an item id do not collide, and the profile may route the block's ids again.
        decode("routes {\n    land 4\n    item 4 id=5\n}\nprofile \"a\" {\n    land 4 source=\"cc\"\n}\n");
    }
}
