//! The atmosphere maps, the Enhanced light textures and the Classic light.mul lights as BC7, BC4 and BC5: size with
//! Zstd 19 (the packers' level), VRAM, and error against the lossless texels.
//! Usage: aux_maps <uddp dir> [<dump dir>]. With a dump dir, writes each texture and its BC7 decode as raw RGBA
//! (`<key>_<w>x<h>_{rgba,bc7}.rgba`) for a visual check.
//!
//! BC7 is the packers' encoder (`AnalyticalWide`, RDO off) and block_compression's ISPC port at `alpha_slow`, the
//! kernels uot-gpu-compute runs on the GPU. BC4 and BC5 are block_compression's. A light texture is colour times
//! alpha, so BC4 keeps its alpha and BC5 its alpha and luminance, with the texture's mean colour as a constant.
use std::path::Path;
use block_compression::{encode::compress_rgba8, decode::decompress_blocks_as_rgba8, BC7Settings, CompressionVariant};
use udd_assets::tex_aux_ec::{
    is_tex_aux_ec_reserved_key, TexAuxEcPackage, EC_ATMOSPHERE_MAP_TERRAIN_TEXTURE_ENTRIES, TEX_AUX_EC_MISSING_TEXTURE_ID,
    TEX_AUX_EC_TERRAIN_TEXTURE_KEY_BASE,
};
use udd_assets::world_lights::WorldLightsPackage;
use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, encode_to_bc7_with_rdo_lambda, Bc7EncoderBackend, ImageExtent,
    RawImageFormat};

const ZSTD_LEVEL: i32 = 19;
const RGBA: usize = 4;
const MAX: f64 = 255.0;
const KIB: f64 = 1024.0;

#[derive(Default, Clone, Copy)]
struct Totals { textures: usize, raw: usize, rgba_z: usize, bc7_z: usize, ispc_z: usize, bc4_z: usize, bc5_z: usize,
    vram_rgba: usize, vram_bc7: usize, vram_bc4: usize }

/// Squared error sums and maxima of `got` against `want`: RGB where alpha > 0, alpha, and the premultiplied colour
/// (rgb * a / 255, what the light buffer adds).
#[derive(Default, Clone, Copy)]
struct Error { rgb_sq: f64, rgb_n: f64, rgb_max: u8, a_sq: f64, a_n: f64, a_max: u8, pm_sq: f64, pm_max: f64,
    mean_shift: [f64; 3] }

impl Error {
    fn measure(want: &[u8], got: &[u8]) -> Self {
        let mut e = Error::default();
        let mut sums = [[0.0f64; 3]; 2];
        for (w, g) in want.chunks_exact(RGBA).zip(got.chunks_exact(RGBA)) {
            for c in 0..3 {
                sums[0][c] += f64::from(w[c]);
                sums[1][c] += f64::from(g[c]);
                let pm = (f64::from(w[c]) * f64::from(w[3]) - f64::from(g[c]) * f64::from(g[3])) / MAX;
                e.pm_sq += pm * pm;
                e.pm_max = e.pm_max.max(pm.abs());
            }
            if w[3] > 0 {
                for c in 0..3 {
                    let d = w[c].abs_diff(g[c]);
                    e.rgb_sq += f64::from(d) * f64::from(d);
                    e.rgb_max = e.rgb_max.max(d);
                }
                e.rgb_n += 3.0;
            }
            let d = w[3].abs_diff(g[3]);
            e.a_sq += f64::from(d) * f64::from(d);
            e.a_max = e.a_max.max(d);
            e.a_n += 1.0;
        }
        let texels = (want.len() / RGBA) as f64;
        e.mean_shift = std::array::from_fn(|c| (sums[1][c] - sums[0][c]) / texels);
        e
    }

