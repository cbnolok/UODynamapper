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
//!     apply-in x1=0 y1=0 x2=0x17FF y2=0xFFFF   // only west of x 0x1800, on every map
//!     item 0x0C37 0x0C38 id=0x1BAE
//! }
//! ```
//!
//! `land` and `item` take one id, or two: the first and last of a range. A land route's `source` defaults to the Land
//! Texture Source preference and its `id` to the routed id; `terrain` names an Enhanced-layout terrain definition in
//! place of `id`. An item route needs `id`. Within one block an id is routed once; the profile's route of an id
//! replaces the `routes` block's.
//!
//! A profile's areas (the user's design, 2026-11-12) limit where its routes apply, by the position of each land tile
//! and static: `apply-in x1= y1= x2= y2= [map=<n>]` and `skip-in` with the same properties name rectangles of tiles,
//! corners included, on the map `n` or on every map without `map`; the part of a rectangle past the map's edge covers
//! nothing. On a map, the profile applies at a tile when no `apply-in` covers the map or one contains the tile, and no
//! `skip-in` contains it: `skip-in` wins. Elsewhere the `routes` block alone applies. The Classic client applies
//! desolation only while the player's x is below 0x1800, on every map (`FUN_0045c5a0`); the bundled file applies it
//! to the statics west of that line, and a custom map may need other areas.

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
const APPLY_IN_NODE: &str = "apply-in";
const SKIP_IN_NODE: &str = "skip-in";
const X1_PROPERTY: &str = "x1";
const Y1_PROPERTY: &str = "y1";
const X2_PROPERTY: &str = "x2";
const Y2_PROPERTY: &str = "y2";
const MAP_PROPERTY: &str = "map";
const AREA_PROPERTIES: &[&str] = &[X1_PROPERTY, Y1_PROPERTY, X2_PROPERTY, Y2_PROPERTY, MAP_PROPERTY];

/// Map ids an area can name: a map plane's index (`UOVec4::m`).
pub const PROFILE_MAP_ID_COUNT: usize = 1 << u8::BITS;

const LAND_ROUTE_SCHEMA: NodeSchema = NodeSchema::leaf(LAND_NODE, &[SOURCE_PROPERTY, ID_PROPERTY, TERRAIN_PROPERTY]);
const ITEM_ROUTE_SCHEMA: NodeSchema = NodeSchema::leaf(ITEM_NODE, &[ID_PROPERTY]);

/// The nodes of the `routes` block.
const ROUTE_BLOCK_CHILDREN: &[NodeSchema] = &[LAND_ROUTE_SCHEMA, ITEM_ROUTE_SCHEMA];
/// The nodes of a profile: its routes and where they apply.
const PROFILE_CHILDREN: &[NodeSchema] = &[
    LAND_ROUTE_SCHEMA,
    ITEM_ROUTE_SCHEMA,
    NodeSchema::leaf(APPLY_IN_NODE, AREA_PROPERTIES),
    NodeSchema::leaf(SKIP_IN_NODE, AREA_PROPERTIES),
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

/// A rectangle of tiles, its corners included; `x1 <= x2` and `y1 <= y2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileRect {
    pub x1: u16,
    pub y1: u16,
    pub x2: u16,
    pub y2: u16,
}

impl TileRect {
    /// Whether the tile at `x`, `y` lies in the rectangle. A map's tiles are all it can contain, so the part of the
    /// rectangle past the map's edge covers nothing.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        (u32::from(self.x1)..=u32::from(self.x2)).contains(&x) && (u32::from(self.y1)..=u32::from(self.y2)).contains(&y)
    }
}

/// An `apply-in` or `skip-in` node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteArea {
    /// `None`: every map.
    pub map: Option<u8>,
    pub rect: TileRect,
    pub line: usize,
}

/// Where a profile's routes apply: its `apply-in` and `skip-in` areas, in file order. Empty: everywhere.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouteAreas {
    pub apply_in: Vec<RouteArea>,
    pub skip_in: Vec<RouteArea>,
}

impl RouteAreas {
    pub fn is_empty(&self) -> bool {
        self.apply_in.is_empty() && self.skip_in.is_empty()
    }

