//! CPU picking uses the same source routing, anchors and bounds as static collection.
//! These are visual bounds only; logical XYZ and movement metadata stay on the tile.
use super::statics_collect::{
    self, StaticArtSourceState, StaticSurfaceLikeResolutionCache, StaticVisualKind,
};
use crate::configs::settings::ClientTextureSource;
use crate::core::render::scene::{camera::RenderZoom, world::land::tile_atlas::TileAtlas};
use crate::core::uo_files_loader::{TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes, TileMetaPackageRes};
use crate::prelude::*;
use bevy::{ecs::system::SystemParam, prelude::*};

#[derive(SystemParam)]
pub(crate) struct StaticPickingResources<'w> {
    source: Res<'w, StaticArtSourceState>,
    zoom: Res<'w, RenderZoom>,
    atlas: Res<'w, TileAtlas>,
    cc: Option<Res<'w, TexArtCcPackageRes>>,
    ec: Option<Res<'w, TexArtEcPackageRes>>,
    land: Option<Res<'w, TexLandEcPackageRes>>,
    meta: Option<Res<'w, TileMetaPackageRes>>,
    cache: ResMut<'w, StaticSurfaceLikeResolutionCache>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum StaticArtRoute {
    CcArt(u32),
    EcArt(u32),
    EcLand(u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StaticArtGeometry {
    pub origin: Vec3,
    pub local_min: Vec2,
    pub local_max: Vec2,
    pub flat_quad: u32,
    pub priority_anchor: Vec2,
    pub route: StaticArtRoute,
}

impl StaticPickingResources<'_> {
    pub(crate) fn is_changed(&self) -> bool {
        self.source.is_changed() || self.zoom.is_changed() || self.atlas.is_changed()
            || self.meta.as_ref().is_some_and(|value| value.is_changed())
            || self.cc.as_ref().is_some_and(|value| value.is_changed())
            || self.ec.as_ref().is_some_and(|value| value.is_changed())
            || self.land.as_ref().is_some_and(|value| value.is_changed())
    }

    pub(crate) fn resolve(&mut self, graphic: u16, x: f32, y: f32, z: i8) -> Option<StaticArtGeometry> {
        // Dot mode uses a separate radar representation, not these textured quads.
        if self.zoom.0 >= 20.0 { return None; }
        let source = self.source.active_source?;
        let meta = self.meta.as_ref().and_then(|package| package.0.item_tile(u32::from(graphic)));
        if meta.is_some_and(|tile| tile.is_nodraw()) { return None; }
        let kind = statics_collect::resolve_static_visual_kind_cached(source, graphic,
            self.meta.as_ref().map(|package| package.0.as_ref()), meta,
            self.ec.as_ref().map(|package| package.0.as_ref()),
            self.land.as_ref().map(|package| package.0.as_ref()), &mut self.cache);
        let depth_class = statics_collect::resolve_static_depth_class(meta);
        let height = scale_uo_z_to_bevy_units(f32::from(z)) + statics_collect::depth_class_y_bias(depth_class);
        let (anchor_x, anchor_y) = statics_collect::surface_like_static_world_anchor(kind, x, y);
        let mut geometry = StaticArtGeometry { origin: Vec3::new(anchor_x, height, anchor_y),
            local_min: Vec2::ZERO, local_max: Vec2::ONE, flat_quad: 2,
            priority_anchor: Vec2::new(anchor_x, anchor_y), route: StaticArtRoute::EcArt(u32::from(graphic)) };
        // Ground material slots cover the owning cell, without a billboard translation.
        if let StaticVisualKind::TexLandEc { resolution } = kind {
            self.land.as_ref()?.0.present_slot(resolution.runtime_slot_id)?;
            geometry.route = StaticArtRoute::EcLand(resolution.runtime_slot_id);
            return Some(geometry);
        }
        let (offset_x, offset_y, width, height) = match kind {
            StaticVisualKind::CcRegular { art_id, fallback_art_id } => {
                let package = &self.cc.as_ref()?.0;
                let selected = if package.present_slot(u32::from(art_id)).is_some() { art_id } else { fallback_art_id };
                let slot = package.present_slot(u32::from(selected))?;
                geometry.route = StaticArtRoute::CcArt(u32::from(selected));
                (slot.draw_offset_x, slot.draw_offset_y, slot.logical_width(), slot.logical_height())
            }
            StaticVisualKind::EcRegular { art_id } | StaticVisualKind::EcSurface { art_id } => {
                let slot = self.ec.as_ref()?.0.present_slot(art_id)?;
                (slot.draw_offset_x, slot.draw_offset_y, slot.logical_width(), slot.logical_height())
            }
            StaticVisualKind::TexLandEc { .. } => unreachable!(),
        };
        if matches!(kind, StaticVisualKind::EcSurface { .. }) {
            geometry.flat_quad = if statics_collect::ec_static_camera_quad(meta) { 3 } else { 2 };
        } else if source == ClientTextureSource::Cc
            && statics_collect::cc_static_uses_flat_geometry(meta, width, height, x, y, z, &self.atlas) {
            let bounds = statics_collect::resolve_cc_surface_like_flat_quad_bounds(width, height);
            let (dx, dy) = statics_collect::cc_surface_like_flat_quad_center_offsets(height);
            geometry.origin.x = x + dx;
            geometry.origin.z = y + dy;
            geometry.priority_anchor = Vec2::new(x, y);
            geometry.local_min = Vec2::new(bounds.local_min_x, bounds.local_min_z);
            geometry.local_max = Vec2::new(bounds.local_max_x, bounds.local_max_z);
            geometry.flat_quad = 1;
        } else {
            let bounds = statics_collect::resolve_static_billboard_bounds(source,
                offset_x, offset_y, width, height);
            geometry.local_min = Vec2::new(bounds.local_min_x, bounds.local_min_y);
            geometry.local_max = Vec2::new(bounds.local_max_x, bounds.local_max_y);
            geometry.flat_quad = 0;
        }
        Some(geometry)
    }
}

