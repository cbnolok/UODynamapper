//! BC7 and BC1 encoding of whole texture pages held on the CPU, on a device of its own: the GPU encode of the
//! converter's atlas pages and BC1 gumps (udd-conv's BC encode device). A page goes up in bands of block rows, each
//! encoded by [`MobileBlockEncoder`] in a submission of its own that ends long before the driver would take it for a
//! hang, and read back before the next, so the pages encoded at once from the packers' threads hold a band each on the
//! GPU. A partial block repeats the page's edge texels, as the CPU BC7 encoder's does, so both encode the same texels.
//!
//! Concurrent calls share one encoder, whose frames wait in it for its next record, so each band records and submits
//! under a lock and waits for its blocks outside it. Each format's pipeline compiles at its first page, under the lock:
//! a converter packing BC1 gumps compiles BC1 only, one packing BC7 pages the quality it asks for.

use std::sync::Mutex;
use std::time::Duration;

use color_eyre::eyre::{self, WrapErr};
use wgpu::util::DeviceExt;

use crate::block_encode::{padded_size, Bc7Quality, MobileBlockEncoder, MobileBlockFormat, BLOCK_SIDE};
use crate::{own_device, Gpu};

/// Bytes of a BC7 block, of a BC1 block, and of a texel.
const BC7_BLOCK_BYTES: usize = 16;
const BC1_BLOCK_BYTES: usize = 8;
const TEXEL_BYTES: usize = 4;
/// The most blocks of a band: 128 Ki blocks, a 4096 × 512 band, 8 MiB of texels.
const MAX_BAND_BLOCKS: u32 = 1 << 17;
/// How long to wait for a band.
const POLL_TIMEOUT: Duration = Duration::from_secs(600);

/// The BC7 and BC1 encoder of pages on a device of its own.
pub struct PageEncoderGpu {
    gpu: Gpu,
    adapter: String,
    encoder: Mutex<MobileBlockEncoder>,
}

impl PageEncoderGpu {
    /// Starts a device of its own (the most powerful adapter's), compiling no pipeline yet.
    pub fn new() -> eyre::Result<Self> {
        let (gpu, adapter) = own_device()?;
        let encoder = MobileBlockEncoder::lazy(&gpu.device);
        Ok(Self { gpu, adapter, encoder: Mutex::new(encoder) })
    }

    /// The adapter it runs on: its name, backend and driver.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Encodes a `width` by `height` page (`rgba`: sRGB-encoded straight-alpha RGBA8, rows top down) to BC7 at
    /// `quality`, returning its blocks, block rows top down.
    pub fn encode(&self, rgba: &[u8], width: u32, height: u32, quality: Bc7Quality)
        -> eyre::Result<Vec<[u8; BC7_BLOCK_BYTES]>> {
        Ok(whole_blocks(&self.encode_blocks(rgba, width, height, MobileBlockFormat::Bc7(quality))?))
    }

    /// [`PageEncoderGpu::encode`] to BC1 with its one bit of alpha: alpha below one half is BC1's transparent black,
    /// as with the CPU BC1 encoder.
    pub fn encode_bc1(&self, rgba: &[u8], width: u32, height: u32) -> eyre::Result<Vec<[u8; BC1_BLOCK_BYTES]>> {
        Ok(whole_blocks(&self.encode_blocks(rgba, width, height, MobileBlockFormat::Bc1)?))
    }

    /// [`PageEncoderGpu::encode`] to `format`, its blocks' bytes one after the other.
    pub fn encode_blocks(&self, rgba: &[u8], width: u32, height: u32, format: MobileBlockFormat)
        -> eyre::Result<Vec<u8>> {
        eyre::ensure!(width > 0 && height > 0, "a {width}x{height} page has no texels");
        eyre::ensure!(rgba.len() == width as usize * height as usize * TEXEL_BYTES,
            "{} bytes for a {width}x{height} RGBA8 page", rgba.len());
        let max_side = self.gpu.device.limits().max_texture_dimension_2d;
        let (padded_width, padded_height) = padded_size(width, height);
        eyre::ensure!(padded_width <= max_side, "a page {width} texels wide is wider than the GPU's textures \
            ({max_side})");
        let blocks_x = padded_width / BLOCK_SIDE;
        let band_height = (MAX_BAND_BLOCKS / blocks_x).clamp(1, max_side / BLOCK_SIDE) * BLOCK_SIDE;
        let mut blocks = Vec::with_capacity(format.encoded_bytes(padded_width, padded_height) as usize);
        for top in (0..padded_height).step_by(band_height as usize) {
            let band = Band { top, height: band_height.min(padded_height - top), padded_width };
            blocks.extend(self.encode_band(rgba, width, height, &band, format)?);
        }
        Ok(blocks)
    }

