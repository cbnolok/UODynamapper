//! Post-upscale sharpening passes for pixel-art output.

#[derive(Clone, Copy, Debug)]
pub struct UnsharpMaskParams {
    pub radius: f32,
    pub amount: f32,
}

impl Default for UnsharpMaskParams {
    fn default() -> Self {
        Self {
            radius: 0.8,
            amount: 0.35,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HighPassSharpenParams {
    pub radius: f32,
    pub strength: f32,
}

impl Default for HighPassSharpenParams {
    fn default() -> Self {
        Self {
            radius: 1.0,
            strength: 0.30,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScaleFxSmartDeblurParams {
    pub deblur_offset: f32,
    pub deblur_strength: f32,
    pub smart_deblur: f32,
}

impl Default for ScaleFxSmartDeblurParams {
    fn default() -> Self {
        Self {
            deblur_offset: 2.25,
            deblur_strength: 5.50,
            smart_deblur: 0.05,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LocalLaplacianClarityParams {
    pub radius: u32,
    pub amount: f32,
}

impl Default for LocalLaplacianClarityParams {
    fn default() -> Self {
        Self {
            radius: 3,
            amount: 0.25,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ContrastEnhanceParams {
    pub intensity: f32,
    pub threshold: f32,
    pub blur_spread: f32,
}

impl Default for ContrastEnhanceParams {
    fn default() -> Self {
        Self {
            intensity: 0.35,
            threshold: 0.10,
            blur_spread: 2.5,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AdaptiveLogContrastParams {
    pub radius: f32,
    pub gamma: f32,
}

impl Default for AdaptiveLogContrastParams {
    fn default() -> Self {
        Self {
            radius: 3.0,
            gamma: 0.80,
        }
    }
}

pub fn apply_unsharp_mask(
    width: u32,
    height: u32,
    rgba: &[u8],
    params: UnsharpMaskParams,
) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 || rgba.is_empty() || params.amount <= 0.0 {
        return (width, height, rgba.to_vec());
    }

    let source = rgba_to_rgb_f32(rgba);
    let blurred = gaussian_blur_rgb(width, height, &source, params.radius);
    let mut out = rgba.to_vec();
    for index in 0..source.len() {
        write_rgb(
            &mut out,
            index,
            [
                source[index][0] + (source[index][0] - blurred[index][0]) * params.amount,
                source[index][1] + (source[index][1] - blurred[index][1]) * params.amount,
                source[index][2] + (source[index][2] - blurred[index][2]) * params.amount,
            ],
        );
    }
    (width, height, out)
}

pub fn apply_high_pass_sharpen(
    width: u32,
    height: u32,
    rgba: &[u8],
    params: HighPassSharpenParams,
) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 || rgba.is_empty() || params.strength <= 0.0 {
        return (width, height, rgba.to_vec());
    }

    let source = rgba_to_rgb_f32(rgba);
    let blurred = gaussian_blur_rgb(width, height, &source, params.radius);
    let mut out = rgba.to_vec();
    for index in 0..source.len() {
        let high_pass = [
            source[index][0] - blurred[index][0],
            source[index][1] - blurred[index][1],
            source[index][2] - blurred[index][2],
        ];
        write_rgb(
            &mut out,
            index,
            [
                source[index][0] + high_pass[0] * params.strength,
                source[index][1] + high_pass[1] * params.strength,
                source[index][2] + high_pass[2] * params.strength,
            ],
        );
    }
    (width, height, out)
}

pub fn apply_scalefx_smart_deblur(
    width: u32,
    height: u32,
    rgba: &[u8],
    params: ScaleFxSmartDeblurParams,
) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 || rgba.is_empty() || params.deblur_strength <= 0.0 {
        return (width, height, rgba.to_vec());
    }

    let source = rgba_to_rgb_f32(rgba);
    let blurred = gaussian_blur_rgb(width, height, &source, params.deblur_offset.max(0.5) * 0.35);
    let mut out = rgba.to_vec();
    let amount = (params.deblur_strength * 0.04).clamp(0.0, 0.5);
    let threshold = params.smart_deblur.max(0.0001);

    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let edge = edge_strength(width, height, &source, x as i32, y as i32);
            let mask = smoothstep(threshold, threshold * 4.0, edge);
            if mask <= 0.0 {
                continue;
            }

            let rgb = [
                source[index][0] + (source[index][0] - blurred[index][0]) * amount * mask,
                source[index][1] + (source[index][1] - blurred[index][1]) * amount * mask,
                source[index][2] + (source[index][2] - blurred[index][2]) * amount * mask,
            ];
            write_rgb(&mut out, index, clamp_to_local_range(width, height, &source, x as i32, y as i32, rgb));
        }
    }

    (width, height, out)
}

pub fn apply_local_laplacian_clarity(
    width: u32,
    height: u32,
    rgba: &[u8],
    params: LocalLaplacianClarityParams,
) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 || rgba.is_empty() || params.amount <= 0.0 {
        return (width, height, rgba.to_vec());
    }

    let source = rgba_to_rgb_f32(rgba);
    let blurred = laplacian_blur_rgb(width, height, &source, params.radius.max(1));
    let mut out = rgba.to_vec();
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let index = (y as u32 * width + x as u32) as usize;
            let edge = edge_strength(width, height, &source, x, y);
            let mask = smoothstep(0.015, 0.12, edge);
            if mask <= 0.0 {
                continue;
            }

            let rgb = [
                source[index][0] + (source[index][0] - blurred[index][0]) * params.amount * mask,
                source[index][1] + (source[index][1] - blurred[index][1]) * params.amount * mask,
                source[index][2] + (source[index][2] - blurred[index][2]) * params.amount * mask,
            ];
            write_rgb(&mut out, index, clamp_to_local_range(width, height, &source, x, y, rgb));
        }
    }
    (width, height, out)
}

pub fn apply_contrast_enhance(
    width: u32,
    height: u32,
    rgba: &[u8],
    params: ContrastEnhanceParams,
) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 || rgba.is_empty() || params.intensity <= 0.0 {
        return (width, height, rgba.to_vec());
    }

    let source = rgba_to_rgb_f32(rgba);
    let blurred = gaussian_blur_rgb(width, height, &source, params.blur_spread.max(0.5));
    let mut out = rgba.to_vec();
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let index = (y as u32 * width + x as u32) as usize;
            let high_pass = [
                source[index][0] - blurred[index][0],
                source[index][1] - blurred[index][1],
                source[index][2] - blurred[index][2],
            ];
            let contrast = luma_abs(high_pass);
            if contrast <= params.threshold {
                continue;
            }

            let mask = smoothstep(params.threshold, params.threshold * 2.5, contrast);
            let rgb = [
                source[index][0] + high_pass[0] * params.intensity * mask,
                source[index][1] + high_pass[1] * params.intensity * mask,
                source[index][2] + high_pass[2] * params.intensity * mask,
            ];
            write_rgb(&mut out, index, clamp_to_local_range(width, height, &source, x, y, rgb));
        }
    }
    (width, height, out)
}

pub fn apply_adaptive_log_contrast(
    width: u32,
    height: u32,
    rgba: &[u8],
    params: AdaptiveLogContrastParams,
) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 || rgba.is_empty() || params.gamma <= 0.0 {
        return (width, height, rgba.to_vec());
    }

    let source = rgba_to_rgb_f32(rgba);
    let local_mean = gaussian_blur_rgb(width, height, &source, params.radius.max(0.5));
    let local_contrast = local_contrast_rgb(width, height, &source, params.radius.max(0.5));
    let mut out = rgba.to_vec();
    let gamma = params.gamma.clamp(0.5, 1.5);

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let index = (y as u32 * width + x as u32) as usize;
            let brightness = luma(local_mean[index]);
            let contrast = local_contrast[index].clamp(0.0, 1.0);
            let strength = (1.0 - (brightness - 0.5).abs() * 0.8).clamp(0.35, 1.0)
                * smoothstep(0.01, 0.20, contrast)
                * 0.28;
            if strength <= 0.0 {
                continue;
            }

            let rgb = [
                adaptive_log_channel(source[index][0], local_mean[index][0], gamma, strength),
                adaptive_log_channel(source[index][1], local_mean[index][1], gamma, strength),
                adaptive_log_channel(source[index][2], local_mean[index][2], gamma, strength),
            ];
            write_rgb(&mut out, index, clamp_to_local_range(width, height, &source, x, y, rgb));
        }
    }
    (width, height, out)
}

fn rgba_to_rgb_f32(rgba: &[u8]) -> Vec<[f32; 3]> {
    rgba.chunks_exact(4)
        .map(|px| {
            [
                f32::from(px[0]) / 255.0,
                f32::from(px[1]) / 255.0,
                f32::from(px[2]) / 255.0,
            ]
        })
        .collect()
}

fn gaussian_blur_rgb(width: u32, height: u32, source: &[[f32; 3]], radius: f32) -> Vec<[f32; 3]> {
    let kernel = gaussian_kernel(radius);
    let mut tmp = vec![[0.0; 3]; source.len()];
    let mut out = vec![[0.0; 3]; source.len()];
    let half = (kernel.len() / 2) as i32;

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let mut acc = [0.0; 3];
            for (tap, weight) in kernel.iter().copied().enumerate() {
                let sample = rgb_at(width, height, source, x + tap as i32 - half, y);
                acc[0] += sample[0] * weight;
                acc[1] += sample[1] * weight;
                acc[2] += sample[2] * weight;
            }
            tmp[(y as u32 * width + x as u32) as usize] = acc;
        }
    }

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let mut acc = [0.0; 3];
            for (tap, weight) in kernel.iter().copied().enumerate() {
                let sample = rgb_at(width, height, &tmp, x, y + tap as i32 - half);
                acc[0] += sample[0] * weight;
                acc[1] += sample[1] * weight;
                acc[2] += sample[2] * weight;
            }
            out[(y as u32 * width + x as u32) as usize] = acc;
        }
    }

    out
}

