use bevy::prelude::*;
use bytemuck::try_cast_slice;
use color_eyre::eyre;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;
use uocf::classic::statics::{PackedStaticTile, StaticTile};
use udd_assets::map_metadata::read_statics_package_metadata;
use udd_container::UddpReader;

use crate::configs::land_routes::{AreaRoutes, MapRouteAreas};

const PACKAGE_CHUNK_BLOCK_DIM: u32 = 4;
const PACKAGE_CHUNK_TILE_DIM: u8 = 32;

#[derive(Resource)]
pub struct StaticsStoreRes(pub Vec<Option<Mutex<LazyStaticsStore>>>);

pub struct LazyStaticsStore {
    package: Arc<UddpReader>,
    map_id: u32,
    block_width: u32,
    block_height: u32,
    package_chunk_height: u32,
    loaded_package_chunks: Vec<bool>,
    cached_blocks: Vec<Option<Box<[PackedStaticTile]>>>,
    /// The item each static graphic draws (the route file's `item` routes) inside the route profile's areas and
    /// outside them, applied as a chunk loads by each static's position; `None` draws every graphic as it is.
    item_routes: Option<Arc<AreaRoutes<Vec<u16>>>>,
    /// The route profile's areas on this store's map.
    item_route_areas: MapRouteAreas,
    /// Counts the changes of `item_routes`, so the caches built from the blocks are rebuilt.
    item_routes_revision: u64,
}

impl LazyStaticsStore {
    pub fn new(package_path: &Path, map_id: u32, width: u32, height: u32) -> eyre::Result<Self> {
        let package_path = package_path.canonicalize()?;
        let package =
            Arc::new(UddpReader::load(&package_path).map_err(|error| eyre::eyre!("{error}"))?);
        let block_width = width / 8;
        let block_height = height / 8;
        let package_chunk_width = block_width.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
        let package_chunk_height = block_height.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
        let num_blocks = (block_width * block_height) as usize;
        let num_package_chunks = (package_chunk_width * package_chunk_height) as usize;
        let metadata = read_statics_package_metadata(&package).map_err(|error| eyre::eyre!("{error}"))?;
        eyre::ensure!(
            metadata.map_id == map_id,
            "statics package is for map {}, expected map {map_id}",
            metadata.map_id
        );
        eyre::ensure!(
            metadata.chunk_count as usize == num_package_chunks,
            "statics package has {} chunks, map {map_id} needs {num_package_chunks}",
            metadata.chunk_count
        );
        let cached_blocks = std::iter::repeat_with(|| None).take(num_blocks).collect();

        Ok(Self {
            package,
            map_id,
            block_width,
            block_height,
            package_chunk_height,
            loaded_package_chunks: vec![false; num_package_chunks],
            cached_blocks,
            item_routes: None,
            item_route_areas: MapRouteAreas::default(),
            item_routes_revision: 0,
        })
    }

    /// Routes the static graphics through `item_routes` from now on: when they change, the blocks read so far are
    /// dropped and read again. Multis are not routed, as the Classic client does not switch them.
    pub fn set_item_routes(&mut self, item_routes: Option<Arc<AreaRoutes<Vec<u16>>>>) {
        if self.item_routes.as_deref() == item_routes.as_deref() {
            return;
        }
        self.item_route_areas = item_routes.as_ref().map(|routes| routes.areas.on_map(self.map_id)).unwrap_or_default();
        self.item_routes = item_routes;
        self.item_routes_revision += 1;
        self.loaded_package_chunks.fill(false);
        self.cached_blocks.fill(None);
    }

    pub fn item_routes_revision(&self) -> u64 {
        self.item_routes_revision
    }

    /// The statics package, for readers that scan it apart from the store (caves.rs).
    pub fn package(&self) -> Arc<UddpReader> {
        self.package.clone()
    }

    fn block_index(&self, block_x: u32, block_y: u32) -> Option<usize> {
        if block_x >= self.block_width || block_y >= self.block_height {
            return None;
        }

        Some((block_x * self.block_height + block_y) as usize)
    }

    fn package_chunk_index(&self, block_x: u32, block_y: u32) -> u32 {
        (block_x / PACKAGE_CHUNK_BLOCK_DIM) * self.package_chunk_height
            + (block_y / PACKAGE_CHUNK_BLOCK_DIM)
    }

