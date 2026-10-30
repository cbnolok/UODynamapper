//! A small software rasterizer for previews: draws a [`RenderFrame`] into a float RGBA canvas with an
//! orthographic camera and the client's D3D9 state: per-axis wrap or clamp addressing, each map's filter mode
//! over the texture's mip levels ([`TextureFilter`], [`crate::mips`]), the depth test and write of each draw
//! (`NiDX9RenderState::ApplyZBuffer`), alpha test and the blend factors (D3D blend math).
//!
//! It is a preview tool, not the client's renderer: draws follow the scene order (sprites the frame order),
//! meshes are unlit by direction (see [`SceneLighting`]), and Chaos MESH particles (their `.nif` scenes are not
//! drawn) are square outlines of their size.

use std::sync::Arc;

use crate::math::Vec3;
use crate::nif::desc::{
    CompareFunction, CullMode, MipFilter, TexelFilter, TextureAddressing, TextureApplyMode, TextureFilter, TextureSlot,
};
use crate::render::{
    AlphaTest, BlendFactor, BlendState, CameraFrame, DepthState, QUAD_INDICES, QuadVertex, RenderFrame,
    SceneLighting, TRIANGLE_VERTICES, combine_textures, expand_mesh_draw, expand_mesh_stage_uvs, expand_sprite,
};

/// Texel centres are at half-texel offsets (bilinear filtering).
const TEXEL_CENTER_OFFSET: f32 = 0.5;
/// Pixel centres are at half-pixel offsets.
const PIXEL_CENTER_OFFSET: f32 = 0.5;
/// Texture used when a texture is missing or undecodable: opaque white.
pub const WHITE_TEXEL: [f32; 4] = [1.0; 4];
/// Index of the alpha channel.
const ALPHA: usize = 3;
const U8_MAX: f32 = 255.0;
/// Smallest half side of a mesh marker, pixels.
const MESH_MARKER_MIN_HALF_PX: f32 = 2.0;
/// Ordinary alpha blending, for markers and overlays.
pub const OVERLAY_BLEND: BlendState = BlendState { src: BlendFactor::SrcAlpha, dst: BlendFactor::InvSrcAlpha };
/// Blending off: the source replaces the destination (opaque mesh draws).
pub const OPAQUE_BLEND: BlendState = BlendState { src: BlendFactor::One, dst: BlendFactor::Zero };
/// Sprite textures of batches without a [`crate::render::BatchState`] (Chaos scripts) repeat: the address mode of
/// the Chaos base map was not traced; Gamebryo's default is `WRAP_S_WRAP_T` (probable, `uosa_chaos.md`, "Render state of
/// quads").
const SPRITE_ADDRESSING: TextureAddressing = TextureAddressing { wrap_s: true, wrap_t: true };
/// Sprite textures of batches without a [`crate::render::BatchState`] (Chaos scripts) filter as the
/// `NiTexturingProperty::Map` default, bilinear (probable: the filtering of the Chaos quads was not traced,
/// `uosa_chaos.md`, "Open questions").
const SPRITE_FILTER: TextureFilter = TextureFilter::BILERP;
/// Depth comparison of batches without a [`crate::render::BatchState`] (Chaos scripts): `NiZBufferProperty` 0xd,
/// LESSEQUAL (`uosa_chaos.md`, "Render state of quads").
const SPRITE_DEPTH_FUNCTION: CompareFunction = CompareFunction::LessEqual;
/// The cleared depth buffer: farther than anything (D3D clears to the far plane).
const DEPTH_CLEAR: f32 = f32::INFINITY;

/// Addressing on both axes: wrap (`D3DTADDRESS_WRAP`).
const WRAP_ADDRESSING: TextureAddressing = TextureAddressing { wrap_s: true, wrap_t: true };
/// Addressing on both axes: clamp (`D3DTADDRESS_CLAMP`).
const CLAMP_ADDRESSING: TextureAddressing = TextureAddressing { wrap_s: false, wrap_t: false };

/// One mip level below level 0 of a [`RasterTexture`], RGBA in [0, 1].
#[derive(Debug, Clone, PartialEq)]
pub struct RasterLevel {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[f32; 4]>,
}

/// A decoded texture, RGBA in [0, 1]: level 0, and the levels below it when the texture has mip levels.
#[derive(Debug, Clone, PartialEq)]
pub struct RasterTexture {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[f32; 4]>,
    /// Levels 1 and below, each half the size of the one above (at least 1); empty for one level. The client
    /// samples a single-level texture with the mip filter off.
    pub mips: Vec<RasterLevel>,
}

/// 8-bit RGBA to texels; `None` when the length does not match.
fn texels_of_rgba8(width: usize, height: usize, rgba: &[u8]) -> Option<Vec<[f32; 4]>> {
    if rgba.len() != width * height * crate::mips::RGBA8_BYTES_PER_PIXEL {
        return None;
    }
    let texels = rgba
        .chunks_exact(crate::mips::RGBA8_BYTES_PER_PIXEL)
        .map(|pixel| std::array::from_fn(|k| f32::from(pixel[k]) / U8_MAX))
        .collect();
    Some(texels)
}

/// The texel index of a texel coordinate on an axis of `size` texels: wrapped, or clamped to the edge texel
/// (`D3DTADDRESS_CLAMP` repeats the edge colour outside [0, 1]).
fn address(index: f32, size: usize, wrap: bool) -> usize {
    let (index, size) = (index as i64, size as i64);
    (if wrap { index.rem_euclid(size) } else { index.clamp(0, size - 1) }) as usize
}

/// One level: texel lookup with point or bilinear filtering and per-axis addressing.
fn sample_level(
    width: usize,
    height: usize,
    texels: &[[f32; 4]],
    u: f32,
    v: f32,
    addressing: TextureAddressing,
    filter: TexelFilter,
) -> [f32; 4] {
    let texel = |x: f32, y: f32| {
        texels[address(y, height, addressing.wrap_t) * width + address(x, width, addressing.wrap_s)]
    };
    let (x, y) = (u * width as f32, v * height as f32);
    if filter == TexelFilter::Point {
        return texel(x.floor(), y.floor());
    }
    let (x, y) = (x - TEXEL_CENTER_OFFSET, y - TEXEL_CENTER_OFFSET);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (a, b, c, d) = (texel(x0, y0), texel(x0 + 1.0, y0), texel(x0, y0 + 1.0), texel(x0 + 1.0, y0 + 1.0));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (d[k] - c[k]) * fx;
        top + (bottom - top) * fy
    })
}

