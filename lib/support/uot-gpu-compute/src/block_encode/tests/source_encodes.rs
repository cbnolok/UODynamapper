//! A package's BC7 under SPAN: lossless 1x frames (the Enhanced client's, exported with `uocf-cli animframe export`)
//! encoded as a package stores them, upscaled to 2x as `mobile_upscale` does, then encoded again as the renderer's
//! frame atlas does; beside them, the frames a package stores. A BC7 block across a change of hue (an Enhanced
//! daemon's orange hoof going grey) takes one colour line and comes out washed; at 1x the art's mottle hides it, SPAN
//! smooths the mottle away and leaves a pale patch with the block's straight edge (mobile_animation_motion.md, "Pale
//! patches on the hooves at SPAN 2x").
//!
//! For each frame it writes RGBA PNGs `{name}{encode}_{stage}.png`. The encode is empty for the frame as given, `Cpu`
//! for the packers' CPU encoder (AnalyticalWide) and each [`Bc7Quality`] for the GPU encoder, the packers' at
//! `GPU_BC7_QUALITY`; a packaged frame is only taken as stored. The stage is `1x` (the decoded frame), `2x` (its SPAN
//! upscale) and each of [`RUNTIME_QUALITIES`] (the 2x frame encoded at run time, decoded).
//! `docs/agent_notes/mobile_animation_motion/span_encode_table.py` turns them into the error table. Ignored: it needs a
//! GPU adapter, the exported frames and, for packaged frames, the packages.
//!
//! ```sh
//! SOURCE_FRAMES=/path/to/daemon_stand/n00001.png=daemon_stand \
//! SOURCE_PACKAGED_FRAMES=9/stand/3/0=daemon_stand_package \
//! BLOCK_ENCODE_PACKAGES=/path/to/uddp BLOCK_ENCODE_OUTPUT=/path/to/span-encodes \
//!     cargo test -p uot-gpu-compute --lib -- --ignored --exact \
//!     block_encode::tests::source_encodes::evaluate_source_frame_encodes --nocapture
//! ```

use super::*;
use crate::frame_upscales::{upscale_workspace_canvas, FrameUpscaler};
use crate::strides::StrideFrame;

/// Lossless frames: `path=name` pairs, comma-separated.
const FRAMES_VARIABLE: &str = "SOURCE_FRAMES";
/// Frames of the Enhanced package: `body/movement/direction/frame=name` pairs, comma-separated; the movement is one
/// of [`MOVEMENTS`].
const PACKAGED_VARIABLE: &str = "SOURCE_PACKAGED_FRAMES";
const MOVEMENTS: [(&str, u8); 2] = [("stand", anim_cc::MOVE_STAND), ("walk", anim_cc::MOVE_WALK)];
/// The profiles of `world_rendering.mobile_bc7_quality`, the renderer's encode of its 2x frames.
const RUNTIME_QUALITIES: [Bc7Quality; 2] = [Bc7Quality::UltraFast, Bc7Quality::VeryFast];
const CPU_ENCODE: &str = "Cpu";
const TEXEL_BYTES: usize = 4;

