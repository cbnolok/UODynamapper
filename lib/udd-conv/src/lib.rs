#![allow(unused)]
pub mod bc7;
pub mod tex_art_cc;
pub mod cc_map;
pub mod chunk_compression;
pub mod page_compression;
pub mod cc_gumps;
pub mod ec_gumps;
/// The paperdoll atlas pages and the manifests every gump package carries; public so tests elsewhere can build whole
/// gump packages.
pub mod gump_atlas;
pub mod single_gumps;
pub mod cc_radar;
pub mod cc_statics;
pub mod classic_sources;
pub mod classic_patches;
pub mod conversion_tables;
pub mod tex_land_cc;
pub mod tex_art_ec;
pub mod tex_land_ec;
pub mod package_progress;
mod progress;
pub mod source_paths;
pub mod tilemeta;
pub mod tex_aux_ec;
pub mod terrain_meta;
pub mod upscale_pipeline;
pub mod upscale_profile;
pub mod world_lights;
pub mod hues;
pub mod mobile_anim_cc;
pub mod mobile_anim_ec;
pub mod effects_ec;
mod prefix_search;
mod rgba_bounds;
pub mod upscale {
	pub use crate::upscale_pipeline::{
		apply_upscale_passes, apply_upscale_passes_owned, UpscalePass,
	};
	pub use uot_image_postprocess::upscaling::{
		apply_filter_passes, apply_filter_passes_owned, EnhancementFilter, UpscaleConfig,
		UpscaleFilter, UpscalePass as FilterUpscalePass,
	};
}

pub use udd_container::CompressionFlag as CompressionFlag;
pub use udd_container::{BuildProgress, BuildProgressPhase};
pub use udd_assets::tex_art_cc::PagePixelFormat as PagePixelFormat;

pub(crate) const BC7_BLOCK_DIM: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AtlasPackingMode {
	#[default]
	MaximumPacking,
	Bc7Oriented,
}

pub fn texture_atlas_packing_mode(pixel_format: PagePixelFormat) -> AtlasPackingMode {
	match pixel_format {
		PagePixelFormat::Rgba8888 => AtlasPackingMode::MaximumPacking,
		PagePixelFormat::Bc7 => AtlasPackingMode::Bc7Oriented,
	}
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
	// Every pixel of the rect outside the content takes the nearest content pixel. Sources are content pixels,
	// which are never written, so the rows can be filled in any order.
	let left_end = content_x.clamp(rect_x, rect_right);
	let right_start = content_right.clamp(rect_x, rect_right);
	let pixel_offset = |x: u32, y: u32| (y as usize * atlas_width as usize + x as usize) * RGBA_BYTES;

	for y in rect_y..rect_bottom {
		let sample_y = y.clamp(content_y, content_bottom - 1);
		let row_inside_content = y >= content_y && y < content_bottom;
		if !row_inside_content && left_end < right_start {
			let src = pixel_offset(left_end, sample_y);
			let len = (right_start - left_end) as usize * RGBA_BYTES;
			pixels.copy_within(src..src + len, pixel_offset(left_end, y));
		}
		fill_rgba_run(pixels, pixel_offset(rect_x, y), left_end - rect_x, pixel_offset(content_x, sample_y));
		fill_rgba_run(
			pixels,
			pixel_offset(right_start, y),
			rect_right - right_start,
			pixel_offset(content_right - 1, sample_y),
		);
	}
}

const RGBA_BYTES: usize = 4;

/// Writes the pixel at byte offset `src` to the `count` pixels starting at byte offset `dst`.
fn fill_rgba_run(pixels: &mut [u8], dst: usize, count: u32, src: usize) {
	if count == 0 {
		return;
	}
	let rgba: [u8; RGBA_BYTES] = pixels[src..src + RGBA_BYTES].try_into().expect("one RGBA pixel");
	for pixel in pixels[dst..dst + count as usize * RGBA_BYTES].chunks_exact_mut(RGBA_BYTES) {
		pixel.copy_from_slice(&rgba);
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

	/// The per-pixel extrusion the row-wise one replaced.
	fn extrude_rgba_rect_edges_per_pixel(
		pixels: &mut [u8],
		atlas_width: u32,
		atlas_height: u32,
		rect: (u32, u32, u32, u32),
		content: (u32, u32, u32, u32),
	) {
		let (rect_x, rect_y, rect_width, rect_height) = rect;
		let (content_x, content_y, content_width, content_height) = content;
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

	#[test]
	fn extrude_rgba_rect_edges_matches_the_per_pixel_extrusion() {
		const ATLAS_WIDTH: u32 = 13;
		const ATLAS_HEIGHT: u32 = 11;
		const MAX_PADDING: u32 = 3;
		const MAX_CONTENT: u32 = 5;
		let base: Vec<u8> = (0..ATLAS_WIDTH * ATLAS_HEIGHT * 4).map(|i| (i * 37 % 251) as u8).collect();
		for content_width in 1..=MAX_CONTENT {
			for content_height in 1..=MAX_CONTENT {
				for left in 0..=MAX_PADDING {
					for top in 0..=MAX_PADDING {
						for right in 0..=MAX_PADDING {
							for bottom in 0..=MAX_PADDING {
								for (overshoot_x, overshoot_y) in (0..=right).flat_map(|x| (0..=bottom).map(move |y| (x, y))) {
									// Rects reaching past the atlas edge by the overshoot are clipped by both.
									let rect = (
										ATLAS_WIDTH - content_width - left - right + overshoot_x,
										ATLAS_HEIGHT - content_height - top - bottom + overshoot_y,
										left + content_width + right,
										top + content_height + bottom,
									);
									let content = (rect.0 + left, rect.1 + top, content_width, content_height);
									let mut expected = base.clone();
									extrude_rgba_rect_edges_per_pixel(&mut expected, ATLAS_WIDTH, ATLAS_HEIGHT, rect, content);
									let mut actual = base.clone();
									extrude_rgba_rect_edges(
										&mut actual,
										ATLAS_WIDTH,
										ATLAS_HEIGHT,
										rect.0,
										rect.1,
										rect.2,
										rect.3,
										content.0,
										content.1,
										content.2,
										content.3,
									);
									assert_eq!(actual, expected, "rect {rect:?}, content {content:?}");
								}
							}
						}
					}
				}
			}
		}
	}

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

			let top_left = ((6 + 1) * 4) as usize;
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
