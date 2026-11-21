//! Movement actions a body's `AnimationSequence` entry gives, as the Enhanced and Classic packages store them.

pub use uocf::animation_sequence::{GROUP_MOUNTED, GROUP_PEACE, GROUP_WAR, MOVE_RUN, MOVE_STAND, MOVE_WALK};

/// Sequence groups whose move actions a body record keeps: war, peace and mounted
/// (`uocf::animation_sequence::GROUP_WAR` ..= `GROUP_MOUNTED`), in that order.
pub const MOVE_ACTION_GROUPS: [i8; MOVE_ACTION_GROUP_COUNT] = [GROUP_WAR, GROUP_PEACE, GROUP_MOUNTED];
pub const MOVE_ACTION_GROUP_COUNT: usize = 3;
/// Movements of a group: stand, walk and run (`uocf::animation_sequence::MOVE_*`), in that order.
pub const MOVE_ACTION_MOVEMENTS: [u8; MOVE_ACTION_MOVEMENT_COUNT] = [MOVE_STAND, MOVE_WALK, MOVE_RUN];
pub const MOVE_ACTION_MOVEMENT_COUNT: usize = 3;
/// `move_actions` entry of a group the sequence does not have.
pub const NO_MOVE_ACTION: u16 = u16::MAX;

/// The entry of `move_actions` (by `MOVE_ACTION_GROUPS`, then `MOVE_ACTION_MOVEMENTS`) for `movement` in sequence
/// group `group`; `None` for `NO_MOVE_ACTION` or arguments outside those sets.
pub fn move_action(
    move_actions: &[[u16; MOVE_ACTION_MOVEMENT_COUNT]; MOVE_ACTION_GROUP_COUNT],
    group: i8,
    movement: u8,
) -> Option<u16> {
    let group = MOVE_ACTION_GROUPS.iter().position(|&known| known == group)?;
    let movement = MOVE_ACTION_MOVEMENTS.iter().position(|&known| known == movement)?;
    Some(move_actions[group][movement]).filter(|&action| action != NO_MOVE_ACTION)
}
