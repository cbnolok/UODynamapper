use crate::core::render::scene::player::Player;
use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;

/// Base delay between tiles at speed multiplier 1.0 (20 steps per second).
const BASE_MOVE_COOLDOWN: f32 = 0.05;

pub struct PlayerMovementPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(PlayerMovementPlugin);
impl Plugin for PlayerMovementPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app
            .insert_resource(MoveCooldown(Timer::from_seconds(
                BASE_MOVE_COOLDOWN,
                TimerMode::Repeating,
            )))
            .insert_resource(MoveDirection::default())
            .add_systems(Update, (sys_player_input, sys_player_move).in_set(MovementSysSet::MovementActions));
    }
}

#[derive(Resource, Default)]
pub struct MoveCooldown(Timer);

#[derive(Debug, Default, Resource)]
pub struct MoveDirection {
    pub dir: Option<IVec2>,
    pub vertical_dir: i32,
}
// Reads WASD "intent" and stores it
fn sys_player_input(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut move_dir: ResMut<MoveDirection>,
) {
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
    move_dir.dir = if dir != IVec2::ZERO { Some(dir) } else { None };

    let mut v_dir = 0;
    if keyboard_input.pressed(KeyCode::PageUp) {
        v_dir += 1;
    }
    if keyboard_input.pressed(KeyCode::PageDown) {
        v_dir -= 1;
    }
    move_dir.vertical_dir = v_dir;
}

fn sys_player_move(
    time: Res<Time>,
    mut cooldown: ResMut<MoveCooldown>,
    move_dir: Res<MoveDirection>,
    mut query: Query<&mut Transform, With<Player>>,
    settings: Res<Settings>,
) {
    let multiplier = settings.app.input.movement_speed_multiplier;
    cooldown.0.tick(time.delta().mul_f32(multiplier));

    // Only move if cooldown finished and a direction is pressed
    if cooldown.0.just_finished() {
        if let Some(dir) = move_dir.dir {
            for mut transform in query.iter_mut() {
                // Move by exactly 1.0 per tile/step, ignoring the multiplier for distance.
                let delta = Vec3::new(dir.x as f32, 0.0, dir.y as f32);
                transform.translation += delta;
            }
            cooldown.0.reset();
        }

        if move_dir.vertical_dir != 0 {
            for mut transform in query.iter_mut() {
                // Adjust height. Use the scale utility if available or a standard step.
                // In UO a height step is often 1, but we scale it for Bevy.
                let delta_y = crate::util_lib::uo_coords::scale_uo_z_to_bevy_units(move_dir.vertical_dir as f32);
                transform.translation.y += delta_y;
            }
            cooldown.0.reset();
        }
    }
}
