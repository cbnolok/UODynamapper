//! Lighting: the static lights and the client light sprites, what covers a light, the light scene they draw into,
//! the client night composite, the player's light, and the item light scales.

pub mod static_lights;
pub mod client_light_sprites;
pub mod light_cover;
pub(crate) mod light_scene;
pub mod client_night;
pub mod dynamic_light;
pub mod light_scale;

#[cfg(test)]
mod light_scene_tests;
