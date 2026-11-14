//! Highlight geometry uses the same source routing, anchors and bounds as static collection.
//! These are visual bounds only; logical XYZ and movement metadata stay on the tile.
use super::statics_collect::{
    self, StaticArtSourceState, StaticSurfaceLikeResolutionCache, StaticVisualKind,
    StaticChunkRenderCache, StaticPickTile, RenderStaticInstances, RenderStaticLandInstances,
};
use crate::configs::settings::ClientTextureSource;
use crate::core::render::scene::{camera::RenderZoom, world::land::tile_atlas::TileAtlas};
use crate::core::uo_files_loader::{TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes, TileMetaPackageRes, TexAuxEcPackageRes};
use crate::prelude::*;
use bevy::{ecs::system::SystemParam, prelude::*};

#[derive(SystemParam)]
pub(crate) struct StaticPickingResources<'w> {
    source: Res<'w, StaticArtSourceState>,
    zoom: Res<'w, RenderZoom>,
    atlas: Res<'w, TileAtlas>,
    cc: Option<Res<'w, TexArtCcPackageRes>>,
    ec: Option<Res<'w, TexArtEcPackageRes>>,
    aux: Option<Res<'w, TexAuxEcPackageRes>>,
    land: Option<Res<'w, TexLandEcPackageRes>>,
    meta: Option<Res<'w, TileMetaPackageRes>>,
    cache: ResMut<'w, StaticSurfaceLikeResolutionCache>,
    chunks: Res<'w, StaticChunkRenderCache>,
    sprites: Res<'w, RenderStaticInstances>,
    ground: Res<'w, RenderStaticLandInstances>,
    batches: Res<'w, statics_collect::RenderStaticChunkBatches>,
    gpu_pick: ResMut<'w, super::gpu_picking::VisibleStaticPick>,
    gpu_request: ResMut<'w, super::gpu_picking::PickRequest>,
    uniforms: Res<'w, crate::configs::shader_presets::UniformState>,
    time: Res<'w, Time>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum StaticArtRoute {
    CcArt(u32),
    EcArt(u32),
    EcLand(u32),
    MissingArt,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StaticArtGeometry {
    pub origin: Vec3,
    pub local_min: Vec2,
    pub local_max: Vec2,
    pub flat_quad: u32,
    pub route: StaticArtRoute,
}

impl<'w> StaticPickingResources<'w> {
    pub(crate) fn visible_hit(&mut self, cursor: Vec2, camera: &Camera,
        transform: &GlobalTransform, map: u32) -> Option<StaticPickTile> {
        let scale = camera.target_scaling_factor()?;
        let viewport = camera.physical_viewport_rect()?;
        let pixel = (cursor * scale).floor().as_uvec2();
        if !viewport.contains(pixel) || self.zoom.0 >= 20.0 {
            self.gpu_pick.bypass_change_detection().cancel(&mut self.gpu_request);
            return None;
        }
        let signature = super::gpu_picking::PickSignature {
            pixel, viewport: UVec4::new(viewport.min.x, viewport.min.y, viewport.width(), viewport.height()),
            clip_from_world: camera.clip_from_view() * transform.to_matrix().inverse(),
            revisions: [u64::from(self.sprites.last_changed().get()), u64::from(self.ground.last_changed().get()),
                self.meta.as_ref().map_or(0, |meta| u64::from(meta.last_changed().get())), self.atlas.content_revision,
                self.source.active_source.map_or(2, |source| match source { ClientTextureSource::Cc => 0, ClientTextureSource::Ec => 1 })],
            alpha_controls: UVec4::new(self.uniforms.effects.post_process_profile, self.uniforms.effects.terrain_transition_flags,
                self.uniforms.effects.enable_linear_filtering, self.uniforms.effects.enable_water_animation),
            animation: if self.uniforms.effects.enable_water_animation != 0 { self.time.elapsed_secs_wrapped().to_bits() } else { 0 },
            map,
        };
        let keys = self.batches.sprite.iter().chain(&self.batches.ground)
            .filter(|b| b.key.map_id == map).map(|b| b.key).collect();
        let chunks = &self.chunks;
        let batches = &self.batches;
        self.gpu_pick.bypass_change_detection().query(&mut self.gpu_request, signature, || chunks.gpu_pick_targets(batches), keys)
    }

