use bevy::ecs::schedule::SystemSet;


#[derive(SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub enum StartupSysSet {
    LoadStartupFiles,
    SetupScene,
}

#[derive(SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub enum SceneRenderSysSet {
    SyncLandChunks,
    RenderLandChunks,
}