impl RasterTexture {
    /// A single-level texture of texels; `None` when the sizes do not match or are zero.
    pub fn from_texels(width: usize, height: usize, texels: Vec<[f32; 4]>) -> Option<Self> {
        if width == 0 || height == 0 || texels.len() != width * height {
            return None;
        }
        Some(Self { width, height, texels, mips: Vec::new() })
    }

    /// A single-level texture from 8-bit RGBA rows; `None` when the sizes do not match or are zero.
    pub fn from_rgba8(width: usize, height: usize, rgba: &[u8]) -> Option<Self> {
        Self::from_rgba8_levels(width, height, std::slice::from_ref(&rgba))
    }

    /// A texture from 8-bit RGBA levels, level 0 first, each level half the size of the one above (at least
    /// 1); `None` when level 0 does not match its size or the size is zero. Levels after the first one whose
    /// size does not match are dropped. Use [`crate::mips::client_mip_levels`] for the levels the client has.
    pub fn from_rgba8_levels(width: usize, height: usize, levels: &[impl AsRef<[u8]>]) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let mut texture = Self::from_texels(width, height, texels_of_rgba8(width, height, levels.first()?.as_ref())?)?;
        let (mut level_width, mut level_height) = (width, height);
        for rgba in &levels[1..] {
            (level_width, level_height) = ((level_width / 2).max(1), (level_height / 2).max(1));
            let Some(texels) = texels_of_rgba8(level_width, level_height, rgba.as_ref()) else { break };
            texture.mips.push(RasterLevel { width: level_width, height: level_height, texels });
        }
        Some(texture)
    }

    /// Mip levels, level 0 included.
    pub fn level_count(&self) -> usize {
        1 + self.mips.len()
    }

    /// Bilinear sample of level 0 with wrap addressing.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        self.sample_addressed(u, v, WRAP_ADDRESSING)
    }

    /// Bilinear sample of level 0 with clamp addressing: texels outside the texture repeat its edge.
    pub fn sample_clamped(&self, u: f32, v: f32) -> [f32; 4] {
        self.sample_addressed(u, v, CLAMP_ADDRESSING)
    }

    /// Bilinear sample of level 0, each axis wrapped or clamped.
    pub fn sample_addressed(&self, u: f32, v: f32, addressing: TextureAddressing) -> [f32; 4] {
        self.sample_level(0, u, v, addressing, TexelFilter::Linear)
    }

    /// One level (clamped to the last), point or bilinear.
    fn sample_level(&self, level: usize, u: f32, v: f32, addressing: TextureAddressing, filter: TexelFilter) -> [f32; 4] {
        match level.checked_sub(1).and_then(|index| self.mips.get(index).or(self.mips.last())) {
            Some(mip) => sample_level(mip.width, mip.height, &mip.texels, u, v, addressing, filter),
            None => sample_level(self.width, self.height, &self.texels, u, v, addressing, filter),
        }
    }

    /// The D3D9 sampler: `lod` is the level of detail, log2 of the texels of level 0 one pixel step covers
    /// ([`texture_lod`]). At or below 0 the texture is magnified (the mag filter on level 0); above it the min
    /// filter samples level 0 (mip filter off, or a single level), the nearest level (`floor(lod + 0.5)`), or a
    /// blend of the two levels around `lod`.
    pub fn sample_filtered(
        &self,
        u: f32,
        v: f32,
        lod: f32,
        addressing: TextureAddressing,
        filter: TextureFilter,
    ) -> [f32; 4] {
        if !(lod > 0.0) {
            return self.sample_level(0, u, v, addressing, filter.mag);
        }
        let mip = if self.mips.is_empty() { MipFilter::None } else { filter.mip };
        match mip {
            MipFilter::None => self.sample_level(0, u, v, addressing, filter.min),
            MipFilter::Point => self.sample_level((lod + 0.5).floor() as usize, u, v, addressing, filter.min),
            MipFilter::Linear => {
                let fine = lod.floor();
                let blend = lod - fine;
                let fine = fine as usize;
                let a = self.sample_level(fine, u, v, addressing, filter.min);
                let b = self.sample_level(fine + 1, u, v, addressing, filter.min);
                std::array::from_fn(|k| a[k] + (b[k] - a[k]) * blend)
            }
        }
    }
}

/// The level of detail of a texture of `width` x `height` texels mapped with texture coordinates whose screen
/// derivatives are `du` = (du/dx, du/dy) and `dv` = (dv/dx, dv/dy): `log2` of the longer of the two pixel steps
/// in texels (the D3D9 reference rasterizer's scale factor).
pub fn texture_lod(width: usize, height: usize, du: [f32; 2], dv: [f32; 2]) -> f32 {
    let (w, h) = (width as f32, height as f32);
    let step_x = (du[0] * w).hypot(dv[0] * h);
    let step_y = (du[1] * w).hypot(dv[1] * h);
    step_x.max(step_y).log2()
}

/// Yaw of the client view ([`CameraFrame::client_view`]) for [`orbit_axes`].
pub const CLIENT_VIEW_YAW: f32 = 0.0;
/// Elevation of the client view ([`CameraFrame::client_view`]) for [`orbit_axes`]: effects are camera-aligned
/// billboards, so the client sees an effect's local frame straight along +y.
pub const CLIENT_VIEW_ELEVATION: f32 = 0.0;

/// The camera axes of a view along +y, `elevation` radians above the x-y plane: (direction, up, right).
/// Elevation [`CLIENT_VIEW_ELEVATION`] is the client view.
pub fn camera_axes(elevation: f32) -> (Vec3, Vec3, Vec3) {
    orbit_axes(CLIENT_VIEW_YAW, elevation)
}

/// The camera axes of a view `elevation` radians above the x-y plane, turned `yaw` radians from +y toward +x
/// about +z: (direction, up, right). In an effect's local frame, yaw [`CLIENT_VIEW_YAW`] and elevation
/// [`CLIENT_VIEW_ELEVATION`] are the client view (direction +y, up +z, right +x); other angles orbit it.
pub fn orbit_axes(yaw: f32, elevation: f32) -> (Vec3, Vec3, Vec3) {
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_elevation, cos_elevation) = elevation.sin_cos();
    let direction = Vec3::new(sin_yaw * cos_elevation, cos_yaw * cos_elevation, -sin_elevation);
    let up = Vec3::new(sin_yaw * sin_elevation, cos_yaw * sin_elevation, cos_elevation);
    (direction, up, direction.cross(up))
}