    pub fn block_tiles(&mut self, block_x: u32, block_y: u32) -> eyre::Result<&[PackedStaticTile]> {
        let Some(block_index) = self.block_index(block_x, block_y) else {
            return Ok(&[]);
        };

        if self.cached_blocks[block_index].is_none() {
            let package_chunk_index = self.package_chunk_index(block_x, block_y) as usize;
            if !self.loaded_package_chunks[package_chunk_index] {
                self.load_package_chunk(block_x, block_y)?;
            }
        }

        match self.cached_blocks[block_index].as_deref() {
            Some(tiles) => Ok(tiles),
            None => Ok(&[]),
        }
    }

    fn load_package_chunk(&mut self, block_x: u32, block_y: u32) -> eyre::Result<()> {
        let chunk_origin_x = (block_x / PACKAGE_CHUNK_BLOCK_DIM) * PACKAGE_CHUNK_BLOCK_DIM;
        let chunk_origin_y = (block_y / PACKAGE_CHUNK_BLOCK_DIM) * PACKAGE_CHUNK_BLOCK_DIM;
        // The chunk's first tile, which the statics' offsets count from.
        let chunk_tile_x = (block_x / PACKAGE_CHUNK_BLOCK_DIM) * u32::from(PACKAGE_CHUNK_TILE_DIM);
        let chunk_tile_y = (block_y / PACKAGE_CHUNK_BLOCK_DIM) * u32::from(PACKAGE_CHUNK_TILE_DIM);
        let package_chunk_index = self.package_chunk_index(block_x, block_y);

        let raw_bytes = self
            .package
            .read_file_by_dense_id(package_chunk_index)
            .map_err(|error| eyre::eyre!("{error}"))?;
        let static_tiles: &[StaticTile] = try_cast_slice(&raw_bytes)
            .map_err(|_| eyre::eyre!("invalid statics chunk size {}", raw_bytes.len()))?;

        let mut per_block_tiles = (0..(PACKAGE_CHUNK_BLOCK_DIM * PACKAGE_CHUNK_BLOCK_DIM))
            .map(|_| Vec::new())
            .collect::<Vec<Vec<PackedStaticTile>>>();

        for tile in static_tiles.iter().copied() {
            if tile.x_offset >= PACKAGE_CHUNK_TILE_DIM || tile.y_offset >= PACKAGE_CHUNK_TILE_DIM {
                return Err(eyre::eyre!(
                    "invalid statics chunk local offset ({}, {})",
                    tile.x_offset,
                    tile.y_offset
                ));
            }

            let local_block_x = (tile.x_offset / 8) as u32;
            let local_block_y = (tile.y_offset / 8) as u32;
            let block_slot = (local_block_x * PACKAGE_CHUNK_BLOCK_DIM + local_block_y) as usize;
            per_block_tiles[block_slot].push(PackedStaticTile {
                graphic: route_static_graphic(self.item_routes.as_deref(), &self.item_route_areas, tile.graphic,
                    chunk_tile_x + u32::from(tile.x_offset), chunk_tile_y + u32::from(tile.y_offset)),
                xy_packed: (tile.x_offset & 0x07) | ((tile.y_offset & 0x07) << 3),
                z: tile.z,
                hue: tile.hue,
            });
        }

        for local_block_x in 0..PACKAGE_CHUNK_BLOCK_DIM {
            for local_block_y in 0..PACKAGE_CHUNK_BLOCK_DIM {
                let native_block_x = chunk_origin_x + local_block_x;
                let native_block_y = chunk_origin_y + local_block_y;
                if native_block_x >= self.block_width || native_block_y >= self.block_height {
                    continue;
                }
                let block_slot = (local_block_x * PACKAGE_CHUNK_BLOCK_DIM + local_block_y) as usize;
                let native_block_index = self
                    .block_index(native_block_x, native_block_y)
                    .expect("validated native block index");
                self.cached_blocks[native_block_index] =
                    Some(std::mem::take(&mut per_block_tiles[block_slot]).into_boxed_slice());
            }
        }

        self.loaded_package_chunks[package_chunk_index as usize] = true;
        Ok(())
    }
}

