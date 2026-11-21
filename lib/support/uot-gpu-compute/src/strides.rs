//! Measures how far a walking body's planted feet travel in one cycle of its animation (its stride), with the RIFE
//! flow on the GPU. The frames come as plain RGBA with their anchors; the udd tools feed it the walk and run clips of
//! the animation packages (`udd_conv::animation::mobile_strides`), which store the strides for Dynamapper to pace the
//! cycles by.
//!
//! For each pair of frames of a clip (the last with the first too), RIFE's flow at t = 0.5 gives each pixel of the
//! in-between its motion from the first frame to the second (the flow to the second minus the flow to the first).
//! The frames sit on their anchor, so the body stays put and its planted feet move backward at its speed: of the
//! opaque pixels (alpha over one half) in the lowest fifth of the silhouette, the most backward motion along the
//! facing's direction of travel (its 10th percentile) is the body's travel for the pair. Summed over the cycle and
//! divided by the screen pixels of a tile in that direction, it is the stride in tiles per cycle. Only the side facings
//! (south, south-west, west) measure well, toward and away from the camera the feet barely move on screen. The median
//! of the facings that measured is the stride, even when they disagree (the run-time clamp to half and twice the
//! clip's own cycle bounds a poor measurement); a clip with fewer than [`MIN_FACINGS`] of them, or whose stride is not
//! positive or shorter than [`MIN_STRIDE_TILES`], gets none (its clip keeps its own timing). The method was chosen on
//! a PyTorch prototype of the same steps.

use std::time::Duration;
use color_eyre::eyre::{self, WrapErr};
use crate::rife::{RifeModel, RifeWorkspace, RIFE_FLOW_CHANNELS, RIFE_IMAGE_CHANNELS, RIFE_SIZE_MULTIPLE};

/// Facings that must measure for a stride.
pub const MIN_FACINGS: usize = 2;
/// The shortest stride taken. Bodies without planted feet (floating, slithering) measure from a few thousandths to a
/// tenth or two of a tile, as do small creatures whose feet are a few pixels; paced by that, they would play their
/// clips at the fastest the clamp allows, twice their own speed. Most walking bodies measure half a tile to three.
pub const MIN_STRIDE_TILES: f32 = 0.25;
/// Half a tile's screen width and height: a step along UO x moves (22, 22) pixels, along UO y (-22, 22).
const HALF_TILE_PX: f32 = 22.0;
/// Facings, 0 north and clockwise, as the clients number them.
pub const FACING_COUNT: u8 = 8;
/// The step in UO x and y of each facing.
const FACING_STEPS: [(i32, i32); FACING_COUNT as usize] =
    [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];
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

/// A device of the most powerful adapter, and the adapter's description.
fn gpu() -> eyre::Result<(Gpu, String)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.wrap_err("no GPU adapter")?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?}, {})", info.name, info.backend, info.driver_info);
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(), ..Default::default()
        }).await.wrap_err("no GPU device")?;
        Ok((Gpu { device, queue }, adapter_name))
    })
}

/// The unit screen direction (x right, y down) a body facing `facing` travels in, and the screen pixels of a tile
/// that way.
fn travel_direction(facing: u8) -> (f32, f32, f32) {
    let (step_x, step_y) = FACING_STEPS[usize::from(facing)];
    let x = HALF_TILE_PX * (step_x - step_y) as f32;
    let y = HALF_TILE_PX * (step_x + step_y) as f32;
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

/// A frame of a clip: its size, its anchor (the point on the ground, from the frame's top-left corner) and its
/// straight-alpha RGBA rows, top down.
#[derive(Debug, Clone, Copy)]
pub struct StrideFrame<'a> {
    pub width: u16,
    pub height: u16,
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub rgba: &'a [u8],
}

/// One cycle of a body moving toward `facing` (below [`FACING_COUNT`]): its frames in order, drawn mirrored
/// horizontally about the anchor when `mirrored`.
#[derive(Debug, Clone)]
pub struct StrideClip<'a> {
    pub facing: u8,
    pub mirrored: bool,
    pub frames: Vec<StrideFrame<'a>>,
}

/// The GPU device, the RIFE model, and the workspaces of the frame sizes measured last.
pub struct StrideMeasurer {
    gpu: Gpu,
    adapter: String,
    model: RifeModel,
    workspaces: Vec<((u32, u32), RifeWorkspace)>,
}

impl StrideMeasurer {
    /// Starts a device of its own and compiles RIFE on it.
    pub fn new() -> eyre::Result<Self> {
        let (gpu, adapter) = gpu()?;
        let model = RifeModel::new(&gpu.device).map_err(|error| eyre::eyre!("RIFE: {error}"))?;
        Ok(Self { gpu, adapter, model, workspaces: Vec::new() })
    }