/// An orthographic view: camera axes and the mapping of world space onto a canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrthoView {
    /// The camera; its position only matters for axial sprites (their view vector).
    pub camera: CameraFrame,
    /// World point drawn at the canvas centre.
    pub center: Vec3,
    pub pixels_per_unit: f32,
    pub width: usize,
    pub height: usize,
}

impl OrthoView {
    /// The client view ([`CameraFrame::client_view`]) of `center`, the camera `distance` away.
    pub fn client_view(center: Vec3, distance: f32, pixels_per_unit: f32, width: usize, height: usize) -> Self {
        Self::orbit(CLIENT_VIEW_YAW, CLIENT_VIEW_ELEVATION, center, distance, pixels_per_unit, width, height)
    }

    /// A view looking at `center` from `elevation` radians above the horizon, the camera `distance` away.
    pub fn new(elevation: f32, center: Vec3, distance: f32, pixels_per_unit: f32, width: usize, height: usize) -> Self {
        Self::orbit(0.0, elevation, center, distance, pixels_per_unit, width, height)
    }

    /// [`Self::new`] with the camera turned `yaw` radians from +y toward +x ([`orbit_axes`]).
    pub fn orbit(
        yaw: f32,
        elevation: f32,
        center: Vec3,
        distance: f32,
        pixels_per_unit: f32,
        width: usize,
        height: usize,
    ) -> Self {
        let (direction, up, right) = orbit_axes(yaw, elevation);
        let camera = CameraFrame { position: center - direction * distance, direction, up, right };
        Self { camera, center, pixels_per_unit, width, height }
    }

    /// Canvas pixel coordinates of a world point.
    pub fn to_screen(&self, position: Vec3) -> (f32, f32) {
        let offset = position - self.center;
        (
            self.width as f32 * 0.5 + offset.dot(self.camera.right) * self.pixels_per_unit,
            self.height as f32 * 0.5 - offset.dot(self.camera.up) * self.pixels_per_unit,
        )
    }

    /// The view plane coordinates (right, up) of a world point, in world units.
    pub fn project(&self, position: Vec3) -> (f32, f32) {
        (position.dot(self.camera.right), position.dot(self.camera.up))
    }

    /// Depth of a world point along the view direction, relative to [`Self::center`].
    pub fn depth(&self, position: Vec3) -> f32 {
        (position - self.center).dot(self.camera.direction)
    }

    /// The world point on the view plane through [`Self::center`] under canvas pixel coordinates.
    pub fn to_world(&self, (x, y): (f32, f32)) -> Vec3 {
        self.center + self.camera.right * ((x - self.width as f32 * 0.5) / self.pixels_per_unit)
            - self.camera.up * ((y - self.height as f32 * 0.5) / self.pixels_per_unit)
    }
}

/// The extent of what a frame draws on the view plane: (min right, max right, min up, max up), padded by the
/// sprite sizes. `None` for an empty frame.
pub fn frame_extent(frame: &RenderFrame, camera: &CameraFrame) -> Option<[f32; 4]> {
    let project = |point: Vec3| (point.dot(camera.right), point.dot(camera.up));
    let mut extent: Option<[f32; 4]> = None;
    each_drawn_point(frame, |point, pad| {
        let (x, y) = project(point);
        let [min_x, max_x, min_y, max_y] = extent.get_or_insert([x, x, y, y]);
        *min_x = min_x.min(x - pad);
        *max_x = max_x.max(x + pad);
        *min_y = min_y.min(y - pad);
        *max_y = max_y.max(y + pad);
    });
    extent
}

/// The world-space box around everything a frame draws, whatever the camera: (min, max), sprites padded by
/// their size. `None` for an empty frame.
pub fn frame_bounds(frame: &RenderFrame) -> Option<(Vec3, Vec3)> {
    let mut bounds: Option<(Vec3, Vec3)> = None;
    each_drawn_point(frame, |point, pad| {
        let pad = Vec3::new(pad, pad, pad);
        let (min, max) = bounds.get_or_insert((point, point));
        let (low, high) = (point - pad, point + pad);
        *min = Vec3::new(min.x.min(low.x), min.y.min(low.y), min.z.min(low.z));
        *max = Vec3::new(max.x.max(high.x), max.y.max(high.y), max.z.max(high.z));
    });
    bounds
}

/// Every point a frame draws with the padding around it: sprite centres (half size plus axial length), Chaos
/// mesh particle centres (half scale) and mesh draw vertices (no padding).
fn each_drawn_point(frame: &RenderFrame, mut include: impl FnMut(Vec3, f32)) {
    for sprite in &frame.sprites {
        include(sprite.position, sprite.size.abs() * 0.5 + sprite.length.abs());
    }
    for mesh in &frame.meshes {
        include(mesh.translation, mesh.scale.abs() * 0.5);
    }
    for draw in &frame.mesh_draws {
        for &local in draw.local_positions() {
            include(draw.translation + draw.rotation.mul_vec3(Vec3::from_array(local)) * draw.scale, 0.0);
        }
    }
}

/// A float RGBA canvas with D3D-style blending and a depth buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[f32; 4]>,
    /// Per pixel, the depth ([`OrthoView::depth`]) of the nearest depth write; [`DEPTH_CLEAR`] where none.
    depth: Vec<f32>,
}

impl Canvas {
    pub fn new(width: usize, height: usize, background: [f32; 4]) -> Self {
        Self { width, height, pixels: vec![background; width * height], depth: vec![DEPTH_CLEAR; width * height] }
    }

    /// `out = src * src_factor + dst * dst_factor`, clamped to [0, 1].
    pub fn blend(&mut self, x: usize, y: usize, source: [f32; 4], blend: BlendState) {
        let destination = &mut self.pixels[y * self.width + x];
        let source_factor = blend_factor(blend.src, source, *destination);
        let destination_factor = blend_factor(blend.dst, source, *destination);
        *destination = std::array::from_fn(|k| {
            (source[k] * source_factor[k] + destination[k] * destination_factor[k]).clamp(0.0, 1.0)
        });
    }

    /// 8-bit RGBA rows.
    pub fn to_rgba8(&self) -> Vec<u8> {
        self.pixels.iter().flat_map(|pixel| pixel.map(|channel| (channel * U8_MAX).round() as u8)).collect()
    }

