//! The BC7 profiles on SPAN 2x frames, the frames Dynamapper's `mobile_upscale` encodes into its BC7 atlas at run
//! time: real clips of the animation packages, standing and walking, upscaled by [`FrameUpscaler`] on the workspace
//! canvas of each clip's largest frame (as the game's sequence lays them out, Classic alpha snapped), then encoded with
//! every [`Bc7Quality`]. It writes each 2x frame and its decodings as RGBA PNGs, the GPU encode time of each frame
//! alone and the error table into a folder; `docs/agent_notes/mobile_animation_motion/span_bc7_sheets.py` turns them
//! into comparison sheets, the composited errors and the crowd's backlog. Ignored: it needs a GPU adapter and the
//! packages.
//!
//! ```sh
//! BLOCK_ENCODE_PACKAGES=/path/to/uddp BLOCK_ENCODE_OUTPUT=/path/to/span-bc7 \
//!     cargo test -p uot-gpu-compute --lib -- --ignored --exact \
//!     block_encode::tests::span_frames::evaluate_span_frame_profiles --nocapture
//! ```

use super::*;
use crate::frame_upscales::{upscale_workspace_canvas, upscales_whole, FrameUpscaler, UPSCALE_TILE_WORKSPACE_SIDE};
use crate::strides::StrideFrame;
use crate::upscale::tiles::{upscale_tiles, TileSpan};
use crate::upscale::{upscale_canvas_size, UPSCALE_BORDER};

/// Name, whether from the Classic package, in-game body id.
const CREATURES: [(&str, bool, u32); 5] = [("enhanced_human_400", false, 400), ("enhanced_daemon_9", false, 9),
    ("enhanced_dragon_12", false, 12), ("classic_horse_200", true, 200), ("classic_daemon_9", true, 9)];
/// The movements evaluated, with the fractions of their clip whose frames are taken.
const MOVEMENTS: [(&str, u8, &[f32]); 2] = [("stand", anim_cc::MOVE_STAND, &[0.0, 0.5]),
    ("walk", anim_cc::MOVE_WALK, &[0.0, 0.34, 0.67])];
const FRAMES_FILE: &str = "frames.tsv";
const CLIPS_FILE: &str = "clips.tsv";

/// A clip's frames as the upscaler takes them.
fn stride_frames(clip: &MobileClip) -> Vec<StrideFrame<'_>> {
    clip.frames.iter().map(|frame| StrideFrame { width: frame.width, height: frame.height, anchor_x: frame.anchor_x,
        anchor_y: frame.anchor_y, rgba: &frame.rgba }).collect()
}

fn save_rgba(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    image::RgbaImage::from_raw(width, height, rgba.to_vec()).unwrap().save(path).unwrap();
}

#[test]
#[ignore = "requires a GPU adapter and the animation packages in BLOCK_ENCODE_PACKAGES; run explicitly to evaluate \
            the BC7 profiles on SPAN 2x frames"]
