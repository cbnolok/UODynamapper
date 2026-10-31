//! CPU picking uses the same source routing, anchors and bounds as static collection.
//! These are visual bounds only; logical XYZ and movement metadata stay on the tile.
use super::statics_collect::{
    self, StaticArtSourceState, StaticSurfaceLikeResolutionCache, StaticVisualKind,
    StaticChunkRenderCache, StaticPickTile, RenderStaticInstances, RenderStaticLandInstances,
};
use super::static_alpha::{StaticAlphaCache, sprite_sample_position};
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
    chunks: Res<'w, StaticChunkRenderCache>,
    sprites: Res<'w, RenderStaticInstances>,
    ground: Res<'w, RenderStaticLandInstances>,
    alpha: ResMut<'w, StaticAlphaCache>,
    uniforms: Res<'w, crate::configs::shader_presets::UniformState>,
    time: Res<'w, Time>,
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

impl<'w> StaticPickingResources<'w> {
    pub(crate) fn is_changed(&self) -> bool {
        self.source.is_changed() || self.zoom.is_changed() || self.atlas.is_changed()
            || self.meta.as_ref().is_some_and(|value| value.is_changed())
            || self.cc.as_ref().is_some_and(|value| value.is_changed())
            || self.ec.as_ref().is_some_and(|value| value.is_changed())
            || self.land.as_ref().is_some_and(|value| value.is_changed())
            || self.sprites.is_changed() || self.ground.is_changed() || self.uniforms.is_changed()
            || self.uniforms.effects.enable_water_animation != 0
    }

