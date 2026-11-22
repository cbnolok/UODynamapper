//! The GPU BC7 RDO: the entropy reduction of uot-texture-codecs' CPU RDO (bc7enc_rdo's ERT) on a compute shader
//! (bc7_rdo.wgsl), with the CPU's price model (13 bits a literal byte, its match length and distance prices, the
//! cheaper continuation and REP0 matches) and its error scales (the smooth block scale, the ultrasmooth regions,
//! found over the whole page on the CPU), but a search of its own: each block takes the cheapest of all its candidates
//! (an earlier block of the same mode within the look-back, a run of 16 down to 3 bytes, an offset), the nearest,
//! longest, lowest one on a tie. Its blocks differ from the CPU's; its size and error at a λ and look-back are close
//! to them (docs/agent_notes/atlas_page_compression.md, "BC7 RDO on mobile animation pages").
//!
//! The output depends on the page, λ and look-back only. A page is walked in chunks of whole block rows (at least
//! [`CHUNK_MIN_BLOCKS`] and the look-back), each by one workgroup, block after block; the chunks are the page's, not
//! the threads', and the minimum over the candidates does not depend on how the workgroup's threads share them. Pages
//! RDO'd at once ([`Bc7RdoGpu::reduce_pages`], or concurrent [`Bc7RdoGpu::reduce_entropy`] calls, which join one
//! batch) come out as they do alone. The match state (the last match's distance, the block a continuing run would
//! start from) is the previous matched block's, as in ert.cpp and the BC1 RDO, not the CPU BC7 RDO's state of the
//! search so far. The CPU's segment hash, which skips a trial seen in a nearer block, is not needed: a farther block's
//! trial costs no less, and the tie goes to the nearer one.
//!
//! The RDO's relative movement and two-match search are not done; parameters asking for them are an error.

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use color_eyre::eyre::{self, WrapErr};
use uot_texture_codecs::bc7::rdo::{
    block_error_scales, compute_dist_cost_estimate, compute_match_len_cost, Bc7RdoParams, LITERAL_BITS,
    MATCH_CONTINUE_BITS, MATCH_REP0_BITS,
};
use uot_texture_codecs::bc7::tables::{
    BC7_ANCHOR_SECOND_SUBSET, BC7_ANCHOR_THIRD_SUBSET1, BC7_ANCHOR_THIRD_SUBSET2, BC7_PARTITION2, BC7_PARTITION3,
};
use wgpu::util::DeviceExt;

use crate::warm_up::CompileReport;
use crate::{network_shader_module, own_device, Gpu};

const SHADER: &str = include_str!("../shaders/block_encode/bc7_rdo.wgsl");
/// The compile [`Bc7RdoGpu::with_device`] reports.
pub const BC7_RDO_COMPILE: &str = "BC7 RDO walk";
/// The compiles [`Bc7RdoGpu::with_device`] reports.
pub const BC7_RDO_COMPILES: usize = 1;

/// Bytes of a BC7 block, and texels.
const BLOCK_BYTES: usize = 16;
const BLOCK_TEXELS: usize = 16;
const TEXEL_BYTES: u64 = 4;
/// The run lengths a match may have (bc7enc_rdo's), longest first in the candidate order.
const MIN_MATCH_LEN: usize = 3;
/// Candidates per earlier block: a run of 16 bytes at one offset, of 15 at two, … of 3 at fourteen.
const CANDIDATES_PER_BLOCK: usize = 105;
/// The rates per block distance in the rates buffer: one per run length 0 to 16 (bc7_rdo.wgsl's RATE_ROW).
const RATE_ROW: usize = BLOCK_BYTES + 1;
/// Partition shapes per subset count.
const PARTITIONS: usize = 64;
/// A chunk's smallest number of blocks, as the CPU RDO's: below it, the blocks at a chunk's start (which see fewer
/// earlier blocks) weigh more.
pub const CHUNK_MIN_BLOCKS: usize = 2048;
/// Blocks times look-back walked per submission: 256 blocks at a look-back of 64, so that no submission runs long
/// enough for the driver to take it for a hang.
const SUBMISSION_TRIAL_BLOCKS: usize = 256 * 64;
/// The most blocks RDO'd in one batch: 2M blocks are 128 MiB of texels.
const MAX_BATCH_BLOCKS: usize = 2 << 20;
/// How long to wait for a batch.
const POLL_TIMEOUT: Duration = Duration::from_secs(600);
/// The chunk state a chunk starts with: no match distance, no block to continue from.
const NO_REP0: u32 = 0;
const NO_BLOCK: u32 = u32::MAX;

