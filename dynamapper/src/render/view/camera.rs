use crate::configs::settings::{AntiAliasingMode, SectGraphics, Settings};
use crate::render::scene::player::Player;
use crate::render::scene::RecomputeVisibleChunksEvent;
use super::footprint::{camera_ground_footprint, FreeCameraFootprint};
use crate::render::terrain::draw_mesh::scale_from_zoom;
use crate::export::ExportJob;
use crate::core::system_sets::*;
use crate::prelude::*;
use crate::util_lib::math::Between;
use bevy::anti_alias::{fxaa::Fxaa, smaa::Smaa};
use bevy::camera::ScalingMode;
use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::ui::IsDefaultUiCamera;
use bevy::window::Window;
use bevy_egui::EguiContexts;

pub const UO_TILE_PIXEL_SIZE: f32 = 44.0;

/* PUBLIC CONSTANTS: ZOOM */
pub const DEFAULT_ZOOM: f32 = 1.0;
pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 50.0;
mod perspective;

#[derive(Resource, Default)]
pub struct UiCameraResource(pub Option<Entity>);

/* RENDERING MAGIC CONSTANTS */
const ORTHO_NEAR: f32 = -1000.0;
const ORTHO_FAR: f32 = 1000.0;

/// sqrt(3) makes the two ground diagonals project to equal lengths at the
/// isometric pitch. The former trial-and-error value 1.79 stretched the grid.
pub(crate) const ORTHO_WIDTH_SCALE_FACTOR: f32 = 1.7320508;

/// Factor to correct the rendered tile size to our desired size.
/// Due to the orthographic projection, pixel size is not 1:1 but it will be distorted.
pub const TILE_SIZE_FACTOR: f32 = {
    // Correct the isometric diagonal's size exactly; the old measured width
    // of 62 approximated 44*sqrt(2), leaving a small horizontal scale error.
    const MEASURED_TILE_PIXEL_SIZE: f32 = UO_TILE_PIXEL_SIZE * std::f32::consts::SQRT_2;
    // The pixel width and height of a diamond tile at neutral zoom (UO standard).
    const DESIRED_TILE_PIXEL_SIZE: f32 = UO_TILE_PIXEL_SIZE;
    MEASURED_TILE_PIXEL_SIZE / DESIRED_TILE_PIXEL_SIZE
};

pub(crate) const ORTHO_SIZE_FACTOR: f32 = {
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
#[component(clone_behavior = Ignore)]
pub struct PlayerCamera;
impl PlayerCamera {
    pub const BASE_OFFSET_FROM_PLAYER: Vec3 = Vec3::new(5.0, 5.0, 5.0);

    pub fn perspective_offset(window_height: f32, zoom: f32) -> Vec3 {
        // The orbit distance matches the orthographic framing, with the art view's pitch.
        // The lens supplies vertical stretch instead of changing the camera pose.
        Vec3::ONE.normalize() * perspective::orbit_distance(window_height, zoom)
    }
}

pub struct CameraPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(CameraPlugin);

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<UiCameraResource>()
            .insert_resource(RenderZoom::default())
            .add_systems(
                Startup,
                sys_setup_cam.in_set(StartupSysSet::SetupSceneStage1),
            )
            .add_systems(
                Update,
                sys_apply_world_camera_graphics_settings
                    .run_if(resource_changed::<Settings>)
                    .in_set(MovementSysSet::UpdateCamera), // or some appropriate set
            )
            // In export mode the export job owns the world camera: its target, projection and placement.
            .add_systems(
                Update,
                sys_update_camera_projection_to_view.run_if(not(resource_exists::<ExportJob>)),
            )
            .add_systems(
                Update,
                (
                    sys_camera_zoom,
                    sys_camera_follow_player
                        .run_if(not(|s: Res<Settings>| s.app.window.free_camera)),
                    sys_free_camera_movement.run_if(|s: Res<Settings>| s.app.window.free_camera),
                    sys_stream_free_camera_view
                        .after(sys_camera_zoom)
                        .after(sys_free_camera_movement)
                        .after(sys_update_camera_projection_to_view),
                )
                    .run_if(not(resource_exists::<ExportJob>))
                    .in_set(MovementSysSet::UpdateCamera),
            );
    }
}

