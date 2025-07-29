#![allow(unused_parens)]
use super::{diagnostics::*, TILE_NUM_PER_CHUNK_1D, TILE_NUM_PER_CHUNK_TOTAL};
use bevy::prelude::*;
use std::collections::VecDeque;

/// Pool to minimize dynamic allocations for chunk mesh vertex buffers.
/// Allocates [at most] `capacity` buffers up front, then dynamically allocates "spillover" as needed.
/// Buffers obtained from pool should be returned immediately after use.
/// Buffers that weren't pre-pooled are simply dropped instead of being recycled.
pub struct MeshBuffers {
    pub positions: Vec<[f32; 3]>, // Each vertex position (x, y, z)
    pub normals: Vec<[f32; 3]>,   // Per-vertex surface normal (affects lighting)
    pub uvs: Vec<[f32; 2]>,       // Per-vertex texture coordinate
    pub indices: Vec<u32>,        // Indices composing triangles from positions
    pool_alloc: bool,             // true if from pool, false if dynamically allocated
}

#[derive(Resource)]
pub struct LandChunkMeshBufferPool {
    pool: VecDeque<MeshBuffers>, // The available pooled buffers (up to fixed size)
    used: usize,                 // Count of currently checked out buffers
    allocs: usize,               // Running total of allocations (for diagnostics)
    high_water: usize,           // Max number of concurrent checked-out at once (diagnostics)
    #[allow(unused)]
    capacity: usize, // Pool capacity
}

impl LandChunkMeshBufferPool {
    /// Initialize the pool with a given fixed capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let buffer_template = || MeshBuffers {
            positions: vec![[0.0; 3]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
            normals: vec![[0.0; 3]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
            uvs: vec![[0.0; 2]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
            indices: vec![0u32; (TILE_NUM_PER_CHUNK_TOTAL * 6)],
            pool_alloc: true,
        };
        let mut pool = VecDeque::with_capacity(capacity);
        for _ in 0..capacity {
            pool.push_back(buffer_template());
        }
        Self {
            pool,
            used: 0,
            allocs: 0,
            high_water: 0,
            capacity,
        }
    }
    /// Allocate a mesh buffer, using the pool if not empty, otherwise dynamically.
    pub fn alloc(&mut self, diag: &mut LandChunkMeshDiagnostics) -> MeshBuffers {
        let buffers = self.pool.pop_front().unwrap_or_else(|| {
            // Dynamic: rare unless view frustum is huge or a bug causes leaks
            MeshBuffers {
                positions: vec![[0.0; 3]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
                normals: vec![[0.0; 3]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
                uvs: vec![[0.0; 2]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
                indices: vec![0u32; (TILE_NUM_PER_CHUNK_TOTAL * 6)],
                pool_alloc: false,
            }
        });
        self.used += 1;
        self.allocs += 1;
        self.high_water = self.high_water.max(self.used);
        diag.mesh_allocs = self.allocs;
        diag.alloc_high_water = self.high_water;
        buffers
    }
    /// Return a mesh buffer to the pool if compatible, otherwise drop it (let Rust reclaim).
    pub fn free(&mut self, buffers: MeshBuffers, diag: &mut LandChunkMeshDiagnostics) {
        if buffers.pool_alloc {
            self.pool.push_back(buffers);
        }
        if self.used > 0 {
            self.used -= 1;
        }
        diag.pool_in_positions = self.pool.len();
        diag.pool_in_normals = self.pool.len();
        diag.pool_in_uvs = self.pool.len();
        diag.pool_in_indices = self.pool.len();
    }
}
