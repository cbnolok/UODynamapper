//! `facetNN.mul` world map images ([`crate::classic::facet_image`]) rendered from a facet's map and statics: the
//! slope rule ([`FacetImageRenderer`]), the tiles an edit re-renders ([`changed_tiles`],
//! [`FacetImageRenderer::update`]), and what a conversion writes for a client that draws the images
//! ([`plan_facet_image`]).
//!
//! **The rule is an approximation of how the shipped images were made**, measured on the 2024 client
//! (`scripts/dev-helpers/ec-research/ghidra-classic-map-image/README.md`). A tile takes the `radarcol.mul` colour
//! of its highest static by (z + `tiledata.mul` height, z), when that top reaches the land z, else of its land
//! tile. Every 5-bit channel is then shifted by the land slope `s = z(x + 1, y) - z(x, y + 1)` (the edge tiles
//! use their own z past the edge), clamped to 0..=31: -1 when `|s| <= 2`, +3 when `s >= 3`, -4 when `s <= -3`.
//! The pixels it gives exactly, and within one 5-bit step, of the shipped standard-edition images:
//!
//! | Facet | Exact | Within one step |
//! |---|---|---|
//! | 0 Felucca | 93.9% | 96.2% |
//! | 2 Ilshenar | 83.9% | 94.1% |
//! | 3 Malas | 95.4% | 98.6% |
//! | 4 Tokuno | 94.4% | 96.7% |
//! | 5 Ter Mur | 90.4% | 94.4% |
//!
//! The rest is not in the map data: steeper slopes darken by -11 at times, about 5% of pixels do not shift every
//! channel alike, and Ilshenar has black regions (822,295 pixels with bit 15 set, drawn black, and plain black
//! over land). So an edit re-renders only the tiles it changes, and the tiles left of and above a tile whose land
//! z changed (their slope reads it), keeping every other pixel as shipped; a bit-15 pixel of a tile the edit did
//! not change is kept too.

crate::eyre_imports!();
use std::collections::BTreeMap;
use std::path::Path;

use crate::classic::facet_image::{FacetImageFile, UNDRAWN_BIT};
use crate::classic::map::MapBlock;
use crate::classic::radarcol::load_radarcol;
use crate::classic::statics_write::StaticRecord;
use crate::classic::tiledata::TileData;
use crate::classic::world::ClassicWorld;
use crate::utils::color::Rgb555;
use crate::utils::path::find_file_ignoring_case;

/// `radarcol.mul` holds the land colours, then the item colours from this index on.
pub const RADARCOL_ITEM_BASE: usize = 0x4000;
/// A slope this steep or steeper (either way) changes the shift.
pub const STEEP_SLOPE: i32 = 3;
/// The channel shift of a tile on flat land or a gentle slope.
pub const FLAT_SHIFT: i32 = -1;
/// The channel shift of a tile whose land rises towards (x + 1, y) by [`STEEP_SLOPE`] or more.
pub const RISING_SHIFT: i32 = 3;
/// The channel shift of a tile whose land falls towards (x + 1, y) by [`STEEP_SLOPE`] or more.
pub const FALLING_SHIFT: i32 = -4;
/// About this share of a whole rendered image matches the shipped style (Felucca and Tokuno; the module table).
pub const SHIPPED_STYLE_MATCH_PERCENT: u32 = 94;
const RADARCOL_FILE: &str = "radarcol.mul";
const TILEDATA_FILE: &str = "tiledata.mul";
const CHANNEL_BITS: u32 = 5;
const CHANNEL_MAX: i32 = (1 << CHANNEL_BITS) - 1;
const RED_SHIFT: u32 = 2 * CHANNEL_BITS;
const GREEN_SHIFT: u32 = CHANNEL_BITS;
const CELLS_PER_BLOCK: usize = (MapBlock::CELLS_PER_ROW * MapBlock::CELLS_PER_COLUMN) as usize;

/// The channel shift for land slope `slope` (module docs).
pub fn slope_shift(slope: i32) -> i32 {
    if slope >= STEEP_SLOPE {
        RISING_SHIFT
    } else if slope <= -STEEP_SLOPE {
        FALLING_SHIFT
    } else {
        FLAT_SHIFT
    }
}

