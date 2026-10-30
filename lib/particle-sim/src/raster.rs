//! A small software rasterizer for previews: draws a [`RenderFrame`] into a float RGBA canvas with an
//! orthographic camera, bilinear wrap-addressed textures and the client's blend factors (D3D blend math).
//!
//! It is a preview tool, not the client's renderer: no depth buffer (sprites are drawn in frame order, as the
//! client does with depth writes off), no mip-mapping, and MESH particles (no NIF renderer here) are drawn as
//! square outlines of their size.

use crate::math::Vec3;
use crate::render::{BlendFactor, BlendState, CameraFrame, QUAD_INDICES, QuadVertex, RenderFrame, expand_sprite};

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
        let x = u * self.width as f32 - TEXEL_CENTER_OFFSET;
        let y = v * self.height as f32 - TEXEL_CENTER_OFFSET;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let texel = |dx: f32, dy: f32| {
            let column = ((x0 + dx) as i64).rem_euclid(self.width as i64) as usize;
            let row = ((y0 + dy) as i64).rem_euclid(self.height as i64) as usize;
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

/// The camera axes of a view from the south looking north (+y), `elevation` radians above the horizon:
/// (direction, up, right).
pub fn camera_axes(elevation: f32) -> (Vec3, Vec3, Vec3) {
    let direction = Vec3::new(0.0, elevation.cos(), -elevation.sin());
    let up = Vec3::new(0.0, elevation.sin(), elevation.cos());
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
    /// A view looking at `center` from `elevation` radians above the horizon, the camera `distance` away.
    pub fn new(elevation: f32, center: Vec3, distance: f32, pixels_per_unit: f32, width: usize, height: usize) -> Self {
        let (direction, up, right) = camera_axes(elevation);
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
}

/// The extent of what a frame draws on the view plane: (min right, max right, min up, max up), padded by the
/// sprite sizes. `None` for an empty frame.
pub fn frame_extent(frame: &RenderFrame, camera: &CameraFrame) -> Option<[f32; 4]> {
    let project = |point: Vec3| (point.dot(camera.right), point.dot(camera.up));
    let mut extent: Option<[f32; 4]> = None;
    let mut include = |point: Vec3, pad: f32| {
        let (x, y) = project(point);
        let [min_x, max_x, min_y, max_y] = extent.get_or_insert([x, x, y, y]);
        *min_x = min_x.min(x - pad);
        *max_x = max_x.max(x + pad);
        *min_y = min_y.min(y - pad);
        *max_y = max_y.max(y + pad);
    };
    for sprite in &frame.sprites {
        include(sprite.position, sprite.size.abs() * 0.5 + sprite.length.abs());
    }
    for mesh in &frame.meshes {
        include(mesh.translation, mesh.scale.abs() * 0.5);
    }
    extent
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

    /// Draws every sprite batch of `frame` (textures by batch texture name; missing ones draw white) and the
    /// mesh particles as outlines in `mesh_color`.
    pub fn draw_frame<'t>(
        &mut self,
        view: &OrthoView,
        frame: &RenderFrame,
        texture: &dyn Fn(&str) -> Option<&'t RasterTexture>,
        mesh_color: [f32; 4],
    ) {
        for batch in &frame.batches {
            let texture = texture(&batch.texture);
            for sprite in &frame.sprites[batch.sprites.clone()] {
                let vertices = expand_sprite(batch.kind, sprite, &view.camera);
                for triangle in QUAD_INDICES.chunks_exact(3) {
                    let corners = [0, 1, 2].map(|k| vertices[triangle[k] as usize]);
                    self.fill_triangle(view, &corners, texture, batch.blend);
                }
            }
        }
        for mesh in &frame.meshes {
            let (x, y) = view.to_screen(mesh.translation);
            let half = (mesh.scale.abs() * view.pixels_per_unit * 0.5).max(MESH_MARKER_MIN_HALF_PX);
            self.outline_rect((x - half, y - half), (x + half, y + half), mesh_color, OVERLAY_BLEND);
        }
    }
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

    #[test]
    fn textures_wrap_and_filter() {
        let texture = RasterTexture::from_rgba8(2, 1, &[0, 0, 0, 255, 255, 255, 255, 255]).unwrap();
        assert_eq!(texture.sample(0.25, 0.5), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(texture.sample(0.75, 0.5), [1.0; 4]);
        assert_eq!(texture.sample(0.5, 0.5)[0], 0.5);
        assert!(RasterTexture::from_rgba8(2, 2, &[0; 4]).is_none());
    }
}
