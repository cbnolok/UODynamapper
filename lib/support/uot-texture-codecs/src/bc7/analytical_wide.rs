//! Wide-vector front-end for the analytical BC7 encoder.
//!
//! This keeps the scalar mode-search core intact while batching block extraction
//! and dispatching larger images over worker threads. It is intentionally a separate entry
//! point so deeper SIMD work can move into the analytical core without changing
//! callers again.

#![allow(
    clippy::manual_swap,
    clippy::too_many_arguments,
    clippy::manual_is_multiple_of,
    clippy::manual_range_contains,
    clippy::unnecessary_cast,
    clippy::needless_range_loop,
    clippy::explicit_counter_loop
)]

use super::analytical::{Pixel, pack_bc7_rgb, pack_bc7_rgba};
use rayon::prelude::*;

const PARALLEL_BLOCK_THRESHOLD: usize = 256;
const PROGRESS_BLOCK_BATCH: usize = 256;

pub fn pack_bc7_rgba_blocks_wide(
    blocks: &mut [u8],
    rgba_pixels: &[u8],
    width: u32,
    height: u32,
    flags: u32,
) {
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    assert_eq!(blocks.len(), blocks_x * blocks_y * 16);
    assert_eq!(rgba_pixels.len(), width as usize * height as usize * 4);

    let width = width as usize;
    let height = height as usize;
    let force_rgb = rgba_pixels.chunks_exact(4).all(|pixel| pixel[3] == 255);

    if blocks_x * blocks_y >= PARALLEL_BLOCK_THRESHOLD {
        blocks
            .par_chunks_mut(PROGRESS_BLOCK_BATCH * 16)
            .enumerate()
            .for_each(|(chunk_index, chunk)| {
                let first_block_index = chunk_index * PROGRESS_BLOCK_BATCH;
                for (local_block_index, block) in chunk.chunks_exact_mut(16).enumerate() {
                    pack_one_block(
                        block
                            .try_into()
                            .expect("BC7 block buffer is allocated in 16-byte blocks"),
                        rgba_pixels,
                        width,
                        height,
                        blocks_x,
                        first_block_index + local_block_index,
                        flags,
                        force_rgb,
                    );
                }
            });
    } else {
        for (block_index, block) in blocks.chunks_exact_mut(16).enumerate() {
            pack_one_block(
                block
                    .try_into()
                    .expect("BC7 block buffer is allocated in 16-byte blocks"),
                rgba_pixels,
                width,
                height,
                blocks_x,
                block_index,
                flags,
                force_rgb,
            );
        }
    }
}

pub fn pack_bc7_rgba_blocks_wide_with_progress<F>(
    blocks: &mut [u8],
    rgba_pixels: &[u8],
    width: u32,
    height: u32,
    flags: u32,
    progress: F,
) where
    F: Fn(usize) + Sync,
{
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    assert_eq!(blocks.len(), blocks_x * blocks_y * 16);
    assert_eq!(rgba_pixels.len(), width as usize * height as usize * 4);

    let width = width as usize;
    let height = height as usize;
    let force_rgb = rgba_pixels.chunks_exact(4).all(|pixel| pixel[3] == 255);

    if blocks_x * blocks_y >= PARALLEL_BLOCK_THRESHOLD {
        blocks
            .par_chunks_mut(PROGRESS_BLOCK_BATCH * 16)
            .enumerate()
            .for_each(|(chunk_index, chunk)| {
                let first_block_index = chunk_index * PROGRESS_BLOCK_BATCH;
                for (local_block_index, block) in chunk.chunks_exact_mut(16).enumerate() {
                    pack_one_block(
                        block
                            .try_into()
                            .expect("BC7 block buffer is allocated in 16-byte blocks"),
                        rgba_pixels,
                        width,
                        height,
                        blocks_x,
                        first_block_index + local_block_index,
                        flags,
                        force_rgb,
                    );
                }
                progress(chunk.len() / 16);
            });
    } else {
        for (block_index, block) in blocks.chunks_exact_mut(16).enumerate() {
            pack_one_block(
                block
                    .try_into()
                    .expect("BC7 block buffer is allocated in 16-byte blocks"),
                rgba_pixels,
                width,
                height,
                blocks_x,
                block_index,
                flags,
                force_rgb,
            );
        }
        progress(blocks_x * blocks_y);
    }
}

fn pack_one_block(
    block: &mut [u8; 16],
    rgba_pixels: &[u8],
    width: usize,
    height: usize,
    blocks_x: usize,
    block_index: usize,
    flags: u32,
    force_rgb: bool,
) {
    let block_y = block_index / blocks_x;
    let block_x = block_index % blocks_x;
    let mut pixels = [[0u8; 4]; 16];
    let base_x = block_x * 4;
    let base_y = block_y * 4;
    if base_x + 4 <= width && base_y + 4 <= height {
        for row in 0..4 {
            let offset = ((base_y + row) * width + base_x) * 4;
            pixels[row * 4..row * 4 + 4]
                .as_flattened_mut()
                .copy_from_slice(&rgba_pixels[offset..offset + 16]);
        }
    } else {
        for row in 0..4 {
            let src_y = (base_y + row).min(height - 1);
            if base_x + 4 <= width {
                let offset = (src_y * width + base_x) * 4;
                pixels[row * 4..row * 4 + 4]
                    .as_flattened_mut()
                    .copy_from_slice(&rgba_pixels[offset..offset + 16]);
            } else {
                for col in 0..4 {
                    let src_x = (base_x + col).min(width - 1);
                    let src = (src_y * width + src_x) * 4;
                    pixels[row * 4 + col].copy_from_slice(&rgba_pixels[src..src + 4]);
                }
            }
        }
    }

    let pixels: &[Pixel; 16] = &pixels;
    if force_rgb {
        pack_bc7_rgb(block, pixels, flags);
    } else {
        pack_bc7_rgba(block, pixels, flags);
    }
}
