//! The route file at run time (`configs::land_routes`, LandRoutes.kdl): read at startup, read again when the user's
//! copy is saved or Preferences asks for it, with the profile Preferences > Graphics names in force. A profile with
//! areas applies to the land tiles and statics inside them (`RouteAreas`), the `routes` block alone elsewhere: the
//! routes are split by area once (`AreaRoutes`), and the player's position plays no part. Each change of the file in
//! force or of the profile in force counts a revision: the land routes are resolved again and the land atlas rebuilt
//! when they changed (`sys_apply_land_texture_source_changes`), and the statics stores read their blocks again when
//! the item routes changed (`sys_apply_item_routes`).
//!
//! A user copy that does not read leaves the bundled file in force, with the error in the log and in the window, until
//! it is fixed (`configs::runtime_specs`).

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bevy::prelude::*;

use crate::configs::land_routes::{
    AreaRoutes, LandRoute, LandRoutesFile, LAND_ROUTES_FILE_NAME, LAND_ROUTES_SPECS_FOLDER,
};
use crate::configs::runtime_specs::{LoadedSpec, SpecFiles};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::statics::StaticsStoreRes;

/// How often the user's copy is checked for a save.
const ROUTE_FILE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// What the log calls the file.
const ROUTE_FILE_WHAT: &str = "the land and item routes";

#[derive(Resource)]
pub struct RouteFileRes {
    files: SpecFiles,
    file: LandRoutesFile,
    /// The user copy's modification time when it was last read; `None` when there was none.
    user_modified: Option<SystemTime>,
    /// The profile in force: the one the settings name, when the file has it.
    profile: Option<String>,
    /// Counts the changes of the file in force and of the profile in force.
    revision: u64,
    /// Set by Preferences' "Reload" button: the file is read again on the next frame.
    pub reload_requested: bool,
    /// The settings' profile last warned about as missing from the file.
    warned_missing_profile: Option<String>,
    /// Errors of the user's copy read at startup, shown in the window once the world is on screen: messages pushed
    /// during loading expire before the first frame.
    pending_notices: Vec<String>,
}

/// The modification time of `path`; `None` when it does not exist.
fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}

/// Logs where the routes were read from and why the user's copy was not read; returns the in-window notice, if any.
fn report(loaded: &LoadedSpec<LandRoutesFile>) -> Option<String> {
    console_logger::one(LogSev::Info, LogAbout::Settings,
        &format!("Loaded {ROUTE_FILE_WHAT} from {}", loaded.path.display()));
    let error = loaded.user_error.as_ref()?;
    let message = format!("{error:#}: {} applies until it is fixed.", loaded.path.display());
    console_logger::one(LogSev::Warn, LogAbout::Settings, &message);
    Some(message)
}

impl RouteFileRes {
    /// Reads the route file at startup: the bundled file is required, a missing or invalid one stops the app.
    pub fn load() -> Self {
        let files = SpecFiles::of(LAND_ROUTES_SPECS_FOLDER, LAND_ROUTES_FILE_NAME);
        let user_modified = modified(&files.user);
        let loaded = files.load(LandRoutesFile::decode)
            .unwrap_or_else(|error| crate::configs::settings::exit_on_startup_config_error(&error));
        let pending_notices = report(&loaded).into_iter().collect();
        Self {
            files,
            file: loaded.value,
            user_modified,
            profile: None,
            revision: 1,
            reload_requested: false,
            warned_missing_profile: None,
            pending_notices,
        }
    }

    /// Reads the file again; counts a revision when what is in force changes.
    fn reload(&mut self) {
        self.user_modified = modified(&self.files.user);
        match self.files.load(LandRoutesFile::decode) {
            Ok(loaded) => {
                if let Some(notice) = report(&loaded) {
                    crate::ingame_sysmessage_logger::warning(notice);
                }
                if loaded.value != self.file {
                    self.file = loaded.value;
                    self.revision += 1;
                }
            }
            // The bundled file no longer reads: what is in force stays.
            Err(error) => {
                let message = format!("{error:#}: the routes in force stay");
                console_logger::one(LogSev::Warn, LogAbout::Settings, &message);
                crate::ingame_sysmessage_logger::warning(message);
            }
        }
    }