fn sys_setup_cam(
    mut commands: Commands,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
    settings: Res<Settings>,
) {
    log_system_add_startup::<CameraPlugin>(StartupSysSet::SetupSceneStage1, fname!());
    let main_window = windows.single().unwrap();
    let window_width = main_window.resolution.width();
    let window_height = main_window.resolution.height() / ORTHO_WIDTH_SCALE_FACTOR;
    let zoom = render_zoom.0;
    assert!(zoom.between(MIN_ZOOM, MAX_ZOOM));

    // Compute the orthographic width/height (world units) so that visible tiles fill the window at tile size/zoom.
    // How many world units can fit horizontally & vertically?
    let ortho_width = window_width / ORTHO_SIZE_FACTOR;
    let ortho_height = window_height / ORTHO_SIZE_FACTOR;
    //println!("Ortographic camera width={ortho_width}, height={ortho_height}");

    // Find player start position for focus (if needed).
    let start_p = settings.session_state.world.last_p;
    let player_start_pos: Vec3 = start_p.to_bevy_vec3_ignore_map();

    let projection = if settings.app.window.perspective_camera {
        perspective::projection()
    } else {
        Projection::Orthographic(OrthographicProjection {
            // NOTE: You control zoom by adjusting .scale (or by adjusting orthographic width/height).
            scale: 1.0 * zoom,
            scaling_mode: ScalingMode::Fixed {
                width: ortho_width,
                height: ortho_height,
            },
            near: ORTHO_NEAR,
            far: ORTHO_FAR,
            ..OrthographicProjection::default_3d()
        })
    };

    let offset = if settings.app.window.perspective_camera {
        PlayerCamera::perspective_offset(main_window.resolution.height(), zoom)
    } else {
        PlayerCamera::BASE_OFFSET_FROM_PLAYER
    };

    // Setup world camera - order 0 to render the 3D world FIRST
    let world_cam = commands
        .spawn((
            PlayerCamera,
            Camera3d::default(),
            // Disable Bevy's fullscreen tonemapping pass: the land shader already
            // performs its own Reinhard tonemapping, and the default
            // Tonemapping::ReinhardLuminance adds a redundant resolution-dependent
            // fullscreen blit every frame.
            //bevy::core_pipeline::tonemapping::Tonemapping::None,
            // Depth prepass: writes depth in a cheap pass (vertex-only, no custom
            // fragment work) so the main pass benefits from hardware early-z
            // rejection of occluded fragments behind terrain elevation.
            //bevy::core_pipeline::prepass::DepthPrepass,
            Camera {
                order: 0, // Render world first
                ..default()
            },
            projection,
            Transform::from_translation(player_start_pos + offset)
                .looking_at(player_start_pos, Vec3::Y),
            GlobalTransform::default(),
        ))
        .id();

    // Setup dedicated UI camera - order 1 to render UI ON TOP of the 3D world
    // Camera2d MUST be used for Bevy UI to work, regardless of order
    let ui_cam = commands
        .spawn((
            Camera2d,
            Camera {
                order: 10,                           // Render UI after world (on top)
                clear_color: ClearColorConfig::None, // Don't clear, just overlay
                //is_active: false,                    // TEMP: disabled for perf testing
                ..default()
            },
            IsDefaultUiCamera,
            // Explicitly attach the egui context and its pass schedule to avoid
            // relying on insertion hooks/order nuances.
            bevy_egui::PrimaryEguiContext,
            //bevy_egui::EguiContext::default(),
            //EguiMultipassSchedule::new(EguiPrimaryContextPass),
        ))
        .id();

    commands.insert_resource(UiCameraResource(Some(ui_cam)));

    apply_world_camera_graphics_settings(
        &mut commands,
        world_cam,
        Some(ui_cam),
        &settings.graphics,
    );

    console_logger::one(LogSev::Debug, LogAbout::Camera, "Spawned.");
}

//------------------------------------
// Graphic Settings Updates
//------------------------------------

