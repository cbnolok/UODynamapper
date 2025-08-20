use crate::core::render::scene::player::Player;
use crate::core::system_sets::*;
use crate::prelude::*;
use crate::util_lib::math::Between;
use bevy::prelude::*;
use bevy::render::camera::ScalingMode;
use bevy::window::Window;
use crate::external_data::settings::Settings;

pub const UO_TILE_PIXEL_SIZE: f32 = 44.0;

/* PUBLIC CONSTANTS: ZOOM */
pub const DEFAULT_ZOOM: f32 = 1.0;
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 6.0;

/* RENDERING MAGIC CONSTANTS */
/// Magic number found through trial and error with the aim of rendering tiles of same width and height.
const ORTHO_WIDTH_SCALE_FACTOR: f32 = 1.79;

/// Factor to correct the rendered tile size to our desired size.
/// Due to the orthographic projection, pixel size is not 1:1 but it will be distorted.
pub const TILE_SIZE_FACTOR: f32 = {
    // Using ORTHO_WIDTH_SCALE_FACTOR, the tiles are rendered bigger than desired.
    const MEASURED_TILE_PIXEL_SIZE: f32 = 62.0;
    // The pixel width and height of a diamond tile at neutral zoom (UO standard).
    const DESIRED_TILE_PIXEL_SIZE: f32 = UO_TILE_PIXEL_SIZE;
    MEASURED_TILE_PIXEL_SIZE / DESIRED_TILE_PIXEL_SIZE
};

const ORTHO_SIZE_FACTOR: f32 = {
    const DESIRED_TILE_PIXEL_SIZE: f32 = UO_TILE_PIXEL_SIZE;
    // Calculate correction factor to scale down the rendered size via the projection settings.
    DESIRED_TILE_PIXEL_SIZE / TILE_SIZE_FACTOR
};

#[derive(Resource, Clone, Copy, Debug)]
pub struct RenderZoom(pub f32);

impl Default for RenderZoom {
    fn default() -> Self {
        RenderZoom(DEFAULT_ZOOM)
    }
}
impl RenderZoom {
    pub fn write_val(&mut self, val: f32) {
        self.0 = val.clamp(MIN_ZOOM, MAX_ZOOM);
    }
}

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PlayerCamera;
impl PlayerCamera {
    pub const BASE_OFFSET_FROM_PLAYER: Vec3 = Vec3::new(5.0, 5.0, 5.0);
}

pub struct CameraPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(CameraPlugin);

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(
            Startup,
            sys_setup_cam.in_set(StartupSysSet::SetupSceneStage1),
        )
        .insert_resource(RenderZoom::default())
        .add_systems(Update, sys_update_camera_projection_to_view)
        .add_systems(
            Update,
            sys_camera_follow_player.in_set(MovementSysSet::UpdateCamera),
        );
    }
}

fn sys_setup_cam(
    mut commands: Commands,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
    settings: Res<Settings>,
) {
    let main_window = windows.single().unwrap();
    let window_width = main_window.resolution.width() as f32;
    let window_height = main_window.resolution.height() as f32 / ORTHO_WIDTH_SCALE_FACTOR;
    let zoom = render_zoom.0;
    assert!(zoom.between(MIN_ZOOM, MAX_ZOOM));

    // Compute the orthographic width/height (world units) so that visible tiles fill the window at tile size/zoom.
    // How many world units can fit horizontally & vertically?
    let ortho_width = window_width / ORTHO_SIZE_FACTOR;
    let ortho_height = window_height / ORTHO_SIZE_FACTOR;
    //println!("Ortographic camera width={ortho_width}, height={ortho_height}");

    // Find player start position for focus (if needed).
    let player_start_pos: Vec3 = settings.world.start_p.to_bevy_vec3_ignore_map();

    // Setup camera with "military"/oblique angle, looking at player start.
    commands.spawn((
        PlayerCamera::default(),
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            // NOTE: You control zoom by adjusting .scale (or by adjusting orthographic width/height).
            scale: 1.0 * zoom,
            scaling_mode: ScalingMode::Fixed {
                width: ortho_width,
                height: ortho_height,
            },
            near: -10000.0,
            far: 10000.0,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(player_start_pos + PlayerCamera::BASE_OFFSET_FROM_PLAYER)
            .looking_at(player_start_pos, Vec3::Y),
        GlobalTransform::default(),
    ));

    logger::one(None, LogSev::Debug, LogAbout::Camera, "Spawned.");
}

//------------------------------------
// World light
//------------------------------------

// We won't use a world light source, we'll bake the light in the material and the shader.
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

fn sys_update_camera_projection_to_view(
    mut camera_q: Query<&mut Projection, With<Camera3d>>,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
) {
    let main_window = windows.single().unwrap();
    let window_width = main_window.resolution.width() as f32;
    let window_height = main_window.resolution.height() as f32 / ORTHO_WIDTH_SCALE_FACTOR;
    let zoom = render_zoom.0;
    assert!(zoom.between(MIN_ZOOM, MAX_ZOOM));

    // Compute the orthographic width/height (world units) so that visible tiles fill the window at tile size/zoom.
    // How many world units can fit horizontally & vertically?
    let ortho_width = window_width / ORTHO_SIZE_FACTOR;
    let ortho_height = window_height / ORTHO_SIZE_FACTOR;

    let mut proj = camera_q.single_mut().unwrap();
    if let Projection::Orthographic(ref mut ortho) = *proj {
        ortho.scaling_mode = ScalingMode::Fixed {
            width: ortho_width,
            height: ortho_height,
        };
        ortho.scale = 1.0 * zoom;
    }
}

fn sys_camera_follow_player(
    mut camera_q: Query<&mut Transform, (With<Camera3d>, Without<Player>)>,
    player_q: Query<&Transform, (With<Player>, Without<Camera3d>)>,
) {
    let mut camera_transform = camera_q.single_mut().unwrap();
    let player_transform = player_q.single().unwrap();

    *camera_transform = Transform::from_translation(
        player_transform.translation.clone() + PlayerCamera::BASE_OFFSET_FROM_PLAYER,
    )
    .looking_at(player_transform.translation, Vec3::Y);
}