/// The statics of the package chunk `chunk_index`, unrouted, with offsets from the chunk's first cell. The map
/// package numbers its chunks the same way (`maps::read_package_chunk_land_z`).
pub fn read_package_chunk_statics(package: &UddpReader, chunk_index: u32) -> eyre::Result<Vec<StaticTile>> {
    const TILE_BYTES: usize = std::mem::size_of::<StaticTile>();
    let bytes = package.read_file_by_dense_id_cow(chunk_index).map_err(|error| eyre::eyre!("{error}"))?;
    if bytes.len() % TILE_BYTES != 0 {
        return Err(eyre::eyre!("invalid statics chunk size {}", bytes.len()));
    }
    // A borrowed chunk may be unaligned: the statics are read from their bytes.
    Ok(bytes.chunks_exact(TILE_BYTES).map(bytemuck::pod_read_unaligned).collect())
}

/// The graphic a static at the tile `x`, `y` draws under `item_routes`: their inside table where `areas` (the
/// route profile's areas on its map) contain the tile, else their outside table. Multis (graphics from
/// `MULTI_STATIC_GRAPHIC_OFFSET`) keep theirs.
fn route_static_graphic(item_routes: Option<&AreaRoutes<Vec<u16>>>, areas: &MapRouteAreas, graphic: u16, x: u32, y: u32)
    -> u16 {
    if graphic >= crate::core::multis::MULTI_STATIC_GRAPHIC_OFFSET {
        return graphic;
    }
    let Some(routes) = item_routes else { return graphic };
    let table = match &routes.inside {
        Some(inside) if areas.contains(x, y) => inside,
        _ => &routes.outside,
    };
    table.get(usize::from(graphic)).copied().unwrap_or(graphic)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routes_of(text: &str, profile: &str) -> AreaRoutes<Vec<u16>> {
        crate::configs::land_routes::LandRoutesFile::decode("LandRoutes.kdl", text)
            .unwrap_or_else(|error| panic!("{error:#}")).item_routes_by_area(Some(profile)).unwrap()
    }

    #[test]
    fn item_routes_switch_statics_but_not_multis() {
        let multi = crate::core::multis::MULTI_STATIC_GRAPHIC_OFFSET;
        let routes = routes_of(&format!("profile \"a\" {{\n    item 0x0CA7 id=0x0C84\n    item {multi} id=1\n}}\n"), "a");
        let everywhere = MapRouteAreas::default();
        assert_eq!(route_static_graphic(Some(&routes), &everywhere, 0x0CA7, 0, 0), 0x0C84);
        assert_eq!(route_static_graphic(Some(&routes), &everywhere, 0x0CA8, 0, 0), 0x0CA8);
        assert_eq!(route_static_graphic(Some(&routes), &everywhere, multi, 0, 0), multi);
        assert_eq!(route_static_graphic(None, &everywhere, 0x0CA7, 0, 0), 0x0CA7);
    }

    /// Each static takes the profile's route by its own tile: inside the areas of its map, and outside the holes.
    #[test]
    fn item_routes_follow_each_static_position() {
        let routes = routes_of(
            "routes {\n    item 0x0C45 id=0x0C46\n}\nprofile \"desolation\" {\n    \
             apply-in x1=0 y1=0 x2=0x17FF y2=0xFFFF\n    skip-in x1=100 y1=100 x2=109 y2=109 map=1\n    \
             item 0x0C37 id=0x1BAE\n}\n", "desolation");
        let graphic = |map, graphic, x, y| route_static_graphic(Some(&routes), &routes.areas.on_map(map), graphic, x, y);
        for map in [0, 1, 5] {
            assert_eq!(graphic(map, 0x0C37, 0x17FF, 0x0FFF), 0x1BAE);
            assert_eq!(graphic(map, 0x0C37, 0x1800, 0x0FFF), 0x0C37);
            // The `routes` block applies on both sides.
            assert_eq!((graphic(map, 0x0C45, 0x17FF, 0), graphic(map, 0x0C45, 0x1800, 0)), (0x0C46, 0x0C46));
        }
        // The hole of map 1.
        assert_eq!((graphic(1, 0x0C37, 100, 100), graphic(1, 0x0C37, 109, 110)), (0x0C37, 0x1BAE));
        assert_eq!(graphic(0, 0x0C37, 100, 100), 0x1BAE);
    }
}
