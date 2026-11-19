//! Perspective distance with the same isometric lens stretch as the art view.
use bevy::{camera::{CameraProjection, SubCameraView}, math::Vec3A, prelude::*};
use super::{ORTHO_SIZE_FACTOR, ORTHO_WIDTH_SCALE_FACTOR};

// Fixed, narrow FOV to mitigate edge fisheye distortion
const VERTICAL_FOV_RADIANS: f32 = 0.25;

#[derive(Debug, Clone)]
struct IsometricPerspective(PerspectiveProjection);

pub(super) fn projection() -> Projection {
    Projection::custom(IsometricPerspective(PerspectiveProjection {
        fov: VERTICAL_FOV_RADIANS,
        near: 0.1,
        far: 20000.0, // High far plane for high de-zoom
        ..PerspectiveProjection::default()
    }))
}

/// Distance from the camera to its focus at which a world unit there covers as many pixels as in
/// the orthographic view at the same zoom. A fixed FOV shows a fixed world height at a given
/// distance, so the distance has to grow with the window for tiles to keep their pixel size.
pub(super) fn orbit_distance(window_height: f32, zoom: f32) -> f32 {
    window_height * zoom / (2.0 * (VERTICAL_FOV_RADIANS / 2.0).tan() * ORTHO_SIZE_FACTOR)
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
    fn orbit_distance_gives_the_focus_the_orthographic_scale_at_any_window_and_zoom() {
        const VIEW_UNIT: Vec2 = Vec2::new(0.25, 0.125);
        for (width, height) in [(1280.0, 800.0), (2880.0, 1692.0)] {
            for zoom in [0.25, 1.0, 2.69, 50.0] {
                let mut perspective = projection();
                perspective.update(width, height);
                let mut ortho = Projection::Orthographic(OrthographicProjection {
                    scaling_mode: bevy::camera::ScalingMode::Fixed {
                        width: width / ORTHO_SIZE_FACTOR,
                        height: height / ORTHO_WIDTH_SCALE_FACTOR / ORTHO_SIZE_FACTOR,
                    },
                    scale: zoom,
                    ..OrthographicProjection::default_3d()
                });
                ortho.update(width, height);

                let distance = orbit_distance(height, zoom);
                let point = VIEW_UNIT.extend(-distance);
                let from_perspective = projected(perspective.get_clip_from_view(), point).truncate();
                let from_ortho = projected(ortho.get_clip_from_view(), point).truncate();
                assert!(
                    (from_perspective - from_ortho).abs().max_element() < 0.0001,
                    "{width}x{height} zoom {zoom}: {from_perspective:?} != {from_ortho:?}"
                );
            }
        }
    }

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
