//! Hued NIF effects: `UOHuedParticleEffectNif` (UOSA.exe, constructor 0x0041d9c0), as described in
//! `scripts/dev-helpers/ec-research/ghidra-effects/results/uosa_effects.md`, "Hued NIF effects".
//!
//! - **Selection** (`UOParticleEffectDefinition::Instantiate` 0x0040e490): only `.nif` particles are hued, and only
//!   with an object whose hue H is non-zero and the definition's `hued` flag set ([`hue_mode`]). Chaos (`.ems`)
//!   effects never are.
//! - **Time**: the clock advances by `dt * HUED_EFFECT_TIME_SCALE` (half speed); see
//!   [`crate::NifEffectInstance::set_time_scale`].
//! - **Rendering** ([`draw_hued_frame`]): the effect is drawn into an offscreen target by an orthographic camera
//!   with the world camera's rotation, placed `HUED_CAMERA_DISTANCE` behind the effect ([`offscreen_view`]); a
//!   camera-facing quad of half side `HUED_QUAD_HALF_SIZE` at the effect position then draws the target with the
//!   `uoeffects.psh` pixel shader ([`hued_color`]): the colour is the hue gradient entry of the pixel's luminance
//!   ([`hue_lookup`] in the `Hues.uop` atlas `build/hues/hues.dds`), the alpha is the rendered alpha times the
//!   hue alpha times the vertex alpha; blending is SRCALPHA / INVSRCALPHA with alpha test GREATER 0, whatever the
//!   NIF's own blend modes.
//!
//! The target clear colour, the quad's texture coordinates, the samplers and the hue coordinate are confirmed
//! (`uosa_effects.md`, "Hued NIF effects"). The one departure from the client: the target keeps square texels
//! ([`offscreen_view`]), which changes only its horizontal resolution.

use crate::math::Vec3;
use crate::raster::{Canvas, OVERLAY_BLEND, OrthoView, RasterTexture, SceneDrawOptions};
use crate::render::{CameraFrame, RenderFrame, SortKey};

