//! RIFE in-betweens of a clip given as plain RGBA frames with their anchors ([`StrideFrame`]s), on a device of a tool's
//! own: what Dynamapper's RIFE modes show (its render/mobiles/rife_in_betweens.rs makes the same frames on its own
//! device, from its frame atlas), for a tool to preview. Between each two frames of the clip, the last toward the
//! first as the clips loop, `factor - 1` frames at 1/factor, 2/factor, ... of the way, with the blend's nearest warp
//! ([`RifePipelines::new`]). Each pair lies as the game lays it: both frames on one card with their anchors on one
//! point, the RGB the network looks at over 50% grey everywhere, the padding too (rife/atlas.wgsl's `pack`); an
//! in-between comes back with straight alpha, snapped to 0 or 1 for Classic frames (its `store`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use color_eyre::eyre::{self, WrapErr};
use crate::rife::{RifeModel, RifePipelines, RifeWorkspace, RIFE_ALPHA_CHANNEL, RIFE_COMPILES, RIFE_IMAGE_CHANNELS,
    RIFE_SIZE_MULTIPLE, RIFE_TIMESTEP_SLOTS};
use crate::strides::StrideFrame;
use crate::warm_up::{CompileReport, PipelineCompile};
use crate::{own_device, Gpu};

/// The most frames shown for each of a clip's: its own and one in-between per timestep slot.
pub const MAX_FACTOR: u32 = RIFE_TIMESTEP_SLOTS as u32 + 1;
/// The grey the network sees behind the frames (atlas.wgsl's `BACKGROUND_GREY`).
const BACKDROP_GREY: f32 = 0.5;
/// A Classic in-between is opaque from this coverage up, as atlas.wgsl's `CLASSIC_ALPHA_SNAP`.
const CLASSIC_ALPHA_SNAP: f32 = 0.5;
/// The merged planes read back: premultiplied RGB, then alpha.
const PREMULTIPLIED_CHANNEL: u32 = 3;
const READ_CHANNELS: u32 = RIFE_IMAGE_CHANNELS - PREMULTIPLIED_CHANNEL;
const COLOR_CHANNELS: usize = 3;
const MAX_CHANNEL: f32 = 255.0;
const RGBA_BYTES: usize = 4;
const F32_BYTES: u64 = 4;
/// Workspaces kept for reuse, by padded frame size.
const MAX_CACHED_WORKSPACES: usize = 4;
const POLL_TIMEOUT: Duration = Duration::from_secs(60);

/// A frame of a sequence: a clip's own frame, or an in-between on its pair's card. Its anchor is from its top-left
/// corner; its RGBA has straight alpha, rows top down.
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceFrame {
    pub width: u32,
    pub height: u32,
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub rgba: Vec<u8>,
}

impl SequenceFrame {
    fn of(frame: &StrideFrame) -> Self {
        Self { width: u32::from(frame.width), height: u32::from(frame.height), anchor_x: frame.anchor_x,
            anchor_y: frame.anchor_y, rgba: frame.rgba.to_vec() }
    }
}

/// The card of a pair: both frames with their anchors on one point. Its size, its anchor, and each frame's place on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PairCard {
    width: u32,
    height: u32,
    anchor_x: i32,
    anchor_y: i32,
    first: (u32, u32),
    next: (u32, u32),
}

impl PairCard {
    /// in_between.rs's `union_placement`.
    fn of(first: &StrideFrame, next: &StrideFrame) -> Self {
        let left = first.anchor_x.max(next.anchor_x);
        let right = (i32::from(first.width) - first.anchor_x).max(i32::from(next.width) - next.anchor_x);
        let up = first.anchor_y.max(next.anchor_y);
        let down = (i32::from(first.height) - first.anchor_y).max(i32::from(next.height) - next.anchor_y);
        let place = |frame: &StrideFrame| ((left - frame.anchor_x) as u32, (up - frame.anchor_y) as u32);
        Self { width: (left + right) as u32, height: (up + down) as u32, anchor_x: left, anchor_y: up,
            first: place(first), next: place(next) }
    }
}

