//! Measures the strides of the mobiles' walk and run clips with the RIFE flow on the GPU and stores them in the
//! animation packages (`metadata/strides.bin`, `udd_assets::animation::mobile_strides`): every body's walk and run
//! clips, peace and war (where the animation sequence gives war moves of their own), and an Enhanced mount's mounted
//! moves. The strides are keyed by body and action, so a war mode reads its own moves without new strides, and each
//! carries the hash of the frames measured, so the app takes it only for those frames.
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
//! shorter than [`MIN_STRIDE_TILES`], gets none (its clip keeps its own timing). The method was chosen on a PyTorch
//! prototype of the same steps.
//!
//! Run as `dynamapper measure-strides changed|full` (`just measure-mobile-strides`, [`USAGE`]): changed hashes every
//! clip and measures only those whose frames are new or changed since their stride was measured, keeping the others
//! and dropping the strides of clips gone; full measures every clip again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use bevy::math::IVec2;
use color_eyre::eyre::{self, WrapErr};
use udd_assets::animation::mobile_anim_cc::MobileAnimCcPackage;
use udd_assets::animation::mobile_anim_ec::MobileAnimEcPackage;
use udd_assets::animation::mobile_strides::{
    format_stride_table, plan_strides, sort_stride_records, stride_clip_hashes, stride_table_fields,
    write_package_strides, MobileAnimClips, MobileClip, MobileStrideRecord, StrideSource, STRIDE_FACINGS,
    STRIDE_FACING_COUNT,
};
use crate::render::mobiles::rife::{RifeModel, RifeWorkspace, RIFE_FLOW_CHANNELS, RIFE_IMAGE_CHANNELS,
    RIFE_SIZE_MULTIPLE};
use crate::world::mobile_motion::direction_of_step;

/// The command line's first argument that runs the measurement instead of the app.
pub const COMMAND: &str = "measure-strides";
pub const USAGE: &str = "\
usage: dynamapper measure-strides <changed|full> [--source cc|ec] [--packages DIR] [--output DIR] [--report DIR]
  changed          measure the clips whose frames are new or changed since their stride; keep the rest
  full             measure every clip again
  --source cc|ec   only the Classic (mobile_anim_cc.uddp) or the Enhanced (mobile_anim_ec.uddp) package
  --packages DIR   the folder of the packages (default: udd_path of the app's settings)
  --output DIR     write the packages there (default: in place, through a .partial file beside each)
  --report DIR     write each package's strides there as a text table, mobile_strides_<cc|ec>.tsv";
const MODE_CHANGED: &str = "changed";
const MODE_FULL: &str = "full";
const OPTION_SOURCE: &str = "--source";
const OPTION_PACKAGES: &str = "--packages";
const OPTION_OUTPUT: &str = "--output";
const OPTION_REPORT: &str = "--report";
const CLASSIC_PACKAGE: &str = "mobile_anim_cc.uddp";
const ENHANCED_PACKAGE: &str = "mobile_anim_ec.uddp";
const REPORT_PREFIX: &str = "mobile_strides_";
const REPORT_EXTENSION: &str = "tsv";
/// Clips between progress lines.
const PROGRESS_EVERY: usize = 500;
const MILLIS_PER_SEC: f32 = 1000.0;

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

fn gpu() -> eyre::Result<Gpu> {
    bevy::tasks::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.wrap_err("no GPU adapter")?;
        let info = adapter.get_info();
        println!("adapter: {} ({:?}, {})", info.name, info.backend, info.driver_info);
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(), ..Default::default()
        }).await.wrap_err("no GPU device")?;
        Ok(Gpu { device, queue })
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
    fn new() -> eyre::Result<Self> {
        let gpu = gpu()?;
        let model = RifeModel::new(&gpu.device).map_err(|error| eyre::eyre!("RIFE: {error}"))?;
        Ok(Self { gpu, model, workspaces: Vec::new() })
    }

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
    fn travel_px(&mut self, clip: &MobileClip, ux: f32, uy: f32) -> Option<f32> {
        let frames = &clip.frames;
        let count = frames.len();
        if count < 2 {
            return None;
        }
        let left = frames.iter().map(|frame| -frame.anchor_x).min()? - CANVAS_MARGIN;
        let top = frames.iter().map(|frame| -frame.anchor_y).min()? - CANVAS_MARGIN;
        let right = frames.iter().map(|frame| i32::from(frame.width) - frame.anchor_x).max()? + CANVAS_MARGIN;
        let bottom = frames.iter().map(|frame| i32::from(frame.height) - frame.anchor_y).max()? + CANVAS_MARGIN;
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
            let (frame_width, frame_height) = (usize::from(frame.width), usize::from(frame.height));
            let x0 = (-frame.anchor_x - left) as usize;
            let y0 = (-frame.anchor_y - top) as usize;
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
    facings: [Option<f32>; STRIDE_FACING_COUNT],
    /// How far the facings' strides spread (largest minus smallest) as a fraction of their median, for the log.
    spread: Option<f32>,
    stride: Result<f32, &'static str>,
}

