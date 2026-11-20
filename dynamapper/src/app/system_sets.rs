use bevy::ecs::schedule::SystemSet;


#[derive(strum_macros::AsRefStr, SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub enum StartupSystems {
    First,
    LoadStartupUOFiles,
    SetupSceneStage1,
    SetupSceneStage2,
    Done,
}

#[derive(strum_macros::AsRefStr, SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub enum SceneRenderLandSystems {
    ListenSyncRequests,
    SyncLandChunks,
    RenderLandChunks,
}
#[derive(strum_macros::AsRefStr, SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub enum SceneRenderArtSystems {
    CollectVisibleStatics,
    RenderStaticSprites,
}

#[derive(strum_macros::AsRefStr, SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub enum MovementSystems {
    MovementActions,
    UpdateCamera,
}

