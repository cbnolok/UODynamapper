use crate::render::view::camera::{PlayerCamera, UiCameraResource};
use crate::world::player::Player;
use crate::world::maps::WorldGeoData;
use crate::render::scene::RecomputeVisibleChunksEvent;
use crate::app::system_sets::*;
use crate::input::{InjectedPointer, effective_cursor_position};
use crate::prelude::*;
use crate::world::coords::UOVec4;
use crate::world::mobile_motion::{direction_of_step, step_secs, MobileMotion, Movement};
use crate::world::statics::StaticsStoreRes;
use crate::world::walk_z::{body_step, MapWalkCells};
use crate::content::{MapPlanesRes, TileMetaPackageRes};
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
                .in_set(MovementSystems::MovementActions)
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
    /// The run key is held, or the cursor is far: a drawn body runs.
    pub run: bool,
}
impl Default for MoveDirection {
    fn default() -> Self {
        Self {
            dir: None,
            vertical_dir: 0,
            speed_multiplier: 1.0,
            run: false,
        }
    }
}

/// The cells a drawn body slides at most to its position; farther, it is placed there at once.
const MAX_SLIDE_CELLS: f32 = 2.0;

/// The mouse's speed multiplier from which a drawn body runs (`parse_mouse_movement`).
const MOUSE_RUN_SPEED_MULTIPLIER: f32 = 2.0;

/// Whether the player is drawn as a body, and whether it rides: then it moves at the client's pace
/// (`mobile_motion::step_secs`) instead of the free speed of the marker.
fn drawn_body(settings: &Settings) -> Option<bool> {
    let world = &settings.core.world;
    (world.player_body != 0 && !world.hide_player).then_some(world.player_mount_body != 0)
}

// Reads WASD and Mouse "intent" and stores it
fn sys_player_input(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    injected_pointer: Option<Res<InjectedPointer>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    player_q: Query<&Transform, With<Player>>,
    mut move_dir: ResMut<MoveDirection>,
    mut egui_contexts: bevy_egui::EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    settings: Res<Settings>,
) {
    // Keep movement and mouse cursor input separate.
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        // ctx()
        if ctx.egui_wants_pointer_input() {
            move_dir.dir = None;
            move_dir.vertical_dir = 0;
            return;
        }
    }
    if let Some(ui_cam) = egui_ui_camera.0 {
        if let Ok(ctx) = egui_contexts.ctx_for_entity_mut(ui_cam) {
            // ctx_for_entity
            if ctx.egui_wants_pointer_input() {
                move_dir.dir = None;
                move_dir.vertical_dir = 0;
                return;
            }
        }
    }

    // Mouse-driven movement still works even if movement keys are held.
    if let Some((mouse_dir, mouse_speed)) =
        parse_mouse_movement(&mouse_input, &windows, &camera_q, &player_q, injected_pointer.as_deref())
    {
        move_dir.dir = Some(mouse_dir);
        move_dir.speed_multiplier = mouse_speed;
        move_dir.run = mouse_speed >= MOUSE_RUN_SPEED_MULTIPLIER;
    } else {
        // Fallback to WASD.
        move_dir.dir = parse_wasd_movement(&keyboard_input);
        move_dir.speed_multiplier = 1.0;
        move_dir.run = keyboard_input.pressed(settings.keybindings.run);
    }

    move_dir.vertical_dir = parse_vertical_movement(&keyboard_input, &settings);
}

fn advance_horizontal_position(current_pos: UOVec4, dir: IVec2, max_x: u32, max_y: u32) -> UOVec4 {
    let next_x =
        (i32::from(current_pos.x) + dir.x).clamp(0, (max_x.saturating_sub(1)) as i32) as u16;
    let next_y =
        (i32::from(current_pos.y) + dir.y).clamp(0, (max_y.saturating_sub(1)) as i32) as u16;
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
    injected_pointer: Option<&InjectedPointer>,
) -> Option<(IVec2, f32)> {
    if !mouse_input.pressed(MouseButton::Right) {
        return None;
    }

    let window = windows.single().ok()?;
    let cursor_pos = effective_cursor_position(window, injected_pointer)?;
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
    let speed_multiplier = if dist >= 3.0 { MOUSE_RUN_SPEED_MULTIPLIER } else { 1.0 };
    Some((snapped_dir, speed_multiplier))
}

fn parse_vertical_movement(keyboard_input: &Res<ButtonInput<KeyCode>>, settings: &Settings) -> i32 {
    let mut v_dir = 0;
    if keyboard_input.pressed(settings.keybindings.increase_z) {
        v_dir += 1;
    }
    if keyboard_input.pressed(settings.keybindings.decrease_z) {
        v_dir -= 1;
    }
    v_dir
}