struct Frame {
    name: String,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn save_rgba(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    image::RgbaImage::from_raw(width, height, rgba.to_vec()).unwrap().save(path).unwrap();
}

/// `path=name` or `key=name` pairs of a comma-separated variable; none when it is not set.
fn pairs(variable: &str) -> Vec<(String, String)> {
    std::env::var(variable).map(|list| list.split(',').filter(|pair| !pair.is_empty()).map(|pair| {
        let (key, name) = pair.split_once('=').unwrap_or_else(|| panic!("{variable}: `{pair}` is not key=name"));
        (key.to_string(), name.to_string())
    }).collect()).unwrap_or_default()
}

fn packaged_frames(list: &[(String, String)]) -> Vec<Frame> {
    if list.is_empty() {
        return Vec::new();
    }
    let folder = PathBuf::from(std::env::var_os(PACKAGES_VARIABLE)
        .unwrap_or_else(|| panic!("set {PACKAGES_VARIABLE} for {PACKAGED_VARIABLE}")));
    let enhanced = MobileAnimEcPackage::load(folder.join(ENHANCED_PACKAGE)).unwrap();
    list.iter().map(|(key, name)| {
        let fields: Vec<&str> = key.split('/').collect();
        let [body, movement, direction, index] = fields[..] else {
            panic!("{PACKAGED_VARIABLE}: `{key}` is not body/movement/direction/frame");
        };
        let movement = MOVEMENTS.iter().find(|(label, _)| *label == movement)
            .unwrap_or_else(|| panic!("{PACKAGED_VARIABLE}: movement `{movement}` is not one of {MOVEMENTS:?}")).1;
        let body = body.parse().unwrap();
        let action = enhanced.move_action(body, anim_ec::GROUP_PEACE, movement).expect("an Enhanced action");
        let clip = enhanced.clip(body, action, direction.parse().unwrap()).unwrap().expect("Enhanced frames");
        let frame = &clip.frames[index.parse::<usize>().unwrap()];
        Frame { name: name.clone(), width: u32::from(frame.width), height: u32::from(frame.height),
            rgba: frame.rgba.to_vec() }
    }).collect()
}

/// The packers' CPU encode of a frame on transparent texels up to whole blocks, decoded.
fn cpu_encoded(frame: &Frame) -> Vec<u8> {
    let (padded_width, padded_height) = padded_size(frame.width, frame.height);
    let row_bytes = frame.width as usize * TEXEL_BYTES;
    let padded_row_bytes = padded_width as usize * TEXEL_BYTES;
    let mut padded = vec![0u8; padded_row_bytes * padded_height as usize];
    for row in 0..frame.height as usize {
        padded[row * padded_row_bytes..][..row_bytes].copy_from_slice(&frame.rgba[row * row_bytes..][..row_bytes]);
    }
    let extent = ImageExtent::new(padded_width, padded_height).unwrap();
    let blocks = uot_texture_codecs::bc7::encode_to_bc7(&padded, extent, RawImageFormat::Rgba8888,
        Bc7EncoderBackend::AnalyticalWide).unwrap().into_blocks();
    let decoded = uot_texture_codecs::bc7::decode_bc7_to_rgba8888(&blocks, extent).unwrap();
    (0..frame.height as usize).flat_map(|row| decoded[row * padded_row_bytes..][..row_bytes].iter().copied()).collect()
}

/// A frame through the GPU encoder at `quality`, decoded.
fn gpu_encoded(gpu: &Gpu, encoder: &mut MobileBlockEncoder, quality: Bc7Quality, frame: &Frame) -> Vec<u8> {
    let sample = Sample { name: frame.name.clone(), group: String::new(), classic: false, width: frame.width,
        height: frame.height, rgba: frame.rgba.clone(), sheet: false };
    let source = upload(gpu, &sample);
    let format = MobileBlockFormat::Bc7(quality);
    let encoded = encode(gpu, encoder, format, &[&sample], &[&source]);
    decode(format, &sample, &encoded.blocks[0])
}

#[test]
#[ignore = "requires a GPU adapter and lossless frames in SOURCE_FRAMES (packaged ones need BLOCK_ENCODE_PACKAGES); \
            run explicitly to see a package's BC7 under SPAN"]
fn evaluate_source_frame_encodes() {
    let lossless = pairs(FRAMES_VARIABLE);
    let packaged = pairs(PACKAGED_VARIABLE);
    if lossless.is_empty() && packaged.is_empty() {
        println!("skipped: neither {FRAMES_VARIABLE} nor {PACKAGED_VARIABLE} is set");
        return;
    }
    let output = PathBuf::from(std::env::var_os(OUTPUT_VARIABLE)
        .unwrap_or_else(|| panic!("set {OUTPUT_VARIABLE} to the folder for the frames")));
    std::fs::create_dir_all(&output).unwrap();
    let Some(gpu) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let lossless: Vec<Frame> = lossless.into_iter().map(|(path, name)| {
        let image = image::open(&path).unwrap_or_else(|error| panic!("{path}: {error}")).to_rgba8();
        Frame { name, width: image.width(), height: image.height(), rgba: image.into_raw() }
    }).collect();
    let packaged = packaged_frames(&packaged);
    let max_side = lossless.iter().chain(&packaged).map(|frame| frame.width.max(frame.height)).max().unwrap();
    let mut upscaler = FrameUpscaler::new(max_side).unwrap();
    let mut encoder = MobileBlockEncoder::lazy(&gpu.device);

    let mut variants = Vec::new();
    for frame in lossless {
        variants.push(Frame { name: format!("{}{CPU_ENCODE}", frame.name), rgba: cpu_encoded(&frame), ..frame });
        for quality in Bc7Quality::ALL {
            variants.push(Frame { name: format!("{}{quality:?}", frame.name),
                rgba: gpu_encoded(&gpu, &mut encoder, quality, &frame), ..frame });
        }
        variants.push(frame);
    }
    variants.extend(packaged);
    for frame in &variants {
        save_rgba(&output.join(format!("{}_1x.png", frame.name)), frame.width, frame.height, &frame.rgba);
        let stride = StrideFrame { width: frame.width as u16, height: frame.height as u16, anchor_x: 0, anchor_y: 0,
            rgba: &frame.rgba };
        let made = upscaler.upscale_on(&stride, false, upscale_workspace_canvas(frame.width, frame.height)).unwrap();
        save_rgba(&output.join(format!("{}_2x.png", frame.name)), made.width, made.height, &made.rgba);
        let upscaled = Frame { name: frame.name.clone(), width: made.width, height: made.height, rgba: made.rgba };
        for quality in RUNTIME_QUALITIES {
            let decoded = gpu_encoded(&gpu, &mut encoder, quality, &upscaled);
            save_rgba(&output.join(format!("{}_{quality:?}.png", frame.name)), made.width, made.height, &decoded);
        }
        println!("{}: {}x{} -> {}x{}", frame.name, frame.width, frame.height, made.width, made.height);
    }
}