fn laplacian_blur_rgb(width: u32, height: u32, source: &[[f32; 3]], radius: u32) -> Vec<[f32; 3]> {
    let mut current = source.to_vec();
    for _ in 0..radius {
        let mut next = current.clone();
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let index = (y as u32 * width + x as u32) as usize;
                let center = current[index];
                let center_luma = luma(center);
                let mut acc = center;
                let mut weight_sum = 1.0;
                for (ox, oy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let sample = rgb_at(width, height, &current, x + ox, y + oy);
                    let weight = (1.0 - (center_luma - luma(sample)).abs() * 5.0).clamp(0.0, 1.0);
                    for channel in 0..3 {
                        acc[channel] += sample[channel] * weight;
                    }
                    weight_sum += weight;
                }
                for channel in 0..3 {
                    next[index][channel] = acc[channel] / weight_sum;
                }
            }
        }
        current = next;
    }
    current
}

fn local_contrast_rgb(width: u32, height: u32, source: &[[f32; 3]], radius: f32) -> Vec<f32> {
    let kernel = gaussian_kernel(radius);
    let half = (kernel.len() / 2) as i32;
    let mut out = vec![0.0; source.len()];
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let center = luma(rgb_at(width, height, source, x, y));
            let mut acc = 0.0;
            let mut weight_sum = 0.0;
            for (ky, weight_y) in kernel.iter().copied().enumerate() {
                for (kx, weight_x) in kernel.iter().copied().enumerate() {
                    let weight = weight_x * weight_y;
                    let sample = luma(rgb_at(width, height, source, x + kx as i32 - half, y + ky as i32 - half));
                    acc += (sample - center).abs() * weight;
                    weight_sum += weight;
                }
            }
            out[(y as u32 * width + x as u32) as usize] = acc / weight_sum.max(0.000001);
        }
    }
    out
}

