//! Runtime reader for the terrain-layer sidecar binary produced by `udd-conv`.
//!
//! This module exposes only the deserialization path. The binary format is
//! defined and written by `udd-conv::terrain_meta`; here we read it at runtime
//! without pulling in any build-tool dependencies.
//!
//! Package layout (flat binary, **not** a UDDP):
//! - 4-byte magic `b"TMLY"`
//! - u32 version (must be 2: per-layer class and role by position)
//! - `[u32: span_count]` followed by `span_count` × [`TerrainMetaLayerSpan`] records
//! - `[u32: layer_count]` followed by `layer_count` × [`TerrainMetaLayerRef`] records

use color_eyre::eyre::{self, WrapErr};
use std::io::{Cursor, Read};
use std::path::Path;
use byteorder::{LittleEndian, ReadBytesExt};

use crate::common::ensure_manifest_version;
use uocf::enhanced::terrain_definition::{TerrainLayerClass, TerrainLayerRole};

const TERRAIN_META_MAGIC: [u8; 4] = *b"TMLY";
const TERRAIN_META_VERSION: u32 = 2;
/// Stored bytes of a span (tile id, start, length, padding) and of a layer (texture id, repetition, class, role,
/// padding, source kind, sampler, UV set).
const SPAN_RECORD_BYTES: usize = 12;
const LAYER_RECORD_BYTES: usize = 24;

/// One layer record from a `TerrainDefinitionTexture`, keyed by
/// `TerrainMetaLayerSpan.terrain_tile_id`.
#[derive(Clone, Copy, Debug)]
pub struct TerrainMetaLayerRef {
    /// Source texture id; 0 when absent.
    pub texture_id: u32,
    /// Authored stretch/repetition value.
    pub texture_repetition: f32,
    /// The class of the entry's shader.
    pub layer_class: TerrainLayerClass,
    /// The role of the item position in that class.
    pub role: TerrainLayerRole,
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
        ensure_records_fit(&reader, span_count, SPAN_RECORD_BYTES, "span")?;
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
        ensure_records_fit(&reader, layer_count, LAYER_RECORD_BYTES, "layer")?;
        let mut layers = Vec::with_capacity(layer_count);
        for _ in 0..layer_count {
            let texture_id = reader.read_u32::<LittleEndian>()?;
            let texture_repetition = reader.read_f32::<LittleEndian>()?;
            let class_code = reader.read_u8()?;
            let role_code = reader.read_u8()?;
            let _pad = reader.read_u16::<LittleEndian>()?;
            let layer_class = TerrainLayerClass::from_u8(class_code)
                .ok_or_else(|| eyre::eyre!("terrain meta: unknown layer class {class_code}"))?;
            let role = TerrainLayerRole::from_u8(role_code)
                .ok_or_else(|| eyre::eyre!("terrain meta: unknown layer role {role_code}"))?;
            let _source_kind = reader.read_u32::<LittleEndian>()?;
            let _sampler_index = reader.read_u32::<LittleEndian>()?;
            let _uv_set = reader.read_u32::<LittleEndian>()?;
            layers.push(TerrainMetaLayerRef {
                texture_id,
                texture_repetition,
                layer_class,
                role,
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

/// Fails unless `count` records of `record_bytes` each fit in what is left of the reader, so that a damaged
/// count is an error rather than an allocation of up to 4 Gi records.
fn ensure_records_fit(reader: &Cursor<&[u8]>, count: usize, record_bytes: usize, what: &str) -> eyre::Result<()> {
    let left = reader.get_ref().len().saturating_sub(reader.position() as usize);
    let needed = count.checked_mul(record_bytes);
    eyre::ensure!(
        needed.is_some_and(|needed| needed <= left),
        "terrain meta: {count} {what} records need {record_bytes} bytes each, {left} bytes are left"
    );
    Ok(())
}