// Anti-aliasing in this scene ended up being much less straightforward than
// Bevy's public API suggests, so keep the reasoning here close to the code.
//
// What we learned while debugging the black-screen regressions:
//
// 1. Post-process AA (FXAA/SMAA) must be treated separately from MSAA.
//    In practice, Bevy expects FXAA/SMAA to be enabled by attaching their
//    dedicated components to the 3D camera, while classic MSAA is controlled
//    through the camera's `Msaa` component.
//
// 2. Forcing `Hdr` on this camera was a dead end here.
//    We tried it because some internal Bevy post-process code paths are easier
//    to reason about in HDR, but in this project it caused severe regressions:
//    our custom land shader already performs its own tonemapping step, while
//    Bevy's 3D pipeline also expects to tonemap HDR views for StandardMaterial.
//    That combination produced black output / double-processing depending on
//    the exact camera state.
//
// 3. Disabling Bevy tonemapping globally was also wrong.
//    Even when that looked like a plausible fix for the custom shader, it broke
//    Bevy's StandardMaterial path (the player cube and other stock 3D content).
//    The result was another black-screen branch.  So this file must NOT try to
//    outsmart Bevy by overriding HDR/tonemapping policy per camera.
//
// 4. The final reliable rule was: keep both cameras on the same MSAA sample
//    count when they render to the same target, and only attach FXAA/SMAA to
//    the world camera.
//    The UI camera should not run post-process AA itself, but it still needs a
//    matching `Msaa` component so Bevy does not end up composing views that use
//    incompatible sample counts on the same window target.
//
// 5. Camera-side AA selection is intentionally limited to component wiring.
//    If FXAA/SMAA ever appear to do nothing, also verify the application-level
//    anti-alias plugin/bootstrap in the app setup.  This function only manages
//    per-camera state and deliberately avoids wider render-pipeline overrides.

fn sys_apply_world_camera_graphics_settings(
    mut commands: Commands,
    camera_q: Query<Entity, With<PlayerCamera>>,
    ui_camera: Res<UiCameraResource>,
    settings: Res<Settings>,
    mut applied_mode: Local<Option<AntiAliasingMode>>,
) {
    log_system_add_update::<CameraPlugin>(fname!());
    // Settings change for unrelated reasons, the player's position among them on every step: re-insert the
    // cameras' AA components only when the mode itself changes.
    if *applied_mode == Some(settings.graphics.anti_aliasing) {
        return;
    }
    if let Some(camera_entity) = camera_q.iter().next() {
        *applied_mode = Some(settings.graphics.anti_aliasing);
        apply_world_camera_graphics_settings(
            &mut commands,
            camera_entity,
            ui_camera.0,
            &settings.graphics,
        );
    }
}

fn apply_world_camera_graphics_settings(
    commands: &mut Commands,
    world_camera: Entity,
    ui_camera: Option<Entity>,
    graphics: &SectGraphics,
) {
    // FXAA/SMAA are post-process techniques, so they run with MSAA disabled.
    // True MSAA modes keep the selected sample count instead.
    let msaa = match graphics.anti_aliasing {
        AntiAliasingMode::Off | AntiAliasingMode::Fxaa | AntiAliasingMode::Smaa => Msaa::Off,
        AntiAliasingMode::Msaa2x => Msaa::Sample2,
        AntiAliasingMode::Msaa4x => Msaa::Sample4,
    };

    // Keep the UI camera aligned with the world camera sample count.
    // During debugging we hit a black-screen failure mode when only the 3D
    // camera changed `Msaa` while the UI camera kept Bevy's default sampling
    // path on the same window target.
    if let Some(ui_camera) = ui_camera {
        commands.entity(ui_camera).insert(msaa);
    }

    let mut entity_commands = commands.entity(world_camera);
    entity_commands.insert(msaa);

    // Only the world camera gets the post-process AA components.
    // The UI camera keeps its normal rendering path and just mirrors MSAA.
    //
    // Do not add `Hdr`, remove `Hdr`, or override `Tonemapping` here.
    // Those experiments were exactly what caused the earlier regressions.
    match graphics.anti_aliasing {
        AntiAliasingMode::Off => {
            entity_commands.remove::<Fxaa>().remove::<Smaa>();
        }
        AntiAliasingMode::Fxaa => {
            entity_commands.insert(Fxaa::default()).remove::<Smaa>();
        }
        AntiAliasingMode::Smaa => {
            entity_commands.remove::<Fxaa>().insert(Smaa::default());
        }
        AntiAliasingMode::Msaa2x => {
            entity_commands.remove::<Fxaa>().remove::<Smaa>();
        }
        AntiAliasingMode::Msaa4x => {
            entity_commands.remove::<Fxaa>().remove::<Smaa>();
        }
    }
}

//------------------------------------
// World light
//------------------------------------