fn measure(measurer: &mut Measurer, clips: &[Option<MobileClip>; STRIDE_FACING_COUNT]) -> Measurement {
    let facings: [Option<f32>; STRIDE_FACING_COUNT] = std::array::from_fn(|index| {
        let clip = clips[index].as_ref()?;
        let (mut ux, uy, px_per_tile) = travel_direction(STRIDE_FACINGS[index]);
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

/// What the command line asks for.
#[derive(Debug, PartialEq)]
struct Options {
    /// Measure every clip again, not only the changed and new ones.
    everything: bool,
    sources: Vec<StrideSource>,
    /// `None`: the app's settings say.
    packages: Option<PathBuf>,
    output: Option<PathBuf>,
    report: Option<PathBuf>,
}

fn parse_options(args: &[String]) -> eyre::Result<Options> {
    let mut args = args.iter();
    let everything = match args.next().map(String::as_str) {
        Some(MODE_CHANGED) => false,
        Some(MODE_FULL) => true,
        Some(other) => eyre::bail!("unknown mode `{other}`"),
        None => eyre::bail!("no mode"),
    };
    let mut options = Options { everything, sources: StrideSource::ALL.to_vec(), packages: None, output: None,
        report: None };
    while let Some(option) = args.next() {
        let mut value = || args.next().ok_or_else(|| eyre::eyre!("{option} needs a value"));
        match option.as_str() {
            OPTION_SOURCE => {
                let token = value()?;
                options.sources = vec![StrideSource::from_token(token)
                    .ok_or_else(|| eyre::eyre!("unknown source `{token}`"))?];
            }
            OPTION_PACKAGES => options.packages = Some(PathBuf::from(value()?)),
            OPTION_OUTPUT => options.output = Some(PathBuf::from(value()?)),
            OPTION_REPORT => options.report = Some(PathBuf::from(value()?)),
            other => eyre::bail!("unknown option `{other}`"),
        }
    }
    Ok(options)
}

/// Runs `dynamapper measure-strides` with the arguments after the command.
pub fn run(args: &[String]) -> ExitCode {
    let options = match parse_options(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match measure_packages(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("measure-strides: {error:?}");
            ExitCode::FAILURE
        }
    }
}

fn measure_packages(options: &Options) -> eyre::Result<()> {
    let started = Instant::now();
    let folder = match &options.packages {
        Some(folder) => folder.clone(),
        None => PathBuf::from(crate::settings::load_from_files()?.runtime_assets.udd_path),
    };
    let mut measurer = None;
    for &source in &options.sources {
        let name = match source {
            StrideSource::Classic => CLASSIC_PACKAGE,
            StrideSource::Enhanced => ENHANCED_PACKAGE,
        };
        let path = folder.join(name);
        if !path.is_file() {
            println!("{}: no {}, skipped", source.label(), path.display());
            continue;
        }
        let output = options.output.as_ref().map_or_else(|| path.clone(), |folder| folder.join(name));
        let loading = Instant::now();
        match source {
            StrideSource::Classic => measure_package(MobileAnimCcPackage::load(&path)?, &path, &output, loading,
                options, &mut measurer),
            StrideSource::Enhanced => measure_package(MobileAnimEcPackage::load(&path)?, &path, &output, loading,
                options, &mut measurer),
        }.wrap_err_with(|| format!("measure the strides of {}", path.display()))?;
    }
    println!("done in {:.1} s", started.elapsed().as_secs_f32());
    Ok(())
}

/// Measures the strides of `package`, read from `path`, and writes them to `output` with the package.
fn measure_package<P: MobileAnimClips + Sync>(package: P, path: &Path, output: &Path, loading: Instant,
    options: &Options, measurer: &mut Option<Measurer>) -> eyre::Result<()> {
    let source = package.stride_source();
    let label = source.label();
    let load_secs = loading.elapsed().as_secs_f32();

    let hashing = Instant::now();
    let clips = package.stride_clips();
    let threads = std::thread::available_parallelism().map_or(1, |threads| threads.get());
    let slowest = Mutex::new((Duration::ZERO, 0));
    let hashes = stride_clip_hashes(&package, &clips, threads, |done, index, took| {
        let mut slowest = slowest.lock().unwrap();
        if took > slowest.0 {
            *slowest = (took, index);
        }
        if done % PROGRESS_EVERY == 0 {
            println!("{label}: hashed {done} of {} clips", clips.len());
        }
    });
    let current = clips.iter().zip(hashes)
        .map(|(&(body, action), hash)| Ok((body, action, hash.wrap_err_with(|| format!("body {body} action {action}"))?)))
        .collect::<eyre::Result<Vec<_>>>()?;
    let hash_secs = hashing.elapsed().as_secs_f32();
    let (slowest_took, slowest_index) = slowest.into_inner().unwrap();

    let existing = package.stride_records().to_vec();
    let plan = plan_strides(&current, &existing, options.everything);
    // Clips sharing their frames share a measurement: those kept give theirs to the changed and new.
    let mut known: HashMap<u64, ([Option<f32>; STRIDE_FACING_COUNT], Option<f32>)> = plan.unchanged.iter()
        .map(|record| (record.clip_hash, (record.facing_tiles, record.stride_tiles))).collect();
    let measuring = Instant::now();
    let (mut measured, mut shared) = (0, 0);
    let mut records = plan.unchanged.clone();
    for &(body, action, clip_hash) in &plan.to_measure {
        let (facing_tiles, stride_tiles) = match known.get(&clip_hash) {
            Some(&known) => {
                shared += 1;
                known
            }
            None => {
                let measurer = match measurer {
                    Some(measurer) => measurer,
                    None => measurer.insert(Measurer::new()?),
                };
                let measurement = measure(measurer, &package.side_clips(body, action)?);
                measured += 1;
                let record = MobileStrideRecord { body_id: body, action_id: action, clip_hash,
                    stride_tiles: measurement.stride.ok(), facing_tiles: measurement.facings };
                let spread = measurement.spread.map_or("-".to_string(), |spread| format!("{spread:.2}"));
                let verdict = measurement.stride.err().unwrap_or("ok");
                println!("{}\t{spread}\t{verdict}", stride_table_fields(source, &record));
                known.insert(clip_hash, (record.facing_tiles, record.stride_tiles));
                (record.facing_tiles, record.stride_tiles)
            }
        };
        records.push(MobileStrideRecord { body_id: body, action_id: action, clip_hash, stride_tiles, facing_tiles });
    }
    let measure_secs = measuring.elapsed().as_secs_f32();
    sort_stride_records(&mut records);
    let with_stride = records.iter().filter(|record| record.stride_tiles.is_some()).count();
    let summary = format!("{label}: {} clips, {} without frames; {} unchanged, {} changed, {} new, {} removed; \
        {measured} measured, {shared} sharing the frames of another; {with_stride} with a stride, {} without",
        clips.len(), plan.without_frames, plan.unchanged.len(), plan.changed, plan.new, plan.removed.len(),
        records.len() - with_stride);
    let (slowest_body, slowest_action) = clips.get(slowest_index).copied().unwrap_or_default();
    let timing = format!("{label}: loaded in {load_secs:.1} s, hashed in {hash_secs:.1} s on {threads} threads \
        (slowest: body {slowest_body} action {slowest_action}, {:.1} ms), measured in {measure_secs:.1} s",
        slowest_took.as_secs_f32() * MILLIS_PER_SEC);
    println!("{summary}\n{timing}");

    if let Some(folder) = &options.report {
        std::fs::create_dir_all(folder).wrap_err_with(|| format!("create {}", folder.display()))?;
        let report = folder.join(format!("{REPORT_PREFIX}{}.{REPORT_EXTENSION}", source.token()));
        let comments = [format!("The strides of {}, in tiles per cycle (dynamapper {COMMAND}).", path.display()),
            summary];
        std::fs::write(&report, format_stride_table(source, &records, &comments))
            .wrap_err_with(|| format!("write {}", report.display()))?;
    }
    drop(package);
    if records == existing && output == path {
        println!("{label}: the strides are unchanged; {} is left as it is", path.display());
        return Ok(());
    }
    if let Some(folder) = output.parent() {
        std::fs::create_dir_all(folder).wrap_err_with(|| format!("create {}", folder.display()))?;
    }
    let writing = Instant::now();
    write_package_strides(path, output, &records)?;
    println!("{label}: wrote {} in {:.1} s", output.display(), writing.elapsed().as_secs_f32());
    Ok(())
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

    #[test]
    fn the_command_line_reads_a_mode_and_options() {
        let args = |line: &str| line.split_whitespace().map(String::from).collect::<Vec<_>>();
        let options = parse_options(&args("changed --source ec --packages /p --report /r")).unwrap();
        assert_eq!(options, Options { everything: false, sources: vec![StrideSource::Enhanced],
            packages: Some("/p".into()), output: None, report: Some("/r".into()) });
        assert_eq!(parse_options(&args("full")).unwrap().sources, StrideSource::ALL.to_vec());
        for bad in ["", "some", "full --source kr", "full --output", "full --colour red"] {
            assert!(parse_options(&args(bad)).is_err(), "{bad}");
        }
    }
}
