use bevy::ecs::resource::Resource;
use bytemuck::{Pod, Zeroable};
use color_eyre::eyre::{self, WrapErr};
use std::path::{Path, PathBuf};
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PackedMapTexel {
    tile_id: u16,
    packed_meta: u16,
}

pub struct MapPlane {
    pub index: u32,
    pub size_blocks: MapSizeBlocks,
    package_path: PathBuf,
    package: Arc<UddpReader>,
    cached_block_indices: Vec<u32>,
    cached_blocks_arena: Vec<CachedBlock>,
    cached_blocks_free_list: Vec<u32>,
    cached_blocks_bitmask: Vec<u64>,
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
        map_size_tiles_override: Option<MapSizeCells>,
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
        let total_blocks = package_metadata
            .map(|metadata| metadata.chunk_count)
            .unwrap_or(records.len() as u32);
        let map_size_tiles = match (package_metadata, map_size_tiles_override) {
            (Some(metadata), _) => validate_map_size_override(MapSizeCells {
                width: metadata.width_tiles,
                height: metadata.height_tiles,
            })?,
            (None, Some(size)) => validate_map_size_override(size)?,
            (None, None) => infer_map_size_tiles(map_index, total_blocks)?,
        };
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
            package_path,
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

    pub fn package_path(&self) -> &Path {
        &self.package_path
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
    let mut blocks = Vec::with_capacity(positions.len());
    for &pos in positions {
        let package_height_chunks = size_blocks_height.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
        let package_index = (pos.x / PACKAGE_CHUNK_BLOCK_DIM) * package_height_chunks
            + (pos.y / PACKAGE_CHUNK_BLOCK_DIM);
        let bytes = package
            .read_file_by_dense_id(package_index)
            .map_err(|error| eyre::eyre!("{error}"))
            .wrap_err_with(|| format!("read map chunk {package_index}"))?;
        blocks.push(decode_map_block_from_chunk(&bytes, pos)?);
    }
    Ok(blocks)
}

fn read_map_package_metadata(
    package: &UddpReader,
    map_index: u32,
    records: &[FileRecord],
) -> eyre::Result<Option<MapPackageMetadata>> {
    let Some(record) = records.last() else {
        return Ok(None);
    };
    if unpack_type(record.locator.meta32) != DataType::Metadata as u8 {
        return Ok(None);
    }

    let FileKey::Id(id) = record.key else {
        return Ok(None);
    };
    let expected_id = records.len().saturating_sub(1) as u32;
    if id != expected_id {
        return Ok(None);
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

    Ok(Some(metadata))
}

fn validate_map_size_override(size: MapSizeCells) -> eyre::Result<MapSizeCells> {
    if size.width % MapBlock::CELLS_PER_ROW != 0 || size.height % MapBlock::CELLS_PER_COLUMN != 0 {
        Err(eyre::eyre!("Invalid manual map size"))
    } else {
        Ok(size)
    }
}

fn infer_map_size_tiles(map_index: u32, total_chunks: u32) -> eyre::Result<MapSizeCells> {
    let candidates: &[(u32, u32)] = match map_index {
        0..=1 => &[(6144, 4096), (7168, 4096)],
        2 => &[(2304, 1600)],
        3 => &[(2560, 2048)],
        4 => &[(1448, 1448)],
        5 => &[(1280, 4096)],
        _ => return Err(eyre::eyre!("Invalid map number")),
    };

    let matches = candidates
        .iter()
        .filter_map(|&(width, height)| {
            let width_blocks = width / MapBlock::CELLS_PER_ROW;
            let height_blocks = height / MapBlock::CELLS_PER_COLUMN;
            let width_chunks = width_blocks.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
            let height_chunks = height_blocks.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
            (width_chunks * height_chunks == total_chunks).then_some(MapSizeCells { width, height })
        })
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [size] => Ok(*size),
        [] => Err(eyre::eyre!(
            "Unable to infer map {map_index} dimensions from {total_chunks} chunks"
        )),
        _ => Err(eyre::eyre!(
            "Ambiguous map {map_index} dimensions for {total_chunks} chunks; set an explicit map size"
        )),
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