    pub(crate) fn visible_tiles(&self, map_id: u32) -> impl Iterator<Item = StaticPickTile> + 'w {
        Res::clone(&self.chunks).into_inner().visible_pick_tiles(map_id)
    }

    pub(crate) fn prepare_alpha(&mut self) {
        self.alpha.sync_packages([self.cc.as_ref().map(|v| v.last_changed()),
            self.ec.as_ref().map(|v| v.last_changed())]);
    }

    pub(crate) fn alpha_visible(&mut self, geometry: StaticArtGeometry,
        mut hit: ProjectedQuadHit, is_wet: bool) -> bool {
        // Authored material surfaces compose several images and coverage masks.
        // Their base image's alpha alone is not their visible coverage.
        if geometry.flat_quad >= 2 { return true; }
        if is_wet && self.uniforms.effects.enable_water_animation != 0 {
            let t = self.time.elapsed_secs_wrapped();
            let scale = Vec2::new(1.3 + t.sin() * 0.28, 1.3 + t.cos() * 0.14);
            hit.uv = Vec2::splat(0.5) + (hit.uv - Vec2::splat(0.5)) / scale;
            hit.dx /= scale;
            hit.dy /= scale;
        }
        let alpha = match geometry.route {
            StaticArtRoute::CcArt(id) => {
                let Some(package) = self.cc.as_ref() else { return false; };
                let Some(slot) = package.0.present_slot(id) else { return false; };
                let Some(page) = package.0.pages().get(slot.page_index as usize) else { return false; };
                let sample = Vec2::new(f32::from(slot.x), f32::from(slot.y))
                    + sprite_sample_position(hit.uv, hit.dx, hit.dy, Vec2::new(f32::from(slot.width), f32::from(slot.height)));
                self.alpha.sample((0, slot.page_index), sample, ||
                    package.0.read_page_rgba(slot.page_index).ok().map(|rgba| (page.used_width, page.used_height, rgba)))
            }
            StaticArtRoute::EcArt(id) => {
                let Some(package) = self.ec.as_ref() else { return false; };
                let Some(slot) = package.0.present_slot(id) else { return false; };
                let Some(page) = package.0.pages().get(slot.page_index as usize) else { return false; };
                let sample = Vec2::new(f32::from(slot.x), f32::from(slot.y))
                    + sprite_sample_position(hit.uv, hit.dx, hit.dy, Vec2::new(f32::from(slot.width), f32::from(slot.height)));
                self.alpha.sample((1, slot.page_index), sample, ||
                    package.0.read_page_rgba(slot.page_index).ok().map(|rgba| (page.used_width, page.used_height, rgba)))
            }
            StaticArtRoute::EcLand(_) => return true,
        };
        // Both sprite passes discard below this coverage; a failed page is not
        // silently treated as an opaque rectangle.
        alpha.is_some_and(|value| value >= 0.0001)
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
        let (offset_x, offset_y, width, height, pixel_ratio) = match kind {
            StaticVisualKind::CcRegular { art_id, fallback_art_id } => {
                let package = &self.cc.as_ref()?.0;
                let selected = if package.present_slot(u32::from(art_id)).is_some() { art_id } else { fallback_art_id };
                let slot = package.present_slot(u32::from(selected))?;
                geometry.route = StaticArtRoute::CcArt(u32::from(selected));
                (slot.draw_offset_x, slot.draw_offset_y, slot.logical_width(), slot.logical_height(), 1.0)
            }
            StaticVisualKind::EcRegular { art_id } | StaticVisualKind::EcSurface { art_id } => {
                let slot = self.ec.as_ref()?.0.present_slot(art_id)?;
                let ratio = self.ec.as_ref()?.0.sprite_geometry(art_id)?.legacy_pixel_ratio();
                (slot.draw_offset_x, slot.draw_offset_y, slot.logical_width(), slot.logical_height(), ratio)
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
            let bounds = statics_collect::resolve_static_billboard_bounds(pixel_ratio,
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProjectedQuadHit {
    pub uv: Vec2,
    pub dx: Vec2,
    pub dy: Vec2,
}

pub(crate) fn projected_quad_hit(point: Vec2, camera: &Camera,
    transform: &GlobalTransform, corners: [Vec3; 4]) -> Option<ProjectedQuadHit> {
    let clip_from_world = camera.clip_from_view() * transform.to_matrix().inverse();
    let mut screen = [Vec2::ZERO; 4];
    let mut inverse_w = [0.0; 4];
    for index in 0..4 {
        screen[index] = camera.world_to_viewport(transform, corners[index]).ok()?;
        let clip = clip_from_world * corners[index].extend(1.0);
        if !clip.is_finite() || clip.w <= 0.0 { return None; }
        inverse_w[index] = 1.0 / clip.w;
    }
    projected_quad_uv(point, screen, inverse_w)
}

fn projected_quad_uv(point: Vec2, corners: [Vec2; 4], inverse_w: [f32; 4]) -> Option<ProjectedQuadHit> {
    // Match the renderer's two triangles, rather than their axis-aligned bounding box.
    let texcoords = [Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::ONE];
    for indices in [[0, 2, 1], [1, 2, 3]] {
        let [a, b, c] = indices.map(|index| corners[index]);
        let area = (b - a).perp_dot(c - a);
        if !area.is_finite() || area.abs() <= 0.000001 { continue; }
        let barycentric = |p: Vec2| {
            let u = (p - a).perp_dot(c - a) / area;
            let v = (b - a).perp_dot(p - a) / area;
            Vec3::new(1.0 - u - v, u, v)
        };
        let weights = barycentric(point);
        if weights.min_element() < 0.0 { continue; }
        let interpolate = |weights: Vec3| {
            let weighted = weights * Vec3::from_array(indices.map(|i| inverse_w[i]));
            let uv = indices.into_iter().enumerate().map(|(j, i)| texcoords[i] * weighted[j]).sum::<Vec2>();
            uv / weighted.element_sum()
        };
        let uv = interpolate(weights);
        let dx = interpolate(barycentric(point + Vec2::X)) - uv;
        let dy = interpolate(barycentric(point + Vec2::Y)) - uv;
        if uv.is_finite() && dx.is_finite() && dy.is_finite() { return Some(ProjectedQuadHit { uv, dx, dy }); }
    }
    None
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
                assert!(projected_quad_uv(point, corners, [1.0; 4]).is_some());
            }
            for point in [Vec2::new(0.8, 0.8), Vec2::new(-0.8, -0.8), Vec2::new(2.0, 0.0)] {
                assert!(projected_quad_uv(point, corners, [1.0; 4]).is_none());
            }
        }
        assert!(projected_quad_uv(Vec2::ZERO, [Vec2::ZERO; 4], [1.0; 4]).is_none());
    }

    #[test]
    fn sprite_hit_interpolates_uvs_in_the_drawn_triangle_with_perspective_divide() {
        let corners = [Vec2::ZERO, Vec2::new(2.0, 0.0), Vec2::new(0.0, 2.0), Vec2::splat(2.0)];
        let hit = projected_quad_uv(Vec2::ONE, corners, [1.0; 4]).unwrap();
        assert_eq!(hit.uv, Vec2::splat(0.5));
        assert_eq!(hit.dx, Vec2::new(0.5, 0.0));
        assert_eq!(hit.dy, Vec2::new(0.0, 0.5));
        // An equal screen-space split is not an equal UV split under perspective.
        let hit = projected_quad_uv(Vec2::ONE, corners, [1.0, 0.5, 1.0, 0.5]).unwrap();
        assert!((hit.uv - Vec2::new(1.0 / 3.0, 2.0 / 3.0)).length() < 0.000001);
        let lower = projected_quad_uv(Vec2::splat(1.5), corners, [1.0; 4]).unwrap();
        assert_eq!(lower.uv, Vec2::splat(0.75));
    }
}