/// `colour` with every channel shifted by `shift`, clamped; bit 15 clear.
pub fn shifted_colour(colour: u16, shift: i32) -> u16 {
    let channel = |at: u32| ((i32::from(colour >> at) & CHANNEL_MAX) + shift).clamp(0, CHANNEL_MAX) as u16;
    channel(RED_SHIFT) << RED_SHIFT | channel(GREEN_SHIFT) << GREEN_SHIFT | channel(0)
}

/// A tile an edit changed, and whether its land z changed (then the tiles left of and above it re-render too).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangedTile {
    pub x: u32,
    pub y: u32,
    pub land_z_changed: bool,
}

/// The tiles whose land id, land z or statics differ between two facets of the same size.
pub fn changed_tiles(old: &ClassicWorld, new: &ClassicWorld) -> eyre::Result<Vec<ChangedTile>> {
    let (a, b) = (old.size_cells(), new.size_cells());
    eyre::ensure!((a.width, a.height) == (b.width, b.height), "the maps are {}x{} and {}x{}", a.width, a.height, b.width, b.height);
    let height_blocks = new.map.size_blocks().height as usize;
    let mut changed = Vec::new();
    for block in 0..new.map.block_count() {
        let (old_cells, new_cells) = (old.map.block_cells(block), new.map.block_cells(block));
        let (old_statics, new_statics) = (old.statics.block(block), new.statics.block(block));
        if old_cells == new_cells && old_statics == new_statics {
            continue;
        }
        let (block_x, block_y) = ((block / height_blocks) as u32, (block % height_blocks) as u32);
        for cell in 0..CELLS_PER_BLOCK {
            let (cell_x, cell_y) = (cell as u32 % MapBlock::CELLS_PER_ROW, cell as u32 / MapBlock::CELLS_PER_ROW);
            let in_cell = |record: &&StaticRecord| u32::from(record.x) == cell_x && u32::from(record.y) == cell_y;
            let statics_differ = !old_statics.iter().filter(in_cell).eq(new_statics.iter().filter(in_cell));
            if old_cells[cell] != new_cells[cell] || statics_differ {
                changed.push(ChangedTile {
                    x: block_x * MapBlock::CELLS_PER_ROW + cell_x,
                    y: block_y * MapBlock::CELLS_PER_COLUMN + cell_y,
                    land_z_changed: old_cells[cell].1 != new_cells[cell].1,
                });
            }
        }
    }
    Ok(changed)
}

/// What [`FacetImageRenderer::update`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImageUpdate {
    /// Tiles rendered by the rule.
    pub tiles_rendered: usize,
    /// Pixels whose stored colour changed.
    pub pixels_changed: usize,
    /// Rows re-encoded.
    pub rows_changed: usize,
    /// Bit-15 (drawn black) pixels of unchanged tiles left as they were.
    pub undrawn_kept: usize,
}

/// Renders tiles by the slope rule (module docs) from a client's `radarcol.mul` and `tiledata.mul` heights.
#[derive(Debug, Clone)]
pub struct FacetImageRenderer {
    radarcol: Vec<u16>,
    item_heights: Vec<i8>,
}

impl FacetImageRenderer {
    /// From the radar colours and every item's `tiledata.mul` height (an item past the list has height 0).
    pub fn new(radarcol: Vec<u16>, item_heights: Vec<i8>) -> Self {
        Self { radarcol, item_heights }
    }

    pub fn from_files(radarcol: &[Rgb555], tiledata: &TileData) -> Self {
        Self::new(
            radarcol.iter().map(Rgb555::value).collect(),
            tiledata.item_tiles().iter().map(|tile| tile.height_raw()).collect(),
        )
    }

