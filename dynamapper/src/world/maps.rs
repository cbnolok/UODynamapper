use bevy::ecs::resource::Resource;
use bytemuck::{Pod, Zeroable};
use color_eyre::eyre::{self, WrapErr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use udd_assets::map_metadata::{decode_map_package_metadata, MapPackageMetadata};
use uocf::classic::map::{MapBlock, MapBlockRelPos, MapCell, MapSizeBlocks, MapSizeCells};
use udd_container::{unpack_type, DataType, FileKey, FileRecord, UddpReader};

const PACKAGE_CHUNK_BLOCK_DIM: u32 = 4;
const PACKAGE_CHUNK_TILE_DIM: usize = 32;
const PACKAGE_CHUNK_TEXEL_COUNT: usize = PACKAGE_CHUNK_TILE_DIM * PACKAGE_CHUNK_TILE_DIM;

#[derive(Resource, Default)]
pub struct MapPlaneMetadata {
    pub id: u8,
    pub width: u32,
    pub height: u32,
}

/// The catalog of the loaded maps: each plane's size in tiles, by map id.
#[derive(Resource, Default)]
pub struct WorldGeoData {
    pub maps: indexmap::IndexMap<u32, MapPlaneMetadata, nohash_hasher::BuildNoHashHasher<u32>>,
}

/// Fills the catalog from the loaded map planes, once, at startup.
pub(crate) fn sys_populate_world_geo_data(
    map_planes_res: Option<bevy::prelude::Res<crate::content::MapPlanesRes>>,
    mut world_geo_data_res: bevy::prelude::ResMut<WorldGeoData>,
) {
    crate::util_lib::tracked_plugin::log_system_add_startup::<crate::render::scene::world::WorldPlugin>(
        crate::app::system_sets::StartupSystems::SetupSceneStage1,
        crate::fname!(),
    );
    let Some(map_planes_res) = map_planes_res else {
        return;
    };
    if world_geo_data_res.maps.is_empty() {
        for (id, plane) in map_planes_res.0.iter().enumerate() {
            // MapPlanesRes now contains Option<MapPlane> indexed by map ID.
            let Some(plane) = plane else { continue; };
            let id = id as u32;
            world_geo_data_res.maps.insert(id, MapPlaneMetadata {
                id: id as u8,
                width: plane.size_blocks.width * 8,
                height: plane.size_blocks.height * 8,
            });
            crate::console_logger::one(
                crate::console_logger::LogSev::Info,
                crate::console_logger::LogAbout::RenderWorldLand,
                &format!("Populated WorldGeoData for map {id} ({}x{} tiles).", plane.size_blocks.width * 8, plane.size_blocks.height * 8),
            );
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PackedMapTexel {
    tile_id: u16,
    packed_meta: u16,
}

pub struct MapPlane {
    pub index: u32,
    pub size_blocks: MapSizeBlocks,
    package: Arc<UddpReader>,
    cached_block_indices: Vec<u32>,
    cached_blocks_arena: Vec<CachedBlock>,
    cached_blocks_free_list: Vec<u32>,
    cached_blocks_bitmask: Vec<u64>,
    /// Bumped for every block inserted, not by `evict_idle_blocks` (`LCMesh::last_blocks_loaded_version` says why).
    pub blocks_loaded_version: u64,
}

pub struct CachedBlock {
    pub block: MapBlock,
    pub last_accessed: Instant,
}

impl MapPlane {
    pub fn load(
        package_path: PathBuf,
        map_index: u32,
    ) -> eyre::Result<Self> {
        let package_path = package_path
            .canonicalize()
            .wrap_err_with(|| format!("Check map{map_index}.uddp path"))?;
        let package = Arc::new(
            UddpReader::load(&package_path)
                .map_err(|error| eyre::eyre!("{error}"))
                .wrap_err_with(|| format!("Open map{map_index}.uddp"))?,
        );

        let records = package.records();
        let package_metadata = read_map_package_metadata(&package, map_index, &records)?;
        let total_blocks = package_metadata.chunk_count;
        let map_size_tiles = validate_map_size(MapSizeCells {
            width: package_metadata.width_tiles,
            height: package_metadata.height_tiles,
        })?;
        let size_blocks = MapSizeBlocks {
            width: map_size_tiles.width / MapBlock::CELLS_PER_ROW,
            height: map_size_tiles.height / MapBlock::CELLS_PER_COLUMN,
        };
        let expected_chunks = size_blocks.width.div_ceil(PACKAGE_CHUNK_BLOCK_DIM)
            * size_blocks.height.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
        if total_blocks != expected_chunks {
            return Err(eyre::eyre!(
                "Malformed map package: expected {expected_chunks} chunks for map {map_index}, found {total_blocks}"
            ));
        }

        let native_block_count = size_blocks.width * size_blocks.height;
        let cached_block_indices = vec![u32::MAX; native_block_count as usize];
        let cached_blocks_bitmask = vec![0; ((native_block_count as usize) + 63) / 64];

        Ok(Self {
            index: map_index,
            size_blocks,
            package,
            cached_block_indices,
            cached_blocks_arena: Vec::new(),
            cached_blocks_free_list: Vec::new(),
            cached_blocks_bitmask,
            blocks_loaded_version: 0,
        })
    }

    pub fn size_cells(&self) -> MapSizeCells {
        MapSizeCells {
            width: self.size_blocks.width * MapBlock::CELLS_PER_ROW,
            height: self.size_blocks.height * MapBlock::CELLS_PER_COLUMN,
        }
    }

    pub fn package(&self) -> Arc<UddpReader> {
        self.package.clone()
    }

    /// Which of the land ids below `id_count` the map uses: the cells of every package chunk, read in parallel.
    pub fn used_land_ids(&self, id_count: usize) -> eyre::Result<Vec<bool>> {
        const TEXEL_BYTES: usize = std::mem::size_of::<PackedMapTexel>();
        let chunk_count = self.size_blocks.width.div_ceil(PACKAGE_CHUNK_BLOCK_DIM)
            * self.size_blocks.height.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
        let threads = std::thread::available_parallelism().map_or(1, |threads| threads.get()) as u32;
        let per_thread = chunk_count.div_ceil(threads);
        let package = &self.package;
        let parts = std::thread::scope(|scope| {
            let workers = (0..threads).map(|thread| scope.spawn(move || -> eyre::Result<Vec<bool>> {
                let mut used = vec![false; id_count];
                for chunk in thread * per_thread..((thread + 1) * per_thread).min(chunk_count) {
                    let bytes = package.read_file_by_dense_id_cow(chunk).map_err(|error| eyre::eyre!("{error}"))
                        .wrap_err_with(|| format!("read map chunk {chunk}"))?;
                    // A borrowed chunk may be unaligned: the texels are read from their bytes.
                    if bytes.len() % TEXEL_BYTES != 0 {
                        return Err(eyre::eyre!("invalid packed map chunk size {}", bytes.len()));
                    }
                    for texel in bytes.chunks_exact(TEXEL_BYTES) {
                        let texel: PackedMapTexel = bytemuck::pod_read_unaligned(texel);
                        if let Some(used) = used.get_mut(usize::from(texel.tile_id)) {
                            *used = true;
                        }
                    }
                }
                Ok(used)
            })).collect::<Vec<_>>();
            workers.into_iter().map(|worker| worker.join().expect("map scan thread")).collect::<Vec<_>>()
        });
        let mut used = vec![false; id_count];
        for part in parts {
            for (used, part) in used.iter_mut().zip(part?) {
                *used |= part;
            }
        }
        Ok(used)
    }

    pub fn block(&mut self, pos: MapBlockRelPos) -> Option<&MapBlock> {
        if pos.x >= self.size_blocks.width || pos.y >= self.size_blocks.height {
            return None;
        }
        let idx = (pos.x * self.size_blocks.height) + pos.y;
        let arena_idx = self.cached_block_indices[idx as usize];
        if arena_idx != u32::MAX {
            let cached = &mut self.cached_blocks_arena[arena_idx as usize];
            cached.last_accessed = Instant::now();
            Some(&cached.block)
        } else {
            None
        }
    }

    pub fn block_no_update(&self, pos: MapBlockRelPos) -> Option<&MapBlock> {
        if pos.x >= self.size_blocks.width || pos.y >= self.size_blocks.height {
            return None;
        }
        let idx = (pos.x * self.size_blocks.height) + pos.y;
        let arena_idx = self.cached_block_indices[idx as usize];
        if arena_idx != u32::MAX {
            Some(&self.cached_blocks_arena[arena_idx as usize].block)
        } else {
            None
        }
    }

    pub fn evict_idle_blocks(&mut self, timeout: Duration) -> usize {
        let now = Instant::now();
        let mut evicted = 0;

        for word_idx in 0..self.cached_blocks_bitmask.len() {
            let mut bits = self.cached_blocks_bitmask[word_idx];
            let mut cleared_bits = 0u64;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                let idx = (word_idx * 64 + bit) as u32;
                if idx >= self.size_blocks.width * self.size_blocks.height {
                    bits &= bits - 1;
                    continue;
                }

                let arena_idx = self.cached_block_indices[idx as usize];
                if arena_idx != u32::MAX {
                    let cached = &self.cached_blocks_arena[arena_idx as usize];
                    if now.duration_since(cached.last_accessed) > timeout {
                        self.cached_blocks_free_list.push(arena_idx);
                        self.cached_block_indices[idx as usize] = u32::MAX;
                        evicted += 1;
                        cleared_bits |= 1u64 << bit;
                    }
                }
                bits &= bits - 1;
            }
            if cleared_bits != 0 {
                self.cached_blocks_bitmask[word_idx] &= !cleared_bits;
            }
        }

        evicted
    }

    #[inline(always)]
    pub fn is_block_cached(&self, pos: &MapBlockRelPos) -> bool {
        let idx = (pos.x * self.size_blocks.height) + pos.y;
        let word_idx = (idx >> 6) as usize;
        if word_idx >= self.cached_blocks_bitmask.len() {
            return false;
        }
        let bit_idx = (idx & 63) as usize;
        (self.cached_blocks_bitmask[word_idx] & (1u64 << bit_idx)) != 0
    }

    pub fn insert_preloaded_blocks(&mut self, blocks: Vec<MapBlock>) {
        let now = Instant::now();
        let height = self.size_blocks.height;
        for block in blocks {
            let pos = &block.internal_coords;
            let idx = (pos.x * height) + pos.y;
            let arena_idx = self.cached_block_indices[idx as usize];
            if arena_idx == u32::MAX {
                let new_arena_idx = if let Some(free_idx) = self.cached_blocks_free_list.pop() {
                    self.cached_blocks_arena[free_idx as usize] = CachedBlock {
                        block,
                        last_accessed: now,
                    };
                    free_idx
                } else {
                    let next_idx = self.cached_blocks_arena.len() as u32;
                    self.cached_blocks_arena.push(CachedBlock {
                        block,
                        last_accessed: now,
                    });
                    next_idx
                };
                self.cached_block_indices[idx as usize] = new_arena_idx;

                let word_idx = (idx / 64) as usize;
                let bit_idx = (idx % 64) as usize;
                if word_idx < self.cached_blocks_bitmask.len() {
                    self.cached_blocks_bitmask[word_idx] |= 1u64 << bit_idx;
                }
                self.blocks_loaded_version += 1;
            }
        }
    }
}

pub fn load_blocks_from_package(
    package: &UddpReader,
    positions: &[MapBlockRelPos],
    size_blocks_height: u32,
) -> eyre::Result<Vec<MapBlock>> {
    let package_height_chunks = size_blocks_height.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
    // A package chunk holds PACKAGE_CHUNK_BLOCK_DIM x PACKAGE_CHUNK_BLOCK_DIM blocks: each is decompressed once.
    let mut chunks = std::collections::HashMap::new();
    let mut blocks = Vec::with_capacity(positions.len());
    for &pos in positions {
        let package_index = (pos.x / PACKAGE_CHUNK_BLOCK_DIM) * package_height_chunks
            + (pos.y / PACKAGE_CHUNK_BLOCK_DIM);
        let bytes = match chunks.entry(package_index) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => entry.insert(package
                .read_file_by_dense_id(package_index)
                .map_err(|error| eyre::eyre!("{error}"))
                .wrap_err_with(|| format!("read map chunk {package_index}"))?),
        };
        blocks.push(decode_map_block_from_chunk(bytes, pos)?);
    }
    Ok(blocks)
}

/// The side of a package chunk in cells; a chunk holds `PACKAGE_CHUNK_BLOCK_DIM` x `PACKAGE_CHUNK_BLOCK_DIM` blocks.
pub const PACKAGE_CHUNK_CELL_DIM: u32 = PACKAGE_CHUNK_TILE_DIM as u32;

/// The package chunk of the cell (`x`, `y`) on a map `width` x `height` cells, and the cell's index in it (row by row,
/// as `read_package_chunk_land_z` gives them); `None` off the map.
pub fn package_chunk_cell(width: u32, height: u32, x: i32, y: i32) -> Option<(u32, usize)> {
    let (x, y) = (u32::try_from(x).ok()?, u32::try_from(y).ok()?);
    if x >= width || y >= height {
        return None;
    }
    let chunk_columns = height.div_ceil(PACKAGE_CHUNK_CELL_DIM);
    let chunk = (x / PACKAGE_CHUNK_CELL_DIM) * chunk_columns + y / PACKAGE_CHUNK_CELL_DIM;
    Some((chunk, ((y % PACKAGE_CHUNK_CELL_DIM) * PACKAGE_CHUNK_CELL_DIM + x % PACKAGE_CHUNK_CELL_DIM) as usize))
}

/// The land z of every cell of the package chunk `chunk_index` (chunks run down each column of chunks first), row by
/// row: `PACKAGE_CHUNK_CELL_DIM` x `PACKAGE_CHUNK_CELL_DIM` values.
pub fn read_package_chunk_land_z(package: &UddpReader, chunk_index: u32) -> eyre::Result<Vec<i8>> {
    const TEXEL_BYTES: usize = std::mem::size_of::<PackedMapTexel>();
    let bytes = package.read_file_by_dense_id_cow(chunk_index).map_err(|error| eyre::eyre!("{error}"))
        .wrap_err_with(|| format!("read map chunk {chunk_index}"))?;
    if bytes.len() != PACKAGE_CHUNK_TEXEL_COUNT * TEXEL_BYTES {
        return Err(eyre::eyre!("invalid packed map chunk size {}", bytes.len()));
    }
    // A borrowed chunk may be unaligned: the texels are read from their bytes.
    Ok(bytes.chunks_exact(TEXEL_BYTES).map(|texel| {
        let texel: PackedMapTexel = bytemuck::pod_read_unaligned(texel);
        (texel.packed_meta as u8).wrapping_sub(128) as i8
    }).collect())
}

fn read_map_package_metadata(
    package: &UddpReader,
    map_index: u32,
    records: &[FileRecord],
) -> eyre::Result<MapPackageMetadata> {
    let record = records
        .last()
        .ok_or_else(|| eyre::eyre!("map package has no records"))?;
    if unpack_type(record.locator.meta32) != DataType::Metadata as u8 {
        return Err(eyre::eyre!("map package is missing its final metadata record"));
    }

    let FileKey::Id(id) = record.key else {
        return Err(eyre::eyre!("map package metadata record is not dense-id addressed"));
    };
    let expected_id = records.len().saturating_sub(1) as u32;
    if id != expected_id {
        return Err(eyre::eyre!(
            "map package metadata record id {id} does not match expected final id {expected_id}"
        ));
    }

    let bytes = package
        .read_file_by_dense_id(id)
        .map_err(|error| eyre::eyre!("{error}"))
        .wrap_err("read map package metadata")?;
    let metadata = decode_map_package_metadata(&bytes)
        .map_err(|error| eyre::eyre!("invalid map package metadata: {error}"))?;
    if metadata.map_id != map_index {
        return Err(eyre::eyre!(
            "map package metadata is for map {}, expected map {}",
            metadata.map_id,
            map_index
        ));
    }
    if metadata.chunk_count == 0 {
        return Err(eyre::eyre!("map package metadata has zero chunks"));
    }
    if metadata.chunk_count + 1 != records.len() as u32 {
        return Err(eyre::eyre!(
            "map package metadata declares {} chunks, but package has {} records",
            metadata.chunk_count,
            records.len()
        ));
    }

    Ok(metadata)
}

fn validate_map_size(size: MapSizeCells) -> eyre::Result<MapSizeCells> {
    if size.width % MapBlock::CELLS_PER_ROW != 0 || size.height % MapBlock::CELLS_PER_COLUMN != 0 {
        Err(eyre::eyre!("Invalid map package dimensions"))
    } else {
        Ok(size)
    }
}

fn decode_map_block_from_chunk(bytes: &[u8], block_pos: MapBlockRelPos) -> eyre::Result<MapBlock> {
    let texels: &[PackedMapTexel] = bytemuck::try_cast_slice(bytes)
        .map_err(|_| eyre::eyre!("invalid packed map chunk size {}", bytes.len()))?;
    if texels.len() != PACKAGE_CHUNK_TEXEL_COUNT {
        return Err(eyre::eyre!(
            "invalid packed map texel count {} for block {},{}",
            texels.len(),
            block_pos.x,
            block_pos.y
        ));
    }

    let mut cells = [MapCell::default(); MapBlock::CELLS_PER_BLOCK as usize];
    let chunk_local_block_x = (block_pos.x % PACKAGE_CHUNK_BLOCK_DIM) as usize;
    let chunk_local_block_y = (block_pos.y % PACKAGE_CHUNK_BLOCK_DIM) as usize;
    let base_x = chunk_local_block_x * 8;
    let base_y = chunk_local_block_y * 8;

    for local_y in 0..8usize {
        for local_x in 0..8usize {
            let chunk_index = (base_y + local_y) * PACKAGE_CHUNK_TILE_DIM + (base_x + local_x);
            let texel = texels[chunk_index];
            cells[(local_y * 8) + local_x] = MapCell {
                id: texel.tile_id,
                z: (texel.packed_meta as u8).wrapping_sub(128) as i8,
                _pad: 0,
            };
        }
    }

    Ok(MapBlock {
        internal_coords: block_pos,
        cells,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_container::{AddFileRequest, CompressionFlag, LookupMode, UddpBuilder};

    /// The tile id each cell of the test map carries: its own coordinates.
    fn tile_id(x: u32, y: u32) -> u16 {
        (x * 256 + y) as u16
    }

    #[test]
    fn blocks_load_from_shared_chunks_in_any_order() {
        const CHUNKS: u32 = 2;
        let size_blocks_height = CHUNKS * PACKAGE_CHUNK_BLOCK_DIM;
        let chunk_cells = PACKAGE_CHUNK_TILE_DIM as u32;
        let mut builder = UddpBuilder::new(LookupMode::DenseId);
        // Chunks run down each column of chunks first.
        let chunks = (0..CHUNKS).flat_map(|column| (0..CHUNKS).map(move |row| (column, row))).map(|(column, row)| {
            (0..PACKAGE_CHUNK_TEXEL_COUNT as u32).map(|index| PackedMapTexel {
                tile_id: tile_id(column * chunk_cells + index % chunk_cells, row * chunk_cells + index / chunk_cells),
                packed_meta: 128 + (index % 7) as u16,
            }).collect::<Vec<_>>()
        }).collect::<Vec<_>>();
        for texels in &chunks {
            builder.add_file(AddFileRequest {
                data_type: DataType::Metadata as u8, compression: CompressionFlag::ZstdNoDict,
                width: 0, height: 0, virtual_path: None, path_hash64: None, id: None, data: bytemuck::cast_slice(texels),
            }).unwrap();
        }
        let package = UddpReader::open(builder.build().unwrap()).unwrap();

        // Blocks of different chunks interleaved, one block twice.
        let positions = [(0, 0), (5, 1), (1, 0), (2, 7), (7, 7), (0, 0), (4, 4)]
            .map(|(x, y)| MapBlockRelPos { x, y });
        let blocks = load_blocks_from_package(&package, &positions, size_blocks_height).unwrap();
        assert_eq!(blocks.len(), positions.len());
        for (block, position) in blocks.iter().zip(positions) {
            assert_eq!((block.internal_coords.x, block.internal_coords.y), (position.x, position.y));
            for (index, cell) in block.cells.iter().enumerate() {
                let (x, y) = (position.x * 8 + index as u32 % 8, position.y * 8 + index as u32 / 8);
                assert_eq!(cell.id, tile_id(x, y), "block {},{} cell {index}", position.x, position.y);
            }
        }
    }
}
