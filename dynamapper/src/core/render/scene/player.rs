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
        )
        .add_systems(Update, sys_update_player_visibility.run_if(in_state(AppState::InGame)));
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
    
    // Store handles in a resource for dynamic toggling or just let the system handle it

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

    if !settings.core.world.hide_player {
        player_entity.insert((
            Mesh3d(mesh_handle),
            MeshMaterial3d(material_handle),
        ));
    }

    console_logger::one(
        None,
        LogSev::Debug,
        LogAbout::Player,
        format!("Spawned player at pos {player_start_pos}.").as_str(),
    );
}

pub fn sys_update_player_visibility(
    mut commands: Commands,
    settings: Res<Settings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    player_q: Query<Entity, With<Player>>,
    render_q: Query<Entity, (With<Player>, With<Mesh3d>)>,
) {
    let hide = settings.core.world.hide_player;
    let is_rendered = !render_q.is_empty();

    if hide && is_rendered {
        for entity in render_q.iter() {
            commands.entity(entity).remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>();
        }
    } else if !hide && !is_rendered {
        // We need the handles again. Since we don't store them, we recreate them or 
        // better, the startup should have stored them. 
        // For simplicity here, just recreate (AssetServer would be better for real assets).
        let mesh_handle = meshes.add(Mesh::from(Cuboid {
            half_size: Vec3::splat(0.5),
        }));
        let material_handle = materials.add(StandardMaterial {
            base_color: Color::Srgba(color::palettes::basic::GREEN),
            ..default()
        });
        for entity in player_q.iter() {
            commands.entity(entity).insert((
                Mesh3d(mesh_handle.clone()),
                MeshMaterial3d(material_handle.clone()),
            ));
        }
    }
}
