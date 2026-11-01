//! Lossy WebP through libwebp (`libwebp-sys` bundles libwebp 1.6.0 and builds it with `cc`).

use color_eyre::eyre;
use libwebp_sys as ffi;

/// The user's settings (2026-11-01): method 4, quality 85, sharp YUV. Measured on 11 renderer crops: mean
/// SSIMULACRA2 74.9, minimum 65.7, 1.57 bits per pixel, 26 ms per 512² tile.
pub const DEFAULT_QUALITY: f32 = 85.0;
pub const DEFAULT_METHOD: i32 = 4;
pub const DEFAULT_SHARP_YUV: bool = true;
pub const MAX_QUALITY: f32 = 100.0;
pub const MAX_METHOD: i32 = 6;
const RGB_CHANNELS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WebpSettings {
    pub quality: f32,
    pub method: i32,
    pub sharp_yuv: bool,
}

impl Default for WebpSettings {
    fn default() -> Self {
        Self { quality: DEFAULT_QUALITY, method: DEFAULT_METHOD, sharp_yuv: DEFAULT_SHARP_YUV }
    }
}

impl WebpSettings {
    pub fn validate(self) -> Result<(), String> {
        if !(0.0..=MAX_QUALITY).contains(&self.quality) {
            return Err(format!("WebP quality {} is outside 0..={MAX_QUALITY}", self.quality));
        }
        if !(0..=MAX_METHOD).contains(&self.method) {
            return Err(format!("WebP method {} is outside 0..={MAX_METHOD}", self.method));
        }
        Ok(())
    }
}

/// Frees the picture's buffers on every path.
struct Picture(ffi::WebPPicture);

impl Drop for Picture {
    fn drop(&mut self) {
        // SAFETY: the picture was initialised by WebPPictureInit; freeing an empty picture is allowed.
        unsafe { ffi::WebPPictureFree(&mut self.0) }
    }
}

/// Frees the memory writer's buffer on every path.
struct Writer(ffi::WebPMemoryWriter);

impl Drop for Writer {
    fn drop(&mut self) {
        // SAFETY: the writer was initialised by WebPMemoryWriterInit.
        unsafe { ffi::WebPMemoryWriterClear(&mut self.0) }
    }
}

/// Encodes tightly packed RGB8 rows, top to bottom.
pub fn encode_rgb(rgb: &[u8], width: u32, height: u32, settings: WebpSettings) -> eyre::Result<Vec<u8>> {
    settings.validate().map_err(|error| eyre::eyre!(error))?;
    let stride = width as usize * RGB_CHANNELS;
    if rgb.len() != stride * height as usize {
        eyre::bail!("RGB buffer of {} bytes is not {width}x{height}", rgb.len());
    }
    let mut config = ffi::WebPConfig::new().map_err(|()| eyre::eyre!("libwebp config version mismatch"))?;
    config.quality = settings.quality;
    config.method = settings.method;
    config.use_sharp_yuv = i32::from(settings.sharp_yuv);
    // SAFETY: config is a valid, initialised WebPConfig.
    if unsafe { ffi::WebPValidateConfig(&config) } == 0 {
        eyre::bail!("libwebp rejected the configuration");
    }
    let mut picture = Picture(ffi::WebPPicture::new().map_err(|()| eyre::eyre!("libwebp picture version mismatch"))?);
    // ARGB input: the RGB to YUV conversion then happens in WebPEncode, where use_sharp_yuv applies.
    picture.0.use_argb = 1;
    picture.0.width = i32::try_from(width)?;
    picture.0.height = i32::try_from(height)?;
    // SAFETY: rgb holds height rows of `stride` bytes, checked above.
    if unsafe { ffi::WebPPictureImportRGB(&mut picture.0, rgb.as_ptr(), i32::try_from(stride)?) } == 0 {
        eyre::bail!("libwebp could not import the picture (error {:?})", picture.0.error_code);
    }
    let mut writer = std::mem::MaybeUninit::<ffi::WebPMemoryWriter>::uninit();
    // SAFETY: WebPMemoryWriterInit initialises every field.
    let mut writer = Writer(unsafe {
        ffi::WebPMemoryWriterInit(writer.as_mut_ptr());
        writer.assume_init()
    });
    picture.0.writer = Some(ffi::WebPMemoryWrite);
    picture.0.custom_ptr = (&mut writer.0 as *mut ffi::WebPMemoryWriter).cast();
    // SAFETY: config and picture are valid; the writer outlives the call.
    if unsafe { ffi::WebPEncode(&config, &mut picture.0) } == 0 {
        eyre::bail!("libwebp could not encode (error {:?})", picture.0.error_code);
    }
    // SAFETY: the writer holds `size` bytes at `mem` after a successful encode.
    Ok(unsafe { std::slice::from_raw_parts(writer.0.mem, writer.0.size) }.to_vec())
}

/// Decodes a WebP to RGB8, returning width, height and pixels.
pub fn decode_rgb(data: &[u8]) -> eyre::Result<(u32, u32, Vec<u8>)> {
    let (mut width, mut height) = (0, 0);
    // SAFETY: data is a valid slice; libwebp writes the dimensions on success.
    let pixels = unsafe { ffi::WebPDecodeRGB(data.as_ptr(), data.len(), &mut width, &mut height) };
    if pixels.is_null() {
        eyre::bail!("libwebp could not decode {} bytes", data.len());
    }
    let len = width as usize * height as usize * RGB_CHANNELS;
    // SAFETY: on success libwebp returns width * height RGB pixels, freed with WebPFree.
    let rgb = unsafe {
        let rgb = std::slice::from_raw_parts(pixels, len).to_vec();
        ffi::WebPFree(pixels.cast());
        rgb
    };
    Ok((u32::try_from(width)?, u32::try_from(height)?, rgb))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: u32 = 64;

    #[test]
    fn encodes_and_decodes_a_gradient() {
        let rgb: Vec<u8> = (0..SIZE * SIZE).flat_map(|i| [(i % SIZE * 4) as u8, (i / SIZE * 4) as u8, 128]).collect();
        let data = encode_rgb(&rgb, SIZE, SIZE, WebpSettings::default()).unwrap();
        assert_eq!(&data[..4], b"RIFF");
        let (w, h, back) = decode_rgb(&data).unwrap();
        assert_eq!((w, h), (SIZE, SIZE));
        let max_error = rgb.iter().zip(&back).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(max_error < 32, "{max_error}");
    }

    #[test]
    fn rejects_bad_settings_and_sizes() {
        let rgb = vec![0; (SIZE * SIZE * 3) as usize];
        assert!(encode_rgb(&rgb, SIZE, SIZE + 1, WebpSettings::default()).is_err());
        assert!(encode_rgb(&rgb, SIZE, SIZE, WebpSettings { quality: 101.0, ..Default::default() }).is_err());
        assert!(encode_rgb(&rgb, SIZE, SIZE, WebpSettings { method: 7, ..Default::default() }).is_err());
    }
}
