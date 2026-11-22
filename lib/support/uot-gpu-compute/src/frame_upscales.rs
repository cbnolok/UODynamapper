//! SPAN 2x of a clip's plain RGBA frames with their anchors ([`StrideFrame`]s), on a device of a tool's own: the
//! upscales Dynamapper's `mobile_upscale` makes from its frame atlas (render/mobiles/upscales.rs), for a tool that
//! wants them ahead of time. Each frame goes onto a canvas with its [`UPSCALE_BORDER`], one frame a submission, and
//! comes back as the whole 2x canvas, the halo past the frame's edge included, its anchor moved with it.

use std::time::Duration;
use color_eyre::eyre::{self, WrapErr};
use crate::in_betweens::SequenceFrame;
use crate::strides::StrideFrame;
use crate::upscale::{upscale_canvas_size, UpscaleModel, UpscaleWorkspace, UPSCALE_BORDER, UPSCALE_OUTPUT_SCALE,
    UPSCALE_OUTPUT_TEXEL_BYTES};
use crate::{own_device, Gpu, SourceRect};

/// Canvas sides are rounded up to this, so frames of nearly one size share a workspace (as the game does).
const CANVAS_SIZE_STEP: u32 = 8;
/// Workspaces kept for reuse, by canvas size.
const MAX_CACHED_WORKSPACES: usize = 4;
const POLL_TIMEOUT: Duration = Duration::from_secs(60);
const RGBA_BYTES: usize = 4;

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
        let (gpu, adapter) = own_device()?;
        let model = UpscaleModel::new(&gpu.device).map_err(|error| eyre::eyre!("SPAN: {error}"))?;
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
        let size = (width.div_ceil(CANVAS_SIZE_STEP) * CANVAS_SIZE_STEP, height.div_ceil(CANVAS_SIZE_STEP) * CANVAS_SIZE_STEP);
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

    /// The 2x canvas of `frame`, its alpha snapped to 0 or 1 at one half with `snap_alpha` (Classic frames).
    pub fn upscale(&mut self, frame: &StrideFrame, snap_alpha: bool) -> eyre::Result<SequenceFrame> {
        let (frame_width, frame_height) = (u32::from(frame.width), u32::from(frame.height));
        let max_side = self.source.width();
        eyre::ensure!(frame_width <= max_side && frame_height <= max_side,
            "a {frame_width}x{frame_height} frame is larger than the upscaler's {max_side}x{max_side}");
        let (canvas_width, canvas_height) = upscale_canvas_size(frame_width, frame_height);
        let index = self.workspace(canvas_width, canvas_height)?;
        let workspace = &self.workspaces[index].1;
        let (device, queue) = (&self.gpu.device, &self.gpu.queue);
        queue.write_texture(self.source.as_image_copy(), frame.rgba, wgpu::TexelCopyBufferLayout {
            offset: 0, bytes_per_row: Some(frame_width * UPSCALE_OUTPUT_TEXEL_BYTES), rows_per_image: Some(frame_height),
        }, wgpu::Extent3d { width: frame_width, height: frame_height, depth_or_array_layers: 1 });
        workspace.set_frame(queue, 0, SourceRect { layer: 0, x: 0, y: 0, width: frame_width, height: frame_height },
            snap_alpha).map_err(|error| eyre::eyre!("{error}"))?;
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
        let (width, height) = (canvas_width * UPSCALE_OUTPUT_SCALE, canvas_height * UPSCALE_OUTPUT_SCALE);
        let row_bytes = width as usize * RGBA_BYTES;
        let pitch = workspace.output_bytes_per_row() as usize;
        let rgba = {
            let mapped = slice.get_mapped_range().map_err(|error| eyre::eyre!("read the upscale: {error}"))?;
            mapped.chunks_exact(pitch).take(height as usize).flat_map(|row| &row[..row_bytes]).copied().collect()
        };
        readback.unmap();
        let shift = UPSCALE_BORDER as i32;
        let scale = UPSCALE_OUTPUT_SCALE as i32;
        Ok(SequenceFrame { width, height, anchor_x: (frame.anchor_x + shift) * scale,
            anchor_y: (frame.anchor_y + shift) * scale, rgba })
    }
}
