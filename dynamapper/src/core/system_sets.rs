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
pub enum SceneRenderLandSysSet {
    ListenSyncRequests,
    SyncLandChunks,
    RenderLandChunks,
}

#[derive(strum_macros::AsRefStr, SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub enum MovementSysSet {
    MovementActions,
    UpdateCamera,
}