    fn add(&mut self, o: &Error) {
        self.rgb_sq += o.rgb_sq; self.rgb_n += o.rgb_n; self.rgb_max = self.rgb_max.max(o.rgb_max);
        self.a_sq += o.a_sq; self.a_n += o.a_n; self.a_max = self.a_max.max(o.a_max);
        self.pm_sq += o.pm_sq; self.pm_max = self.pm_max.max(o.pm_max);
    }

    fn psnr(sq: f64, n: f64) -> f64 { if sq == 0.0 { f64::INFINITY } else { 10.0 * (MAX * MAX / (sq / n)).log10() } }

    fn line(&self) -> String {
        format!("RGB {:5.1} dB max {:3} | A {:5.1} dB max {:3} | premult {:5.1} dB max {:5.1}",
            Self::psnr(self.rgb_sq, self.rgb_n), self.rgb_max, Self::psnr(self.a_sq, self.a_n), self.a_max,
            Self::psnr(self.pm_sq, self.a_n * 3.0), self.pm_max)
    }
}

fn zstd(bytes: &[u8]) -> usize { zstd::bulk::compress(bytes, ZSTD_LEVEL).unwrap().len() }

fn ispc(variant: CompressionVariant, rgba: &[u8], w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
    let mut blocks = vec![0u8; variant.blocks_byte_size(w, h)];
    compress_rgba8(variant, rgba, &mut blocks, w, h, w * RGBA as u32);
    let mut decoded = vec![0u8; rgba.len()];
    decompress_blocks_as_rgba8(variant, w, h, &blocks, &mut decoded);
    (blocks, decoded)
}

/// The texture's colour weighted by alpha, as `client_light_sprites::light_texture_glow_rgb` takes it.
fn glow(rgba: &[u8]) -> [f64; 3] {
    let (mut s, mut a) = ([0.0f64; 3], 0.0f64);
    for t in rgba.chunks_exact(RGBA) {
        for c in 0..3 { s[c] += f64::from(t[c]) * f64::from(t[3]); }
        a += f64::from(t[3]);
    }
    if a == 0.0 { [0.0; 3] } else { s.map(|v| v / a) }
}

struct Row { totals: Totals, bc7: Error, ispc: Error, bc4: Error, bc5: Error }