    /// A filled square centred on (x, y).
    pub fn fill_square(&mut self, x: f32, y: f32, half: f32, color: [f32; 4], blend: BlendState) {
        let (x0, x1) = ((x - half).max(0.0) as usize, (x + half).min(self.width as f32).max(0.0) as usize);
        let (y0, y1) = ((y - half).max(0.0) as usize, (y + half).min(self.height as f32).max(0.0) as usize);
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend(px, py, color, blend);
            }
        }
    }

    /// A one-pixel rectangle outline between two corners (clipped to the canvas).
    pub fn outline_rect(&mut self, (x0, y0): (f32, f32), (x1, y1): (f32, f32), color: [f32; 4], blend: BlendState) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let clamp_x = |value: f32| (value.round() as i64).clamp(0, self.width as i64 - 1) as usize;
        let clamp_y = |value: f32| (value.round() as i64).clamp(0, self.height as i64 - 1) as usize;
        let (left, right) = (clamp_x(x0.min(x1)), clamp_x(x0.max(x1)));
        let (top, bottom) = (clamp_y(y0.min(y1)), clamp_y(y0.max(y1)));
        for px in left..=right {
            self.blend(px, top, color, blend);
            if bottom != top {
                self.blend(px, bottom, color, blend);
            }
        }
        for py in top + 1..bottom {
            self.blend(left, py, color, blend);
            if right != left {
                self.blend(right, py, color, blend);
            }
        }
    }

    /// Rasterizes one textured triangle (pixel centres, both windings), modulating the texture by the vertex
    /// colour.
    pub fn fill_triangle(
        &mut self,
        view: &OrthoView,
        corners: &[QuadVertex; 3],
        texture: Option<&RasterTexture>,
        blend: BlendState,
    ) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let screen = corners.map(|vertex| view.to_screen(vertex.position));
        let [(ax, ay), (bx, by), (cx, cy)] = screen;
        let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
        if area.abs() < f32::EPSILON || !area.is_finite() {
            return;
        }
        let (limit_x, limit_y) = (self.width as f32 - 1.0, self.height as f32 - 1.0);
        let min_x = ax.min(bx).min(cx).floor().clamp(0.0, limit_x) as usize;
        let max_x = ax.max(bx).max(cx).ceil().clamp(0.0, limit_x) as usize;
        let min_y = ay.min(by).min(cy).floor().clamp(0.0, limit_y) as usize;
        let max_y = ay.max(by).max(cy).ceil().clamp(0.0, limit_y) as usize;
        let color = corners[0].color;
        for py in min_y..=max_y {
            for px in min_x..=max_x {
                let (x, y) = (px as f32 + PIXEL_CENTER_OFFSET, py as f32 + PIXEL_CENTER_OFFSET);
                let wa = ((bx - x) * (cy - y) - (by - y) * (cx - x)) / area;
                let wb = ((cx - x) * (ay - y) - (cy - y) * (ax - x)) / area;
                let wc = 1.0 - wa - wb;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let u = wa * corners[0].uv[0] + wb * corners[1].uv[0] + wc * corners[2].uv[0];
                let v = wa * corners[0].uv[1] + wb * corners[1].uv[1] + wc * corners[2].uv[1];
                let texel = texture.map_or(WHITE_TEXEL, |texture| texture.sample(u, v));
                let source = std::array::from_fn(|k| texel[k] * color[k]);
                self.blend(px, py, source, blend);
            }
        }
    }

    /// Draws every sprite batch of `frame` (textures by batch texture name; missing ones draw white), its mesh
    /// draws, and the Chaos mesh particles as outlines in `mesh_color` ([`Self::draw_scene`] with the default
    /// lighting).
    pub fn draw_frame<'t>(
        &mut self,
        view: &OrthoView,
        frame: &RenderFrame,
        texture: &dyn Fn(&str) -> Option<&'t RasterTexture>,
        mesh_color: [f32; 4],
    ) {
        let options = SceneDrawOptions { mesh_markers: Some(mesh_color), ..SceneDrawOptions::default() };
        self.draw_scene(view, frame, texture, &options);
    }

    /// Draws a frame in scene order: the mesh draws and sprite batches sorted by their scene order (stable, so
    /// batches without a [`crate::render::BatchState`] keep the frame order), each triangle with every texture
    /// stage sampled and combined ([`combine_textures`]), alpha test, face culling, depth test and write against
    /// the canvas depth buffer, and blending; then the Chaos mesh particle outlines. Textures are looked up by
    /// name; a missing one samples white.
    ///
    /// The draws of one effect share a scene order: draw several effects with one call each.
    pub fn draw_scene<'t>(
        &mut self,
        view: &OrthoView,
        frame: &RenderFrame,
        texture: &dyn Fn(&str) -> Option<&'t RasterTexture>,
        options: &SceneDrawOptions,
    ) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        for (_, triangles) in scene_triangles(frame, &view.camera, options) {
            let samplers: Vec<Option<&RasterTexture>> = triangles.stages.iter().map(|stage| texture(&stage.texture)).collect();
            for first in (0..triangles.vertices.len()).step_by(TRIANGLE_VERTICES) {
                self.fill_scene_triangle(view, first, &triangles, &samplers, options);
            }
        }
        if let (true, Some(color)) = (options.sprites, options.mesh_markers) {
            for mesh in &frame.meshes {
                let (x, y) = view.to_screen(mesh.translation);
                let half = (mesh.scale.abs() * view.pixels_per_unit * 0.5).max(MESH_MARKER_MIN_HALF_PX);
                self.outline_rect((x - half, y - half), (x + half, y + half), color, OVERLAY_BLEND);
            }
        }
    }

    /// One triangle (vertices `first..first + 3` of `draw`) with per-vertex colours: every texture stage sampled
    /// (each axis wrapped or clamped, the map's filters over the mip levels) and combined, alpha test, depth test
    /// and write, blend. Front faces are counter-clockwise on the view plane. Pixels whose depth
    /// ([`OrthoView::depth`]) is outside the options' depth clip are not drawn.
    fn fill_scene_triangle(
        &mut self,
        view: &OrthoView,
        first: usize,
        draw: &Triangles,
        samplers: &[Option<&RasterTexture>],
        options: &SceneDrawOptions,
    ) {
        let corners = &draw.vertices[first..first + TRIANGLE_VERTICES];
        let [(ax, ay), (bx, by), (cx, cy)] = [0, 1, 2].map(|k| view.to_screen(corners[k].position));
        // Screen y points down: a counter-clockwise view-plane triangle has a negative screen area.
        let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
        if area.abs() < f32::EPSILON || !area.is_finite() {
            return;
        }
        let front = area < 0.0;
        match draw.cull {
            CullMode::Back if !front => return,
            CullMode::Front if front => return,
            _ => {}
        }
        let (limit_x, limit_y) = (self.width as f32 - 1.0, self.height as f32 - 1.0);
        let min_x = ax.min(bx).min(cx).floor().clamp(0.0, limit_x) as usize;
        let max_x = ax.max(bx).max(cx).ceil().clamp(0.0, limit_x) as usize;
        let min_y = ay.min(by).min(cy).floor().clamp(0.0, limit_y) as usize;
        let max_y = ay.max(by).max(cy).ceil().clamp(0.0, limit_y) as usize;
        // The rasterizer is orthographic and affine: texture coordinates change at a constant rate per triangle,
        // so each stage has one level of detail.
        let weight_dx = [(by - cy) / area, (cy - ay) / area, (ay - by) / area];
        let weight_dy = [(cx - bx) / area, (ax - cx) / area, (bx - ax) / area];
        let lods: Vec<f32> = draw
            .stages
            .iter()
            .zip(samplers)
            .map(|(stage, sampler)| {
                let Some(texture) = sampler.filter(|_| options.client_filtering) else { return 0.0 };
                let uvs = &stage.uvs[first..first + TRIANGLE_VERTICES];
                let rate = |axis: usize, weights: &[f32; 3]| {
                    (0..TRIANGLE_VERTICES).map(|k| uvs[k][axis] * weights[k]).sum::<f32>()
                };
                let du = [rate(0, &weight_dx), rate(0, &weight_dy)];
                let dv = [rate(1, &weight_dx), rate(1, &weight_dy)];
                texture_lod(texture.width, texture.height, du, dv)
            })
            .collect();
        let depth_test = if options.depth_buffer { draw.depth } else { None };
        let mut samples = Vec::with_capacity(draw.stages.len());
        for py in min_y..=max_y {
            for px in min_x..=max_x {
                let (x, y) = (px as f32 + PIXEL_CENTER_OFFSET, py as f32 + PIXEL_CENTER_OFFSET);
                let wa = ((bx - x) * (cy - y) - (by - y) * (cx - x)) / area;
                let wb = ((cx - x) * (ay - y) - (cy - y) * (ax - x)) / area;
                let wc = 1.0 - wa - wb;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let weights = [wa, wb, wc];
                let lerp = |value: &dyn Fn(usize) -> f32| (0..TRIANGLE_VERTICES).map(|k| weights[k] * value(k)).sum::<f32>();
                let depth = lerp(&|k| view.depth(corners[k].position));
                if let Some([near, far]) = options.depth_clip {
                    if !(near..=far).contains(&depth) {
                        continue;
                    }
                }
                let color: [f32; 4] = std::array::from_fn(|channel| lerp(&|k| corners[k].color[channel]));
                samples.clear();
                for ((stage, sampler), &lod) in draw.stages.iter().zip(samplers).zip(&lods) {
                    let uvs = &stage.uvs[first..first + TRIANGLE_VERTICES];
                    let (u, v) = (lerp(&|k| uvs[k][0]), lerp(&|k| uvs[k][1]));
                    let texel = sampler.map_or(WHITE_TEXEL, |texture| {
                        if options.client_filtering {
                            texture.sample_filtered(u, v, lod, stage.addressing, stage.filter)
                        } else {
                            texture.sample_addressed(u, v, stage.addressing)
                        }
                    });
                    samples.push((stage.slot, texel));
                }
                let source = combine_textures(draw.apply_mode, color, &samples);
                if let Some(test) = draw.alpha_test {
                    if !test.function.passes(source[ALPHA], test.reference) {
                        continue;
                    }
                }
                let pixel = py * self.width + px;
                if let Some(test) = depth_test {
                    if !test.function.passes(depth, self.depth[pixel]) {
                        continue;
                    }
                    if test.write {
                        self.depth[pixel] = depth;
                    }
                }
                self.blend(px, py, source, draw.blend);
            }
        }
    }
}