    /// Puts the profile `requested` (empty: none) in force when the file has it; a profile the file lacks applies no
    /// profile, with a warning. Counts a revision when the profile in force changes.
    fn select_profile(&mut self, requested: &str) {
        let selected = (!requested.is_empty()).then(|| self.file.profile(requested)).flatten();
        let profile = selected.map(|profile| profile.name.as_str());
        if !requested.is_empty() && selected.is_none() && self.warned_missing_profile.as_deref() != Some(requested) {
            self.warned_missing_profile = Some(requested.to_string());
            let message = format!("The route profile \"{requested}\" is not in {}: no profile applies.",
                LAND_ROUTES_FILE_NAME);
            console_logger::one(LogSev::Warn, LogAbout::Settings, &message);
            crate::ingame_sysmessage_logger::warning(message);
        }
        if profile != self.profile.as_deref() {
            self.profile = profile.map(str::to_string);
            self.revision += 1;
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn has_pending_notices(&self) -> bool {
        !self.pending_notices.is_empty()
    }

    /// Where the user's copy is read from.
    pub fn user_path(&self) -> &std::path::Path {
        &self.files.user
    }

    /// The names of the file's profiles, in file order.
    pub fn profile_names(&self) -> impl Iterator<Item = &str> {
        self.file.profiles.iter().map(|profile| profile.name.as_str())
    }

    /// The route of every land id under the profile in force, inside its areas and outside them.
    pub fn land_routes(&self) -> AreaRoutes<Vec<Option<LandRoute>>> {
        self.file.land_routes_by_area(self.profile.as_deref())
    }

    /// The item each static graphic draws under the profile in force, inside its areas and outside them; `None` when
    /// no item is routed.
    pub fn item_routes(&self) -> Option<Arc<AreaRoutes<Vec<u16>>>> {
        self.file.item_routes_by_area(self.profile.as_deref()).map(Arc::new)
    }
}

/// The label of "no profile" in Preferences.
pub const NO_ROUTE_PROFILE_LABEL: &str = "None";

/// The entries of Preferences' "Season / profile" choice: no profile, then the file's profiles, then `selected` when
/// the file lacks it, marked; each as its setting value and its label.
pub fn route_profile_entries<'a>(profiles: impl Iterator<Item = &'a str>, selected: &str) -> Vec<(String, String)> {
    let mut entries = vec![(String::new(), NO_ROUTE_PROFILE_LABEL.to_string())];
    entries.extend(profiles.map(|name| (name.to_string(), name.to_string())));
    if !entries.iter().any(|(value, _)| value == selected) {
        entries.push((selected.to_string(), format!("{selected} (missing)")));
    }
    entries
}

/// Reads the route file again when the user's copy is saved, created or deleted, or Preferences asks for it.
pub fn sys_watch_route_file(mut routes: ResMut<RouteFileRes>, time: Res<Time<Real>>, mut timer: Local<Option<Timer>>) {
    let timer = timer.get_or_insert_with(|| Timer::new(ROUTE_FILE_CHECK_INTERVAL, TimerMode::Repeating));
    let checked = timer.tick(time.delta()).just_finished();
    let saved = checked && modified(&routes.files.user) != routes.user_modified;
    if routes.reload_requested || saved {
        routes.reload_requested = false;
        routes.reload();
    }
}

/// Puts the profile the settings name in force (Preferences > Graphics > "Season / profile"), also after the file is
/// read again: a profile may appear in it or leave it.
pub fn sys_select_route_profile(settings: Res<crate::configs::settings::Settings>, mut routes: ResMut<RouteFileRes>) {
    if settings.is_changed() || routes.is_changed() {
        // The revision tells the systems that apply the routes what changed; the change flag stays the file's.
        routes.bypass_change_detection().select_profile(&settings.graphics.route_profile);
    }
}

/// Routes the static graphics of every statics store through the item routes in force, whenever they change.
pub fn sys_apply_item_routes(
    routes: Res<RouteFileRes>,
    statics: Option<Res<StaticsStoreRes>>,
    mut applied: Local<u64>,
) {
    let Some(statics) = statics else { return };
    if *applied == routes.revision {
        return;
    }
    *applied = routes.revision;
    let item_routes = routes.item_routes();
    for store in statics.0.iter().flatten() {
        store.lock().set_item_routes(item_routes.clone());
    }
}

/// Shows the startup errors of the user's copy in the window once the world is on screen.
pub fn sys_show_route_file_notices(mut routes: ResMut<RouteFileRes>) {
    for notice in std::mem::take(&mut routes.pending_notices) {
        crate::ingame_sysmessage_logger::warning(notice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_choice_lists_none_the_file_and_a_missing_selection() {
        let profiles = ["spring", "winter"];
        let labels = |selected| route_profile_entries(profiles.into_iter(), selected).into_iter()
            .map(|(_, label)| label).collect::<Vec<_>>();
        assert_eq!(labels(""), ["None", "spring", "winter"]);
        assert_eq!(labels("winter"), ["None", "spring", "winter"]);
        assert_eq!(labels("fall"), ["None", "spring", "winter", "fall (missing)"]);
        assert_eq!(route_profile_entries(profiles.into_iter(), "")[0].0, "");
    }

    fn routes_of(text: &str) -> RouteFileRes {
        let file = LandRoutesFile::decode(LAND_ROUTES_FILE_NAME, text).unwrap_or_else(|error| panic!("{error:#}"));
        RouteFileRes {
            files: SpecFiles::under(std::path::Path::new("assets"), std::path::Path::new("cache"),
                LAND_ROUTES_SPECS_FOLDER, LAND_ROUTES_FILE_NAME),
            file,
            user_modified: None,
            profile: None,
            revision: 1,
            reload_requested: false,
            warned_missing_profile: None,
            pending_notices: Vec::new(),
        }
    }

    /// The profile in force follows the settings alone, a switch counting one revision; a profile with areas splits
    /// its routes by them, and the land routes of a profile without land stay those of the `routes` block.
    #[test]
    fn the_profile_in_force_switches_with_the_settings_and_routes_by_area() {
        let mut routes = routes_of(
            "profile \"desolation\" {\n    apply-in x1=0 y1=0 x2=0x17FF y2=0xFFFF\n    \
             item 0x0C37 id=0x1BAE\n}\nprofile \"fall\" {\n    item 0x0CD1 id=0x0CD2\n}\n");
        routes.select_profile("desolation");
        let selected = routes.revision();
        let desolation = routes.item_routes().unwrap();
        assert_eq!((desolation.outside[0x0C37], desolation.inside.as_ref().unwrap()[0x0C37]), (0x0C37, 0x1BAE));
        assert!(desolation.areas.contains(0, 0x17FF, 0) && !desolation.areas.contains(0, 0x1800, 0));
        assert_eq!(routes.land_routes().inside, None);
        routes.select_profile("desolation");
        assert_eq!(routes.revision(), selected);

        // A profile without areas applies everywhere.
        routes.select_profile("fall");
        assert_eq!(routes.revision(), selected + 1);
        let fall = routes.item_routes().unwrap();
        assert_eq!((fall.outside[0x0CD1], fall.inside.is_none()), (0x0CD2, true));
        routes.select_profile("");
        assert_eq!((routes.revision(), routes.item_routes()), (selected + 2, None));
    }
}