/// Hue value of an object without a hue: never hued.
pub const NO_HUE: u32 = 0;
/// `UOHuedParticleEffectNif::Update` (0x0041edc0) advances the clock by `dt` times this (`float` 0.5 at
/// 0x00c95604): hued NIF effects run at half speed, and a non-looping one lasts twice as long. The lifetime tick
/// countdown is unaffected.
pub const HUED_EFFECT_TIME_SCALE: f32 = 0.5;
/// Luminance weights of the rendered colour (`uoeffects.psh`, `Shaders.uop`): `dot(rgb, weights)`.
pub const HUE_LUMINANCE_WEIGHTS: [f32; 3] = [0.2125, 0.7154, 0.0721];
/// Hue rows per atlas column (`BuildHueQuad` 0x0041df40): column = H / rows, row = H % rows.
pub const HUE_ATLAS_ROWS: u32 = 1024;
/// Scale of the hue coordinates (`float` 1/1024 at 0x00d05158; the shader's `/ 1024`): one atlas texel of the
/// 1024 x 1024 `hues.dds`.
pub const HUE_ATLAS_UV_SCALE: f32 = 1.0 / 1024.0;
/// Texels between atlas columns in the client's hue coordinate (`float` 255.0 at 0x00d04ef0):
/// `u = column * HUE_ATLAS_UV_SCALE * HUE_ATLAS_COLUMN_STRIDE`. The atlas strips are 256 wide
/// ([`HUE_ATLAS_STRIP_WIDTH`]), so columns 1 and 2 sample `column` texels left of their strip.
pub const HUE_ATLAS_COLUMN_STRIDE: f32 = 255.0;
/// Texels the luminance spans (`uoeffects.psh`: `uv2 + (lum * 255 / 1024, 0)`).
pub const HUE_LUMINANCE_SPAN: f32 = 255.0;
/// Width of one hue gradient strip of `hues.dds` (`uocf::enhanced::hues::HUE_STRIP_WIDTH`).
pub const HUE_ATLAS_STRIP_WIDTH: u32 = 256;
/// Side of the offscreen render target (`BuildHueQuad`: 512 x 512).
pub const HUED_TARGET_SIZE_PX: usize = 512;
/// Half height of the offscreen orthographic frustum, world units (`CreateOffscreenCamera` 0x00472160: top 4.0 at
/// 0x00c9b2c8, bottom -4.0 at 0x00d05258); the half width is this times the screen aspect.
pub const HUED_FRUSTUM_HALF_HEIGHT: f32 = 4.0;
/// Near and far planes of the offscreen camera (0x00c9c3a4, 0x00c9c3a8).
pub const HUED_CAMERA_NEAR: f32 = 8.0;
pub const HUED_CAMERA_FAR: f32 = 12.0;
/// The offscreen camera stands this far behind the effect along the view direction (`PlaceOffscreenCamera`
/// 0x004722a0: the world viewer's camera offset, `g_UOWorldViewer` +0x18 = -10 x view direction).
pub const HUED_CAMERA_DISTANCE: f32 = 10.0;
/// Half side of the camera-facing hue quad along camera right and up (`float` 4.8541555 at 0x00d05360).
pub const HUED_QUAD_HALF_SIZE: f32 = 4.854_155_5;
/// Clear colour of the offscreen target: transparent black (confirmed). `RenderToTexture` 0x0041ebc0 saves the
/// renderer's background colour, sets it to `NiColorA(0, 0, 0, 0)` (vtable +0x58, 0x00761580, which keeps the
/// alpha), clears colour, depth and stencil (flags 7), and restores the saved colour after drawing.
pub const HUED_TARGET_CLEAR: [f32; 4] = [0.0; 4];
/// Alpha test of the hue quad (`NiAlphaProperty` flags 0x12ED): GREATER than this.
pub const HUED_ALPHA_REFERENCE: f32 = 0.0;
/// Vertex colour of the hue quad: white.
pub const HUED_QUAD_VERTEX_ALPHA: f32 = 1.0;
/// Screen aspect (width / height) of the offscreen frustum by default: the world camera's 4:3 (`left/right = ∓f /
/// 0.75`, 0x00db9754). The client uses the actual screen size (0x00e3352c or 0x00e33520).
pub const DEFAULT_HUED_SCREEN_ASPECT: f32 = 4.0 / 3.0;

/// Whether a particle plays hued, and why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HueMode {
    /// `UOHuedParticleEffectNif`.
    Hued,
    /// A Chaos (`.ems`) effect: the object is not passed, so never hued.
    Chaos,
    /// The definition's `hued` flag is clear.
    FlagNotSet,
    /// The object hue is [`NO_HUE`] (or there is no object).
    NoHue,
}

/// How the client chooses the class of a particle (`UOParticleEffectDefinition::Instantiate` 0x0040e490). Without
/// a hue nothing else matters, so [`HueMode::NoHue`] comes first.
pub fn hue_mode(chaos: bool, hued_flag: bool, hue: u32) -> HueMode {
    if hue == NO_HUE {
        HueMode::NoHue
    } else if chaos {
        HueMode::Chaos
    } else if !hued_flag {
        HueMode::FlagNotSet
    } else {
        HueMode::Hued
    }
}

/// How the hue atlas is sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HueAtlasSampling {
    /// As the client: bilinear with wrap addressing (base map flags 0x3100 = `WRAP_S_WRAP_T` + `FILTER_BILERP`,
    /// `BuildHueQuad` 0x0041e922) at the shader's coordinate, which falls on texel edges. Each sample blends rows
    /// `row - 1` and `row` (row 0 wraps to 1023, a transparent row of the atlas) and, at whole luminance steps, two
    /// adjacent gradient entries; columns 1 and 2 sit `column` texels left of their strip. Nothing clamps or masks
    /// the hue: a hue past the atlas's loaded rows samples whatever texels its coordinate reaches (u wraps past 1),
    /// transparent where the atlas is empty (confirmed: `uosa_effects.md`, "Shader and formula").
    #[default]
    Client,
    /// The entry the formula aims at: texel (`column * 256 + round(lum * 255)`, `row`), unfiltered.
    IntendedEntry,
}