/// bc7_rdo.wgsl's `Params`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WalkParams {
    lookback_blocks: u32,
    chunk_count: u32,
    block_begin: u32,
    block_count: u32,
    thresh_ratio_squared: f32,
    literal_rate: f32,
    skip_zero_error: u32,
    pad: u32,
}

/// What a walk takes from the RDO parameters: pages with equal ones can share a batch.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WalkSettings {
    lambda: f32,
    lookback_blocks: usize,
    thresh_ratio_squared: f32,
    skip_zero_error: bool,
}

impl WalkSettings {
    fn new(params: &Bc7RdoParams) -> eyre::Result<Self> {
        eyre::ensure!(!params.allow_relative_movement, "the GPU BC7 RDO has no relative movement");
        eyre::ensure!(!params.try_two_matches, "the GPU BC7 RDO tries one match per block");
        eyre::ensure!(params.lambda.is_finite() && params.lambda > 0.0, "BC7 RDO λ {} is not positive",
            params.lambda);
        let ratio = params.max_allowed_rms_increase_ratio;
        Ok(Self {
            lambda: params.lambda,
            lookback_blocks: (params.lookback_window_size / BLOCK_BYTES).max(1),
            thresh_ratio_squared: ratio * ratio,
            skip_zero_error: params.skip_zero_mse_blocks,
        })
    }

    /// λ times the bits of each match (bc7_rdo.wgsl's `rates`): by block distance and run length, then a
    /// continuing run's and a REP0 run's by run length.
    fn rates(&self) -> Vec<f32> {
        let literal_bits = |len: usize| (BLOCK_BYTES - len) as f32 * LITERAL_BITS;
        let mut rates = vec![0.0f32; (self.lookback_blocks + 3) * RATE_ROW];
        for delta in 1..=self.lookback_blocks {
            let dist_bits = compute_dist_cost_estimate((delta * BLOCK_BYTES) as u32) as f32;
            for len in MIN_MATCH_LEN..=BLOCK_BYTES {
                let match_bits = dist_bits + compute_match_len_cost(len as u32) as f32;
                rates[delta * RATE_ROW + len] = (literal_bits(len) + match_bits) * self.lambda;
            }
        }
        let continue_row = (self.lookback_blocks + 1) * RATE_ROW;
        let rep0_row = continue_row + RATE_ROW;
        for len in MIN_MATCH_LEN..=BLOCK_BYTES {
            rates[continue_row + len] = (literal_bits(len) + MATCH_CONTINUE_BITS) * self.lambda;
            rates[rep0_row + len] = (literal_bits(len) + MATCH_REP0_BITS) * self.lambda;
        }
        rates
    }

    /// Blocks per chunk of a page `blocks_x` blocks wide: whole rows, at least [`CHUNK_MIN_BLOCKS`] and the look-back.
    fn chunk_blocks(&self, blocks_x: usize) -> usize {
        CHUNK_MIN_BLOCKS.max(self.lookback_blocks).div_ceil(blocks_x).max(1) * blocks_x
    }

    fn submission_blocks(&self) -> usize {
        (SUBMISSION_TRIAL_BLOCKS / self.lookback_blocks).max(1)
    }
}

/// The candidates' run lengths and offsets as bc7_rdo.wgsl reads them (length | offset << 8), in their order:
/// longest first, lowest offset first.
fn candidate_shapes() -> Vec<u32> {
    let shapes: Vec<u32> = (MIN_MATCH_LEN..=BLOCK_BYTES).rev()
        .flat_map(|len| (0..=BLOCK_BYTES - len).map(move |ofs| len as u32 | (ofs as u32) << 8))
        .collect();
    debug_assert_eq!(shapes.len(), CANDIDATES_PER_BLOCK);
    shapes
}

/// bc7_rdo.wgsl's `tables`: the partitions as texel masks, the anchors, the candidate shapes.
fn tables() -> Vec<u32> {
    let mut tables = Vec::with_capacity(3 * PARTITIONS + CANDIDATES_PER_BLOCK);
    tables.extend((0..PARTITIONS).map(|p| (0..BLOCK_TEXELS)
        .fold(0u32, |mask, t| mask | u32::from(BC7_PARTITION2[p * BLOCK_TEXELS + t]) << t)));
    tables.extend((0..PARTITIONS).map(|p| (0..BLOCK_TEXELS)
        .fold(0u32, |mask, t| mask | u32::from(BC7_PARTITION3[p * BLOCK_TEXELS + t]) << (2 * t))));
    tables.extend((0..PARTITIONS).map(|p| u32::from(BC7_ANCHOR_SECOND_SUBSET[p])
        | u32::from(BC7_ANCHOR_THIRD_SUBSET1[p]) << 8 | u32::from(BC7_ANCHOR_THIRD_SUBSET2[p]) << 16));
    tables.extend(candidate_shapes());
    tables
}

