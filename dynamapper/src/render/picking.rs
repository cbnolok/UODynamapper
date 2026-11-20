//! Picking: the statics drawn under the cursor (read back from the GPU), their highlight geometry, the hovered static
//! and its outline, and the terrain cell under the cursor.
pub(crate) mod gpu_picking;
pub(crate) mod hovered_static;
pub(crate) mod static_geometry;
pub(crate) mod terrain_cursor;

use crate::settings::Settings;
use crate::world::player::Player;

/// The map the cursor picks on: the player's, else the session's last; none without a player.
pub(crate) fn cursor_map_id(player: Option<&Player>, settings: &Settings) -> Option<u8> {
    Some(player?.current_pos.map(|p| p.m).unwrap_or(settings.session_state.world.last_p.m))
}
