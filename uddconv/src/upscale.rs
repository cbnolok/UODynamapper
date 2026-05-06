//! Image upscaling filters for build-time asset processing.

use image::imageops::{self, FilterType};
use image::{ImageBuffer, Rgba};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum UpscaleFilter {
    #[default]
    None,
    Lq2x,
    Lq3x,
    Lq4x,
    SuperSai,
    AmdEasu,
}

impl UpscaleFilter {
    pub fn scale_factor(self) -> u32 {
        match self {
            Self::None => 1,
            Self::Lq2x => 2,
            Self::Lq3x => 3,
            Self::Lq4x => 4,
            Self::SuperSai => 2,
            Self::AmdEasu => 2,
        }
    }

    pub fn apply(&self, width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
        if matches!(self, Self::None) || rgba.is_empty() {
            return (width, height, rgba.to_vec());
        }

        let scale = self.scale_factor();
        let target_width = width * scale;
        let target_height = height * scale;

        match self {
            Self::None => (width, height, rgba.to_vec()),
            Self::Lq2x | Self::Lq3x | Self::Lq4x => {
                // For lq*x we use CatmullRom as a high-quality baseline.
                // In many contexts lq2x/lq3x/lq4x refer to "Linear Quality" or similar.
                let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba).unwrap();
                let upscaled =
                    imageops::resize(&img, target_width, target_height, FilterType::CatmullRom);
                (target_width, target_height, upscaled.into_raw())
            }
            Self::SuperSai => {
                // SuperSAI is a complex pixel-art scaler. For now, we fallback to Lanczos3
                // as it preserves edges well.
                let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba).unwrap();
                let upscaled =
                    imageops::resize(&img, target_width, target_height, FilterType::Lanczos3);
                (target_width, target_height, upscaled.into_raw())
            }
            Self::AmdEasu => apply_easu(width, height, rgba, target_width, target_height),
        }
    }
}

