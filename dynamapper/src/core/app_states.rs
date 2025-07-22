
use bevy::state::state::States;
use crate::logger;

#[derive(strum_macros::AsRefStr, States, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    LoadStartupFiles,
    /// Setup player, allocate gpu textures (land).
    SetupSceneStage1,
    /// Setup land.
    SetupSceneStage2,
    InGame,
    Stop,
}

#[track_caller]
pub fn log_appstate_change(appstate_name: &'static str) {
    logger::one(
        None,
        logger::LogSev::Debug,
        logger::LogAbout::AppState,
        format!("Changing AppState to: {appstate_name}.").as_str(),
    );
    use std::io::Write; // for flush().
    let _ = std::io::stdout().flush();
}
