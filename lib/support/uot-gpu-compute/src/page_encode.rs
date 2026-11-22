//! BC7 and BC1 encoding of whole texture pages held on the CPU, on a device of its own: the GPU encode of the
//! converter's atlas pages and BC1 gumps (udd-conv's BC encode device). A partial block repeats the page's edge
//! texels, as the CPU BC7 encoder's does, so both encode the same texels.
//!
//! For BC7, a block of one colour (much of a sprite page: its transparent background) is the CPU encoder's solid
//! block, made here on the CPU; bc7.wgsl gives such a block the same one, so only the other blocks go to the GPU.
//! Those go up side by side in bands of at most [`MAX_BAND_BLOCKS`], each encoded by [`MobileBlockEncoder`] in a
//! submission of its own that ends long before the driver would take it for a hang, and read back before the next, so
//! the pages encoded at once from the packers' threads hold a band each on the GPU. The kernels encode a block from its
//! own texels alone, so its blocks do not depend on where it lies in the band.
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
use uot_texture_codecs::bc7::analytical::pack_bc7_rgba;

/// Bytes of a BC7 block, of a BC1 block, and of a texel.
const BC7_BLOCK_BYTES: usize = 16;
const BC1_BLOCK_BYTES: usize = 8;
const TEXEL_BYTES: usize = 4;
/// The texels of a block, and the bytes of its texels and of one of its rows.
const BLOCK_TEXELS: usize = (BLOCK_SIDE * BLOCK_SIDE) as usize;
const BLOCK_TEXEL_BYTES: usize = BLOCK_TEXELS * TEXEL_BYTES;
const BLOCK_ROW_BYTES: usize = BLOCK_SIDE as usize * TEXEL_BYTES;
/// The alpha of a texel, after its colour.
const ALPHA: usize = 3;
/// The most blocks of a band: 128 Ki blocks, 8 MiB of texels.
pub const MAX_BAND_BLOCKS: usize = 1 << 17;
/// The blocks side by side in a row of a band (4096 texels), unless the GPU's textures are narrower.
const BAND_ROW_BLOCKS: u32 = 1 << 10;
/// The CPU encoder's flags for a block of one colour, which takes its solid block whatever they are.
const SOLID_BLOCK_FLAGS: u32 = 0;
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
        let page = Page { rgba, width, height };
        let (padded_width, padded_height) = padded_size(width, height);
        let blocks_x = (padded_width / BLOCK_SIDE) as usize;
        let block_count = blocks_x * (padded_height / BLOCK_SIDE) as usize;
        let block_bytes = format.block_bytes() as usize;
        let mut blocks = vec![0; block_count * block_bytes];
        // The blocks the GPU encodes: their index on the page, and their texels.
        let (mut indices, mut texels) = (Vec::new(), Vec::new());
        for index in 0..block_count {
            let block = page.block_texels(index % blocks_x, index / blocks_x);
            match solid_block(format, &block) {
                Some(solid) => blocks[index * block_bytes..][..block_bytes].copy_from_slice(&solid),
                None => {
                    indices.push(index);
                    texels.push(block);
                }
            }
        }
        let row_blocks = BAND_ROW_BLOCKS.min(self.gpu.device.limits().max_texture_dimension_2d / BLOCK_SIDE) as usize;
        for (indices, texels) in indices.chunks(MAX_BAND_BLOCKS).zip(texels.chunks(MAX_BAND_BLOCKS)) {
            let encoded = self.encode_band(&Band::pack(texels, row_blocks), format)?;
            for (&index, block) in indices.iter().zip(encoded.chunks_exact(block_bytes)) {
                blocks[index * block_bytes..][..block_bytes].copy_from_slice(block);
            }
        }
        Ok(blocks)
    }

    /// The blocks of `band` in `format`, row by row of the band.
    fn encode_band(&self, band: &Band, format: MobileBlockFormat) -> eyre::Result<Vec<u8>> {
        let device = &self.gpu.device;
        let texture = device.create_texture_with_data(&self.gpu.queue, &wgpu::TextureDescriptor {
            label: Some("block page band"),
            size: wgpu::Extent3d { width: band.width, height: band.height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }, wgpu::util::TextureDataOrder::LayerMajor, &band.texels);
        let view = texture.create_view(&Default::default());
        let bytes = format.encoded_bytes(band.width, band.height);
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
                encoder.add_frame(format, &view, band.width, band.height, &output, 0);
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

/// A page's texels: sRGB-encoded straight-alpha RGBA8, rows top down.
struct Page<'a> {
    rgba: &'a [u8],
    width: u32,
    height: u32,
}

impl Page<'_> {
    /// The texels of the block at block column `x`, block row `y`, row by row, those past the page's right and bottom
    /// edges repeating its last column and row.
    fn block_texels(&self, x: usize, y: usize) -> [u8; BLOCK_TEXEL_BYTES] {
        let (width, height, side) = (self.width as usize, self.height as usize, BLOCK_SIDE as usize);
        let mut texels = [0; BLOCK_TEXEL_BYTES];
        for (row, texel_row) in texels.chunks_exact_mut(BLOCK_ROW_BYTES).enumerate() {
            let source_row = (y * side + row).min(height - 1) * width;
            if (x + 1) * side <= width {
                texel_row.copy_from_slice(&self.rgba[(source_row + x * side) * TEXEL_BYTES..][..BLOCK_ROW_BYTES]);
            } else {
                for (column, texel) in texel_row.chunks_exact_mut(TEXEL_BYTES).enumerate() {
                    let source_x = (x * side + column).min(width - 1);
                    texel.copy_from_slice(&self.rgba[(source_row + source_x) * TEXEL_BYTES..][..TEXEL_BYTES]);
                }
            }
        }
        texels
    }
}

