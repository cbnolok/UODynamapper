//! Terrain-layer sidecar tables for `TerrainDefinition.uop`.
//!
//! Each `TerrainMetaLayerRef` captures one layer from a `TerrainDefinitionTexture`,
//! preserving the full layer graph with each item's class and role (by position, as the client binds them;
//! `uocf::enhanced::terrain_definition::TerrainLayerClass`) so later phases need not re-parse the original UOP.
//!
//! Package layout (flat binary, not a UDDP):
//! - `[u32: span_count]` followed by `span_count` × `TerrainMetaLayerSpan` records
//! - `[u32: layer_count]` followed by `layer_count` × `TerrainMetaLayerRef` records
//!
//! This is intentionally kept as a simple flat binary so it can be embedded into
//! any UDDP package as a metadata entry, or written standalone for inspection.

use bytemuck::{Pod, Zeroable};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use color_eyre::eyre::{self, WrapErr};
use std::io::{Cursor, Read};
use std::path::Path;
use uocf::enhanced::terrain_definition::TerrainDefinitionPackage;

// Magic and version constants.
const TERRAIN_META_MAGIC: [u8; 4] = *b"TMLY";
/// 2: per-layer class and role by position replace the name/stretch heuristic bytes.
const TERRAIN_META_VERSION: u32 = 2;

/// One layer record from a `TerrainDefinitionTexture`, keyed by
/// `TerrainMetaLayerSpan.terrain_tile_id`.
///
/// Fields map 1-to-1 to `TerrainDefinitionTextureLayer`. The `_pad` field
/// keeps the struct at a multiple of 4 bytes for `bytemuck::Pod`.
#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy, Debug, Default)]
pub struct TerrainMetaLayerRef {
    /// `layer.texture_id`, 0 when absent.
    pub texture_id: u32,
    /// `layer.texture_repetition`.
    pub texture_repetition: f32,
    /// `TerrainLayerClass as u8` of the entry's shader.
    pub layer_class: u8,
    /// `TerrainLayerRole as u8` of the item position.
    pub role: u8,
    pub _pad: [u8; 2],
    /// `layer.source_kind` (stored as u32 for alignment).
    pub source_kind: u32,
    /// `layer.sampler_index`.
    pub sampler_index: u32,
    /// `layer.uv_set`.
    pub uv_set: u32,
}

/// Index into the flat `TerrainMetaLayerRef` table, one per terrain tile id.
///
/// A single `TerrainDefinitionEntry` may cover multiple alias tile ids; each
/// alias id gets its own span pointing to the same underlying layer slice.
#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy, Debug, Default)]
pub struct TerrainMetaLayerSpan {
    pub terrain_tile_id: u32,
    pub start: u32,
    pub len: u16,
    pub _pad: u16,
}

/// Runtime-loaded terrain-layer sidecar.
pub struct TerrainMetaPackage {
    spans: Vec<TerrainMetaLayerSpan>,
    layers: Vec<TerrainMetaLayerRef>,
}

