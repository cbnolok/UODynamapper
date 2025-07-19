use super::scene::SceneStartupData;
use crate::core::system_sets::*;
use crate::prelude::*;
//use bevy::pbr::ClusterConfig;
use bevy::prelude::*;
use bevy::render::camera::ScalingMode;

/// The pixel width and height of a diamond tile at neutral zoom (UO standard).
pub const TILE_PIXEL_SIZE: f32 = 44.0;
/// The default zoom level (1.0 == each tile is TILE_PIXEL_SIZE px).
pub const DEFAULT_ZOOM: f32 = 1.0;
/// Minimum/maximum allowed zoom (prevent extreme viewing).
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 6.0;
/// How much padding (in tiles) to render beyond what is strictly visible, for seamless edges.
pub const TILE_PADDING: usize = 2;

#[derive(Resource, Clone, Copy, Debug)]
pub struct RenderZoom(pub f32);

impl Default for RenderZoom {
    fn default() -> Self {
        RenderZoom(DEFAULT_ZOOM)
    }
}

/// Computes how many full tiles fit horizontally and vertically in the window at the current zoom.
pub fn tiles_visible_in_window(window_width: f32, window_height: f32, zoom: f32) -> (usize, usize) {
    // The number of world units fully visible, at this zoom.
    let world_units_x = window_width  / (TILE_PIXEL_SIZE * zoom);
    let world_units_y = window_height / (TILE_PIXEL_SIZE * zoom);
    // Each tile == 1.0 world unit
    let n_tiles_x = world_units_x.ceil() as usize + TILE_PADDING * 2;
    let n_tiles_y = world_units_y.ceil() as usize + TILE_PADDING * 2;
    (n_tiles_x, n_tiles_y)
}

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
        )
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
    let window_width = main_window.physical_width() as f32;
    let window_height = main_window.physical_height() as f32;
    let zoom = render_zoom.0.clamp(MIN_ZOOM, MAX_ZOOM);

    // How many tiles do we want fully visible at this zoom and window size?
    //let (tiles_x, tiles_y) = tiles_visible_in_window(window_width, window_height, zoom);

    // Compute the orthographic width/height (world units) so that visible tiles fill the window at tile size/zoom.
    //let ortho_width = tiles_x as f32 * TILE_PIXEL_SIZE;
    //let ortho_height = tiles_y as f32 * TILE_PIXEL_SIZE;
    let ortho_width = window_width / TILE_PIXEL_SIZE;   // in world units
    let ortho_height = window_height / TILE_PIXEL_SIZE;
    println!("Ortographic camera width={ortho_width}, height={ortho_height}");

    // Find player start position for focus (if needed).
    let player_start_pos = scene_startup_data_res
        .as_ref()
        .map(|s| s.player_start_pos.to_bevy_vec3_ignore_map())
        .unwrap_or(Vec3::ZERO);

    // Setup camera with "military"/oblique angle, looking at player start.
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            // NOTE: You control zoom by adjusting .scale (or by adjusting orthographic width/height).
            scale: 1.0,
            scaling_mode: ScalingMode::Fixed {
                width: ortho_width,
                height: ortho_height,
            },
            ..OrthographicProjection::default_3d()
        }),
        // UO-military projection: viewed from y+z axis, angled
        Transform::from_xyz(
            player_start_pos.x + 5.0, // player_start_pos + fixed oblique offset
            player_start_pos.y + 5.0,
            player_start_pos.z + 5.0,
        )
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

    /*
pub fn sys_update_camera_projection_to_view(
    mut camera_q: Query<&mut Projection, With<Camera3d>>,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>
) {
    let window = windows.single().unwrap();
    let zoom = render_zoom.0.clamp(MIN_ZOOM, MAX_ZOOM);
    let (tiles_x, tiles_y) = tiles_visible_in_window(
        window.physical_width() as f32,
        window.physical_height() as f32,
        zoom,
    );
    let width  = tiles_x as f32; // world units: one per tile!
    let height = tiles_y as f32;
    let mut proj = camera_q.single_mut().unwrap();
    if let Projection::Orthographic(ref mut ortho) = *proj {
        ortho.scaling_mode = ScalingMode::Fixed { width, height };
        ortho.scale = 1.0;
    }
}
    */

pub fn sys_update_camera_projection_to_view(
    mut camera_q: Query<&mut Projection, With<Camera3d>>,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
) {
    let window = windows.single().unwrap();
    let zoom = render_zoom.0.clamp(MIN_ZOOM, MAX_ZOOM);

    // How many world units can fit horizontally & vertically?
    let world_units_x = window.physical_width() as f32 / (TILE_PIXEL_SIZE * zoom);
    let world_units_y = window.physical_height() as f32 / (TILE_PIXEL_SIZE * zoom);

    // Set camera projection accordingly (world units)
    let width = world_units_x;
    let height = world_units_y;

    let mut proj = camera_q.single_mut().unwrap();
    if let Projection::Orthographic(ref mut ortho) = *proj {
        ortho.scaling_mode = ScalingMode::Fixed { width, height };
        ortho.scale = 1.0;
    }
}
