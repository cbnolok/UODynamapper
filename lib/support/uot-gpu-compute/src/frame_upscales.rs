//! SPAN 2x of a clip's plain RGBA frames with their anchors ([`StrideFrame`]s), on a device of a tool's own: the
//! upscales Dynamapper's `mobile_upscale` makes from its frame atlas (render/mobiles/upscales.rs), for a tool that
//! wants them ahead of time. Each frame goes onto a canvas with its [`UPSCALE_BORDER`], one frame a submission, and
//! comes back as the whole 2x canvas, the halo past the frame's edge included, its anchor moved with it. A canvas of
//! more than [`UPSCALE_MAX_WHOLE_CANVAS_PIXELS`] upscales in tiles ([`crate::upscale::tiles`]) on a workspace of
//! [`UPSCALE_TILE_WORKSPACE_SIDE`] square, a submission each, as the game's does: the same 2x canvas.

use std::time::Duration;
use color_eyre::eyre::{self, WrapErr};
use crate::in_betweens::SequenceFrame;
use crate::strides::StrideFrame;
use crate::upscale::tiles::{upscale_tiles, TileSpan};
use crate::upscale::{upscale_canvas_size, UpscaleModel, UpscaleWorkspace, UPSCALE_BORDER, UPSCALE_OUTPUT_SCALE,
    UPSCALE_OUTPUT_TEXEL_BYTES};
use crate::{own_device_and_units, Gpu, SourceRect};

/// Canvas sides are rounded up to this, so frames of nearly one size share a workspace (as the game does).
pub const UPSCALE_CANVAS_SIZE_STEP: u32 = 8;
/// The side of the largest canvas upscaled whole, and its pixels: 260 by 260 takes about 90 MB of workspace and 30 ms
/// on the Radeon 880M. A larger canvas upscales in tiles.
pub const UPSCALE_MAX_WHOLE_CANVAS_SIDE: u32 = 260;
pub const UPSCALE_MAX_WHOLE_CANVAS_PIXELS: u32 = UPSCALE_MAX_WHOLE_CANVAS_SIDE * UPSCALE_MAX_WHOLE_CANVAS_SIDE;
/// The side of the workspace a canvas too large to upscale whole upscales on, in tiles: the largest multiple of
/// [`UPSCALE_CANVAS_SIZE_STEP`] whose square is within [`UPSCALE_MAX_WHOLE_CANVAS_PIXELS`] (256).
pub const UPSCALE_TILE_WORKSPACE_SIDE: u32 =
    UPSCALE_MAX_WHOLE_CANVAS_PIXELS.isqrt() / UPSCALE_CANVAS_SIZE_STEP * UPSCALE_CANVAS_SIZE_STEP;
/// Workspaces kept for reuse, by canvas size.
const MAX_CACHED_WORKSPACES: usize = 4;
const POLL_TIMEOUT: Duration = Duration::from_secs(60);
const RGBA_BYTES: usize = 4;

/// The workspace canvas of frames up to `width` by `height`: with the border, rounded up to
/// [`UPSCALE_CANVAS_SIZE_STEP`].
pub fn upscale_workspace_canvas(width: u32, height: u32) -> [u32; 2] {
    let (width, height) = upscale_canvas_size(width, height);
    [width.next_multiple_of(UPSCALE_CANVAS_SIZE_STEP), height.next_multiple_of(UPSCALE_CANVAS_SIZE_STEP)]
}

/// Whether a workspace canvas upscales whole, else in tiles.
pub fn upscales_whole(canvas: [u32; 2]) -> bool {
    canvas[0] * canvas[1] <= UPSCALE_MAX_WHOLE_CANVAS_PIXELS
}

/// The GPU device, the SPAN model, the texture a frame is uploaded to and the workspaces of the canvases used last.
pub struct FrameUpscaler {
    gpu: Gpu,
    adapter: String,
    model: UpscaleModel,
    source: wgpu::Texture,
    source_view: wgpu::TextureView,
    workspaces: Vec<((u32, u32), UpscaleWorkspace)>,
}

