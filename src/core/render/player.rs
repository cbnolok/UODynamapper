use bevy::{color, prelude::*};
use crate::core::render::scene::SceneStartupData;

#[derive(Component)]
pub struct Player;

pub struct PlayerPlugin;
impl Plugin for PlayerPlugin
{
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, sys_spawn_player_entity);                                         
    }    
}

pub fn sys_spawn_player_entity(
    mut commands:   Commands,
    mut meshes:     ResMut<Assets<Mesh>>,
    mut materials:  ResMut<Assets<StandardMaterial>>,
    scene_startup_data_res: Option<Res<SceneStartupData>>,
) {
    // A cube, to mimic the player position and to have another rendered object to have a visual comparison.
    let mesh_handle = meshes.add(Mesh::from(Cuboid { half_size: Vec3::splat(0.5) }));
    let material_handle = materials.add(StandardMaterial {
        base_color: Color::Srgba(color::palettes::basic::GREEN),
        ..default()
    });
    let player_start_pos = scene_startup_data_res.unwrap().player_start_pos;

    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material_handle),
        Transform::from_xyz(player_start_pos.x, player_start_pos.y + 2.0, player_start_pos.z),
        GlobalTransform::default(),
    ));
}