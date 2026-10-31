//! Runtime reader for the terrain-layer sidecar binary produced by `udd-conv`.
//!
//! This module exposes only the deserialization path. The binary format is
//! defined and written by `udd-conv::terrain_meta`; here we read it at runtime
//! without pulling in any build-tool dependencies.
//!
//! Package layout (flat binary, **not** a UDDP):
//! - 4-byte magic `b"TMLY"`
//! - u32 version (must be 1)
//! - `[u32: span_count]` followed by `span_count` × [`TerrainMetaLayerSpan`] records
//! - `[u32: layer_count]` followed by `layer_count` × [`TerrainMetaLayerRef`] records

use color_eyre::eyre::{self, WrapErr};
use std::io::{Cursor, Read};
use std::path::Path;
use byteorder::{LittleEndian, ReadBytesExt};

use crate::common::ensure_manifest_version;

const TERRAIN_META_MAGIC: [u8; 4] = *b"TMLY";
const TERRAIN_META_VERSION: u32 = 1;

/// One layer record from a `TerrainDefinitionTexture`, keyed by
/// `TerrainMetaLayerSpan.terrain_tile_id`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TerrainMetaLayerRef {
    /// Source texture id; 0 when absent.
    pub texture_id: u32,
    /// Authored stretch/repetition value.
    pub texture_repetition: f32,
    /// 1 when the layer is a support layer by name heuristic.
    pub is_support: u8,
    /// 1 when the layer name has a support-like clue.
    pub has_name_clue: u8,
    /// 1 when the layer has a preferred primary repetition.
    pub has_pref_rep: u8,
}

/// Index into the flat `TerrainMetaLayerRef` table, one per terrain tile id.
#[derive(Clone, Copy, Debug, Default)]
pub struct TerrainMetaLayerSpan {
    pub terrain_tile_id: u32,
    pub start: u32,
    pub len: u16,
}

/// Runtime-loaded terrain-layer sidecar.
pub struct TerrainMetaPackage {
    spans: Vec<TerrainMetaLayerSpan>,
    layers: Vec<TerrainMetaLayerRef>,
}

impl TerrainMetaPackage {
    /// Load from the flat binary format written by `udd-conv::terrain_meta::write_terrain_meta`.
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let bytes = std::fs::read(path.as_ref())
            .wrap_err_with(|| format!("read {}", path.as_ref().display()))?;
        Self::from_bytes(&bytes)
    }

    /// Deserialise from a byte slice.
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut reader = Cursor::new(bytes);

        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if magic != TERRAIN_META_MAGIC {
            eyre::bail!(
                "terrain meta: unexpected magic {:?}, expected {:?}",
                magic,
                TERRAIN_META_MAGIC
            );
        }
        let version = reader.read_u32::<LittleEndian>()?;
        ensure_manifest_version("terrain meta", version, TERRAIN_META_VERSION)?;

        let span_count = reader.read_u32::<LittleEndian>()? as usize;
        let mut spans = Vec::with_capacity(span_count);
        for _ in 0..span_count {
            let terrain_tile_id = reader.read_u32::<LittleEndian>()?;
            let start = reader.read_u32::<LittleEndian>()?;
            let len = reader.read_u16::<LittleEndian>()?;
            let _pad = reader.read_u16::<LittleEndian>()?;
            spans.push(TerrainMetaLayerSpan {
                terrain_tile_id,
                start,
                len,
            });
        }

        let layer_count = reader.read_u32::<LittleEndian>()? as usize;
        let mut layers = Vec::with_capacity(layer_count);
        for _ in 0..layer_count {
            let texture_id = reader.read_u32::<LittleEndian>()?;
            let texture_repetition = reader.read_f32::<LittleEndian>()?;
            let is_support = reader.read_u8()?;
            let has_name_clue = reader.read_u8()?;
            let has_pref_rep = reader.read_u8()?;
            let _pad = reader.read_u8()?;
            let _source_kind = reader.read_u32::<LittleEndian>()?;
            let _sampler_index = reader.read_u32::<LittleEndian>()?;
            let _uv_set = reader.read_u32::<LittleEndian>()?;
            layers.push(TerrainMetaLayerRef {
                texture_id,
                texture_repetition,
                is_support,
                has_name_clue,
                has_pref_rep,
            });
        }

        Ok(Self { spans, layers })
    }

    /// Return all layer refs for a given terrain tile id, or an empty slice.
    pub fn layers_for_tile(&self, terrain_tile_id: u32) -> &[TerrainMetaLayerRef] {
        let Some(span) = self
            .spans
            .iter()
            .find(|span| span.terrain_tile_id == terrain_tile_id)
        else {
            return &self.layers[0..0];
        };
        let start = span.start as usize;
        let end = start.saturating_add(span.len as usize);
        if start > self.layers.len() || end > self.layers.len() {
            return &self.layers[0..0];
        }
        &self.layers[start..end]
    }
}
