use super::render::UOVec3;
use bevy::prelude::Vec3;

//------------------------------------
// World light
//------------------------------------

/// Used by shaders to calculate lighting.
//#[derive(Resource, Deref)]
//pub struct LightDir(pub Vec3);
pub const BAKED_GLOBAL_LIGHT: Vec3 = Vec3::new(-1.0, 2.5, -1.0);
pub const RENDER_DISTANCE_FROM_PLAYER: f32 = 80.0;
pub const PLAYER_START_P: UOVec3 = UOVec3::new(5, 5, 0);
