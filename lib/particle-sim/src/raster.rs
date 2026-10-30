//! A small software rasterizer for previews: draws a [`RenderFrame`] into a float RGBA canvas with an
//! orthographic camera, bilinear textures and the client's blend factors (D3D blend math).
//!
//! It is a preview tool, not the client's renderer: no depth buffer (draws follow the scene order, sprites the
//! frame order, as the client does with depth writes off), no mip-mapping, unlit-by-direction meshes (see
//! [`SceneLighting`]), and Chaos MESH particles (their `.nif` scenes are not drawn) are square outlines of their
//! size.

use std::sync::Arc;

use crate::math::Vec3;
use crate::nif::desc::{CullMode, TextureAddressing, TextureApplyMode, TextureSlot};
use crate::render::{
    AlphaTest, BlendFactor, BlendState, CameraFrame, QUAD_INDICES, QuadVertex, RenderFrame, SceneLighting,
    TRIANGLE_VERTICES, combine_textures, expand_mesh_draw, expand_mesh_stage_uvs, expand_sprite,
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
/// Sprite textures of batches without a [`crate::render::BatchState`] (Chaos scripts) repeat.
const SPRITE_ADDRESSING: TextureAddressing = TextureAddressing { wrap_s: true, wrap_t: true };

/// A decoded texture, RGBA in [0, 1].
#[derive(Debug, Clone, PartialEq)]
pub struct RasterTexture {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[f32; 4]>,
}

impl RasterTexture {
    /// A texture from 8-bit RGBA rows; `None` when the sizes do not match or are zero.
    pub fn from_rgba8(width: usize, height: usize, rgba: &[u8]) -> Option<Self> {
        if width == 0 || height == 0 || rgba.len() != width * height * 4 {
            return None;
        }
        let texels = rgba
            .chunks_exact(4)
            .map(|pixel| std::array::from_fn(|k| f32::from(pixel[k]) / U8_MAX))
            .collect();
        Some(Self { width, height, texels })
    }

    /// Bilinear sample with wrap addressing (Gamebryo's default, probable for the client).
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        self.sample_addressed(u, v, true)
    }

    /// Bilinear sample with clamp addressing: texels outside the texture repeat its edge.
    pub fn sample_clamped(&self, u: f32, v: f32) -> [f32; 4] {
        self.sample_addressed(u, v, false)
    }

    fn sample_addressed(&self, u: f32, v: f32, wrap: bool) -> [f32; 4] {
        let x = u * self.width as f32 - TEXEL_CENTER_OFFSET;
        let y = v * self.height as f32 - TEXEL_CENTER_OFFSET;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let address = |index: f32, size: usize| {
            let (index, size) = (index as i64, size as i64);
            (if wrap { index.rem_euclid(size) } else { index.clamp(0, size - 1) }) as usize
        };
        let texel = |dx: f32, dy: f32| {
            let column = address(x0 + dx, self.width);
            let row = address(y0 + dy, self.height);
            self.texels[row * self.width + column]
        };
        let (a, b, c, d) = (texel(0.0, 0.0), texel(1.0, 0.0), texel(0.0, 1.0), texel(1.0, 1.0));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            top + (bottom - top) * fy
        })
    }
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

/// A float RGBA canvas with D3D-style blending.
#[derive(Debug, Clone, PartialEq)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[f32; 4]>,
}

impl Canvas {
    pub fn new(width: usize, height: usize, background: [f32; 4]) -> Self {
        Self { width, height, pixels: vec![background; width * height] }
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
    /// stage sampled and combined ([`combine_textures`]), alpha test, face culling and blending; then the Chaos
    /// mesh particle outlines. Textures are looked up by name; a missing one samples white.
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
                self.fill_scene_triangle(view, first, &triangles, &samplers, options.depth_clip);
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
    /// (wrapped or clamped) and combined, alpha test, blend. Front faces are counter-clockwise on the view plane.
    /// Pixels whose depth ([`OrthoView::depth`]) is outside `depth_clip` are not drawn.
    fn fill_scene_triangle(
        &mut self,
        view: &OrthoView,
        first: usize,
        draw: &Triangles,
        samplers: &[Option<&RasterTexture>],
        depth_clip: Option<[f32; 2]>,
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
        let address = |value: f32, wrap: bool| if wrap { value } else { value.clamp(0.0, 1.0) };
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
                if let Some([near, far]) = depth_clip {
                    let depth = lerp(&|k| view.depth(corners[k].position));
                    if !(near..=far).contains(&depth) {
                        continue;
                    }
                }
                let color: [f32; 4] = std::array::from_fn(|channel| lerp(&|k| corners[k].color[channel]));
                samples.clear();
                for (stage, sampler) in draw.stages.iter().zip(samplers) {
                    let uvs = &stage.uvs[first..first + TRIANGLE_VERTICES];
                    let u = address(lerp(&|k| uvs[k][0]), stage.addressing.wrap_s);
                    let v = address(lerp(&|k| uvs[k][1]), stage.addressing.wrap_t);
                    samples.push((stage.slot, sampler.map_or(WHITE_TEXEL, |texture| texture.sample(u, v))));
                }
                let source = combine_textures(draw.apply_mode, color, &samples);
                if let Some(test) = draw.alpha_test {
                    if !test.function.passes(source[ALPHA], test.reference) {
                        continue;
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
}

impl Default for SceneDrawOptions {
    fn default() -> Self {
        Self { lighting: SceneLighting::default(), sprites: true, meshes: true, mesh_markers: None, depth_clip: None }
    }
}

/// One texture stage of a draw: the texture name and the stage's texture coordinates per vertex.
struct Stage {
    slot: TextureSlot,
    texture: Arc<str>,
    uvs: Vec<[f32; 2]>,
    addressing: TextureAddressing,
}

/// The triangles of one draw (a mesh draw or a sprite batch) and their render state.
struct Triangles {
    vertices: Vec<QuadVertex>,
    stages: Vec<Stage>,
    apply_mode: TextureApplyMode,
    blend: BlendState,
    alpha_test: Option<AlphaTest>,
    cull: CullMode,
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
                    Stage { slot: stage.slot, texture: stage.texture.clone(), uvs, addressing: stage.addressing }
                })
                .collect();
            let triangles = Triangles {
                vertices,
                stages,
                apply_mode: draw.apply_mode,
                blend: draw.blend.unwrap_or(OPAQUE_BLEND),
                alpha_test: draw.alpha_test,
                cull: draw.cull,
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
                        })
                        .collect(),
                    vertices,
                    apply_mode: state.apply_mode,
                    blend: batch.blend,
                    alpha_test: state.alpha_test,
                    cull: CullMode::None,
                },
                None => Triangles {
                    stages: (!batch.texture.is_empty())
                        .then(|| Stage {
                            slot: TextureSlot::Base,
                            texture: batch.texture.clone(),
                            uvs: vertices.iter().map(|vertex| vertex.uv).collect(),
                            addressing: SPRITE_ADDRESSING,
                        })
                        .into_iter()
                        .collect(),
                    vertices,
                    apply_mode: TextureApplyMode::Modulate,
                    blend: batch.blend,
                    alpha_test: None,
                    cull: CullMode::None,
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