/// The network's input for one pair, laid out on planes `width` wide: each frame's RGB over grey, premultiplied RGB
/// and alpha at its place on the card; grey and transparent elsewhere.
fn pack_pair(first: &StrideFrame, next: &StrideFrame, card: &PairCard, width: u32, height: u32) -> Vec<f32> {
    let plane = (width * height) as usize;
    let image_floats = RIFE_IMAGE_CHANNELS as usize * plane;
    let mut images = vec![0f32; 2 * image_floats];
    for (image, (frame, (x0, y0))) in images.chunks_exact_mut(image_floats).zip([(first, card.first), (next, card.next)]) {
        image[..COLOR_CHANNELS * plane].fill(BACKDROP_GREY);
        let frame_width = usize::from(frame.width);
        for y in 0..usize::from(frame.height) {
            for x in 0..frame_width {
                let texel = &frame.rgba[(y * frame_width + x) * RGBA_BYTES..][..RGBA_BYTES];
                let alpha = f32::from(texel[COLOR_CHANNELS]) / MAX_CHANNEL;
                let at = (y0 as usize + y) * width as usize + x0 as usize + x;
                for channel in 0..COLOR_CHANNELS {
                    let premultiplied = f32::from(texel[channel]) / MAX_CHANNEL * alpha;
                    image[channel * plane + at] = premultiplied + BACKDROP_GREY * (1.0 - alpha);
                    image[(PREMULTIPLIED_CHANNEL as usize + channel) * plane + at] = premultiplied;
                }
                image[RIFE_ALPHA_CHANNEL as usize * plane + at] = alpha;
            }
        }
    }
    images
}

/// An in-between's card from its premultiplied RGB and alpha planes, `width` wide: straight alpha, snapped to 0 or 1
/// with `snap_alpha` (atlas.wgsl's `store`).
fn unpack_card(planes: &[f32], width: u32, height: u32, card: &PairCard, snap_alpha: bool) -> SequenceFrame {
    let plane = (width * height) as usize;
    let mut rgba = vec![0u8; (card.width * card.height) as usize * RGBA_BYTES];
    let to_byte = |value: f32| (value.clamp(0.0, 1.0) * MAX_CHANNEL).round() as u8;
    for y in 0..card.height as usize {
        for x in 0..card.width as usize {
            let at = y * width as usize + x;
            let coverage = planes[COLOR_CHANNELS * plane + at].clamp(0.0, 1.0);
            let alpha = if snap_alpha { if coverage >= CLASSIC_ALPHA_SNAP { 1.0 } else { 0.0 } } else { coverage };
            let texel = &mut rgba[(y * card.width as usize + x) * RGBA_BYTES..][..RGBA_BYTES];
            if alpha > 0.0 {
                for channel in 0..COLOR_CHANNELS {
                    texel[channel] = to_byte(planes[channel * plane + at] / coverage);
                }
            }
            texel[COLOR_CHANNELS] = to_byte(alpha);
        }
    }
    SequenceFrame { width: card.width, height: card.height, anchor_x: card.anchor_x, anchor_y: card.anchor_y, rgba }
}

/// The GPU device, the RIFE model with the nearest blend, and the workspaces of the card sizes used last.
pub struct InBetweenMaker {
    gpu: Gpu,
    adapter: String,
    model: RifeModel,
    workspaces: Vec<((u32, u32), RifeWorkspace)>,
}

impl InBetweenMaker {
    /// Starts a device of its own and compiles RIFE on it, telling `compile` of each of the [`RIFE_COMPILES`] as it
    /// starts and ends.
    pub fn new(compile: impl FnMut(PipelineCompile)) -> eyre::Result<Self> {
        let (gpu, adapter) = own_device()?;
        let pipelines = RifePipelines::new(&gpu.device, &mut CompileReport::new(RIFE_COMPILES, compile));
        let model = RifeModel::with_pipelines(&gpu.device, pipelines).map_err(|error| eyre::eyre!("RIFE: {error}"))?;
        Ok(Self { gpu, adapter, model, workspaces: Vec::new() })
    }

    /// The adapter computing: its name, backend and driver.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// The index of the workspace of cards of this size, kept by its padded size.
    fn workspace(&mut self, width: u32, height: u32) -> eyre::Result<usize> {
        let size = (width.div_ceil(RIFE_SIZE_MULTIPLE) * RIFE_SIZE_MULTIPLE,
            height.div_ceil(RIFE_SIZE_MULTIPLE) * RIFE_SIZE_MULTIPLE);
        if let Some(index) = self.workspaces.iter().position(|(kept, _)| *kept == size) {
            return Ok(index);
        }
        if self.workspaces.len() >= MAX_CACHED_WORKSPACES {
            self.workspaces.remove(0);
        }
        let workspace = RifeWorkspace::new(&self.model, &self.gpu.device, size.0, size.1)
            .map_err(|error| eyre::eyre!("RIFE workspace {}x{}: {error}", size.0, size.1))?;
        self.workspaces.push((size, workspace));
        Ok(self.workspaces.len() - 1)
    }

