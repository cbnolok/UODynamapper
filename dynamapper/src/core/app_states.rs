
use bevy::state::state::States;
use crate::logger;

// OnEnter systems only run for one frame
#[derive(strum_macros::AsRefStr, States, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    StartupSetup,
    InGame,
    Stop,
}

#[track_caller]
pub fn log_appstate_change(new_appstate_name: &'static str) {
    logger::one(
        None,
        logger::LogSev::Debug,
        logger::LogAbout::AppState,
        format!("Changing AppState to: {new_appstate_name}.").as_str(),
    );
    use std::io::Write; // for flush().
    let _ = std::io::stdout().flush();
}