fn sys_update_camera_projection_to_view(
    mut camera_q: Query<&mut Projection, With<PlayerCamera>>,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
    settings: Res<Settings>,
) {
    let main_window = windows.single().unwrap();
    let window_width = main_window.resolution.width();
    let window_height = main_window.resolution.height() / ORTHO_WIDTH_SCALE_FACTOR;
    let zoom = render_zoom.0;
    assert!(zoom.between(MIN_ZOOM, MAX_ZOOM));

    // Compute the orthographic width/height (world units) so that visible tiles fill the window at tile size/zoom.
    // How many world units can fit horizontally & vertically?
    let ortho_width = window_width / ORTHO_SIZE_FACTOR;
    let ortho_height = window_height / ORTHO_SIZE_FACTOR;

    let mut proj = camera_q.single_mut().unwrap();
    let is_perspective = settings.app.window.perspective_camera;

    let mut projection_changed = false;

    // Toggle logic
    let transition_to_persp = is_perspective && matches!(*proj, Projection::Orthographic(_));
    let transition_to_ortho = !is_perspective && proj.is_perspective();

    if transition_to_persp {
        *proj = perspective::projection();
        projection_changed = true;
    } else if transition_to_ortho {
        *proj = Projection::Orthographic(OrthographicProjection {
            near: ORTHO_NEAR,
            far: ORTHO_FAR,
            ..OrthographicProjection::default_3d()
        });
        projection_changed = true;
    }

    match *proj {
        Projection::Orthographic(ref mut ortho) => {
            match ortho.scaling_mode {
                ScalingMode::Fixed { width, height } => {
                    if (width - ortho_width).abs() > f32::EPSILON
                        || (height - ortho_height).abs() > f32::EPSILON
                    {
                        ortho.scaling_mode = ScalingMode::Fixed {
                            width: ortho_width,
                            height: ortho_height,
                        };
                        projection_changed = true;
                    }
                }
                _ => {
                    ortho.scaling_mode = ScalingMode::Fixed {
                        width: ortho_width,
                        height: ortho_height,
                    };
                    projection_changed = true;
                }
            }

            if (ortho.scale - zoom).abs() > f32::EPSILON {
                ortho.scale = 1.0 * zoom;
                projection_changed = true;
            }
        }
        _ => {}
    }

    if projection_changed {
        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
    }
}

fn sys_camera_follow_player(
    mut camera_q: Query<&mut Transform, (With<PlayerCamera>, Without<Player>)>,
    player_q: Query<&Transform, (With<Player>, Without<PlayerCamera>)>,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
    mut last_camera_chunk: Local<Option<(i32, i32)>>,
    settings: Res<Settings>,
    render_zoom: Res<RenderZoom>,
    windows: Query<&Window>,
) {
    let mut camera_transform = match camera_q.single_mut().ok() {
        Some(t) => t,
        None => return,
    };
    let player_transform = match player_q.single().ok() {
        Some(t) => t,
        None => return,
    };

    let offset = if settings.app.window.perspective_camera {
        // The window is gone for the last frames of an exit.
        let Ok(window) = windows.single() else { return };
        PlayerCamera::perspective_offset(window.resolution.height(), render_zoom.0)
    } else {
        PlayerCamera::BASE_OFFSET_FROM_PLAYER
    };

    let desired_transform = Transform::from_translation(player_transform.translation + offset)
        .looking_at(player_transform.translation, Vec3::Y);

    if *camera_transform != desired_transform {
        *camera_transform = desired_transform;

        let cam_translation = camera_transform.translation;
        let current_camera_chunk = (
            (cam_translation.x.floor() as i32)
                .div_euclid(crate::render::terrain::TILE_NUM_PER_CHUNK_DIM as i32),
            (cam_translation.z.floor() as i32)
                .div_euclid(crate::render::terrain::TILE_NUM_PER_CHUNK_DIM as i32),
        );
        if *last_camera_chunk != Some(current_camera_chunk) {
            *last_camera_chunk = Some(current_camera_chunk);
            chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
        }
    }
}