    fn encode_band(&self, rgba: &[u8], width: u32, height: u32, band: &Band, format: MobileBlockFormat)
        -> eyre::Result<Vec<u8>> {
        let device = &self.gpu.device;
        let texture = device.create_texture_with_data(&self.gpu.queue, &wgpu::TextureDescriptor {
            label: Some("block page band"),
            size: wgpu::Extent3d { width: band.padded_width, height: band.height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }, wgpu::util::TextureDataOrder::LayerMajor, &band.texels(rgba, width, height));
        let view = texture.create_view(&Default::default());
        let bytes = format.encoded_bytes(band.padded_width, band.height);
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("block page blocks"), size: bytes, mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("block page readback"), size: bytes, mapped_at_creation: false,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        });
        let mut commands = device.create_command_encoder(&Default::default());
        let submission = {
            let mut encoder = self.encoder.lock().map_err(|_| eyre::eyre!("the GPU block encoder panicked"))?;
            {
                let mut pass = commands.begin_compute_pass(&Default::default());
                encoder.add_frame(format, &view, band.padded_width, band.height, &output, 0);
                encoder.record(&mut pass);
            }
            commands.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
            self.gpu.queue.submit([commands.finish()])
        };

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: Some(POLL_TIMEOUT) })
            .map_err(|error| eyre::eyre!("wait for the GPU block encode: {error}"))?;
        receiver.recv().wrap_err("the block encode readback was dropped")?.wrap_err("map the block encode readback")?;
        let blocks = slice.get_mapped_range().map_err(|error| eyre::eyre!("read the blocks: {error}"))?.to_vec();
        readback.unmap();
        Ok(blocks)
    }
}

/// `bytes` as blocks of `N` bytes.
fn whole_blocks<const N: usize>(bytes: &[u8]) -> Vec<[u8; N]> {
    bytes.chunks_exact(N).map(|block| block.try_into().expect("whole blocks")).collect()
}

/// A band of whole block rows of a page: its first texel row, its height and the page's padded width.
struct Band {
    top: u32,
    height: u32,
    padded_width: u32,
}

impl Band {
    /// The band's texels from `rgba` (a `width` by `height` page), the texels past the page's right and bottom
    /// edges repeating its last column and row.
    fn texels(&self, rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        let row_bytes = width as usize * TEXEL_BYTES;
        let last_texel = (width as usize - 1) * TEXEL_BYTES;
        let mut texels = Vec::with_capacity(self.padded_width as usize * self.height as usize * TEXEL_BYTES);
        for y in self.top..self.top + self.height {
            let row = &rgba[y.min(height - 1) as usize * row_bytes..][..row_bytes];
            texels.extend_from_slice(row);
            for _ in width..self.padded_width {
                texels.extend_from_slice(&row[last_texel..]);
            }
        }
        texels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uot_texture_codecs::bc1::decode_bc1_to_rgba8888;
    use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, ImageExtent};

    /// The texels of a band repeat the page's last column and row into the padding.
    #[test]
    fn a_band_repeats_the_edge_texels_into_its_padding() {
        const WIDTH: u32 = 3;
        const HEIGHT: u32 = 2;
        let rgba: Vec<u8> = (0..WIDTH * HEIGHT * TEXEL_BYTES as u32).map(|byte| byte as u8).collect();
        let band = Band { top: 0, height: BLOCK_SIDE, padded_width: BLOCK_SIDE };
        let texels = band.texels(&rgba, WIDTH, HEIGHT);
        let texel = |x: usize, y: usize| &texels[(y * BLOCK_SIDE as usize + x) * TEXEL_BYTES..][..TEXEL_BYTES];
        let source = |x: usize, y: usize| &rgba[(y * WIDTH as usize + x) * TEXEL_BYTES..][..TEXEL_BYTES];
        for y in 0..BLOCK_SIDE as usize {
            for x in 0..BLOCK_SIDE as usize {
                assert_eq!(texel(x, y), source(x.min(WIDTH as usize - 1), y.min(HEIGHT as usize - 1)), "{x}, {y}");
            }
        }
    }

