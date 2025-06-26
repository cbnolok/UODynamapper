//! Boots Bevy, installs cache and chunk systems, sets light direction.
/*
#![allow(unused_parens)]
*/
pub mod texture_cache;
pub mod terrain_chunk_mesh;
pub mod camera;
pub mod constants;
pub mod player;
pub mod util_lib;
pub mod worldmap_base_mesh;

use bevy::prelude::*;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn run_bevy_app() -> AppExit {
    // Install the custom own log subscriber (must come BEFORE Bevy app launch!)
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_ansi(true) // colored output like Bevy default
                .with_level(true)
                .with_target(true)
                // Use chrono for timestamp, format with NO milliseconds
                .with_timer(fmt::time::ChronoLocal::new("%Y-%m-%d %H:%M:%s".into()))
                .compact() // Looks a lot like Bevy default (use .pretty() for multiline pretty logs)
        )
        .with(EnvFilter::from_default_env())
        .init();

    App::new()
        .add_plugins(DefaultPlugins.build()
            .disable::<bevy::log::LogPlugin>()
            .set(ImagePlugin::default_nearest()))
        .add_plugins(MaterialPlugin::<terrain_chunk_mesh::TerrainMaterial>::default())   // ← registers Assets<TerrainMaterial>, TODO: do this in a separate startup system
        .add_plugins(texture_cache::TextureCachePlugin)
        .add_systems(Startup, (camera::setup_cam, spawn_worldmap_chunks, player::spawn_player_entity))
        .add_systems(Update, (terrain_chunk_mesh::build_visible_terrain_chunks,))
        .run()
}

// Spawn a 3×3 grid of placeholder chunks.
fn spawn_worldmap_chunks(mut commands: Commands) {
    // TODO: pass player position.
    for gx in 0..=2 {
        for gy in 0..=2 {
            commands.spawn((
                terrain_chunk_mesh::TCMesh { gx, gy },
                Transform::default(),
                GlobalTransform::default(),
            ));
        }
    }
}
