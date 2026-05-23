//! Runtime-dispatched front-end for the analytical BC7 encoder.
//!
//! `pulp` gives the compiler target-feature-specific versions of the block
//! extraction path on x86/x86_64. The analytical mode-search core remains the
//! shared implementation, so this backend is byte-identical to the scalar path.

use crate::bc7_analytical::{pack_bc7_rgba, Pixel};

pub fn pack_bc7_rgba_blocks_pulp(
    blocks: &mut [u8],
    rgba_pixels: &[u8],
    width: u32,
    height: u32,
    flags: u32,
) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        pulp::Arch::new().dispatch(|| {
            pack_bc7_rgba_blocks_pulp_kernel(blocks, rgba_pixels, width, height, flags);
        });
    }

    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        pack_bc7_rgba_blocks_pulp_kernel(blocks, rgba_pixels, width, height, flags);
    }
}

#[inline(always)]
fn pack_bc7_rgba_blocks_pulp_kernel(
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
    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let mut pixels = [[0u8; 4]; 16];
            for row in 0..4 {
                let src_y = (block_y * 4 + row).min(height - 1);
                let base_x = block_x * 4;
                for col in 0..4 {
                    let src_x = (base_x + col).min(width - 1);
                    let src = (src_y * width + src_x) * 4;
                    pixels[row * 4 + col].copy_from_slice(&rgba_pixels[src..src + 4]);
                }
            }

            let block_index = block_y * blocks_x + block_x;
            let block: &mut [u8; 16] = blocks[block_index * 16..(block_index + 1) * 16]
                .as_mut()
                .try_into()
                .expect("BC7 block buffer is allocated in 16-byte blocks");
            let pixels: &[Pixel; 16] = &pixels;
            pack_bc7_rgba(block, pixels, flags);
        }
    }
}