fn gaussian_kernel(radius: f32) -> Vec<f32> {
    let sigma = radius.max(0.1);
    let half = (sigma * 3.0).ceil() as i32;
    let mut kernel = Vec::with_capacity((half * 2 + 1) as usize);
    let mut sum = 0.0;
    for x in -half..=half {
        let xf = x as f32;
        let weight = (-0.5 * (xf / sigma) * (xf / sigma)).exp();
        kernel.push(weight);
        sum += weight;
    }
    for weight in &mut kernel {
        *weight /= sum;
    }
    kernel
}

fn edge_strength(width: u32, height: u32, source: &[[f32; 3]], x: i32, y: i32) -> f32 {
    let center = luma(rgb_at(width, height, source, x, y));
    let left = luma(rgb_at(width, height, source, x - 1, y));
    let right = luma(rgb_at(width, height, source, x + 1, y));
    let up = luma(rgb_at(width, height, source, x, y - 1));
    let down = luma(rgb_at(width, height, source, x, y + 1));
    (center - left).abs()
        .max((center - right).abs())
        .max((center - up).abs())
        .max((center - down).abs())
}

fn clamp_to_local_range(
    width: u32,
    height: u32,
    source: &[[f32; 3]],
    x: i32,
    y: i32,
    rgb: [f32; 3],
) -> [f32; 3] {
    let mut lo = [1.0f32; 3];
    let mut hi = [0.0f32; 3];
    for oy in -1..=1 {
        for ox in -1..=1 {
            let sample = rgb_at(width, height, source, x + ox, y + oy);
            for channel in 0..3 {
                lo[channel] = lo[channel].min(sample[channel]);
                hi[channel] = hi[channel].max(sample[channel]);
            }
        }
    }
    [
        rgb[0].clamp(lo[0], hi[0]),
        rgb[1].clamp(lo[1], hi[1]),
        rgb[2].clamp(lo[2], hi[2]),
    ]
}

