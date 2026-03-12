use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::{color, prelude::*};
use crate::external_data::settings::Settings;

#[derive(Component)]
pub struct Player {
    pub current_pos: Option<UOVec4>,
    pub prev_rendered_pos: Option<UOVec4>,
}

pub struct PlayerPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(PlayerPlugin);
impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(
            Startup,
            sys_spawn_player_entity.in_set(StartupSysSet::SetupSceneStage1),
        );
    }
}

pub fn sys_spawn_player_entity(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    settings: Res<Settings>,
) {
    log_system_add_startup::<PlayerPlugin>(StartupSysSet::SetupSceneStage1, fname!());

    // A cube, to mimic the player position and to have another rendered object to have a visual comparison.
    let mesh_handle = meshes.add(Mesh::from(Cuboid {
        half_size: Vec3::splat(0.5),
    }));
    let material_handle = materials.add(StandardMaterial {
        base_color: Color::Srgba(color::palettes::basic::GREEN),
        ..default()
    });

    let start_p = settings.core.world.start_p;
    let player_start_pos: Vec3 = start_p.to_bevy_vec3_ignore_map();

    let mut player_entity = commands.spawn((
        Transform::from_xyz(player_start_pos.x, player_start_pos.y, player_start_pos.z),
        GlobalTransform::default(),
        Player {
            current_pos: Some(start_p),
            prev_rendered_pos: None,
        },
    ));

    if settings.core.world.hide_player {
        player_entity.insert((
            Mesh3d(mesh_handle),
            MeshMaterial3d(material_handle),
        ));
    }

    logger::one(
        None,
        LogSev::Debug,
        LogAbout::Player,
        format!("Spawned player at pos {player_start_pos}.").as_str(),
    );
}