impl FrameUpscaler {
    /// Starts a device of its own and compiles SPAN on it, with room for frames up to `max_side` texels a side.
    pub fn new(max_side: u32) -> eyre::Result<Self> {
        let (gpu, adapter, units) = own_device_and_units()?;
        let model = UpscaleModel::new(&gpu.device, units).map_err(|error| eyre::eyre!("SPAN: {error}"))?;
        let source = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame to upscale"),
            size: wgpu::Extent3d { width: max_side, height: max_side, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let source_view = source.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
        });
        Ok(Self { gpu, adapter, model, source, source_view, workspaces: Vec::new() })
    }

    /// The adapter computing: its name, backend and driver.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// The index of the workspace of canvases of this size, kept by its rounded size.
    fn workspace(&mut self, width: u32, height: u32) -> eyre::Result<usize> {
        let size = (width.next_multiple_of(UPSCALE_CANVAS_SIZE_STEP), height.next_multiple_of(UPSCALE_CANVAS_SIZE_STEP));
        if let Some(index) = self.workspaces.iter().position(|(kept, _)| *kept == size) {
            return Ok(index);
        }
        if self.workspaces.len() >= MAX_CACHED_WORKSPACES {
            self.workspaces.remove(0);
        }
        let workspace = UpscaleWorkspace::new(&self.model, &self.gpu.device, &self.source_view, size.0, size.1)
            .map_err(|error| eyre::eyre!("SPAN workspace {}x{}: {error}", size.0, size.1))?;
        self.workspaces.push((size, workspace));
        Ok(self.workspaces.len() - 1)
    }

    /// Multiply-adds of the network on the canvas `frame` needs, as rounded for its workspace: what the game's cost
    /// model prices a frame by.
    pub fn multiply_adds(&mut self, frame_width: u32, frame_height: u32) -> eyre::Result<u64> {
        let (width, height) = upscale_canvas_size(frame_width, frame_height);
        let index = self.workspace(width, height)?;
        Ok(self.workspaces[index].1.network().iter().map(|dispatch| dispatch.multiply_adds()).sum())
    }

    /// The 2x canvas of `frame`, its alpha snapped to 0 or 1 at one half with `snap_alpha` (Classic frames): on the
    /// workspace of its own canvas, or in tiles past [`UPSCALE_MAX_WHOLE_CANVAS_PIXELS`].
    pub fn upscale(&mut self, frame: &StrideFrame, snap_alpha: bool) -> eyre::Result<SequenceFrame> {
        self.upscale_on(frame, snap_alpha, upscale_workspace_canvas(u32::from(frame.width), u32::from(frame.height)))
    }

    /// [`Self::upscale`] on the workspace of `canvas` ([`upscale_workspace_canvas`] of the largest frame of a clip, as
    /// the game's sequence has it), or in tiles when `canvas` is past [`UPSCALE_MAX_WHOLE_CANVAS_PIXELS`]. The 2x
    /// canvas is the same on any workspace that holds it, but for the rounding of sums.
    pub fn upscale_on(&mut self, frame: &StrideFrame, snap_alpha: bool, canvas: [u32; 2])
        -> eyre::Result<SequenceFrame> {
        let (frame_width, frame_height) = (u32::from(frame.width), u32::from(frame.height));
        let max_side = self.source.width();
        eyre::ensure!(frame_width <= max_side && frame_height <= max_side,
            "a {frame_width}x{frame_height} frame is larger than the upscaler's {max_side}x{max_side}");
        let (canvas_width, canvas_height) = upscale_canvas_size(frame_width, frame_height);
        let tiled = !upscales_whole(canvas);
        let workspace_canvas = if tiled { [UPSCALE_TILE_WORKSPACE_SIDE; 2] } else { canvas };
        eyre::ensure!(tiled || canvas_width <= canvas[0] && canvas_height <= canvas[1],
            "a {frame_width}x{frame_height} frame's canvas is larger than the {}x{} workspace", canvas[0], canvas[1]);
        let index = self.workspace(workspace_canvas[0], workspace_canvas[1])?;
        self.gpu.queue.write_texture(self.source.as_image_copy(), frame.rgba, wgpu::TexelCopyBufferLayout {
            offset: 0, bytes_per_row: Some(frame_width * UPSCALE_OUTPUT_TEXEL_BYTES), rows_per_image: Some(frame_height),
        }, wgpu::Extent3d { width: frame_width, height: frame_height, depth_or_array_layers: 1 });
        let source = SourceRect { layer: 0, x: 0, y: 0, width: frame_width, height: frame_height };
        let (width, height) = (canvas_width * UPSCALE_OUTPUT_SCALE, canvas_height * UPSCALE_OUTPUT_SCALE);
        let row_bytes = width as usize * RGBA_BYTES;
        let tiles = if tiled {
            upscale_tiles([canvas_width, canvas_height], workspace_canvas,
                [TileSpan { start: 0, end: canvas_width }, TileSpan { start: 0, end: canvas_height }])
                .map_err(|error| eyre::eyre!("{error}"))?
        } else {
            Vec::new()
        };
        let mut rgba = vec![0u8; row_bytes * height as usize];
        if tiles.is_empty() {
            let output = self.run(index, source, [UPSCALE_BORDER as i32; 2], snap_alpha)?;
            let pitch = self.workspaces[index].1.output_bytes_per_row() as usize;
            for (row, out) in output.chunks_exact(pitch).zip(rgba.chunks_exact_mut(row_bytes)) {
                out.copy_from_slice(&row[..row_bytes]);
            }
        }
        for tile in &tiles {
            let output = self.run(index, source, tile.frame_origin(), snap_alpha)?;
            let pitch = self.workspaces[index].1.output_bytes_per_row() as usize;
            let ([from_x, from_y], [core_width, core_height]) = tile.output_core();
            let [to_x, to_y] = [0, 1].map(|axis| tile.core[axis].start * UPSCALE_OUTPUT_SCALE);
            let bytes = core_width as usize * RGBA_BYTES;
            for y in 0..core_height as usize {
                let from = (from_y as usize + y) * pitch + from_x as usize * RGBA_BYTES;
                let to = (to_y as usize + y) * row_bytes + to_x as usize * RGBA_BYTES;
                rgba[to..to + bytes].copy_from_slice(&output[from..from + bytes]);
            }
        }
        let shift = UPSCALE_BORDER as i32;
        let scale = UPSCALE_OUTPUT_SCALE as i32;
        Ok(SequenceFrame { width, height, anchor_x: (frame.anchor_x + shift) * scale,
            anchor_y: (frame.anchor_y + shift) * scale, rgba })
    }

    /// Runs the network of workspace `index` on the frame `source` of the source texture, its top left at `origin` on
    /// the workspace's canvas, and reads the 2x output back: rows of the workspace's output pitch.
    fn run(&self, index: usize, source: SourceRect, origin: [i32; 2], snap_alpha: bool) -> eyre::Result<Vec<u8>> {
        let workspace = &self.workspaces[index].1;
        let (device, queue) = (&self.gpu.device, &self.gpu.queue);
        workspace.set_frame_at(queue, 0, source, origin, snap_alpha);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upscale readback"), size: workspace.output().size(), mapped_at_creation: false,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            workspace.upscale(0).for_each(|dispatch| dispatch.record(&mut pass));
        }
        encoder.copy_buffer_to_buffer(workspace.output(), 0, &readback, 0, workspace.output().size());
        queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(POLL_TIMEOUT) })
            .map_err(|error| eyre::eyre!("wait for the GPU: {error}"))?;
        receiver.recv().wrap_err("the upscale readback was dropped")?.wrap_err("map the upscale")?;
        let output = slice.get_mapped_range().map_err(|error| eyre::eyre!("read the upscale: {error}"))?.to_vec();
        readback.unmap();
        Ok(output)
    }
}
