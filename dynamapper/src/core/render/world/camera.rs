use super::scene::SceneStartupData;
use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;
use bevy::render::camera::ScalingMode;

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
const TILE_SIZE_FACTOR: f32 = {
    // Using ORTHO_WIDTH_SCALE_FACTOR, the tiles are rendered bigger than desired.
    const MEASURED_TILE_PIXEL_SIZE: f32 = 62.0;
    // The pixel width and height of a diamond tile at neutral zoom (UO standard).
    const DESIRED_TILE_PIXEL_SIZE: f32 = UO_TILE_PIXEL_SIZE;
    // Calculate correction factor to scale down the rendered size via the projection settings.
    const SCALE_FACTOR: f32 = MEASURED_TILE_PIXEL_SIZE / DESIRED_TILE_PIXEL_SIZE;
    DESIRED_TILE_PIXEL_SIZE / SCALE_FACTOR
};

#[derive(Resource, Clone, Copy, Debug)]
pub struct RenderZoom(pub f32);

impl Default for RenderZoom {
    fn default() -> Self {
        RenderZoom(DEFAULT_ZOOM)
    }
}

#[derive(Component, Clone, Copy, Debug, Default)]
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
        )
        .insert_resource(RenderZoom::default())
        .add_systems(Update, sys_update_camera_projection_to_view);
    }
}

pub fn sys_setup_cam(
    mut commands: Commands,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
    // Use any data you need for initial camera placement (e.g. player start position)
    scene_startup_data_res: Option<Res<SceneStartupData>>,
) {
    let main_window = windows.single().unwrap();
    let window_width = main_window.resolution.width() as f32;
    let window_height = main_window.resolution.height() as f32 / ORTHO_WIDTH_SCALE_FACTOR;
    let zoom = render_zoom.0.clamp(MIN_ZOOM, MAX_ZOOM);

    // Compute the orthographic width/height (world units) so that visible tiles fill the window at tile size/zoom.
    // How many world units can fit horizontally & vertically?
    let ortho_width = window_width / TILE_SIZE_FACTOR;
    let ortho_height = window_height / TILE_SIZE_FACTOR;
    //println!("Ortographic camera width={ortho_width}, height={ortho_height}");

    // Find player start position for focus (if needed).
    let player_start_pos = scene_startup_data_res
        .as_ref()
        .map(|s| s.player_start_pos.to_bevy_vec3_ignore_map())
        .unwrap_or(Vec3::ZERO);

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
            ..OrthographicProjection::default_3d()
        }),
        // UO-military projection: viewed from y+z axis, angled
        Transform::from_translation(player_start_pos + PlayerCamera::BASE_OFFSET_FROM_PLAYER)
            .looking_at(player_start_pos, Vec3::Y),
        GlobalTransform::default(),
    ));
}

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

pub fn sys_update_camera_projection_to_view(
    mut camera_q: Query<&mut Projection, With<Camera3d>>,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
) {
    let main_window = windows.single().unwrap();
    let window_width = main_window.resolution.width() as f32;
    let window_height = main_window.resolution.height() as f32 / ORTHO_WIDTH_SCALE_FACTOR;
    let zoom = render_zoom.0.clamp(MIN_ZOOM, MAX_ZOOM);

    // Compute the orthographic width/height (world units) so that visible tiles fill the window at tile size/zoom.
    // How many world units can fit horizontally & vertically?
    let ortho_width = window_width / TILE_SIZE_FACTOR;
    let ortho_height = window_height / TILE_SIZE_FACTOR;

    let mut proj = camera_q.single_mut().unwrap();
    if let Projection::Orthographic(ref mut ortho) = *proj {
        ortho.scaling_mode = ScalingMode::Fixed {
            width: ortho_width,
            height: ortho_height,
        };
        ortho.scale = 1.0 * zoom;
    }
}