impl TerrainMetaPackage {
    /// Load from the flat binary format written by [`write_terrain_meta`].
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let bytes = std::fs::read(path.as_ref())
            .wrap_err_with(|| format!("read {}", path.as_ref().display()))?;
        Self::from_bytes(&bytes)
    }

    /// Deserialise from a byte slice (the same layout as written by [`write_terrain_meta`]).
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
        if version != TERRAIN_META_VERSION {
            eyre::bail!(
                "terrain meta: unsupported version {version}, expected {TERRAIN_META_VERSION}"
            );
        }

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
                _pad,
            });
        }

        let layer_count = reader.read_u32::<LittleEndian>()? as usize;
        let mut layers = Vec::with_capacity(layer_count);
        for _ in 0..layer_count {
            let texture_id = reader.read_u32::<LittleEndian>()?;
            let texture_repetition = reader.read_f32::<LittleEndian>()?;
            let layer_class = reader.read_u8()?;
            let role = reader.read_u8()?;
            let mut _pad = [0u8; 2];
            reader.read_exact(&mut _pad)?;
            let source_kind = reader.read_u32::<LittleEndian>()?;
            let sampler_index = reader.read_u32::<LittleEndian>()?;
            let uv_set = reader.read_u32::<LittleEndian>()?;
            layers.push(TerrainMetaLayerRef {
                texture_id,
                texture_repetition,
                layer_class,
                role,
                _pad,
                source_kind,
                sampler_index,
                uv_set,
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

    /// Iterate all (terrain_tile_id, layer_refs) pairs in span order.
    pub fn iter_tile_layers(&self) -> impl Iterator<Item = (u32, &[TerrainMetaLayerRef])> {
        self.spans.iter().map(|span| {
            let start = span.start as usize;
            let end = start.saturating_add(span.len as usize);
            let end = end.min(self.layers.len());
            (span.terrain_tile_id, &self.layers[start..end])
        })
    }
}

/// Build result from [`build_terrain_meta_tables`].
pub struct TerrainMetaTables {
    pub spans: Vec<TerrainMetaLayerSpan>,
    pub layers: Vec<TerrainMetaLayerRef>,
}

/// Build terrain-layer sidecar tables from a loaded `TerrainDefinitionPackage`.
///
/// Emits one span per terrain tile id (one `TerrainDefinitionEntry` may expand
/// to multiple tile ids via its runtime alias list). Each span points into a flat
/// layer table of `TerrainMetaLayerRef` records.
pub fn build_terrain_meta_tables(
    terrain_definition: &TerrainDefinitionPackage,
) -> eyre::Result<TerrainMetaTables> {
    let mut spans: Vec<TerrainMetaLayerSpan> = Vec::new();
    let mut layers: Vec<TerrainMetaLayerRef> = Vec::new();

    for entry in &terrain_definition.entries {
        let Some(texture) = entry.texture.as_ref() else {
            // Entry has no texture block: emit one span per tile id with len=0.
            for tile_id in entry.runtime_slot_ids() {
                let start = u32::try_from(layers.len())
                    .map_err(|_| eyre::eyre!("terrain meta layer table exceeds u32 address space"))?;
                spans.push(TerrainMetaLayerSpan {
                    terrain_tile_id: tile_id,
                    start,
                    len: 0,
                    _pad: 0,
                });
            }
            continue;
        };

        // Collect layer records for this entry's texture block.
        let entry_layer_start = layers.len();
        let layer_class = texture.layer_class();
        for (position, layer) in texture.layers.iter().enumerate() {
            let layer_ref = TerrainMetaLayerRef {
                texture_id: layer.texture_id.unwrap_or(0),
                texture_repetition: layer.texture_repetition,
                layer_class: layer_class as u8,
                role: layer_class.role(position) as u8,
                _pad: [0; 2],
                source_kind: layer.source_kind as u32,
                sampler_index: layer.sampler_index,
                uv_set: layer.uv_set,
            };
            layers.push(layer_ref);
        }
        let entry_layer_end = layers.len();
        let entry_layer_len = u16::try_from(entry_layer_end - entry_layer_start).map_err(|_| {
            eyre::eyre!(
                "terrain entry {} has more than u16::MAX layers",
                entry.id
            )
        })?;

        // Emit one span per runtime tile id (alias or self).
        for tile_id in entry.runtime_slot_ids() {
            let start = u32::try_from(entry_layer_start)
                .map_err(|_| eyre::eyre!("terrain meta layer table exceeds u32 address space"))?;
            spans.push(TerrainMetaLayerSpan {
                terrain_tile_id: tile_id,
                start,
                len: entry_layer_len,
                _pad: 0,
            });
        }
    }

    Ok(TerrainMetaTables { spans, layers })
}

/// Serialise a `TerrainMetaTables` to its flat binary format.
pub fn serialize_terrain_meta(tables: &TerrainMetaTables) -> eyre::Result<Vec<u8>> {
    let span_bytes = std::mem::size_of::<TerrainMetaLayerSpan>();
    let layer_bytes = std::mem::size_of::<TerrainMetaLayerRef>();
    let capacity = 12
        + tables.spans.len() * span_bytes
        + 4
        + tables.layers.len() * layer_bytes;
    let mut bytes: Vec<u8> = Vec::with_capacity(capacity);

    bytes.extend_from_slice(&TERRAIN_META_MAGIC);
    bytes.write_u32::<LittleEndian>(TERRAIN_META_VERSION)?;
    bytes.write_u32::<LittleEndian>(tables.spans.len() as u32)?;
    for span in &tables.spans {
        bytes.write_u32::<LittleEndian>(span.terrain_tile_id)?;
        bytes.write_u32::<LittleEndian>(span.start)?;
        bytes.write_u16::<LittleEndian>(span.len)?;
        bytes.write_u16::<LittleEndian>(span._pad)?;
    }
    bytes.write_u32::<LittleEndian>(tables.layers.len() as u32)?;
    for layer in &tables.layers {
        bytes.write_u32::<LittleEndian>(layer.texture_id)?;
        bytes.write_f32::<LittleEndian>(layer.texture_repetition)?;
        bytes.write_u8(layer.layer_class)?;
        bytes.write_u8(layer.role)?;
        bytes.extend_from_slice(&layer._pad);
        bytes.write_u32::<LittleEndian>(layer.source_kind)?;
        bytes.write_u32::<LittleEndian>(layer.sampler_index)?;
        bytes.write_u32::<LittleEndian>(layer.uv_set)?;
    }

    Ok(bytes)
}

/// Build and write a terrain-layer sidecar binary from a `TerrainDefinitionPackage`.
pub fn write_terrain_meta(
    terrain_definition: &TerrainDefinitionPackage,
    out_file: &Path,
) -> eyre::Result<()> {
    let tables = build_terrain_meta_tables(terrain_definition)?;
    let bytes = serialize_terrain_meta(&tables)?;
    std::fs::write(out_file, &bytes)
        .wrap_err_with(|| format!("write {}", out_file.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::terrain_definition::{
        TerrainDefinitionEntry, TerrainDefinitionPackage, TerrainDefinitionTexture,
        TerrainDefinitionTextureLayer, TerrainDefinitionTileAlias, TerrainLayerClass, TerrainLayerRole,
    };

    fn make_package() -> TerrainDefinitionPackage {
        TerrainDefinitionPackage {
            entries: vec![
                TerrainDefinitionEntry {
                    id: 10,
                    name_index: 0,
                    aliases: vec![TerrainDefinitionTileAlias {
                        subtype: 0,
                        legacy_land_id: 100,
                        tile_flags: 0,
                    }],
                    texture: Some(TerrainDefinitionTexture {
                        layers: vec![
                            TerrainDefinitionTextureLayer {
                                texture_id: Some(2000001),
                                texture_repetition: 4.0,
                                source_kind: 0,
                                sampler_index: 0,
                                uv_set: 0,
                                ..Default::default()
                            },
                            TerrainDefinitionTextureLayer {
                                texture_id: Some(2000002),
                                texture_repetition: 1.0,
                                path: Some("noise_layer.dds".to_string()),
                                source_kind: 0,
                                sampler_index: 1,
                                uv_set: 0,
                                ..Default::default()
                            },
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                TerrainDefinitionEntry {
                    id: 20,
                    name_index: 0,
                    aliases: vec![],
                    texture: None,
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn terrain_meta_roundtrips_layer_graph() {
        let package = make_package();
        let tables = build_terrain_meta_tables(&package).expect("build tables");
        let bytes = serialize_terrain_meta(&tables).expect("serialize");
        let loaded = TerrainMetaPackage::from_bytes(&bytes).expect("load");

        // Entry 10's alias is tile_id 100
        let layers = loaded.layers_for_tile(100);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].texture_id, 2000001);
        assert_eq!(layers[0].texture_repetition, 4.0);
        assert_eq!(layers[0].layer_class, TerrainLayerClass::Default as u8);
        assert_eq!(layers[0].role, TerrainLayerRole::ColourA as u8);
        assert_eq!(layers[1].texture_id, 2000002);
        assert_eq!(layers[1].role, TerrainLayerRole::ColourB as u8);
        assert_eq!(layers[1].sampler_index, 1);

        // Entry 20 has no texture block — layers_for_tile returns empty.
        let layers_20 = loaded.layers_for_tile(20);
        assert_eq!(layers_20.len(), 0);
    }

    #[test]
    fn terrain_meta_magic_validation() {
        let package = make_package();
        let tables = build_terrain_meta_tables(&package).expect("build tables");
        let mut bytes = serialize_terrain_meta(&tables).expect("serialize");
        bytes[0] = 0xFF; // corrupt magic
        let result = TerrainMetaPackage::from_bytes(&bytes);
        assert!(result.is_err());
    }
}
