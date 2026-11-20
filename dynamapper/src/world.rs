//! The logical world: UO coordinates, the map, statics and multi stores, the player, its movement and teleport, and the
//! world analyses the renderer draws by: the caves and the floor-view cut (z-view).
//! The render tree draws from it (`render::scene::world` is the rendered world, not this one).

pub mod caves;
pub mod coords;
pub mod maps;
pub mod multis;
pub mod player;
pub mod mobile_motion;
pub mod player_movement;
pub mod statics;
pub(crate) mod teleport;
pub mod z_view;
