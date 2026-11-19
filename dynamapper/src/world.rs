//! The logical world: UO coordinates, the map, statics and multi stores, and the player's movement and teleport.
//! The render tree draws from it (`core::render::scene::world` is the rendered world, not this one).

pub mod coords;
pub mod maps;
pub mod multis;
pub mod player_movement;
pub mod statics;
pub(crate) mod teleport;