    /// From the `radarcol.mul` and `tiledata.mul` of a client folder.
    pub fn load(dir: &Path) -> eyre::Result<Self> {
        let find = |name: &str| find_file_ignoring_case(dir, name).ok_or_else(|| eyre!("no {name} in {}", dir.display()));
        let radarcol_path = find(RADARCOL_FILE)?;
        let radarcol = load_radarcol(&radarcol_path).wrap_err_with(|| radarcol_path.display().to_string())?;
        let tiledata = TileData::load(find(TILEDATA_FILE)?)?;
        Ok(Self::from_files(&radarcol, &tiledata))
    }

    /// The radar colour of a tile before shading: its highest static reaching the land, else the land.
    fn base_colour(&self, land: u16, land_z: i8, statics: &[StaticRecord]) -> u16 {
        let height = |graphic: u16| i16::from(self.item_heights.get(usize::from(graphic)).copied().unwrap_or(0));
        let top = statics.iter().max_by_key(|record| (i16::from(record.z) + height(record.graphic), record.z));
        let item = top
            .filter(|record| i16::from(record.z) + height(record.graphic) >= i16::from(land_z))
            .and_then(|record| self.radarcol.get(RADARCOL_ITEM_BASE + usize::from(record.graphic)));
        item.or_else(|| self.radarcol.get(usize::from(land))).copied().unwrap_or(0)
    }

    /// The pixel of tile (x, y).
    pub fn tile_colour(&self, world: &ClassicWorld, x: u32, y: u32) -> eyre::Result<u16> {
        let size = world.size_cells();
        let (land, z) = world.map.cell(x, y)?;
        let base = self.base_colour(land, z, &world.statics.cell_statics(x, y)?);
        let (_, right_z) = world.map.cell((x + 1).min(size.width - 1), y)?;
        let (_, below_z) = world.map.cell(x, (y + 1).min(size.height - 1))?;
        Ok(shifted_colour(base, slope_shift(i32::from(right_z) - i32::from(below_z))))
    }

    /// The whole image of `world`.
    pub fn render(&self, world: &ClassicWorld) -> eyre::Result<FacetImageFile> {
        let size = world.size_cells();
        let (width, height) = (
            u16::try_from(size.width).map_err(|_| eyre!("a {}-tile wide map is wider than an image holds", size.width))?,
            u16::try_from(size.height).map_err(|_| eyre!("a {}-tile tall map is taller than an image holds", size.height))?,
        );
        let (w, h) = (usize::from(width), usize::from(height));
        let mut base = vec![0u16; w * h];
        let mut z = vec![0i8; w * h];
        let height_blocks = world.map.size_blocks().height as usize;
        let mut cell_statics: Vec<Vec<StaticRecord>> = vec![Vec::new(); CELLS_PER_BLOCK];
        for block in 0..world.map.block_count() {
            let (block_x, block_y) = ((block / height_blocks) as u32, (block % height_blocks) as u32);
            cell_statics.iter_mut().for_each(Vec::clear);
            for record in world.statics.block(block) {
                cell_statics[usize::from(record.y) * MapBlock::CELLS_PER_ROW as usize + usize::from(record.x)].push(*record);
            }
            for (cell, &(land, land_z)) in world.map.block_cells(block).iter().enumerate() {
                let x = (block_x * MapBlock::CELLS_PER_ROW) as usize + cell % MapBlock::CELLS_PER_ROW as usize;
                let y = (block_y * MapBlock::CELLS_PER_COLUMN) as usize + cell / MapBlock::CELLS_PER_ROW as usize;
                base[y * w + x] = self.base_colour(land, land_z, &cell_statics[cell]);
                z[y * w + x] = land_z;
            }
        }
        let mut pixels = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let slope = i32::from(z[y * w + (x + 1).min(w - 1)]) - i32::from(z[(y + 1).min(h - 1) * w + x]);
                pixels.push(shifted_colour(base[y * w + x], slope_shift(slope)));
            }
        }
        FacetImageFile::from_pixels(width, height, &pixels)
    }

    /// Re-renders in `image` (of `world`'s size) the `changed` tiles, and the tiles left of and above those whose
    /// land z changed; a bit-15 pixel of a tile not itself changed is kept. Only the rows touched are re-encoded.
    pub fn update(&self, world: &ClassicWorld, image: &mut FacetImageFile, changed: &[ChangedTile]) -> eyre::Result<ImageUpdate> {
        let size = world.size_cells();
        image.check_size(size.width, size.height)?;
        // (y, x) -> whether the tile itself changed.
        let mut tiles: BTreeMap<(u32, u32), bool> = BTreeMap::new();
        for tile in changed {
            tiles.insert((tile.y, tile.x), true);
            if tile.land_z_changed {
                for (x, y) in [(tile.x.checked_sub(1), Some(tile.y)), (Some(tile.x), tile.y.checked_sub(1))] {
                    if let (Some(x), Some(y)) = (x, y) {
                        tiles.entry((y, x)).or_insert(false);
                    }
                }
            }
        }
        let mut pixels = image.pixels();
        let width = size.width as usize;
        let mut report = ImageUpdate::default();
        for (&(y, x), &itself) in &tiles {
            let at = y as usize * width + x as usize;
            if !itself && pixels[at] & UNDRAWN_BIT != 0 {
                report.undrawn_kept += 1;
                continue;
            }
            let colour = self.tile_colour(world, x, y)?;
            report.tiles_rendered += 1;
            if colour != pixels[at] {
                report.pixels_changed += 1;
                pixels[at] = colour;
            }
        }
        report.rows_changed = image.set_pixels(&pixels)?;
        Ok(report)
    }
}

