//! Perspective distance with the same isometric lens stretch as the art view.
use bevy::{camera::{CameraProjection, SubCameraView}, math::Vec3A, prelude::*};
use super::ORTHO_WIDTH_SCALE_FACTOR;

#[derive(Debug, Clone)]
struct IsometricPerspective(PerspectiveProjection);

pub(super) fn projection() -> Projection {
    Projection::custom(IsometricPerspective(PerspectiveProjection {
        fov: 0.25, // Fixed, narrow FOV to mitigate edge fisheye distortion
        near: 0.1,
        far: 20000.0, // High far plane for high de-zoom
        ..PerspectiveProjection::default()
    }))
}

impl CameraProjection for IsometricPerspective {
    fn get_clip_from_view(&self) -> Mat4 {
        let mut matrix = self.0.get_clip_from_view();
        matrix.y_axis.y *= ORTHO_WIDTH_SCALE_FACTOR;
        matrix
    }

    fn get_clip_from_view_for_sub(&self, sub: &SubCameraView) -> Mat4 {
        let mut matrix = self.0.get_clip_from_view_for_sub(sub);
        // Scale the lens, not the crop's center offset. This also keeps tiled
        // captures and viewport-to-world picking consistent with the full view.
        matrix.y_axis.y *= ORTHO_WIDTH_SCALE_FACTOR;
        matrix
    }

    fn update(&mut self, width: f32, height: f32) { self.0.update(width, height); }
    fn far(&self) -> f32 { self.0.far() }

    fn get_frustum_corners(&self, near: f32, far: f32) -> [Vec3A; 8] {
        self.0.get_frustum_corners(near, far).map(|mut point| {
            point.y /= ORTHO_WIDTH_SCALE_FACTOR;
            point
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projected(matrix: Mat4, point: Vec3) -> Vec3 { matrix.project_point3(point) }

    #[test]
    fn lens_and_frustum_match_including_offcenter_subviews() {
        let mut projection = projection();
        projection.update(1280.0, 800.0);
        assert!(projection.is_perspective());
        let matrix = projection.get_clip_from_view();
        for corner in projection.get_frustum_corners(-0.1, -100.0) {
            let ndc = projected(matrix, corner.into());
            assert!((ndc.x.abs() - 1.0).abs() < 0.00001);
            assert!((ndc.y.abs() - 1.0).abs() < 0.00001);
        }
        let sub = SubCameraView { full_size: UVec2::new(1280, 800), size: UVec2::new(640, 400), offset: Vec2::new(128.0, 80.0) };
        let point = Vec3::new(0.1, 0.2, -5.0);
        let full = projected(matrix, point);
        let cropped = projected(projection.get_clip_from_view_for_sub(&sub), point);
        let expected = Vec2::new(
            2.0 * ((full.x * 0.5 + 0.5) * 1280.0 - 128.0) / 640.0 - 1.0,
            1.0 - 2.0 * ((0.5 - full.y * 0.5) * 800.0 - 80.0) / 400.0,
        );
        assert!((cropped.truncate() - expected).length() < 0.00001);
    }
}
