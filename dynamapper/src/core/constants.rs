use crate::util_lib::uo_coords::UOVec4;
use bevy::prelude::Vec3;

//------------------------------------
// World light
//------------------------------------

/// Used by shaders to calculate lighting.
//#[derive(Resource, Deref)]
//pub struct LightDir(pub Vec3);
pub const BAKED_GLOBAL_LIGHT: Vec3 = Vec3::new(-1.0, 2.5, -1.0);
pub const MAX_RENDER_DISTANCE_FROM_PLAYER: f32 = 80.0;  // TODO: remove in the future?
pub const PLAYER_START_P: UOVec4 = UOVec4::new(5, 5, 0, 0);