fn study(label: &str, key: u32, w: u32, h: u32, rgba: &[u8], dump: Option<&Path>, row: &mut Row) {
    let extent = ImageExtent::new(w, h).unwrap();
    let bc7 = encode_to_bc7_with_rdo_lambda(rgba, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide,
        0.0).unwrap();
    let bc7_rgba = decode_bc7_to_rgba8888(bc7.blocks(), extent).unwrap();
    let (ispc_blocks, ispc_rgba) = ispc(CompressionVariant::BC7(BC7Settings::alpha_slow()), rgba, w, h);
    // BC4: alpha in R; colour the texture's glow colour. BC5: alpha in R, the texel's largest channel in G, so the
    // colour keeps its brightness and takes the glow colour's hue.
    let colour = glow(rgba);
    let peak = colour.iter().cloned().fold(0.0, f64::max).max(1.0);
    let single = |pick: &dyn Fn(&[u8]) -> [u8; 2]| -> Vec<u8> {
        rgba.chunks_exact(RGBA).flat_map(|t| { let [r, g] = pick(t); [r, g, 0, u8::MAX] }).collect()
    };
    let rebuild = |decoded: &[u8], luminance: bool| -> Vec<u8> {
        decoded.chunks_exact(RGBA).flat_map(|t| {
            let scale = if luminance { f64::from(t[1]) / peak } else { 1.0 };
            let c = colour.map(|v| (v * scale).round().clamp(0.0, MAX) as u8);
            [c[0], c[1], c[2], t[0]]
        }).collect()
    };
    let (bc4_blocks, bc4_dec) = ispc(CompressionVariant::BC4, &single(&|t| [t[3], 0]), w, h);
    let (bc5_blocks, bc5_dec) = ispc(CompressionVariant::BC5, &single(&|t| [t[3], t[0].max(t[1]).max(t[2])]), w, h);
    let (bc4_rgba, bc5_rgba) = (rebuild(&bc4_dec, false), rebuild(&bc5_dec, true));
    let e = [Error::measure(rgba, &bc7_rgba), Error::measure(rgba, &ispc_rgba), Error::measure(rgba, &bc4_rgba),
        Error::measure(rgba, &bc5_rgba)];
    let sizes = Totals { textures: 1, raw: rgba.len(), rgba_z: zstd(rgba), bc7_z: zstd(bc7.blocks()), ispc_z: zstd(&ispc_blocks),
        bc4_z: zstd(&bc4_blocks), bc5_z: zstd(&bc5_blocks), vram_rgba: rgba.len(), vram_bc7: bc7.blocks().len(),
        vram_bc4: bc4_blocks.len() };
    println!("{label} {key:#010x} {w}x{h}: zstd RGBA {:6.1} KiB, BC7 {:6.1}, ISPC BC7 {:6.1}, BC4 {:5.1}, BC5 {:5.1}",
        sizes.rgba_z as f64 / KIB, sizes.bc7_z as f64 / KIB, sizes.ispc_z as f64 / KIB, sizes.bc4_z as f64 / KIB,
        sizes.bc5_z as f64 / KIB);
    println!("    BC7  {}  mean shift {:+.3?}", e[0].line(), e[0].mean_shift);
    println!("    ISPC {}", e[1].line());
    println!("    BC4  {}", e[2].line());
    println!("    BC5  {}", e[3].line());
    let g7 = glow(&bc7_rgba);
    println!("    glow colour {:.2?} -> BC7 {:.2?}", colour, g7);
    if let Some(dir) = dump {
        for (name, px) in [("rgba", rgba), ("bc7", &bc7_rgba[..])] {
            std::fs::write(dir.join(format!("{key:08x}_{w}x{h}_{name}.rgba")), px).unwrap();
        }
    }
    let t = &mut row.totals;
    t.textures += 1; t.raw += sizes.raw; t.rgba_z += sizes.rgba_z; t.bc7_z += sizes.bc7_z; t.ispc_z += sizes.ispc_z;
    t.bc4_z += sizes.bc4_z; t.bc5_z += sizes.bc5_z; t.vram_rgba += sizes.vram_rgba; t.vram_bc7 += sizes.vram_bc7;
    t.vram_bc4 += sizes.vram_bc4;
    row.bc7.add(&e[0]); row.ispc.add(&e[1]); row.bc4.add(&e[2]); row.bc5.add(&e[3]);
}

fn summary(label: &str, row: &Row) {
    let t = row.totals;
    let k = |v: usize| v as f64 / KIB;
    println!("== {label}: {} textures, raw {:.1} KiB; zstd RGBA {:.1}, BC7 {:.1}, ISPC BC7 {:.1}, BC4 {:.1}, BC5 {:.1} KiB; \
        VRAM RGBA {:.1}, BC7 {:.1}, BC4 {:.1} KiB", t.textures, k(t.raw), k(t.rgba_z), k(t.bc7_z), k(t.ispc_z), k(t.bc4_z),
        k(t.bc5_z), k(t.vram_rgba), k(t.vram_bc7), k(t.vram_bc4));
    println!("   BC7  {}", row.bc7.line());
    println!("   ISPC {}", row.ispc.line());
    println!("   BC4  {}", row.bc4.line());
    println!("   BC5  {}", row.bc5.line());
}