/// What [`Canvas::draw_scene`] draws, and the lights of the meshes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneDrawOptions {
    /// Lights of the mesh draws (sprite colours are lit by the effect itself).
    pub lighting: SceneLighting,
    /// Draw the sprite batches (particles) and the Chaos mesh particle outlines.
    pub sprites: bool,
    /// Draw the mesh draws (NIF meshes and mesh particles).
    pub meshes: bool,
    /// Colour of the Chaos mesh particle outlines; `None` leaves them out.
    pub mesh_markers: Option<[f32; 4]>,
    /// The near and far planes: the depth range ([`OrthoView::depth`], relative to the view centre) drawn;
    /// `None` draws every depth.
    pub depth_clip: Option<[f32; 2]>,
    /// Test and write depth as each draw's `NiZBufferProperty` says (the client's); off, draws only follow the
    /// scene order.
    pub depth_buffer: bool,
    /// Sample with each map's filter mode over the texture's mip levels (the client's); off, level 0 bilinear.
    pub client_filtering: bool,
}

impl Default for SceneDrawOptions {
    fn default() -> Self {
        Self {
            lighting: SceneLighting::default(),
            sprites: true,
            meshes: true,
            mesh_markers: None,
            depth_clip: None,
            depth_buffer: true,
            client_filtering: true,
        }
    }
}

/// The depth buffer use of a draw, as `NiDX9RenderState::ApplyZBuffer` sets D3D9 (`NiDX9RenderState.cpp:611-632`):
/// the depth buffer is on when the property tests or writes; the comparison is the property's function when it
/// tests, `ALWAYS` when it only writes. A fragment passes when `depth <function> stored depth`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DepthTest {
    function: CompareFunction,
    write: bool,
}

impl DepthTest {
    /// `None` when the property neither tests nor writes (`D3DRS_ZENABLE` false).
    fn of(state: DepthState, function: CompareFunction) -> Option<Self> {
        (state.test || state.write)
            .then(|| Self { function: if state.test { function } else { CompareFunction::Always }, write: state.write })
    }
}

