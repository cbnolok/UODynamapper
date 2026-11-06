//! Conversion helpers for deriving `multimap.rle` drawings from RGBA map renders.

crate::eyre_imports!();

use crate::classic::multimap_rle::{self, MultimapRleImage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultimapStyle {
    /// First-pass edge extraction from the source render.
    Edge,
    /// Symbolic hand-drawn style closer to Classic Client `multimap.rle`.
    Classic,
}

#[derive(Debug, Clone)]
pub struct DecodedRgba {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl DecodedRgba {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> eyre::Result<Self> {
        let expected_len = output_len(width, height)?
            .checked_mul(4)
            .ok_or_else(|| eyre::eyre!("RGBA byte length overflows for {}x{}", width, height))?;
        if rgba.len() != expected_len {
            eyre::bail!(
                "RGBA buffer has {} bytes, expected {} for {}x{}",
                rgba.len(),
                expected_len,
                width,
                height
            );
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageRenderOptions {
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: Option<u32>,
    pub source_height: Option<u32>,
    pub output_width: Option<u32>,
    pub output_height: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct MultimapRenderOptions {
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: Option<u32>,
    pub source_height: Option<u32>,
    pub output_width: u32,
    pub output_height: u32,
    pub edge_threshold: u16,
    pub line_radius: u32,
    pub style: MultimapStyle,
}

impl Default for MultimapRenderOptions {
    fn default() -> Self {
        Self {
            source_x: 0,
            source_y: 0,
            source_width: None,
            source_height: None,
            output_width: multimap_rle::DEFAULT_WIDTH,
            output_height: multimap_rle::DEFAULT_HEIGHT,
            edge_threshold: 28,
            line_radius: 0,
            style: MultimapStyle::Classic,
        }
    }
}

pub struct RenderedRgba {
    pub width: u32,
    pub height: u32,
    pub crop: SourceRect,
    pub rgba: Vec<u8>,
}

pub struct RenderedMultimap {
    pub image: MultimapRleImage,
    pub crop: SourceRect,
}

pub fn render_source_image(
    source: &DecodedRgba,
    options: ImageRenderOptions,
) -> eyre::Result<RenderedRgba> {
    let crop = explicit_or_remaining_rect(
        source.width,
        source.height,
        options.source_x,
        options.source_y,
        options.source_width,
        options.source_height,
    )?;
    let output_width = options.output_width.unwrap_or(crop.width);
    let output_height = options.output_height.unwrap_or(crop.height);
    let rgba = resample_rgba(source, crop, output_width, output_height)?;
    Ok(RenderedRgba {
        width: output_width,
        height: output_height,
        crop,
        rgba,
    })
}

pub fn render_multimap(
    source: &DecodedRgba,
    options: MultimapRenderOptions,
) -> eyre::Result<RenderedMultimap> {
    let crop = multimap_default_rect(
        source.width,
        source.height,
        options.source_x,
        options.source_y,
        options.source_width,
        options.source_height,
        options.output_width,
        options.output_height,
    )?;
    let samples = resample_rgb(source, crop, options.output_width, options.output_height)?;
    let pixels = match options.style {
        MultimapStyle::Edge => edge_multimap_pixels(
            &samples,
            options.output_width,
            options.output_height,
            options.edge_threshold,
            options.line_radius,
        )?,
        MultimapStyle::Classic => classic_multimap_pixels(
            &samples,
            options.output_width,
            options.output_height,
            options.edge_threshold,
            options.line_radius,
        )?,
    };
    let image = MultimapRleImage::new(options.output_width, options.output_height, pixels)?;
    Ok(RenderedMultimap { image, crop })
}

pub fn explicit_or_remaining_rect(
    image_width: u32,
    image_height: u32,
    x: u32,
    y: u32,
    width: Option<u32>,
    height: Option<u32>,
) -> eyre::Result<SourceRect> {
    let remaining_width = image_width
        .checked_sub(x)
        .ok_or_else(|| eyre::eyre!("source-x {} is outside source width {}", x, image_width))?;
    let remaining_height = image_height
        .checked_sub(y)
        .ok_or_else(|| eyre::eyre!("source-y {} is outside source height {}", y, image_height))?;
    checked_rect(
        image_width,
        image_height,
        x,
        y,
        width.unwrap_or(remaining_width),
        height.unwrap_or(remaining_height),
    )
}

pub fn multimap_default_rect(
    image_width: u32,
    image_height: u32,
    x: u32,
    y: u32,
    width: Option<u32>,
    height: Option<u32>,
    output_width: u32,
    output_height: u32,
) -> eyre::Result<SourceRect> {
    let remaining_width = image_width
        .checked_sub(x)
        .ok_or_else(|| eyre::eyre!("source-x {} is outside source width {}", x, image_width))?;
    let remaining_height = image_height
        .checked_sub(y)
        .ok_or_else(|| eyre::eyre!("source-y {} is outside source height {}", y, image_height))?;
    let default_width = output_width.saturating_mul(2).min(remaining_width);
    let default_height = output_height.saturating_mul(2).min(remaining_height);
    checked_rect(
        image_width,
        image_height,
        x,
        y,
        width.unwrap_or(default_width),
        height.unwrap_or(default_height),
    )
}

fn checked_rect(
    image_width: u32,
    image_height: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> eyre::Result<SourceRect> {
    if width == 0 || height == 0 {
        eyre::bail!("source crop dimensions must be non-zero, got {}x{}", width, height);
    }
    let end_x = x
        .checked_add(width)
        .ok_or_else(|| eyre::eyre!("source crop X range overflows"))?;
    let end_y = y
        .checked_add(height)
        .ok_or_else(|| eyre::eyre!("source crop Y range overflows"))?;
    if end_x > image_width || end_y > image_height {
        eyre::bail!(
            "source crop {}x{}+{},{} exceeds source dimensions {}x{}",
            width,
            height,
            x,
            y,
            image_width,
            image_height
        );
    }
    Ok(SourceRect {
        x,
        y,
        width,
        height,
    })
}

pub fn resample_rgba(
    image: &DecodedRgba,
    rect: SourceRect,
    output_width: u32,
    output_height: u32,
) -> eyre::Result<Vec<u8>> {
    let output_pixels = output_len(output_width, output_height)?;
    let mut output = vec![0u8; output_pixels * 4];
    for y in 0..output_height {
        let (source_y0, source_y1) = source_span(rect.y, rect.height, y, output_height);
        for x in 0..output_width {
            let (source_x0, source_x1) = source_span(rect.x, rect.width, x, output_width);
            let [r, g, b, a] = average_rgba(image, source_x0, source_y0, source_x1, source_y1);
            let index = ((y * output_width + x) as usize) * 4;
            output[index] = r;
            output[index + 1] = g;
            output[index + 2] = b;
            output[index + 3] = a;
        }
    }
    Ok(output)
}

fn resample_rgb(
    image: &DecodedRgba,
    rect: SourceRect,
    output_width: u32,
    output_height: u32,
) -> eyre::Result<Vec<[u8; 3]>> {
    let output_pixels = output_len(output_width, output_height)?;
    let mut output = Vec::with_capacity(output_pixels);
    for y in 0..output_height {
        let (source_y0, source_y1) = source_span(rect.y, rect.height, y, output_height);
        for x in 0..output_width {
            let (source_x0, source_x1) = source_span(rect.x, rect.width, x, output_width);
            let [r, g, b, _] = average_rgba(image, source_x0, source_y0, source_x1, source_y1);
            output.push([r, g, b]);
        }
    }
    Ok(output)
}

fn source_span(offset: u32, length: u32, output: u32, output_length: u32) -> (u32, u32) {
    let start = offset + ((u64::from(output) * u64::from(length)) / u64::from(output_length)) as u32;
    let end = offset
        + (((u64::from(output + 1) * u64::from(length)) + u64::from(output_length - 1))
            / u64::from(output_length)) as u32;
    (start, end.max(start + 1))
}

fn average_rgba(image: &DecodedRgba, x0: u32, y0: u32, x1: u32, y1: u32) -> [u8; 4] {
    let mut r = 0u64;
    let mut g = 0u64;
    let mut b = 0u64;
    let mut a = 0u64;
    let mut count = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let index = ((y * image.width + x) as usize) * 4;
            r += u64::from(image.rgba[index]);
            g += u64::from(image.rgba[index + 1]);
            b += u64::from(image.rgba[index + 2]);
            a += u64::from(image.rgba[index + 3]);
            count += 1;
        }
    }
    [
        (r / count) as u8,
        (g / count) as u8,
        (b / count) as u8,
        (a / count) as u8,
    ]
}

fn edge_multimap_pixels(
    samples: &[[u8; 3]],
    width: u32,
    height: u32,
    edge_threshold: u16,
    line_radius: u32,
) -> eyre::Result<Vec<u8>> {
    let pixel_count = output_len(width, height)?;
    if samples.len() != pixel_count {
        eyre::bail!(
            "sample buffer has {} pixels, expected {} for {}x{}",
            samples.len(),
            pixel_count,
            width,
            height
        );
    }

    let mut edges = vec![false; pixel_count];
    for y in 0..height {
        for x in 0..width {
            let edge = edge_strength(samples, width, height, x, y);
            edges[(y * width + x) as usize] = edge >= edge_threshold;
        }
    }

    let mut pixels = vec![multimap_rle::WHITE_PIXEL; pixel_count];
    for y in 0..height {
        for x in 0..width {
            if !edges[(y * width + x) as usize] {
                continue;
            }
            ink(&mut pixels, width, height, x, y, line_radius);
        }
    }
    Ok(pixels)
}

fn classic_multimap_pixels(
    samples: &[[u8; 3]],
    width: u32,
    height: u32,
    edge_threshold: u16,
    line_radius: u32,
) -> eyre::Result<Vec<u8>> {
    let pixel_count = output_len(width, height)?;
    if samples.len() != pixel_count {
        eyre::bail!(
            "sample buffer has {} pixels, expected {} for {}x{}",
            samples.len(),
            pixel_count,
            width,
            height
        );
    }

    let mut water = vec![false; pixel_count];
    for (index, &rgb) in samples.iter().enumerate() {
        water[index] = is_water(rgb);
    }

    let mut coast = vec![false; pixel_count];
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let here_water = water[index];
            let min_x = x.saturating_sub(1);
            let max_x = (x + 1).min(width - 1);
            let min_y = y.saturating_sub(1);
            let max_y = (y + 1).min(height - 1);
            for ny in min_y..=max_y {
                for nx in min_x..=max_x {
                    if nx == x && ny == y {
                        continue;
                    }
                    if water[(ny * width + nx) as usize] != here_water {
                        coast[index] = true;
                    }
                }
            }
        }
    }

    let mut pixels = vec![multimap_rle::WHITE_PIXEL; pixel_count];
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let rgb = samples[index];
            let edge = edge_strength(samples, width, height, x, y);

            if coast[index] {
                ink(&mut pixels, width, height, x, y, line_radius);
                continue;
            }

            if water[index] {
                if near_coast(&coast, width, height, x, y, 3) && hash2(x, y, 0x71) % 7 == 0 {
                    ink(&mut pixels, width, height, x, y, 0);
                }
                continue;
            }

            if edge >= edge_threshold.saturating_mul(2) && hash2(x, y, 0x19) % 3 != 0 {
                ink(&mut pixels, width, height, x, y, 0);
                continue;
            } else if edge >= edge_threshold && hash2(x, y, 0x41) % 4 == 0 {
                ink(&mut pixels, width, height, x, y, 0);
                continue;
            }

            match terrain_mark(rgb) {
                TerrainMark::Mountain => {
                    if mountain_hatch(x, y) && edge >= edge_threshold / 2 {
                        ink(&mut pixels, width, height, x, y, 0);
                    } else if hash2(x, y, 0x53) % 89 == 0 {
                        ink(&mut pixels, width, height, x, y, 0);
                    }
                }
                TerrainMark::Forest => {
                    if forest_dot(x, y) {
                        ink(&mut pixels, width, height, x, y, 0);
                    }
                }
                TerrainMark::Dry => {
                    if dry_dash(x, y) {
                        ink(&mut pixels, width, height, x, y, 0);
                    }
                }
                TerrainMark::Open => {
                    if edge >= edge_threshold && hash2(x, y, 0x2d) % 5 == 0 {
                        ink(&mut pixels, width, height, x, y, 0);
                    } else if hash2(x, y, 0x3b) % 137 == 0 {
                        ink(&mut pixels, width, height, x, y, 0);
                    }
                }
            }
        }
    }

    Ok(pixels)
}

#[derive(Clone, Copy)]
enum TerrainMark {
    Open,
    Mountain,
    Forest,
    Dry,
}

fn is_water(rgb: [u8; 3]) -> bool {
    let [r, g, b] = rgb;
    let blue_dominant = u16::from(b) > u16::from(r) + 12 && u16::from(b) > u16::from(g) + 6;
    let dark_blue = b > 40 && r < 95 && g < 120 && b >= g;
    blue_dominant || dark_blue
}

fn terrain_mark(rgb: [u8; 3]) -> TerrainMark {
    let [r, g, b] = rgb;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let saturation = max - min;
    let brightness = ((u16::from(r) + u16::from(g) + u16::from(b)) / 3) as u8;

    if saturation < 30 && brightness < 142 {
        TerrainMark::Mountain
    } else if g > r.saturating_add(7) && g > b.saturating_add(5) && brightness < 170 {
        TerrainMark::Forest
    } else if r > b.saturating_add(16) && g > b.saturating_add(10) && brightness > 92 {
        TerrainMark::Dry
    } else {
        TerrainMark::Open
    }
}

fn mountain_hatch(x: u32, y: u32) -> bool {
    let cell_x = x / 8;
    let cell_y = y / 7;
    let local_x = x % 8;
    let local_y = y % 7;
    if hash2(cell_x, cell_y, 0xa5) % 4 == 0 {
        return false;
    }
    let offset = hash2(cell_x, cell_y, 0xb3) % 2;
    local_y == local_x / 2 + offset || local_y == (7 - local_x) / 2 + offset
}

fn forest_dot(x: u32, y: u32) -> bool {
    let cell_x = x / 3;
    let cell_y = y / 3;
    let local_x = x % 3;
    let local_y = y % 3;
    hash2(cell_x, cell_y, 0xc7) % 9 == 0 && (local_x == 1 || local_y == 1)
}

fn dry_dash(x: u32, y: u32) -> bool {
    let cell_x = x / 5;
    let cell_y = y / 4;
    let local_x = x % 5;
    let local_y = y % 4;
    hash2(cell_x, cell_y, 0xdd) % 13 == 0 && local_y == 1 && local_x <= 2
}

fn near_coast(coast: &[bool], width: u32, height: u32, x: u32, y: u32, radius: u32) -> bool {
    let min_x = x.saturating_sub(radius);
    let max_x = (x + radius).min(width - 1);
    let min_y = y.saturating_sub(radius);
    let max_y = (y + radius).min(height - 1);
    for ny in min_y..=max_y {
        for nx in min_x..=max_x {
            if coast[(ny * width + nx) as usize] {
                return true;
            }
        }
    }
    false
}

fn ink(pixels: &mut [u8], width: u32, height: u32, x: u32, y: u32, radius: u32) {
    let min_x = x.saturating_sub(radius);
    let max_x = (x + radius).min(width - 1);
    let min_y = y.saturating_sub(radius);
    let max_y = (y + radius).min(height - 1);
    for ink_y in min_y..=max_y {
        for ink_x in min_x..=max_x {
            pixels[(ink_y * width + ink_x) as usize] = multimap_rle::BLACK_PIXEL;
        }
    }
}

fn hash2(x: u32, y: u32, seed: u32) -> u32 {
    let mut value = x
        .wrapping_mul(0x9e37_79b1)
        .wrapping_add(y.wrapping_mul(0x85eb_ca6b))
        .wrapping_add(seed.wrapping_mul(0xc2b2_ae35));
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

fn edge_strength(samples: &[[u8; 3]], width: u32, height: u32, x: u32, y: u32) -> u16 {
    let left = x.saturating_sub(1);
    let right = (x + 1).min(width - 1);
    let top = y.saturating_sub(1);
    let bottom = (y + 1).min(height - 1);

    let tl = luma(samples[(top * width + left) as usize]);
    let tc = luma(samples[(top * width + x) as usize]);
    let tr = luma(samples[(top * width + right) as usize]);
    let ml = luma(samples[(y * width + left) as usize]);
    let mr = luma(samples[(y * width + right) as usize]);
    let bl = luma(samples[(bottom * width + left) as usize]);
    let bc = luma(samples[(bottom * width + x) as usize]);
    let br = luma(samples[(bottom * width + right) as usize]);

    let gx = -tl + tr - 2 * ml + 2 * mr - bl + br;
    let gy = -tl - 2 * tc - tr + bl + 2 * bc + br;
    let luma_edge = ((gx.abs() + gy.abs()) / 8) as u16;
    luma_edge.max(color_edge(samples, width, height, x, y))
}

fn color_edge(samples: &[[u8; 3]], width: u32, height: u32, x: u32, y: u32) -> u16 {
    let here = samples[(y * width + x) as usize];
    let mut strongest = 0u16;
    if x + 1 < width {
        strongest = strongest.max(color_distance(here, samples[(y * width + x + 1) as usize]));
    }
    if y + 1 < height {
        strongest = strongest.max(color_distance(here, samples[((y + 1) * width + x) as usize]));
    }
    strongest
}

fn color_distance(a: [u8; 3], b: [u8; 3]) -> u16 {
    let dr = i16::from(a[0]) - i16::from(b[0]);
    let dg = i16::from(a[1]) - i16::from(b[1]);
    let db = i16::from(a[2]) - i16::from(b[2]);
    ((dr.abs() + dg.abs() + db.abs()) / 3) as u16
}

fn luma(rgb: [u8; 3]) -> i32 {
    (i32::from(rgb[0]) * 299 + i32::from(rgb[1]) * 587 + i32::from(rgb[2]) * 114) / 1000
}

fn output_len(width: u32, height: u32) -> eyre::Result<usize> {
    if width == 0 || height == 0 {
        eyre::bail!("output dimensions must be non-zero, got {}x{}", width, height);
    }
    width
        .checked_mul(height)
        .and_then(|pixels| usize::try_from(pixels).ok())
        .ok_or_else(|| eyre::eyre!("output dimensions are too large: {}x{}", width, height))
}