/// `--facet-image`: what a conversion writes for a client that draws the images.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FacetImageMode {
    /// The target's image as it is; refused when its size is not the map's.
    Keep,
    /// The target's image with the tiles that differ from its map re-rendered; the whole image rendered when the
    /// sizes differ or there is no image or map to start from.
    #[default]
    Update,
    /// The whole image rendered.
    Regenerate,
    /// No image; refused when the target's image is not the map's size.
    None,
}

/// Why [`plan_facet_image`] rendered the whole image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WholeRender {
    Requested,
    NoImage,
    /// The target's image is (width, height).
    SizeDiffers(u16, u16),
    /// The target's map could not be read to compare with.
    NoCurrentMap,
}

impl WholeRender {
    pub fn describe(self) -> String {
        match self {
            Self::Requested => "as requested".to_owned(),
            Self::NoImage => "the target has no image".to_owned(),
            Self::SizeDiffers(width, height) => format!("the target's image is {width}x{height}"),
            Self::NoCurrentMap => "the target's map could not be read to compare with".to_owned(),
        }
    }
}

/// The image [`plan_facet_image`] chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FacetImagePlan {
    /// The target's image as it is (its map is unchanged, or `keep`).
    Unchanged(FacetImageFile),
    /// The target's image with some tiles re-rendered.
    Updated(FacetImageFile, ImageUpdate),
    /// The whole image rendered by the rule: an approximation of the shipped style.
    Rendered(FacetImageFile, WholeRender),
    /// No image written.
    NotWritten,
}

