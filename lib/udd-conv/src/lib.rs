#![allow(unused)]
pub mod bc7;
pub mod tex_art_cc;
pub mod eckr_terrain_kdl;
pub mod cc_tex_land_ec_transcode {
    pub use crate::eckr_terrain_kdl::*;
}
pub mod cc_map;
pub mod cc_gumps;
pub mod cc_radar;
pub mod cc_statics;
pub mod classic_patches;
pub mod tex_land_cc;
pub mod tex_art_ec;
pub mod tex_land_ec;
pub mod package_progress;
pub mod source_paths;
pub mod tilemeta;
pub use image_postprocess::upscaling;
pub mod world_lights;
pub mod hues;
pub mod mobile_anim_cc;
pub mod mobile_anim_ec;
pub use upscaling as upscale;

pub use udd_container::CompressionFlag as CompressionFlag;
pub use udd_assets::tex_art_cc::PagePixelFormat as PagePixelFormat;

pub(crate) const BC7_BLOCK_DIM: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AtlasPackingMode {
	#[default]
	MaximumPacking,
	Bc7Oriented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackingAxis {
	pub leading_padding: u32,
	pub alloc_extent: u32,
	pub used_extent: u32,
}

pub(crate) const fn align_up_u32(value: u32, alignment: u32) -> u32 {
	if alignment == 0 {
		value
	} else {
		value.div_ceil(alignment) * alignment
	}
	}

pub(crate) fn resolve_packing_axis(
	tile_extent: u32,
	atlas_extent: u32,
	gutter: u16,
	packing_mode: AtlasPackingMode,
	allow_axis_gutter_drop: bool,
) -> Option<PackingAxis> {
	match packing_mode {
		AtlasPackingMode::MaximumPacking => {
			let mut leading_padding = u32::from(gutter);
			if tile_extent + leading_padding * 2 > atlas_extent {
				if allow_axis_gutter_drop {
					leading_padding = 0;
				} else {
					return None;
				}
			}
			let alloc_extent = tile_extent + leading_padding * 2;
			if alloc_extent > atlas_extent {
				return None;
			}
			Some(PackingAxis {
				leading_padding,
				alloc_extent,
				used_extent: tile_extent,
			})
		}
		AtlasPackingMode::Bc7Oriented => {
			let content_extent = align_up_u32(tile_extent, BC7_BLOCK_DIM);
			let mut leading_padding = align_up_u32(u32::from(gutter), BC7_BLOCK_DIM);
			if content_extent + leading_padding * 2 > atlas_extent {
				if allow_axis_gutter_drop {
					leading_padding = 0;
				} else {
					return None;
				}
			}
			let alloc_extent = content_extent + leading_padding * 2;
			if alloc_extent > atlas_extent {
				return None;
			}
			Some(PackingAxis {
				leading_padding,
				alloc_extent,
				used_extent: content_extent,
			})
		}
	}
}

pub(crate) fn extrude_rgba_rect_edges(
	pixels: &mut [u8],
	atlas_width: u32,
	atlas_height: u32,
	rect_x: u32,
	rect_y: u32,
	rect_width: u32,
	rect_height: u32,
	content_x: u32,
	content_y: u32,
	content_width: u32,
	content_height: u32,
) {
	if content_width == 0 || content_height == 0 || rect_width == 0 || rect_height == 0 {
		return;
	}

	let rect_right = (rect_x + rect_width).min(atlas_width);
	let rect_bottom = (rect_y + rect_height).min(atlas_height);
	let content_right = content_x + content_width;
	let content_bottom = content_y + content_height;

	for y in rect_y..rect_bottom {
		let sample_y = y.clamp(content_y, content_bottom - 1);
		for x in rect_x..rect_right {
			if x >= content_x && x < content_right && y >= content_y && y < content_bottom {
				continue;
			}
			let sample_x = x.clamp(content_x, content_right - 1);
			let dst = ((y as usize * atlas_width as usize) + x as usize) * 4;
			let src = ((sample_y as usize * atlas_width as usize) + sample_x as usize) * 4;
			let rgba = [pixels[src], pixels[src + 1], pixels[src + 2], pixels[src + 3]];
			pixels[dst..dst + 4].copy_from_slice(&rgba);
		}
	}
}

pub(crate) fn merge_unplaced_tiles<T, K, F>(
	mut leftovers: Vec<T>,
	mut unplaced: Vec<T>,
	mut key_fn: F,
) -> Vec<T>
where
	K: Ord,
	F: FnMut(&T) -> K,
{
	leftovers.append(&mut unplaced);
	leftovers.sort_by_key(|tile| key_fn(tile));
	leftovers
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn extrude_rgba_rect_edges_clamps_gutter_to_nearest_content_pixel() {
		let mut pixels = vec![0u8; 6 * 6 * 4];
		for y in 2..4 {
			for x in 2..4 {
				let offset = ((y * 6 + x) * 4) as usize;
				pixels[offset] = (x * 10) as u8;
				pixels[offset + 1] = (y * 20) as u8;
				pixels[offset + 2] = 200;
				pixels[offset + 3] = 255;
			}
		}

		extrude_rgba_rect_edges(&mut pixels, 6, 6, 1, 1, 4, 4, 2, 2, 2, 2);

		let top_left = ((1 * 6 + 1) * 4) as usize;
		let content_top_left = ((2 * 6 + 2) * 4) as usize;
		assert_eq!(&pixels[top_left..top_left + 4], &pixels[content_top_left..content_top_left + 4]);

		let right = ((2 * 6 + 4) * 4) as usize;
		let content_right = ((2 * 6 + 3) * 4) as usize;
		assert_eq!(&pixels[right..right + 4], &pixels[content_right..content_right + 4]);

		let bottom = ((4 * 6 + 3) * 4) as usize;
		let content_bottom = ((3 * 6 + 3) * 4) as usize;
		assert_eq!(&pixels[bottom..bottom + 4], &pixels[content_bottom..content_bottom + 4]);
	}
}