/// The sampling the hued renderer uses unless told otherwise: the client's.
pub const DEFAULT_HUE_ATLAS_SAMPLING: HueAtlasSampling = HueAtlasSampling::Client;

/// The second texture coordinate of the hue quad for hue `hue` (`BuildHueQuad` 0x0041df40): (0, 0) for
/// [`NO_HUE`], else `u = (H / 1024) * (1 / 1024) * 255`, `v = (H % 1024) / 1024`.
pub fn hue_atlas_uv(hue: u32) -> [f32; 2] {
    if hue == NO_HUE {
        return [0.0; 2];
    }
    let (column, row) = (hue / HUE_ATLAS_ROWS, hue % HUE_ATLAS_ROWS);
    [column as f32 * HUE_ATLAS_UV_SCALE * HUE_ATLAS_COLUMN_STRIDE, row as f32 * HUE_ATLAS_UV_SCALE]
}

/// Luminance of a rendered colour (`uoeffects.psh`).
pub fn luminance(rgb: [f32; 3]) -> f32 {
    rgb.iter().zip(HUE_LUMINANCE_WEIGHTS).map(|(channel, weight)| channel * weight).sum()
}

/// The atlas colour of hue `hue` at luminance `luminance`.
pub fn hue_lookup(atlas: &RasterTexture, hue: u32, luminance: f32, sampling: HueAtlasSampling) -> [f32; 4] {
    match sampling {
        HueAtlasSampling::Client => {
            let [u, v] = hue_atlas_uv(hue);
            atlas.sample(u + luminance * HUE_LUMINANCE_SPAN * HUE_ATLAS_UV_SCALE, v)
        }
        HueAtlasSampling::IntendedEntry => {
            let entry = (luminance.clamp(0.0, 1.0) * HUE_LUMINANCE_SPAN).round() as u32;
            let x = (hue / HUE_ATLAS_ROWS * HUE_ATLAS_STRIP_WIDTH + entry) as usize % atlas.width;
            let y = (hue % HUE_ATLAS_ROWS) as usize % atlas.height;
            atlas.texels[y * atlas.width + x]
        }
    }
}

/// The hue quad's pixel shader: the hue colour of the rendered pixel's luminance, alpha = rendered alpha x hue
/// alpha x vertex alpha. [`NO_HUE`] (hue coordinate (0, 0)) leaves the rendered pixel as it is.
pub fn hued_color(rendered: [f32; 4], vertex_alpha: f32, hue: u32, atlas: &RasterTexture, sampling: HueAtlasSampling) -> [f32; 4] {
    if hue == NO_HUE {
        return rendered;
    }
    let [red, green, blue, alpha] = rendered;
    let [hue_red, hue_green, hue_blue, hue_alpha] = hue_lookup(atlas, hue, luminance([red, green, blue]), sampling);
    [hue_red, hue_green, hue_blue, alpha * hue_alpha * vertex_alpha]
}

/// The settings of the hued renderer that the client takes from elsewhere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HuedRenderSettings {
    /// Screen width / height: the offscreen frustum is `2 * HUED_FRUSTUM_HALF_HEIGHT * aspect` wide.
    pub aspect: f32,
    pub sampling: HueAtlasSampling,
}

impl Default for HuedRenderSettings {
    fn default() -> Self {
        Self { aspect: DEFAULT_HUED_SCREEN_ASPECT, sampling: DEFAULT_HUE_ATLAS_SAMPLING }
    }
}

