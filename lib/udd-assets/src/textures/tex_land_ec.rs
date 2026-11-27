use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;
#[cfg(test)]
use std::cell::Cell;
#[cfg(all(debug_assertions, not(test)))]
use std::sync::atomic::{AtomicU64, Ordering};
use color_eyre::eyre::{self, WrapErr};
use byteorder::{LittleEndian, ReadBytesExt};
use udd_container::{xxh64_virtual_path, UddpReader, UddpReaderOptions};
use crate::common::{
    AtlasCacheOptions, AtlasPageCache, decode_atlas_page_rgba, ensure_manifest_version, read_ec_overlay_table, read_path_entry,
    read_path_entry_cow, EcEquivalentRecord, EcOverlayTable,
};
use crate::ec_terrain_overrides::EcTerrainOverrides;
use crate::tex_art_cc::{AtlasPackingMode, PagePixelFormat};
use uocf::enhanced::terrain_definition::{TerrainLayerClass, TerrainLayerRole};

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"ELPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"ELSL";
/// The slot table's magic when slots lie on base pages ([`SLOT_FLAG_BASE_PAGE`], an overlay): a reader that does not
/// know the flag refuses the table, rather than drawing those slots from pages the overlay does not hold.
pub const OVERLAY_SLOT_MANIFEST_MAGIC: [u8; 4] = *b"ELSO";
const TERRAIN_PROVENANCE_MAGIC: [u8; 4] = *b"ELTP";
const GRUNGE_MAGIC: [u8; 4] = *b"ELGR";
/// The version of the page and slot manifests, which stands for the package's metadata as a whole.
pub const TEX_LAND_EC_METADATA_VERSION: u32 = 1;
/// The version of the terrain provenance manifest.
pub const TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION: u32 = 1;
/// The version of the grunge entry's header ([`UDDP_GRUNGE_ENTRY_VPATH`]).
pub const TEX_LAND_EC_GRUNGE_VERSION: u32 = 1;

pub const UDDP_PAGE_MANIFEST_ENTRY_VPATH: &str = "metadata/pages.bin";
pub const UDDP_SLOT_MANIFEST_ENTRY_VPATH: &str = "metadata/slots.bin";
pub const UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH: &str = "metadata/terrain_provenance.bin";
pub const UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH: &str = "metadata/terrain_overrides.json";
pub const UDDP_TRANSCODE_ENTRY_VPATH: &str = "metadata/transcode.bin";
pub const UDDP_LEGACY_TERRAIN_MAP_ENTRY_VPATH: &str = "metadata/legacy_terrain_map.csv";
pub const UDDP_SOURCE_MATERIALS_ENTRY_VPATH: &str = "metadata/source_materials.json";
/// Optional: the client's grunge texture. Only the Kingdom Reborn land ([`TexLandEcPackage::kr_land`]) carries it, KR's
/// `build/worldart/01000045.dds`. A header (magic `ELGR`, u32 [`TEX_LAND_EC_GRUNGE_VERSION`], u32 width, u32 height,
/// u8 [`PagePixelFormat`], always RGBA8888), then the RGBA8888 rows: [`encode_grunge_entry`], [`decode_grunge_entry`].
pub const UDDP_GRUNGE_ENTRY_VPATH: &str = "textures/grunge.rgba8888";
/// Bytes per pixel of the grunge entry.
const GRUNGE_BYTES_PER_PIXEL: usize = 4;
/// Bytes of the grunge entry's header: magic, version, width, height and pixel format.
const GRUNGE_HEADER_SIZE: usize = 17;
/// `schema` and `schema_version` of the terrain overrides JSON entry.
pub const TERRAIN_OVERRIDES_SCHEMA: &str = "tex_land_ec_terrain_overrides";
pub const TERRAIN_OVERRIDES_SCHEMA_VERSION: u32 = 1;

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const SLOT_FLAG_LAND: u16 = 1 << 1;
pub const SLOT_FLAG_STATIC: u16 = 1 << 2;
/// Only in an overlay ([`TexLandEcPackage::is_overlay`]): the slot's page, position and extent are those of the base
/// package's slot of the same texture, whose page `page_index` is in the base package. [`TexLandEcPackage::kr_land`]
/// appends the base pages to the overlay's own and clears the flag.
pub const SLOT_FLAG_BASE_PAGE: u16 = 1 << 3;
/// Magic of the overlay table of this layout ([`TexLandEcPackage::is_overlay`]), whose ids are land texture ids.
pub const TEX_LAND_EC_EQUIVALENTS_MAGIC: [u8; 4] = *b"ELEQ";
pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;
pub const MISSING_TEXTURE_ID: u32 = u32::MAX;
pub const MISSING_SLOT_ID: u32 = u32::MAX;
pub const MISSING_TERRAIN_LAYER_INDEX: u32 = u32::MAX;

/// The provenance byte of a selected texture that is not a source item (an override or a missing layer).
pub const TERRAIN_LAYER_ROLE_NONE: u8 = u8::MAX;

pub const TERRAIN_OVERRIDE_ACTION_LAYER: u16 = 1 << 0;
pub const TERRAIN_OVERRIDE_ACTION_TEXTURE: u16 = 1 << 1;
pub const TERRAIN_OVERRIDE_ACTION_IGNORE: u16 = 1 << 2;

#[cfg(all(debug_assertions, not(test)))]
static FROM_UDDP_PACKAGE_WITH_OPTIONS_CALLS: AtomicU64 = AtomicU64::new(0);
#[cfg(all(debug_assertions, not(test)))]
static SET_TERRAIN_OVERRIDES_CALLS: AtomicU64 = AtomicU64::new(0);

// Under `cfg(test)` the counters are per-thread: the test harness runs sibling tests
// that load packages concurrently, and shared counters would make absolute-count
// assertions depend on scheduling. Diagnostic (non-test) builds keep the process-wide
// atomics, which are read across threads by the application.
#[cfg(test)]
thread_local! {
    static FROM_UDDP_PACKAGE_WITH_OPTIONS_CALLS: Cell<u64> = const { Cell::new(0) };
    static SET_TERRAIN_OVERRIDES_CALLS: Cell<u64> = const { Cell::new(0) };
}