/// One texture stage of a draw: the texture name and the stage's texture coordinates per vertex.
struct Stage {
    slot: TextureSlot,
    texture: Arc<str>,
    uvs: Vec<[f32; 2]>,
    addressing: TextureAddressing,
    filter: TextureFilter,
}

/// The triangles of one draw (a mesh draw or a sprite batch) and their render state.
struct Triangles {
    vertices: Vec<QuadVertex>,
    stages: Vec<Stage>,
    apply_mode: TextureApplyMode,
    blend: BlendState,
    alpha_test: Option<AlphaTest>,
    cull: CullMode,
    depth: Option<DepthTest>,
}

/// The draws of a frame as triangle lists with their scene order, sorted (stable) by it.
fn scene_triangles(frame: &RenderFrame, camera: &CameraFrame, options: &SceneDrawOptions) -> Vec<(u32, Triangles)> {
    let mut out = Vec::new();
    if options.meshes {
        for draw in &frame.mesh_draws {
            let mut vertices = Vec::new();
            expand_mesh_draw(draw, &options.lighting, &mut vertices);
            let stages = draw
                .textures
                .iter()
                .map(|stage| {
                    let mut uvs = Vec::with_capacity(vertices.len());
                    expand_mesh_stage_uvs(draw, stage, &mut uvs);
                    Stage {
                        slot: stage.slot,
                        texture: stage.texture.clone(),
                        uvs,
                        addressing: stage.addressing,
                        filter: stage.filter,
                    }
                })
                .collect();
            let triangles = Triangles {
                vertices,
                stages,
                apply_mode: draw.apply_mode,
                blend: draw.blend.unwrap_or(OPAQUE_BLEND),
                alpha_test: draw.alpha_test,
                cull: draw.cull,
                depth: DepthTest::of(draw.depth, draw.depth_function),
            };
            out.push((draw.order, triangles));
        }
    }
    if options.sprites {
        for (index, batch) in frame.batches.iter().enumerate() {
            let mut vertices = Vec::new();
            for sprite in &frame.sprites[batch.sprites.clone()] {
                let quad = expand_sprite(batch.kind, sprite, camera);
                vertices.extend(QUAD_INDICES.iter().map(|&corner| quad[corner as usize]));
            }
            let triangles = match frame.batch_state(index) {
                Some(state) => Triangles {
                    stages: state
                        .textures
                        .iter()
                        .map(|stage| Stage {
                            slot: stage.slot,
                            texture: stage.texture.clone(),
                            uvs: vertices.iter().map(|vertex| stage.uv_transform.apply(vertex.uv)).collect(),
                            addressing: stage.addressing,
                            filter: stage.filter,
                        })
                        .collect(),
                    vertices,
                    apply_mode: state.apply_mode,
                    blend: batch.blend,
                    alpha_test: state.alpha_test,
                    cull: CullMode::None,
                    depth: DepthTest::of(batch.depth, state.depth_function),
                },
                None => Triangles {
                    stages: (!batch.texture.is_empty())
                        .then(|| Stage {
                            slot: TextureSlot::Base,
                            texture: batch.texture.clone(),
                            uvs: vertices.iter().map(|vertex| vertex.uv).collect(),
                            addressing: SPRITE_ADDRESSING,
                            filter: SPRITE_FILTER,
                        })
                        .into_iter()
                        .collect(),
                    vertices,
                    apply_mode: TextureApplyMode::Modulate,
                    blend: batch.blend,
                    alpha_test: None,
                    cull: CullMode::None,
                    depth: DepthTest::of(batch.depth, SPRITE_DEPTH_FUNCTION),
                },
            };
            out.push((frame.batch_state(index).map_or(0, |state| state.order), triangles));
        }
    }
    out.sort_by_key(|(order, _)| *order);
    out
}