fn sys_player_move(
    time: Res<Time>,
    mut cooldown: ResMut<MoveCooldown>,
    move_dir: Res<MoveDirection>,
    mut query: Query<(&mut Transform, &mut Player, Option<&mut MobileMotion>)>,
    settings: Res<Settings>,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
    world_geo_data: Res<WorldGeoData>,
    ground: GroundData,
) {
    let body = drawn_body(&settings);
    if let Some(mounted) = body {
        // A drawn body steps at the client's pace for its movement.
        let movement = if move_dir.run { Movement::Run } else { Movement::Walk };
        let duration = std::time::Duration::from_secs_f32(step_secs(movement, mounted));
        if cooldown.0.duration() != duration {
            cooldown.0.set_duration(duration);
        }
        cooldown.0.tick(time.delta());
    } else {
        if cooldown.0.duration().as_secs_f32() != BASE_MOVE_COOLDOWN {
            cooldown.0.set_duration(std::time::Duration::from_secs_f32(BASE_MOVE_COOLDOWN));
        }
        let multiplier = settings.app.input.movement_speed_multiplier * move_dir.speed_multiplier;
        cooldown.0.tick(time.delta().mul_f32(multiplier));
    }

    // Only move if cooldown finished and a direction is pressed
    if cooldown.0.just_finished() {
        let smooth_movement = settings.app.input.smooth_movement;
        if let Some(dir) = move_dir.dir {
            for (mut transform, mut player, motion) in query.iter_mut() {
                let current_pos = player
                    .current_pos
                    .unwrap_or_else(|| UOVec4::new(0, 0, 0, 0));
                // A drawn body follows the ground, turns beside a corner it cannot cut and stops where the client
                // blocks the step, facing it, as the client walks.
                let (dir, ground_z) = match body.map_or(BodyStep::Unknown, |_| ground.step(current_pos, dir)) {
                    BodyStep::Walk(dir, z) => (dir, Some(z)),
                    BodyStep::Blocked => {
                        if let (Some(mut motion), Some(facing)) = (motion, direction_of_step(dir)) {
                            motion.facing = facing;
                        }
                        continue;
                    }
                    BodyStep::Unknown => (dir, None),
                };
                if let (Some(mut motion), Some(mounted)) = (motion, body) {
                    motion.step(dir, move_dir.run, mounted);
                }
                // Move by exactly 1.0 per tile/step, ignoring the multiplier for distance.
                let delta = Vec3::new(dir.x as f32, 0.0, dir.y as f32);
                if !smooth_movement {
                    transform.translation += delta;
                }

                // Sync the UO coordinate state
                let (max_x, max_y) =
                    if let Some(meta) = world_geo_data.maps.get(&(current_pos.m as u32)) {
                        (meta.width, meta.height)
                    } else {
                        (u16::MAX as u32, u16::MAX as u32)
                    };
                let mut next_pos = advance_horizontal_position(current_pos, dir, max_x, max_y);
                if let Some(z) = ground_z {
                    next_pos.z = z;
                    if !smooth_movement {
                        transform.translation.y = crate::world::coords::scale_uo_z_to_bevy_units(f32::from(z));
                    }
                }
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
            for (mut transform, mut player, _) in query.iter_mut() {
                let current_pos = player
                    .current_pos
                    .unwrap_or_else(|| UOVec4::new(0, 0, 0, 0));
                // Adjust height. Use the scale utility if available or a standard step.
                // In UO a height step is often 1, but we scale it for Bevy.
                let delta_y = crate::world::coords::scale_uo_z_to_bevy_units(
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

/// The map data a drawn body walks on.
#[derive(bevy::ecs::system::SystemParam)]
struct GroundData<'w> {
    map_planes: Option<Res<'w, MapPlanesRes>>,
    statics: Option<Res<'w, StaticsStoreRes>>,
    tilemeta: Option<Res<'w, TileMetaPackageRes>>,
}

/// A drawn body's next step.
enum BodyStep {
    /// The step it takes and its z.
    Walk(IVec2, i8),
    /// The client blocks it: the body turns towards it and stays.
    Blocked,
    /// The map's data is not there: the step goes as asked, at the body's z.
    Unknown,
}

impl GroundData<'_> {
    /// The step of a drawn body at `pos` towards `dir`, as the client walks it (`body_step`).
    fn step(&self, pos: UOVec4, dir: IVec2) -> BodyStep {
        self.walk(pos, dir).unwrap_or(BodyStep::Unknown)
    }

    fn walk(&self, pos: UOVec4, dir: IVec2) -> Option<BodyStep> {
        let direction = direction_of_step(dir)?;
        let map = usize::from(pos.m);
        let plane = self.map_planes.as_ref()?.0.get(map)?.as_ref()?;
        let mut statics = self.statics.as_ref()?.0.get(map)?.as_ref()?.lock();
        let tilemeta = &self.tilemeta.as_ref()?.0;
        let mut cells = MapWalkCells::new(plane.package(), plane.size_cells(), &mut statics, tilemeta);
        let (x, y, z) = (i32::from(pos.x), i32::from(pos.y), i32::from(pos.z));
        Some(match body_step(&mut cells, x, y, z, direction) {
            Some((to_x, to_y, to_z)) => BodyStep::Walk(IVec2::new(to_x - x, to_y - y),
                to_z.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8),
            None => BodyStep::Blocked,
        })
    }
}

fn sys_smooth_player_transform(
    time: Res<Time>,
    settings: Res<Settings>,
    move_dir: Res<MoveDirection>,
    mut query: Query<(&mut Transform, &Player, Option<&mut MobileMotion>)>,
) {
    let Ok((mut transform, player, motion)) = query.single_mut() else {
        return;
    };
    // A drawn body's step time: it crosses a cell in it at an even pace, as the client slides a walking mobile.
    let body_step_secs = match (motion, drawn_body(&settings)) {
        (Some(mut motion), Some(mounted)) => {
            motion.tick(time.delta_secs(), settings.world_rendering.mobile_stop, move_dir.dir.is_some());
            Some(motion.step_secs(mounted))
        }
        _ => None,
    };
    if !settings.app.input.smooth_movement {
        return;
    }

    let Some(current_pos) = player.current_pos else {
        return;
    };

    let target = current_pos.to_bevy_vec3_ignore_map();
    let delta = target - transform.translation;
    if delta.length_squared() <= 0.000_001 {
        transform.translation = target;
        return;
    }

    if let Some(step_secs) = body_step_secs {
        // More than a step away (a teleport, a map change): there at once.
        if delta.x.abs() > MAX_SLIDE_CELLS || delta.z.abs() > MAX_SLIDE_CELLS {
            transform.translation = target;
            return;
        }
        let max_cells = time.delta_secs() / step_secs.max(f32::EPSILON);
        transform.translation = slide_towards(transform.translation, target, max_cells);
        return;
    }

    let smoothing = (time.delta_secs() * 14.0).clamp(0.0, 1.0);
    transform.translation = transform.translation.lerp(target, smoothing);
}

/// A drawn body's slide from `from` towards `target` by at most `max_cells` along each map axis. Its height follows
/// the slide evenly, as the client raises or lowers a stepping mobile over its step (ClassicUO `Mobile.ProcessSteps`,
/// `Offset.Z`): a body stepping up a stair or a slope neither floats over its cell nor sinks into the next one.
/// A change of height alone is made at once.
fn slide_towards(from: Vec3, target: Vec3, max_cells: f32) -> Vec3 {
    let towards = |from: f32, to: f32| from + (to - from).clamp(-max_cells, max_cells);
    let (x, z) = (towards(from.x, target.x), towards(from.z, target.z));
    let cells_left = |x: f32, z: f32| (target.x - x).abs().max((target.z - z).abs());
    let (before, after) = (cells_left(from.x, from.z), cells_left(x, z));
    let y = if before > 0.0 { target.y - (target.y - from.y) * (after / before) } else { target.y };
    Vec3::new(x, y, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stepping_body_rises_with_its_slide() {
        let rise = crate::world::coords::scale_uo_z_to_bevy_units(10.0);
        let (from, target) = (Vec3::ZERO, Vec3::new(1.0, rise, 0.0));
        let quarter = slide_towards(from, target, 0.25);
        assert!((quarter.x - 0.25).abs() < 1e-6 && (quarter.y - rise * 0.25).abs() < 1e-6, "{quarter}");
        let half = slide_towards(quarter, target, 0.25);
        assert!((half.y - rise * 0.5).abs() < 1e-6, "{half}");
        assert_eq!(slide_towards(half, target, 1.0), target);
        // A diagonal step: the height follows the longer axis.
        let diagonal = slide_towards(from, Vec3::new(1.0, rise, 1.0), 0.5);
        assert!((diagonal.y - rise * 0.5).abs() < 1e-6, "{diagonal}");
        // A height change alone: at once.
        assert_eq!(slide_towards(target, target + Vec3::Y, 0.25), target + Vec3::Y);
    }
}