/// The offscreen camera of a hued effect at `effect_position` (`PlaceOffscreenCamera` 0x004722a0): the world
/// camera's axes, `HUED_CAMERA_DISTANCE` behind the effect. A hued NIF instance's billboards face it.
pub fn offscreen_camera(world: &CameraFrame, effect_position: Vec3) -> CameraFrame {
    CameraFrame { position: effect_position - world.direction * HUED_CAMERA_DISTANCE, ..*world }
}

/// The offscreen target's view: [`offscreen_camera`] centred on the effect, `HUED_TARGET_SIZE_PX` pixels for the
/// frustum's height. The client's target is 512 x 512 texels over a frustum `aspect` times wider than high; this
/// one keeps square texels, so it is `round(512 * aspect)` pixels wide (only the horizontal texel density differs).
pub fn offscreen_view(world: &CameraFrame, effect_position: Vec3, aspect: f32) -> OrthoView {
    let pixels_per_unit = HUED_TARGET_SIZE_PX as f32 / (2.0 * HUED_FRUSTUM_HALF_HEIGHT);
    let width = ((HUED_TARGET_SIZE_PX as f32 * aspect).round() as usize).max(1);
    OrthoView {
        camera: offscreen_camera(world, effect_position),
        center: effect_position,
        pixels_per_unit,
        width,
        height: HUED_TARGET_SIZE_PX,
    }
}

/// `UOHuedParticleEffectNif::RenderToTexture` (0x0041ebc0): `frame` drawn into a cleared offscreen target, clipped
/// to the near and far planes, in the order of [`Canvas::draw_group`]. The target is `A8R8G8B8`
/// (`NiRenderedTexture::Create` 0x0067f480): its alpha starts at the clear alpha 0 and accumulates with the
/// draws' blend factors ([`crate::raster::DestinationAlpha::Accumulate`]). `options` are the scene options of the
/// draw (the client sets no light here).
pub fn render_hued_target<'t>(
    frame: &RenderFrame,
    texture: &dyn Fn(&str) -> Option<&'t RasterTexture>,
    options: &SceneDrawOptions,
    world: &CameraFrame,
    effect_position: Vec3,
    aspect: f32,
) -> RasterTexture {
    let view = offscreen_view(world, effect_position, aspect);
    let mut target = Canvas::render_target(view.width, view.height, HUED_TARGET_CLEAR);
    let depth_clip = Some([HUED_CAMERA_NEAR - HUED_CAMERA_DISTANCE, HUED_CAMERA_FAR - HUED_CAMERA_DISTANCE]);
    target.draw_scene(&view, frame, texture, &SceneDrawOptions { depth_clip, ..*options });
    RasterTexture { width: target.width, height: target.height, texels: target.pixels, mips: Vec::new() }
}