/// The factor of a blend function for one source and destination colour.
pub fn blend_factor(factor: BlendFactor, source: [f32; 4], destination: [f32; 4]) -> [f32; 4] {
    let splat = |value: f32| [value; 4];
    let invert = |color: [f32; 4]| color.map(|channel| 1.0 - channel);
    match factor {
        BlendFactor::One => splat(1.0),
        BlendFactor::Zero => splat(0.0),
        BlendFactor::SrcColor => source,
        BlendFactor::InvSrcColor => invert(source),
        BlendFactor::DestColor => destination,
        BlendFactor::InvDestColor => invert(destination),
        BlendFactor::SrcAlpha => splat(source[ALPHA]),
        BlendFactor::InvSrcAlpha => splat(1.0 - source[ALPHA]),
        BlendFactor::DestAlpha => splat(destination[ALPHA]),
        BlendFactor::InvDestAlpha => splat(1.0 - destination[ALPHA]),
        BlendFactor::SrcAlphaSat => {
            let f = source[ALPHA].min(1.0 - destination[ALPHA]);
            [f, f, f, 1.0]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{DrawBatch, PARTICLE_DEPTH_STATE, SpriteInstance, SpriteKind, UvRect};
    use std::sync::Arc;

    const SIDE: usize = 16;
    const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    /// Emissive colour of the test triangle.
    const TRIANGLE_EMISSIVE: [f32; 3] = [0.5; 3];

    fn frame_with_one_sprite(size: f32, blend: BlendState) -> RenderFrame {
        let mut frame = RenderFrame::default();
        frame.sprites.push(SpriteInstance {
            position: Vec3::ZERO,
            size,
            rotation: 0.0,
            color: [1.0, 0.5, 0.0, 1.0],
            uv: UvRect::FULL,
            direction: Vec3::ZERO,
            length: 0.0,
        });
        frame.batches.push(DrawBatch {
            texture: Arc::from(""),
            blend,
            depth: PARTICLE_DEPTH_STATE,
            two_sided: false,
            kind: SpriteKind::Billboard,
            sprites: 0..1,
        });
        frame
    }

    #[test]
    fn additive_sprites_add_their_colour_at_the_centre() {
        let view = OrthoView::new(0.0, Vec3::ZERO, 100.0, 1.0, SIDE, SIDE);
        let mut canvas = Canvas::new(SIDE, SIDE, BLACK);
        let additive = BlendState { src: BlendFactor::One, dst: BlendFactor::One };
        canvas.draw_frame(&view, &frame_with_one_sprite(4.0, additive), &|_| None, WHITE_TEXEL);
        let center = canvas.pixels[(SIDE / 2) * SIDE + SIDE / 2];
        assert_eq!(center, [1.0, 0.5, 0.0, 1.0]);
        assert_eq!(canvas.pixels[0], BLACK, "the corner is outside the 4-pixel quad");
    }

    #[test]
    fn extent_covers_the_sprite_quad() {
        let view = OrthoView::new(0.0, Vec3::ZERO, 100.0, 1.0, SIDE, SIDE);
        let extent = frame_extent(&frame_with_one_sprite(4.0, OVERLAY_BLEND), &view.camera).unwrap();
        assert_eq!(extent, [-2.0, 2.0, -2.0, 2.0]);
        assert!(frame_extent(&RenderFrame::default(), &view.camera).is_none());
    }

    /// A counter-clockwise (as seen from the camera) triangle covering the lower left half of the canvas, lit by
    /// the default ambient light only: `0.5 * ambient`.
    fn frame_with_one_triangle(cull: CullMode) -> RenderFrame {
        use crate::math::Mat3;
        use crate::nif::desc::{CompareFunction, NifVertexColor};
        use crate::render::{DEFAULT_MATERIAL_COLORS, DepthState, MaterialColors, MeshDraw, MeshGeometry};
        let half = SIDE as f32 * 0.5;
        // The view looks along +y: x is right, z is up.
        let geometry = MeshGeometry {
            positions: vec![[-half, 0.0, -half], [half, 0.0, -half], [-half, 0.0, half]],
            triangles: vec![[0, 1, 2]],
            ..MeshGeometry::default()
        };
        let mut frame = RenderFrame::default();
        frame.mesh_draws.push(MeshDraw {
            order: 0,
            name: Arc::from("triangle"),
            geometry: Arc::new(geometry),
            positions: None,
            rotation: Mat3::IDENTITY,
            translation: Vec3::ZERO,
            scale: 1.0,
            // Unlit by the client's (empty) lighting, the triangle shows its emissive colour.
            material: MaterialColors { emissive: TRIANGLE_EMISSIVE, ..DEFAULT_MATERIAL_COLORS },
            vertex_color: NifVertexColor::default(),
            apply_mode: TextureApplyMode::Modulate,
            textures: Vec::new(),
            blend: None,
            alpha_test: None,
            depth: DepthState { test: true, write: true },
            depth_function: CompareFunction::LessEqual,
            cull,
            lights: Vec::new(),
        });
        frame
    }

    #[test]
    fn mesh_draws_fill_their_front_faces_and_bound_the_frame() {
        let view = OrthoView::new(0.0, Vec3::ZERO, 100.0, 1.0, SIDE, SIDE);
        let lower_left = (SIDE * 3 / 4) * SIDE + SIDE / 4;
        let mut canvas = Canvas::new(SIDE, SIDE, BLACK);
        canvas.draw_scene(&view, &frame_with_one_triangle(CullMode::Back), &|_| None, &SceneDrawOptions::default());
        assert_eq!(canvas.pixels[lower_left], [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(canvas.pixels[(SIDE / 4) * SIDE + SIDE * 3 / 4], BLACK, "the upper right half is empty");
        let mut culled = Canvas::new(SIDE, SIDE, BLACK);
        culled.draw_scene(&view, &frame_with_one_triangle(CullMode::Front), &|_| None, &SceneDrawOptions::default());
        assert_eq!(culled.pixels[lower_left], BLACK, "front faces culled");
        let mut hidden = Canvas::new(SIDE, SIDE, BLACK);
        let no_meshes = SceneDrawOptions { meshes: false, ..SceneDrawOptions::default() };
        hidden.draw_scene(&view, &frame_with_one_triangle(CullMode::Back), &|_| None, &no_meshes);
        assert_eq!(hidden.pixels[lower_left], BLACK, "meshes hidden");
        let half = SIDE as f32 * 0.5;
        let (min, max) = frame_bounds(&frame_with_one_triangle(CullMode::Back)).unwrap();
        assert_eq!((min, max), (Vec3::new(-half, 0.0, -half), Vec3::new(half, 0.0, half)));
    }

    #[test]
    fn orbit_axes_turn_from_north_toward_east() {
        let (direction, up, right) = orbit_axes(std::f32::consts::FRAC_PI_2, 0.0);
        assert!((direction - Vec3::X).length() < 1e-6, "{direction:?}");
        assert!((up - Vec3::Z).length() < 1e-6);
        assert!((right - Vec3::new(0.0, -1.0, 0.0)).length() < 1e-6, "{right:?}");
        assert_eq!(camera_axes(0.3), orbit_axes(0.0, 0.3));
    }

    #[test]
    fn textures_wrap_and_filter() {
        let texture = RasterTexture::from_rgba8(2, 1, &[0, 0, 0, 255, 255, 255, 255, 255]).unwrap();
        assert_eq!(texture.sample(0.25, 0.5), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(texture.sample(0.75, 0.5), [1.0; 4]);
        assert_eq!(texture.sample(0.5, 0.5)[0], 0.5);
        // At the left edge, wrapping blends in the last texel; clamping repeats the first.
        assert_eq!(texture.sample(0.0, 0.5)[0], 0.5);
        assert_eq!(texture.sample_clamped(0.0, 0.5)[0], 0.0);
        assert_eq!(texture.sample_clamped(1.0, 0.5)[0], 1.0);
        assert!(RasterTexture::from_rgba8(2, 2, &[0; 4]).is_none());
    }

    #[test]
    fn clamped_axes_repeat_the_edge_texel_and_axes_address_separately() {
        // Black, white in a row; one row.
        let texture = RasterTexture::from_rgba8(2, 1, &[0, 0, 0, 255, 255, 255, 255, 255]).unwrap();
        let clamp_s = TextureAddressing { wrap_s: false, wrap_t: true };
        // At u = 1 the clamped axis stays on the last texel; wrapping blends the first one in.
        assert_eq!(texture.sample_addressed(1.0, 0.5, clamp_s)[0], 1.0);
        assert_eq!(texture.sample_addressed(1.0, 0.5, WRAP_ADDRESSING)[0], 0.5);
        assert_eq!(texture.sample_addressed(1.7, 0.5, clamp_s)[0], 1.0, "outside [0, 1] the edge repeats");
        assert_eq!(texture.sample_addressed(-0.5, 0.5, clamp_s)[0], 0.0);
    }

    #[test]
    fn map_filters_pick_texels_and_mip_levels() {
        // Level 0: black, white; level 1: grey.
        let levels = [vec![0, 0, 0, 255, 255, 255, 255, 255], vec![100, 100, 100, 255]];
        let texture = RasterTexture::from_rgba8_levels(2, 1, &levels).unwrap();
        assert_eq!(texture.level_count(), 2);
        let grey = 100.0 / U8_MAX;
        let trilinear = TextureFilter::from_raw(crate::nif::desc::FILTER_TRILERP);
        let nearest = TextureFilter::from_raw(crate::nif::desc::FILTER_NEAREST);
        let mip_nearest = TextureFilter::from_raw(crate::nif::desc::FILTER_BILERP_MIPNEAREST);
        // Magnified: level 0 with the mag filter.
        assert_eq!(texture.sample_filtered(0.5, 0.5, -1.0, CLAMP_ADDRESSING, trilinear)[0], 0.5);
        assert_eq!(texture.sample_filtered(0.5, 0.5, -1.0, CLAMP_ADDRESSING, nearest)[0], 1.0, "point: the texel under u");
        // Minified one level: level 1; half way: a blend of the levels; bilinear without mips: level 0.
        assert_eq!(texture.sample_filtered(0.5, 0.5, 1.0, CLAMP_ADDRESSING, trilinear)[0], grey);
        assert!((texture.sample_filtered(0.5, 0.5, 0.5, CLAMP_ADDRESSING, trilinear)[0] - (0.5 + grey) / 2.0).abs() < 1e-6);
        assert_eq!(texture.sample_filtered(0.5, 0.5, 1.0, CLAMP_ADDRESSING, TextureFilter::BILERP)[0], 0.5);
        // Mip nearest rounds the level of detail; past the last level the last one is used.
        assert_eq!(texture.sample_filtered(0.5, 0.5, 0.4, CLAMP_ADDRESSING, mip_nearest)[0], 0.5);
        assert_eq!(texture.sample_filtered(0.5, 0.5, 0.6, CLAMP_ADDRESSING, mip_nearest)[0], grey);
        assert_eq!(texture.sample_filtered(0.5, 0.5, 5.0, CLAMP_ADDRESSING, trilinear)[0], grey);
        // A single-level texture samples level 0 whatever the mip filter.
        let single = RasterTexture::from_rgba8(2, 1, &[0, 0, 0, 255, 255, 255, 255, 255]).unwrap();
        assert_eq!(single.sample_filtered(0.5, 0.5, 1.0, CLAMP_ADDRESSING, trilinear)[0], 0.5);
        // One pixel step covering two texels of a 2-wide texture is level 1.
        assert_eq!(texture_lod(2, 1, [1.0, 0.0], [0.0, 0.0]), 1.0);
    }

    #[test]
    fn the_depth_buffer_hides_sprites_behind_depth_writing_meshes() {
        let view = OrthoView::new(0.0, Vec3::ZERO, 100.0, 1.0, SIDE, SIDE);
        // Inside both the triangle (lower left half) and the 4-pixel sprite quad.
        let pixel = (SIDE / 2 + 1) * SIDE + SIDE / 2 - 1;
        let draw = |sprite_depth: f32, depth_buffer: bool| {
            let mut frame = frame_with_one_triangle(CullMode::None);
            let sprites = frame_with_one_sprite(4.0, OPAQUE_BLEND);
            frame.sprites = sprites.sprites;
            frame.sprites[0].position = Vec3::new(0.0, sprite_depth, 0.0);
            frame.batches = sprites.batches;
            let mut canvas = Canvas::new(SIDE, SIDE, BLACK);
            let options = SceneDrawOptions { depth_buffer, ..SceneDrawOptions::default() };
            canvas.draw_scene(&view, &frame, &|_| None, &options);
            canvas.pixels[pixel]
        };
        let triangle = [0.5, 0.5, 0.5, 1.0];
        let sprite = [1.0, 0.5, 0.0, 1.0];
        // The view looks along +y: a sprite at y = 1 is behind the triangle's depth write at y = 0.
        assert_eq!(draw(1.0, true), triangle);
        assert_eq!(draw(-1.0, true), sprite);
        assert_eq!(draw(1.0, false), sprite, "without the depth buffer the later draw covers");
        // Depth state as NiDX9RenderState::ApplyZBuffer maps it.
        assert_eq!(DepthTest::of(DepthState { test: false, write: false }, CompareFunction::Less), None);
        let write_only = DepthTest::of(DepthState { test: false, write: true }, CompareFunction::Less).unwrap();
        assert_eq!(write_only, DepthTest { function: CompareFunction::Always, write: true });
    }

    #[test]
    fn the_client_view_draws_local_z_up_and_x_right() {
        let distance = 100.0;
        let view = OrthoView::client_view(Vec3::ZERO, distance, 1.0, SIDE, SIDE);
        assert_eq!(view.camera, CameraFrame::client_view(distance));
        let (center_x, center_y) = view.to_screen(Vec3::ZERO);
        let (up_x, up_y) = view.to_screen(Vec3::Z);
        let (right_x, right_y) = view.to_screen(Vec3::X);
        assert!(up_y < center_y && up_x == center_x, "local +z is up on the canvas (canvas y points down)");
        assert!(right_x > center_x && right_y == center_y, "local +x is right");
        assert_eq!(view.to_screen(Vec3::Y), (center_x, center_y), "local +y is the view direction");
        assert_eq!(view.depth(Vec3::Y), 1.0);
        assert_eq!(view.to_world(view.to_screen(Vec3::new(3.0, 0.0, -2.0))), Vec3::new(3.0, 0.0, -2.0));
    }

    #[test]
    fn depth_clipping_keeps_the_range_between_the_planes() {
        let view = OrthoView::new(0.0, Vec3::ZERO, 100.0, 1.0, SIDE, SIDE);
        let center = (SIDE / 2) * SIDE + SIDE / 2;
        let draw = |depth_clip| {
            let mut canvas = Canvas::new(SIDE, SIDE, BLACK);
            let options = SceneDrawOptions { depth_clip, ..SceneDrawOptions::default() };
            canvas.draw_scene(&view, &frame_with_one_sprite(4.0, OPAQUE_BLEND), &|_| None, &options);
            canvas.pixels[center]
        };
        assert_ne!(draw(Some([-1.0, 1.0])), BLACK, "the sprite is at depth 0");
        assert_eq!(draw(Some([0.5, 1.0])), BLACK, "in front of the near plane");
        assert_eq!(draw(Some([-1.0, -0.5])), BLACK, "behind the far plane");
    }
}