fn new_row() -> Row { Row { totals: Totals::default(), bc7: Error::default(), ispc: Error::default(), bc4: Error::default(),
    bc5: Error::default() } }

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("uddp dir");
    let dump = args.next().map(std::path::PathBuf::from);
    if let Some(dump) = &dump { std::fs::create_dir_all(dump).unwrap(); }
    let aux = TexAuxEcPackage::load(format!("{dir}/tex_aux_ec.uddp")).unwrap();
    let (mut atmosphere, mut lights, mut missing) = (new_row(), new_row(), new_row());
    for slot in aux.slots().iter().filter(|slot| is_tex_aux_ec_reserved_key(slot.texture_id) && !slot.is_missing_source()) {
        let key = slot.texture_id;
        let rgba = aux.load_rgba(key).unwrap().unwrap();
        let (w, h) = (u32::from(slot.width), u32::from(slot.height));
        let format = if slot.is_bc7() { "BC7" } else { "RGBA" };
        let (label, row) = if key == TEX_AUX_EC_MISSING_TEXTURE_ID {
            ("missing", &mut missing)
        } else if EC_ATMOSPHERE_MAP_TERRAIN_TEXTURE_ENTRIES.contains(&(key & !TEX_AUX_EC_TERRAIN_TEXTURE_KEY_BASE)) {
            ("atmosphere", &mut atmosphere)
        } else {
            ("light", &mut lights)
        };
        let alpha: Vec<u8> = rgba.chunks_exact(RGBA).map(|t| t[3]).collect();
        let grey = rgba.chunks_exact(RGBA).all(|t| t[0] == t[1] && t[1] == t[2]);
        println!("{label} {key:#010x} stored {format}, alpha {}..{}, grey {grey}", alpha.iter().min().unwrap(),
            alpha.iter().max().unwrap());
        study(label, key, w, h, &rgba, dump.as_deref(), row);
    }
    summary("atmosphere maps", &atmosphere);
    summary("light textures", &lights);
    summary("missing texture", &missing);

    // light.mul: signed intensities, one byte a texel, offset by 128 into BC4's R (BC4 SNORM codes the same way).
    let lights_mul = WorldLightsPackage::load(format!("{dir}/world_lights.uddp")).unwrap();
    let (mut raw, mut z_raw, mut z_bc4, mut vram_bc4, mut sq, mut n, mut max_err, mut count) = (0, 0, 0, 0, 0.0, 0.0, 0u8, 0);
    for slot in lights_mul.slots().iter().filter(|slot| slot.is_present()) {
        let values = lights_mul.read_light_intensities(slot.light_id).unwrap();
        let (w, h) = (u32::from(slot.width), u32::from(slot.height));
        let bytes: Vec<u8> = values.iter().map(|&v| v as u8).collect();
        raw += bytes.len();
        z_raw += zstd(&bytes);
        let (pw, ph) = (w.div_ceil(4) * 4, h.div_ceil(4) * 4);
        let mut padded = vec![0u8; (pw * ph) as usize * RGBA];
        for y in 0..h { for x in 0..w {
            let v = (i16::from(values[(y * w + x) as usize]) + 128) as u8;
            padded[((y * pw + x) as usize) * RGBA..][..RGBA].copy_from_slice(&[v, 0, 0, u8::MAX]);
        } }
        let (blocks, decoded) = ispc(CompressionVariant::BC4, &padded, pw, ph);
        z_bc4 += zstd(&blocks);
        vram_bc4 += blocks.len();
        for y in 0..h { for x in 0..w {
            let want = (i16::from(values[(y * w + x) as usize]) + 128) as u8;
            let d = want.abs_diff(decoded[((y * pw + x) as usize) * RGBA]);
            sq += f64::from(d) * f64::from(d);
            n += 1.0;
            max_err = max_err.max(d);
        } }
        count += 1;
    }
    // A light.mul intensity step is 1 of 31; the runtime's RGBA8 SNORM texture of a light is 4 bytes a texel.
    println!("== light.mul: {count} lights, {:.1} KiB raw i8; zstd i8 {:.1} KiB, BC4 {:.1} KiB; VRAM as uploaded \
        (Rgba8Snorm) {:.1} KiB, BC4 {:.1} KiB; BC4 error {:.2} intensity steps RMS, max {max_err} steps",
        raw as f64 / KIB, z_raw as f64 / KIB, z_bc4 as f64 / KIB, (raw * RGBA) as f64 / KIB, vram_bc4 as f64 / KIB,
        (sq / n).sqrt());
}
