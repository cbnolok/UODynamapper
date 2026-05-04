use bevy::prelude::*;
use color_eyre::eyre;
use parking_lot::Mutex;
use std::fs::File;
use std::path::Path;
use uocf::classic::statics::{PackedStaticTile, StaticsReader};

#[derive(Resource)]
pub struct StaticsStoreRes(pub Vec<Option<Mutex<LazyStaticsStore>>>);

pub struct LazyStaticsStore {
    reader: StaticsReader<File>,
    cached_blocks: Vec<Option<Box<[PackedStaticTile]>>>,
}

impl LazyStaticsStore {
    pub fn new(index_path: &Path, mul_path: &Path, width: u32, height: u32) -> eyre::Result<Self> {
        let reader = StaticsReader::new(index_path, mul_path, width, height)?;
        let num_blocks = (reader.block_width * reader.block_height) as usize;
        let cached_blocks = std::iter::repeat_with(|| None)
            .take(num_blocks)
            .collect();

        Ok(Self {
            reader,
            cached_blocks,
        })
    }

    fn block_index(&self, block_x: u32, block_y: u32) -> Option<usize> {
        if block_x >= self.reader.block_width || block_y >= self.reader.block_height {
            return None;
        }

        Some((block_x * self.reader.block_height + block_y) as usize)
    }

    pub fn block_tiles(&mut self, block_x: u32, block_y: u32) -> eyre::Result<&[PackedStaticTile]> {
        let Some(block_index) = self.block_index(block_x, block_y) else {
            return Ok(&[]);
        };

        if self.cached_blocks[block_index].is_none() {
            let packed_tiles = self
                .reader
                .read_block(block_x, block_y)?
                .into_iter()
                .map(|tile| PackedStaticTile {
                    graphic: tile.graphic,
                    xy_packed: (tile.x_offset & 0x07) | ((tile.y_offset & 0x07) << 3),
                    z: tile.z,
                    hue: tile.hue,
                })
                .collect::<Vec<_>>()
                .into_boxed_slice();
            self.cached_blocks[block_index] = Some(packed_tiles);
        }

        match self.cached_blocks[block_index].as_deref() {
            Some(tiles) => Ok(tiles),
            None => Ok(&[]),
        }
    }
}
