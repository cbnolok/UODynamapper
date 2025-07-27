use bevy::ecs::schedule::SystemSet;


#[derive(strum_macros::AsRefStr, SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub enum StartupSysSet {
    First,
    LoadStartupUOFiles,
    SetupSceneStage1,
    SetupSceneStage2,
    Done,
}

#[derive(strum_macros::AsRefStr, SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub enum SceneRenderSysSet {
    SyncLandChunks,
    RenderLandChunks,
}