/// The CPU encoder's block for `texels` (a block's, row by row) when they are all one colour and `format` is BC7: its
/// solid block, which bc7.wgsl gives such a block too, with alpha 255 for BC7 without alpha.
fn solid_block(format: MobileBlockFormat, texels: &[u8; BLOCK_TEXEL_BYTES]) -> Option<[u8; BC7_BLOCK_BYTES]> {
    let opaque = match format {
        MobileBlockFormat::Bc7(_) => false,
        MobileBlockFormat::Bc7Opaque(_) => true,
        MobileBlockFormat::Bc1 => return None,
    };
    let mut texel: [u8; TEXEL_BYTES] = texels[..TEXEL_BYTES].try_into().expect("a texel");
    if texels.chunks_exact(TEXEL_BYTES).any(|other| other != texel) {
        return None;
    }
    if opaque {
        texel[ALPHA] = u8::MAX;
    }
    let mut block = [0; BC7_BLOCK_BYTES];
    pack_bc7_rgba(&mut block, &[texel; BLOCK_TEXELS], SOLID_BLOCK_FLAGS);
    Some(block)
}

/// Blocks to encode side by side in rows: their texture's texels, width and height, the slots past the last block
/// transparent.
struct Band {
    texels: Vec<u8>,
    width: u32,
    height: u32,
}

