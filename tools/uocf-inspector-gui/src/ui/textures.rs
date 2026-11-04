//! Checked creation of egui textures: every texture of the tool goes through here.
//!
//! The renderer treats a bad texture as fatal. wgpu's default error handler panics on a zero-sized texture or one
//! above the GPU limit ("Device::create_texture ... Dimension X is zero"), and
//! `ColorImage::from_rgba_unmultiplied` panics when the pixel buffer does not match the size. Two things produced
//! such images:
//! - The data: `Interface.uop` and the Kingdom Reborn `TileArtLegacy.uop` ship 18-byte header-only 0x0 TGAs.
//! - Our reading: `gumpartLegacyMUL.uop` and `artLegacyMUL.uop` entries are named `.tga` but hold the classic encoding.
//!   `decode_tga_rgba8` returns an empty image for any payload with a zero 16-bit word at the TGA's width or height,
//!   which is most gumps and part of the art. See `decode_classic_uop_entry`.
//!
//! So a size is checked here, before anything reaches the GPU, and the caller gets a readable message to show instead.

use eframe::egui;
use std::sync::{Arc, Mutex};

const RGBA_BYTES_PER_PIXEL: usize = 4;
/// Distinct GPU errors kept for the log: a bad resource can repeat its error every frame.
const MAX_REPORTED_GPU_ERRORS: usize = 16;
const CLASSIC_GUMP_ENTRY_FOLDER: &str = "build/gumpartlegacymul/";
const CLASSIC_ART_ENTRY_FOLDER: &str = "build/artlegacymul/";
const TGA_EXTENSION: &str = ".tga";
const LAND_TILE_SIDE: usize = 44;

/// Decodes an entry of `gumpartLegacyMUL.uop` or `artLegacyMUL.uop` (`name` in lower case) to width, height and RGBA.
/// `None` for any other entry. Their names end in `.tga`, but they keep the classic encoding.
pub(crate) fn decode_classic_uop_entry(name: &str, data: &[u8]) -> Option<Result<(u32, u32, Vec<u8>), String>> {
    if name.starts_with(CLASSIC_GUMP_ENTRY_FOLDER) {
        let decoded = uocf::classic::gump::decode_uop_gump(data);
        return Some(decoded.map(|(width, height, pixels)| (width as u32, height as u32, pixels)).map_err(|error| format!("{error:#}")));
    }
    let art_id: u32 = name.strip_prefix(CLASSIC_ART_ENTRY_FOLDER)?.strip_suffix(TGA_EXTENSION)?.parse().ok()?;
    if art_id < uocf::classic::art::STATIC_TILE_ID_BASE {
        let mut pixels = [0u8; LAND_TILE_SIDE * LAND_TILE_SIDE * RGBA_BYTES_PER_PIXEL];
        let decoded = uocf::classic::art::decode_land_tile_from_raw(data, &mut pixels);
        return Some(
            decoded
                .map(|()| (LAND_TILE_SIDE as u32, LAND_TILE_SIDE as u32, pixels.to_vec()))
                .map_err(|error| format!("{error:#}")),
        );
    }
    let decoded = uocf::classic::art::decode_static_tile_from_raw(data);
    Some(decoded.map(|(width, height, pixels)| (width as u32, height as u32, pixels)).map_err(|error| format!("{error:#}")))
}

/// Why an RGBA image of `size` with `byte_len` bytes cannot become a texture.
pub(crate) fn rgba_image_problem(size: [usize; 2], byte_len: usize, max_side: usize) -> Option<String> {
    let [width, height] = size;
    if width == 0 || height == 0 {
        return Some(format!("the image is empty ({width} x {height} pixels)"));
    }
    if width > max_side || height > max_side {
        return Some(format!("the image is {width} x {height} pixels, above the {max_side} pixels per side of this GPU"));
    }
    let expected = width * height * RGBA_BYTES_PER_PIXEL;
    (byte_len != expected).then(|| format!("a {width} x {height} image has {byte_len} bytes of pixels, not {expected}"))
}

/// The largest texture side of the renderer.
fn max_texture_side(ctx: &egui::Context) -> usize {
    ctx.input(|input| input.max_texture_side)
}

/// Creates a texture from unmultiplied RGBA pixels, or says why the image cannot be one.
pub(crate) fn load_rgba(
    ctx: &egui::Context,
    name: impl Into<String>,
    size: [usize; 2],
    rgba: &[u8],
    options: egui::TextureOptions,
) -> Result<egui::TextureHandle, String> {
    match rgba_image_problem(size, rgba.len(), max_texture_side(ctx)) {
        Some(problem) => Err(problem),
        None => Ok(ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied(size, rgba), options)),
    }
}

/// Creates a texture from an image built elsewhere (a worker thread), or says why the image cannot be one.
pub(crate) fn load_color_image(
    ctx: &egui::Context,
    name: impl Into<String>,
    image: egui::ColorImage,
    options: egui::TextureOptions,
) -> Result<egui::TextureHandle, String> {
    let byte_len = image.pixels.len().saturating_mul(RGBA_BYTES_PER_PIXEL);
    match rgba_image_problem(image.size, byte_len, max_texture_side(ctx)) {
        Some(problem) => Err(problem),
        None => Ok(ctx.load_texture(name, image, options)),
    }
}

/// Keeps the GPU errors the checks above could not foresee, for the log: without this, wgpu's default handler panics
/// on the first one. The UI goes on, though a resource that failed to create can still lose the frames that use it.
#[derive(Clone, Default)]
pub(crate) struct GpuErrors {
    pending: Arc<Mutex<GpuErrorLog>>,
}

