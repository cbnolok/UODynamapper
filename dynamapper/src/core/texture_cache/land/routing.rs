//! Which land source draws each land id (the user's design, 2026-11-12).
//!
//! The Land Texture Source preference picks a source; a land id whose definition or texture that source lacks is drawn
//! from the next source of its chain instead: the selected source, then the other EC-layout source (Kingdom Reborn and
//! Enhanced stand in for each other), then Classic. Classic draws the id's texmap or, without one, its land art as
//! the Classic client does (`LandView.Draw` falls back to the art when the texmap is missing). An id no source has
//! draws the missing-texture pattern. Classic selected stays Classic: it never borrows EC-layout terrain.
//!
//! The chain and the summary text are independent of rendering; the land cache feeds them what each package holds.

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
/// The fallback chain keeps the land id; an explicit override can name another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandTileTarget {
    pub route: LandTileRoute,
    pub id: u32,
}

impl LandTileTarget {
    /// Whether the target's source holds its id, by what `available` says each source holds for an id.
    fn is_held(self, available: &impl Fn(u32) -> LandTileAvailability) -> bool {
        let held = available(self.id);
        match self.route {
            LandTileRoute::Enhanced(LandTextureSource::Ec) => held.ec,
            LandTileRoute::Enhanced(LandTextureSource::Kr) => held.kr,
            LandTileRoute::Enhanced(LandTextureSource::Cc) => false,
            LandTileRoute::ClassicTexmap => held.classic_texmap,
            LandTileRoute::ClassicArt => held.classic_art,
            LandTileRoute::Missing => true,
        }
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
}

impl LandRoutes {
    /// Routes every id under `selected`; `loaded` tells which packages are loaded, `available` what each holds.
    /// `overrides` is an explicit route table in front of the chain: an id's override applies when its source holds
    /// the target id, and the chain decides otherwise.
    pub fn resolve(
        selected: LandTextureSource,
        loaded: impl Fn(LandTextureSource) -> bool,
        available: impl Fn(u32) -> LandTileAvailability,
        overrides: impl Fn(u32) -> Option<LandTileTarget>,
    ) -> Self {
        let atlas = land_source_chain(selected).iter().copied().find(|&source| loaded(source))
            .unwrap_or(LandTextureSource::Cc);
        let targets = (0..LAND_ROUTE_TILE_COUNT as u32).map(|id| {
            overrides(id).filter(|target| target.is_held(&available))
                .unwrap_or(LandTileTarget { route: route_land_tile(selected, available(id)), id })
        }).collect();
        Self { selected, atlas, targets }
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

    /// The one warning a load or source switch logs: how many ids the selected source lacks and where each goes, with
    /// a sample of ids. `None` when every id the sources have is drawn from the selected one. Ids no source has are
    /// not counted here (most ids are unused); [`MissingLandIds`] reports those as the map shows them.
    pub fn summary(&self) -> Option<String> {
        // In chain order: the EC-layout sources, then Classic.
        const DESTINATIONS: [LandTileRoute; 4] = [
            LandTileRoute::Enhanced(LandTextureSource::Ec),
            LandTileRoute::Enhanced(LandTextureSource::Kr),
            LandTileRoute::ClassicTexmap,
            LandTileRoute::ClassicArt,
        ];
        let destinations = DESTINATIONS.iter()
            .map(|&route| (route, self.ids_drawn_from(route).map(|(id, _)| id).collect::<Vec<_>>()))
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
            LandRoutes::resolve(selected, |source| loaded.contains(&source), |_| NONE, |_| None).atlas()
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
        }, |_| None);
        assert_eq!(routes.route(1), LandTileRoute::Enhanced(Ec));
        assert_eq!(routes.route(3), LandTileRoute::ClassicTexmap);
        assert_eq!(routes.route(4), LandTileRoute::Missing);
        assert_eq!(routes.route(LAND_ROUTE_TILE_COUNT as u32), LandTileRoute::Missing);
        assert_eq!(routes.summary().unwrap(), "Kingdom Reborn (tex_land_kr.uddp) lacks 3 land id(s): drawing 2 from \
            Enhanced (tex_land_ec.uddp) (e.g. 0x0001, 0x0002); 1 from Classic (tex_land_cc.uddp) (e.g. 0x0003).");
    }

    #[test]
    fn an_override_in_front_of_the_chain_applies_when_its_source_holds_the_target() {
        let to_kr_7 = LandTileTarget { route: LandTileRoute::Enhanced(Kr), id: 7 };
        let overrides = |id| match id {
            1 => Some(to_kr_7),
            2 => Some(LandTileTarget { route: LandTileRoute::ClassicTexmap, id: 8 }),
            _ => None,
        };
        let routes = LandRoutes::resolve(Ec, |_| true, |id| match id {
            8 => LandTileAvailability { classic_texmap: false, ..ALL },
            _ => ALL,
        }, overrides);
        assert_eq!(routes.target(1), to_kr_7);
        // Its target lacks the texmap: the chain decides.
        assert_eq!(routes.target(2), LandTileTarget { route: LandTileRoute::Enhanced(Ec), id: 2 });
        assert_eq!(routes.target(3), LandTileTarget { route: LandTileRoute::Enhanced(Ec), id: 3 });
    }

    #[test]
    fn a_complete_source_and_classic_art_log_no_summary() {
        assert_eq!(LandRoutes::resolve(Ec, |_| true, |_| ALL, |_| None).summary(), None);
        let art_only = LandTileAvailability { classic_art: true, ..NONE };
        assert_eq!(LandRoutes::resolve(Cc, |_| true, |_| art_only, |_| None).summary(), None);
    }

    #[test]
    fn a_missing_selected_package_names_what_is_shown_instead() {
        let folder = std::path::Path::new("/uddp");
        let without_kr = LandRoutes::resolve(Kr, |source| source != Kr, |_| NONE, |_| None);
        assert_eq!(without_kr.missing_package_notice(folder).unwrap(),
            "Kingdom Reborn (tex_land_kr.uddp) not found in /uddp: showing Enhanced (tex_land_ec.uddp) land.");
        let classic_only = LandRoutes::resolve(Kr, |source| source == Cc, |_| NONE, |_| None);
        assert!(classic_only.missing_package_notice(folder).unwrap().ends_with("showing Classic (tex_land_cc.uddp) land."));
        assert_eq!(LandRoutes::resolve(Kr, |_| true, |_| NONE, |_| None).missing_package_notice(folder), None);
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