    /// The areas on the map `map`: those naming it and those naming no map.
    pub fn on_map(&self, map: u32) -> MapRouteAreas {
        let rects = |areas: &[RouteArea]| areas.iter()
            .filter(|area| area.map.is_none_or(|area_map| u32::from(area_map) == map))
            .map(|area| area.rect).collect();
        MapRouteAreas { apply_in: rects(&self.apply_in), skip_in: rects(&self.skip_in) }
    }

    /// Whether the profile applies at the tile `x`, `y` of the map `map` ([`MapRouteAreas::contains`]).
    pub fn contains(&self, map: u32, x: u32, y: u32) -> bool {
        self.on_map(map).contains(x, y)
    }
}

/// A profile's areas on one map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapRouteAreas {
    apply_in: Vec<TileRect>,
    skip_in: Vec<TileRect>,
}

impl MapRouteAreas {
    /// Whether the profile applies at the tile `x`, `y`: inside an `apply-in`, or anywhere when the map has none, and
    /// inside no `skip-in`.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        (self.apply_in.is_empty() || self.apply_in.iter().any(|rect| rect.contains(x, y)))
            && !self.skip_in.iter().any(|rect| rect.contains(x, y))
    }
}

/// A named set of routes, chosen at run time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteProfile {
    pub name: String,
    /// Where the profile's routes apply.
    pub areas: RouteAreas,
    pub block: RouteBlock,
}

/// Route tables by where they apply: `inside` a profile's areas, `outside` elsewhere.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AreaRoutes<T> {
    /// The routes where the profile does not apply: the `routes` block's, and the profile's too when it applies
    /// everywhere.
    pub outside: T,
    /// The routes where the profile applies, the profile's in front of the `routes` block's; `None` when the profile
    /// has no area or its routes add nothing to `outside`.
    pub inside: Option<T>,
    /// Where `inside` applies; empty when `inside` is `None`.
    pub areas: RouteAreas,
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

    /// [`Self::land_routes`] inside the profile's areas and the `routes` block's outside them.
    pub fn land_routes_by_area(&self, profile: Option<&str>) -> AreaRoutes<Vec<Option<LandRoute>>> {
        self.routes_by_area(profile, Self::land_routes)
    }

    /// [`Self::item_routes`] inside the profile's areas and the `routes` block's outside them, each a table of every
    /// static graphic; `None` when no item is routed anywhere.
    pub fn item_routes_by_area(&self, profile: Option<&str>) -> Option<AreaRoutes<Vec<u16>>> {
        let routes = self.routes_by_area(profile, Self::item_routes);
        if routes.outside.is_none() && routes.inside.is_none() {
            return None;
        }
        let identity = || (0..ITEM_ROUTE_ID_COUNT).map(|id| id as u16).collect();
        Some(AreaRoutes {
            outside: routes.outside.unwrap_or_else(identity),
            inside: routes.inside.map(|inside| inside.unwrap_or_else(identity)),
            areas: routes.areas,
        })
    }

    /// The tables `routes` gives without the profile and with it, split by the profile's areas.
    fn routes_by_area<T: PartialEq>(&self, profile: Option<&str>, routes: impl Fn(&Self, Option<&str>) -> T)
        -> AreaRoutes<T> {
        let profile = profile.and_then(|name| self.profile(name));
        let Some(limited) = profile.filter(|profile| !profile.areas.is_empty()) else {
            // A profile without areas applies everywhere.
            let outside = routes(self, profile.map(|profile| profile.name.as_str()));
            return AreaRoutes { outside, inside: None, areas: RouteAreas::default() };
        };
        let outside = routes(self, None);
        let inside = routes(self, Some(&limited.name));
        if inside == outside {
            return AreaRoutes { outside, inside: None, areas: RouteAreas::default() };
        }
        AreaRoutes { outside, inside: Some(inside), areas: limited.areas.clone() }
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
                // The `routes` schema has no areas.
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
                let (block, areas) = decode_block(node, PROFILE_CHILDREN)?;
                file.profiles.push(RouteProfile { name: name_text, areas, block });
            }
        }
    }
    Ok(file)
}