/// The image to write with `world` for a client that draws the images: `current_image` and `current_world` are
/// the target's image and map as they are, `renderer` its colours (needed whenever a tile is rendered).
pub fn plan_facet_image(
    mode: FacetImageMode,
    world: &ClassicWorld,
    current_image: Option<&FacetImageFile>,
    current_world: Option<&ClassicWorld>,
    renderer: Option<&FacetImageRenderer>,
) -> eyre::Result<FacetImagePlan> {
    let size = world.size_cells();
    let misdrawn = current_image.filter(|image| image.check_size(size.width, size.height).is_err());
    let need_renderer = || renderer.ok_or_else(|| eyre!("rendering the image needs the target's radarcol.mul and tiledata.mul"));
    let refuse_stale = |image: &FacetImageFile, mode: &str| {
        eyre!(
            "--facet-image {mode}: the target's image is {}x{} and the map {}x{}; the client ignores the image's header \
             and would draw the wrong tiles: use update or regenerate",
            image.width(),
            image.height(),
            size.width,
            size.height
        )
    };
    match mode {
        FacetImageMode::Keep => match (current_image, misdrawn) {
            (_, Some(image)) => Err(refuse_stale(image, "keep")),
            (Some(image), None) => Ok(FacetImagePlan::Unchanged(image.clone())),
            (None, None) => Ok(FacetImagePlan::NotWritten),
        },
        FacetImageMode::None => match misdrawn {
            Some(image) => Err(refuse_stale(image, "none")),
            None => Ok(FacetImagePlan::NotWritten),
        },
        FacetImageMode::Regenerate => Ok(FacetImagePlan::Rendered(need_renderer()?.render(world)?, WholeRender::Requested)),
        FacetImageMode::Update => {
            let reason = match (current_image, misdrawn) {
                (None, _) => WholeRender::NoImage,
                (Some(image), Some(_)) => WholeRender::SizeDiffers(image.width(), image.height()),
                (Some(image), None) => {
                    let current = current_world.filter(|current| {
                        let c = current.size_cells();
                        (c.width, c.height) == (size.width, size.height)
                    });
                    match current {
                        Some(current) => {
                            let changed = changed_tiles(current, world)?;
                            if changed.is_empty() {
                                return Ok(FacetImagePlan::Unchanged(image.clone()));
                            }
                            let mut image = image.clone();
                            let update = need_renderer()?.update(world, &mut image, &changed)?;
                            return Ok(FacetImagePlan::Updated(image, update));
                        }
                        None => WholeRender::NoCurrentMap,
                    }
                }
            };
            Ok(FacetImagePlan::Rendered(need_renderer()?.render(world)?, reason))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::map::MapSizeCells;
    use crate::classic::world::LandFill;

    const SIZE: MapSizeCells = MapSizeCells { width: 16, height: 16 };
    const GROWN: MapSizeCells = MapSizeCells { width: 24, height: 16 };
    const LAND: u16 = 3;
    const ITEM: u16 = 0x10;
    const LAND_COLOUR: u16 = 10 << RED_SHIFT | 10 << GREEN_SHIFT | 10;
    const ITEM_COLOUR: u16 = 20 << RED_SHIFT | 5 << GREEN_SHIFT | 1;
    const ITEM_HEIGHT: i8 = 5;
    const HILL_Z: i8 = 10;

    fn renderer() -> FacetImageRenderer {
        let mut radarcol = vec![0u16; RADARCOL_ITEM_BASE + usize::from(ITEM) + 1];
        radarcol[usize::from(LAND)] = LAND_COLOUR;
        radarcol[RADARCOL_ITEM_BASE + usize::from(ITEM)] = ITEM_COLOUR;
        let mut heights = vec![0i8; usize::from(ITEM) + 1];
        heights[usize::from(ITEM)] = ITEM_HEIGHT;
        FacetImageRenderer::new(radarcol, heights)
    }

    fn world() -> ClassicWorld {
        ClassicWorld::filled(SIZE, LandFill { land: LAND, z: 0 }).unwrap()
    }

    #[test]
    fn the_slope_rule_shades_by_the_land_right_and_below() {
        assert_eq!((slope_shift(2), slope_shift(3), slope_shift(-3)), (FLAT_SHIFT, RISING_SHIFT, FALLING_SHIFT));
        assert_eq!(shifted_colour(0x7fff, RISING_SHIFT), 0x7fff);
        assert_eq!(shifted_colour(0, FLAT_SHIFT), 0);
        let mut world = world();
        world.map.set_cell(5, 4, LAND, HILL_Z).unwrap();
        let renderer = renderer();
        assert_eq!(renderer.tile_colour(&world, 4, 4).unwrap(), shifted_colour(LAND_COLOUR, RISING_SHIFT));
        assert_eq!(renderer.tile_colour(&world, 5, 3).unwrap(), shifted_colour(LAND_COLOUR, FALLING_SHIFT));
        assert_eq!(renderer.tile_colour(&world, 8, 8).unwrap(), shifted_colour(LAND_COLOUR, FLAT_SHIFT));
        // A static under the land is not seen; one reaching it is.
        world.statics.add_static(8, 8, ITEM, -ITEM_HEIGHT - 1, 0).unwrap();
        assert_eq!(renderer.tile_colour(&world, 8, 8).unwrap(), shifted_colour(LAND_COLOUR, FLAT_SHIFT));
        world.statics.add_static(8, 8, ITEM, -ITEM_HEIGHT, 0).unwrap();
        assert_eq!(renderer.tile_colour(&world, 8, 8).unwrap(), shifted_colour(ITEM_COLOUR, FLAT_SHIFT));
        let image = renderer.render(&world).unwrap();
        let pixels = image.pixels();
        let at = |x: usize, y: usize| pixels[y * SIZE.width as usize + x];
        assert_eq!((at(4, 4), at(5, 3), at(8, 8)), (
            renderer.tile_colour(&world, 4, 4).unwrap(),
            renderer.tile_colour(&world, 5, 3).unwrap(),
            renderer.tile_colour(&world, 8, 8).unwrap()
        ));
    }

    #[test]
    fn an_edit_re_renders_its_tile_and_the_slope_neighbours_keeping_unchanged_black() {
        let renderer = renderer();
        let old = world();
        let mut image = renderer.render(&old).unwrap();
        let mut pixels = image.pixels();
        let width = SIZE.width as usize;
        pixels[3 * width + 4] |= UNDRAWN_BIT; // above the edit: unchanged, kept black
        image.set_pixels(&pixels).unwrap();
        let mut new = old.clone();
        new.map.set_cell(4, 4, LAND, HILL_Z).unwrap();
        let changed = changed_tiles(&old, &new).unwrap();
        assert_eq!(changed, [ChangedTile { x: 4, y: 4, land_z_changed: true }]);
        let update = renderer.update(&new, &mut image, &changed).unwrap();
        assert_eq!(update, ImageUpdate { tiles_rendered: 2, pixels_changed: 1, rows_changed: 1, undrawn_kept: 1 });
        let after = image.pixels();
        assert_eq!(after[4 * width + 3], shifted_colour(LAND_COLOUR, RISING_SHIFT));
        assert_ne!(after[3 * width + 4] & UNDRAWN_BIT, 0);
        // A static only re-renders its own tile.
        let mut statics = new.clone();
        statics.statics.add_static(9, 9, ITEM, 0, 0).unwrap();
        assert_eq!(changed_tiles(&new, &statics).unwrap(), [ChangedTile { x: 9, y: 9, land_z_changed: false }]);
    }

    #[test]
    fn plans_copy_update_render_or_refuse() {
        let renderer = renderer();
        let world = world();
        let image = renderer.render(&world).unwrap();
        let plan = |mode, world: &ClassicWorld| plan_facet_image(mode, world, Some(&image), Some(&self::world()), Some(&renderer));
        assert_eq!(plan(FacetImageMode::Update, &world).unwrap(), FacetImagePlan::Unchanged(image.clone()));
        let mut edited = world.clone();
        edited.map.set_cell(2, 2, LAND, HILL_Z).unwrap();
        assert!(matches!(plan(FacetImageMode::Update, &edited).unwrap(), FacetImagePlan::Updated(_, update) if update.tiles_rendered == 3));
        let (grown, _) = world.resized(GROWN, LandFill { land: LAND, z: 0 }, false).unwrap();
        let FacetImagePlan::Rendered(rendered, WholeRender::SizeDiffers(16, 16)) = plan(FacetImageMode::Update, &grown).unwrap() else {
            panic!("a resized map renders the whole image");
        };
        let read = FacetImageFile::from_bytes(&rendered.to_bytes()).unwrap();
        read.check_size(GROWN.width, GROWN.height).unwrap();
        assert!(format!("{:#}", plan(FacetImageMode::None, &grown).unwrap_err()).contains("wrong tiles"));
        assert!(plan(FacetImageMode::Keep, &grown).is_err());
        assert_eq!(plan(FacetImageMode::None, &world).unwrap(), FacetImagePlan::NotWritten);
        assert!(plan_facet_image(FacetImageMode::Regenerate, &world, None, None, None).is_err());
    }
}
