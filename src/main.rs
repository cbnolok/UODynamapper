//! Boots Bevy, installs cache and chunk systems, sets light direction.
#![allow(unused_parens)]

mod tile_cache;
mod chunk_mesh;
mod util;
mod worldmap_base_mesh;

use bevy::{
    color, pbr::ExtendedMaterial, prelude::*
};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use tile_cache::*;
use chunk_mesh::*;
use worldmap_base_mesh::*;

fn main() {
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
        .insert_resource(LightDir(Vec3::new(-0.4, 1.0, 0.3).normalize()))
        // embed the WGSL in the asset system:
        //.add_systems(Startup, |mut app: ResMut<App>| {
        //    load_internal_asset!(app, TERRAIN_SHADER_HANDLE, "terrain.wgsl", TERRAIN_SHADER);
        //})
        .add_plugins(MaterialPlugin::<ExtendedMaterial<StandardMaterial, TerrainMaterial>>::default())   // ← registers Assets<TerrainMaterial>
        .add_systems(Startup, (setup_cam, setup_cache, spawn_chunks, build_debug_obj))
        .add_systems(Update, (chunk_mesh::build_visible_chunks,))
        .run();
}

// Simple directional light direction (uniform) – stored as resource.
#[derive(Resource, Deref)]
pub struct LightDir(pub Vec3);

pub fn setup_cam(mut commands: Commands) {
    // Set up a directional light (sun)
    commands.spawn((
        DirectionalLight {
            shadows_enabled: false, // Disable shadows if not needed
            ..default()
        },
        Transform::from_xyz(8.0, 50.0, 8.0).looking_at(Vec3::new(8.0, 0.0, 8.0), Vec3::Y),
        GlobalTransform::default(), // Needed for transforming the light in world space
    ));

    // Center of your chunk/grid
    let center = Vec3::new(8.0, 0.0, 8.0);

    // Camera position: 30 units above & 30 units back
    let cam_pos = Vec3::new(center.x, 30.0, center.z + 30.0);

    commands.spawn((
        Camera3d::default(), // Marker component for 3D cameras
        Transform::from_xyz(cam_pos.x, cam_pos.y, cam_pos.z).looking_at(center, Vec3::Y),
        GlobalTransform::default(), // Needed for transforming the camera in world space
    ));

    // Additional camera example (uncomment to use!)
    /*
    commands.spawn((
        Camera3d,
        Projection::Perspective(PerspectiveProjection::default()),
        Transform::from_xyz(-20.0, 40.0, 60.0).looking_at(Vec3::new(8.0, 0.0, 8.0), Vec3::Y),
        GlobalTransform::default(),
    ));
    */
}

// Allocate GPU texture array and TileCache.
fn setup_cache(
    mut cmd: Commands,
    mut images: ResMut<Assets<Image>>,
    //rd: Res<RenderDevice>,
) {
    let handle = tile_cache::create_gpu_array(&mut images); //, &rd);
    cmd.insert_resource(TileCache::new(handle));
}

// Spawn a 3×3 grid of placeholder chunks.
fn spawn_chunks(mut commands: Commands) {
    for gx in -1..=1 {
        for gy in -1..=1 {
            commands.spawn((
                MapMeshChunk { gx, gy },
                Transform::default(),
                GlobalTransform::default(),
            ));
        }
    }
}

fn build_debug_obj(
    mut commands:   Commands,
    mut meshes:     ResMut<Assets<Mesh>>,
    mut materials:  ResMut<Assets<StandardMaterial>>,
) {
    let mesh_handle = meshes.add(Mesh::from(Cuboid { half_size: Vec3::splat(0.5) }));
    let material_handle = materials.add(StandardMaterial {
        base_color: Color::Srgba(color::palettes::basic::GREEN),
        ..default()
    });

    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material_handle),
        Transform::from_xyz(8.0, 0.5, 8.0),
        GlobalTransform::default(),
    ));
}