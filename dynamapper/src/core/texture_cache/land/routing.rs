//! Which land source draws each land id (the user's design, 2026-11-12).
//!
//! The Land Texture Source preference picks a source; a land id whose definition or texture that source lacks is drawn
//! from the next source of its chain instead: the selected source, then the other EC-layout source (Kingdom Reborn and
//! Enhanced stand in for each other), then Classic. Classic draws the id's texmap or, without one, its land art as
//! the Classic client does (`LandView.Draw` falls back to the art when the texmap is missing). An id no source has
//! draws the missing-texture pattern. Classic selected stays Classic: it never borrows EC-layout terrain.
//!
//! The route file (`configs::land_routes`, LandRoutes.kdl) stands in front of the chain: a route names the source and
//! the id within it that draws a land id. A route its source cannot draw falls back, with a warning: to the chain of
//! its source on the id it names, then, when no source of that chain holds the id, to the selected source's chain on
//! the routed id itself.
//!
//! The chain and the summary text are independent of rendering; the land cache feeds them what each package holds.

use crate::configs::land_routes::LandRouteTarget;
use crate::configs::settings::LandTextureSource;

/// Land ids the routes cover, the EC lookup's tile capacity.
pub const LAND_ROUTE_TILE_COUNT: usize = 16_384;
/// Ids named per destination in the rerouting summary and per missing-texture warning.
const LOG_SAMPLE_IDS: usize = 8;

/// The chain a selected source draws from, in order.
pub const fn land_source_chain(selected: LandTextureSource) -> &'static [LandTextureSource] {
    match selected {
        LandTextureSource::Cc => &[LandTextureSource::Cc],
        LandTextureSource::Ec => &[LandTextureSource::Ec, LandTextureSource::Kr, LandTextureSource::Cc],
        LandTextureSource::Kr => &[LandTextureSource::Kr, LandTextureSource::Ec, LandTextureSource::Cc],
    }
}

/// What each source holds for one land id. An EC-layout source holds it when the package is loaded and the id's
/// base texture resolves to a present slot; Classic by its texmap, or by its land art.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LandTileAvailability {
    pub ec: bool,
    pub kr: bool,
    pub classic_texmap: bool,
    pub classic_art: bool,
}

/// The source one land id is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandTileRoute {
    /// An EC-layout package: tex_land_ec.uddp (`Ec`) or tex_land_kr.uddp (`Kr`).
    Enhanced(LandTextureSource),
    /// The Classic texmap (tex_land_cc.uddp).
    ClassicTexmap,
    /// The Classic land art (tex_art_cc.uddp), for a Classic id without a texmap.
    ClassicArt,
    /// No source has it: the missing-texture pattern.
    Missing,
}

impl LandTileRoute {
    fn label(self) -> &'static str {
        match self {
            Self::Enhanced(source) => source.land_label(),
            Self::ClassicTexmap => LandTextureSource::Cc.land_label(),
            Self::ClassicArt => "Classic land art (tex_art_cc.uddp)",
            Self::Missing => "the missing-texture pattern",
        }
    }
}

/// What one land id is drawn as: a route and the land id within that source whose definition, texmap or art it draws.
/// The fallback chain keeps the land id; a route of the route file can name another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandTileTarget {
    pub route: LandTileRoute,
    pub id: u32,
}

/// A route the route file asks for one land id: its source (the selected one where the file names none), what it
/// draws there, and its line in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandRouteRequest {
    pub source: LandTextureSource,
    pub target: LandRouteTarget,
    pub line: usize,
}

/// Why a route is not drawn as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandRouteMiss {
    /// Classic selected never draws EC-layout land: the selected source's chain draws the routed id.
    EnhancedUnderClassic,
    /// No land id of the source is drawn with the terrain definition: the selected source's chain draws the routed id.
    NoTerrain(u32),
    /// No source of the route's chain holds the id it names: the selected source's chain draws the routed id.
    TargetMissing(u32),
    /// The route's source lacks the id it names: the next source of its chain that holds it draws it.
    Fallback,
}