    /// The clip's frames each followed by its `factor - 1` in-betweens toward the next (1 to [`MAX_FACTOR`]; a clip of
    /// one frame has none); Classic frames' in-betweens with `snap_alpha`. `progress` hears of each pair done, and how
    /// many there are. `None` once `cancel` is set, checked between pairs.
    pub fn sequence(&mut self, frames: &[StrideFrame], factor: u32, snap_alpha: bool, cancel: &AtomicBool,
        mut progress: impl FnMut(usize, usize)) -> eyre::Result<Option<Vec<SequenceFrame>>> {
        eyre::ensure!((1..=MAX_FACTOR).contains(&factor), "{factor} frames for each of a clip's: 1 to {MAX_FACTOR}");
        if factor == 1 || frames.len() < 2 {
            return Ok(Some(frames.iter().map(SequenceFrame::of).collect()));
        }
        let in_betweens = (factor - 1) as usize;
        let mut sequence = Vec::with_capacity(frames.len() * factor as usize);
        for (index, first) in frames.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let next = &frames[(index + 1) % frames.len()];
            let card = PairCard::of(first, next);
            let workspace_index = self.workspace(card.width, card.height)?;
            let workspace = &self.workspaces[workspace_index].1;
            let (width, height) = workspace.size();
            let device = &self.gpu.device;
            let queue = &self.gpu.queue;
            queue.write_buffer(workspace.input(), 0, bytemuck::cast_slice(&pack_pair(first, next, &card, width, height)));
            for slot in 0..in_betweens {
                workspace.set_timestep(queue, slot, (slot + 1) as f32 / factor as f32);
            }
            let plane_bytes = u64::from(width * height) * F32_BYTES;
            let read_bytes = u64::from(READ_CHANNELS) * plane_bytes;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("in-betweens"), size: read_bytes * in_betweens as u64, mapped_at_creation: false,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                workspace.encode().iter().for_each(|dispatch| dispatch.record(&mut pass));
            }
            for slot in 0..in_betweens {
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    workspace.interpolate(slot).for_each(|dispatch| dispatch.record(&mut pass));
                }
                encoder.copy_buffer_to_buffer(workspace.merged(), u64::from(PREMULTIPLIED_CHANNEL) * plane_bytes,
                    &readback, slot as u64 * read_bytes, read_bytes);
            }
            queue.submit([encoder.finish()]);
            let slice = readback.slice(..);
            let (sender, receiver) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(POLL_TIMEOUT) })
                .map_err(|error| eyre::eyre!("wait for the GPU: {error}"))?;
            receiver.recv().wrap_err("the in-between readback was dropped")?.wrap_err("map the in-betweens")?;
            let values: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range()
                .map_err(|error| eyre::eyre!("read the in-betweens: {error}"))?).to_vec();
            readback.unmap();

            sequence.push(SequenceFrame::of(first));
            for planes in values.chunks_exact((read_bytes / F32_BYTES) as usize) {
                sequence.push(unpack_card(planes, width, height, &card, snap_alpha));
            }
            progress(index + 1, frames.len());
        }
        Ok(Some(sequence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixels of a frame of one colour, `width` by `height`.
    fn solid(width: u16, height: u16, rgba: [u8; 4]) -> Vec<u8> {
        rgba.repeat(usize::from(width) * usize::from(height))
    }

    #[test]
    fn a_pair_card_holds_both_frames_with_their_anchors_on_one_point() {
        let (a, b) = (solid(4, 6, [u8::MAX; 4]), solid(6, 3, [u8::MAX; 4]));
        let first = StrideFrame { width: 4, height: 6, anchor_x: 1, anchor_y: 5, rgba: &a };
        let next = StrideFrame { width: 6, height: 3, anchor_x: 4, anchor_y: 2, rgba: &b };
        let card = PairCard::of(&first, &next);
        // Left of the anchor 4, right 3; above 5, below 1.
        assert_eq!((card.width, card.height, card.anchor_x, card.anchor_y), (7, 6, 4, 5));
        assert_eq!((card.first, card.next), ((3, 0), (0, 3)));
    }

    #[test]
    fn packing_then_unpacking_gives_back_a_frame_on_its_card() {
        const RED_HALF: [u8; 4] = [200, 40, 0, 128];
        let (a, b) = (solid(2, 2, RED_HALF), solid(1, 1, [0, 0, 0, u8::MAX]));
        let first = StrideFrame { width: 2, height: 2, anchor_x: 0, anchor_y: 0, rgba: &a };
        let next = StrideFrame { width: 1, height: 1, anchor_x: 0, anchor_y: 0, rgba: &b };
        let card = PairCard::of(&first, &next);
        let (width, height) = (RIFE_SIZE_MULTIPLE, RIFE_SIZE_MULTIPLE);
        let images = pack_pair(&first, &next, &card, width, height);
        let plane = (width * height) as usize;
        // The padding is grey to the network and transparent.
        assert_eq!(images[plane - 1], BACKDROP_GREY);
        assert_eq!(images[RIFE_ALPHA_CHANNEL as usize * plane + plane - 1], 0.0);
        let first_planes = &images[PREMULTIPLIED_CHANNEL as usize * plane..RIFE_IMAGE_CHANNELS as usize * plane];
        let straight = unpack_card(first_planes, width, height, &card, false);
        assert_eq!((straight.width, straight.height), (2, 2));
        assert_eq!(straight.rgba[..RGBA_BYTES], RED_HALF);
        let snapped = unpack_card(first_planes, width, height, &card, true);
        assert_eq!(snapped.rgba[..RGBA_BYTES], [RED_HALF[0], RED_HALF[1], RED_HALF[2], u8::MAX]);
    }

    /// Two equal frames: the flow is nil, so the nearest warp gives each in-between the frame itself.
    #[test]
    #[ignore = "requires a GPU adapter; run explicitly for RIFE changes"]
    fn the_in_betweens_of_a_still_clip_are_its_frame() {
        let mut maker = match InBetweenMaker::new(|_| {}) {
            Ok(maker) => maker,
            Err(error) => {
                println!("skipped: {error:#}");
                return;
            }
        };
        // A disc of radius 8 in a 24-pixel frame, its colour varying across it.
        const SIZE: u16 = 24;
        const CENTRE: i32 = 12;
        const RADIUS: i32 = 8;
        const ANCHOR_Y: i32 = 20;
        const GRADIENT: u16 = 10;
        const RED: u8 = 200;
        const TOLERANCE: u8 = 2;
        let mut rgba = Vec::new();
        for y in 0..SIZE {
            for x in 0..SIZE {
                let inside = (i32::from(x) - CENTRE).pow(2) + (i32::from(y) - CENTRE).pow(2) < RADIUS * RADIUS;
                let texel = [RED, (x * GRADIENT) as u8, (y * GRADIENT) as u8, u8::MAX];
                rgba.extend_from_slice(&if inside { texel } else { [0; RGBA_BYTES] });
            }
        }
        let frame = StrideFrame { width: SIZE, height: SIZE, anchor_x: CENTRE, anchor_y: ANCHOR_Y, rgba: &rgba };
        let cancel = AtomicBool::new(false);
        let mut pairs = 0;
        let sequence = maker.sequence(&[frame, frame], MAX_FACTOR, true, &cancel, |done, _| pairs = done)
            .unwrap().unwrap();
        assert_eq!((sequence.len(), pairs), (2 * MAX_FACTOR as usize, 2));
        for frame in &sequence {
            assert_eq!((frame.width, frame.height, frame.anchor_x, frame.anchor_y),
                (u32::from(SIZE), u32::from(SIZE), CENTRE, ANCHOR_Y));
            let differing = frame.rgba.chunks_exact(RGBA_BYTES).zip(rgba.chunks_exact(RGBA_BYTES))
                .filter(|(made, stored)| made[3] != stored[3] || (stored[3] != 0 && made.iter().zip(stored.iter())
                    .any(|(a, b)| a.abs_diff(*b) > TOLERANCE)))
                .count();
            assert_eq!(differing, 0);
        }
        cancel.store(true, Ordering::Relaxed);
        assert!(maker.sequence(&[frame, frame], 2, true, &cancel, |_, _| {}).unwrap().is_none());
    }
}
