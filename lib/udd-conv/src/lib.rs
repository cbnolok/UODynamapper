#![allow(unused)]
pub mod bc7;
pub mod tex_art_cc;
pub mod cc_tex_land_ec_transcode;
pub mod cc_map;
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