/// A route not drawn as written, for the warning about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandRouteNote {
    pub id: u32,
    pub request: LandRouteRequest,
    pub miss: LandRouteMiss,
}

/// What a route draws: the first source of its source's chain that holds the id it names.
fn resolve_request(
    selected: LandTextureSource,
    id: u32,
    request: LandRouteRequest,
    available: &impl Fn(u32) -> LandTileAvailability,
    terrain_land_id: &impl Fn(LandTextureSource, u32) -> Option<u32>,
) -> Result<LandTileTarget, LandRouteMiss> {
    if selected == LandTextureSource::Cc && request.source.uses_enhanced_land() {
        return Err(LandRouteMiss::EnhancedUnderClassic);
    }
    let target_id = match request.target {
        LandRouteTarget::SameId => id,
        LandRouteTarget::Id(target) => u32::from(target),
        LandRouteTarget::Terrain(terrain) => {
            terrain_land_id(request.source, terrain).ok_or(LandRouteMiss::NoTerrain(terrain))?
        }
    };
    match route_land_tile(request.source, available(target_id)) {
        LandTileRoute::Missing => Err(LandRouteMiss::TargetMissing(target_id)),
        route => Ok(LandTileTarget { route, id: target_id }),
    }
}

/// Whether `route` is drawn from `source` itself. Classic draws an id without a texmap from its land art.
fn is_source_route(source: LandTextureSource, route: LandTileRoute) -> bool {
    match route {
        LandTileRoute::Enhanced(enhanced) => enhanced == source,
        LandTileRoute::ClassicTexmap | LandTileRoute::ClassicArt => source == LandTextureSource::Cc,
        LandTileRoute::Missing => false,
    }
}

/// The first source of `selected`'s chain that holds the id.
pub fn route_land_tile(selected: LandTextureSource, available: LandTileAvailability) -> LandTileRoute {
    for &source in land_source_chain(selected) {
        match source {
            LandTextureSource::Ec if available.ec => return LandTileRoute::Enhanced(source),
            LandTextureSource::Kr if available.kr => return LandTileRoute::Enhanced(source),
            LandTextureSource::Cc if available.classic_texmap => return LandTileRoute::ClassicTexmap,
            LandTextureSource::Cc if available.classic_art => return LandTileRoute::ClassicArt,
            _ => {}
        }
    }
    LandTileRoute::Missing
}

/// The route of every land id under one selected source.
#[derive(Clone, Debug)]
pub struct LandRoutes {
    selected: LandTextureSource,
    /// The source whose pages the land page atlas holds: the first loaded source of the chain. An EC-layout atlas
    /// draws the ids routed to Classic through the texmap arrays; a Classic atlas is the Classic source itself.
    atlas: LandTextureSource,
    targets: Vec<LandTileTarget>,
    /// The ids a route draws, which the rerouting summary leaves out.
    routed: Vec<bool>,
    /// The routes not drawn as written.
    notes: Vec<LandRouteNote>,
}

