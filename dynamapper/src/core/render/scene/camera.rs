use crate::configs::settings::{AntiAliasingMode, SectGraphics, Settings};
use crate::core::render::scene::world::effects::fog_pass::FogPostProcessUniform;
use crate::core::render::scene::player::Player;
use crate::core::render::scene::RecomputeVisibleChunksEvent;
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

#[derive(Resource, Default)]
pub struct UiCameraResource(pub Option<Entity>);

/* RENDERING MAGIC CONSTANTS */
/// Magic number found through trial and error with the aim of rendering tiles of same width and height.
pub(crate) const ORTHO_WIDTH_SCALE_FACTOR: f32 = 1.79;

/// Factor to correct the rendered tile size to our desired size.
/// Due to the orthographic projection, pixel size is not 1:1 but it will be distorted.
pub const TILE_SIZE_FACTOR: f32 = {
    // Using ORTHO_WIDTH_SCALE_FACTOR, the tiles are rendered bigger than desired.
    const MEASURED_TILE_PIXEL_SIZE: f32 = 62.0;
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
    pub const BASE_OFFSET_FROM_PLAYER_PERSP: Vec3 =
        Vec3::new(50.0, 50.0 * ORTHO_WIDTH_SCALE_FACTOR, 50.0);
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
            .add_systems(Update, sys_update_camera_projection_to_view)
            .add_systems(
                Update,
                (
                    sys_camera_zoom,
                    sys_camera_follow_player
                        .run_if(not(|s: Res<Settings>| s.app.window.free_camera)),
                    sys_free_camera_movement.run_if(|s: Res<Settings>| s.app.window.free_camera),
                )
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
        Projection::Perspective(PerspectiveProjection {
            fov: 0.25, // Fixed, narrow FOV to mitigate edge fisheye distortion
            near: 0.1,
            far: 20000.0, // High far plane for high de-zoom
            ..PerspectiveProjection::default()
        })
    } else {
        Projection::Orthographic(OrthographicProjection {
            // NOTE: You control zoom by adjusting .scale (or by adjusting orthographic width/height).
            scale: 1.0 * zoom,
            scaling_mode: ScalingMode::Fixed {
                width: ortho_width,
                height: ortho_height,
            },
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_3d()
        })
    };

    let offset = if settings.app.window.perspective_camera {
        PlayerCamera::BASE_OFFSET_FROM_PLAYER_PERSP * zoom
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
            // Attach the fog post-process uniform. The FogPostProcessPlugin's sync
            // system will fill in the correct values from UniformState every frame.
            FogPostProcessUniform::default(),
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
) {
    log_system_add_update::<CameraPlugin>(fname!());
    if let Some(camera_entity) = camera_q.iter().next() {
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
    let transition_to_ortho = !is_perspective && matches!(*proj, Projection::Perspective(_));

    if transition_to_persp {
        *proj = Projection::Perspective(PerspectiveProjection {
            fov: 0.25,
            near: 0.1,
            far: 20000.0,
            ..PerspectiveProjection::default()
        });
        projection_changed = true;
    } else if transition_to_ortho {
        *proj = Projection::Orthographic(OrthographicProjection {
            near: -100.0,
            far: 100.0,
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
        Projection::Perspective(ref mut persp) if (persp.fov - 0.25).abs() > f32::EPSILON => {
            persp.fov = 0.25;
            persp.far = 20000.0;
            projection_changed = true;
        }
        _ => {}
    }

    if projection_changed {
        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
    }
}

fn sys_camera_follow_player(
    mut camera_q: Query<&mut Transform, (With<Camera3d>, Without<Player>)>,
    player_q: Query<&Transform, (With<Player>, Without<Camera3d>)>,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
    mut last_camera_chunk: Local<Option<(i32, i32)>>,
    settings: Res<Settings>,
    render_zoom: Res<RenderZoom>,
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
        PlayerCamera::BASE_OFFSET_FROM_PLAYER_PERSP * render_zoom.0
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
                .div_euclid(crate::core::render::scene::world::land::TILE_NUM_PER_CHUNK_DIM as i32),
            (cam_translation.z.floor() as i32)
                .div_euclid(crate::core::render::scene::world::land::TILE_NUM_PER_CHUNK_DIM as i32),
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
    mut chunk_recompute_writer: MessageWriter<super::RecomputeVisibleChunksEvent>,
) {
    // If egui is using the mouse/keyboard, don't zoom
    let ctx = match egui_contexts.ctx_mut() {
        Ok(c) => c,
        Err(_) => return,
    };
    if ctx.wants_pointer_input() || ctx.wants_keyboard_input() {
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
        let new_zoom = zoom_res.0 * multiplier;
        zoom_res.write_val(new_zoom);
        chunk_recompute_writer.write(super::RecomputeVisibleChunksEvent {});
    }
}

fn sys_free_camera_movement(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut camera_q: Query<&mut Transform, With<Camera3d>>,
    mut egui_contexts: EguiContexts,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
) {
    let ctx = match egui_contexts.ctx_mut() {
        Ok(c) => c,
        Err(_) => return,
    };
    if ctx.wants_keyboard_input() {
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

    let mut camera_changed = false;

    if move_vec != Vec3::ZERO {
        transform.translation += move_vec.normalize() * move_speed;
        camera_changed = true;
    }

    if y_delta != 0.0 {
        transform.translation.y += y_delta * move_speed;
        camera_changed = true;
    }

    if keyboard.pressed(KeyCode::ShiftRight)
        && (keyboard.pressed(KeyCode::KeyA)
            || keyboard.pressed(KeyCode::KeyD)
            || keyboard.pressed(KeyCode::KeyW)
            || keyboard.pressed(KeyCode::KeyS)
            || keyboard.pressed(KeyCode::KeyQ)
            || keyboard.pressed(KeyCode::KeyE))
    {
        camera_changed = true;
    }

    if camera_changed {
        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
    }
}