fn sys_camera_zoom(
    mut zoom_res: ResMut<RenderZoom>,
    mut scroll_events: MessageReader<MouseWheel>,
    mut kbd_events: MessageReader<KeyboardInput>,
    mut egui_contexts: EguiContexts,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
    settings: Res<Settings>,
    mut camera_q: Query<&mut Transform, With<PlayerCamera>>,
    windows: Query<&Window>,
) {
    // If egui is using the mouse/keyboard, don't zoom
    let ctx = match egui_contexts.ctx_mut() {
        Ok(c) => c,
        Err(_) => return,
    };
    if ctx.egui_wants_pointer_input() || ctx.egui_wants_keyboard_input() {
        return;
    }

    let mut zoom_delta = 0.0;

    // Mouse scroll
    for event in scroll_events.read() {
        zoom_delta -= event.y; // Accumulate raw scroll ticks (typically ±1)
    }

    // Keyboard +/-
    for ev in kbd_events.read() {
        if ev.state != bevy::input::ButtonState::Pressed {
            continue;
        }
        if let Key::Character(input) = &ev.logical_key {
            match input.as_str() {
                "+" | "=" => zoom_delta -= 1.0,
                "-" | "_" => zoom_delta += 1.0,
                _ => {}
            }
        }
    }

    if zoom_delta != 0.0 {
        // Exponential zoom: each scroll tick multiplies/divides by a constant factor.
        // This gives perceptually uniform zoom steps — big jumps when zoomed out,
        // fine control when zoomed in. ~25 ticks to go from 1× to 50×.
        let factor = 1.15_f32; // ~15% per tick
        let multiplier = factor.powf(zoom_delta);
        let previous_zoom = zoom_res.0;
        let new_zoom = previous_zoom * multiplier;
        zoom_res.write_val(new_zoom);
        if settings.app.window.free_camera && settings.app.window.perspective_camera {
            if let (Ok(mut transform), Ok(window)) = (camera_q.single_mut(), windows.single()) {
                // Follow perspective applies zoom to its orbit distance. Free
                // perspective uses the same dolly distance along its current
                // viewing direction, preserving any manual pan and rotation.
                let window_height = window.resolution.height();
                let distance = perspective::orbit_distance(window_height, zoom_res.0)
                    - perspective::orbit_distance(window_height, previous_zoom);
                let backward = *transform.back();
                transform.translation += backward * distance;
            }
        }
        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
    }
}

/// While the free camera is on, keeps `FreeCameraFootprint` on the tiles its view covers, and asks for a recompute
/// only when that changes: the land chunks follow the camera's view, not the player. Turning it off removes the
/// footprint, and the chunks go back to the player's.
fn sys_stream_free_camera_view(
    mut commands: Commands,
    settings: Res<Settings>,
    camera_q: Query<(&Transform, &Projection), With<PlayerCamera>>,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
    footprint: Option<Res<FreeCameraFootprint>>,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
) {
    if !settings.app.window.free_camera {
        if footprint.is_some() {
            commands.remove_resource::<FreeCameraFootprint>();
            chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
        }
        return;
    }
    let (Ok((transform, projection)), Ok(window)) = (camera_q.single(), windows.single()) else {
        return;
    };
    let zoom = render_zoom.0.clamp(MIN_ZOOM, MAX_ZOOM);
    let viewport = Vec2::new(window.width(), window.height());
    let current = FreeCameraFootprint(camera_ground_footprint(
        projection,
        transform,
        viewport,
        zoom,
        scale_from_zoom(zoom),
    ));
    if footprint.as_deref() != Some(&current) {
        commands.insert_resource(current);
        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
    }
}

