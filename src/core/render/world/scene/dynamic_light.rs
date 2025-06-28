use bevy::prelude::*;
use crate::{fname, impl_tracked_plugin, util_lib::tracked_plugin::*};
use super::SceneStartupData;

#[derive(Component)]
pub struct PlayerDynamicLight {
    camera_player_rel_pos: Vec3,
}

pub struct PlayerDynamicLightPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(PlayerDynamicLightPlugin);

impl Plugin for PlayerDynamicLightPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(Startup, sys_spawn_dynamic_light);
    }
}

pub fn sys_spawn_dynamic_light(
    mut commands: Commands,
    //camera_q: Query<&PlayerDynamicLight>,
    scene_startup_data_res: Option<Res<SceneStartupData>>,
) {
    log_system_add_startup::<PlayerDynamicLightPlugin>(fname!());
    // Camera position relative to the player: a little south east and higher than the player.
    let camera_player_rel_pos = Vec3::new(10.0, 50.0, 8.0);
    let light_component = PlayerDynamicLight {
        camera_player_rel_pos,
    };

    let player_start_pos = scene_startup_data_res.unwrap().player_start_pos;
    let camera_pos = light_component.camera_player_rel_pos + player_start_pos;

    // Set up a directional light (sun)
    println!("Spawning directional light at {}, looking at {}.", camera_pos, player_start_pos);
    commands.spawn((
        DirectionalLight {
            shadows_enabled: false, // Disable shadows if not needed
            illuminance: 500.0,
            ..default()
        },
        Transform::from_xyz(camera_pos.x, camera_pos.y, camera_pos.z)
            .looking_at(player_start_pos, Vec3::Y),
        GlobalTransform::default(), // Needed for transforming the light in world space
        light_component,
    ));

    /*
    commands.spawn((
        PointLight {
            shadows_enabled: true,
            intensity: 10_000_000.,
            range: 100.0,
            shadow_depth_bias: 0.2,
            ..default()
        },
        Transform::from_xyz(camera_pos.x, camera_pos.y, camera_pos.z)
            .looking_at(player_start_pos, Vec3::Y),
    ));
    */
}

//scene_update_data_res: Option<Res<SceneUpdateData>>,
//cam_q: Query<&Transform, With<Camera3d>>,
//let chunk_draw_range = scene_update_data_res.unwrap().chunk_draw_range;