#[derive(Default)]
struct GpuErrorLog {
    seen: Vec<String>,
    unread: Vec<String>,
}

impl GpuErrors {
    /// Replaces wgpu's panicking handler of the device of `render_state`.
    pub(crate) fn install(&self, render_state: &eframe::egui_wgpu::RenderState) {
        let errors = self.clone();
        render_state.device.on_uncaptured_error(Arc::new(move |error| errors.record(error.to_string())));
    }

    fn record(&self, message: String) {
        let Ok(mut log) = self.pending.lock() else {
            return;
        };
        if log.seen.len() < MAX_REPORTED_GPU_ERRORS && !log.seen.contains(&message) {
            log.seen.push(message.clone());
            log.unread.push(message);
        }
    }

    /// The errors recorded since the last call.
    pub(crate) fn take_unread(&self) -> Vec<String> {
        self.pending.lock().map(|mut log| std::mem::take(&mut log.unread)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_SIDE: usize = 8192;
    const LAND_DIAMOND_BYTES: usize = 2048;

    #[test]
    fn a_gump_entry_is_decoded_as_a_gump_though_named_tga() {
        // 1x2: the width, the height, two row offsets in dwords, and one run of one white pixel per row.
        let payload: Vec<u8> = [1u32, 2, 2, 3, 0x0001_7FFF, 0x0001_7FFF].iter().flat_map(|word| word.to_le_bytes()).collect();
        let (width, height, pixels) =
            decode_classic_uop_entry("build/gumpartlegacymul/00000001.tga", &payload).unwrap().unwrap();
        assert_eq!((width, height), (1, 2));
        assert_eq!(pixels.len(), 2 * RGBA_BYTES_PER_PIXEL);
    }

    #[test]
    fn an_art_entry_is_decoded_as_a_tile_of_the_kind_its_id_says() {
        let land = vec![0u8; LAND_DIAMOND_BYTES];
        let (width, height, _) = decode_classic_uop_entry("build/artlegacymul/00000003.tga", &land).unwrap().unwrap();
        assert_eq!((width as usize, height as usize), (LAND_TILE_SIDE, LAND_TILE_SIDE));
        let static_with_a_land_payload = decode_classic_uop_entry("build/artlegacymul/00016384.tga", &land).unwrap();
        assert!(static_with_a_land_payload.is_err(), "a static of width 0 is refused, not an empty image");
    }

    #[test]
    fn other_entries_are_left_to_the_image_decoders() {
        assert!(decode_classic_uop_entry("build/legacytexture/00000001.tga", &[]).is_none());
        assert!(decode_classic_uop_entry("build/artlegacymul/notes.txt", &[]).is_none());
    }

    #[test]
    fn an_empty_image_is_refused_on_either_side() {
        assert!(rgba_image_problem([0, 0], 0, MAX_SIDE).is_some_and(|problem| problem.contains("empty")));
        assert!(rgba_image_problem([0, 4], 0, MAX_SIDE).is_some());
        assert!(rgba_image_problem([4, 0], 0, MAX_SIDE).is_some());
    }

    #[test]
    fn an_image_above_the_gpu_limit_is_refused() {
        let problem = rgba_image_problem([MAX_SIDE + 1, 1], (MAX_SIDE + 1) * RGBA_BYTES_PER_PIXEL, MAX_SIDE);
        assert!(problem.is_some_and(|problem| problem.contains("GPU")));
        assert!(rgba_image_problem([1, MAX_SIDE + 1], (MAX_SIDE + 1) * RGBA_BYTES_PER_PIXEL, MAX_SIDE).is_some());
    }

    #[test]
    fn a_pixel_buffer_of_the_wrong_length_is_refused() {
        assert!(rgba_image_problem([2, 2], 15, MAX_SIDE).is_some());
        assert!(rgba_image_problem([2, 2], 17, MAX_SIDE).is_some());
        assert_eq!(rgba_image_problem([2, 2], 16, MAX_SIDE), None);
    }

    #[test]
    fn a_context_gets_no_texture_for_a_bad_image_and_does_not_panic() {
        let ctx = egui::Context::default();
        assert!(load_rgba(&ctx, "empty", [0, 0], &[], egui::TextureOptions::NEAREST).is_err());
        assert!(load_rgba(&ctx, "short", [2, 2], &[0; 4], egui::TextureOptions::NEAREST).is_err());
        assert!(load_rgba(&ctx, "good", [1, 1], &[0; 4], egui::TextureOptions::NEAREST).is_ok());
        let empty = egui::ColorImage::from_rgba_unmultiplied([0, 0], &[]);
        assert!(load_color_image(&ctx, "empty image", empty, egui::TextureOptions::NEAREST).is_err());
    }

    #[test]
    fn repeated_gpu_errors_are_reported_once_and_the_list_is_bounded() {
        let errors = GpuErrors::default();
        errors.record("same".to_owned());
        errors.record("same".to_owned());
        assert_eq!(errors.take_unread(), vec!["same".to_owned()]);
        assert!(errors.take_unread().is_empty());
        errors.record("same".to_owned());
        assert!(errors.take_unread().is_empty(), "already reported");
        for index in 0..MAX_REPORTED_GPU_ERRORS * 2 {
            errors.record(format!("error {index}"));
        }
        assert_eq!(errors.take_unread().len(), MAX_REPORTED_GPU_ERRORS - 1);
    }
}
