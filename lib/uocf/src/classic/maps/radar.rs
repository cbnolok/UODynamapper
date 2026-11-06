//! The radar image of a Classic facet: one pixel per tile, coloured from `radarcol.mul`, as the inspector's Maps tab
//! draws it and its Multimap builder reads it.
//!
//! A tile takes the colour of its highest static, by its top (`z` plus the item's `tiledata.mul` height) and then by
//! its `z`, the later static winning a tie ([`radar_top_static`]), when that top is not below the land; else the
//! colour of its land tile. Land colours are `radarcol.mul[land id]` and static colours
//! `radarcol.mul[0x4000 + graphic]`; a colour past the end of the file is black. Hues and the land's slope do not
//! change the colour, unlike the shaded `facetNN.mul` images ([`crate::classic::facet_image_render`], which picks the
//! static the same way) and the Enhanced Client's `facetNN.dds` ([`crate::enhanced::facet_radar`]).

crate::eyre_imports!();
use std::path::Path;

use crate::classic::facet_image_render::RADARCOL_ITEM_BASE;
use crate::classic::map::{MapBlockRelPos, MapPlane};
use crate::classic::radarcol::load_radarcol;
use crate::classic::statics::{PackedStaticTile, StaticsStore, UO_BLOCK_DIM};
use crate::classic::tiledata::TileData;
use crate::utils::color::Rgb555;
use crate::utils::path::find_file_ignoring_case;

pub const RADARCOL_FILE: &str = "radarcol.mul";
/// Bytes per pixel of a [`RadarImage`].
pub const RADAR_RGBA_CHANNELS: usize = 4;
const OPAQUE: u8 = u8::MAX;
/// Shift of a 5-bit channel to 8 bits, and of its high bits into the low ones.
const CHANNEL_SHIFT: u32 = 3;
const CHANNEL_REFILL_SHIFT: u32 = 2;

/// The static a radar draws among a tile's `statics`, with its top: the highest top (`z` plus `height(graphic)`),
/// then the highest `z`; the later static on a tie. `graphic_and_z` reads a static's graphic and `z`.
pub fn radar_top_static<T>(
    statics: impl IntoIterator<Item = T>,
    graphic_and_z: impl Fn(&T) -> (u16, i8),
    height: impl Fn(u16) -> i16,
) -> Option<(T, i16)> {
    statics
        .into_iter()
        .map(|item| {
            let (graphic, z) = graphic_and_z(&item);
            let top = i16::from(z) + height(graphic);
            (item, top, z)
        })
        .max_by_key(|&(_, top, z)| (top, z))
        .map(|(item, top, _)| (item, top))
}

/// The colours a radar is drawn with: `radarcol.mul`, and each item's `tiledata.mul` height for picking the static.
#[derive(Debug, Clone)]
pub struct RadarColours {
    radarcol: Vec<Rgb555>,
    item_heights: Vec<i8>,
}

impl RadarColours {
    /// From the radar colours and every item's height (an item past the list has height 0).
    pub fn new(radarcol: Vec<Rgb555>, item_heights: Vec<i8>) -> Self {
        Self { radarcol, item_heights }
    }

    /// From the radar colours and the item heights of `tiledata`; without one every item has height 0.
    pub fn with_tiledata(radarcol: Vec<Rgb555>, tiledata: Option<&TileData>) -> Self {
        let item_heights =
            tiledata.map_or_else(Vec::new, |tiledata| tiledata.item_tiles().iter().map(|tile| tile.height_raw()).collect());
        Self::new(radarcol, item_heights)
    }

    /// The `tiledata.mul` height of item `graphic` (0 past the list).
    pub fn item_height(&self, graphic: u16) -> i16 {
        i16::from(self.item_heights.get(usize::from(graphic)).copied().unwrap_or(0))
    }

    /// The radar colour of a tile of land `land` at `land_z` with `statics` on it (module docs); `None` when
    /// `radarcol.mul` has no colour for the land or static drawn.
    pub fn tile_colour(&self, land: u16, land_z: i8, statics: impl IntoIterator<Item = PackedStaticTile>) -> Option<Rgb555> {
        match radar_top_static(statics, |tile| (tile.graphic, tile.z), |graphic| self.item_height(graphic)) {
            Some((tile, top)) if top >= i16::from(land_z) => {
                self.radarcol.get(RADARCOL_ITEM_BASE + usize::from(tile.graphic)).copied()
            }
            _ => self.radarcol.get(usize::from(land)).copied(),
        }
    }
}

