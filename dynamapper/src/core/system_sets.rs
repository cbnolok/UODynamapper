use bevy::ecs::schedule::SystemSet;

#[derive(SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub enum StartupSysSet {
    LoadStartupFiles,
    SetupScene,
}