impl LandRoutes {
    /// Routes every id under `selected`; `loaded` tells which packages are loaded, `available` what each holds.
    /// `requests` are the route file's routes, in front of the chain (the module notes); `terrain_land_id` names the
    /// first land id of an EC-layout source drawn with a terrain definition.
    pub fn resolve(
        selected: LandTextureSource,
        loaded: impl Fn(LandTextureSource) -> bool,
        available: impl Fn(u32) -> LandTileAvailability,
        requests: impl Fn(u32) -> Option<LandRouteRequest>,
        terrain_land_id: impl Fn(LandTextureSource, u32) -> Option<u32>,
    ) -> Self {
        let atlas = land_source_chain(selected).iter().copied().find(|&source| loaded(source))
            .unwrap_or(LandTextureSource::Cc);
        let mut routed = vec![false; LAND_ROUTE_TILE_COUNT];
        let mut notes = Vec::new();
        let targets = (0..LAND_ROUTE_TILE_COUNT as u32).map(|id| {
            let chain = LandTileTarget { route: route_land_tile(selected, available(id)), id };
            let Some(request) = requests(id) else { return chain };
            let note = |miss| LandRouteNote { id, request, miss };
            match resolve_request(selected, id, request, &available, &terrain_land_id) {
                Ok(target) => {
                    routed[id as usize] = true;
                    if !is_source_route(request.source, target.route) {
                        notes.push(note(LandRouteMiss::Fallback));
                    }
                    target
                }
                Err(miss) => {
                    notes.push(note(miss));
                    chain
                }
            }
        }).collect();
        Self { selected, atlas, targets, routed, notes }
    }

    pub fn selected(&self) -> LandTextureSource {
        self.selected
    }

    pub fn atlas(&self) -> LandTextureSource {
        self.atlas
    }

    /// What `id` is drawn as; ids past the route table have none of the sources.
    pub fn target(&self, id: u32) -> LandTileTarget {
        self.targets.get(id as usize).copied().unwrap_or(LandTileTarget { route: LandTileRoute::Missing, id })
    }

    /// The route of `id`.
    pub fn route(&self, id: u32) -> LandTileRoute {
        self.target(id).route
    }

    /// Whether any id is drawn through `route`.
    pub fn draws_from(&self, route: LandTileRoute) -> bool {
        self.targets.iter().any(|target| target.route == route)
    }

    /// The ids drawn through `route`, with their targets.
    pub fn ids_drawn_from(&self, route: LandTileRoute) -> impl Iterator<Item = (u32, LandTileTarget)> + '_ {
        self.targets.iter().enumerate().filter(move |(_, target)| target.route == route)
            .map(|(id, &target)| (id as u32, target))
    }

    /// The route an id takes from its own selected source.
    fn own_route(&self, route: LandTileRoute) -> bool {
        match route {
            LandTileRoute::Enhanced(source) => source == self.selected,
            // Classic draws land without a texmap from its art, as the Classic client does.
            LandTileRoute::ClassicTexmap | LandTileRoute::ClassicArt => self.selected == LandTextureSource::Cc,
            LandTileRoute::Missing => true,
        }
    }

    /// The one warning a load or source switch logs: how many of the ids the maps use (`used`) the selected source
    /// lacks and where each goes, with a sample of ids. `None` when every such id the sources have is drawn from the
    /// selected one. Ids no source has are not counted here; [`MissingLandIds`] reports those as the map shows them.
    /// Nor are the ids a route draws: [`Self::route_report`] names the routes not drawn as written.
    pub fn summary(&self, used: impl Fn(u32) -> bool) -> Option<String> {
        // In chain order: the EC-layout sources, then Classic.
        const DESTINATIONS: [LandTileRoute; 4] = [
            LandTileRoute::Enhanced(LandTextureSource::Ec),
            LandTileRoute::Enhanced(LandTextureSource::Kr),
            LandTileRoute::ClassicTexmap,
            LandTileRoute::ClassicArt,
        ];
        let destinations = DESTINATIONS.iter()
            .map(|&route| (route, self.ids_drawn_from(route).map(|(id, _)| id)
                .filter(|&id| used(id) && !self.routed[id as usize]).collect::<Vec<_>>()))
            .filter(|(route, ids)| !ids.is_empty() && !self.own_route(*route))
            .collect::<Vec<_>>();
        if destinations.is_empty() {
            return None;
        }
        let total: usize = destinations.iter().map(|(_, ids)| ids.len()).sum();
        let parts = destinations.iter().map(|(route, ids)| {
            format!("{} from {} (e.g. {})", ids.len(), route.label(), sample_ids(ids.iter().copied(), ids.len()))
        }).collect::<Vec<_>>();
        Some(format!("{} lacks {total} land id(s): drawing {}.", self.selected.land_label(), parts.join("; ")))
    }

    /// The warning about the routes not drawn as written: how many, and a sample with their line and what is drawn
    /// instead. `None` when every route is drawn as written.
    pub fn route_report(&self) -> Option<String> {
        if self.notes.is_empty() {
            return None;
        }
        let mut parts = self.notes.iter().take(LOG_SAMPLE_IDS).map(|note| {
            let source = note.request.source.land_label();
            let drawn = self.target(note.id);
            let why = match note.miss {
                LandRouteMiss::EnhancedUnderClassic => format!("Classic selected never draws {source}"),
                LandRouteMiss::NoTerrain(terrain) => format!("no land id of {source} is drawn with terrain {terrain}"),
                LandRouteMiss::TargetMissing(target) => format!("no source of the {source} chain has {target:#06X}"),
                LandRouteMiss::Fallback => format!("{source} lacks {:#06X}", drawn.id),
            };
            format!("{:#06X} (line {}): {why}, drawn as {:#06X} from {}", note.id, note.request.line, drawn.id,
                drawn.route.label())
        }).collect::<Vec<_>>();
        if self.notes.len() > LOG_SAMPLE_IDS {
            parts.push("...".to_string());
        }
        Some(format!("{} land route(s) are not drawn as written: {}.", self.notes.len(), parts.join("; ")))
    }

    /// The in-window notice for a selected package that is not loaded: what is shown instead, and where the package
    /// was looked for. `None` when the selected package is loaded.
    pub fn missing_package_notice(&self, udd_folder: &std::path::Path) -> Option<String> {
        (self.atlas != self.selected).then(|| {
            format!("{} not found in {}: showing {} land.", self.selected.land_label(), udd_folder.display(),
                self.atlas.land_label())
        })
    }
}