impl Band {
    /// The band of `blocks` (each a block's texels, row by row), `row_blocks` of them to a row.
    fn pack(blocks: &[[u8; BLOCK_TEXEL_BYTES]], row_blocks: usize) -> Self {
        let side = BLOCK_SIDE as usize;
        let columns = row_blocks.min(blocks.len());
        let rows = blocks.len().div_ceil(columns);
        let row_bytes = columns * BLOCK_ROW_BYTES;
        let mut texels = vec![0; rows * side * row_bytes];
        for (slot, block) in blocks.iter().enumerate() {
            let (column, row) = (slot % columns, slot / columns);
            for (texel_row, block_row) in block.chunks_exact(BLOCK_ROW_BYTES).enumerate() {
                texels[(row * side + texel_row) * row_bytes + column * BLOCK_ROW_BYTES..][..BLOCK_ROW_BYTES]
                    .copy_from_slice(block_row);
            }
        }
        let texels_across = |blocks: usize| u32::try_from(blocks * side).expect("a band's side fits u32");
        Self { texels, width: texels_across(columns), height: texels_across(rows) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uot_texture_codecs::bc1::decode_bc1_to_rgba8888;
    use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, ImageExtent};

    /// The texels of a page's blocks repeat its last column and row past its edges.
    #[test]
    fn a_block_repeats_the_edge_texels_into_its_padding() {
        const WIDTH: u32 = 5;
        const HEIGHT: u32 = 6;
        let rgba: Vec<u8> = (0..WIDTH * HEIGHT * TEXEL_BYTES as u32).map(|byte| byte as u8).collect();
        let page = Page { rgba: &rgba, width: WIDTH, height: HEIGHT };
        let source = |x: usize, y: usize| &rgba[(y * WIDTH as usize + x) * TEXEL_BYTES..][..TEXEL_BYTES];
        let side = BLOCK_SIDE as usize;
        for (block_x, block_y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let texels = page.block_texels(block_x, block_y);
            for (index, texel) in texels.chunks_exact(TEXEL_BYTES).enumerate() {
                let (x, y) = (block_x * side + index % side, block_y * side + index / side);
                assert_eq!(texel, source(x.min(WIDTH as usize - 1), y.min(HEIGHT as usize - 1)), "{x}, {y}");
            }
        }
    }

    /// A band lays its blocks out row by row, `row_blocks` to a row, the slots after the last one zero.
    #[test]
    fn a_band_packs_its_blocks_in_rows() {
        const ROW_BLOCKS: usize = 3;
        const BLOCKS: usize = 7;
        let blocks: Vec<[u8; BLOCK_TEXEL_BYTES]> = (0..BLOCKS)
            .map(|block| std::array::from_fn(|byte| (block * BLOCK_TEXEL_BYTES + byte) as u8)).collect();
        let band = Band::pack(&blocks, ROW_BLOCKS);
        let side = BLOCK_SIDE as usize;
        assert_eq!((band.width as usize, band.height as usize),
            (ROW_BLOCKS * side, BLOCKS.div_ceil(ROW_BLOCKS) * side));
        for (x, y) in (0..band.height as usize).flat_map(|y| (0..band.width as usize).map(move |x| (x, y))) {
            let texel = &band.texels[(y * band.width as usize + x) * TEXEL_BYTES..][..TEXEL_BYTES];
            let index = y / side * ROW_BLOCKS + x / side;
            let expected = blocks.get(index).map_or([0; TEXEL_BYTES], |block| {
                block[((y % side) * side + x % side) * TEXEL_BYTES..][..TEXEL_BYTES].try_into().unwrap()
            });
            assert_eq!(texel, expected, "{x}, {y}");
        }
        assert_eq!(Band::pack(&blocks[..1], ROW_BLOCKS).width, BLOCK_SIDE, "a band no wider than its blocks");
    }

    /// A block of one colour is the CPU encoder's block for it, BC7 without alpha its opaque block; BC1 and a block
    /// of two colours have none.
    #[test]
    fn a_block_of_one_colour_is_the_cpu_encoders() {
        use uot_texture_codecs::bc7::{encode_to_bc7, Bc7EncoderBackend, RawImageFormat};
        const COLOUR: [u8; TEXEL_BYTES] = [200, 13, 77, 140];
        let texels: [u8; BLOCK_TEXEL_BYTES] = std::array::from_fn(|byte| COLOUR[byte % TEXEL_BYTES]);
        let extent = ImageExtent::new(BLOCK_SIDE, BLOCK_SIDE).unwrap();
        let cpu = |texels: &[u8]| encode_to_bc7(texels, extent, RawImageFormat::Rgba8888,
            Bc7EncoderBackend::AnalyticalWide).unwrap().into_blocks();
        let format = MobileBlockFormat::Bc7(Bc7Quality::UltraFast);
        assert_eq!(solid_block(format, &texels).unwrap().to_vec(), cpu(&texels));
        let mut opaque = texels;
        opaque.chunks_exact_mut(TEXEL_BYTES).for_each(|texel| texel[ALPHA] = u8::MAX);
        assert_eq!(solid_block(MobileBlockFormat::Bc7Opaque(Bc7Quality::UltraFast), &texels).unwrap().to_vec(),
            cpu(&opaque));
        assert!(solid_block(MobileBlockFormat::Bc1, &texels).is_none());
        let mut two_colours = texels;
        two_colours[0] ^= 1;
        assert!(solid_block(format, &two_colours).is_none());
    }

    /// A page of several bands, with partial blocks and blocks of one colour, encodes to BC7 blocks that decode close
    /// to it, the same on a second run and the same as the block encoder gives the page whole in one frame, and to BC1
    /// blocks that keep its one bit of alpha: transparent black below one half, opaque above.
    #[test]
    #[ignore = "requires a GPU adapter; run explicitly for page encoder changes"]
    fn a_page_of_several_bands_encodes_close_to_its_texels() {
        const WIDTH: u32 = 1027;
        // More blocks the GPU encodes than a band holds (checked below).
        const HEIGHT: u32 = 4100;
        /// A transparent rectangle, of whole blocks and partial ones.
        const CLEAR_COLUMNS: std::ops::Range<u32> = 100..302;
        const MAX_MEAN_ERROR: f64 = 4.0;
        const MAX_BC1_MEAN_COLOUR_ERROR: f64 = 4.0;
        const HALF_ALPHA: u8 = 128;
        let start = std::time::Instant::now();
        let encoder = PageEncoderGpu::new().expect("a GPU adapter");
        println!("adapter: {} (started in {:.2?})", encoder.adapter(), start.elapsed());
        // Smooth gradients with an alpha ramp, what BC7 encodes well, and a transparent rectangle.
        let rgba: Vec<u8> = (0..HEIGHT).flat_map(|y| (0..WIDTH).flat_map(move |x| {
            if CLEAR_COLUMNS.contains(&x) { [0; TEXEL_BYTES] }
            else { [(x / 5) as u8, (y / 9) as u8, ((x + y) / 13) as u8, (y / 9) as u8] }
        })).collect();
        let start = std::time::Instant::now();
        let blocks = encoder.encode(&rgba, WIDTH, HEIGHT, Bc7Quality::UltraFast).unwrap();
        println!("first BC7 UltraFast page, its pipeline compiled: {:.2?}", start.elapsed());
        let extent = ImageExtent::new(WIDTH, HEIGHT).unwrap();
        let page = Page { rgba: &rgba, width: WIDTH, height: HEIGHT };
        let blocks_wide = extent.blocks_wide() as usize;
        let on_the_gpu = (0..blocks.len()).filter(|&index| solid_block(MobileBlockFormat::Bc7(Bc7Quality::UltraFast),
            &page.block_texels(index % blocks_wide, index / blocks_wide)).is_none()).count();
        assert!(on_the_gpu > MAX_BAND_BLOCKS, "{on_the_gpu} blocks on the GPU, one band");
        assert_eq!(blocks.len(), extent.blocks_wide() as usize * extent.blocks_high() as usize);
        let decoded = decode_bc7_to_rgba8888(blocks.as_flattened(), extent).unwrap();
        let error: f64 = rgba.iter().zip(&decoded).map(|(&a, &b)| f64::from(a.abs_diff(b))).sum();
        let mean = error / rgba.len() as f64;
        assert!(mean < MAX_MEAN_ERROR, "mean error {mean}");
        let start = std::time::Instant::now();
        assert_eq!(encoder.encode(&rgba, WIDTH, HEIGHT, Bc7Quality::UltraFast).unwrap(), blocks);
        println!("second BC7 UltraFast page: {:.2?}", start.elapsed());
        assert_eq!(whole_page_in_one_frame(&encoder, &rgba, WIDTH, HEIGHT), blocks.as_flattened(),
            "the blocks of the page whole");

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

    /// The BC7 UltraFast blocks of a page encoded whole, in one frame of the block encoder: its padded texels in one
    /// texture, every block on the GPU.
    fn whole_page_in_one_frame(encoder: &PageEncoderGpu, rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        let page = Page { rgba, width, height };
        let (padded_width, padded_height) = padded_size(width, height);
        let side = BLOCK_SIDE as usize;
        let mut texels = vec![0; padded_width as usize * padded_height as usize * TEXEL_BYTES];
        let row_bytes = padded_width as usize * TEXEL_BYTES;
        for y in 0..padded_height as usize / side {
            for x in 0..padded_width as usize / side {
                for (row, block_row) in page.block_texels(x, y).chunks_exact(BLOCK_ROW_BYTES).enumerate() {
                    texels[(y * side + row) * row_bytes + x * BLOCK_ROW_BYTES..][..BLOCK_ROW_BYTES]
                        .copy_from_slice(block_row);
                }
            }
        }
        let band = Band { texels, width: padded_width, height: padded_height };
        encoder.encode_band(&band, MobileBlockFormat::Bc7(Bc7Quality::UltraFast)).unwrap()
    }
}