fn evaluate_span_frame_profiles() {
    let Some(folder) = std::env::var_os(PACKAGES_VARIABLE).map(PathBuf::from) else {
        println!("skipped: {PACKAGES_VARIABLE} is not set");
        return;
    };
    let output = PathBuf::from(std::env::var_os(OUTPUT_VARIABLE)
        .unwrap_or_else(|| panic!("set {OUTPUT_VARIABLE} to the folder for the frames and the summary")));
    std::fs::create_dir_all(&output).unwrap();
    let Some(gpu) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let classic = MobileAnimCcPackage::load(folder.join(CLASSIC_PACKAGE)).unwrap();
    let enhanced = MobileAnimEcPackage::load(folder.join(ENHANCED_PACKAGE)).unwrap();

    let mut clips = Vec::new();
    for (creature, is_classic, body) in CREATURES {
        for (movement, id, fractions) in MOVEMENTS {
            let clip = if is_classic {
                let action = classic.movement_action(body as u16, id, false).expect("a Classic action");
                classic.clip(body, action, DIRECTION).unwrap().expect("Classic frames")
            } else {
                let action = enhanced.move_action(body, anim_ec::GROUP_PEACE, id).expect("an Enhanced action");
                enhanced.clip(body, action, DIRECTION).unwrap().expect("Enhanced frames")
            };
            clips.push((creature, is_classic, movement, fractions, clip));
        }
    }
    let max_side = clips.iter().flat_map(|clip| &clip.4.frames)
        .map(|frame| u32::from(frame.width.max(frame.height))).max().unwrap();
    let mut upscaler = FrameUpscaler::new(max_side).unwrap();
    let mut summary = String::new();
    writeln!(summary, "GPU timestamps: {}; upscaler: {}", gpu.timestamps, upscaler.adapter()).unwrap();

    // The clips' workspaces and passes, for the crowd's backlog: every frame of a clip upscales on its canvas.
    let mut clip_rows = String::from("creature\tmovement\tframes\tcanvas_w\tcanvas_h\twhole\tpasses_per_frame\t\
        multiply_adds_per_pass\tframes_2x_px\n");
    let mut samples = Vec::new();
    for (creature, is_classic, movement, fractions, clip) in &clips {
        let frames = stride_frames(clip);
        let canvas = upscale_workspace_canvas(
            frames.iter().map(|frame| u32::from(frame.width)).max().unwrap(),
            frames.iter().map(|frame| u32::from(frame.height)).max().unwrap());
        let whole = upscales_whole(canvas);
        let pass_side = |side: u32| side - 2 * UPSCALE_BORDER;
        let (passes, multiply_adds) = if whole {
            (1, upscaler.multiply_adds(pass_side(canvas[0]), pass_side(canvas[1])).unwrap())
        } else {
            // Each frame tiles its own canvas, as FrameUpscaler::upscale_on does.
            let tiles: usize = frames.iter().map(|frame| {
                let (width, height) = upscale_canvas_size(u32::from(frame.width), u32::from(frame.height));
                upscale_tiles([width, height], [UPSCALE_TILE_WORKSPACE_SIDE; 2],
                    [TileSpan { start: 0, end: width }, TileSpan { start: 0, end: height }]).unwrap().len()
            }).sum();
            (tiles.div_ceil(frames.len()), upscaler.multiply_adds(pass_side(UPSCALE_TILE_WORKSPACE_SIDE),
                pass_side(UPSCALE_TILE_WORKSPACE_SIDE)).unwrap())
        };
        let upscaled: Vec<_> = frames.iter().map(|frame| upscaler.upscale_on(frame, *is_classic, canvas).unwrap())
            .collect();
        let pixels_2x: Vec<String> = upscaled.iter().map(|made| (made.width * made.height).to_string()).collect();
        writeln!(clip_rows, "{creature}\t{movement}\t{}\t{}\t{}\t{whole}\t{passes}\t{multiply_adds}\t{}",
            frames.len(), canvas[0], canvas[1], pixels_2x.join(",")).unwrap();
        let mut taken: Vec<usize> = fractions.iter()
            .map(|fraction| ((fraction * frames.len() as f32) as usize).min(frames.len() - 1)).collect();
        taken.dedup();
        for index in taken {
            let made = &upscaled[index];
            samples.push(Sample { name: format!("{creature}_{movement}_{index}"), group: creature.to_string(),
                classic: *is_classic, width: made.width, height: made.height, rgba: made.rgba.clone(), sheet: true });
        }
    }
    std::fs::write(output.join(CLIPS_FILE), clip_rows).unwrap();

    let mut encoder = MobileBlockEncoder::lazy(&gpu.device);
    let sources: Vec<Source> = samples.iter().map(|sample| upload(&gpu, sample)).collect();
    for sample in &samples {
        save_rgba(&output.join(format!("{}_source.png", sample.name)), sample.width, sample.height, &sample.rgba);
    }
    let mut groups: Vec<String> = samples.iter().map(|sample| sample.group.clone()).collect();
    groups.dedup();
    let mut frame_rows = String::from("sample\tcreature\tclassic\twidth\theight\tprofile\talone_ms\n");
    writeln!(summary, "\n| creature | profile | frames | mean 2x px | GPU ms/frame alone (median of frames) | max | \
        GPU ms/frame in one pass | RGB PSNR dB (alpha > 0) | RGB max | shown PSNR dB (premultiplied) | shown max | \
        alpha PSNR dB | alpha max | soft alpha | halo (max alpha) | holes |").unwrap();
    writeln!(summary, "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|").unwrap();
    for group in &groups {
        let members: Vec<usize> = (0..samples.len()).filter(|&index| &samples[index].group == group).collect();
        let group_samples: Vec<&Sample> = members.iter().map(|&index| &samples[index]).collect();
        let group_sources: Vec<&Source> = members.iter().map(|&index| &sources[index]).collect();
        for quality in Bc7Quality::ALL {
            let format = MobileBlockFormat::Bc7(quality);
            let encoded = encode(&gpu, &mut encoder, format, &group_samples, &group_sources);
            let mut errors = Errors::default();
            let mut texels = 0u64;
            for (position, sample) in group_samples.iter().enumerate() {
                let decoded = decode(format, sample, &encoded.blocks[position]);
                errors.add(&sample.rgba, &decoded);
                texels += u64::from(sample.width * sample.height);
                save_rgba(&output.join(format!("{}_{quality:?}.png", sample.name)), sample.width, sample.height,
                    &decoded);
                writeln!(frame_rows, "{}\t{group}\t{}\t{}\t{}\t{quality:?}\t{:.4}", sample.name, sample.classic,
                    sample.width, sample.height, encoded.alone_ms[position]).unwrap();
            }
            let mut alone = encoded.alone_ms.clone();
            let max = alone.iter().copied().fold(0.0, f64::max);
            let row = format!("| {group} | {quality:?} | {} | {} | {:.3} | {max:.3} | {:.3} | {} |", members.len(),
                texels / members.len() as u64, median(&mut alone), encoded.together_ms / members.len() as f64,
                errors.columns());
            println!("{row}");
            writeln!(summary, "{row}").unwrap();
        }
    }
    std::fs::write(output.join(FRAMES_FILE), frame_rows).unwrap();
    println!("{summary}");
    std::fs::write(output.join(SUMMARY_FILE), summary).unwrap();
}
