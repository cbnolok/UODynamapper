use bevy::prelude::Vec3;

#[derive(Clone, Copy, Debug, Default)]
pub struct UOVec3 {
    pub x: u16,
    pub y: u16,
    pub z: i8,
}
impl UOVec3 {
    #[inline(always)]
    pub const fn new(x: u16, y: u16, z: i8) -> Self {
        Self { x, y, z }
    }
    #[inline(always)]
    pub fn to_vec3(&self) -> Vec3 {
        Vec3::new(self.x as f32, self.z as f32, self.y as f32)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UOVec4 {
    pub x: u16,
    pub y: u16,
    pub z: i8,
    pub m: u8,
}
impl UOVec4 {
    #[inline(always)]
    pub const fn new(x: u16, y: u16, z: i8, m: u8) -> Self {
        Self { x, y, z, m }
    }
    pub fn to_bevy_vec3(&self) -> (Vec3, u8) {
        (Vec3::new(self.x as f32, self.z as f32, self.y as f32), self.m)
    }
    pub fn to_bevy_vec3_ignore_map(&self) -> Vec3 {
        Vec3::new(self.x as f32, self.z as f32, self.y as f32)
    }
}

pub trait ToUOVec {
    fn to_uo_vec3(&self) -> UOVec3;
    fn to_uo_vec4(&self, map: u8) -> UOVec4;
}
impl ToUOVec for Vec3 {
    #[inline(always)]
    fn to_uo_vec3(&self) -> UOVec3 {
        UOVec3::new(self.x as u16, self.z as u16, self.y as i8)
    }
    #[inline(always)]
    fn to_uo_vec4(&self, map: u8) -> UOVec4 {
        UOVec4::new(self.x as u16, self.z as u16, self.y as i8, map)
    }
}
