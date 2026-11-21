//! Writes the strides table, `assets/data/mobile_strides.txt`, from the Classic and Enhanced animation packages: the
//! stride of every body's walk and run clips, peace and war (where the animation sequence gives war moves of their own),
//! and of an Enhanced mount's mounted moves, with the RIFE flow on the GPU. The table is keyed by body and action, so a
//! war mode reads its own moves without a new table.
//!
//! For each pair of frames of a clip (the last with the first too), RIFE's flow at t = 0.5 gives each pixel of the
//! in-between its motion from the first frame to the second (the flow to the second minus the flow to the first).
//! The frames sit on their anchor, so the body stays put and its planted feet move backward at its speed: of the
//! opaque pixels (alpha over one half) in the lowest fifth of the silhouette, the most backward motion along the
//! facing's direction of travel (its 10th percentile) is the body's travel for the pair. Summed over the cycle and
//! divided by the screen pixels of a tile in that direction, it is the stride in tiles per cycle. Only the side facings
//! (south, south-west, west) measure well, toward and away from the camera the feet barely move on screen. The median
//! of those that measured is the stride, even when they disagree (the run-time clamp to half and twice the clip's own
//! cycle bounds a poor measurement); a clip with fewer than [`MIN_FACINGS`] of them, or whose stride is not positive or
//! shorter than [`MIN_STRIDE_TILES`], gets none (its clip keeps its own timing). The report lists each facing's stride
//! and their spread for inspection. The method was chosen on a PyTorch prototype of the same steps.
//!
//! Run, with the packages' folder (holding `mobile_anim_cc.uddp` and `mobile_anim_ec.uddp`) and optionally a folder
//! for the per-facing report:
//!
//! ```sh
//! MOBILE_STRIDE_PACKAGES=/path/to/uddp MOBILE_STRIDE_REPORT=/path/to/report \
//!     cargo test -p dynamapper -- --ignored --exact render::mobiles::pace::measure::write_the_stride_table --nocapture
//! ```

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};
use bevy::math::IVec2;
use udd_assets::animation::mobile_anim_cc::{self as anim_cc, MobileAnimCcPackage};
use udd_assets::animation::mobile_anim_ec::{self as anim_ec, MobileAnimEcPackage};
use crate::render::mobiles::clips::{decode_clip, AnimPackages, AnimSource, ClipKey, DecodedClip};
use crate::render::mobiles::rife::{RifeModel, RifeWorkspace, RIFE_FLOW_CHANNELS, RIFE_IMAGE_CHANNELS,
    RIFE_SIZE_MULTIPLE};
use crate::render::mobiles::placement::FramePlacement;
use crate::world::mobile_motion::direction_of_step;
use super::{stride_line, BUNDLED_STRIDES_NAME};

const PACKAGES_VARIABLE: &str = "MOBILE_STRIDE_PACKAGES";
const REPORT_VARIABLE: &str = "MOBILE_STRIDE_REPORT";
const CLASSIC_PACKAGE: &str = "mobile_anim_cc.uddp";
const ENHANCED_PACKAGE: &str = "mobile_anim_ec.uddp";
const REPORT_FILE: &str = "mobile_strides_report.tsv";

/// The facings measured: south, south-west and west, across the screen.
const SIDE_FACINGS: [u8; 3] = [4, 5, 6];
/// Facings that must measure for a stride.
const MIN_FACINGS: usize = 2;
/// The shortest stride taken. Bodies without planted feet (floating, slithering) measure from a few thousandths to a
/// tenth or two of a tile, as do small creatures whose feet are a few pixels; paced by that, they would play their
/// clips at the fastest the clamp allows, twice their own speed. Most walking bodies measure half a tile to three.
const MIN_STRIDE_TILES: f32 = 0.25;
/// Half a tile's screen width and height: a step along UO x moves (22, 22) pixels, along UO y (-22, 22).
const HALF_TILE_PX: f32 = 22.0;
/// Transparent border around a clip's frames, room for the flow near their edges.
const CANVAS_MARGIN: i32 = 8;
/// The in-between whose flow is read, and the workspace slot it runs in.
const MEASURE_TIMESTEP: f32 = 0.5;
const MEASURE_SLOT: usize = 0;
/// The grey RIFE sees behind a frame (its RGB-over-grey channels).
const BACKDROP_GREY: f32 = 0.5;
/// A pixel of the in-between counts as the body above this alpha.
const OPAQUE_ALPHA: f32 = 0.5;
/// The lowest fraction of the silhouette's height whose motion is read: the feet.
const FEET_BAND: f32 = 0.2;
/// The percentile of the feet's motion along the travel taken as the body's travel: the most backward, the planted
/// feet, without the outliers.
const TRAVEL_PERCENTILE: f32 = 10.0;
const PERCENT: f32 = 100.0;
const MAX_CHANNEL: f32 = 255.0;
const RGBA_BYTES: usize = 4;
const F32_BYTES: u64 = 4;
/// Workspaces kept for reuse, by frame size.
const MAX_CACHED_WORKSPACES: usize = 4;
const POLL_TIMEOUT: Duration = Duration::from_secs(60);

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Gpu {
    bevy::tasks::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.expect("a GPU adapter");
        let info = adapter.get_info();
        println!("adapter: {} ({:?}, {})", info.name, info.backend, info.driver_info);
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(), ..Default::default()
        }).await.expect("a GPU device");
        Gpu { device, queue }
    })
}