/// A page to RDO in a batch: its blocks (`blocks_x` wide, rows top down) and their texels in block order.
pub struct Bc7RdoPage<'a> {
    pub blocks: &'a mut [[u8; BLOCK_BYTES]],
    pub rgba_blocks: &'a [[u8; 4]],
    pub blocks_x: usize,
    pub blocks_y: usize,
}

/// A page ready for the GPU: its blocks, texels and error scales.
struct PreparedPage {
    blocks: Vec<[u8; BLOCK_BYTES]>,
    texels: Vec<u32>,
    scales: Vec<f32>,
    blocks_x: usize,
}

impl PreparedPage {
    fn new(blocks: &[[u8; BLOCK_BYTES]], rgba_blocks: &[[u8; 4]], blocks_x: usize, blocks_y: usize,
        params: &Bc7RdoParams) -> eyre::Result<Self> {
        let count = blocks_x * blocks_y;
        eyre::ensure!(blocks.len() == count && rgba_blocks.len() == count * BLOCK_TEXELS,
            "a {blocks_x}x{blocks_y}-block page has {} blocks and {} texels", blocks.len(), rgba_blocks.len());
        Ok(Self {
            blocks: blocks.to_vec(),
            texels: rgba_blocks.iter().map(|texel| u32::from_le_bytes(*texel)).collect(),
            scales: block_error_scales(rgba_blocks, blocks_x, blocks_y, params),
            blocks_x,
        })
    }
}

/// Pages waiting for a batch, and the batches' results, for [`Bc7RdoGpu::reduce_entropy`]'s callers. One of them
/// leads: it runs batches of what waits until nothing does.
#[derive(Default)]
struct BatchQueue {
    pending: Vec<(u64, WalkSettings, PreparedPage)>,
    done: HashMap<u64, Result<Vec<[u8; BLOCK_BYTES]>, String>>,
    leading: bool,
    next_id: u64,
}

/// The GPU BC7 RDO on a device: its pipeline, and the pages waiting for it.
pub struct Bc7RdoGpu {
    gpu: Gpu,
    adapter: String,
    pipeline: wgpu::ComputePipeline,
    tables: wgpu::Buffer,
    max_batch_blocks: usize,
    queue: Mutex<BatchQueue>,
    batch_done: Condvar,
}

impl Bc7RdoGpu {
    /// Starts a device of its own (the most powerful adapter's) and compiles the RDO on it.
    pub fn new() -> eyre::Result<Self> {
        let (gpu, adapter) = own_device()?;
        Ok(Self::with_device(gpu.device, gpu.queue, adapter, &mut CompileReport::silent(BC7_RDO_COMPILES)))
    }

