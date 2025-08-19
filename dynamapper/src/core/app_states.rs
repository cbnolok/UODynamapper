use bevy::prelude::States;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, States)]
pub enum AppState {
    #[default]
    LoadStartupFiles,
    SetupRenderer,
    InGame,
    Stop,
}
