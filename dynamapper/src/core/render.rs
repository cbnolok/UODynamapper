pub mod world;

use crate::{/*fname,*/ impl_tracked_plugin, util_lib::tracked_plugin::*};
use bevy::prelude::*;


#[derive(Clone, Copy, Debug, Default)]
pub struct UOVec3 {
    x: u16,
    y: u16,
    z: i8,
}
impl UOVec3 {
    #[inline(always)]
    pub const fn new(x: u16, y: u16, z: i8) -> Self {
        Self { x, y, z }
    }
    pub fn to_vec3(&self) -> Vec3 {
        Vec3::new(self.x as f32, self.z as f32, self.y as f32)
    }
}

/*
#[derive(Clone, Copy, Debug, Default)]
pub struct UOVec4 {
    x: u16,
    y: u16,
    z: i8,
    m: u8,
}
impl UOVec4 {
    #[inline(always)]
    pub const fn new(x: u16, y: u16, z: i8, m: u8) -> Self {
        Self { x, y, z, m }
    }
    pub fn to_vec3(&self) -> (Vec3, i8) {
        (Vec3::new(x as f32, z as f32, y as f32), self.m)
    }
}
*/

pub struct RenderPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(RenderPlugin);
impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((world::WorldPlugin {
            registered_by: "RenderPlugin",
        },));
    }
}