impl StaticArtGeometry {
    /// Mirrors the non-shadow vertex paths in art/main.wesl and ec_item_quad.wesl.
    pub(crate) fn corners(self, camera: &Camera, transform: &GlobalTransform) -> [Vec3; 4] {
        let clip = camera.clip_from_view();
        let camera_quad = self.flat_quad == 3 && clip.w_axis.w == 1.0
            && transform.back().as_vec3().dot(Vec3::splat(0.577350269)) >= 0.999999;
        let pixel_aspect = camera.logical_viewport_size().map(|size|
            (clip.x_axis.x * size.x) / (clip.y_axis.y * size.y));
        [Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::ONE].map(|corner| {
            if camera_quad {
                if let Some(aspect) = pixel_aspect {
                    let screen = Vec2::new(0.707105 + (corner.x - corner.y) * 0.7671049,
                        1.4742099 - (corner.x + corner.y) * 0.7671049);
                    return self.origin + Vec3::new(0.5, 0.0, 1.5)
                        + transform.right().as_vec3() * screen.x
                        + transform.up().as_vec3() * (screen.y * aspect);
                }
            }
            let mut uv = corner;
            if self.flat_quad == 0 { uv.y = 1.0 - uv.y; }
            let local = self.local_min + (self.local_max - self.local_min) * uv;
            let d = std::f32::consts::FRAC_1_SQRT_2;
            self.origin + match self.flat_quad {
                0 => Vec3::new(local.x * d, local.y, -local.x * d),
                1 => Vec3::new((local.x + local.y) * d, 0.0, (-local.x + local.y) * d),
                _ => Vec3::new(local.x, 0.0, local.y),
            }
        })
    }
}

