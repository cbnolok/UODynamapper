use crate::core::render::scene::camera::{PlayerCamera, UiCameraResource};
use crate::core::render::scene::player::Player;
use crate::core::render::scene::RecomputeVisibleChunksEvent;
use crate::core::system_sets::*;
use crate::prelude::*;
use crate::util_lib::uo_coords::UOVec4;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// Base delay between tiles at speed multiplier 1.0 (20 steps per second).
const BASE_MOVE_COOLDOWN: f32 = 0.05;

pub struct PlayerMovementPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(PlayerMovementPlugin);
impl Plugin for PlayerMovementPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.insert_resource(MoveCooldown(Timer::from_seconds(
            BASE_MOVE_COOLDOWN,
            TimerMode::Repeating,
        )))
        .insert_resource(MoveDirection::default())
        .add_systems(
            Update,
            (
                sys_player_input,
                sys_player_move,
                sys_smooth_player_transform,
            )
                .chain()
                .in_set(MovementSysSet::MovementActions)
                .run_if(not(|s: Res<Settings>| s.app.window.free_camera)),
        );
    }
}

#[derive(Resource, Default)]
pub struct MoveCooldown(Timer);

#[derive(Debug, Resource)]
pub struct MoveDirection {
    pub dir: Option<IVec2>,
    pub vertical_dir: i32,
    pub speed_multiplier: f32,
}
impl Default for MoveDirection {
    fn default() -> Self {
        Self {
            dir: None,
            vertical_dir: 0,
            speed_multiplier: 1.0,
        }
    }
}

// Reads WASD and Mouse "intent" and stores it
fn sys_player_input(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    player_q: Query<&Transform, With<Player>>,
    mut move_dir: ResMut<MoveDirection>,
    mut egui_contexts: bevy_egui::EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
) {
    // Keep movement and mouse cursor input separate.
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() {
            move_dir.dir = None;
            move_dir.vertical_dir = 0;
            return;
        }
    }
    if let Some(ui_cam) = egui_ui_camera.0 {
        if let Ok(ctx) = egui_contexts.ctx_for_entity_mut(ui_cam) {
            if ctx.wants_pointer_input() {
                move_dir.dir = None;
                move_dir.vertical_dir = 0;
                return;
            }
        }
    }

    // Mouse-driven movement still works even if movement keys are held.
    if let Some((mouse_dir, mouse_speed)) =
        parse_mouse_movement(&mouse_input, &windows, &camera_q, &player_q)
    {
        move_dir.dir = Some(mouse_dir);
        move_dir.speed_multiplier = mouse_speed;
    } else {
        // Fallback to WASD.
        move_dir.dir = parse_wasd_movement(&keyboard_input);
        move_dir.speed_multiplier = 1.0;
    }

    move_dir.vertical_dir = parse_vertical_movement(&keyboard_input);
}

fn advance_horizontal_position(current_pos: UOVec4, dir: IVec2) -> UOVec4 {
    let next_x = (i32::from(current_pos.x) + dir.x).clamp(0, u16::MAX as i32) as u16;
    let next_y = (i32::from(current_pos.y) + dir.y).clamp(0, u16::MAX as i32) as u16;
    UOVec4::new(next_x, next_y, current_pos.z, current_pos.m)
}

fn advance_vertical_position(current_pos: UOVec4, vertical_dir: i32) -> UOVec4 {
    let next_z =
        (i32::from(current_pos.z) + vertical_dir).clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    UOVec4::new(current_pos.x, current_pos.y, next_z, current_pos.m)
}

fn parse_wasd_movement(keyboard_input: &Res<ButtonInput<KeyCode>>) -> Option<IVec2> {
    let mut dir = IVec2::ZERO;
    if keyboard_input.pressed(KeyCode::KeyW) {
        dir.y -= 1;
    }
    if keyboard_input.pressed(KeyCode::KeyS) {
        dir.y += 1;
    }
    if keyboard_input.pressed(KeyCode::KeyA) {
        dir.x -= 1;
    }
    if keyboard_input.pressed(KeyCode::KeyD) {
        dir.x += 1;
    }

    if dir != IVec2::ZERO {
        Some(dir)
    } else {
        None
    }
}