fn sys_free_camera_movement(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut camera_q: Query<&mut Transform, With<PlayerCamera>>,
    mut egui_contexts: EguiContexts,
) {
    let ctx = match egui_contexts.ctx_mut() {
        Ok(c) => c,
        Err(_) => return,
    };
    if ctx.egui_wants_keyboard_input() {
        return;
    }

    let Some(mut transform) = camera_q.single_mut().ok() else {
        return;
    };

    let move_speed = 10.0 * time.delta_secs();
    // Reduced from 1.0 to 0.15 for finer-grained rotation control (~8.6°/s).
    let rotate_speed = 0.15 * time.delta_secs();
    let mut move_vec = Vec3::ZERO;

    if keyboard.pressed(KeyCode::ArrowUp) {
        move_vec.z -= 1.0;
        move_vec.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::ArrowDown) {
        move_vec.z += 1.0;
        move_vec.x += 1.0;
    }
    if keyboard.pressed(KeyCode::ArrowLeft) {
        move_vec.x -= 1.0;
        move_vec.z += 1.0;
    }
    if keyboard.pressed(KeyCode::ArrowRight) {
        move_vec.x += 1.0;
        move_vec.z -= 1.0;
    }

    // Altitude
    let mut y_delta = 0.0;
    if keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight) {
        if keyboard.pressed(KeyCode::ArrowUp) {
            y_delta += 1.0;
        }
        if keyboard.pressed(KeyCode::ArrowDown) {
            y_delta -= 1.0;
        }
    }

    // Rotation (Yaw, Pitch, Roll) - Only active when Right Shift is held
    if keyboard.pressed(KeyCode::ShiftRight) {
        // Yaw (A/D)
        if keyboard.pressed(KeyCode::KeyA) {
            transform.rotate_local_y(rotate_speed);
        }
        if keyboard.pressed(KeyCode::KeyD) {
            transform.rotate_local_y(-rotate_speed);
        }
        // Pitch (W/S)
        if keyboard.pressed(KeyCode::KeyW) {
            transform.rotate_local_x(rotate_speed);
        }
        if keyboard.pressed(KeyCode::KeyS) {
            transform.rotate_local_x(-rotate_speed);
        }
        // Roll (Q/E)
        if keyboard.pressed(KeyCode::KeyQ) {
            transform.rotate_local_z(rotate_speed);
        }
        if keyboard.pressed(KeyCode::KeyE) {
            transform.rotate_local_z(-rotate_speed);
        }
    }

    if move_vec != Vec3::ZERO {
        transform.translation += move_vec.normalize() * move_speed;
    }

    if y_delta != 0.0 {
        transform.translation.y += y_delta * move_speed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MAP_SIZE: (u32, u32) = (7168, 4096);
    const TEST_VIEWPORT: Vec2 = Vec2::new(1900.0, 1000.0);

    fn follow_orthographic_projection(viewport: Vec2, zoom: f32) -> Projection {
        Projection::Orthographic(OrthographicProjection {
            near: ORTHO_NEAR,
            far: ORTHO_FAR,
            scaling_mode: ScalingMode::Fixed {
                width: viewport.x / ORTHO_SIZE_FACTOR,
                height: viewport.y / ORTHO_WIDTH_SCALE_FACTOR / ORTHO_SIZE_FACTOR,
            },
            scale: zoom,
            ..OrthographicProjection::default_3d()
        })
    }

    /// The free camera left at the follow camera's pose streams at least the chunks the follow view does.
    #[test]
    fn free_camera_footprint_covers_the_follow_view_at_the_follow_pose() {
        let focus = Vec3::new(1500.0, 0.0, 1600.0);
        for zoom in [MIN_ZOOM, 1.0, 4.0] {
            let scale = scale_from_zoom(zoom);
            let transform = Transform::from_translation(focus + PlayerCamera::BASE_OFFSET_FROM_PLAYER)
                .looking_at(focus, Vec3::Y);
            let footprint = camera_ground_footprint(
                &follow_orthographic_projection(TEST_VIEWPORT, zoom),
                &transform,
                TEST_VIEWPORT,
                zoom,
                scale,
            );
            let streamed = super::super::footprint::chunks_in_tile_rect(&footprint, TEST_MAP_SIZE.0, TEST_MAP_SIZE.1, scale);
            let followed = super::super::footprint::compute_visible_chunks(
                focus, zoom, TEST_VIEWPORT.x, TEST_VIEWPORT.y, TEST_MAP_SIZE.0, TEST_MAP_SIZE.1, scale,
            );
            let missing: Vec<_> = followed.iter().filter(|chunk| streamed.binary_search(chunk).is_err()).collect();
            assert!(missing.is_empty(), "zoom {zoom}: the free footprint misses the follow view's chunks {missing:?}");
        }
    }

    /// A free perspective camera looking at the horizon streams up to its reach, not to the map's edge, and its
    /// footprint lies on the chunk grid, so moving within a chunk asks for no recompute.
    #[test]
    fn free_camera_footprint_at_the_horizon_stops_at_the_reach() {
        let zoom = 1.0;
        let scale = scale_from_zoom(zoom);
        let mut projection = perspective::projection();
        projection.update(TEST_VIEWPORT.x, TEST_VIEWPORT.y);
        let position = Vec3::new(2000.0, 50.0, 2000.0);
        let transform = Transform::from_translation(position).looking_to(Vec3::new(1.0, 0.0, 1.0), Vec3::Y);
        let footprint = camera_ground_footprint(&projection, &transform, TEST_VIEWPORT, zoom, scale);
        let span = crate::render::terrain::TILE_NUM_PER_CHUNK_DIM * scale;
        for edge in [footprint.tile_x0, footprint.tile_y0, footprint.tile_x1, footprint.tile_y1] {
            assert_eq!(edge % span, 0, "{footprint:?} is off the chunk grid");
        }
        let half_view = super::super::footprint::orthographic_ground_half_extent(zoom, TEST_VIEWPORT.x, TEST_VIEWPORT.y);
        let bound = 2.0 * (super::super::footprint::FREE_CAMERA_REACH_HALF_VIEWS * half_view
            + super::super::footprint::chunk_edge_pad_tiles(scale)
            + span as f32);
        assert!((footprint.tile_x1 - footprint.tile_x0) as f32 <= bound, "{footprint:?} wider than {bound}");
        assert!((footprint.tile_y1 - footprint.tile_y0) as f32 <= bound, "{footprint:?} taller than {bound}");
        let nudged = Transform { translation: position + Vec3::new(0.01, 0.0, 0.01), ..transform };
        assert_eq!(camera_ground_footprint(&projection, &nudged, TEST_VIEWPORT, zoom, scale), footprint);
    }

    /// Settings change for reasons unrelated to AA (the player's position on every step): the cameras' AA components
    /// are inserted again only when the mode itself changes.
    #[test]
    fn camera_aa_components_are_reapplied_only_when_the_mode_changes() {
        let mut settings = crate::configs::settings::bundled_settings();
        settings.graphics.anti_aliasing = AntiAliasingMode::Msaa4x;
        let mut app = App::new();
        let camera = app.world_mut().spawn(PlayerCamera).id();
        app.insert_resource(settings)
            .insert_resource(UiCameraResource(None))
            .add_systems(Update, sys_apply_world_camera_graphics_settings);
        app.update();
        assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Sample4));

        // Same mode, settings touched: nothing is inserted again.
        app.world_mut().entity_mut(camera).remove::<Msaa>();
        app.world_mut().resource_mut::<Settings>().session_state.window.zoom += 1.0;
        app.update();
        assert_eq!(app.world().get::<Msaa>(camera), None);

        app.world_mut().resource_mut::<Settings>().graphics.anti_aliasing = AntiAliasingMode::Fxaa;
        app.update();
        assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Off));
        assert!(app.world().get::<Fxaa>(camera).is_some());
    }

    /// Both cameras sit on the isometric diagonal (1, 1, 1) of their focus and look back along it, whatever the window
    /// and the zoom: the orthographic one at BASE_OFFSET_FROM_PLAYER, the perspective one at its orbit distance. The
    /// pixel scale of that view (the 44 x 44 diamond, 4 px per z unit) is guarded by world/coords.rs
    /// `default_camera_matches_legacy_tile_pixel_and_height_steps`, the orbit distance's by perspective.rs
    /// `orbit_distance_gives_the_focus_the_orthographic_scale_at_any_window_and_zoom`.
    #[test]
    fn both_cameras_look_along_the_isometric_diagonal() {
        const DIRECTION_TOLERANCE: f32 = 1e-6;
        const WINDOW_HEIGHTS: [f32; 3] = [600.0, 1000.0, 1692.0];
        let isometric = Vec3::ONE.normalize();
        let ortho = PlayerCamera::BASE_OFFSET_FROM_PLAYER;
        assert!((ortho.normalize() - isometric).length() < DIRECTION_TOLERANCE, "{ortho:?}");
        // The focus lies inside the orthographic depth range the depth (art/depth.wesl) is projected through.
        assert!(ortho.length() < ORTHO_FAR && -ortho.length() > ORTHO_NEAR);
        let forward = Transform::from_translation(ortho).looking_at(Vec3::ZERO, Vec3::Y).forward();
        assert!((*forward + isometric).length() < DIRECTION_TOLERANCE, "{forward:?}");
        for height in WINDOW_HEIGHTS {
            for zoom in [MIN_ZOOM, DEFAULT_ZOOM, MAX_ZOOM] {
                let offset = PlayerCamera::perspective_offset(height, zoom);
                assert!((offset.normalize() - isometric).length() < DIRECTION_TOLERANCE,
                    "window height {height}, zoom {zoom}: {offset:?}");
            }
        }
    }
}