/// Draws the hue quad of a target from [`render_hued_target`] into `canvas`: a square of half side
/// `HUED_QUAD_HALF_SIZE` along the camera right and up of `view` at `effect_position`, each pixel the target sampled
/// bilinearly with clamping (shader map flags `0x3100 & 0xc1ff | 0x100` = `CLAMP_S_CLAMP_T` + `FILTER_BILERP`,
/// 0x0041e874) and run through [`hued_color`], alpha tested and alpha blended. The texture coordinates span the
/// whole target with no half-texel offset (confirmed, `BuildHueQuad` 0x0041df40, UV writes 0x0041e420..0x0041e499:
/// vertices top-left, top-right, bottom-left, bottom-right at camera up/right take (0, 0), (1, 0), (0, 1), (1, 1)).
/// As the quad is square and the frustum is not, the effect shows `HUED_QUAD_HALF_SIZE / HUED_FRUSTUM_HALF_HEIGHT`
/// (about 1.21) times taller than plain, and that divided by the aspect as wide.
pub fn draw_hued_quad(
    canvas: &mut Canvas,
    view: &OrthoView,
    target: &RasterTexture,
    effect_position: Vec3,
    hue: u32,
    atlas: &RasterTexture,
    sampling: HueAtlasSampling,
) {
    if canvas.width == 0 || canvas.height == 0 {
        return;
    }
    let camera = &view.camera;
    let corners = [-1.0, 1.0].map(|sign| effect_position + (camera.right + camera.up) * (sign * HUED_QUAD_HALF_SIZE));
    let ((x0, y1), (x1, y0)) = (view.to_screen(corners[0]), view.to_screen(corners[1]));
    let (limit_x, limit_y) = (canvas.width as f32 - 1.0, canvas.height as f32 - 1.0);
    let (min_x, max_x) = (x0.min(x1).floor().clamp(0.0, limit_x) as usize, x0.max(x1).ceil().clamp(0.0, limit_x) as usize);
    let (min_y, max_y) = (y0.min(y1).floor().clamp(0.0, limit_y) as usize, y0.max(y1).ceil().clamp(0.0, limit_y) as usize);
    let side = 2.0 * HUED_QUAD_HALF_SIZE;
    for py in min_y..=max_y {
        for px in min_x..=max_x {
            let offset = view.to_world((px as f32 + PIXEL_CENTER, py as f32 + PIXEL_CENTER)) - effect_position;
            let (u, v) = (offset.dot(camera.right) / side + PIXEL_CENTER, PIXEL_CENTER - offset.dot(camera.up) / side);
            if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                continue;
            }
            let rendered = target.sample_clamped(u, v);
            let source = hued_color(rendered, HUED_QUAD_VERTEX_ALPHA, hue, atlas, sampling);
            if source[ALPHA] > HUED_ALPHA_REFERENCE {
                canvas.blend(px, py, source, OVERLAY_BLEND);
            }
        }
    }
}

/// The place of a hue quad in the world draw group ([`crate::raster::GroupItem::Custom`]): it blends (flags
/// 0x12ED: blending on, no-sorter bit 0x2000 clear), so it is sorted, by its bound centre, the effect position.
pub fn hue_quad_sort_key(effect_position: Vec3) -> SortKey {
    SortKey::sorted(effect_position)
}

/// A hued NIF effect's frame drawn as the client does: [`render_hued_target`] with the camera of `view` (the
/// world camera), then [`draw_hued_quad`].
pub fn draw_hued_frame<'t>(
    canvas: &mut Canvas,
    view: &OrthoView,
    frame: &RenderFrame,
    texture: &dyn Fn(&str) -> Option<&'t RasterTexture>,
    options: &SceneDrawOptions,
    effect_position: Vec3,
    hue: u32,
    atlas: &RasterTexture,
    settings: &HuedRenderSettings,
) {
    let target = render_hued_target(frame, texture, options, &view.camera, effect_position, settings.aspect);
    draw_hued_quad(canvas, view, &target, effect_position, hue, atlas, settings.sampling);
}