fn rgb_at(width: u32, height: u32, source: &[[f32; 3]], x: i32, y: i32) -> [f32; 3] {
    let x = x.clamp(0, width as i32 - 1) as u32;
    let y = y.clamp(0, height as i32 - 1) as u32;
    source[(y * width + x) as usize]
}

fn write_rgb(out: &mut [u8], index: usize, rgb: [f32; 3]) {
    let base = index * 4;
    out[base] = float_to_u8(rgb[0]);
    out[base + 1] = float_to_u8(rgb[1]);
    out[base + 2] = float_to_u8(rgb[2]);
}

fn luma(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

fn luma_abs(rgb: [f32; 3]) -> f32 {
    rgb[0].abs() * 0.2126 + rgb[1].abs() * 0.7152 + rgb[2].abs() * 0.0722
}

fn adaptive_log_channel(value: f32, local_mean: f32, gamma: f32, strength: f32) -> f32 {
    let delta = value - local_mean;
    let shaped = delta.signum() * delta.abs().powf(gamma);
    value + (shaped - delta) * strength
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn float_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::{
        apply_adaptive_log_contrast, apply_contrast_enhance, apply_high_pass_sharpen,
        apply_local_laplacian_clarity, apply_scalefx_smart_deblur, apply_unsharp_mask,
        AdaptiveLogContrastParams, ContrastEnhanceParams, HighPassSharpenParams,
        LocalLaplacianClarityParams, ScaleFxSmartDeblurParams, UnsharpMaskParams,
    };

    #[test]
    fn sharpen_passes_preserve_size_and_alpha() {
        let rgba = vec![
            64, 64, 64, 0, 96, 96, 96, 64, 128, 128, 128, 128,
            160, 160, 160, 192, 192, 192, 192, 255, 224, 224, 224, 32,
        ];

        for out in [
            apply_unsharp_mask(3, 2, &rgba, UnsharpMaskParams::default()).2,
            apply_high_pass_sharpen(3, 2, &rgba, HighPassSharpenParams::default()).2,
            apply_scalefx_smart_deblur(3, 2, &rgba, ScaleFxSmartDeblurParams::default()).2,
            apply_local_laplacian_clarity(3, 2, &rgba, LocalLaplacianClarityParams::default()).2,
            apply_contrast_enhance(3, 2, &rgba, ContrastEnhanceParams::default()).2,
            apply_adaptive_log_contrast(3, 2, &rgba, AdaptiveLogContrastParams::default()).2,
        ] {
            assert_eq!(out.len(), rgba.len());
            assert_eq!(out.iter().skip(3).step_by(4).copied().collect::<Vec<_>>(), vec![0, 64, 128, 192, 255, 32]);
        }
    }

    #[test]
    fn flat_images_remain_flat() {
        let rgba = [80, 120, 160, 255].repeat(16);
        let (_, _, out) = apply_scalefx_smart_deblur(4, 4, &rgba, ScaleFxSmartDeblurParams::default());
        assert_eq!(out, rgba);
    }

    #[test]
    fn unsharp_increases_edge_contrast() {
        let rgba = vec![
            80, 80, 80, 255, 112, 112, 112, 255, 144, 144, 144, 255,
            80, 80, 80, 255, 112, 112, 112, 255, 144, 144, 144, 255,
            80, 80, 80, 255, 112, 112, 112, 255, 144, 144, 144, 255,
        ];
        let (_, _, out) = apply_unsharp_mask(3, 3, &rgba, UnsharpMaskParams::default());
        assert!(out[0] < rgba[0]);
        assert!(out[32] > rgba[32]);
    }

    #[test]
    fn clarity_increases_mid_boundary_contrast() {
        let rgba = vec![
            96, 96, 96, 255, 104, 104, 104, 255, 128, 128, 128, 255, 152, 152, 152, 255,
            96, 96, 96, 255, 104, 104, 104, 255, 128, 128, 128, 255, 152, 152, 152, 255,
            96, 96, 96, 255, 104, 104, 104, 255, 128, 128, 128, 255, 152, 152, 152, 255,
        ];
        let (_, _, out) = apply_local_laplacian_clarity(4, 3, &rgba, LocalLaplacianClarityParams::default());
        assert!(out[4] <= rgba[4]);
        assert!(out[12] >= rgba[12]);
    }

    #[test]
    fn contrast_enhance_threshold_preserves_flat_regions() {
        let rgba = [
            100, 100, 100, 255, 102, 102, 102, 255, 104, 104, 104, 255,
            106, 106, 106, 255, 108, 108, 108, 255, 110, 110, 110, 255,
            112, 112, 112, 255, 114, 114, 114, 255, 116, 116, 116, 255,
        ];
        let (_, _, out) = apply_contrast_enhance(3, 3, &rgba, ContrastEnhanceParams {
            intensity: 0.5,
            threshold: 0.15,
            blur_spread: 2.0,
        });
        assert_eq!(out, rgba);
    }

    #[test]
    fn adaptive_log_preserves_alpha_and_changes_edge_pixels() {
        let rgba = vec![
            70, 70, 70, 7, 90, 90, 90, 17, 150, 150, 150, 27,
            70, 70, 70, 37, 90, 90, 90, 47, 150, 150, 150, 57,
            70, 70, 70, 67, 90, 90, 90, 77, 150, 150, 150, 87,
        ];
        let (_, _, out) = apply_adaptive_log_contrast(3, 3, &rgba, AdaptiveLogContrastParams::default());
        assert_eq!(out.iter().skip(3).step_by(4).copied().collect::<Vec<_>>(), vec![7, 17, 27, 37, 47, 57, 67, 77, 87]);
        assert_ne!(out, rgba);
    }
}