    /// Compiles the RDO on `device`, telling `report` (one compile, [`BC7_RDO_COMPILE`]); `adapter` describes it.
    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue, adapter: String, report: &mut CompileReport)
        -> Self {
        let pipeline = report.compile(BC7_RDO_COMPILE, || {
            // SAFETY: every loop of bc7_rdo.wgsl counts up to a constant, a uniform or a chunk's end read once.
            let module = unsafe { network_shader_module(&device, "BC7 RDO", SHADER.to_owned()) };
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(BC7_RDO_COMPILE), layout: None, module: &module, entry_point: Some("walk"),
                compilation_options: Default::default(), cache: None,
            })
        });
        let tables = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("BC7 RDO tables"), contents: bytemuck::cast_slice(&tables()),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let limits = device.limits();
        let binding_limit = u64::from(limits.max_storage_buffer_binding_size).min(limits.max_buffer_size);
        let max_batch_blocks = MAX_BATCH_BLOCKS
            .min((binding_limit / (BLOCK_TEXELS as u64 * TEXEL_BYTES)) as usize);
        Self {
            gpu: Gpu { device, queue }, adapter, pipeline, tables, max_batch_blocks,
            queue: Mutex::new(BatchQueue::default()), batch_done: Condvar::new(),
        }
    }

    /// The adapter it runs on: its name, backend and driver.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Reduces the entropy of a page's `blocks` (`blocks_x` by `blocks_y`, rows top down) against `rgba_blocks`,
    /// their texels in block order. Concurrent calls with the same λ, look-back and skipping run as one batch; each
    /// page comes out as it would alone.
    pub fn reduce_entropy(&self, blocks: &mut [[u8; BLOCK_BYTES]], rgba_blocks: &[[u8; 4]], blocks_x: usize,
        blocks_y: usize, params: &Bc7RdoParams) -> eyre::Result<()> {
        let settings = WalkSettings::new(params)?;
        let page = PreparedPage::new(blocks, rgba_blocks, blocks_x, blocks_y, params)?;
        eyre::ensure!(page.blocks.len() <= self.max_batch_blocks,
            "a page of {} blocks is more than the GPU BC7 RDO's {} at once", page.blocks.len(), self.max_batch_blocks);
        let (id, lead) = {
            let mut queue = self.queue.lock().expect("the BC7 RDO queue lock");
            let id = queue.next_id;
            queue.next_id += 1;
            queue.pending.push((id, settings, page));
            let lead = !queue.leading;
            queue.leading = true;
            (id, lead)
        };
        if lead {
            self.lead_batches();
        }
        let mut queue = self.queue.lock().expect("the BC7 RDO queue lock");
        loop {
            if let Some(result) = queue.done.remove(&id) {
                drop(queue);
                blocks.copy_from_slice(&result.map_err(|message| eyre::eyre!(message))?);
                return Ok(());
            }
            queue = self.batch_done.wait(queue).expect("the BC7 RDO queue lock");
        }
    }

    /// Runs batches of the waiting pages, those with the first one's settings at a time, until none waits.
    fn lead_batches(&self) {
        loop {
            let (settings, batch) = {
                let mut queue = self.queue.lock().expect("the BC7 RDO queue lock");
                let Some(&(_, settings, _)) = queue.pending.first() else {
                    queue.leading = false;
                    return;
                };
                let mut batch = Vec::new();
                let mut blocks = 0;
                let mut index = 0;
                while index < queue.pending.len() {
                    let (_, page_settings, page) = &queue.pending[index];
                    if *page_settings == settings && (batch.is_empty()
                        || blocks + page.blocks.len() <= self.max_batch_blocks) {
                        blocks += page.blocks.len();
                        let (id, _, page) = queue.pending.remove(index);
                        batch.push((id, page));
                    } else {
                        index += 1;
                    }
                }
                (settings, batch)
            };
            let (ids, pages): (Vec<u64>, Vec<PreparedPage>) = batch.into_iter().unzip();
            let results: Vec<Result<Vec<[u8; BLOCK_BYTES]>, String>> = match self.walk(&settings, &pages) {
                Ok(pages) => pages.into_iter().map(Ok).collect(),
                Err(error) => ids.iter().map(|_| Err(format!("{error:#}"))).collect(),
            };
            let mut queue = self.queue.lock().expect("the BC7 RDO queue lock");
            queue.done.extend(ids.into_iter().zip(results));
            self.batch_done.notify_all();
        }
    }

    /// Reduces the entropy of `pages` with `params`, as many at once as fit.
    pub fn reduce_pages(&self, pages: &mut [Bc7RdoPage], params: &Bc7RdoParams) -> eyre::Result<()> {
        let settings = WalkSettings::new(params)?;
        let mut start = 0;
        while start < pages.len() {
            let mut end = start;
            let mut blocks = 0;
            while end < pages.len() && (end == start || blocks + pages[end].blocks.len() <= self.max_batch_blocks) {
                blocks += pages[end].blocks.len();
                end += 1;
            }
            eyre::ensure!(blocks <= self.max_batch_blocks,
                "a page of {blocks} blocks is more than the GPU BC7 RDO's {} at once", self.max_batch_blocks);
            let prepared = pages[start..end].iter()
                .map(|page| PreparedPage::new(page.blocks, page.rgba_blocks, page.blocks_x, page.blocks_y, params))
                .collect::<eyre::Result<Vec<_>>>()?;
            for (page, reduced) in pages[start..end].iter_mut().zip(self.walk(&settings, &prepared)?) {
                page.blocks.copy_from_slice(&reduced);
            }
            start = end;
        }
        Ok(())
    }

    /// Walks the chunks of `pages` on the GPU, and returns their blocks.
    fn walk(&self, settings: &WalkSettings, pages: &[PreparedPage]) -> eyre::Result<Vec<Vec<[u8; BLOCK_BYTES]>>> {
        let device = &self.gpu.device;
        let total_blocks: usize = pages.iter().map(|page| page.blocks.len()).sum();
        if total_blocks == 0 {
            return Ok(pages.iter().map(|_| Vec::new()).collect());
        }
        let mut blocks = Vec::with_capacity(total_blocks);
        let mut texels = Vec::with_capacity(total_blocks * BLOCK_TEXELS);
        let mut scales = Vec::with_capacity(total_blocks);
        let mut chunks: Vec<[u32; 2]> = Vec::new();
        let mut longest_chunk = 0;
        for page in pages {
            let first = blocks.len();
            let chunk_blocks = settings.chunk_blocks(page.blocks_x);
            for start in (0..page.blocks.len()).step_by(chunk_blocks) {
                let end = (start + chunk_blocks).min(page.blocks.len());
                chunks.push([(first + start) as u32, (first + end) as u32]);
                longest_chunk = longest_chunk.max(end - start);
            }
            blocks.extend_from_slice(&page.blocks);
            texels.extend_from_slice(&page.texels);
            scales.extend_from_slice(&page.scales);
        }
        let chunk_count = chunks.len() as u32;
        eyre::ensure!(chunk_count <= device.limits().max_compute_workgroups_per_dimension,
            "{chunk_count} chunks are more workgroups than the device dispatches");

        let storage = |label, contents: &[u8], usage| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label), contents, usage: wgpu::BufferUsages::STORAGE | usage,
        });
        let block_buffer = storage("BC7 RDO blocks", bytemuck::cast_slice(&blocks), wgpu::BufferUsages::COPY_SRC);
        let texel_buffer = storage("BC7 RDO texels", bytemuck::cast_slice(&texels), wgpu::BufferUsages::empty());
        let scale_buffer = storage("BC7 RDO scales", bytemuck::cast_slice(&scales), wgpu::BufferUsages::empty());
        let chunk_buffer = storage("BC7 RDO chunks", bytemuck::cast_slice(&chunks), wgpu::BufferUsages::empty());
        let states = vec![[NO_REP0, NO_BLOCK]; chunks.len()];
        let state_buffer = storage("BC7 RDO chunk states", bytemuck::cast_slice(&states), wgpu::BufferUsages::empty());
        let rate_buffer = storage("BC7 RDO rates", bytemuck::cast_slice(&settings.rates()),
            wgpu::BufferUsages::empty());
        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("BC7 RDO params"), size: std::mem::size_of::<WalkParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let block_bytes = (total_blocks * BLOCK_BYTES) as u64;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("BC7 RDO readback"), size: block_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let entries = [&block_buffer, &texel_buffer, &scale_buffer, &chunk_buffer, &state_buffer, &rate_buffer,
            &self.tables, &params_buffer];
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("BC7 RDO"), layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries.iter().enumerate().map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32, resource: buffer.as_entire_binding(),
            }).collect::<Vec<_>>(),
        });

        let step = settings.submission_blocks();
        for block_begin in (0..longest_chunk).step_by(step) {
            let params = WalkParams {
                lookback_blocks: settings.lookback_blocks as u32,
                chunk_count,
                block_begin: block_begin as u32,
                block_count: step as u32,
                thresh_ratio_squared: settings.thresh_ratio_squared,
                literal_rate: BLOCK_BYTES as f32 * LITERAL_BITS * settings.lambda,
                skip_zero_error: u32::from(settings.skip_zero_error),
                pad: 0,
            };
            self.gpu.queue.write_buffer(&params_buffer, 0, bytemuck::bytes_of(&params));
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(chunk_count, 1, 1);
            }
            if block_begin + step >= longest_chunk {
                encoder.copy_buffer_to_buffer(&block_buffer, 0, &readback, 0, block_bytes);
            }
            self.gpu.queue.submit([encoder.finish()]);
        }
        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(POLL_TIMEOUT) })
            .map_err(|error| eyre::eyre!("wait for the GPU BC7 RDO: {error}"))?;
        receiver.recv().wrap_err("the BC7 RDO readback was dropped")?.wrap_err("map the BC7 RDO readback")?;
        let reduced: Vec<[u8; BLOCK_BYTES]> = bytemuck::cast_slice(&slice.get_mapped_range()
            .map_err(|error| eyre::eyre!("read the BC7 RDO blocks: {error}"))?).to_vec();
        readback.unmap();

        let mut start = 0;
        Ok(pages.iter().map(|page| {
            let page_blocks = reduced[start..start + page.blocks.len()].to_vec();
            start += page.blocks.len();
            page_blocks
        }).collect())
    }
}