/// The Preferences entry of a land source: its label, marked when its package is not loaded, with a hover text naming
/// the file, where it was looked for and what is drawn instead. The entry stays selectable.
pub fn land_source_entry(source: LandTextureSource, loaded: bool, udd_folder: &std::path::Path)
    -> (String, Option<String>) {
    if loaded {
        return (source.land_label().to_string(), None);
    }
    let next = land_source_chain(source).iter().skip(1).map(|source| source.land_label()).collect::<Vec<_>>();
    let instead = if next.is_empty() { String::new() } else {
        format!(" It can still be selected: land is then drawn from {}.", next.join(", then "))
    };
    (format!("{} (missing)", source.land_label()),
        Some(format!("{} was not found in {}.{instead}", source.land_label(), udd_folder.display())))
}

/// Land ids drawn with the missing-texture pattern that the log has named since the last load or source switch, so
/// each is reported once, when the map first shows it.
#[derive(Clone, Debug)]
pub struct MissingLandIds {
    reported: Vec<bool>,
}

impl Default for MissingLandIds {
    fn default() -> Self {
        Self { reported: vec![false; LAND_ROUTE_TILE_COUNT] }
    }
}

impl MissingLandIds {
    pub fn clear(&mut self) {
        self.reported.fill(false);
    }

    /// The warning for those of `ids` not reported yet, and marks them reported; `None` when all were.
    pub fn report(&mut self, ids: impl IntoIterator<Item = u32>) -> Option<String> {
        let mut new = Vec::new();
        for id in ids {
            if let Some(reported) = self.reported.get_mut(id as usize) {
                if !*reported {
                    *reported = true;
                    new.push(id);
                }
            } else {
                new.push(id);
            }
        }
        (!new.is_empty()).then(|| format!(
            "{} land id(s) on the map have no texture in any land source and draw the missing-texture pattern: {}.",
            new.len(), sample_ids(new.iter().copied(), new.len())))
    }
}

