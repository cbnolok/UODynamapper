use crate::core::render::scene::player::Player;
use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;

const MOVE_COOLDOWN: f32 = 0.01; // seconds

pub struct PlayerMovementPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(PlayerMovementPlugin);
impl Plugin for PlayerMovementPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app
            .insert_resource(MoveCooldown(Timer::from_seconds(
                MOVE_COOLDOWN,
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
}

fn sys_player_move(
    time: Res<Time>,
    mut cooldown: ResMut<MoveCooldown>,
    move_dir: Res<MoveDirection>,
    mut query: Query<&mut Transform, With<Player>>,
) {
    cooldown.0.tick(time.delta());

    // Only move if cooldown finished and a direction is pressed
    if cooldown.0.finished() {
        if let Some(dir) = move_dir.dir {
            for mut transform in query.iter_mut() {
                // Move by exactly 1.0 per tile/step
                let delta = Vec3::new(dir.x as f32, 0.0, dir.y as f32);
                transform.translation += delta;
            }
            cooldown.0.reset();
        }
    }
}