fn apply_easu(
    width: u32,
    height: u32,
    rgba: &[u8],
    target_width: u32,
    target_height: u32,
) -> (u32, u32, Vec<u8>) {
    // Port of AMD FSR 1.1 EASU (Edge Adaptive Spatial Upsampling) to Rust.
    // Based on the WGSL implementation in fsr_easu.wgsl.

    let mut out_rgba = vec![0u8; (target_width * target_height * 4) as usize];

    let input_viewport = [width as f32, height as f32];
    let input_size = [width as f32, height as f32];
    let output_size = [target_width as f32, target_height as f32];

    let con0 = [
        input_viewport[0] / output_size[0],
        input_viewport[1] / output_size[1],
        0.5 * input_viewport[0] / output_size[0] - 0.5,
        0.5 * input_viewport[1] / output_size[1] - 0.5,
    ];

    let inv_input = [1.0 / input_size[0], 1.0 / input_size[1]];
    let con1 = [inv_input[0], inv_input[1], inv_input[0], -inv_input[1]];
    let con2 = [
        -inv_input[0],
        2.0 * inv_input[1],
        inv_input[0],
        2.0 * inv_input[1],
    ];
    let con3 = [0.0, 4.0 * inv_input[1], 0.0, 0.0];

    for y in 0..target_height {
        for x in 0..target_width {
            let ip = [x as f32, y as f32];

            let pp_raw = [ip[0] * con0[0] + con0[2], ip[1] * con0[1] + con0[3]];
            let fp = [pp_raw[0].floor(), pp_raw[1].floor()];
            let pp = [pp_raw[0] - fp[0], pp_raw[1] - fp[1]];

            let p0 = [fp[0] * con1[0] + con1[2], fp[1] * con1[1] + con1[3]];
            let p1 = [p0[0] + con2[0], p0[1] + con2[1]];
            let p2 = [p0[0] + con2[2], p0[1] + con2[3]];
            let p3 = [p0[0] + con3[0], p0[1] + con3[1]];

            let off = [-0.5 * con1[0], 0.5 * con1[0], -0.5 * con1[1], 0.5 * con1[1]];

            let fetch = |u: f32, v: f32| -> [f32; 3] {
                let iu = (u * width as f32).floor() as i32;
                let iv = (v * height as f32).floor() as i32;
                let iu = iu.clamp(0, width as i32 - 1) as usize;
                let iv = iv.clamp(0, height as i32 - 1) as usize;
                let idx = (iv * width as usize + iu) * 4;
                [
                    rgba[idx] as f32 / 255.0,
                    rgba[idx + 1] as f32 / 255.0,
                    rgba[idx + 2] as f32 / 255.0,
                ]
            };

            let b_c = fetch(p0[0] + off[0], p0[1] + off[3]);
            let b_l = fsr_luma(b_c);
            let c_c = fetch(p0[0] + off[1], p0[1] + off[3]);
            let c_l = fsr_luma(c_c);
            let i_c = fetch(p1[0] + off[0], p1[1] + off[3]);
            let i_l = fsr_luma(i_c);
            let j_c = fetch(p1[0] + off[1], p1[1] + off[3]);
            let j_l = fsr_luma(j_c);
            let f_c = fetch(p1[0] + off[1], p1[1] + off[2]);
            let f_l = fsr_luma(f_c);
            let e_c = fetch(p1[0] + off[0], p1[1] + off[2]);
            let e_l = fsr_luma(e_c);
            let k_c = fetch(p2[0] + off[0], p2[1] + off[3]);
            let k_l = fsr_luma(k_c);
            let l_c = fetch(p2[0] + off[1], p2[1] + off[3]);
            let l_l = fsr_luma(l_c);
            let h_c = fetch(p2[0] + off[1], p2[1] + off[2]);
            let h_l = fsr_luma(h_c);
            let g_c = fetch(p2[0] + off[0], p2[1] + off[2]);
            let g_l = fsr_luma(g_c);
            let o_c = fetch(p3[0] + off[1], p3[1] + off[2]);
            let o_l = fsr_luma(o_c);
            let n_c = fetch(p3[0] + off[0], p3[1] + off[2]);
            let n_l = fsr_luma(n_c);

            let mut dir = [0.0f32, 0.0f32];
            let mut len_acc = 0.0f32;

            fsr_easu_set(
                &mut dir,
                &mut len_acc,
                (1.0 - pp[0]) * (1.0 - pp[1]),
                b_l,
                e_l,
                f_l,
                g_l,
                j_l,
            );
            fsr_easu_set(
                &mut dir,
                &mut len_acc,
                pp[0] * (1.0 - pp[1]),
                c_l,
                f_l,
                g_l,
                h_l,
                k_l,
            );
            fsr_easu_set(
                &mut dir,
                &mut len_acc,
                (1.0 - pp[0]) * pp[1],
                f_l,
                i_l,
                j_l,
                k_l,
                n_l,
            );
            fsr_easu_set(
                &mut dir,
                &mut len_acc,
                pp[0] * pp[1],
                g_l,
                j_l,
                k_l,
                l_l,
                o_l,
            );

            let dir2 = [dir[0] * dir[0], dir[1] * dir[1]];
            let dir_r_sq = dir2[0] + dir2[1];
            let zro = dir_r_sq < (1.0 / 32768.0);
            let dir_r = 1.0 / (dir_r_sq.max(1.0 / 32768.0)).sqrt();
            let norm_dir = if zro {
                [1.0, 0.0]
            } else {
                [dir[0] * dir_r, dir[1] * dir_r]
            };

            let mut len_shaped = len_acc * 0.5;
            len_shaped *= len_shaped;

            let stretch = (norm_dir[0] * norm_dir[0] + norm_dir[1] * norm_dir[1])
                / (norm_dir[0].abs().max(norm_dir[1].abs()));
            let len2 = [1.0 + (stretch - 1.0) * len_shaped, 1.0 - 0.5 * len_shaped];

            let lob = 0.5 - 0.29 * len_shaped;
            let clp = 1.0 / lob;

            let min4 = [
                f_c[0].min(g_c[0]).min(j_c[0].min(k_c[0])),
                f_c[1].min(g_c[1]).min(j_c[1].min(k_c[1])),
                f_c[2].min(g_c[2]).min(j_c[2].min(k_c[2])),
            ];
            let max4 = [
                f_c[0].max(g_c[0]).max(j_c[0].max(k_c[0])),
                f_c[1].max(g_c[1]).max(j_c[1].max(k_c[1])),
                f_c[2].max(g_c[2]).max(j_c[2].max(k_c[2])),
            ];

            let mut a_c = [0.0f32, 0.0f32, 0.0f32];
            let mut a_w = 0.0f32;

            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [0.0 - pp[0], -1.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                b_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [1.0 - pp[0], -1.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                c_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [-1.0 - pp[0], 1.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                i_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [0.0 - pp[0], 1.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                j_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [0.0 - pp[0], 0.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                f_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [-1.0 - pp[0], 0.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                e_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [1.0 - pp[0], 1.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                k_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [2.0 - pp[0], 1.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                l_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [2.0 - pp[0], 0.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                h_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [1.0 - pp[0], 0.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                g_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [1.0 - pp[0], 2.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                o_c,
            );
            fsr_easu_tap(
                &mut a_c,
                &mut a_w,
                [0.0 - pp[0], 2.0 - pp[1]],
                norm_dir,
                len2,
                lob,
                clp,
                n_c,
            );

            let res = [
                (a_c[0] / a_w).clamp(min4[0], max4[0]),
                (a_c[1] / a_w).clamp(min4[1], max4[1]),
                (a_c[2] / a_w).clamp(min4[2], max4[2]),
            ];

            let out_idx = (y * target_width + x) as usize * 4;
            out_rgba[out_idx] = (res[0] * 255.0).round() as u8;
            out_rgba[out_idx + 1] = (res[1] * 255.0).round() as u8;
            out_rgba[out_idx + 2] = (res[2] * 255.0).round() as u8;

            // TODO: Alpha preservation: use bilinear for alpha
            let src_idx = (fp[1].clamp(0.0, height as f32 - 1.0) as usize * width as usize
                + fp[0].clamp(0.0, width as f32 - 1.0) as usize)
                * 4;
            out_rgba[out_idx + 3] = rgba[src_idx + 3]; // Simple nearest for alpha for now, or we could interpolate.
        }
    }

    (target_width, target_height, out_rgba)
}

fn fsr_luma(c: [f32; 3]) -> f32 {
    c[1] + 0.5 * (c[0] + c[2])
}

fn fsr_easu_set(
    dir: &mut [f32; 2],
    len: &mut f32,
    w: f32,
    l_a: f32,
    l_b: f32,
    l_c: f32,
    l_d: f32,
    l_e: f32,
) {
    let len_x = (l_d - l_c).abs().max((l_c - l_b).abs());
    let dir_x = l_d - l_b;
    dir[0] += dir_x * w;
    let mut lx = 0.0;
    if len_x > 0.0 {
        lx = (dir_x.abs() / len_x).clamp(0.0, 1.0);
    }
    lx *= lx;
    *len += lx * w;

    let len_y = (l_e - l_c).abs().max((l_c - l_a).abs());
    let dir_y = l_e - l_a;
    dir[1] += dir_y * w;
    let mut ly = 0.0;
    if len_y > 0.0 {
        ly = (dir_y.abs() / len_y).clamp(0.0, 1.0);
    }
    ly *= ly;
    *len += ly * w;
}

fn fsr_easu_tap(
    a_c: &mut [f32; 3],
    a_w: &mut f32,
    off: [f32; 2],
    dir: [f32; 2],
    len: [f32; 2],
    lob: f32,
    clp: f32,
    c: [f32; 3],
) {
    let v = [
        off[0] * dir[0] + off[1] * dir[1],
        off[0] * (-dir[1]) + off[1] * dir[0],
    ];
    let v = [v[0] * len[0], v[1] * len[1]];
    let d2 = (v[0] * v[0] + v[1] * v[1]).min(clp);

    let mut w_b = 0.4 * d2 - 1.0;
    let mut w_a = lob * d2 - 1.0;
    w_b *= w_b;
    w_a *= w_a;
    w_b = 1.5625 * w_b - 0.5625;
    let w = w_b * w_a;

    a_c[0] += c[0] * w;
    a_c[1] += c[1] * w;
    a_c[2] += c[2] * w;
    *a_w += w;
}