fn sample_ids(ids: impl Iterator<Item = u32>, count: usize) -> String {
    let mut text = ids.take(LOG_SAMPLE_IDS).map(|id| format!("{id:#06X}")).collect::<Vec<_>>().join(", ");
    if count > LOG_SAMPLE_IDS {
        text.push_str(", ...");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use LandTextureSource::{Cc, Ec, Kr};

    const ALL: LandTileAvailability = LandTileAvailability { ec: true, kr: true, classic_texmap: true, classic_art: true };
    const NONE: LandTileAvailability = LandTileAvailability { ec: false, kr: false, classic_texmap: false, classic_art: false };

    #[test]
    fn each_source_draws_what_it_has() {
        assert_eq!(route_land_tile(Cc, ALL), LandTileRoute::ClassicTexmap);
        assert_eq!(route_land_tile(Ec, ALL), LandTileRoute::Enhanced(Ec));
        assert_eq!(route_land_tile(Kr, ALL), LandTileRoute::Enhanced(Kr));
    }

    #[test]
    fn a_missing_id_comes_from_the_other_enhanced_source_then_classic() {
        assert_eq!(route_land_tile(Kr, LandTileAvailability { kr: false, ..ALL }), LandTileRoute::Enhanced(Ec));
        assert_eq!(route_land_tile(Ec, LandTileAvailability { ec: false, ..ALL }), LandTileRoute::Enhanced(Kr));
        let classic_only = LandTileAvailability { classic_texmap: true, ..NONE };
        assert_eq!(route_land_tile(Ec, classic_only), LandTileRoute::ClassicTexmap);
        assert_eq!(route_land_tile(Kr, classic_only), LandTileRoute::ClassicTexmap);
    }

    #[test]
    fn classic_land_without_a_texmap_draws_its_art() {
        let art_only = LandTileAvailability { classic_art: true, ..NONE };
        for selected in LandTextureSource::ALL {
            assert_eq!(route_land_tile(selected, art_only), LandTileRoute::ClassicArt);
        }
    }

    #[test]
    fn classic_never_borrows_enhanced_terrain() {
        assert_eq!(route_land_tile(Cc, LandTileAvailability { ec: true, kr: true, ..NONE }), LandTileRoute::Missing);
    }

    #[test]
    fn nothing_has_it_draws_the_missing_pattern() {
        for selected in LandTextureSource::ALL {
            assert_eq!(route_land_tile(selected, NONE), LandTileRoute::Missing);
        }
    }

    #[test]
    fn the_atlas_takes_the_first_loaded_source_of_the_chain() {
        let routes = |selected, loaded: &'static [LandTextureSource]| {
            LandRoutes::resolve(selected, |source| loaded.contains(&source), |_| NONE, |_| None, |_, _| None).atlas()
        };
        assert_eq!(routes(Kr, &[Cc, Ec, Kr]), Kr);
        assert_eq!(routes(Kr, &[Cc, Ec]), Ec);
        assert_eq!(routes(Ec, &[Cc, Kr]), Kr);
        assert_eq!(routes(Ec, &[Cc]), Cc);
        assert_eq!(routes(Cc, &[Cc, Ec, Kr]), Cc);
    }

    #[test]
    fn the_summary_counts_rerouted_ids_by_destination() {
        let routes = LandRoutes::resolve(Kr, |_| true, |id| match id {
            1 | 2 => LandTileAvailability { kr: false, ..ALL },
            3 => LandTileAvailability { classic_texmap: true, ..NONE },
            4 => NONE,
            _ => ALL,
        }, |_| None, |_, _| None);
        assert_eq!(routes.route(1), LandTileRoute::Enhanced(Ec));
        assert_eq!(routes.route(3), LandTileRoute::ClassicTexmap);
        assert_eq!(routes.route(4), LandTileRoute::Missing);
        assert_eq!(routes.route(LAND_ROUTE_TILE_COUNT as u32), LandTileRoute::Missing);
        assert_eq!(routes.summary(|_| true).unwrap(), "Kingdom Reborn (tex_land_kr.uddp) lacks 3 land id(s): drawing 2 \
            from Enhanced (tex_land_ec.uddp) (e.g. 0x0001, 0x0002); 1 from Classic (tex_land_cc.uddp) (e.g. 0x0003).");
        // Only the ids the maps use count.
        assert_eq!(routes.summary(|id| id == 3).unwrap(), "Kingdom Reborn (tex_land_kr.uddp) lacks 1 land id(s): \
            drawing 1 from Classic (tex_land_cc.uddp) (e.g. 0x0003).");
        assert_eq!(routes.summary(|id| id > 4), None);
    }

    fn request(source: LandTextureSource, target: LandRouteTarget, line: usize) -> Option<LandRouteRequest> {
        Some(LandRouteRequest { source, target, line })
    }

    #[test]
    fn a_route_draws_the_id_it_names_from_its_source() {
        let routes = LandRoutes::resolve(Ec, |_| true, |_| ALL, |id| match id {
            1 => request(Kr, LandRouteTarget::Id(7), 3),
            2 => request(Cc, LandRouteTarget::SameId, 4),
            3 => request(Ec, LandRouteTarget::Terrain(40), 5),
            _ => None,
        }, |source, terrain| (source == Ec && terrain == 40).then_some(9));
        assert_eq!(routes.target(1), LandTileTarget { route: LandTileRoute::Enhanced(Kr), id: 7 });
        assert_eq!(routes.target(2), LandTileTarget { route: LandTileRoute::ClassicTexmap, id: 2 });
        assert_eq!(routes.target(3), LandTileTarget { route: LandTileRoute::Enhanced(Ec), id: 9 });
        assert_eq!(routes.target(4), LandTileTarget { route: LandTileRoute::Enhanced(Ec), id: 4 });
        assert_eq!(routes.route_report(), None);
        // Classic routes to Classic art when the target has no texmap, as the chain does.
        let art_only = LandRoutes::resolve(Ec, |_| true, |id| match id {
            8 => LandTileAvailability { classic_art: true, ..NONE },
            _ => ALL,
        }, |id| (id == 1).then_some(LandRouteRequest { source: Cc, target: LandRouteTarget::Id(8), line: 1 }),
            |_, _| None);
        assert_eq!(art_only.target(1), LandTileTarget { route: LandTileRoute::ClassicArt, id: 8 });
        assert_eq!(art_only.route_report(), None);
    }

    #[test]
    fn a_route_its_source_cannot_draw_falls_back_with_a_warning() {
        let routes = LandRoutes::resolve(Ec, |_| true, |id| match id {
            7 => LandTileAvailability { kr: false, ..ALL },
            8 => NONE,
            _ => ALL,
        }, |id| match id {
            1 => request(Kr, LandRouteTarget::Id(7), 3),
            2 => request(Kr, LandRouteTarget::Id(8), 4),
            3 => request(Kr, LandRouteTarget::Terrain(40), 5),
            _ => None,
        }, |_, _| None);
        // The route's chain on the id it names.
        assert_eq!(routes.target(1), LandTileTarget { route: LandTileRoute::Enhanced(Ec), id: 7 });
        // No source has the id it names, nor a land id the terrain: the selected source's chain on the routed id.
        assert_eq!(routes.target(2), LandTileTarget { route: LandTileRoute::Enhanced(Ec), id: 2 });
        assert_eq!(routes.target(3), LandTileTarget { route: LandTileRoute::Enhanced(Ec), id: 3 });
        assert_eq!(routes.route_report().unwrap(), "3 land route(s) are not drawn as written: \
            0x0001 (line 3): Kingdom Reborn (tex_land_kr.uddp) lacks 0x0007, drawn as 0x0007 from Enhanced (tex_land_ec.uddp); \
            0x0002 (line 4): no source of the Kingdom Reborn (tex_land_kr.uddp) chain has 0x0008, drawn as 0x0002 from Enhanced (tex_land_ec.uddp); \
            0x0003 (line 5): no land id of Kingdom Reborn (tex_land_kr.uddp) is drawn with terrain 40, drawn as 0x0003 from Enhanced (tex_land_ec.uddp).");
        // A route counts in the report, not in the summary.
        let lacking = LandRoutes::resolve(Kr, |_| true, |_| LandTileAvailability { kr: false, ..ALL },
            |id| (id == 1).then_some(LandRouteRequest { source: Ec, target: LandRouteTarget::SameId, line: 1 }),
            |_, _| None);
        assert!(lacking.summary(|id| id <= 1).unwrap().contains("lacks 1 land id(s)"));
    }

    #[test]
    fn classic_selected_skips_enhanced_routes() {
        let routes = LandRoutes::resolve(Cc, |_| true, |_| ALL,
            |id| (id == 1).then_some(LandRouteRequest { source: Kr, target: LandRouteTarget::Id(7), line: 2 }),
            |_, _| None);
        assert_eq!(routes.target(1), LandTileTarget { route: LandTileRoute::ClassicTexmap, id: 1 });
        assert!(routes.route_report().unwrap().contains("0x0001 (line 2): Classic selected never draws Kingdom Reborn"));
    }

    #[test]
    fn a_complete_source_and_classic_art_log_no_summary() {
        assert_eq!(LandRoutes::resolve(Ec, |_| true, |_| ALL, |_| None, |_, _| None).summary(|_| true), None);
        let art_only = LandTileAvailability { classic_art: true, ..NONE };
        assert_eq!(LandRoutes::resolve(Cc, |_| true, |_| art_only, |_| None, |_, _| None).summary(|_| true), None);
    }

    #[test]
    fn a_missing_selected_package_names_what_is_shown_instead() {
        let folder = std::path::Path::new("/uddp");
        let without_kr = LandRoutes::resolve(Kr, |source| source != Kr, |_| NONE, |_| None, |_, _| None);
        assert_eq!(without_kr.missing_package_notice(folder).unwrap(),
            "Kingdom Reborn (tex_land_kr.uddp) not found in /uddp: showing Enhanced (tex_land_ec.uddp) land.");
        let classic_only = LandRoutes::resolve(Kr, |source| source == Cc, |_| NONE, |_| None, |_, _| None);
        assert!(classic_only.missing_package_notice(folder).unwrap().ends_with("showing Classic (tex_land_cc.uddp) land."));
        assert_eq!(LandRoutes::resolve(Kr, |_| true, |_| NONE, |_| None, |_, _| None).missing_package_notice(folder), None);
    }

    #[test]
    fn preferences_mark_a_missing_package_and_keep_it_selectable() {
        let folder = std::path::Path::new("/uddp");
        assert_eq!(land_source_entry(Kr, true, folder), (Kr.land_label().to_string(), None));
        let (label, hover) = land_source_entry(Kr, false, folder);
        assert_eq!(label, "Kingdom Reborn (tex_land_kr.uddp) (missing)");
        assert_eq!(hover.unwrap(), "Kingdom Reborn (tex_land_kr.uddp) was not found in /uddp. It can still be \
            selected: land is then drawn from Enhanced (tex_land_ec.uddp), then Classic (tex_land_cc.uddp).");
        assert_eq!(land_source_entry(Cc, false, folder).1.unwrap(), "Classic (tex_land_cc.uddp) was not found in /uddp.");
    }

    #[test]
    fn missing_ids_are_reported_once_until_cleared() {
        let mut missing = MissingLandIds::default();
        assert!(missing.report([5, 6]).unwrap().starts_with("2 land id(s)"));
        assert_eq!(missing.report([5, 6]), None);
        assert!(missing.report([6, 7]).unwrap().contains("0x0007"));
        missing.clear();
        assert!(missing.report([5]).is_some());
    }
}