    pub(crate) fn is_changed(&self) -> bool {
        self.source.is_changed() || self.zoom.is_changed() || self.atlas.is_changed()
            || self.meta.as_ref().is_some_and(|value| value.is_changed())
            || self.cc.as_ref().is_some_and(|value| value.is_changed())
            || self.ec.as_ref().is_some_and(|value| value.is_changed())
            || self.aux.as_ref().is_some_and(|value| value.is_changed())
            || self.land.as_ref().is_some_and(|value| value.is_changed())
            || self.sprites.is_changed() || self.ground.is_changed() || self.uniforms.is_changed()
            || self.gpu_pick.is_changed()
            || self.uniforms.effects.enable_water_animation != 0
    }

    /// The art mode statics are drawn with, if any.
    pub(crate) fn art_source(&self) -> Option<ClientTextureSource> {
        self.source.active_source
    }

    pub(crate) fn resolve(&mut self, graphic: u16, x: f32, y: f32, z: i8) -> Option<StaticArtGeometry> {
        // Dot mode uses a separate radar representation, not these textured quads.
        if self.zoom.0 >= 20.0 { return None; }
        let source = self.source.active_source?;
        let meta = self.meta.as_ref().and_then(|package| package.0.item_tile(u32::from(graphic)));
        if meta.is_some_and(|tile| tile.is_nodraw()) { return None; }
        let kind = statics_collect::resolve_static_visual_kind_cached(source, self.source.land_style, graphic,
            self.meta.as_ref().map(|package| package.0.as_ref()), meta,
            self.cc.as_ref().map(|package| package.0.as_ref()),
            self.ec.as_ref().map(|package| package.0.as_ref()),
            self.land.as_ref().map(|package| package.0.as_ref()), &mut self.cache);
        let renders_wet = statics_collect::static_visual_renders_wet(kind, meta,
            self.meta.as_ref().map(|package| package.0.as_ref()), u32::from(graphic));
        let depth_class = statics_collect::resolve_static_depth_class(meta, renders_wet);
        let height = scale_uo_z_to_bevy_units(f32::from(z)) + statics_collect::depth_class_y_bias(depth_class);
        let (anchor_x, anchor_y) = statics_collect::surface_like_static_world_anchor(kind, x, y);
        let mut geometry = StaticArtGeometry { origin: Vec3::new(anchor_x, height, anchor_y),
            local_min: Vec2::ZERO, local_max: Vec2::ONE, flat_quad: 2,
            route: StaticArtRoute::EcArt(u32::from(graphic)) };
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
                let source = self.ec.as_ref()?.0.sprite_geometry(art_id)?;
                (slot.draw_offset_x, slot.draw_offset_y, slot.logical_width(), slot.logical_height(),
                    source.legacy_pixel_ratio())
            }
            StaticVisualKind::TexLandEc { .. } => unreachable!(),
            StaticVisualKind::Missing => {
                let slot = self.aux.as_ref()?.0.missing_texture_slot();
                geometry.route = StaticArtRoute::MissingArt;
                (-(slot.width as i32 / 2) as i16, 0, f32::from(slot.width), f32::from(slot.height), 1.0)
            }
        };
        if matches!(kind, StaticVisualKind::EcSurface { .. }) {
            geometry.flat_quad = if statics_collect::ec_static_camera_quad(meta) { 3 } else { 2 };
        } else if matches!(kind, StaticVisualKind::CcRegular { .. })
            && statics_collect::cc_static_uses_flat_geometry(meta, width, height, x, y, z, &self.atlas) {
            let bounds = statics_collect::resolve_cc_surface_like_flat_quad_bounds(width, height);
            let (dx, dy) = statics_collect::cc_surface_like_flat_quad_center_offsets(height);
            geometry.origin.x = x + dx;
            geometry.origin.z = y + dy;
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
                local_max: Vec2::ONE, flat_quad: 2, route: StaticArtRoute::EcArt(1304) };
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
            route: StaticArtRoute::EcArt(1304) };
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

}