/// The unit screen direction (x right, y down) a body facing `facing` travels in, and the screen pixels of a tile
/// that way.
fn travel_direction(facing: u8) -> (f32, f32, f32) {
    let delta = [-1, 0, 1].into_iter()
        .flat_map(|x| [-1, 0, 1].map(|y| IVec2::new(x, y)))
        .find(|&delta| direction_of_step(delta) == Some(facing))
        .expect("a facing below DIRECTION_COUNT");
    let x = HALF_TILE_PX * (delta.x - delta.y) as f32;
    let y = HALF_TILE_PX * (delta.x + delta.y) as f32;
    let length = x.hypot(y);
    (x / length, y / length, length)
}

/// The `percentile` (0..100) of `values`, interpolated between neighbours as numpy's default does.
fn percentile(values: &mut [f32], percentile: f32) -> f32 {
    values.sort_unstable_by(f32::total_cmp);
    let rank = percentile / PERCENT * (values.len() - 1) as f32;
    let (low, high) = (rank.floor() as usize, rank.ceil() as usize);
    values[low] + (values[high] - values[low]) * (rank - low as f32)
}

/// The workspaces and the readback of the measurements.
struct Measurer {
    gpu: Gpu,
    model: RifeModel,
    workspaces: Vec<((u32, u32), RifeWorkspace)>,
}

impl Measurer {
    /// The workspace of frames of this size, kept by its padded size.
    fn workspace(&mut self, width: u32, height: u32) -> &RifeWorkspace {
        let (width, height) = (width.div_ceil(RIFE_SIZE_MULTIPLE) * RIFE_SIZE_MULTIPLE,
            height.div_ceil(RIFE_SIZE_MULTIPLE) * RIFE_SIZE_MULTIPLE);
        let index = match self.workspaces.iter().position(|(size, _)| *size == (width, height)) {
            Some(index) => index,
            None => {
                if self.workspaces.len() >= MAX_CACHED_WORKSPACES {
                    self.workspaces.remove(0);
                }
                let workspace = RifeWorkspace::new(&self.model, &self.gpu.device, width, height).unwrap();
                workspace.set_timestep(&self.gpu.queue, MEASURE_SLOT, MEASURE_TIMESTEP);
                self.workspaces.push(((width, height), workspace));
                self.workspaces.len() - 1
            }
        };
        &self.workspaces[index].1
    }