#[cfg(all(debug_assertions, not(test)))]
fn note_from_uddp_package_with_options_call() {
    FROM_UDDP_PACKAGE_WITH_OPTIONS_CALLS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
fn note_from_uddp_package_with_options_call() {
    FROM_UDDP_PACKAGE_WITH_OPTIONS_CALLS.with(|calls| calls.set(calls.get() + 1));
}

#[cfg(all(debug_assertions, not(test)))]
fn note_set_terrain_overrides_call() {
    SET_TERRAIN_OVERRIDES_CALLS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
fn note_set_terrain_overrides_call() {
    SET_TERRAIN_OVERRIDES_CALLS.with(|calls| calls.set(calls.get() + 1));
}

#[cfg(all(debug_assertions, not(test)))]
pub fn debug_from_uddp_package_with_options_call_count() -> u64 {
    FROM_UDDP_PACKAGE_WITH_OPTIONS_CALLS.load(Ordering::Relaxed)
}

#[cfg(test)]
pub fn debug_from_uddp_package_with_options_call_count() -> u64 {
    FROM_UDDP_PACKAGE_WITH_OPTIONS_CALLS.with(Cell::get)
}

#[cfg(all(debug_assertions, not(test)))]
pub fn debug_set_terrain_overrides_call_count() -> u64 {
    SET_TERRAIN_OVERRIDES_CALLS.load(Ordering::Relaxed)
}

#[cfg(test)]
pub fn debug_set_terrain_overrides_call_count() -> u64 {
    SET_TERRAIN_OVERRIDES_CALLS.with(Cell::get)
}

#[cfg(test)]
pub fn debug_reset_package_load_counters() {
    FROM_UDDP_PACKAGE_WITH_OPTIONS_CALLS.with(|calls| calls.set(0));
    SET_TERRAIN_OVERRIDES_CALLS.with(|calls| calls.set(0));
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TexLandEcTerrainOverrideActions {
    pub material_id: u32,
    pub action_count: u32,
    pub action_flags: u16,
}

impl TexLandEcTerrainOverrideActions {
    pub fn has_layer(self) -> bool {
        (self.action_flags & TERRAIN_OVERRIDE_ACTION_LAYER) != 0
    }

    pub fn has_texture(self) -> bool {
        (self.action_flags & TERRAIN_OVERRIDE_ACTION_TEXTURE) != 0
    }

    pub fn has_ignore(self) -> bool {
        (self.action_flags & TERRAIN_OVERRIDE_ACTION_IGNORE) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TexLandEcRuntimeSlotSource {
    TranscodedMaterialAlias,
    TranscodedMaterialPlaceholder,
    DirectAlias,
    DirectSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexLandEcMaterialDecision {
    pub query_tile_id: u32,
    pub material_id: Option<u32>,
    pub runtime_slot_id: Option<u32>,
    pub runtime_slot_source: Option<TexLandEcRuntimeSlotSource>,
    pub provenance_record_count: u32,
    pub primary_texture_id: Option<u32>,
    pub primary_layer_index: Option<u32>,
    /// The material's layer class (from its shader name), when a provenance record names it.
    pub layer_class: Option<TerrainLayerClass>,
    pub override_actions: Option<TexLandEcTerrainOverrideActions>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TexLandEcTerrainOverrideTextureRef {
    pub material_id: u32,
    pub role: String,
    pub texture_id: u32,
    pub layer_index: Option<u32>,
    pub texture_repetition: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TexLandEcTerrainOverrideDetails {
    pub material_id: u32,
    pub ignore_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TexLandEcResolvedOverrideTexture {
    pub material_id: u32,
    pub role: String,
    pub texture_id: u32,
    pub runtime_slot_id: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexLandEcResolvedMaterialLayer {
    pub material_id: u32,
    pub layer_index: u32,
    pub texture_id: u32,
    pub runtime_slot_id: Option<u32>,
    pub texture_repetition: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexLandEcEffectiveOverrideSlotChange {
    pub material_id: u32,
    pub query_tile_id: u32,
    pub runtime_slot_id: Option<u32>,
    pub effective_runtime_slot_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TexLandEcTerrainOverrideLoadSummary {
    pub entry_count: u32,
    pub action_count: u32,
    pub texture_ref_count: u32,
    pub resolved_texture_ref_count: u32,
    pub unresolved_texture_refs: Vec<TexLandEcTerrainOverrideTextureRef>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TexLandEcTerrainProvenanceRecord {
    pub material_id: u32,
    pub material_name_id: i32,
    pub alias_count_index: u32,
    pub alias_slot_id: u32,
    pub alias_tile_flags: u64,
    pub selected_texture_id: u32,
    pub canonical_slot_id: u32,
    pub selected_layer_index: u32,
    pub selected_texture_repetition: f32,
    /// The source item the client draws alone: item 0 for default and bump, item 1 for water.
    pub primary_texture_id: u32,
    pub primary_layer_index: u32,
    /// The material's layer class, from its shader name.
    pub layer_class: TerrainLayerClass,
    /// The role of `selected_layer_index` in `layer_class`; `None` for an override or missing layer.
    pub selected_layer_role: Option<TerrainLayerRole>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexLandEcPageRecord {
    pub page_index: u32,
    pub tile_count: u32,
    pub used_width: u32,
    pub used_height: u32,
    pub pixel_format: PagePixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexLandEcSlotRecord {
    pub art_id: u32,
    pub page_index: u32,
    pub page_tile_index: u16,
    pub flags: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl TexLandEcSlotRecord {
    pub fn absent(art_id: u32) -> Self {
        Self {
            art_id,
            page_index: MISSING_PAGE_INDEX,
            page_tile_index: MISSING_PAGE_TILE_INDEX,
            flags: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }

    pub fn is_present(self) -> bool {
        (self.flags & SLOT_FLAG_PRESENT) != 0
    }

    pub fn is_land(self) -> bool {
        (self.flags & SLOT_FLAG_LAND) != 0
    }

    pub fn is_static(self) -> bool {
        (self.flags & SLOT_FLAG_STATIC) != 0
    }

    /// Whether the slot lies on a page of the base package ([`SLOT_FLAG_BASE_PAGE`]).
    pub fn is_base_page(self) -> bool {
        (self.flags & SLOT_FLAG_BASE_PAGE) != 0
    }
}

/// The overlay entry of this layout, with its path.
pub fn tex_land_ec_overlay_entry(table: &EcOverlayTable) -> (&'static str, Vec<u8>) {
    table.encode(TEX_LAND_EC_EQUIVALENTS_MAGIC, TEX_LAND_EC_METADATA_VERSION)
}

/// The base package's pages an overlay's slots lie on, appended after the overlay's own pages
/// ([`TexLandEcPackage::kr_land`]).
struct BasePages {
    package: UddpReader,
    /// The base package's page index of each appended page, in order.
    pages: Vec<u32>,
}

pub struct TexLandEcPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    packing_mode: AtlasPackingMode,
    pages: Vec<TexLandEcPageRecord>,
    slots: Vec<TexLandEcSlotRecord>,
    terrain_provenance: Vec<TexLandEcTerrainProvenanceRecord>,
    terrain_override_actions: HashMap<u32, TexLandEcTerrainOverrideActions>,
    terrain_override_details: HashMap<u32, TexLandEcTerrainOverrideDetails>,
    terrain_override_texture_refs: HashMap<u32, Vec<TexLandEcTerrainOverrideTextureRef>>,
    source_terrain_map: Option<uocf::enhanced::legacy_terrain_map::LegacyTerrainMap>,
    source_materials: HashMap<u32, uocf::enhanced::terrain_definition::TerrainDefinitionEntry>,
    pub transcode: HashMap<u32, u32>,
    page_cache: AtlasPageCache,
    /// `metadata/ec_equivalents.bin` of an overlay.
    overlay: Option<EcOverlayTable>,
    /// The pages the package holds; `pages` continues with the base package's ones of `base_pages`.
    own_page_count: u32,
    base_pages: Option<BasePages>,
}

impl TexLandEcPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        Self::load_with_options(path, AtlasCacheOptions::disabled())
    }

    pub fn load_with_options(path: impl AsRef<Path>, options: AtlasCacheOptions) -> eyre::Result<Self> {
        let package = UddpReader::load(path.as_ref())
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package_with_options(package, options)
    }

    pub fn load_in_memory(path: impl AsRef<Path>) -> eyre::Result<Self> {
        Self::load_in_memory_with_options(path, AtlasCacheOptions::disabled())
    }

    pub fn load_in_memory_with_options(
        path: impl AsRef<Path>,
        options: AtlasCacheOptions,
    ) -> eyre::Result<Self> {
        let package = UddpReader::load_in_memory(path.as_ref())
            .wrap_err_with(|| format!("load_in_memory {}", path.as_ref().display()))?;
        Self::from_uddp_package_with_options(package, options)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        Self::from_uddp_package_with_options(package, AtlasCacheOptions::disabled())
    }

    pub fn from_uddp_package_with_options(
        package: UddpReader,
        options: AtlasCacheOptions,
    ) -> eyre::Result<Self> {
        #[cfg(any(test, debug_assertions))]
        note_from_uddp_package_with_options_call();

        let page_manifest = read_path_entry_cow(&package, UDDP_PAGE_MANIFEST_ENTRY_VPATH)
            .context("tex_land_ec.uddp missing metadata/pages.bin")?;
        let slot_manifest = read_path_entry_cow(&package, UDDP_SLOT_MANIFEST_ENTRY_VPATH)
            .context("tex_land_ec.uddp missing metadata/slots.bin")?;
        let terrain_provenance_manifest =
            read_path_entry_cow(&package, UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH)
                .context("tex_land_ec.uddp missing metadata/terrain_provenance.bin")?;

        let (page_width, page_height, page_gutter, page_packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let (slot_width, slot_height, slot_gutter, slot_packing_mode, base_page_layout, slots) =
            parse_slot_manifest(&slot_manifest)?;
        let terrain_provenance = parse_terrain_provenance_manifest(&terrain_provenance_manifest)?;

        if (page_width, page_height, page_gutter) != (slot_width, slot_height, slot_gutter) {
            eyre::bail!("tex_land_ec metadata headers disagree on atlas dimensions or gutter");
        }
        if page_packing_mode != slot_packing_mode {
            eyre::bail!("tex_land_ec metadata headers disagree on atlas packing mode");
        }

        let source_terrain_map = if package.find_by_path_hash(xxh64_virtual_path(UDDP_LEGACY_TERRAIN_MAP_ENTRY_VPATH)).is_some() {
            let bytes = read_path_entry_cow(&package, UDDP_LEGACY_TERRAIN_MAP_ENTRY_VPATH)?;
            Some(uocf::enhanced::legacy_terrain_map::LegacyTerrainMap::from_bytes(&bytes)?)
        } else {
            None
        };
        let source_materials_bytes = read_path_entry_cow(&package, UDDP_SOURCE_MATERIALS_ENTRY_VPATH)
            .context("tex_land_ec.uddp missing metadata/source_materials.json; repack with the current udd-cli")?;
        let source_materials: Vec<uocf::enhanced::terrain_definition::TerrainDefinitionEntry> =
            serde_json::from_slice(&source_materials_bytes)
                .wrap_err_with(|| format!("parse {UDDP_SOURCE_MATERIALS_ENTRY_VPATH}"))?;
        let source_materials = source_materials.into_iter().map(|entry| (entry.id, entry)).collect();
        let mut transcode: HashMap<u32, u32> = source_terrain_map.as_ref()
            .into_iter().flat_map(|map| map.routes.values())
            .map(|route| (u32::from(route.legacy_id), route.material_id)).collect();
        // The embedded KDL remains a policy overlay, never the source census.
        transcode.extend(read_transcode_from_package(&package)?);
        let (
            terrain_override_actions,
            terrain_override_details,
            terrain_override_texture_refs,
        ) = read_terrain_overrides_metadata_maps_from_package(&package)?;
        let overlay = read_ec_overlay_table(&package, TEX_LAND_EC_EQUIVALENTS_MAGIC, TEX_LAND_EC_METADATA_VERSION)?;
        let has_base_page_slots = slots.iter().any(|slot| slot.is_base_page());
        if has_base_page_slots != base_page_layout {
            eyre::bail!(
                "the tex_land_ec slot table's magic does not match its slots (slots on base pages: {has_base_page_slots}); \
                 repack with the current udd-cli"
            );
        }
        if overlay.is_none() && has_base_page_slots {
            eyre::bail!("tex_land_ec slots lie on base pages, but the package is no overlay; repack with the current udd-cli");
        }
        let own_page_count = pages.len() as u32;
        Ok(Self {
            package,
            atlas_width: page_width,
            atlas_height: page_height,
            gutter: page_gutter,
            packing_mode: page_packing_mode,
            pages,
            slots,
            terrain_provenance,
            terrain_override_actions,
            terrain_override_details,
            terrain_override_texture_refs,
            source_terrain_map,
            source_materials,
            transcode,
            page_cache: AtlasPageCache::new(options),
            overlay,
            own_page_count,
            base_pages: None,
        })
    }

    /// Lays a candidate routing table over the embedded one, for the audits that try a table before it is packed.
    pub fn set_transcode(&mut self, transcode: HashMap<u32, u32>) {
        self.transcode.extend(transcode);
    }

    pub fn source_terrain_map(&self) -> Option<&uocf::enhanced::legacy_terrain_map::LegacyTerrainMap> {
        self.source_terrain_map.as_ref()
    }

    pub fn source_material(&self, material_id: u32) -> Option<&uocf::enhanced::terrain_definition::TerrainDefinitionEntry> {
        self.source_materials.get(&material_id)
    }

    /// Find a land lookup payload for an item sharing an original liquid graph.
    /// IDs are source texture identities, not item graphics interpreted as land.
    pub fn source_liquid_payload_for_item(
        &self,
        material: &uocf::enhanced::tileart::ArtMaterial,
        refs: &[crate::tilemeta::TileMetaItemTextureRef],
    ) -> Option<u32> {
        use crate::tilemeta::EcMaterialPhysicalPackage;
        use crate::tilemeta::EcMaterialStableRole;
        if material.block_index != 0 || material.shader_name.as_deref() != Some(uocf::enhanced::tileart::WATER_SHADER_NAME) {
            return None;
        }
        // Water maps 0, 1 and 2 by item position (the client binds item i to shader map i).
        let [Some(bump), Some(fallback), Some(surface)] = [
            EcMaterialStableRole::WaterBump, EcMaterialStableRole::Base, EcMaterialStableRole::WaterReflection,
        ].map(|role| refs.iter()
            .find(|r| r.block_index == 0 && r.is_world_art() && r.stable_role() == role
                && r.physical_package() == EcMaterialPhysicalPackage::Texture)) else { return None; };
        let active = material.technique.first().zip(material.tweaks.get(..2))
            .and_then(|(technique, scroll)| self.source_liquid_payload_for_active_sources(
                [(bump.texture_id, bump.texture_stretch), (surface.texture_id, surface.texture_stretch)],
                *technique, [scroll[0], scroll[1]],
            ));
        // Existing water items have local zero Tweaks but share all three source
        // identities. Preserve that established base-sharing path separately.
        active.or_else(|| self.source_liquid_payload([bump.texture_id, fallback.texture_id, surface.texture_id]))
    }

    fn source_liquid_payload(&self, textures: [u32; 3]) -> Option<u32> {
        self.source_materials.values().filter(|entry| {
            entry.texture.as_ref().is_some_and(|texture| {
                texture.layer_class() == TerrainLayerClass::Water
                    && texture.layers.len() >= 3
                    && texture.layers.iter().take(3).map(|layer| layer.texture_id)
                        .eq(textures.into_iter().map(Some))
            })
        }).flat_map(|entry| self.transcode.iter()
            .filter(move |(tile, material)| **tile < 16384 && **material == entry.id)
            .map(|(tile, _)| *tile))
            .min()
    }

    /// Match the normal-capability path when an item has its own fallback mask.
    /// Source 1 is not sampled by that path; sources 0/2 and their active
    /// parameters must agree before the item can share a terrain liquid lookup.
    fn source_liquid_payload_for_active_sources(
        &self,
        sources: [(u32, f32); 2],
        technique: i32,
        scroll: [f32; 2],
    ) -> Option<u32> {
        if sources.iter().any(|(_, repetition)| !repetition.is_finite() || *repetition <= 0.0)
            || scroll.iter().any(|value| !value.is_finite()) {
            return None;
        }
        self.source_materials.values().filter(|entry| {
            entry.texture.as_ref().is_some_and(|texture| {
                texture.layer_class() == TerrainLayerClass::Water
                    && texture.techniques.first().copied() == Some(technique)
                    && texture.tweaks.get(..2) == Some(scroll.as_slice())
                    && [0, 2].into_iter().zip(sources).all(|(index, (id, repetition))| {
                        texture.layers.get(index).is_some_and(|layer|
                            layer.texture_id == Some(id) && layer.texture_repetition == repetition)
                    })
            })
        }).flat_map(|entry| self.transcode.iter()
            .filter(move |(tile, material)| **tile < 16384 && **material == entry.id)
            .map(|(tile, _)| *tile))
            .min()
    }

    /// Replaces the embedded terrain overrides with a candidate table, for the audits and examples that try a table
    /// before it is packed. The runtime reads only the embedded overrides.
    pub fn set_terrain_overrides_from_kdl(
        &mut self,
        path: impl AsRef<Path>,
    ) -> eyre::Result<TexLandEcTerrainOverrideLoadSummary> {
        let overrides = EcTerrainOverrides::load(path)?;
        Ok(self.set_terrain_overrides(&overrides))
    }

    pub fn set_terrain_overrides(
        &mut self,
        overrides: &EcTerrainOverrides,
    ) -> TexLandEcTerrainOverrideLoadSummary {
        #[cfg(any(test, debug_assertions))]
        note_set_terrain_overrides_call();

        let (actions, details, texture_refs) = terrain_override_maps_from_overrides(overrides);

        self.terrain_override_actions = actions;
        self.terrain_override_details = details;
        self.terrain_override_texture_refs = texture_refs;

        self.terrain_override_load_summary()
    }

    pub fn terrain_override_load_summary(&self) -> TexLandEcTerrainOverrideLoadSummary {
        let mut unresolved_texture_refs = Vec::new();
        let mut texture_ref_count = 0u32;
        let mut resolved_texture_ref_count = 0u32;
        let mut resolved_texture_slots = HashMap::<(u32, u32), bool>::new();

        for refs in self.terrain_override_texture_refs.values() {
            for texture_ref in refs {
                texture_ref_count += 1;
                let is_resolved = match resolved_texture_slots
                    .entry((texture_ref.material_id, texture_ref.texture_id))
                {
                    std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let is_resolved = self
                            .resolve_texture_id_slot(texture_ref.material_id, texture_ref.texture_id)
                            .is_some();
                        entry.insert(is_resolved);
                        is_resolved
                    }
                };
                if is_resolved {
                    resolved_texture_ref_count += 1;
                } else {
                    unresolved_texture_refs.push(texture_ref.clone());
                }
            }
        }

        TexLandEcTerrainOverrideLoadSummary {
            entry_count: self.terrain_override_actions.len() as u32,
            action_count: self
                .terrain_override_actions
                .values()
                .map(|actions| actions.action_count)
                .sum(),
            texture_ref_count,
            resolved_texture_ref_count,
            unresolved_texture_refs,
        }
    }

    pub fn package(&self) -> &UddpReader {
        &self.package
    }

    /// The Kingdom Reborn land this package holds ([`crate::common::KR_SECTION_ENTRY_PATH`]), in this layout, read in
    /// place with `reader_options` and `options`; `None` when it holds none. It is an overlay over this package: its
    /// slots of the textures it leaves to this package lie on this package's pages, which it reads as its own
    /// ([`Self::over_base`]), so a lookup through it is one slot read, as in any package.
    pub fn kr_land(&self, reader_options: UddpReaderOptions, options: AtlasCacheOptions) -> eyre::Result<Option<Self>> {
        let Some(section) = crate::common::open_kr_section(&self.package, reader_options)? else {
            return Ok(None);
        };
        let mut land = Self::from_uddp_package_with_options(section, options).wrap_err("the Kingdom Reborn land")?;
        eyre::ensure!(land.is_overlay(), "the Kingdom Reborn land is no overlay; repack with the current udd-cli");
        land.over_base(self).wrap_err("the Kingdom Reborn land")?;
        Ok(Some(land))
    }

    /// Whether this is an overlay: a package that holds only the land textures its client draws differently from the
    /// client of the base package holding it, and lists the others ([`Self::equivalents`]), whose slots
    /// ([`SLOT_FLAG_BASE_PAGE`]) lie on the base package's pages.
    pub fn is_overlay(&self) -> bool {
        self.overlay.is_some()
    }

    /// The land texture ids an overlay leaves to its base package, sorted; empty for a full package.
    pub fn equivalents(&self) -> &[EcEquivalentRecord] {
        self.overlay.as_ref().map_or(&[], |table| &table.equivalents)
    }

    /// The pages the package holds itself; [`Self::pages`] continues with the base pages an overlay reads.
    pub fn own_page_count(&self) -> u32 {
        self.own_page_count
    }

    /// The base package's index of page `page_index`, for a base page an overlay reads ([`Self::kr_land`]); `None`
    /// for one of its own pages.
    pub fn base_page_index(&self, page_index: u32) -> Option<u32> {
        let appended = page_index.checked_sub(self.own_page_count)?;
        self.base_pages.as_ref()?.pages.get(appended as usize).copied()
    }

    /// The slot each packed land texture id was given, from the terrain provenance: the canonical slot of every
    /// texture a material item or override selects.
    pub fn texture_slot_ids(&self) -> HashMap<u32, u32> {
        self.terrain_provenance
            .iter()
            .filter(|record| record.selected_texture_id != MISSING_TEXTURE_ID && record.canonical_slot_id != MISSING_SLOT_ID)
            .map(|record| (record.selected_texture_id, record.canonical_slot_id))
            .collect()
    }

    /// Reads this overlay's base-page slots through `base`: the base pages they lie on are appended after the
    /// package's own, in ascending base order, and the slots point at them, so the runtime lookups stay one slot
    /// read. The pages must share the atlas extent, gutter, packing mode and pixel format.
    fn over_base(&mut self, base: &Self) -> eyre::Result<()> {
        if (self.atlas_width, self.atlas_height, self.gutter, self.packing_mode)
            != (base.atlas_width, base.atlas_height, base.gutter, base.packing_mode)
        {
            eyre::bail!(
                "its atlas ({}x{}, gutter {}, {:?}) is not the base package's ({}x{}, gutter {}, {:?}); repack with the \
                 current udd-cli",
                self.atlas_width, self.atlas_height, self.gutter, self.packing_mode,
                base.atlas_width, base.atlas_height, base.gutter, base.packing_mode
            );
        }
        let base_page_indexes = self
            .slots
            .iter()
            .filter(|slot| slot.is_base_page())
            .map(|slot| slot.page_index)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let own_format = self.pages.first().map(|page| page.pixel_format);
        let mut appended = HashMap::with_capacity(base_page_indexes.len());
        for &base_page_index in &base_page_indexes {
            let page = *base
                .pages
                .get(base_page_index as usize)
                .ok_or_else(|| eyre::eyre!("a slot lies on base page {base_page_index}, which the base package lacks"))?;
            if let Some(own_format) = own_format.filter(|&format| format != page.pixel_format) {
                eyre::bail!(
                    "its pages are {own_format:?}, the base package's {:?}; repack with the current udd-cli",
                    page.pixel_format
                );
            }
            let page_index = self.pages.len() as u32;
            appended.insert(base_page_index, page_index);
            self.pages.push(TexLandEcPageRecord { page_index, ..page });
        }
        for slot in self.slots.iter_mut().filter(|slot| slot.is_base_page()) {
            slot.page_index = appended[&slot.page_index];
            slot.flags &= !SLOT_FLAG_BASE_PAGE;
        }
        self.base_pages = Some(BasePages { package: base.package.clone(), pages: base_page_indexes });
        Ok(())
    }

    /// The client grunge texture ([`UDDP_GRUNGE_ENTRY_VPATH`]), or `None` when the package has none. An entry without
    /// the current header, or whose pixels do not match it, is an error ([`decode_grunge_entry`]).
    pub fn grunge_rgba(&self) -> eyre::Result<Option<GrungeImage>> {
        let hash = xxh64_virtual_path(UDDP_GRUNGE_ENTRY_VPATH);
        if self.package.find_by_path_hash(hash).is_none() {
            return Ok(None);
        }
        let bytes = self.package.read_file_by_path_hash(hash)
            .map_err(|error| eyre::eyre!("read {UDDP_GRUNGE_ENTRY_VPATH}: {error:?}"))?;
        decode_grunge_entry(&bytes).map(Some)
    }

    pub fn atlas_width(&self) -> u32 {
        self.atlas_width
    }

    pub fn atlas_height(&self) -> u32 {
        self.atlas_height
    }

    pub fn gutter(&self) -> u16 {
        self.gutter
    }

    pub fn packing_mode(&self) -> AtlasPackingMode {
        self.packing_mode
    }

    pub fn pages(&self) -> &[TexLandEcPageRecord] {
        &self.pages
    }

    pub fn slots(&self) -> &[TexLandEcSlotRecord] {
        &self.slots
    }

    pub fn terrain_provenance(&self) -> &[TexLandEcTerrainProvenanceRecord] {
        &self.terrain_provenance
    }

    pub fn terrain_override_actions(&self) -> &HashMap<u32, TexLandEcTerrainOverrideActions> {
        &self.terrain_override_actions
    }

    pub fn terrain_override_actions_for(
        &self,
        material_id: u32,
    ) -> Option<&TexLandEcTerrainOverrideActions> {
        self.terrain_override_actions.get(&material_id)
    }

    pub fn terrain_override_details(&self) -> &HashMap<u32, TexLandEcTerrainOverrideDetails> {
        &self.terrain_override_details
    }

    pub fn terrain_override_details_for(
        &self,
        material_id: u32,
    ) -> Option<&TexLandEcTerrainOverrideDetails> {
        self.terrain_override_details.get(&material_id)
    }

    pub fn terrain_override_texture_refs_for(
        &self,
        material_id: u32,
    ) -> &[TexLandEcTerrainOverrideTextureRef] {
        self.terrain_override_texture_refs
            .get(&material_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn resolve_override_texture_slots(
        &self,
        material_id: u32,
    ) -> Vec<TexLandEcResolvedOverrideTexture> {
        self.terrain_override_texture_refs_for(material_id)
            .iter()
            .map(|texture_ref| TexLandEcResolvedOverrideTexture {
                material_id,
                role: texture_ref.role.clone(),
                texture_id: texture_ref.texture_id,
                runtime_slot_id: self.resolve_texture_id_slot(material_id, texture_ref.texture_id),
            })
            .collect()
    }

    pub fn resolve_material_layer_slot(
        &self,
        cc_tile_id: u32,
        layer_index: u32,
    ) -> Option<TexLandEcResolvedMaterialLayer> {
        let decision = self.resolve_material_decision(cc_tile_id);
        let material_id = decision.material_id?;
        self.resolve_material_layer_slot_by_material(material_id, cc_tile_id, layer_index)
    }

    /// Shader roles `[fallback/base, detail, mask, normal, animated liquid or environment map]`,
    /// distinct from source layer indices. Water materials bind by position; the bump class
    /// binds its luminance bump (item 3) as the normal and its environment map (item 4) last.
    pub fn resolve_material_render_layers(
        &self,
        cc_tile_id: u32,
    ) -> [Option<TexLandEcResolvedMaterialLayer>; 5] {
        let decision = self.resolve_material_decision(cc_tile_id);
        let source_texture = decision.material_id.and_then(|id| self.source_material(id))
            .and_then(|entry| entry.texture.as_ref());
        let source_liquid = source_texture
            .filter(|texture| texture.layer_class() == TerrainLayerClass::Water);
        std::array::from_fn(|index| {
            if let Some(source) = source_liquid {
                let material_id = decision.material_id?;
                // Water items by position: 0 distortion (du/dv), 1 fallback colour, 2 main colour.
                let role = match index {
                    0 => TerrainLayerRole::FallbackColour,
                    3 => TerrainLayerRole::Distortion,
                    4 => TerrainLayerRole::MainColour,
                    _ => return None,
                };
                let source_index = TerrainLayerClass::Water.position_of(role)?;
                if let Some(layer) = self.resolve_override_material_layer_slot_by_material(material_id, index as u32) {
                    return Some(layer);
                }
                let layer = source.layers.get(source_index)?;
                let texture_id = layer.texture_id?;
                return Some(TexLandEcResolvedMaterialLayer {
                    material_id, layer_index: source_index as u32, texture_id,
                    runtime_slot_id: self.resolve_texture_id_slot(material_id, texture_id),
                    texture_repetition: layer.texture_repetition,
                });
            }
            if index == 4 {
                // The bump class's item 4, the environment map its stage 5 adds (`TerrainLayerRole::EnvironmentMap`);
                // no other class draws a fifth item.
                let source = source_texture.filter(|texture| texture.layer_class() == TerrainLayerClass::Bump)?;
                let material_id = decision.material_id?;
                let source_index = TerrainLayerClass::Bump.position_of(TerrainLayerRole::EnvironmentMap)?;
                let layer = source.layers.get(source_index)?;
                let texture_id = layer.texture_id?;
                return Some(TexLandEcResolvedMaterialLayer {
                    material_id, layer_index: source_index as u32, texture_id,
                    runtime_slot_id: self.resolve_texture_id_slot(material_id, texture_id),
                    texture_repetition: layer.texture_repetition,
                });
            }
            // A fourth default-terrain source can be ordinary artwork (for
            // example swamp/blue-floor tiles), not a tangent normal map.
            // Only the authored bump family or an explicit normal override
            // establishes that role. Keep raw source-layer access separate.
            if index == 3 && !source_texture.is_some_and(|texture|
                texture.layer_class() == TerrainLayerClass::Bump) {
                return self.resolve_override_material_layer_slot_by_material(decision.material_id?, 3);
            }
            self.resolve_material_layer_slot(cc_tile_id, index as u32)
        })
    }

    fn resolve_material_layer_slot_by_material(
        &self,
        material_id: u32,
        cc_tile_id: u32,
        layer_index: u32,
    ) -> Option<TexLandEcResolvedMaterialLayer> {
        if let Some(layer) =
            self.resolve_override_material_layer_slot_by_material(material_id, layer_index)
        {
            return Some(layer);
        }

        self.terrain_provenance
            .iter()
            .filter(|record| {
                record.material_id == material_id
                    && record.selected_layer_index == layer_index
                    && record.selected_texture_id != MISSING_TEXTURE_ID
            })
            .min_by_key(|record| {
                (
                    record.alias_slot_id != cc_tile_id,
                    record.alias_slot_id == 0 || record.alias_slot_id == MISSING_SLOT_ID,
                    record.alias_count_index,
                    record.alias_slot_id,
                )
            })
            .map(|record| TexLandEcResolvedMaterialLayer {
                material_id,
                layer_index,
                texture_id: record.selected_texture_id,
                runtime_slot_id: self.resolve_provenance_record_slot(record),
                texture_repetition: record.selected_texture_repetition,
            })
    }

    fn resolve_override_material_layer_slot_by_material(
        &self,
        material_id: u32,
        layer_index: u32,
    ) -> Option<TexLandEcResolvedMaterialLayer> {
        let texture_ref = self
            .terrain_override_texture_refs_for(material_id)
            .iter()
            .find(|texture_ref| texture_ref.layer_index == Some(layer_index))
            .or_else(|| {
                self.terrain_override_texture_refs_for(material_id)
                    .iter()
                    .find(|texture_ref| terrain_override_role_matches_layer(&texture_ref.role, layer_index))
            })?;
        let runtime_slot_id = self.resolve_texture_id_slot(material_id, texture_ref.texture_id);
        let texture_repetition = texture_ref.texture_repetition.unwrap_or_else(|| {
            self.resolve_texture_id_provenance_record(material_id, texture_ref.texture_id)
                .map(|record| record.selected_texture_repetition)
                .unwrap_or(1.0)
        });

        Some(TexLandEcResolvedMaterialLayer {
            material_id,
            layer_index,
            texture_id: texture_ref.texture_id,
            runtime_slot_id,
            texture_repetition,
        })
    }

    /// The embedded terrain overrides JSON (`EcTerrainOverrides.kdl` as packed), which every loaded package carries.
    pub fn read_terrain_overrides_metadata(&self) -> eyre::Result<Vec<u8>> {
        read_path_entry(&self.package, UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH)
    }

    pub fn atlas_cache_enabled(&self) -> bool {
        self.page_cache.is_enabled()
    }

    pub fn clear_atlas_cache(&self) {
        self.page_cache.clear();
    }

    pub fn slot_record(&self, art_id: u32) -> Option<&TexLandEcSlotRecord> {
        self.slots.get(art_id as usize)
    }

    pub fn present_slot(&self, art_id: u32) -> Option<&TexLandEcSlotRecord> {
        self.slot_record(art_id).filter(|slot| slot.is_present())
    }

    pub fn resolve_runtime_slot_id(&self, cc_tile_id: u32) -> Option<u32> {
        if let Some(&material_id) = self.transcode.get(&cc_tile_id) {
            let material_records = self
                .terrain_provenance
                .iter()
                .filter(|r| r.material_id == material_id)
                .collect::<Vec<_>>();

            if let Some(slot_id) = self.resolve_material_runtime_slot(&material_records).0 {
                return Some(slot_id);
            }
        }

        let material_records = self
            .terrain_provenance
            .iter()
            .filter(|r| r.alias_slot_id == cc_tile_id)
            .collect::<Vec<_>>();
        if let Some(slot_id) = self.resolve_alias_runtime_slot(&material_records) {
            return Some(slot_id);
        }

        if self.present_slot(cc_tile_id).is_some() {
            return Some(cc_tile_id);
        }

        None
    }

    pub fn resolve_material_decision(&self, cc_tile_id: u32) -> TexLandEcMaterialDecision {
        let mut material_id = self.transcode.get(&cc_tile_id).copied();
        let mut material_records = if let Some(material_id) = material_id {
            self.terrain_provenance
                .iter()
                .filter(|record| record.material_id == material_id)
                .collect::<Vec<_>>()
        } else {
            let records = self
                .terrain_provenance
                .iter()
                .filter(|record| record.alias_slot_id == cc_tile_id)
                .collect::<Vec<_>>();
            if let Some(record) = records.first() {
                material_id = Some(record.material_id);
            }
            records
        };

        let (runtime_slot_id, runtime_slot_source) = if self.transcode.contains_key(&cc_tile_id) {
            self.resolve_material_runtime_slot(&material_records)
        } else if let Some(slot_id) = self.resolve_alias_runtime_slot(&material_records) {
            (Some(slot_id), Some(TexLandEcRuntimeSlotSource::DirectAlias))
        } else if self.present_slot(cc_tile_id).is_some() {
            (Some(cc_tile_id), Some(TexLandEcRuntimeSlotSource::DirectSlot))
        } else {
            (None, None)
        };

        material_records.sort_by_key(|record| {
            (
                optional_texture_sort_key(record.primary_texture_id, MISSING_TEXTURE_ID),
                optional_texture_sort_key(record.selected_texture_id, MISSING_TEXTURE_ID),
                record.alias_count_index,
            )
        });
        let primary_record = material_records
            .iter()
            .find(|record| {
                record.primary_texture_id != MISSING_TEXTURE_ID
                    || record.primary_layer_index != MISSING_TERRAIN_LAYER_INDEX
            })
            .copied();

        TexLandEcMaterialDecision {
            query_tile_id: cc_tile_id,
            material_id,
            runtime_slot_id,
            runtime_slot_source,
            provenance_record_count: material_records.len() as u32,
            primary_texture_id: primary_record
                .and_then(|record| optional_texture_id(record.primary_texture_id)),
            primary_layer_index: primary_record
                .and_then(|record| optional_terrain_layer_index(record.primary_layer_index)),
            layer_class: primary_record.map(|record| record.layer_class),
            override_actions: material_id.and_then(|id| {
                self.terrain_override_actions.get(&id).copied()
            }),
        }
    }

    pub fn resolve_effective_runtime_slot_id(&self, cc_tile_id: u32) -> Option<u32> {
        let decision = self.resolve_material_decision(cc_tile_id);
        if let Some(material_id) = decision.material_id {
            if let Some(slot_id) = self
                .resolve_override_texture_slots(material_id)
                .into_iter()
                .find(|texture| {
                    matches!(texture.role.as_str(), "base" | "t0")
                })
                .and_then(|texture| texture.runtime_slot_id)
            {
                return Some(slot_id);
            }
        }

        decision.runtime_slot_id
    }

    pub fn effective_override_slot_changes(&self) -> Vec<TexLandEcEffectiveOverrideSlotChange> {
        let mut changes = Vec::new();
        let mut material_ids = self
            .terrain_override_actions
            .keys()
            .copied()
            .collect::<Vec<_>>();
        material_ids.sort_unstable();
        material_ids.dedup();

        for material_id in material_ids {
            let Some(query_tile_id) = self.material_query_tile_id(material_id) else {
                continue;
            };
            let decision = self.resolve_material_decision(query_tile_id);
            let effective_runtime_slot_id = self.resolve_effective_runtime_slot_id(query_tile_id);
            if decision.runtime_slot_id != effective_runtime_slot_id {
                changes.push(TexLandEcEffectiveOverrideSlotChange {
                    material_id,
                    query_tile_id,
                    runtime_slot_id: decision.runtime_slot_id,
                    effective_runtime_slot_id,
                });
            }
        }

        changes
    }

    fn resolve_provenance_record_slot(
        &self,
        record: &TexLandEcTerrainProvenanceRecord,
    ) -> Option<u32> {
        if record.canonical_slot_id != 0 && record.canonical_slot_id != MISSING_SLOT_ID {
            if self.present_slot(record.canonical_slot_id).is_some() {
                return Some(record.canonical_slot_id);
            }
        }

        if record.alias_slot_id != 0 && record.alias_slot_id != MISSING_SLOT_ID {
            if self.present_slot(record.alias_slot_id).is_some() {
                return Some(record.alias_slot_id);
            }
        }

        None
    }

    fn resolve_texture_id_slot(&self, material_id: u32, texture_id: u32) -> Option<u32> {
        self.resolve_texture_id_provenance_record(material_id, texture_id)
            .and_then(|record| self.resolve_provenance_record_slot(record))
    }

    fn resolve_texture_id_provenance_record(
        &self,
        material_id: u32,
        texture_id: u32,
    ) -> Option<&TexLandEcTerrainProvenanceRecord> {
        self.terrain_provenance
            .iter()
            .filter(|record| record.selected_texture_id == texture_id)
            .min_by_key(|record| {
                (
                    record.material_id != material_id,
                    self.resolve_provenance_record_slot(record).is_none(),
                    record.alias_count_index,
                    record.alias_slot_id,
                )
            })
    }

    fn resolve_material_runtime_slot(
        &self,
        material_records: &[&TexLandEcTerrainProvenanceRecord],
    ) -> (Option<u32>, Option<TexLandEcRuntimeSlotSource>) {
        if let Some(slot_id) = self.resolve_primary_runtime_slot(material_records) {
            return (
                Some(slot_id),
                Some(TexLandEcRuntimeSlotSource::TranscodedMaterialAlias),
            );
        }

        if let Some(slot_id) = material_records
            .iter()
            .filter(|record| {
                record.alias_slot_id != 0
                    && record.alias_slot_id != MISSING_SLOT_ID
                    && self.resolve_provenance_record_slot(record).is_some()
            })
            .min_by_key(|record| record.alias_count_index)
            .and_then(|record| self.resolve_provenance_record_slot(record))
        {
            return (
                Some(slot_id),
                Some(TexLandEcRuntimeSlotSource::TranscodedMaterialAlias),
            );
        }

        if let Some(slot_id) = material_records
            .iter()
            .filter(|record| {
                record.alias_slot_id == 0
                    && self.resolve_provenance_record_slot(record).is_some()
            })
            .min_by_key(|record| record.alias_count_index)
            .and_then(|record| self.resolve_provenance_record_slot(record))
        {
            return (
                Some(slot_id),
                Some(TexLandEcRuntimeSlotSource::TranscodedMaterialPlaceholder),
            );
        }

        (None, None)
    }

    fn resolve_alias_runtime_slot(
        &self,
        material_records: &[&TexLandEcTerrainProvenanceRecord],
    ) -> Option<u32> {
        self.resolve_primary_runtime_slot(material_records).or_else(|| {
            material_records
                .iter()
                .find_map(|record| self.resolve_provenance_record_slot(record))
        })
    }

    fn resolve_primary_runtime_slot(
        &self,
        material_records: &[&TexLandEcTerrainProvenanceRecord],
    ) -> Option<u32> {
        material_records
            .iter()
            .filter(|record| {
                record.primary_texture_id != MISSING_TEXTURE_ID
                    && record.selected_texture_id == record.primary_texture_id
                    && self.resolve_provenance_record_slot(record).is_some()
            })
            .min_by_key(|record| {
                (
                    record.alias_slot_id == 0 || record.alias_slot_id == MISSING_SLOT_ID,
                    record.alias_count_index,
                    record.alias_slot_id,
                )
            })
            .and_then(|record| self.resolve_provenance_record_slot(record))
    }

    fn material_query_tile_id(&self, material_id: u32) -> Option<u32> {
        self.terrain_provenance
            .iter()
            .filter(|record| record.material_id == material_id)
            .filter_map(|record| {
                (record.alias_slot_id != 0 && record.alias_slot_id != MISSING_SLOT_ID)
                    .then_some(record.alias_slot_id)
            })
            .min()
            .or(Some(material_id))
    }

    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .get(page_index as usize)
            .map(|p| p.pixel_format)
            .unwrap_or(PagePixelFormat::Rgba8888);
        self.page_cache.read_page_bytes(page_index, || {
            // An overlay's pages past its own are the base package's ([`Self::over_base`]).
            let (package, entry_page_index) = match (page_index < self.own_page_count, &self.base_pages) {
                (true, _) => (&self.package, page_index),
                (false, Some(base)) => (
                    &base.package,
                    self.base_page_index(page_index).ok_or_else(|| eyre::eyre!("missing atlas page {page_index}"))?,
                ),
                (false, None) => (&self.package, page_index),
            };
            read_path_entry(package, &crate::tex_art_cc::page_entry_path(entry_page_index, fmt))
                .wrap_err_with(|| format!("unpack atlas page {page_index}"))
        })
    }

    pub fn read_page_rgba(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let page = self
            .pages
            .get(page_index as usize)
            .ok_or_else(|| eyre::eyre!("missing atlas page metadata for {page_index}"))?;
        let page_bytes = self.read_page_bytes(page_index)?;
        decode_atlas_page_rgba(
            &page_bytes,
            page.pixel_format,
            self.atlas_width,
            self.atlas_height,
            page.used_width,
            page.used_height,
        )
    }
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<TexLandEcPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("tex_land_ec page manifest", version, TEX_LAND_EC_METADATA_VERSION)?;
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid tex_land_ec page pixel format"))?;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid tex_land_ec atlas packing mode"))?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(count);
    for _ in 0..count {
        let page_index = cursor.read_u32::<LittleEndian>()?;
        let tile_count = cursor.read_u32::<LittleEndian>()?;
        let used_width = cursor.read_u32::<LittleEndian>()?;
        let used_height = cursor.read_u32::<LittleEndian>()?;
        pages.push(TexLandEcPageRecord {
            page_index,
            tile_count,
            used_width,
            used_height,
            pixel_format,
        });
    }
    Ok((w, h, g, packing_mode, pages))
}

/// The atlas header, whether the table has the base-page layout ([`OVERLAY_SLOT_MANIFEST_MAGIC`]) and the slots.
fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, bool, Vec<TexLandEcSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    let base_page_layout = match magic {
        SLOT_MANIFEST_MAGIC => false,
        OVERLAY_SLOT_MANIFEST_MAGIC => true,
        _ => eyre::bail!("invalid tex_land_ec slot table magic {magic:?}; repack with the current udd-cli"),
    };
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("tex_land_ec slot manifest", version, TEX_LAND_EC_METADATA_VERSION)?;
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid tex_land_ec atlas packing mode"))?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        slots.push(TexLandEcSlotRecord {
            art_id: cursor.read_u32::<LittleEndian>()?,
            page_index: cursor.read_u32::<LittleEndian>()?,
            page_tile_index: cursor.read_u16::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
        });
    }
    Ok((w, h, g, packing_mode, base_page_layout, slots))
}

fn parse_terrain_provenance_manifest(bytes: &[u8]) -> eyre::Result<Vec<TexLandEcTerrainProvenanceRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != TERRAIN_PROVENANCE_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("tex_land_ec terrain provenance", version, TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        records.push(TexLandEcTerrainProvenanceRecord {
            material_id: cursor.read_u32::<LittleEndian>()?,
            material_name_id: cursor.read_i32::<LittleEndian>()?,
            alias_count_index: cursor.read_u32::<LittleEndian>()?,
            alias_slot_id: cursor.read_u32::<LittleEndian>()?,
            alias_tile_flags: cursor.read_u64::<LittleEndian>()?,
            selected_texture_id: cursor.read_u32::<LittleEndian>()?,
            canonical_slot_id: cursor.read_u32::<LittleEndian>()?,
            selected_layer_index: cursor.read_u32::<LittleEndian>()?,
            selected_texture_repetition: cursor.read_f32::<LittleEndian>()?,
            primary_texture_id: cursor.read_u32::<LittleEndian>()?,
            primary_layer_index: cursor.read_u32::<LittleEndian>()?,
            layer_class: {
                let code = cursor.read_u8()?;
                TerrainLayerClass::from_u8(code)
                    .ok_or_else(|| eyre::eyre!("tex_land_ec terrain provenance: unknown layer class {code}"))?
            },
            selected_layer_role: {
                let code = cursor.read_u8()?;
                if code == TERRAIN_LAYER_ROLE_NONE {
                    None
                } else {
                    Some(TerrainLayerRole::from_u8(code).ok_or_else(|| {
                        eyre::eyre!("tex_land_ec terrain provenance: unknown layer role {code}")
                    })?)
                }
            },
        });
    }
    Ok(records)
}

/// The embedded KDL routing overlay (`TerrainTranscode.kdl`), the only routing table the runtime reads: the packer
/// always writes it, and it must parse.
fn read_transcode_from_package(package: &UddpReader) -> eyre::Result<HashMap<u32, u32>> {
    if package.find_by_path_hash(xxh64_virtual_path(UDDP_TRANSCODE_ENTRY_VPATH)).is_none() {
        eyre::bail!(
            "tex_land_ec.uddp missing {UDDP_TRANSCODE_ENTRY_VPATH}, the terrain routing table (TerrainTranscode.kdl); \
             repack it with the current udd-cli"
        );
    }
    let bytes = read_path_entry_cow(package, UDDP_TRANSCODE_ENTRY_VPATH)?;
    let text = std::str::from_utf8(bytes.as_ref())
        .wrap_err_with(|| format!("{UDDP_TRANSCODE_ENTRY_VPATH}: not UTF-8 text"))?;
    Ok(crate::eckr_terrain_kdl::EckrTerrainRouting::from_str(UDDP_TRANSCODE_ENTRY_VPATH, text)?.to_map())
}

/// The embedded terrain overrides (`EcTerrainOverrides.kdl` as packed), the only terrain overrides the runtime reads:
/// the packer always writes them, and they must parse.
fn read_terrain_overrides_metadata_maps_from_package(
    package: &UddpReader,
) -> eyre::Result<(
    HashMap<u32, TexLandEcTerrainOverrideActions>,
    HashMap<u32, TexLandEcTerrainOverrideDetails>,
    HashMap<u32, Vec<TexLandEcTerrainOverrideTextureRef>>,
)> {
    if package.find_by_path_hash(xxh64_virtual_path(UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH)).is_none() {
        eyre::bail!(
            "tex_land_ec.uddp missing {UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH}, the terrain overrides \
             (EcTerrainOverrides.kdl); repack it with the current udd-cli"
        );
    }
    let bytes = read_path_entry_cow(package, UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH)?;
    parse_terrain_overrides_metadata_maps(bytes.as_ref())
        .wrap_err_with(|| format!("parse {UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH}"))
}

fn terrain_override_maps_from_overrides(
    overrides: &EcTerrainOverrides,
) -> (
    HashMap<u32, TexLandEcTerrainOverrideActions>,
    HashMap<u32, TexLandEcTerrainOverrideDetails>,
    HashMap<u32, Vec<TexLandEcTerrainOverrideTextureRef>>,
) {
    let mut actions = HashMap::new();
    let mut details = HashMap::new();
    let mut layer_texture_refs = HashMap::<u32, Vec<TexLandEcTerrainOverrideTextureRef>>::new();
    let mut explicit_texture_refs = HashMap::<u32, Vec<TexLandEcTerrainOverrideTextureRef>>::new();

    for terrain in &overrides.terrains {
        let material_id = terrain.id;
        let action = actions
            .entry(material_id)
            .or_insert_with(|| TexLandEcTerrainOverrideActions {
                material_id,
                action_count: 0,
                action_flags: 0,
            });
        let detail = details
            .entry(material_id)
            .or_insert_with(|| TexLandEcTerrainOverrideDetails {
                material_id,
                ignore_code: None,
            });

        if !terrain.layers.is_empty() {
            action.action_count += terrain.layers.len() as u32;
            action.action_flags |= TERRAIN_OVERRIDE_ACTION_LAYER;
            let refs = layer_texture_refs.entry(material_id).or_default();
            for layer in &terrain.layers {
                let fallback_layer_index = refs.len() as u32;
                let semantic_layer_index =
                    terrain_override_role_layer_index(&layer.role).unwrap_or(fallback_layer_index);
                refs.push(TexLandEcTerrainOverrideTextureRef {
                    material_id,
                    role: layer.role.clone(),
                    texture_id: layer.texture,
                    layer_index: Some(semantic_layer_index),
                    texture_repetition: layer.stretch,
                });
            }
        }

        if !terrain.textures.is_empty() {
            action.action_count += terrain.textures.len() as u32;
            action.action_flags |= TERRAIN_OVERRIDE_ACTION_TEXTURE;
            let refs = explicit_texture_refs.entry(material_id).or_default();
            for texture in &terrain.textures {
                refs.push(TexLandEcTerrainOverrideTextureRef {
                    material_id,
                    role: texture
                        .role
                        .clone()
                        .unwrap_or_else(|| "texture".to_string()),
                    texture_id: texture.texture,
                    layer_index: None,
                    texture_repetition: None,
                });
            }
        }

        if let Some(ignore) = terrain.ignore.as_ref() {
            if !action.has_ignore() {
                action.action_count += 1;
            }
            action.action_flags |= TERRAIN_OVERRIDE_ACTION_IGNORE;
            detail.ignore_code = ignore.code.clone();
        }
    }

    let mut texture_refs = layer_texture_refs;
    for (material_id, mut refs) in explicit_texture_refs {
        texture_refs.entry(material_id).or_default().append(&mut refs);
    }

    (actions, details, texture_refs)
}

#[cfg(test)]
fn parse_terrain_override_actions_metadata(
    bytes: &[u8],
) -> eyre::Result<HashMap<u32, TexLandEcTerrainOverrideActions>> {
    let (actions, _, _) = parse_terrain_overrides_metadata_maps(bytes)?;
    Ok(actions)
}

fn parse_terrain_overrides_metadata_maps(
    bytes: &[u8],
) -> eyre::Result<(
    HashMap<u32, TexLandEcTerrainOverrideActions>,
    HashMap<u32, TexLandEcTerrainOverrideDetails>,
    HashMap<u32, Vec<TexLandEcTerrainOverrideTextureRef>>,
)> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes)?;
    let schema = value.get("schema").and_then(|value| value.as_str());
    eyre::ensure!(
        schema == Some(TERRAIN_OVERRIDES_SCHEMA),
        "schema {schema:?}, expected {TERRAIN_OVERRIDES_SCHEMA:?}"
    );
    let schema_version = json_u32(&value, "schema_version")
        .ok_or_else(|| eyre::eyre!("missing schema_version"))?;
    ensure_manifest_version("tex_land_ec terrain overrides", schema_version, TERRAIN_OVERRIDES_SCHEMA_VERSION)?;
    let mut actions = HashMap::new();
    let mut details_by_material = HashMap::<u32, TexLandEcTerrainOverrideDetails>::new();
    let mut refs_by_material = HashMap::<u32, Vec<TexLandEcTerrainOverrideTextureRef>>::new();
    let Some(entries) = value.get("entries").and_then(|value| value.as_array()) else {
        return Ok((actions, details_by_material, refs_by_material));
    };

    for entry in entries {
        let Some(material_id) = json_u32(entry, "material_id") else {
            continue;
        };

        let action_count = json_u32(entry, "active_action_count").unwrap_or(0);
        let mut action_flags = 0u16;
        if json_array_is_non_empty(entry, "layers") {
            action_flags |= TERRAIN_OVERRIDE_ACTION_LAYER;
        }
        if json_array_is_non_empty(entry, "textures") {
            action_flags |= TERRAIN_OVERRIDE_ACTION_TEXTURE;
        }
        if !entry.get("ignore").unwrap_or(&serde_json::Value::Null).is_null() {
            action_flags |= TERRAIN_OVERRIDE_ACTION_IGNORE;
        }

        actions.insert(
            material_id,
            TexLandEcTerrainOverrideActions {
                material_id,
                action_count,
                action_flags,
            },
        );

        let ignore_code = entry
            .get("ignore")
            .filter(|value| !value.is_null())
            .and_then(|ignore| json_string(ignore, "code"));

        details_by_material.insert(
            material_id,
            TexLandEcTerrainOverrideDetails {
                material_id,
                ignore_code,
            },
        );

        if let Some(layers) = entry.get("layers").and_then(|value| value.as_array()) {
            for (layer_index, layer) in layers.iter().enumerate() {
                let Some(texture_id) = json_u32(layer, "texture_id") else {
                    continue;
                };
                let role = layer
                    .get("role")
                    .and_then(|value| value.as_str())
                    .unwrap_or("layer")
                    .to_string();
                refs_by_material
                    .entry(material_id)
                    .or_default()
                    .push(TexLandEcTerrainOverrideTextureRef {
                        material_id,
                        role,
                        texture_id,
                        layer_index: Some(
                            terrain_override_role_layer_index(
                                layer
                                    .get("role")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("layer"),
                            )
                            .unwrap_or(layer_index as u32),
                        ),
                        texture_repetition: json_f32(layer, "stretch"),
                    });
            }
        }
        if let Some(textures) = entry.get("textures").and_then(|value| value.as_array()) {
            for texture in textures {
                let Some(texture_id) = json_u32(texture, "texture_id") else {
                    continue;
                };
                let role = texture
                    .get("role")
                    .and_then(|value| value.as_str())
                    .unwrap_or("texture")
                    .to_string();
                refs_by_material
                    .entry(material_id)
                    .or_default()
                    .push(TexLandEcTerrainOverrideTextureRef {
                        material_id,
                        role,
                        texture_id,
                        layer_index: None,
                        texture_repetition: None,
                    });
            }
        }
    }

    Ok((actions, details_by_material, refs_by_material))
}

fn json_u32(value: &serde_json::Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
}

fn json_f32(value: &serde_json::Value, key: &str) -> Option<f32> {
    value
        .get(key)
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
}

fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn json_array_is_non_empty(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(|value| value.as_array())
        .is_some_and(|values| !values.is_empty())
}

fn optional_texture_id(value: u32) -> Option<u32> {
    (value != MISSING_TEXTURE_ID).then_some(value)
}

fn optional_terrain_layer_index(value: u32) -> Option<u32> {
    (value != MISSING_TERRAIN_LAYER_INDEX).then_some(value)
}

/// Whether a `texture` override's role (`base`, `detail`, `mask`, `normal`) names the material layer.
fn terrain_override_role_matches_layer(role: &str, layer_index: u32) -> bool {
    matches!(
        (layer_index, role),
        (0, "base") | (1, "detail") | (2, "mask") | (3, "normal")
    )
}

/// The material layer of a `layer` override's role (`t0`, `t1`, `m`, `n`); `s` and unknown roles have none.
fn terrain_override_role_layer_index(role: &str) -> Option<u32> {
    match role {
        "t0" => Some(0),
        "t1" => Some(1),
        "m" => Some(2),
        "n" => Some(3),
        _ => None,
    }
}

fn optional_texture_sort_key(value: u32, missing: u32) -> u32 {
    if value == missing {
        u32::MAX
    } else {
        value
    }
}

/// The client grunge texture of a land package ([`UDDP_GRUNGE_ENTRY_VPATH`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrungeImage {
    pub width: u32,
    pub height: u32,
    /// RGBA8888 rows, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// The bytes of a `width` x `height` RGBA8888 image, or `None` when that overflows.
fn grunge_rgba_len(width: u32, height: u32) -> Option<usize> {
    usize::try_from(width).ok()?.checked_mul(usize::try_from(height).ok()?)?.checked_mul(GRUNGE_BYTES_PER_PIXEL)
}

/// Bytes of [`UDDP_GRUNGE_ENTRY_VPATH`]: the version header, then `rgba`, which must hold `width` x `height` RGBA8888
/// pixels.
pub fn encode_grunge_entry(width: u32, height: u32, rgba: &[u8]) -> eyre::Result<Vec<u8>> {
    eyre::ensure!(
        width > 0 && height > 0 && grunge_rgba_len(width, height) == Some(rgba.len()),
        "{UDDP_GRUNGE_ENTRY_VPATH}: {} bytes is not a {width}x{height} RGBA8888 image",
        rgba.len()
    );
    let mut bytes = Vec::with_capacity(GRUNGE_HEADER_SIZE + rgba.len());
    bytes.extend_from_slice(&GRUNGE_MAGIC);
    bytes.extend_from_slice(&TEX_LAND_EC_GRUNGE_VERSION.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.push(PagePixelFormat::Rgba8888 as u8);
    bytes.extend_from_slice(rgba);
    Ok(bytes)
}

/// The image of a [`UDDP_GRUNGE_ENTRY_VPATH`] entry, after checking its version header.
pub fn decode_grunge_entry(bytes: &[u8]) -> eyre::Result<GrungeImage> {
    eyre::ensure!(
        bytes.len() >= GRUNGE_HEADER_SIZE && bytes[0..4] == GRUNGE_MAGIC,
        "{UDDP_GRUNGE_ENTRY_VPATH}: no ELGR version header; repack with the current udd-cli"
    );
    let mut cursor = Cursor::new(&bytes[GRUNGE_MAGIC.len()..GRUNGE_HEADER_SIZE]);
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version(UDDP_GRUNGE_ENTRY_VPATH, version, TEX_LAND_EC_GRUNGE_VERSION)?;
    let width = cursor.read_u32::<LittleEndian>()?;
    let height = cursor.read_u32::<LittleEndian>()?;
    let pixel_format = cursor.read_u8()?;
    eyre::ensure!(
        PagePixelFormat::from_repr(pixel_format) == Some(PagePixelFormat::Rgba8888),
        "{UDDP_GRUNGE_ENTRY_VPATH}: pixel format {pixel_format} is not RGBA8888"
    );
    let rgba = &bytes[GRUNGE_HEADER_SIZE..];
    eyre::ensure!(
        width > 0 && height > 0 && grunge_rgba_len(width, height) == Some(rgba.len()),
        "{UDDP_GRUNGE_ENTRY_VPATH}: {} pixel bytes do not make the {width}x{height} RGBA8888 image of its header",
        rgba.len()
    );
    Ok(GrungeImage { width, height, rgba: rgba.to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;
    use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

    /// A pixel format byte this reader doesn't know is an error, not a panic.
    #[test]
    fn unknown_page_pixel_format_is_an_error() {
        const UNKNOWN_PIXEL_FORMAT: u8 = u8::MAX;
        const NO_PAGES: u32 = 0;
        let mut bytes = PAGE_MANIFEST_MAGIC.to_vec();
        bytes.extend_from_slice(&TEX_LAND_EC_METADATA_VERSION.to_le_bytes());
        for width_height_gutter in [0u32; 3] {
            bytes.extend_from_slice(&width_height_gutter.to_le_bytes());
        }
        bytes.push(UNKNOWN_PIXEL_FORMAT);
        bytes.push(AtlasPackingMode::MaximumPacking as u8);
        bytes.extend_from_slice(&NO_PAGES.to_le_bytes());

        let error = parse_page_manifest(&bytes).unwrap_err();
        assert!(error.to_string().contains("pixel format"), "{error}");
    }

    const EMPTY_SOURCE_MATERIALS_JSON: &[u8] = b"[]";
    /// A routing table without rows: the client's routes apply unchanged.
    const EMPTY_TRANSCODE_KDL: &[u8] = b"";

    fn add_metadata_file(
        package: &mut UddpBuilder,
        virtual_path: &str,
        data: &[u8],
    ) {
        package
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::None,
                width: 0,
                height: 0,
                virtual_path: Some(virtual_path),
                path_hash64: None,
                id: None,
                data,
            })
            .expect("add metadata file");
    }

    fn page_manifest_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
        bytes.write_u32::<LittleEndian>(TEX_LAND_EC_METADATA_VERSION).unwrap();
        bytes.write_u32::<LittleEndian>(64).unwrap();
        bytes.write_u32::<LittleEndian>(64).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u8(PagePixelFormat::Rgba8888 as u8).unwrap();
        bytes.write_u8(AtlasPackingMode::MaximumPacking as u8).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes
    }

    fn slot_manifest_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SLOT_MANIFEST_MAGIC);
        bytes.write_u32::<LittleEndian>(TEX_LAND_EC_METADATA_VERSION).unwrap();
        bytes.write_u32::<LittleEndian>(64).unwrap();
        bytes.write_u32::<LittleEndian>(64).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u8(AtlasPackingMode::MaximumPacking as u8).unwrap();
        bytes.write_u32::<LittleEndian>(102).unwrap();
        for art_id in 0..=101u32 {
            bytes.write_u32::<LittleEndian>(art_id).unwrap();
            if art_id == 77 || art_id == 100 || art_id == 101 {
                bytes.write_u32::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(SLOT_FLAG_PRESENT | SLOT_FLAG_LAND).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(44).unwrap();
                bytes.write_u16::<LittleEndian>(44).unwrap();
            } else {
                bytes.write_u32::<LittleEndian>(MISSING_PAGE_INDEX).unwrap();
                bytes.write_u16::<LittleEndian>(MISSING_PAGE_TILE_INDEX).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
            }
        }
        bytes
    }

    fn terrain_provenance_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&TERRAIN_PROVENANCE_MAGIC);
        bytes.write_u32::<LittleEndian>(TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION).unwrap();
        bytes.write_u32::<LittleEndian>(3).unwrap();
        for texture_id in [1000003, 2000520, 2000510] {
            let canonical_slot_id = match texture_id {
                1000003 => 77,
                2000520 => 100,
                2000510 => 101,
                _ => unreachable!(),
            };
            bytes.write_u32::<LittleEndian>(52).unwrap();
            bytes.write_i32::<LittleEndian>(0).unwrap();
            bytes.write_u32::<LittleEndian>(0).unwrap();
            bytes.write_u32::<LittleEndian>(77).unwrap();
            bytes.write_u64::<LittleEndian>(0).unwrap();
            bytes.write_u32::<LittleEndian>(texture_id).unwrap();
            bytes.write_u32::<LittleEndian>(canonical_slot_id).unwrap();
            bytes
                .write_u32::<LittleEndian>(if texture_id == 1000003 {
                    MISSING_TERRAIN_LAYER_INDEX
                } else if texture_id == 2000520 {
                    0
                } else {
                    1
                })
                .unwrap();
            bytes
                .write_f32::<LittleEndian>(if texture_id == 1000003 { 0.0 } else { 5.0 })
                .unwrap();
            bytes.write_u32::<LittleEndian>(2000520).unwrap();
            bytes.write_u32::<LittleEndian>(0).unwrap();
            bytes.write_u8(TerrainLayerClass::Default as u8).unwrap();
            bytes
                .write_u8(match texture_id {
                    2000520 => TerrainLayerRole::ColourA as u8,
                    2000510 => TerrainLayerRole::ColourB as u8,
                    _ => TERRAIN_LAYER_ROLE_NONE,
                })
                .unwrap();
        }
        bytes
    }

    fn terrain_overrides_json() -> Vec<u8> {
        br#"{
  "schema": "tex_land_ec_terrain_overrides",
  "schema_version": 1,
  "entries": [
    {
      "material_id": 52,
      "active_action_count": 1,
      "layers": [{"role": "t0", "texture_id": 2000510}],
      "textures": [],
      "ignore": null
    }
  ]
}"#
        .to_vec()
    }

    fn test_package() -> TexLandEcPackage {
        test_package_with_source_map(None)
    }

    fn test_package_with_source_map(source_csv: Option<&[u8]>) -> TexLandEcPackage {
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        add_metadata_file(&mut builder, UDDP_PAGE_MANIFEST_ENTRY_VPATH, &page_manifest_bytes());
        add_metadata_file(&mut builder, UDDP_SLOT_MANIFEST_ENTRY_VPATH, &slot_manifest_bytes());
        add_metadata_file(
            &mut builder,
            UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH,
            &terrain_provenance_bytes(),
        );
        add_metadata_file(
            &mut builder,
            UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH,
            &terrain_overrides_json(),
        );
        if let Some(bytes) = source_csv {
            add_metadata_file(&mut builder, UDDP_LEGACY_TERRAIN_MAP_ENTRY_VPATH, bytes);
            add_metadata_file(&mut builder, UDDP_TRANSCODE_ENTRY_VPATH, b"t 52 203\n");
            use uocf::enhanced::terrain_definition::{TerrainDefinitionEntry, TerrainDefinitionTexture, TerrainDefinitionTextureLayer};
            let source = vec![TerrainDefinitionEntry {
                id: 52,
                texture: Some(TerrainDefinitionTexture {
                    shader_name: Some("UOWaterTerrainLayer".to_owned()),
                    layers: [(2000520, 16.0), (2000510, 4.0), (2000530, 1.0)].into_iter()
                        .map(|(id, rep)| TerrainDefinitionTextureLayer {
                            texture_id: Some(id), texture_repetition: rep, ..Default::default()
                        }).collect(),
                    ..Default::default()
                }),
                ..Default::default()
            }];
            add_metadata_file(&mut builder, UDDP_SOURCE_MATERIALS_ENTRY_VPATH, &serde_json::to_vec(&source).unwrap());
        } else {
            add_metadata_file(&mut builder, UDDP_TRANSCODE_ENTRY_VPATH, EMPTY_TRANSCODE_KDL);
            add_metadata_file(&mut builder, UDDP_SOURCE_MATERIALS_ENTRY_VPATH, EMPTY_SOURCE_MATERIALS_JSON);
        }
        let bytes = builder.build().expect("build test package");
        TexLandEcPackage::from_uddp_package(UddpReader::open(bytes).expect("open package"))
            .expect("load test package")
    }

    #[test]
    fn the_grunge_entry_round_trips_through_its_header() {
        const WIDTH: u32 = 4;
        const HEIGHT: u32 = 8;
        let rgba: Vec<u8> = (0..WIDTH * HEIGHT * GRUNGE_BYTES_PER_PIXEL as u32).map(|byte| byte as u8).collect();
        let bytes = encode_grunge_entry(WIDTH, HEIGHT, &rgba).expect("encode");
        assert_eq!(&bytes[..4], b"ELGR");
        assert_eq!(bytes.len(), GRUNGE_HEADER_SIZE + rgba.len());
        assert_eq!(decode_grunge_entry(&bytes).expect("decode"), GrungeImage { width: WIDTH, height: HEIGHT, rgba: rgba.clone() });
        assert!(encode_grunge_entry(WIDTH, HEIGHT, &rgba[1..]).is_err(), "pixels short of the size");
        assert!(encode_grunge_entry(0, 0, &[]).is_err(), "an empty image");
        assert!(decode_grunge_entry(&bytes[..bytes.len() - 1]).is_err(), "pixels short of the header's size");
        let mut bc7 = bytes.clone();
        bc7[GRUNGE_HEADER_SIZE - 1] = PagePixelFormat::Bc7 as u8;
        assert!(decode_grunge_entry(&bc7).unwrap_err().to_string().contains("not RGBA8888"));
        assert!(test_package().grunge_rgba().expect("no entry is no error").is_none());
    }

    #[test]
    fn the_grunge_entry_rejects_other_versions_and_headerless_bytes_with_the_repack_message() {
        const KR_GRUNGE_SIDE: usize = 256;
        let rgba = [0u8; GRUNGE_BYTES_PER_PIXEL];
        let mut other = encode_grunge_entry(1, 1, &rgba).expect("encode");
        other[4..8].copy_from_slice(&(TEX_LAND_EC_GRUNGE_VERSION + 1).to_le_bytes());
        // The entry before the header: bare RGBA8888 rows.
        let headerless = vec![0u8; KR_GRUNGE_SIDE * KR_GRUNGE_SIDE * GRUNGE_BYTES_PER_PIXEL];
        for bytes in [&other[..], &headerless[..]] {
            let error = decode_grunge_entry(bytes).unwrap_err().to_string();
            assert!(error.contains("repack with the current udd-cli"), "{error}");
        }
    }

    #[test]
    fn a_package_without_the_routing_table_asks_for_a_repack() {
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        add_metadata_file(&mut builder, UDDP_PAGE_MANIFEST_ENTRY_VPATH, &page_manifest_bytes());
        add_metadata_file(&mut builder, UDDP_SLOT_MANIFEST_ENTRY_VPATH, &slot_manifest_bytes());
        add_metadata_file(&mut builder, UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH, &terrain_provenance_bytes());
        add_metadata_file(&mut builder, UDDP_SOURCE_MATERIALS_ENTRY_VPATH, EMPTY_SOURCE_MATERIALS_JSON);
        let reader = UddpReader::open(builder.build().expect("build test package")).expect("open package");
        let error = TexLandEcPackage::from_uddp_package(reader).err().expect("a package without the table").to_string();
        assert!(error.contains(UDDP_TRANSCODE_ENTRY_VPATH) && error.contains("repack"), "{error}");
    }

    #[test]
    fn a_package_without_the_terrain_overrides_asks_for_a_repack() {
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        add_metadata_file(&mut builder, UDDP_PAGE_MANIFEST_ENTRY_VPATH, &page_manifest_bytes());
        add_metadata_file(&mut builder, UDDP_SLOT_MANIFEST_ENTRY_VPATH, &slot_manifest_bytes());
        add_metadata_file(&mut builder, UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH, &terrain_provenance_bytes());
        add_metadata_file(&mut builder, UDDP_TRANSCODE_ENTRY_VPATH, EMPTY_TRANSCODE_KDL);
        add_metadata_file(&mut builder, UDDP_SOURCE_MATERIALS_ENTRY_VPATH, EMPTY_SOURCE_MATERIALS_JSON);
        let reader = UddpReader::open(builder.build().expect("build test package")).expect("open package");
        let error = TexLandEcPackage::from_uddp_package(reader).err().expect("a package without the overrides").to_string();
        assert!(error.contains(UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH) && error.contains("repack"), "{error}");
    }

    /// A package of another metadata version is refused with a repack hint.
    #[test]
    fn a_package_of_another_metadata_version_asks_for_a_repack() {
        const OTHER_METADATA_VERSION: u32 = TEX_LAND_EC_METADATA_VERSION + 1;
        const VERSION_OFFSET: usize = PAGE_MANIFEST_MAGIC.len();
        let mut page_manifest = page_manifest_bytes();
        page_manifest[VERSION_OFFSET..VERSION_OFFSET + size_of::<u32>()]
            .copy_from_slice(&OTHER_METADATA_VERSION.to_le_bytes());

        let error = parse_page_manifest(&page_manifest).unwrap_err().to_string();

        assert!(error.contains(&format!("version {OTHER_METADATA_VERSION}")) && error.contains("repack"), "{error}");
    }

    #[test]
    fn source_terrain_routes_roundtrip_without_losing_subtypes_or_overrides() {
        let source_csv = b"legacyId,newId,newSubType\r\n203,99,2\r\n500,52,1\r\n";
        let mut package = test_package_with_source_map(Some(source_csv));
        assert_eq!(package.transcode[&203], 52, "explicit embedded route overrides source policy");
        assert_eq!(package.resolve_runtime_slot_id(500), Some(100));
        let source = package.source_terrain_map().unwrap();
        assert_eq!(source.source_csv, source_csv);
        assert_eq!(source.routes[&203].material_id, 99);
        assert_eq!(source.routes[&203].subtype, 2);
        package.set_transcode(HashMap::from([(500, 99)]));
        assert_eq!(package.transcode[&500], 99);
        assert_eq!(package.source_terrain_map().unwrap().routes[&500].material_id, 52);
    }

    #[test]
    fn source_liquid_graph_separates_fallback_bump_and_animated_surface() {
        let package = test_package_with_source_map(Some(b"legacyId,newId,newSubType\n500,52,0\n"));
        let [base, detail, mask, normal, animated] = package.resolve_material_render_layers(500);
        assert_eq!(base.unwrap().texture_id, 2000510);
        assert_eq!(normal.unwrap().texture_id, 2000520);
        assert_eq!(normal.unwrap().texture_repetition, 16.0);
        assert_eq!(animated.unwrap().texture_id, 2000530);
        assert!(detail.is_none() && mask.is_none());
        assert_eq!(package.source_liquid_payload([2000520, 2000510, 2000530]), Some(203));
        assert_eq!(package.source_liquid_payload([2000520, 2000510, 2000999]), None);
    }

    #[test]
    fn default_terrain_fourth_art_source_is_not_a_normal_map() {
        let mut package = test_package_with_source_map(Some(b"legacyId,newId,newSubType\n500,52,0\n"));
        package.set_terrain_overrides(&EcTerrainOverrides { terrains: Vec::new() });
        package.source_materials.get_mut(&52).unwrap().texture.as_mut().unwrap()
            .shader_name = Some("UODefaultTerrainLayer".to_owned());
        let mut fourth = package.terrain_provenance.iter()
            .find(|record| record.selected_texture_id == 2000520).copied().unwrap();
        fourth.selected_layer_index = 3;
        package.terrain_provenance.push(fourth);
        // Preserved source access still sees the authored fourth image.
        assert_eq!(package.resolve_material_layer_slot(500, 3).unwrap().texture_id, 2000520);
        assert!(package.resolve_material_render_layers(500)[3].is_none());

        let overrides = EcTerrainOverrides::parse("normal.kdl",
            "terrain 52 { layer \"n\" tex=2000510 stretch=7.0; }").unwrap();
        package.set_terrain_overrides(&overrides);
        let normal = package.resolve_material_render_layers(500)[3].unwrap();
        assert_eq!(normal.texture_id, 2000510);
        assert_eq!(normal.texture_repetition, 7.0);
    }

    #[test]
    fn authored_bump_terrain_retains_its_fourth_normal_source() {
        let mut package = test_package_with_source_map(Some(b"legacyId,newId,newSubType\n500,52,0\n"));
        package.set_terrain_overrides(&EcTerrainOverrides { terrains: Vec::new() });
        package.source_materials.get_mut(&52).unwrap().texture.as_mut().unwrap()
            .shader_name = Some("UOBumpMapTerrainLayer".to_owned());
        let mut fourth = package.terrain_provenance.iter()
            .find(|record| record.selected_texture_id == 2000520).copied().unwrap();
        fourth.selected_layer_index = 3;
        package.terrain_provenance.push(fourth);
        let normal = package.resolve_material_render_layers(500)[3].unwrap();
        assert_eq!(normal.texture_id, 2000520);
        assert_eq!(normal.runtime_slot_id, Some(100));
    }

    #[test]
    fn bump_terrain_binds_its_environment_map_last() {
        const ENVIRONMENT_TEXTURE_ID: u32 = 2000510;
        const ENVIRONMENT_REPETITION: f32 = 2.0;
        let mut package = test_package_with_source_map(Some(b"legacyId,newId,newSubType\n500,52,0\n"));
        package.set_terrain_overrides(&EcTerrainOverrides { terrains: Vec::new() });
        let texture = package.source_materials.get_mut(&52).unwrap().texture.as_mut().unwrap();
        texture.shader_name = Some("UODefaultTerrainLayer".to_owned());
        for (texture_id, texture_repetition) in [(2000520, 12.0), (ENVIRONMENT_TEXTURE_ID, ENVIRONMENT_REPETITION)] {
            texture.layers.push(uocf::enhanced::terrain_definition::TerrainDefinitionTextureLayer {
                texture_id: Some(texture_id), texture_repetition, ..Default::default()
            });
        }
        // A default layer never reads a fifth item.
        assert!(package.resolve_material_render_layers(500)[4].is_none());

        package.source_materials.get_mut(&52).unwrap().texture.as_mut().unwrap()
            .shader_name = Some("UOBumpMapTerrainLayer".to_owned());
        let environment = package.resolve_material_render_layers(500)[4].unwrap();
        assert_eq!(environment.texture_id, ENVIRONMENT_TEXTURE_ID);
        assert_eq!(environment.layer_index, 4);
        assert_eq!(environment.texture_repetition, ENVIRONMENT_REPETITION);
        assert_eq!(environment.runtime_slot_id, package.resolve_texture_id_slot(52, ENVIRONMENT_TEXTURE_ID));
        assert!(environment.runtime_slot_id.is_some());
    }

    #[test]
    fn source_liquid_active_match_excludes_item_fallback_and_other_material_parameters() {
        let mut package = test_package_with_source_map(Some(b"legacyId,newId,newSubType\n500,52,0\n"));
        let texture = package.source_materials.get_mut(&52).unwrap().texture.as_mut().unwrap();
        texture.techniques = vec![2, 1, 0, 0];
        texture.tweaks = vec![-0.012, 0.0012, 0.0, 0.0];
        let sources = [(2000520, 16.0), (2000530, 1.0)];
        let scroll = [-0.012, 0.0012];
        // A local fallback mask has no bearing on the active bump/surface pair.
        assert_eq!(package.source_liquid_payload([2000520, 1011816, 2000530]), None);
        assert_eq!(package.source_liquid_payload_for_active_sources(sources, 2, scroll), Some(203));
        for wrong in [[(2000520, 12.0), sources[1]], [sources[0], (2000999, 1.0)],
            [(2000520, f32::NAN), sources[1]]] {
            assert_eq!(package.source_liquid_payload_for_active_sources(wrong, 2, scroll), None);
        }
        assert_eq!(package.source_liquid_payload_for_active_sources(sources, 0, scroll), None);
        assert_eq!(package.source_liquid_payload_for_active_sources(sources, 2, [-0.1, 0.01]), None);
        assert_eq!(package.source_liquid_payload_for_active_sources(sources, 2, [f32::NAN, 0.0]), None);

        use crate::tilemeta::{EcMaterialPhysicalPackage, TileMetaItemTextureRef};
        let material = uocf::enhanced::tileart::ArtMaterial {
            shader_name: Some("UOWaterShader".into()), technique: vec![2, 1],
            tweaks: scroll.to_vec(), ..Default::default()
        };
        let refs: [_; 3] = [(2000520, 16.0), (1011816, 1.0), (2000530, 1.0)]
            .map(|(texture_id, texture_stretch)| TileMetaItemTextureRef {
                texture_id, texture_stretch, texture_type: 1,
                physical_package: EcMaterialPhysicalPackage::Texture as u8, ..Default::default()
            });
        let refs = [0, 1, 2].map(|position| TileMetaItemTextureRef {
            item_index: position as u8,
            stable_role: crate::tilemeta::EcMaterialStableRole::for_texture_position(
                0, material.shader_name.as_deref(), position as u8) as u8,
            ..refs[position]
        });
        assert_eq!(package.source_liquid_payload_for_item(&material, &refs), Some(203));
        let mut wrong = refs;
        wrong[0].physical_package = EcMaterialPhysicalPackage::LegacyTexture as u8;
        assert_eq!(package.source_liquid_payload_for_item(&material, &wrong), None);
        wrong = refs;
        wrong[2].block_index = 1;
        assert_eq!(package.source_liquid_payload_for_item(&material, &wrong), None);
        let other_shader = uocf::enhanced::tileart::ArtMaterial {
            shader_name: Some("UOSpriteShader".into()), ..material
        };
        assert_eq!(package.source_liquid_payload_for_item(&other_shader, &refs), None);
    }

    #[test]
    fn debug_package_load_counters_track_tex_land_ec_work() {
        debug_reset_package_load_counters();
        assert_eq!(debug_from_uddp_package_with_options_call_count(), 0);
        assert_eq!(debug_set_terrain_overrides_call_count(), 0);

        let mut package = test_package();
        assert_eq!(debug_from_uddp_package_with_options_call_count(), 1);
        assert_eq!(debug_set_terrain_overrides_call_count(), 0);

        let overrides = EcTerrainOverrides::parse(
            "counter.kdl",
            r#"
terrain 52 {
    ignore code="counter_test"
}
"#,
        )
        .expect("parse counter overrides");
        package.set_terrain_overrides(&overrides);

        assert_eq!(debug_set_terrain_overrides_call_count(), 1);
    }

    #[test]
    fn terrain_provenance_parser_rejects_other_versions() {
        let other_version = TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION + 1;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&TERRAIN_PROVENANCE_MAGIC);
        bytes.write_u32::<LittleEndian>(other_version).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();

        let error = parse_terrain_provenance_manifest(&bytes).unwrap_err().to_string();

        assert_eq!(
            error,
            format!(
                "tex_land_ec terrain provenance: package version {other_version} is not supported \
                 (current is {TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION}); repack with the current udd-cli"
            )
        );
    }

    #[test]
    fn terrain_override_metadata_parser_rejects_other_schema_versions() {
        let json = format!(
            r#"{{"schema": "{TERRAIN_OVERRIDES_SCHEMA}", "schema_version": {}, "entries": []}}"#,
            TERRAIN_OVERRIDES_SCHEMA_VERSION + 1
        );
        let error = parse_terrain_override_actions_metadata(json.as_bytes()).unwrap_err().to_string();
        assert!(error.contains("repack"), "{error}");

        let json = format!(r#"{{"schema": "other", "schema_version": {TERRAIN_OVERRIDES_SCHEMA_VERSION}, "entries": []}}"#);
        assert!(parse_terrain_override_actions_metadata(json.as_bytes()).is_err());
    }

    #[test]
    fn terrain_override_metadata_parser_records_action_flags() {
        let actions = parse_terrain_override_actions_metadata(&terrain_overrides_json())
            .expect("parse override actions");
        let action = actions.get(&52).expect("material 52 override");

        assert_eq!(action.action_count, 1);
        assert!(action.has_layer());
    }

    #[test]
    fn material_decision_preserves_current_slot_resolution_and_override_metadata() {
        let package = test_package();
        let decision = package.resolve_material_decision(77);
        let override_slots = package.resolve_override_texture_slots(52);

        assert_eq!(package.resolve_runtime_slot_id(77), Some(100));
        assert_eq!(decision.material_id, Some(52));
        assert_eq!(decision.runtime_slot_id, Some(100));
        assert_eq!(decision.runtime_slot_source, Some(TexLandEcRuntimeSlotSource::DirectAlias));
        assert_eq!(decision.primary_texture_id, Some(2000520));
        assert_eq!(decision.primary_layer_index, Some(0));
        assert_eq!(decision.layer_class, Some(TerrainLayerClass::Default));
        assert_eq!(
            decision.override_actions.map(|actions| actions.action_flags),
            Some(TERRAIN_OVERRIDE_ACTION_LAYER)
        );
        assert_eq!(override_slots.len(), 1);
        assert_eq!(override_slots[0].role, "t0");
        assert_eq!(override_slots[0].texture_id, 2000510);
        assert_eq!(override_slots[0].runtime_slot_id, Some(101));
        assert_eq!(package.resolve_effective_runtime_slot_id(77), Some(101));

        let changes = package.effective_override_slot_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].material_id, 52);
        assert_eq!(changes[0].runtime_slot_id, Some(100));
        assert_eq!(changes[0].effective_runtime_slot_id, Some(101));
    }

    #[test]
    fn loose_transcode_overlays_embedded_transcode_without_dropping_existing_routes() {
        let mut package = test_package();
        package.transcode.insert(2000, 52);
        package.transcode.insert(2001, 99);

        package.set_transcode(HashMap::from([(77, 52), (2001, 52)]));

        assert_eq!(package.transcode.get(&2000), Some(&52));
        assert_eq!(package.transcode.get(&2001), Some(&52));
        assert_eq!(package.resolve_runtime_slot_id(2000), Some(100));
        assert_eq!(package.resolve_runtime_slot_id(2001), Some(100));
        assert_eq!(package.resolve_runtime_slot_id(77), Some(100));
    }

    #[test]
    fn loose_terrain_overrides_replace_embedded_metadata_and_resolve_packed_textures() {
        let mut package = test_package();
        let overrides = EcTerrainOverrides::parse(
            "loose.kdl",
            r#"
terrain 52 {
    layer "t0" tex=2000510 stretch=7.0 code="reviewed_layer_texture"
    layer "n" tex=2000520 stretch=12.0 code="reviewed_layer_texture"
    texture 9999999 role="detail" code="missing_from_package"
}
"#,
        )
        .expect("parse loose overrides");

        let summary = package.set_terrain_overrides(&overrides);
        let details = package
            .terrain_override_details_for(52)
            .expect("material 52 loose override details");
        let layer = package
            .resolve_material_layer_slot(77, 0)
            .expect("loose layer override");
        let normal_layer = package
            .resolve_material_layer_slot(77, 3)
            .expect("loose normal layer override");
        let override_slots = package.resolve_override_texture_slots(52);

        assert_eq!(summary.entry_count, 1);
        assert_eq!(summary.action_count, 3);
        assert_eq!(summary.texture_ref_count, 3);
        assert_eq!(summary.resolved_texture_ref_count, 2);
        assert_eq!(summary.unresolved_texture_refs[0].texture_id, 9999999);
        assert!(details.ignore_code.is_none());
        assert_eq!(layer.texture_id, 2000510);
        assert_eq!(layer.runtime_slot_id, Some(101));
        assert_eq!(layer.texture_repetition, 7.0);
        assert_eq!(normal_layer.texture_id, 2000520);
        assert_eq!(normal_layer.runtime_slot_id, Some(100));
        assert_eq!(normal_layer.texture_repetition, 12.0);
        assert_eq!(override_slots[0].runtime_slot_id, Some(101));
        assert_eq!(override_slots[1].runtime_slot_id, Some(100));
        assert_eq!(override_slots[2].runtime_slot_id, None);
    }

    #[test]
    fn loose_terrain_overrides_merge_duplicate_terrain_nodes_without_reordering_refs() {
        let mut package = test_package();
        let overrides = EcTerrainOverrides::parse(
            "loose.kdl",
            r#"
terrain 52 {
    layer "unknown_a" tex=2000510
    texture 9999999 role="detail"
    ignore code="old_ignore"
}
terrain 52 {
    layer "unknown_b" tex=2000520
    texture 2000510 role="base"
    ignore code="new_ignore"
}
"#,
        )
        .expect("parse loose overrides");

        let summary = package.set_terrain_overrides(&overrides);
        let details = package
            .terrain_override_details_for(52)
            .expect("material 52 loose override details");
        let refs = package.terrain_override_texture_refs_for(52);

        assert_eq!(summary.entry_count, 1);
        assert_eq!(summary.action_count, 5);
        assert_eq!(summary.texture_ref_count, 4);
        assert_eq!(summary.resolved_texture_ref_count, 3);
        assert_eq!(details.ignore_code.as_deref(), Some("new_ignore"));
        assert_eq!(refs[0].role, "unknown_a");
        assert_eq!(refs[0].layer_index, Some(0));
        assert_eq!(refs[1].role, "unknown_b");
        assert_eq!(refs[1].layer_index, Some(1));
        assert_eq!(refs[2].role, "detail");
        assert_eq!(refs[2].layer_index, None);
        assert_eq!(refs[3].role, "base");
        assert_eq!(refs[3].layer_index, None);
    }

    mod overlay {
        use super::*;
        use crate::common::KR_SECTION_ENTRY_PATH;
        use crate::tex_art_cc::page_entry_path;

        const ATLAS_SIDE: u32 = 64;
        const OTHER_ATLAS_SIDE: u32 = 128;
        const GUTTER: u32 = 1;
        const TILE_SIDE: u16 = 16;
        /// The base package's pages: the overlay's slots lie on the second and the third, not the first.
        const BASE_PAGE_BYTES: [&[u8]; 3] = [b"base page 0", b"base page 1", b"base page 2"];
        const OWN_PAGE_BYTES: &[u8] = b"kr page 0";
        const OWN_TEXTURE: u32 = 2_000_010;
        const LEFT_TEXTURE: u32 = 2_000_020;
        const OTHER_LEFT_TEXTURE: u32 = 2_000_030;
        const OWN_SLOT: u32 = 5;
        const LEFT_SLOT: u32 = 6;
        const OTHER_LEFT_SLOT: u32 = 7;
        /// A land id slot aliasing the left texture's slot, as `apply_slot_aliases` copies it.
        const ALIAS_SLOT: u32 = 3;
        const SLOT_COUNT: u32 = 8;
        const LEFT_DIFFERENCE: f32 = 0.5;
        const TEST_MATERIAL_ID: u32 = 1;
        const TEST_REPETITION: f32 = 1.0;

        fn page_manifest(side: u32, page_count: u32) -> Vec<u8> {
            let mut bytes = PAGE_MANIFEST_MAGIC.to_vec();
            for value in [TEX_LAND_EC_METADATA_VERSION, side, side, GUTTER] {
                bytes.write_u32::<LittleEndian>(value).unwrap();
            }
            bytes.write_u8(PagePixelFormat::Rgba8888 as u8).unwrap();
            bytes.write_u8(AtlasPackingMode::MaximumPacking as u8).unwrap();
            bytes.write_u32::<LittleEndian>(page_count).unwrap();
            for page_index in 0..page_count {
                for value in [page_index, 1, side, side] {
                    bytes.write_u32::<LittleEndian>(value).unwrap();
                }
            }
            bytes
        }

        fn slot(art_id: u32, page_index: u32, x: u16, flags: u16) -> TexLandEcSlotRecord {
            TexLandEcSlotRecord {
                art_id, page_index, page_tile_index: 0, flags: SLOT_FLAG_PRESENT | SLOT_FLAG_LAND | flags, x, y: 0,
                width: TILE_SIDE, height: TILE_SIDE,
            }
        }

        fn slot_manifest(side: u32, slots: &[TexLandEcSlotRecord]) -> Vec<u8> {
            let magic = if slots.iter().any(|slot| slot.is_base_page()) { OVERLAY_SLOT_MANIFEST_MAGIC } else { SLOT_MANIFEST_MAGIC };
            slot_manifest_with_magic(magic, side, slots)
        }

        fn slot_manifest_with_magic(magic: [u8; 4], side: u32, slots: &[TexLandEcSlotRecord]) -> Vec<u8> {
            let mut bytes = magic.to_vec();
            for value in [TEX_LAND_EC_METADATA_VERSION, side, side, GUTTER] {
                bytes.write_u32::<LittleEndian>(value).unwrap();
            }
            bytes.write_u8(AtlasPackingMode::MaximumPacking as u8).unwrap();
            bytes.write_u32::<LittleEndian>(SLOT_COUNT).unwrap();
            for art_id in 0..SLOT_COUNT {
                let slot = slots.iter().find(|slot| slot.art_id == art_id).copied()
                    .unwrap_or_else(|| TexLandEcSlotRecord::absent(art_id));
                bytes.write_u32::<LittleEndian>(slot.art_id).unwrap();
                bytes.write_u32::<LittleEndian>(slot.page_index).unwrap();
                bytes.write_u16::<LittleEndian>(slot.page_tile_index).unwrap();
                for value in [slot.flags, slot.x, slot.y, slot.width, slot.height] {
                    bytes.write_u16::<LittleEndian>(value).unwrap();
                }
            }
            bytes
        }

        /// One provenance record per texture, selecting it into its slot.
        fn provenance(textures: &[(u32, u32)]) -> Vec<u8> {
            let mut bytes = TERRAIN_PROVENANCE_MAGIC.to_vec();
            bytes.write_u32::<LittleEndian>(TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION).unwrap();
            bytes.write_u32::<LittleEndian>(textures.len() as u32).unwrap();
            for &(texture_id, slot_id) in textures {
                for value in [TEST_MATERIAL_ID, 0, 0, slot_id] {
                    bytes.write_u32::<LittleEndian>(value).unwrap();
                }
                bytes.write_u64::<LittleEndian>(0).unwrap();
                for value in [texture_id, slot_id, 0] {
                    bytes.write_u32::<LittleEndian>(value).unwrap();
                }
                bytes.write_f32::<LittleEndian>(TEST_REPETITION).unwrap();
                bytes.write_u32::<LittleEndian>(texture_id).unwrap();
                bytes.write_u32::<LittleEndian>(0).unwrap();
                bytes.write_u8(TerrainLayerClass::Default as u8).unwrap();
                bytes.write_u8(TerrainLayerRole::ColourA as u8).unwrap();
            }
            bytes
        }

        fn package(side: u32, pages: &[&[u8]], slots: &[TexLandEcSlotRecord], textures: &[(u32, u32)],
            equivalents: Option<&[EcEquivalentRecord]>) -> Vec<u8> {
            let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
            add_metadata_file(&mut builder, UDDP_PAGE_MANIFEST_ENTRY_VPATH, &page_manifest(side, pages.len() as u32));
            add_metadata_file(&mut builder, UDDP_SLOT_MANIFEST_ENTRY_VPATH, &slot_manifest(side, slots));
            add_metadata_file(&mut builder, UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH, &provenance(textures));
            add_metadata_file(&mut builder, UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH, &terrain_overrides_json());
            add_metadata_file(&mut builder, UDDP_TRANSCODE_ENTRY_VPATH, EMPTY_TRANSCODE_KDL);
            add_metadata_file(&mut builder, UDDP_SOURCE_MATERIALS_ENTRY_VPATH, EMPTY_SOURCE_MATERIALS_JSON);
            for (page_index, bytes) in pages.iter().enumerate() {
                add_metadata_file(&mut builder, &page_entry_path(page_index as u32, PagePixelFormat::Rgba8888), bytes);
            }
            if let Some(equivalents) = equivalents {
                let (path, bytes) = tex_land_ec_overlay_entry(&EcOverlayTable { equivalents: equivalents.to_vec() });
                add_metadata_file(&mut builder, path, &bytes);
            }
            builder.build().expect("build test package")
        }

        fn base_bytes() -> Vec<u8> {
            let slots = [slot(OWN_SLOT, 0, 0, 0), slot(LEFT_SLOT, 2, TILE_SIDE, 0), slot(OTHER_LEFT_SLOT, 1, 2 * TILE_SIDE, 0)];
            package(ATLAS_SIDE, &BASE_PAGE_BYTES, &slots, &[(OWN_TEXTURE, OWN_SLOT), (LEFT_TEXTURE, LEFT_SLOT),
                (OTHER_LEFT_TEXTURE, OTHER_LEFT_SLOT)], None)
        }

        /// The Kingdom Reborn section: its own texture on its page, the two others on the base's pages.
        fn section_bytes(side: u32, equivalents: Option<&[EcEquivalentRecord]>) -> Vec<u8> {
            let slots = [
                slot(OWN_SLOT, 0, 0, 0),
                slot(LEFT_SLOT, 2, TILE_SIDE, SLOT_FLAG_BASE_PAGE),
                slot(ALIAS_SLOT, 2, TILE_SIDE, SLOT_FLAG_BASE_PAGE),
                slot(OTHER_LEFT_SLOT, 1, 2 * TILE_SIDE, SLOT_FLAG_BASE_PAGE),
            ];
            package(side, &[OWN_PAGE_BYTES], &slots, &[(OWN_TEXTURE, OWN_SLOT), (LEFT_TEXTURE, LEFT_SLOT),
                (OTHER_LEFT_TEXTURE, OTHER_LEFT_SLOT)], equivalents)
        }

        fn left() -> [EcEquivalentRecord; 2] {
            [
                EcEquivalentRecord { id: LEFT_TEXTURE, difference: LEFT_DIFFERENCE },
                EcEquivalentRecord { id: OTHER_LEFT_TEXTURE, difference: 0.0 },
            ]
        }

        fn base_with_section(section: Vec<u8>) -> TexLandEcPackage {
            let base = UddpReader::open(base_bytes()).expect("open base");
            let bytes = UddpBuilder::extend_package(&base,
                vec![(xxh64_virtual_path(KR_SECTION_ENTRY_PATH), DataType::Metadata as u8, section)]).expect("nest");
            TexLandEcPackage::from_uddp_package(UddpReader::open(bytes).expect("open")).expect("load base")
        }

        fn kr_land(base: &TexLandEcPackage) -> eyre::Result<Option<TexLandEcPackage>> {
            base.kr_land(UddpReaderOptions::disabled(), AtlasCacheOptions::disabled())
        }

        #[test]
        fn the_kr_land_reads_the_textures_it_leaves_from_the_base_pages_appended_after_its_own() {
            let base = base_with_section(section_bytes(ATLAS_SIDE, Some(&left())));
            let land = kr_land(&base).unwrap().expect("a section");

            assert!(land.is_overlay() && !base.is_overlay());
            assert_eq!(land.equivalents(), &left());
            // Its own page, then base pages 1 and 2 in ascending order.
            assert_eq!(land.own_page_count(), 1);
            assert_eq!(land.pages().iter().map(|page| page.page_index).collect::<Vec<_>>(), [0, 1, 2]);
            assert_eq!((land.base_page_index(0), land.base_page_index(1), land.base_page_index(2)), (None, Some(1), Some(2)));
            assert_eq!(land.read_page_bytes(0).unwrap(), OWN_PAGE_BYTES);
            assert_eq!(land.read_page_bytes(1).unwrap(), BASE_PAGE_BYTES[1]);
            assert_eq!(land.read_page_bytes(2).unwrap(), BASE_PAGE_BYTES[2]);

            // Every slot is an ordinary present slot: a lookup reads it as in a full package.
            let resolved = |slot_id| *land.present_slot(slot_id).expect("present");
            assert_eq!(resolved(OWN_SLOT), slot(OWN_SLOT, 0, 0, 0));
            assert_eq!(resolved(LEFT_SLOT), slot(LEFT_SLOT, 2, TILE_SIDE, 0));
            assert_eq!(resolved(ALIAS_SLOT), slot(ALIAS_SLOT, 2, TILE_SIDE, 0));
            assert_eq!(resolved(OTHER_LEFT_SLOT), slot(OTHER_LEFT_SLOT, 1, 2 * TILE_SIDE, 0));
            assert!(land.slots().iter().all(|slot| !slot.is_base_page()));
            // The texture each slot draws is the base's.
            for texture_id in [LEFT_TEXTURE, OTHER_LEFT_TEXTURE] {
                let kr = resolved(land.texture_slot_ids()[&texture_id]);
                let ec = *base.present_slot(base.texture_slot_ids()[&texture_id]).unwrap();
                assert_eq!((land.base_page_index(kr.page_index), kr.x, kr.width), (Some(ec.page_index), ec.x, ec.width));
            }
        }

        #[test]
        fn a_kr_land_that_is_no_overlay_or_has_another_atlas_asks_for_a_repack() {
            let full = base_with_section(section_bytes(ATLAS_SIDE, None));
            let error = format!("{:#}", kr_land(&full).err().expect("refused"));
            assert!(error.contains("repack"), "{error}");

            let other_atlas = base_with_section(section_bytes(OTHER_ATLAS_SIDE, Some(&left())));
            let error = format!("{:#}", kr_land(&other_atlas).err().expect("refused"));
            assert!(error.contains("not the base package's") && error.contains("repack"), "{error}");
        }

        /// A reader that predates the base-page flag trips on the overlay's slot table magic instead of reading its
        /// slots as lying on pages it lacks; this one refuses either magic on the other layout's slots.
        #[test]
        fn a_slot_table_whose_magic_does_not_match_its_slots_asks_for_a_repack() {
            let refusal = |slot_table: Vec<u8>, pages: &[&[u8]]| {
                let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
                add_metadata_file(&mut builder, UDDP_PAGE_MANIFEST_ENTRY_VPATH, &page_manifest(ATLAS_SIDE, pages.len() as u32));
                add_metadata_file(&mut builder, UDDP_SLOT_MANIFEST_ENTRY_VPATH, &slot_table);
                add_metadata_file(&mut builder, UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH, &provenance(&[]));
                add_metadata_file(&mut builder, UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH, &terrain_overrides_json());
                add_metadata_file(&mut builder, UDDP_TRANSCODE_ENTRY_VPATH, EMPTY_TRANSCODE_KDL);
                add_metadata_file(&mut builder, UDDP_SOURCE_MATERIALS_ENTRY_VPATH, EMPTY_SOURCE_MATERIALS_JSON);
                let (path, bytes) = tex_land_ec_overlay_entry(&EcOverlayTable { equivalents: left().to_vec() });
                add_metadata_file(&mut builder, path, &bytes);
                let package = UddpReader::open(builder.build().unwrap()).unwrap();
                format!("{:#}", TexLandEcPackage::from_uddp_package(package).err().expect("refused"))
            };
            let on_base = [slot(LEFT_SLOT, 2, TILE_SIDE, SLOT_FLAG_BASE_PAGE)];
            let error = refusal(slot_manifest_with_magic(SLOT_MANIFEST_MAGIC, ATLAS_SIDE, &on_base), &[OWN_PAGE_BYTES]);
            assert!(error.contains("magic") && error.contains("repack"), "{error}");
            let own = [slot(OWN_SLOT, 0, 0, 0)];
            let error = refusal(slot_manifest_with_magic(OVERLAY_SLOT_MANIFEST_MAGIC, ATLAS_SIDE, &own), &[OWN_PAGE_BYTES]);
            assert!(error.contains("magic") && error.contains("repack"), "{error}");
        }

        #[test]
        fn a_package_without_a_section_has_no_kr_land() {
            let base = TexLandEcPackage::from_uddp_package(UddpReader::open(base_bytes()).unwrap()).unwrap();
            assert!(kr_land(&base).unwrap().is_none());
        }
    }
}