/// `radarcol.mul` of client folder `dir` (the file name matched ignoring case).
pub fn load_client_radarcol(dir: &Path) -> eyre::Result<Vec<Rgb555>> {
    let path = find_file_ignoring_case(dir, RADARCOL_FILE).ok_or_else(|| eyre!("no {RADARCOL_FILE} in {}", dir.display()))?;
    load_radarcol(&path).wrap_err_with(|| path.display().to_string())
}

fn expand_channel(channel: u8) -> u8 {
    (channel << CHANNEL_SHIFT) | (channel >> CHANNEL_REFILL_SHIFT)
}

/// The opaque RGBA pixel of a radar colour, its 5-bit channels widened to 8 bits; black for none.
pub fn radar_rgba(colour: Option<Rgb555>) -> [u8; RADAR_RGBA_CHANNELS] {
    colour.map_or([0, 0, 0, OPAQUE], |colour| {
        [expand_channel(colour.r()), expand_channel(colour.g()), expand_channel(colour.b()), OPAQUE]
    })
}

/// The tiles per pixel side at which a `width` x `height` tile facet fits `max_side` pixels a side (1 when it fits).
pub fn radar_scale(width: u32, height: u32, max_side: u32) -> u32 {
    width.max(height).div_ceil(max_side.max(1)).max(1)
}

/// A radar render: RGBA rows of `width` x `height` pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadarImage {
    pub width: u32,
    pub height: u32,
    /// Tiles per pixel side: each pixel is the top left tile of its `scale` x `scale` square.
    pub scale: u32,
    pub rgba: Vec<u8>,
}

