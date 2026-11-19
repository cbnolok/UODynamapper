//! The player: its logical position, which movement and teleport change and every view follows.
use crate::prelude::*;
use bevy::prelude::*;

#[derive(Component)]
pub struct Player {
    pub current_pos: Option<UOVec4>,
    /// The position the scene last streamed and drew the world around (`render::scene`): the scene compares it with
    /// `current_pos` to detect moves, map changes and large jumps.
    pub prev_rendered_pos: Option<UOVec4>,
}