    /// The body's travel over one cycle of `clip`, in screen pixels along `(ux, uy)` in the clip's stored frames;
    /// none when an in-between shows no body.
    fn travel_px(&mut self, clip: &DecodedClip, ux: f32, uy: f32) -> Option<f32> {
        let frames = &clip.frames;
        let count = frames.len();
        if count < 2 {
            return None;
        }
        let edges = |near: fn(&FramePlacement) -> f32| frames.iter()
            .map(move |frame| near(&frame.placement).round() as i32);
        let left = edges(|p| -p.anchor_x).min()? - CANVAS_MARGIN;
        let top = edges(|p| -p.anchor_y).min()? - CANVAS_MARGIN;
        let right = edges(|p| p.width - p.anchor_x).max()? + CANVAS_MARGIN;
        let bottom = edges(|p| p.height - p.anchor_y).max()? + CANVAS_MARGIN;
        let (width, height) = {
            let workspace = self.workspace((right - left) as u32, (bottom - top) as u32);
            workspace.size()
        };
        let plane = (width * height) as usize;
        let image_floats = RIFE_IMAGE_CHANNELS as usize * plane;

        // Every frame's channels on the canvas: RGB over grey, premultiplied RGB, alpha. The grey covers the whole
        // canvas, not the padding, as in the prototype: grey on each frame's own rectangle alone changes the flow
        // near the feet.
        let mut images = vec![0f32; count * image_floats];
        for (frame, image) in frames.iter().zip(images.chunks_exact_mut(image_floats)) {
            for channel in 0..3 {
                for row in image[channel * plane..][..plane].chunks_exact_mut(width as usize)
                    .take((bottom - top) as usize) {
                    row[..(right - left) as usize].fill(BACKDROP_GREY);
                }
            }
            let p = frame.placement;
            let (frame_width, frame_height) = (p.width as usize, p.height as usize);
            let x0 = (-p.anchor_x.round() as i32 - left) as usize;
            let y0 = (-p.anchor_y.round() as i32 - top) as usize;
            for y in 0..frame_height {
                for x in 0..frame_width {
                    let texel = &frame.rgba[(y * frame_width + x) * RGBA_BYTES..][..RGBA_BYTES];
                    let alpha = f32::from(texel[3]) / MAX_CHANNEL;
                    let at = (y0 + y) * width as usize + x0 + x;
                    for channel in 0..3 {
                        let premultiplied = f32::from(texel[channel]) / MAX_CHANNEL * alpha;
                        image[channel * plane + at] = premultiplied + BACKDROP_GREY * (1.0 - alpha);
                        image[(3 + channel) * plane + at] = premultiplied;
                    }
                    image[(RIFE_IMAGE_CHANNELS as usize - 1) * plane + at] = alpha;
                }
            }
        }

        let device = &self.gpu.device;
        let upload = wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor {
            label: Some("stride frames"), contents: bytemuck::cast_slice(&images), usage: wgpu::BufferUsages::COPY_SRC,
        });
        let workspace = &self.workspaces.iter().find(|(size, _)| *size == (width, height)).unwrap().1;
        let pair_bytes = workspace.flow_and_alpha_bytes();
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stride flow"), size: pair_bytes * count as u64, mapped_at_creation: false,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        });
        let image_bytes = image_floats as u64 * F32_BYTES;
        let mut encoder = device.create_command_encoder(&Default::default());
        for index in 0..count {
            let next = (index + 1) % count;
            encoder.copy_buffer_to_buffer(&upload, index as u64 * image_bytes, workspace.input(), 0, image_bytes);
            encoder.copy_buffer_to_buffer(&upload, next as u64 * image_bytes, workspace.input(), image_bytes,
                image_bytes);
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                workspace.encode().iter().chain(workspace.interpolate(MEASURE_SLOT))
                    .for_each(|dispatch| dispatch.record(&mut pass));
            }
            workspace.copy_flow_and_alpha(&mut encoder, &readback, index as u64 * pair_bytes);
        }
        self.gpu.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| sender.send(result).unwrap());
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(POLL_TIMEOUT) }).unwrap();
        receiver.recv().unwrap().unwrap();
        let values: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec();
        readback.unmap();

        let mut travel = 0.0;
        let mut along = Vec::new();
        for pair in values.chunks_exact((pair_bytes / F32_BYTES) as usize) {
            let [to_first_x, to_first_y, to_second_x, to_second_y] =
                std::array::from_fn(|channel| &pair[channel * plane..][..plane]);
            let alpha = &pair[RIFE_FLOW_CHANNELS as usize * plane..][..plane];
            let rows = (0..plane).filter(|&at| alpha[at] > OPAQUE_ALPHA).map(|at| at / width as usize);
            let (top, bottom) = rows.fold((usize::MAX, 0), |(top, bottom), row| (top.min(row), bottom.max(row)));
            if top > bottom {
                return None;
            }
            let feet_from = bottom as f32 - FEET_BAND * (bottom - top) as f32;
            along.clear();
            along.extend((0..plane)
                .filter(|&at| alpha[at] > OPAQUE_ALPHA && (at / width as usize) as f32 >= feet_from)
                .map(|at| (to_second_x[at] - to_first_x[at]) * ux + (to_second_y[at] - to_first_y[at]) * uy));
            travel -= percentile(&mut along, TRAVEL_PERCENTILE);
        }
        Some(travel)
    }
}

/// A clip's measurement: the stride of each side facing that measured, and the verdict.
struct Measurement {
    facings: [Option<f32>; SIDE_FACINGS.len()],
    /// How far the facings' strides spread (largest minus smallest) as a fraction of their median, for the report.
    spread: Option<f32>,
    stride: Result<f32, &'static str>,
}