/// Renders the radar of the facet `plane` and `statics` hold at one pixel per `scale` x `scale` tiles (0 counts as
/// 1). The map is read one column of blocks at a time, and the blocks are let go after each column.
pub fn render_radar(plane: &mut MapPlane, statics: &StaticsStore, colours: &RadarColours, scale: u32) -> eyre::Result<RadarImage> {
    let size = plane.size_cells();
    let scale = scale.max(1);
    let (width, height) = (size.width.div_ceil(scale), size.height.div_ceil(scale));
    let mut rgba = vec![0u8; width as usize * height as usize * RADAR_RGBA_CHANNELS];
    let blocks_y = size.height / UO_BLOCK_DIM;
    for block_x in 0..size.width / UO_BLOCK_DIM {
        let mut column: Vec<MapBlockRelPos> = (0..blocks_y).map(|y| MapBlockRelPos { x: block_x, y }).collect();
        plane.load_blocks(&mut column)?;
        for block_y in 0..blocks_y {
            let block = plane
                .block(MapBlockRelPos { x: block_x, y: block_y })
                .ok_or_else(|| eyre!("block {block_x},{block_y} not read"))?;
            let tiles = statics.block_tiles(block_x, block_y);
            for cell_y in 0..UO_BLOCK_DIM {
                for cell_x in 0..UO_BLOCK_DIM {
                    let (x, y) = (block_x * UO_BLOCK_DIM + cell_x, block_y * UO_BLOCK_DIM + cell_y);
                    if x % scale != 0 || y % scale != 0 {
                        continue;
                    }
                    let land = block.cell(cell_x, cell_y)?;
                    let cell_statics = tiles
                        .iter()
                        .copied()
                        .filter(|tile| u32::from(tile.x_offset()) == cell_x && u32::from(tile.y_offset()) == cell_y);
                    let colour = colours.tile_colour(land.id, land.z, cell_statics);
                    let at = ((y / scale) as usize * width as usize + (x / scale) as usize) * RADAR_RGBA_CHANNELS;
                    rgba[at..at + RADAR_RGBA_CHANNELS].copy_from_slice(&radar_rgba(colour));
                }
            }
        }
        plane.evict_idle_blocks(std::time::Duration::ZERO);
    }
    Ok(RadarImage { width, height, scale, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::facet_sources::{ClassicDifOptions, ClassicFacetSources, ClassicMapFormat};
    use crate::classic::map::MapSizeCells;
    use crate::classic::world::{ClassicWorld, LandFill};
    use crate::map_edition::MapEdition;

    const FACET: u32 = 0;
    const SIZE: MapSizeCells = MapSizeCells { width: 16, height: 16 };
    const LAND: u16 = 3;
    const OTHER_LAND: u16 = 4;
    const ITEM: u16 = 0x10;
    const OTHER_ITEM: u16 = 0x11;
    const THIRD_ITEM: u16 = 0x12;
    /// An item id past the end of the test `radarcol.mul`.
    const UNCOLOURED_ITEM: u16 = 0x40;
    const ITEM_HEIGHT: i8 = 5;
    const HILL_Z: i8 = 10;
    const RED_SHIFT: u16 = 10;
    const GREEN_SHIFT: u16 = 5;
    const LAND_COLOUR: u16 = rgb555(10, 10, 10);
    const OTHER_LAND_COLOUR: u16 = rgb555(3, 20, 7);
    const ITEM_COLOUR: u16 = rgb555(31, 5, 1);
    const OTHER_ITEM_COLOUR: u16 = rgb555(2, 30, 16);
    const HALF_SCALE: u32 = 2;
    const BLACK: [u8; RADAR_RGBA_CHANNELS] = [0, 0, 0, OPAQUE];

    const fn rgb555(r: u16, g: u16, b: u16) -> u16 {
        r << RED_SHIFT | g << GREEN_SHIFT | b
    }

    fn colours() -> RadarColours {
        let mut radarcol = vec![Rgb555::new_from_val(0); RADARCOL_ITEM_BASE + usize::from(THIRD_ITEM) + 1];
        radarcol[usize::from(LAND)] = Rgb555::new_from_val(LAND_COLOUR);
        radarcol[usize::from(OTHER_LAND)] = Rgb555::new_from_val(OTHER_LAND_COLOUR);
        radarcol[RADARCOL_ITEM_BASE + usize::from(ITEM)] = Rgb555::new_from_val(ITEM_COLOUR);
        radarcol[RADARCOL_ITEM_BASE + usize::from(OTHER_ITEM)] = Rgb555::new_from_val(OTHER_ITEM_COLOUR);
        let mut heights = vec![0i8; usize::from(ITEM) + 1];
        heights[usize::from(ITEM)] = ITEM_HEIGHT;
        RadarColours::new(radarcol, heights)
    }

    fn rgba(value: u16) -> [u8; RADAR_RGBA_CHANNELS] {
        radar_rgba(Some(Rgb555::new_from_val(value)))
    }

    fn static_tile(graphic: u16, z: i8) -> PackedStaticTile {
        PackedStaticTile { graphic, xy_packed: 0, z, hue: 0 }
    }

    #[test]
    fn channels_widen_to_eight_bits_and_a_missing_colour_is_black() {
        assert_eq!(rgba(0x7fff), [u8::MAX, u8::MAX, u8::MAX, OPAQUE]);
        assert_eq!(rgba(rgb555(16, 1, 0)), [132, 8, 0, OPAQUE]);
        assert_eq!(radar_rgba(None), BLACK);
    }

    #[test]
    fn the_top_static_is_the_highest_top_then_z_the_later_on_a_tie() {
        let colours = colours();
        let height = |graphic| colours.item_height(graphic);
        let pick = |tiles: &[PackedStaticTile]| {
            radar_top_static(tiles.iter().copied(), |tile| (tile.graphic, tile.z), height).map(|(tile, top)| (tile.graphic, top))
        };
        // ITEM at 0 tops out at 5, above OTHER_ITEM at 4.
        assert_eq!(pick(&[static_tile(ITEM, 0), static_tile(OTHER_ITEM, 4)]), Some((ITEM, 5)));
        // Equal tops: the higher z.
        assert_eq!(pick(&[static_tile(OTHER_ITEM, 5), static_tile(ITEM, 0)]), Some((OTHER_ITEM, 5)));
        // Equal top and z: the later one.
        assert_eq!(pick(&[static_tile(ITEM, 0), static_tile(OTHER_ITEM, 5), static_tile(THIRD_ITEM, 5)]), Some((THIRD_ITEM, 5)));
        assert_eq!(pick(&[]), None);
    }

    /// A 16 x 16 facet written as `map0.mul`, `staidx0.mul` and `statics0.mul` and read back as the Maps tab reads
    /// it.
    #[test]
    fn a_synthetic_facet_renders_by_the_radar_rule() {
        let mut world = ClassicWorld::filled(SIZE, LandFill { land: LAND, z: 0 }).unwrap();
        world.map.set_cell(1, 0, LAND, HILL_Z).unwrap();
        world.map.set_cell(2, 0, OTHER_LAND, 0).unwrap();
        // Below the hill: not drawn.
        world.statics.add_static(1, 0, ITEM, HILL_Z - ITEM_HEIGHT - 1, 0).unwrap();
        // Its top reaches the land: drawn.
        world.statics.add_static(3, 0, ITEM, -ITEM_HEIGHT, 0).unwrap();
        // The higher top wins over the higher z.
        world.statics.add_static(4, 2, OTHER_ITEM, 4, 0).unwrap();
        world.statics.add_static(4, 2, ITEM, 0, 0).unwrap();
        world.statics.add_static(9, 9, UNCOLOURED_ITEM, 0, 0).unwrap();
        world.statics.add_static(10, 11, OTHER_ITEM, 0, 0).unwrap();

        let dir = std::env::temp_dir().join(format!("uocf_radar_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        world.map.write_mul(&dir.join("map0.mul")).unwrap();
        world.statics.write_mul(&dir.join("staidx0.mul"), &dir.join("statics0.mul")).unwrap();
        let sources = ClassicFacetSources::resolve(
            std::slice::from_ref(&dir),
            FACET,
            &[ClassicMapFormat::Mul],
            ClassicDifOptions::NONE,
            MapEdition::Standard,
        )
        .unwrap();
        let mut plane = sources.load_map_plane(Some(SIZE), None).unwrap();
        let statics = sources.load_statics_reader(SIZE.width, SIZE.height, None).unwrap().load_all().unwrap();
        let colours = colours();
        let image = render_radar(&mut plane, &statics, &colours, 1).unwrap();
        let half = render_radar(&mut plane, &statics, &colours, HALF_SCALE).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!((image.width, image.height, image.scale), (SIZE.width, SIZE.height, 1));
        assert_eq!(image.rgba.len(), (SIZE.width * SIZE.height) as usize * RADAR_RGBA_CHANNELS);
        let at = |image: &RadarImage, x: u32, y: u32| {
            let at = (y * image.width + x) as usize * RADAR_RGBA_CHANNELS;
            <[u8; RADAR_RGBA_CHANNELS]>::try_from(&image.rgba[at..at + RADAR_RGBA_CHANNELS]).unwrap()
        };
        assert_eq!(at(&image, 0, 0), rgba(LAND_COLOUR));
        assert_eq!(at(&image, 1, 0), rgba(LAND_COLOUR));
        assert_eq!(at(&image, 2, 0), rgba(OTHER_LAND_COLOUR));
        assert_eq!(at(&image, 3, 0), rgba(ITEM_COLOUR));
        assert_eq!(at(&image, 4, 2), rgba(ITEM_COLOUR));
        assert_eq!(at(&image, 9, 9), BLACK);
        assert_eq!(at(&image, 10, 11), rgba(OTHER_ITEM_COLOUR));
        assert_eq!(at(&image, SIZE.width - 1, SIZE.height - 1), rgba(LAND_COLOUR));

        assert_eq!((half.width, half.height, half.scale), (SIZE.width / HALF_SCALE, SIZE.height / HALF_SCALE, HALF_SCALE));
        assert_eq!(at(&half, 2, 1), at(&image, 4, 2));
        assert_eq!(at(&half, 1, 0), rgba(OTHER_LAND_COLOUR));
        assert_eq!(at(&half, 5, 5), rgba(LAND_COLOUR));
    }

    #[test]
    fn the_scale_fits_the_longer_side() {
        assert_eq!(radar_scale(7168, 4096, 16384), 1);
        assert_eq!(radar_scale(7168, 4096, 4096), 2);
        assert_eq!(radar_scale(100, 100, 0), 100);
    }
}
