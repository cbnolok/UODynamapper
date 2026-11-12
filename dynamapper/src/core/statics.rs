use bevy::prelude::*;
use bytemuck::try_cast_slice;
use color_eyre::eyre;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;
use uocf::classic::statics::{PackedStaticTile, StaticTile};
use udd_container::UddpReader;

const PACKAGE_CHUNK_BLOCK_DIM: u32 = 4;
const PACKAGE_CHUNK_TILE_DIM: u8 = 32;

#[derive(Resource)]
pub struct StaticsStoreRes(pub Vec<Option<Mutex<LazyStaticsStore>>>);

pub struct LazyStaticsStore {
    package: Arc<UddpReader>,
    block_width: u32,
    block_height: u32,
    package_chunk_height: u32,
    loaded_package_chunks: Vec<bool>,
    cached_blocks: Vec<Option<Box<[PackedStaticTile]>>>,
    /// The item each static graphic draws (the route file's `item` routes), applied as a chunk loads; `None` draws
    /// every graphic as it is.
    item_routes: Option<Arc<[u16]>>,
    /// Counts the changes of `item_routes`, so the caches built from the blocks are rebuilt.
    item_routes_revision: u64,
}

impl LazyStaticsStore {
    pub fn new(package_path: &Path, width: u32, height: u32) -> eyre::Result<Self> {
        let package_path = package_path.canonicalize()?;
        let package =
            Arc::new(UddpReader::load(&package_path).map_err(|error| eyre::eyre!("{error}"))?);
        let block_width = width / 8;
        let block_height = height / 8;
        let package_chunk_width = block_width.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
        let package_chunk_height = block_height.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
        let num_blocks = (block_width * block_height) as usize;
        let num_package_chunks = (package_chunk_width * package_chunk_height) as usize;
        let cached_blocks = std::iter::repeat_with(|| None).take(num_blocks).collect();

        Ok(Self {
            package,
            block_width,
            block_height,
            package_chunk_height,
            loaded_package_chunks: vec![false; num_package_chunks],
            cached_blocks,
            item_routes: None,
            item_routes_revision: 0,
        })
    }

    /// Routes the static graphics through `item_routes` from now on: when they change, the blocks read so far are
    /// dropped and read again. Multis are not routed, as the Classic client does not switch them.
    pub fn set_item_routes(&mut self, item_routes: Option<Arc<[u16]>>) {
        if self.item_routes.as_deref() == item_routes.as_deref() {
            return;
        }
        self.item_routes = item_routes;
        self.item_routes_revision += 1;
        self.loaded_package_chunks.fill(false);
        self.cached_blocks.fill(None);
    }

    pub fn item_routes_revision(&self) -> u64 {
        self.item_routes_revision
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
                graphic: route_static_graphic(self.item_routes.as_deref(), tile.graphic),
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

/// The graphic a static draws under `item_routes`. Multis (graphics from `MULTI_STATIC_GRAPHIC_OFFSET`) keep theirs.
fn route_static_graphic(item_routes: Option<&[u16]>, graphic: u16) -> u16 {
    if graphic >= crate::core::multis::MULTI_STATIC_GRAPHIC_OFFSET {
        return graphic;
    }
    item_routes.and_then(|routes| routes.get(usize::from(graphic)).copied()).unwrap_or(graphic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_routes_switch_statics_but_not_multis() {
        let mut routes = (0..=u16::MAX).collect::<Vec<_>>();
        routes[0x0CA7] = 0x0C84;
        routes[usize::from(crate::core::multis::MULTI_STATIC_GRAPHIC_OFFSET)] = 1;
        let routes = Arc::<[u16]>::from(routes);
        assert_eq!(route_static_graphic(Some(&routes), 0x0CA7), 0x0C84);
        assert_eq!(route_static_graphic(Some(&routes), 0x0CA8), 0x0CA8);
        assert_eq!(route_static_graphic(Some(&routes), crate::core::multis::MULTI_STATIC_GRAPHIC_OFFSET),
            crate::core::multis::MULTI_STATIC_GRAPHIC_OFFSET);
        assert_eq!(route_static_graphic(None, 0x0CA7), 0x0CA7);
    }
}