pub(crate) fn projected_quad_contains(point: Vec2, corners: [Vec2; 4]) -> bool {
    // Match the renderer's two triangles, rather than their axis-aligned bounding box.
    [[0, 2, 1], [1, 2, 3]].into_iter().any(|indices| {
        let [a, b, c] = indices.map(|index| corners[index]);
        let area = (b - a).perp_dot(c - a);
        if !area.is_finite() || area.abs() <= 0.000001 { return false; }
        let u = (b - a).perp_dot(point - a) / area;
        let v = (point - a).perp_dot(c - a) / area;
        u >= 0.0 && v >= 0.0 && u + v <= 1.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_camera(perspective: bool) -> Camera {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let mut projection = if perspective {
            Projection::Perspective(PerspectiveProjection::default())
        } else {
            Projection::Orthographic(OrthographicProjection::default_3d())
        };
        projection.update(1280.0, 800.0);
        Camera { computed: ComputedCameraValues {
            clip_from_view: projection.get_clip_from_view(),
            target_info: Some(RenderTargetInfo { physical_size: UVec2::new(1280, 800), scale_factor: 1.0 }),
            ..default()
        }, ..default() }
    }

    #[test]
    fn wet_and_ground_hover_quads_stay_horizontal_in_both_camera_modes() {
        let origin = Vec3::new(12.5, -0.7, 20.5);
        for perspective in [false, true] {
            let camera = test_camera(perspective);
            let transform = GlobalTransform::from(Transform::from_xyz(10.0, 10.0, 10.0)
                .looking_at(Vec3::ZERO, Vec3::Y));
            let ground = StaticArtGeometry { origin, local_min: Vec2::ZERO,
                local_max: Vec2::ONE, flat_quad: 2, priority_anchor: Vec2::new(12.0, 20.0), route: StaticArtRoute::EcArt(1304) };
            assert_eq!(ground.corners(&camera, &transform), [origin, origin + Vec3::X,
                origin + Vec3::Z, origin + Vec3::X + Vec3::Z]);
            // The 44px baked CC diamond uses diagonal, centered bounds. Its
            // four edge midpoints must land on the cell's corners, not stand up.
            let wet = StaticArtGeometry { local_min: Vec2::splat(-std::f32::consts::FRAC_1_SQRT_2),
                local_max: Vec2::splat(std::f32::consts::FRAC_1_SQRT_2), flat_quad: 1, ..ground };
            let corners = wet.corners(&camera, &transform);
            for (pair, delta) in [([0, 1], Vec3::new(-0.5, 0.0, -0.5)),
                ([1, 3], Vec3::new(0.5, 0.0, -0.5)),
                ([3, 2], Vec3::new(0.5, 0.0, 0.5)),
                ([2, 0], Vec3::new(-0.5, 0.0, 0.5))] {
                let midpoint = (corners[pair[0]] + corners[pair[1]]) * 0.5;
                assert!((midpoint - origin - delta).length() < 0.00001);
            }
        }
    }

    #[test]
    fn authored_ec_hover_uses_camera_quad_only_in_the_renderers_supported_view() {
        let iso = GlobalTransform::from(Transform::from_xyz(10.0, 10.0, 10.0)
            .looking_at(Vec3::ZERO, Vec3::Y));
        let rotated = GlobalTransform::from(Transform::from_xyz(5.0, 10.0, 14.0)
            .looking_at(Vec3::ZERO, Vec3::Y));
        let geometry = StaticArtGeometry { origin: Vec3::new(2.0, -0.5, 3.0),
            local_min: Vec2::ZERO, local_max: Vec2::ONE, flat_quad: 3,
            priority_anchor: Vec2::new(2.0, 3.0), route: StaticArtRoute::EcArt(1304) };
        for perspective in [false, true] {
            let camera = test_camera(perspective);
            for transform in [&iso, &rotated] {
                let corners = geometry.corners(&camera, transform);
                if !perspective && std::ptr::eq(transform, &iso) {
                    assert!((corners[0].y - corners[3].y).abs() > 0.1);
                    // All authored vertices occupy one camera plane.
                    let normal = transform.back().as_vec3();
                    for corner in corners { assert!((corner - corners[0]).dot(normal).abs() < 0.00001); }
                } else {
                    for corner in corners { assert_eq!(corner.y, geometry.origin.y); }
                }
            }
        }
    }

    #[test]
    fn projected_quad_rejects_empty_bounding_box_corners_in_both_windings() {
        let diamond = [Vec2::new(0.0, -1.0), Vec2::new(1.0, 0.0),
            Vec2::new(-1.0, 0.0), Vec2::new(0.0, 1.0)];
        for corners in [diamond, diamond.map(|p| Vec2::new(-p.x, p.y))] {
            for point in [Vec2::ZERO, Vec2::new(0.2, 0.6), Vec2::new(0.0, -1.0)] {
                assert!(projected_quad_contains(point, corners));
            }
            for point in [Vec2::new(0.8, 0.8), Vec2::new(-0.8, -0.8), Vec2::new(2.0, 0.0)] {
                assert!(!projected_quad_contains(point, corners));
            }
        }
        assert!(!projected_quad_contains(Vec2::ZERO, [Vec2::ZERO; 4]));
    }
}