    /// A page of several bands, with partial blocks, encodes to BC7 blocks that decode close to it, the same on a
    /// second run, and to BC1 blocks that keep its one bit of alpha: transparent black below one half, opaque above.
    #[test]
    #[ignore = "requires a GPU adapter; run explicitly for page encoder changes"]
    fn a_page_of_several_bands_encodes_close_to_its_texels() {
        const WIDTH: u32 = 1027;
        // Over two bands of a page this wide (MAX_BAND_BLOCKS / 257 block columns = 510 block rows).
        const HEIGHT: u32 = 2050;
        const MAX_MEAN_ERROR: f64 = 4.0;
        const MAX_BC1_MEAN_COLOUR_ERROR: f64 = 4.0;
        const HALF_ALPHA: u8 = 128;
        /// The alpha channel of a texel, after its three colour channels.
        const ALPHA: usize = 3;
        let start = std::time::Instant::now();
        let encoder = PageEncoderGpu::new().expect("a GPU adapter");
        println!("adapter: {} (started in {:.2?})", encoder.adapter(), start.elapsed());
        // Smooth gradients with an alpha ramp: what BC7 encodes well.
        let rgba: Vec<u8> = (0..HEIGHT).flat_map(|y| (0..WIDTH).flat_map(move |x| {
            [(x / 5) as u8, (y / 9) as u8, ((x + y) / 13) as u8, (y / 9) as u8]
        })).collect();
        let start = std::time::Instant::now();
        let blocks = encoder.encode(&rgba, WIDTH, HEIGHT, Bc7Quality::UltraFast).unwrap();
        println!("first BC7 UltraFast page, its pipeline compiled: {:.2?}", start.elapsed());
        let extent = ImageExtent::new(WIDTH, HEIGHT).unwrap();
        assert_eq!(blocks.len(), extent.blocks_wide() as usize * extent.blocks_high() as usize);
        let decoded = decode_bc7_to_rgba8888(blocks.as_flattened(), extent).unwrap();
        let error: f64 = rgba.iter().zip(&decoded).map(|(&a, &b)| f64::from(a.abs_diff(b))).sum();
        let mean = error / rgba.len() as f64;
        assert!(mean < MAX_MEAN_ERROR, "mean error {mean}");
        let start = std::time::Instant::now();
        assert_eq!(encoder.encode(&rgba, WIDTH, HEIGHT, Bc7Quality::UltraFast).unwrap(), blocks);
        println!("second BC7 UltraFast page: {:.2?}", start.elapsed());

        let start = std::time::Instant::now();
        let bc1 = encoder.encode_bc1(&rgba, WIDTH, HEIGHT).unwrap();
        println!("first BC1 page, its pipeline compiled: {:.2?}", start.elapsed());
        assert_eq!(bc1.len(), blocks.len());
        let decoded = decode_bc1_to_rgba8888(bc1.as_flattened(), extent).unwrap();
        let (mut colour_error, mut opaque) = (0.0, 0usize);
        for (source, texel) in rgba.chunks_exact(TEXEL_BYTES).zip(decoded.chunks_exact(TEXEL_BYTES)) {
            if source[ALPHA] < HALF_ALPHA {
                assert_eq!(texel, [0; TEXEL_BYTES], "a texel under half alpha is transparent black");
            } else {
                assert_eq!(texel[ALPHA], u8::MAX, "a texel over half alpha is opaque");
                colour_error += source[..ALPHA].iter().zip(&texel[..ALPHA])
                    .map(|(&a, &b)| f64::from(a.abs_diff(b))).sum::<f64>();
                opaque += 1;
            }
        }
        let mean = colour_error / (opaque * ALPHA) as f64;
        assert!(mean < MAX_BC1_MEAN_COLOUR_ERROR, "BC1 mean colour error {mean}");
    }
}
