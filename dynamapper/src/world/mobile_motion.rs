//! How a body moves as the client animates it: the direction it faces and whether it stands, walks or runs. A step
//! turns the body toward the step and sets the movement; the body stands once a step's time has passed with no other
//! step after it.

use bevy::prelude::*;

/// The client's facings: 0 north (UO y - 1), then clockwise to 7 north-west.
pub const DIRECTION_COUNT: u8 = 8;

/// The time a step takes in the Classic client (ClassicUO `MovementSpeed`): on foot, walking and running, then
/// mounted, walking and running.
pub const STEP_SECS_WALK: f32 = 0.4;
pub const STEP_SECS_RUN: f32 = 0.2;
pub const STEP_SECS_MOUNTED_WALK: f32 = 0.2;
pub const STEP_SECS_MOUNTED_RUN: f32 = 0.1;

/// How many of its step times a body keeps moving after its last step before it stands: past one, so that the next
/// step of a held key comes before it stops.
const STAND_AFTER_STEPS: f32 = 1.5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Movement {
    #[default]
    Stand,
    Walk,
    Run,
}

/// The time a step takes moving as `movement`, mounted or not; standing, the walking time.
pub fn step_secs(movement: Movement, mounted: bool) -> f32 {
    match (movement, mounted) {
        (Movement::Run, false) => STEP_SECS_RUN,
        (Movement::Run, true) => STEP_SECS_MOUNTED_RUN,
        (_, false) => STEP_SECS_WALK,
        (_, true) => STEP_SECS_MOUNTED_WALK,
    }
}

/// The facing of a step by `delta` in UO x and y, each -1, 0 or 1; none for no step.
pub fn direction_of_step(delta: IVec2) -> Option<u8> {
    let direction = match (delta.x.signum(), delta.y.signum()) {
        (0, -1) => 0,
        (1, -1) => 1,
        (1, 0) => 2,
        (1, 1) => 3,
        (0, 1) => 4,
        (-1, 1) => 5,
        (-1, 0) => 6,
        (-1, -1) => 7,
        _ => return None,
    };
    Some(direction)
}

/// A body's facing and movement.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct MobileMotion {
    /// 0..`DIRECTION_COUNT`, as `direction_of_step`.
    pub facing: u8,
    pub movement: Movement,
    /// The seconds since the last step, and the time that step takes.
    since_step: f32,
    step_secs: f32,
    /// The seconds since the movement last changed: the time into the clip shown.
    clip_secs: f32,
}

impl MobileMotion {
    /// A step by `delta`, running or walking, mounted or not.
    pub fn step(&mut self, delta: IVec2, run: bool, mounted: bool) {
        if let Some(facing) = direction_of_step(delta) {
            self.facing = facing;
        }
        let movement = if run { Movement::Run } else { Movement::Walk };
        self.set_movement(movement);
        self.since_step = 0.0;
        self.step_secs = step_secs(movement, mounted);
    }

    /// The time a step of the current movement takes, as `step_secs`.
    pub fn step_secs(&self, mounted: bool) -> f32 {
        step_secs(self.movement, mounted)
    }

    /// The seconds into the clip of the current movement.
    pub fn clip_secs(&self) -> f32 {
        self.clip_secs
    }

    fn set_movement(&mut self, movement: Movement) {
        if self.movement != movement {
            self.movement = movement;
            self.clip_secs = 0.0;
        }
    }

    /// Advances the clock by `delta_secs`: the body stands once its last step is long past.
    pub fn tick(&mut self, delta_secs: f32) {
        self.since_step += delta_secs;
        self.clip_secs += delta_secs;
        if self.movement != Movement::Stand && self.since_step > self.step_secs * STAND_AFTER_STEPS {
            self.set_movement(Movement::Stand);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_faces_its_direction_clockwise_from_north() {
        let steps = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];
        for (direction, (x, y)) in steps.into_iter().enumerate() {
            assert_eq!(direction_of_step(IVec2::new(x, y)), Some(direction as u8));
        }
        assert_eq!(direction_of_step(IVec2::ZERO), None);
    }

    #[test]
    fn a_body_stands_once_its_step_is_long_past() {
        let mut motion = MobileMotion::default();
        motion.step(IVec2::new(1, 0), true, false);
        assert_eq!((motion.facing, motion.movement), (2, Movement::Run));
        motion.tick(STEP_SECS_RUN);
        assert_eq!(motion.movement, Movement::Run);
        motion.tick(STEP_SECS_RUN);
        assert_eq!(motion.movement, Movement::Stand);
        assert_eq!(motion.facing, 2);
        assert_eq!(motion.clip_secs(), 0.0);
    }

    #[test]
    fn steps_of_one_movement_keep_its_clip_running() {
        let mut motion = MobileMotion::default();
        motion.step(IVec2::new(0, 1), false, false);
        motion.tick(STEP_SECS_WALK);
        motion.step(IVec2::new(1, 1), false, false);
        assert_eq!(motion.facing, 3);
        assert_eq!(motion.clip_secs(), STEP_SECS_WALK);
        motion.step(IVec2::new(1, 1), true, false);
        assert_eq!(motion.clip_secs(), 0.0);
    }
}