fn measure(measurer: &mut Measurer, clips: &[Option<DecodedClip>; SIDE_FACINGS.len()]) -> Measurement {
    let facings: [Option<f32>; SIDE_FACINGS.len()] = std::array::from_fn(|index| {
        let clip = clips[index].as_ref()?;
        let (mut ux, uy, px_per_tile) = travel_direction(SIDE_FACINGS[index]);
        if clip.mirrored {
            ux = -ux;
        }
        measurer.travel_px(clip, ux, uy).map(|travel| travel / px_per_tile)
    });
    let mut strides: Vec<f32> = facings.iter().flatten().copied().collect();
    if strides.len() < MIN_FACINGS {
        return Measurement { facings, spread: None, stride: Err("too few facings") };
    }
    let median = percentile(&mut strides, PERCENT / 2.0);
    let spread = (median > 0.0).then(|| (strides[strides.len() - 1] - strides[0]) / median);
    let stride = if median <= 0.0 {
        Err("not positive")
    } else if median < MIN_STRIDE_TILES {
        Err("too short")
    } else {
        Ok(median)
    };
    Measurement { facings, spread, stride }
}

/// A hash of the clips' frames: bodies and actions showing the same frames share one measurement.
fn frames_hash(clips: &[Option<DecodedClip>]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for clip in clips {
        match clip {
            Some(clip) => {
                clip.mirrored.hash(&mut hasher);
                for frame in &clip.frames {
                    let p = frame.placement;
                    [p.width, p.height, p.anchor_x, p.anchor_y].map(f32::to_bits).hash(&mut hasher);
                    frame.rgba.hash(&mut hasher);
                }
            }
            None => 0u8.hash(&mut hasher),
        }
    }
    hasher.finish()
}

/// The clips to measure: each server body and the actions of its walk and run (and, Enhanced, of its moves as a
/// mount), as the runtime asks for them.
fn classic_clips(package: &MobileAnimCcPackage) -> Vec<(u32, u16)> {
    let bodies: BTreeSet<u16> = package.animations().iter().map(|record| record.body_id)
        .chain(package.body_resolve().iter().map(|record| record.body_id))
        .chain(package.body_types().iter().map(|record| record.body_id))
        .collect();
    let mut clips = BTreeSet::new();
    for body in bodies {
        // A body with Enhanced frames has the animation sequence's war moves too.
        let sequence = package.body_type_record(package.frames_body(body).type_body_id);
        for movement in [anim_cc::MOVE_WALK, anim_cc::MOVE_RUN] {
            let war = sequence.and_then(|record| record.move_action(anim_cc::GROUP_WAR, movement));
            for action in [package.movement_action(body, movement, false), war].into_iter().flatten() {
                clips.insert((u32::from(body), action));
            }
        }
    }
    clips.into_iter().collect()
}

fn enhanced_clips(package: &MobileAnimEcPackage) -> Vec<(u32, u16)> {
    let mut clips = BTreeSet::new();
    for body in package.bodies() {
        for movement in [anim_ec::MOVE_WALK, anim_ec::MOVE_RUN] {
            let actions = [body.move_action(anim_ec::GROUP_PEACE, movement), body.move_action(anim_ec::GROUP_WAR, movement),
                body.mount_move_action(movement)];
            for action in actions.into_iter().flatten() {
                clips.insert((body.body_id, action));
            }
        }
    }
    clips.into_iter().collect()
}

const SOURCE_NAMES: [(AnimSource, &str); 2] = [(AnimSource::Classic, "Classic"), (AnimSource::Enhanced, "Enhanced")];

#[test]
#[ignore = "requires a GPU adapter and the animation packages in MOBILE_STRIDE_PACKAGES; run explicitly to write \
            assets/data/mobile_strides.txt"]