    /// The adapter measuring: its name, backend and driver.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// The workspace of frames of this size, kept by its padded size.
    fn workspace(&mut self, width: u32, height: u32) -> eyre::Result<&RifeWorkspace> {
        let (width, height) = (width.div_ceil(RIFE_SIZE_MULTIPLE) * RIFE_SIZE_MULTIPLE,
            height.div_ceil(RIFE_SIZE_MULTIPLE) * RIFE_SIZE_MULTIPLE);
        let index = match self.workspaces.iter().position(|(size, _)| *size == (width, height)) {
            Some(index) => index,
            None => {
                if self.workspaces.len() >= MAX_CACHED_WORKSPACES {
                    self.workspaces.remove(0);
                }
                let workspace = RifeWorkspace::new(&self.model, &self.gpu.device, width, height)
                    .map_err(|error| eyre::eyre!("RIFE workspace {width}x{height}: {error}"))?;
                workspace.set_timestep(&self.gpu.queue, MEASURE_SLOT, MEASURE_TIMESTEP);
                self.workspaces.push(((width, height), workspace));
                self.workspaces.len() - 1
            }
        };
        Ok(&self.workspaces[index].1)
    }

    /// The body's travel over one cycle of `clip`, in screen pixels along `(ux, uy)` in the clip's stored frames;
    /// none when an in-between shows no body.
    fn travel_px(&mut self, clip: &StrideClip, ux: f32, uy: f32) -> eyre::Result<Option<f32>> {
        let frames = &clip.frames;
        let count = frames.len();
        if count < 2 {
            return Ok(None);
        }
        let bounds = (|| {
            let left = frames.iter().map(|frame| -frame.anchor_x).min()? - CANVAS_MARGIN;
            let top = frames.iter().map(|frame| -frame.anchor_y).min()? - CANVAS_MARGIN;
            let right = frames.iter().map(|frame| i32::from(frame.width) - frame.anchor_x).max()? + CANVAS_MARGIN;
            let bottom = frames.iter().map(|frame| i32::from(frame.height) - frame.anchor_y).max()? + CANVAS_MARGIN;
            Some((left, top, right, bottom))
        })();
        let Some((left, top, right, bottom)) = bounds else { return Ok(None) };
        let (width, height) = self.workspace((right - left) as u32, (bottom - top) as u32)?.size();
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
        let workspace = &self.workspaces.iter().find(|(size, _)| *size == (width, height))
            .expect("the workspace just made or kept").1;
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
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(POLL_TIMEOUT) })
            .map_err(|error| eyre::eyre!("wait for the GPU: {error}"))?;
        receiver.recv().wrap_err("the flow readback was dropped")?.wrap_err("map the flow readback")?;
        let values: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range()
            .map_err(|error| eyre::eyre!("read the flow: {error}"))?).to_vec();
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
                return Ok(None);
            }
            let feet_from = bottom as f32 - FEET_BAND * (bottom - top) as f32;
            along.clear();
            along.extend((0..plane)
                .filter(|&at| alpha[at] > OPAQUE_ALPHA && (at / width as usize) as f32 >= feet_from)
                .map(|at| (to_second_x[at] - to_first_x[at]) * ux + (to_second_y[at] - to_first_y[at]) * uy));
            travel -= percentile(&mut along, TRAVEL_PERCENTILE);
        }
        Ok(Some(travel))
    }

    /// Measures the clips of a movement in several facings (the side ones; `None` where a facing has no frames).
    pub fn measure<const FACINGS: usize>(&mut self, clips: &[Option<StrideClip>; FACINGS])
        -> eyre::Result<Measurement<FACINGS>> {
        let mut facings = [None; FACINGS];
        for (facing, clip) in facings.iter_mut().zip(clips) {
            let Some(clip) = clip.as_ref() else { continue };
            eyre::ensure!(clip.facing < FACING_COUNT, "facing {} of {FACING_COUNT}", clip.facing);
            let (mut ux, uy, px_per_tile) = travel_direction(clip.facing);
            if clip.mirrored {
                ux = -ux;
            }
            *facing = self.travel_px(clip, ux, uy)?.map(|travel| travel / px_per_tile);
        }
        Ok(Measurement::of_facings(facings))
    }
}

/// A clip's measurement: the stride of each facing that measured, and the verdict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement<const FACINGS: usize> {
    pub facings: [Option<f32>; FACINGS],
    /// How far the facings' strides spread (largest minus smallest) as a fraction of their median, for the log.
    pub spread: Option<f32>,
    /// The stride in tiles, or why the clip has none.
    pub stride: Result<f32, &'static str>,
}

impl<const FACINGS: usize> Measurement<FACINGS> {
    fn of_facings(facings: [Option<f32>; FACINGS]) -> Self {
        let mut strides: Vec<f32> = facings.iter().flatten().copied().collect();
        if strides.len() < MIN_FACINGS {
            return Self { facings, spread: None, stride: Err("too few facings") };
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
        Self { facings, spread, stride }
    }

    /// `ok`, or why the clip has no stride.
    pub fn verdict(&self) -> &'static str {
        self.stride.err().unwrap_or("ok")
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

    #[test]
    fn a_stride_is_the_median_of_at_least_two_facings_and_long_enough() {
        let stride = |facings| Measurement::<3>::of_facings(facings).stride;
        assert_eq!(stride([Some(1.0), Some(2.0), Some(4.0)]), Ok(2.0));
        assert_eq!(stride([Some(1.0), None, Some(2.0)]), Ok(1.5));
        assert_eq!(stride([Some(1.0), None, None]), Err("too few facings"));
        assert_eq!(stride([Some(0.1), Some(0.2), None]), Err("too short"));
        assert_eq!(stride([Some(-1.0), Some(0.5), None]), Err("not positive"));
    }
}
