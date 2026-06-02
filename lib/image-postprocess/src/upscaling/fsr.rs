//! AMD FSR 1.1 EASU (Edge Adaptive Spatial Upsampling) and RCAS (Robust Contrast Adaptive Sharpening).
//!
//! Reference: https://github.com/GPUOpen-Effects/FidelityFX-FSR/blob/master/ffx-fsr/ffx_fsr1.h

use wide::f32x4;

pub fn apply_easu(
    width: u32,
    height: u32,
    rgba: &[u8],
    target_width: u32,
    target_height: u32,
) -> (u32, u32, Vec<u8>) {
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
                / (norm_dir[0].abs().max(norm_dir[1].abs()).max(1e-6));
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
                (a_c[0] / a_w.max(1e-6)).clamp(min4[0], max4[0]),
                (a_c[1] / a_w.max(1e-6)).clamp(min4[1], max4[1]),
                (a_c[2] / a_w.max(1e-6)).clamp(min4[2], max4[2]),
            ];

            let out_idx = (y * target_width + x) as usize * 4;
            out_rgba[out_idx] = (res[0] * 255.0).round() as u8;
            out_rgba[out_idx + 1] = (res[1] * 255.0).round() as u8;
            out_rgba[out_idx + 2] = (res[2] * 255.0).round() as u8;

            let src_idx = (fp[1].clamp(0.0, height as f32 - 1.0) as usize * width as usize
                + fp[0].clamp(0.0, width as f32 - 1.0) as usize)
                * 4;
            out_rgba[out_idx + 3] = rgba[src_idx + 3];
        }
    }

    (target_width, target_height, out_rgba)
}

pub fn apply_rcas(
    width: u32,
    height: u32,
    rgba: &[u8],
    sharpness: f32,
) -> Vec<u8> {
    let mut out_rgba = vec![0u8; (width * height * 4) as usize];
    
    // sharpness is in "stops" (0.0 = max sharpness, higher = less)
    let sharp_val = (-sharpness).exp2();
    const FSR_RCAS_LIMIT: f32 = 0.1875;
    
    let get_p = |x: i32, y: i32| -> f32x4 {
        let px = x.clamp(0, width as i32 - 1) as usize;
        let py = y.clamp(0, height as i32 - 1) as usize;
        let idx = (py * width as usize + px) * 4;
        f32x4::from([
            rgba[idx] as f32 / 255.0,
            rgba[idx + 1] as f32 / 255.0,
            rgba[idx + 2] as f32 / 255.0,
            0.0,
        ])
    };

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            // Neighborhood:
            //    b 
            //  d e f
            //    h
            let b = get_p(x, y - 1);
            let d = get_p(x - 1, y);
            let e = get_p(x, y);
            let f = get_p(x + 1, y);
            let h = get_p(x, y + 1);

            // Approximate luma
            let b_l = fsr_luma4(b);
            let d_l = fsr_luma4(d);
            let e_l = fsr_luma4(e);
            let f_l = fsr_luma4(f);
            let h_l = fsr_luma4(h);

            // Noise detection
            let nz = 0.25 * b_l + 0.25 * d_l + 0.25 * f_l + 0.25 * h_l - e_l;
            let luma_max = b_l.max(d_l).max(f_l).max(h_l);
            let luma_min = b_l.min(d_l).min(f_l).min(h_l);
            let range = (luma_max - luma_min).max(1e-6);
            let nz = (nz.abs() / range).clamp(0.0, 1.0);
            let nz = -0.5 * nz + 1.0;

            // Min and max of ring
            let mn4 = b.min(d).min(f).min(h);
            let mx4 = b.max(d).max(f).max(h);

            // Limiters (as per ffx_fsr1.h)
            // hitMin = min(mn4, e) * rcp(4.0 * mx4)
            let hit_min = mn4.min(e) / (mx4 * f32x4::splat(4.0)).max(f32x4::splat(1e-6));
            // hitMax = (1.0 - max(mx4, e)) * rcp(4.0 * mn4 - 4.0)
            // Using abs() and max(1e-6) to avoid division by zero
            let hit_max = (f32x4::splat(1.0) - mx4.max(e))
                / (mn4 * f32x4::splat(4.0) - f32x4::splat(4.0))
                    .abs()
                    .max(f32x4::splat(1e-6));
            let lobe = (f32x4::splat(0.0) - hit_min).max(hit_max).to_array();
            
            // final_lobe = clamp(max(lobeR, lobeG, lobeB), -limit, 0.0) * sharp_val
            let mut final_lobe = lobe[0].max(lobe[1]).max(lobe[2]).min(0.0).max(-FSR_RCAS_LIMIT);
            final_lobe *= sharp_val * nz;
            
            // Resolve
            let rcp_l = 1.0 / (4.0 * final_lobe + 1.0);
            let res = ((b + d + h + f) * f32x4::splat(final_lobe) + e) * f32x4::splat(rcp_l);
            let res = res.to_array();

            let out_idx = (y as u32 * width + x as u32) as usize * 4;
            out_rgba[out_idx] = (res[0].clamp(0.0, 1.0) * 255.0).round() as u8;
            out_rgba[out_idx + 1] = (res[1].clamp(0.0, 1.0) * 255.0).round() as u8;
            out_rgba[out_idx + 2] = (res[2].clamp(0.0, 1.0) * 255.0).round() as u8;
            out_rgba[out_idx + 3] = rgba[out_idx + 3];
        }
    }

    out_rgba
}

fn fsr_luma(c: [f32; 3]) -> f32 {
    c[1] + 0.5 * (c[0] + c[2])
}

fn fsr_luma4(c: f32x4) -> f32 {
    let c = c.to_array();
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
    let lx = if len_x > 1e-6 {
        (dir_x.abs() / len_x).clamp(0.0, 1.0)
    } else {
        0.0
    };
    *len += lx * lx * w;

    let len_y = (l_e - l_c).abs().max((l_c - l_a).abs());
    let dir_y = l_e - l_a;
    dir[1] += dir_y * w;
    let ly = if len_y > 1e-6 {
        (dir_y.abs() / len_y).clamp(0.0, 1.0)
    } else {
        0.0
    };
    *len += ly * ly * w;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rcas_preserves_size_and_alpha() {
        let rgba = vec![
            0, 0, 0, 10,
            255, 0, 0, 20,
            0, 255, 0, 30,
            0, 0, 255, 40,
        ];

        let out = apply_rcas(2, 2, &rgba, 0.0);

        assert_eq!(out.len(), rgba.len());
        assert_eq!(out[3], 10);
        assert_eq!(out[7], 20);
        assert_eq!(out[11], 30);
        assert_eq!(out[15], 40);
    }
}
