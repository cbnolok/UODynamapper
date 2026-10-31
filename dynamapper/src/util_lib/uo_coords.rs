use bevy::prelude::Vec3;
use serde::{Serialize, Deserialize};

/// Rendering unit conversion; logical map Z remains the original signed byte.
/// Four legacy-art pixels per Z unit in the calibrated 44-pixel isometric grid.
/// The default camera projects world Y at 44 pixels per world unit.
/// Keep world/uo_coords.wesl in sync.
pub const UO_Z_TO_WORLD_SCALE: f32 = 4.0 / 44.0;

#[inline(always)]
pub fn scale_uo_z_to_bevy_units(z: f32) -> f32 {
    z * UO_Z_TO_WORLD_SCALE // previously arbitrary (0.1)
}

#[inline(always)]
pub fn scale_bevy_units_to_uo_z(y: f32) -> f32 {
    y / UO_Z_TO_WORLD_SCALE
}

#[derive(Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
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
        Vec3::new(self.x as f32, scale_uo_z_to_bevy_units(self.z as f32), self.y as f32)
    }
}

#[derive(Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
        (Vec3::new(self.x as f32, scale_uo_z_to_bevy_units(self.z as f32), self.y as f32), self.m)
    }
    pub fn to_bevy_vec3_ignore_map(&self) -> Vec3 {
        Vec3::new(self.x as f32, scale_uo_z_to_bevy_units(self.z as f32), self.y as f32)
    }
}

pub trait ToUOVec {
    fn to_uo_vec3(&self) -> UOVec3;
    fn to_uo_vec4(&self, map: u8) -> UOVec4;
}
impl ToUOVec for Vec3 {
    #[inline(always)]
    fn to_uo_vec3(&self) -> UOVec3 {
        UOVec3::new(self.x as u16, self.z as u16, scale_bevy_units_to_uo_z(self.y).round() as i8)
    }
    #[inline(always)]
    fn to_uo_vec4(&self, map: u8) -> UOVec4 {
        UOVec4::new(self.x as u16, self.z as u16, scale_bevy_units_to_uo_z(self.y).round() as i8, map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_map_heights_roundtrip_through_render_coordinates() {
        for z in i8::MIN..=i8::MAX {
            for (x, y, map) in [(0, 0, 0), (2834, 987, 0), (u16::MAX, u16::MAX, 5)] {
                let position = UOVec4::new(x, y, z, map);
                let world = position.to_bevy_vec3_ignore_map();
                assert!(world.to_uo_vec4(map) == position, "height {z}, map {map}");
                let point = UOVec3::new(x, y, z);
                assert!(point.to_vec3().to_uo_vec3() == point, "height {z}");
            }
        }
    }

    #[test]
    fn default_camera_matches_legacy_tile_pixel_and_height_steps() {
        use bevy::{camera::CameraProjection, prelude::*};
        use crate::core::render::scene::camera::{ORTHO_SIZE_FACTOR, ORTHO_WIDTH_SCALE_FACTOR, PlayerCamera};
        let width = 1280.0;
        let height = 800.0;
        let mut projection = OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::Fixed {
                width: width / ORTHO_SIZE_FACTOR,
                height: height / (ORTHO_WIDTH_SCALE_FACTOR * ORTHO_SIZE_FACTOR),
            },
            ..OrthographicProjection::default_3d()
        };
        projection.update(width, height);
        let camera = Transform::from_translation(PlayerCamera::BASE_OFFSET_FROM_PLAYER)
            .looking_at(Vec3::ZERO, Vec3::Y);
        let clip_from_world = projection.get_clip_from_view() * camera.to_matrix().inverse();
        let pixels = |world| {
            let ndc = clip_from_world.project_point3(world);
            Vec2::new(ndc.x * width * 0.5, ndc.y * height * 0.5)
        };
        let origin = pixels(Vec3::ZERO);
        let cases = [
            (Vec3::X, Vec2::new(22.0, -22.0)),
            (Vec3::Z, Vec2::new(-22.0, -22.0)),
            (Vec3::Y * UO_Z_TO_WORLD_SCALE, Vec2::new(0.0, 4.0)),
            (Vec3::new(1.0, 0.0, -1.0) / 44.0, Vec2::new(1.0, 0.0)),
            (Vec3::Y / 44.0, Vec2::new(0.0, 1.0)),
        ];
        for (world, expected) in cases {
            assert!(((pixels(world) - origin) - expected).length() < 0.0001,
                "projection of {world:?} should be {expected:?}");
        }
    }
}