fn write_the_stride_table() {
    let folder = std::path::PathBuf::from(std::env::var_os(PACKAGES_VARIABLE)
        .unwrap_or_else(|| panic!("set {PACKAGES_VARIABLE} to the folder of the animation packages")));
    let started = Instant::now();
    let classic = Arc::new(MobileAnimCcPackage::load(folder.join(CLASSIC_PACKAGE)).unwrap());
    let enhanced = Arc::new(MobileAnimEcPackage::load(folder.join(ENHANCED_PACKAGE)).unwrap());
    let packages = AnimPackages { classic: Some(classic.clone()), enhanced: Some(enhanced.clone()) };
    let gpu = gpu();
    let model = RifeModel::new(&gpu.device).unwrap();
    let mut measurer = Measurer { gpu, model, workspaces: Vec::new() };

    let mut table = String::new();
    let mut report = String::from("source\tbody\taction\tsouth\tsouth-west\twest\tspread\tstride\tverdict\n");
    let mut summary = Vec::new();
    for (source, name) in SOURCE_NAMES {
        let source_started = Instant::now();
        let clips = match source {
            AnimSource::Classic => classic_clips(&classic),
            AnimSource::Enhanced => enhanced_clips(&enhanced),
        };
        let mut measured: HashMap<u64, Measurement> = HashMap::new();
        let (mut with_stride, mut rejected, mut without_frames) = (0, 0, 0);
        for &(body, action) in &clips {
            let decoded: [Option<DecodedClip>; SIDE_FACINGS.len()] = std::array::from_fn(|index| {
                decode_clip(&packages, ClipKey { source, body, action, direction: SIDE_FACINGS[index] })
            });
            if decoded.iter().all(Option::is_none) {
                without_frames += 1;
                continue;
            }
            let measurement = measured.entry(frames_hash(&decoded))
                .or_insert_with(|| measure(&mut measurer, &decoded));
            let facings = measurement.facings.map(|tiles| tiles.map_or("-".to_string(), |tiles| format!("{tiles:.3}")));
            let (stride, verdict) = match measurement.stride {
                Ok(tiles) => {
                    with_stride += 1;
                    writeln!(table, "{}", stride_line(source, body, action, tiles)).unwrap();
                    (format!("{tiles:.3}"), "ok")
                }
                Err(reason) => {
                    rejected += 1;
                    ("-".to_string(), reason)
                }
            };
            let spread = measurement.spread.map_or("-".to_string(), |spread| format!("{spread:.2}"));
            let row = format!("{name}\t{body}\t{action}\t{}\t{spread}\t{stride}\t{verdict}", facings.join("\t"));
            println!("{row}");
            writeln!(report, "{row}").unwrap();
        }
        let line = format!("{name}: {} clips: {with_stride} with a stride, {rejected} rejected, {without_frames} \
            without frames; {} distinct frame sets measured in {:.0} s", clips.len(), measured.len(),
            source_started.elapsed().as_secs_f32());
        println!("{line}");
        summary.push(line);
    }

    let header = format!("# The strides of the mobiles' walk and run clips, in tiles per cycle (render/mobiles/pace.rs):\n\
        # a line per clip, `source body action tiles`, source `cc` (Classic) or `ec` (Enhanced), the server body and the\n\
        # action the clients' tables give for the movement. Measured with RIFE flow on the frames by\n\
        # render/mobiles/pace/measure.rs, which says how; regenerate with\n\
        #   {PACKAGES_VARIABLE}=<folder of {CLASSIC_PACKAGE} and {ENHANCED_PACKAGE}> \\\n\
        #     cargo test -p dynamapper -- --ignored --exact render::mobiles::pace::measure::write_the_stride_table\n\
        # {}\n", summary.join("\n# "));
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(BUNDLED_STRIDES_NAME);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, header + &table).unwrap();
    println!("wrote {} in {:.0} s", path.display(), started.elapsed().as_secs_f32());
    if let Some(report_dir) = std::env::var_os(REPORT_VARIABLE).map(std::path::PathBuf::from) {
        std::fs::create_dir_all(&report_dir).unwrap();
        std::fs::write(report_dir.join(REPORT_FILE), report).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_side_facings_travel_across_the_screen_a_tile_at_a_time() {
        let (x, y, px) = travel_direction(5);
        assert!((x + 1.0).abs() < 1e-6 && y.abs() < 1e-6 && (px - 2.0 * HALF_TILE_PX).abs() < 1e-4);
        let (x, y, px) = travel_direction(4);
        assert!((x + y).abs() < 1e-6 && y > 0.0 && (px - 2.0f32.sqrt() * HALF_TILE_PX).abs() < 1e-4);
        let (x, y, _) = travel_direction(6);
        assert!((x - y).abs() < 1e-6 && y < 0.0);
    }

    #[test]
    fn a_percentile_interpolates_as_numpy() {
        let mut values = [4.0, 1.0, 3.0, 2.0];
        assert!((percentile(&mut values, 10.0) - 1.3).abs() < 1e-6);
        assert!((percentile(&mut values, 50.0) - 2.5).abs() < 1e-6);
    }
}