/// The routes of a block and its areas, which only a profile's `children` allow.
fn decode_block(block_node: &Node<LineSpan>, children: &[NodeSchema]) -> eyre::Result<(RouteBlock, RouteAreas)> {
    let block_name: &str = &block_node.node_name;
    let mut block = RouteBlock::default();
    let mut areas = RouteAreas::default();
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
            APPLY_IN_NODE => areas.apply_in.push(decode_area(node, line)?),
            _ => areas.skip_in.push(decode_area(node, line)?),
        }
    }
    Ok((block, areas))
}

/// An `apply-in` or `skip-in` node: its corners, in tiles, and its map.
fn decode_area(node: &Node<LineSpan>, line: usize) -> eyre::Result<RouteArea> {
    strict::arguments::<0>(node)?;
    let coordinate = |name| -> eyre::Result<(u16, &Value<LineSpan>)> {
        let value = strict::required_property(node, name)?;
        Ok((strict::value_u16(value, name)?, value))
    };
    let (x1, _) = coordinate(X1_PROPERTY)?;
    let (y1, _) = coordinate(Y1_PROPERTY)?;
    let (x2, x2_value) = coordinate(X2_PROPERTY)?;
    let (y2, y2_value) = coordinate(Y2_PROPERTY)?;
    for (first, last, value, first_name, last_name) in
        [(x1, x2, x2_value, X1_PROPERTY, X2_PROPERTY), (y1, y2, y2_value, Y1_PROPERTY, Y2_PROPERTY)] {
        if last < first {
            return Err(strict::error_at(value.literal.span(),
                format!("{last_name}: {last} ({last:#06X}) is less than {first_name}, {first} ({first:#06X})")));
        }
    }
    let map = strict::property(node, MAP_PROPERTY).map(|value| {
        let map = strict::value_u32(value, MAP_PROPERTY)?;
        u8::try_from(map).map_err(|_| strict::error_at(value.literal.span(),
            format!("{MAP_PROPERTY}: {map} is past the last map, {}", PROFILE_MAP_ID_COUNT - 1)))
    }).transpose()?;
    Ok(RouteArea { map, rect: TileRect { x1, y1, x2, y2 }, line })
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
        // Desolation carries the client's rule as an area: west of x 0x1800 on every map. The seasons apply
        // everywhere.
        let desolation = file.profile("desolation").unwrap();
        assert_eq!(desolation.areas.apply_in.iter().map(|area| (area.map, area.rect)).collect::<Vec<_>>(),
            [(None, TileRect { x1: 0, y1: 0, x2: 0x17FF, y2: u16::MAX })]);
        assert!(desolation.areas.skip_in.is_empty());
        assert!(file.profiles.iter().filter(|profile| profile.name != "desolation")
            .all(|profile| profile.areas.is_empty()));
        assert!(desolation.areas.contains(0, 0x17FF, 0x0FFF) && !desolation.areas.contains(0, 0x1800, 0));
        assert!(desolation.areas.contains(5, 0, 0x2000));
    }

    #[test]
    fn areas_read_with_their_lines() {
        let file = decode(
            r#"
            profile "desolation" {
                apply-in x1=0 y1=0 x2=0x17FF y2=0xFFFF
                item 0x0C37 id=0x1BAE
                skip-in x1=10 y1=20 x2=10 y2=30 map=0
                apply-in x1=5 y1=6 x2=7 y2=8 map=5
            }
            "#,
        );
        let profile = file.profile("desolation").unwrap();
        assert_eq!(profile.areas, RouteAreas {
            apply_in: vec![
                RouteArea { map: None, rect: TileRect { x1: 0, y1: 0, x2: 0x17FF, y2: 0xFFFF }, line: 3 },
                RouteArea { map: Some(5), rect: TileRect { x1: 5, y1: 6, x2: 7, y2: 8 }, line: 6 },
            ],
            skip_in: vec![RouteArea { map: Some(0), rect: TileRect { x1: 10, y1: 20, x2: 10, y2: 30 }, line: 5 }],
        });
        assert_eq!(profile.block.items, vec![ItemRouteRule { first: 0xC37, last: 0xC37, id: 0x1BAE }]);
    }

    /// On a map, a profile applies inside its `apply-in` areas for that map or for every map, or everywhere when
    /// there is none, and never inside a `skip-in`. Corners are inclusive.
    #[test]
    fn a_profile_applies_inside_its_areas_and_outside_its_holes() {
        let file = decode(
            r#"
            profile "everywhere" {}
            profile "west" {
                apply-in x1=0 y1=0 x2=0x17FF y2=0xFFFF
            }
            profile "holes" {
                skip-in x1=10 y1=10 x2=19 y2=19
                skip-in x1=100 y1=100 x2=100 y2=100 map=1
            }
            profile "per-map" {
                apply-in x1=0 y1=0 x2=99 y2=99 map=1
                apply-in x1=200 y1=200 x2=299 y2=299 map=1
                apply-in x1=50 y1=50 x2=59 y2=59 map=2
            }
            profile "overlaps" {
                apply-in x1=0 y1=0 x2=20 y2=20
                apply-in x1=10 y1=10 x2=30 y2=30
                skip-in x1=15 y1=15 x2=25 y2=25
                skip-in x1=20 y1=20 x2=40 y2=40
            }
            "#,
        );
        let applies = |name, map, x, y| file.profile(name).unwrap().areas.contains(map, x, y);
        assert!(applies("everywhere", 0, 0, 0) && applies("everywhere", 7, 0xFFFF, 0xFFFF));
        // Every map: x 0x17FF is in, 0x1800 is out.
        for map in [0, 1, 4] {
            assert!(applies("west", map, 0x17FF, 0x0FFF) && applies("west", map, 0, 0));
            assert!(!applies("west", map, 0x1800, 0) && !applies("west", map, 0x1BFF, 100));
        }
        // A `skip-in` alone: everywhere but its holes; one for map 1 only holes map 1.
        assert!(applies("holes", 0, 9, 10) && !applies("holes", 0, 10, 10) && !applies("holes", 0, 19, 19));
        assert!(applies("holes", 0, 20, 19) && applies("holes", 0, 100, 100) && !applies("holes", 1, 100, 100));
        // Areas for other maps leave a map without any applying everywhere.
        assert!(applies("per-map", 1, 99, 99) && applies("per-map", 1, 250, 200) && !applies("per-map", 1, 100, 0));
        assert!(!applies("per-map", 1, 150, 150) && applies("per-map", 2, 55, 55) && !applies("per-map", 2, 0, 0));
        assert!(applies("per-map", 0, 150, 150) && applies("per-map", 3, 0xFFFF, 0));
        // Overlapping areas: either `apply-in` holds, either `skip-in` wins.
        assert!(applies("overlaps", 0, 5, 5) && applies("overlaps", 0, 30, 12) && applies("overlaps", 0, 14, 25));
        assert!(!applies("overlaps", 0, 15, 15) && !applies("overlaps", 0, 30, 30) && !applies("overlaps", 0, 31, 31));
    }

    #[test]
    fn area_errors_name_their_line() {
        assert_error("routes {\n    apply-in x1=0 y1=0 x2=1 y2=1\n}\n", 2,
            "unknown node `apply-in` in `routes`: expected `land` or `item`");
        assert_error("profile \"a\" {\n    season 3\n}\n", 2,
            "unknown node `season` in `profile`: expected one of `land`, `item`, `apply-in`, `skip-in`");
        assert_error("profile \"a\" {\n    when player-x-below=0x1800\n}\n", 2, "unknown node `when` in `profile`");
        assert_error("profile \"a\" {\n    skip-in x=10\n}\n", 2,
            "unknown property `x` of `skip-in`: expected one of `x1`, `y1`, `x2`, `y2`, `map`");
        assert_error("profile \"a\" {\n    apply-in y1=0 x2=1 y2=1\n}\n", 2, "`apply-in`: missing x1=");
        assert_error("profile \"a\" {\n    apply-in x1=0 y1=0 x2=1\n}\n", 2, "`apply-in`: missing y2=");
        assert_error("profile \"a\" {\n    skip-in 3 x1=0 y1=0 x2=1 y2=1\n}\n", 2, "`skip-in` takes 0 arguments, found 1");
        assert_error("profile \"a\" {\n    apply-in x1=0 y1=0 x2=1 y2=1 {\n        land 1\n    }\n}\n", 3,
            "`apply-in` takes no children");
        assert_error("profile \"a\" {\n    apply-in x1=0 y1=0 x2=0x10000 y2=1\n}\n", 2, "x2: not a u16");
        assert_error("profile \"a\" {\n    apply-in x1=-1 y1=0 x2=1 y2=1\n}\n", 2, "x1: not a u16");
        assert_error("profile \"a\" {\n    apply-in x1=0 y1=\"0\" x2=1 y2=1\n}\n", 2, "y1: expected an integer");
        assert_error("profile \"a\" {\n    apply-in x1=10 y1=0 x2=9 y2=1\n}\n", 2,
            "x2: 9 (0x0009) is less than x1, 10 (0x000A)");
        assert_error("profile \"a\" {\n    land 3\n    skip-in x1=0 y1=0x1800 x2=0 y2=0x17FF\n}\n", 3,
            "y2: 6143 (0x17FF) is less than y1, 6144 (0x1800)");
        assert_error("profile \"a\" {\n    apply-in x1=0 y1=0 x2=1 y2=1 map=256\n}\n", 2, "map: 256 is past the last map, 255");
        assert_error("profile \"a\" {\n    skip-in x1=0 y1=0 x2=1 y2=1 map=-1\n}\n", 2, "map: not a u32");
        // A single tile and an area repeated read.
        decode("profile \"a\" {\n    apply-in x1=4 y1=4 x2=4 y2=4\n    apply-in x1=4 y1=4 x2=4 y2=4\n}\n");
    }

    /// Without areas the profile's routes apply everywhere; with them, the `routes` block's apply outside and the
    /// profile's in front of them inside, and a profile adding nothing needs no inside table.
    #[test]
    fn routes_split_by_the_profile_areas() {
        let file = decode(
            r#"
            routes {
                land 1 source="cc"
                item 10 id=20
            }
            profile "everywhere" {
                land 2 source="ec"
                item 11 id=21
            }
            profile "west" {
                apply-in x1=0 y1=0 x2=0x17FF y2=0xFFFF
                land 2 source="ec"
                item 11 id=21
            }
            profile "land-only" {
                skip-in x1=0 y1=0 x2=9 y2=9 map=1
                land 3
            }
            profile "nothing" {
                apply-in x1=0 y1=0 x2=9 y2=9
            }
            "#,
        );
        let everywhere = file.land_routes_by_area(Some("everywhere"));
        assert_eq!((everywhere.outside, everywhere.inside), (file.land_routes(Some("everywhere")), None));
        assert!(everywhere.areas.is_empty());
        let west = file.land_routes_by_area(Some("west"));
        assert_eq!(west.outside, file.land_routes(None));
        assert_eq!(west.inside.as_ref(), Some(&file.land_routes(Some("west"))));
        assert_eq!(west.areas, file.profile("west").unwrap().areas);
        let items = file.item_routes_by_area(Some("west")).unwrap();
        assert_eq!((items.outside[10], items.outside[11], items.outside[12]), (20, 11, 12));
        assert_eq!(items.inside.as_ref().map(|inside| (inside[10], inside[11])), Some((20, 21)));
        // A profile routing land alone keeps one item table; one routing nothing keeps no inside table.
        let land_only = file.item_routes_by_area(Some("land-only")).unwrap();
        assert_eq!((land_only.inside, land_only.areas.is_empty()), (None, true));
        assert!(file.land_routes_by_area(Some("land-only")).inside.unwrap()[3].is_some());
        assert_eq!(file.land_routes_by_area(Some("nothing")).inside, None);
        assert_eq!(file.land_routes_by_area(None), AreaRoutes { outside: file.land_routes(None), inside: None,
            areas: RouteAreas::default() });

        // Nothing routes an item anywhere.
        let plain = decode("profile \"a\" {\n    apply-in x1=0 y1=0 x2=9 y2=9\n    land 3\n}\n");
        assert_eq!(plain.item_routes_by_area(Some("a")), None);
        let profile_items = decode("profile \"a\" {\n    apply-in x1=0 y1=0 x2=9 y2=9\n    item 3 id=4\n}\n");
        let items = profile_items.item_routes_by_area(Some("a")).unwrap();
        assert_eq!((items.outside[3], items.inside.unwrap()[3]), (3, 4));
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