fn parse_mouse_movement(
    mouse_input: &Res<ButtonInput<MouseButton>>,
    windows: &Query<&Window, With<PrimaryWindow>>,
    camera_q: &Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    player_q: &Query<&Transform, With<Player>>,
) -> Option<(IVec2, f32)> {
    if !mouse_input.pressed(MouseButton::Right) {
        return None;
    }

    let window = windows.single().ok()?;
    let cursor_pos = window.cursor_position()?;
    let (camera, camera_transform) = camera_q.single().ok()?;
    let player_transform = player_q.single().ok()?;

    let ray = camera
        .viewport_to_world(camera_transform, cursor_pos)
        .ok()?;

    // Find intersection with the player's current ground plane (Y-level)
    let ground_y = player_transform.translation.y;
    if ray.direction.y.abs() <= 0.0001 {
        return None;
    }

    let t = (ground_y - ray.origin.y) / ray.direction.y;
    let world_pos = ray.origin + ray.direction * t;

    let diff = world_pos - player_transform.translation;
    let diff_xz = Vec2::new(diff.x, diff.z);

    // Use Chebyshev distance for "tiles" distance (max of X or Z difference)
    let dist = diff_xz.abs().max_element();

    // Deadzone: stop if too close to target to prevent overshooting/vibrating.
    // In UO, you generally stop when you are "on" the tile.
    if dist <= 0.8 {
        return None;
    }

    let angle = diff_xz.y.atan2(diff_xz.x);
    let octant = (angle / (std::f32::consts::PI / 4.0)).round() as i32;
    let snapped_dir = match octant {
        0 => IVec2::new(1, 0),
        1 => IVec2::new(1, 1),
        2 => IVec2::new(0, 1),
        3 => IVec2::new(-1, 1),
        4 | -4 => IVec2::new(-1, 0),
        -3 => IVec2::new(-1, -1),
        -2 => IVec2::new(0, -1),
        -1 => IVec2::new(1, -1),
        _ => IVec2::ZERO,
    };

    if snapped_dir == IVec2::ZERO {
        return None;
    }

    // UO Speed logic: >= 3 tiles away = Run (2x speed)
    let speed_multiplier = if dist >= 3.0 { 2.0 } else { 1.0 };
    Some((snapped_dir, speed_multiplier))
}

fn parse_vertical_movement(keyboard_input: &Res<ButtonInput<KeyCode>>) -> i32 {
    let mut v_dir = 0;
    if keyboard_input.pressed(KeyCode::PageUp) {
        v_dir += 1;
    }
    if keyboard_input.pressed(KeyCode::PageDown) {
        v_dir -= 1;
    }
    v_dir
}

fn sys_player_move(
    time: Res<Time>,
    mut cooldown: ResMut<MoveCooldown>,
    move_dir: Res<MoveDirection>,
    mut query: Query<(&mut Transform, &mut Player)>,
    settings: Res<Settings>,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
) {
    let multiplier = settings.app.input.movement_speed_multiplier * move_dir.speed_multiplier;
    cooldown.0.tick(time.delta().mul_f32(multiplier));

    // Only move if cooldown finished and a direction is pressed
    if cooldown.0.just_finished() {
        let smooth_movement = settings.app.input.smooth_movement;
        if let Some(dir) = move_dir.dir {
            for (mut transform, mut player) in query.iter_mut() {
                let current_pos = player
                    .current_pos
                    .unwrap_or_else(|| UOVec4::new(0, 0, 0, 0));
                // Move by exactly 1.0 per tile/step, ignoring the multiplier for distance.
                let delta = Vec3::new(dir.x as f32, 0.0, dir.y as f32);
                if !smooth_movement {
                    transform.translation += delta;
                }

                // Sync the UO coordinate state
                let next_pos = advance_horizontal_position(current_pos, dir);
                let old_map = player.current_pos.map(|p| p.m);
                player.current_pos = Some(next_pos);
                if old_map != Some(next_pos.m) {
                    chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
                }
            }
            // NOTE: Do NOT call cooldown.0.reset() for a Repeating timer if we want to preserve
            // the fractional 'overflow' of the timer when the multiplier is high.
            // Repeating timers automatically wrap around.
        }

        if move_dir.vertical_dir != 0 {
            for (mut transform, mut player) in query.iter_mut() {
                let current_pos = player
                    .current_pos
                    .unwrap_or_else(|| UOVec4::new(0, 0, 0, 0));
                // Adjust height. Use the scale utility if available or a standard step.
                // In UO a height step is often 1, but we scale it for Bevy.
                let delta_y = crate::util_lib::uo_coords::scale_uo_z_to_bevy_units(
                    move_dir.vertical_dir as f32,
                );
                if !smooth_movement {
                    transform.translation.y += delta_y;
                }

                // Sync the UO coordinate state
                let next_pos = advance_vertical_position(current_pos, move_dir.vertical_dir);
                let old_map = player.current_pos.map(|p| p.m);
                player.current_pos = Some(next_pos);
                if old_map != Some(next_pos.m) {
                    chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
                }
            }
        }
    }
}

fn sys_smooth_player_transform(
    time: Res<Time>,
    settings: Res<Settings>,
    mut query: Query<(&mut Transform, &Player)>,
) {
    if !settings.app.input.smooth_movement {
        return;
    }

    let Ok((mut transform, player)) = query.single_mut() else {
        return;
    };

    let Some(current_pos) = player.current_pos else {
        return;
    };

    let target = current_pos.to_bevy_vec3_ignore_map();
    let delta = target - transform.translation;
    if delta.length_squared() <= 0.000_001 {
        transform.translation = target;
        return;
    }

    let smoothing = (time.delta_secs() * 14.0).clamp(0.0, 1.0);
    transform.translation = transform.translation.lerp(target, smoothing);
}
