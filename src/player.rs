use bevy::{color, prelude::*};

#[derive(Component)]
pub struct Player;

// also spawn light entity!

pub fn spawn_player_entity(
    mut commands:   Commands,
    mut meshes:     ResMut<Assets<Mesh>>,
    mut materials:  ResMut<Assets<StandardMaterial>>,
) {
    // A cube, to mimic the player position and to have another rendered object to have a visual comparison.
    let mesh_handle = meshes.add(Mesh::from(Cuboid { half_size: Vec3::splat(0.5) }));
    let material_handle = materials.add(StandardMaterial {
        base_color: Color::Srgba(color::palettes::basic::GREEN),
        ..default()
    });

    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material_handle),
        Transform::from_xyz(32.0, 1.0, 32.0),
        GlobalTransform::default(),
    ));
}