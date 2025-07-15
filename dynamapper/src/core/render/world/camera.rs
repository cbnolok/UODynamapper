use super::scene::SceneStartupData;
use crate::core::system_sets::*;
use crate::prelude::*;
//use bevy::pbr::ClusterConfig;
use bevy::prelude::*;
use bevy::render::camera::ScalingMode;

#[derive(Component)]
struct PlayerCamera;
impl PlayerCamera {
    const BASE_OFFSET_FROM_PLAYER: Vec3 = Vec3::new(5.0, 5.0, 5.0);
}

pub struct CameraPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(CameraPlugin);

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(
            OnEnter(AppState::SetupScene),
            sys_setup_cam.in_set(StartupSysSet::SetupScene),
        );
    }
}

pub fn sys_setup_cam(
    mut commands: Commands,
    scene_startup_data_res: Option<Res<SceneStartupData>>,
    //scene_update_data_res: Option<Res<SceneUpdateData>>,
    //player_q: Query<&Transform, With<player::Player>>,
    //mut camera_q: Query<&mut Transform, With<PlayerCamera>>,
    //mut camera_q: Query<
    //    (&mut Transform, &PlayerCameraTransform),
    //    (Without<Player>, With<PlayerCamera>),
    //>,
) {
    log_system_add_onenter::<CameraPlugin>(AppState::SetupScene, fname!());

    let player_start_pos = scene_startup_data_res.unwrap().player_start_pos;
    //let chunk_draw_range = scene_update_data_res.unwrap().chunk_draw_range;

    //------------------------------------
    // World light
    //------------------------------------

    // We won't use a world light source, we'll bake the light from the material and the shader.
    // We use it now just to light the "player" cube.
    /*
    // Set up a directional light (sun)
    commands.spawn((
        DirectionalLight {
            shadows_enabled: false, // Disable shadows if not needed
            ..default()
        },
        Transform::from_xyz(8.0, 50.0, 8.0).looking_at(Vec3::new(8.0, 0.0, 8.0), Vec3::Y),
        GlobalTransform::default(), // Needed for transforming the light in world space
    ));
    */

    //------------------------------------
    // Camera
    //------------------------------------

    // Center of the chunk/grid
    let center = player_start_pos.to_bevy_vec3_ignore_map();

    // Non-UO Camera position: 30 units above & 30 units back
    //let cam_pos = Vec3::new(center.x, 30.0, center.z + 30.0);
    let cam_pos = center + PlayerCamera::BASE_OFFSET_FROM_PLAYER;

    commands.spawn((
        Camera3d::default(), // Marker component for 3D cameras
        Projection::Orthographic(OrthographicProjection {
            // Military/oblique (used in UO):
            scale: 40.0, //4.55,
            scaling_mode: ScalingMode::Fixed {
                width: 1.65,
                height: 1.0 / 2.0_f32.sqrt(),
            },
            ..OrthographicProjection::default_3d() // Isometric projection:
                                                   //scale: 1.0,
                                                   //scaling_mode: ScalingMode::FixedVertical(2.0),
        }),
        Transform::from_xyz(cam_pos.x, cam_pos.y, cam_pos.z).looking_at(center, Vec3::Y),
        GlobalTransform::default(), // Needed for transforming the camera in world space
        // If we ever have render/draw complexity and we need to reduce overhead this can be an option (save on light calculations):
        /*
        ClusterConfig::FixedZ {
            // 4096 clusters is the Bevy default
            // if you don't have many lights, you can reduce this value
            total: 1024,
            // Bevy default is 24 Z-slices
            // For a top-down-view game, 1 is probably optimal.
            z_slices: 1,
            dynamic_resizing: true,
            z_config: Default::default(),
        },
        */
    ));
}