/// Pixel centres and the quad's centre texture coordinate.
const PIXEL_CENTER: f32 = 0.5;
/// Index of the alpha channel.
const ALPHA: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nif::{NifEffect, NifEffectDesc, NifEffectInstance};
    use crate::raster::OPAQUE_BLEND;
    use crate::render::{DrawBatch, PARTICLE_DEPTH_STATE, SpriteInstance, SpriteKind, UvRect};
    use crate::system::{NodeTransform, ParticleSystemInstance};
    use std::sync::Arc;

    const ATLAS_SIDE: usize = 1024;
    const TOLERANCE: f32 = 1e-6;

    /// A 1024 x 1024 atlas whose texel (x, y) is (x / 1024, y / 1024, 0.5, 1): a sample tells which texels it
    /// blended.
    fn coded_atlas() -> RasterTexture {
        let side = ATLAS_SIDE as f32;
        let texels = (0..ATLAS_SIDE * ATLAS_SIDE)
            .map(|index| [(index % ATLAS_SIDE) as f32 / side, (index / ATLAS_SIDE) as f32 / side, 0.5, 1.0])
            .collect();
        RasterTexture { width: ATLAS_SIDE, height: ATLAS_SIDE, texels, mips: Vec::new() }
    }

    fn assert_texel(actual: [f32; 4], x: f32, y: f32) {
        let side = ATLAS_SIDE as f32;
        assert!((actual[0] * side - x).abs() < 1e-3 && (actual[1] * side - y).abs() < 1e-3, "{actual:?} != texel ({x}, {y})");
    }

    #[test]
    fn only_nif_particles_with_the_flag_and_a_hue_are_hued() {
        assert_eq!(hue_mode(false, true, 0x21), HueMode::Hued);
        assert_eq!(hue_mode(true, true, 0x21), HueMode::Chaos);
        assert_eq!(hue_mode(false, false, 0x21), HueMode::FlagNotSet);
        assert_eq!(hue_mode(false, true, NO_HUE), HueMode::NoHue);
        assert_eq!(hue_mode(true, false, NO_HUE), HueMode::NoHue);
    }

    /// Hand-computed hue coordinates and samples of the client formula.
    #[test]
    fn the_hue_lookup_follows_the_client_formula() {
        assert_eq!(hue_atlas_uv(NO_HUE), [0.0, 0.0]);
        assert_eq!(hue_atlas_uv(1), [0.0, 1.0 / 1024.0]);
        assert_eq!(hue_atlas_uv(1025), [255.0 / 1024.0, 1.0 / 1024.0]);
        assert_eq!(hue_atlas_uv(2053), [510.0 / 1024.0, 5.0 / 1024.0]);
        assert!((luminance([1.0, 1.0, 1.0]) - 1.0).abs() < TOLERANCE);
        assert!((luminance([0.0, 1.0, 0.0]) - 0.7154).abs() < TOLERANCE);

        let atlas = coded_atlas();
        let client = HueAtlasSampling::Client;
        // H = 1, lum 0: u = 0, v = 1/1024, both on texel edges: x blends 1023 (wrapped) and 0, y blends 0 and 1.
        assert_texel(hue_lookup(&atlas, 1, 0.0, client), 511.5, 0.5);
        // H = 1025 (column 1, row 1), lum 1: u = 510/1024, x blends 509 and 510, one texel left of the strip end.
        assert_texel(hue_lookup(&atlas, 1025, 1.0, client), 509.5, 0.5);
        // H = 2053 (column 2, row 5), lum 0.5: u = (510 + 127.5) / 1024 is a texel centre (637).
        assert_texel(hue_lookup(&atlas, 2053, 0.5, client), 637.0, 4.5);

        let intended = HueAtlasSampling::IntendedEntry;
        assert_texel(hue_lookup(&atlas, 1, 0.0, intended), 0.0, 1.0);
        assert_texel(hue_lookup(&atlas, 1025, 1.0, intended), 511.0, 1.0);
        assert_texel(hue_lookup(&atlas, 2053, 0.5, intended), 640.0, 5.0);

        let rendered = [0.0, 1.0, 0.0, 0.5];
        let hued = hued_color(rendered, 0.5, 2053, &atlas, intended);
        let expected = hue_lookup(&atlas, 2053, luminance([0.0, 1.0, 0.0]), intended);
        assert_eq!(hued[..3], expected[..3]);
        assert_eq!(hued[ALPHA], 0.25, "rendered alpha x hue alpha x vertex alpha");
        assert_eq!(hued_color(rendered, 1.0, NO_HUE, &atlas, intended), rendered);
    }

    #[test]
    fn hued_effects_run_at_half_speed() {
        let duration = 1.0;
        let run = |scale: f32| {
            let mut instance = NifEffectInstance::with_seed(Arc::new(NifEffect::prepare(NifEffectDesc::default())), 0);
            instance.set_time_scale(scale);
            instance.start(duration, false);
            instance.set_lifetime_ticks(Some(u32::MAX));
            let step_ms = 100;
            let mut updates = 0;
            while !instance.is_finished() {
                instance.update(step_ms, &NodeTransform::default());
                updates += 1;
            }
            updates
        };
        let (plain, hued) = (run(1.0), run(HUED_EFFECT_TIME_SCALE));
        assert_eq!(plain, 10);
        assert_eq!(hued, 2 * plain, "a non-looping hued effect lasts twice as long");
    }

    /// One opaque white sprite at the origin.
    fn frame_with_one_sprite(size: f32) -> RenderFrame {
        let mut frame = RenderFrame::default();
        frame.sprites.push(SpriteInstance {
            position: Vec3::ZERO,
            size,
            rotation: 0.0,
            color: [1.0; 4],
            uv: UvRect::FULL,
            direction: Vec3::ZERO,
            length: 0.0,
        });
        frame.batches.push(DrawBatch {
            texture: Arc::from(""),
            blend: OPAQUE_BLEND,
            depth: PARTICLE_DEPTH_STATE,
            two_sided: false,
            kind: SpriteKind::Billboard,
            sprites: 0..1,
            sort: SortKey::sorted(Vec3::ZERO),
        });
        frame
    }

    #[test]
    fn hued_frames_draw_the_hue_colour_through_the_scaled_quad() {
        const SIDE: usize = 64;
        const BACKGROUND: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
        let hue_color = [0.25, 0.5, 0.75, 1.0];
        let atlas = RasterTexture::from_texels(ATLAS_SIDE, ATLAS_SIDE, vec![hue_color; ATLAS_SIDE * ATLAS_SIDE]).unwrap();
        let pixels_per_unit = 4.0;
        let view = OrthoView::client_view(Vec3::ZERO, 100.0, pixels_per_unit, SIDE, SIDE);
        let settings = HuedRenderSettings { aspect: 1.0, sampling: HueAtlasSampling::Client };
        let sprite_size = 4.0;
        let draw = |frame: &RenderFrame| {
            let mut canvas = Canvas::new(SIDE, SIDE, BACKGROUND);
            draw_hued_frame(&mut canvas, &view, frame, &|_| None, &SceneDrawOptions::default(), Vec3::ZERO, 0x21, &atlas, &settings);
            canvas
        };
        let canvas = draw(&frame_with_one_sprite(sprite_size));
        let at = |x: f32, z: f32| {
            let (px, py) = view.to_screen(Vec3::new(x, 0.0, z));
            canvas.pixels[py as usize * SIDE + px as usize]
        };
        assert_eq!(at(0.0, 0.0), hue_color);
        // The 4-unit sprite fills the target from -2 to 2; the quad shows it 4.854 / 4 times larger.
        let shown_half = sprite_size * 0.5 * HUED_QUAD_HALF_SIZE / HUED_FRUSTUM_HALF_HEIGHT;
        let inside = shown_half - 1.0 / pixels_per_unit;
        let outside = shown_half + 1.0 / pixels_per_unit;
        assert_eq!(at(inside, 0.0), hue_color, "local +x is drawn right");
        assert_eq!(at(0.0, -inside), hue_color);
        assert_eq!(at(outside, 0.0), BACKGROUND, "the transparent target fails the alpha test");
        assert_eq!(at(0.0, outside), BACKGROUND);
        // Beyond the far plane the target stays empty.
        let mut far = frame_with_one_sprite(sprite_size);
        far.sprites[0].position = Vec3::Y * (HUED_CAMERA_FAR - HUED_CAMERA_DISTANCE + 1.0);
        assert!(draw(&far).pixels.iter().all(|&pixel| pixel == BACKGROUND));
    }
}
