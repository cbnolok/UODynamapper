use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;
use color_eyre::eyre::{self, WrapErr};
use byteorder::{LittleEndian, ReadBytesExt};
use udd_container::{xxh64_virtual_path, UddpReader};
use crate::common::{AtlasCacheOptions, AtlasPageCache, decode_atlas_page_rgba, read_path_entry};
use crate::tex_art_cc::{AtlasPackingMode, PagePixelFormat};

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"ELPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"ELSL";
const TERRAIN_PROVENANCE_MAGIC: [u8; 4] = *b"ELTP";
const TEX_LAND_EC_METADATA_VERSION: u32 = 3;
const TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION: u32 = 2;

pub const UDDP_PAGE_MANIFEST_ENTRY_VPATH: &str = "metadata/pages.bin";
pub const UDDP_SLOT_MANIFEST_ENTRY_VPATH: &str = "metadata/slots.bin";
pub const UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH: &str = "metadata/terrain_provenance.bin";
pub const UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH: &str = "metadata/terrain_overrides.json";
pub const UDDP_TRANSCODE_ENTRY_VPATH: &str = "metadata/transcode.bin";

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const SLOT_FLAG_LAND: u16 = 1 << 1;
pub const SLOT_FLAG_STATIC: u16 = 1 << 2;
pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;
pub const MISSING_TEXTURE_ID: u32 = u32::MAX;
pub const MISSING_SLOT_ID: u32 = u32::MAX;
pub const MISSING_TERRAIN_LAYER_INDEX: u32 = u32::MAX;

pub const TERRAIN_PRIMARY_REASON_UNKNOWN: u8 = 0;
pub const TERRAIN_PRIMARY_REASON_NON_SUPPORT_PREFERRED_REPETITION: u8 = 1;
pub const TERRAIN_PRIMARY_REASON_NON_SUPPORT_REPETITION_FALLBACK: u8 = 2;
pub const TERRAIN_PRIMARY_REASON_SUPPORT_PREFERRED_REPETITION_FALLBACK: u8 = 3;
pub const TERRAIN_PRIMARY_REASON_SUPPORT_REPETITION_FALLBACK: u8 = 4;

pub const TERRAIN_PRIMARY_FLAG_SELECTED_CURRENT_SUPPORT: u16 = 1 << 0;
pub const TERRAIN_PRIMARY_FLAG_SELECTED_SUPPORT_LIKE: u16 = 1 << 1;
pub const TERRAIN_PRIMARY_FLAG_SELECTED_PREFERRED_REPETITION: u16 = 1 << 2;
pub const TERRAIN_PRIMARY_FLAG_FALLBACK_REASON: u16 = 1 << 3;
pub const TERRAIN_PRIMARY_FLAG_MULTIPLE_PREFERRED_NON_SUPPORT: u16 = 1 << 4;
pub const TERRAIN_PRIMARY_FLAG_SUPPORT_LIKE_OUTSIDE_CURRENT_HEURISTIC: u16 = 1 << 5;
pub const TERRAIN_PRIMARY_FLAG_OPAQUE_UNK6_TIEBREAKER: u16 = 1 << 6;

pub const TERRAIN_OVERRIDE_ACTION_POLICY: u16 = 1 << 0;
pub const TERRAIN_OVERRIDE_ACTION_LIQUID: u16 = 1 << 1;
pub const TERRAIN_OVERRIDE_ACTION_LAYER: u16 = 1 << 2;
pub const TERRAIN_OVERRIDE_ACTION_TEXTURE: u16 = 1 << 3;
pub const TERRAIN_OVERRIDE_ACTION_IGNORE: u16 = 1 << 4;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TexLandEcTerrainOverrideActions {
    pub material_id: u32,
    pub action_count: u32,
    pub action_flags: u16,
}

impl TexLandEcTerrainOverrideActions {
    pub fn has_policy(self) -> bool {
        (self.action_flags & TERRAIN_OVERRIDE_ACTION_POLICY) != 0
    }

    pub fn has_liquid(self) -> bool {
        (self.action_flags & TERRAIN_OVERRIDE_ACTION_LIQUID) != 0
    }

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
    pub primary_selection_reason: u8,
    pub primary_selection_flags: u16,
    pub override_actions: Option<TexLandEcTerrainOverrideActions>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TexLandEcTerrainOverrideTextureRef {
    pub material_id: u32,
    pub role: String,
    pub texture_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TexLandEcResolvedOverrideTexture {
    pub material_id: u32,
    pub role: String,
    pub texture_id: u32,
    pub runtime_slot_id: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TexLandEcTerrainProvenanceRecord {
    pub material_id: u32,
    pub material_name_id: i32,
    pub alias_count_index: u32,
    pub alias_slot_id: u32,
    pub alias_tile_flags: u64,
    pub selected_texture_id: u32,
    pub canonical_slot_id: u32,
    pub primary_texture_id: u32,
    pub primary_layer_index: u32,
    pub primary_selection_reason: u8,
    pub primary_selection_flags: u16,
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
    terrain_override_texture_refs: HashMap<u32, Vec<TexLandEcTerrainOverrideTextureRef>>,
    pub transcode: HashMap<u32, u32>,
    page_cache: AtlasPageCache,
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
        let page_manifest = read_path_entry(&package, UDDP_PAGE_MANIFEST_ENTRY_VPATH)
            .context("tex_land_ec.uddp missing metadata/pages.bin")?;
        let slot_manifest = read_path_entry(&package, UDDP_SLOT_MANIFEST_ENTRY_VPATH)
            .context("tex_land_ec.uddp missing metadata/slots.bin")?;
        let terrain_provenance_manifest =
            read_path_entry(&package, UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH)
                .context("tex_land_ec.uddp missing metadata/terrain_provenance.bin")?;

        let (page_width, page_height, page_gutter, page_packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let (slot_width, slot_height, slot_gutter, slot_packing_mode, slots) =
            parse_slot_manifest(&slot_manifest)?;
        let terrain_provenance = parse_terrain_provenance_manifest(&terrain_provenance_manifest)?;

        if (page_width, page_height, page_gutter) != (slot_width, slot_height, slot_gutter) {
            eyre::bail!("tex_land_ec metadata headers disagree on atlas dimensions or gutter");
        }
        if page_packing_mode != slot_packing_mode {
            eyre::bail!("tex_land_ec metadata headers disagree on atlas packing mode");
        }

        let transcode = read_transcode_from_package(&package).unwrap_or_default();
        let terrain_override_actions = read_terrain_override_actions_from_package(&package)
            .unwrap_or_default();
        let terrain_override_texture_refs =
            read_terrain_override_texture_refs_from_package(&package).unwrap_or_default();
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
            terrain_override_texture_refs,
            transcode,
            page_cache: AtlasPageCache::new(options),
        })
    }

    pub fn set_transcode(&mut self, transcode: HashMap<u32, u32>) {
        self.transcode = transcode;
    }

    pub fn package(&self) -> &UddpReader {
        &self.package
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
                runtime_slot_id: self
                    .terrain_provenance
                    .iter()
                    .find(|record| {
                        record.material_id == material_id
                            && record.selected_texture_id == texture_ref.texture_id
                    })
                    .and_then(|record| self.resolve_provenance_record_slot(record)),
            })
            .collect()
    }

    pub fn read_terrain_overrides_metadata(&self) -> eyre::Result<Option<Vec<u8>>> {
        if self
            .package
            .find_by_path_hash(xxh64_virtual_path(UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH))
            .is_none()
        {
            return Ok(None);
        }

        read_path_entry(&self.package, UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH).map(Some)
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
                return Some(slot_id);
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
                return Some(slot_id);
            }
        }

        for record in self
            .terrain_provenance
            .iter()
            .filter(|r| r.alias_slot_id == cc_tile_id)
        {
            if let Some(slot_id) = self.resolve_provenance_record_slot(record) {
                return Some(slot_id);
            }
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
        } else if let Some(slot_id) = material_records
            .iter()
            .find_map(|record| self.resolve_provenance_record_slot(record))
        {
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
            primary_selection_reason: primary_record
                .map(|record| record.primary_selection_reason)
                .unwrap_or(TERRAIN_PRIMARY_REASON_UNKNOWN),
            primary_selection_flags: primary_record
                .map(|record| record.primary_selection_flags)
                .unwrap_or(0),
            override_actions: material_id.and_then(|id| {
                self.terrain_override_actions.get(&id).copied()
            }),
        }
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

    fn resolve_material_runtime_slot(
        &self,
        material_records: &[&TexLandEcTerrainProvenanceRecord],
    ) -> (Option<u32>, Option<TexLandEcRuntimeSlotSource>) {
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

    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .get(page_index as usize)
            .map(|p| p.pixel_format)
            .unwrap_or(PagePixelFormat::Rgba8888);
        self.page_cache.read_page_bytes(page_index, || {
            read_path_entry(&self.package, &crate::tex_art_cc::page_entry_path(page_index, fmt))
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
    if version != TEX_LAND_EC_METADATA_VERSION {
        eyre::bail!("invalid version");
    }
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?).unwrap();
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?).unwrap();
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

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<TexLandEcSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SLOT_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != TEX_LAND_EC_METADATA_VERSION {
        eyre::bail!("invalid version");
    }
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?).unwrap();
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
    Ok((w, h, g, packing_mode, slots))
}

fn parse_terrain_provenance_manifest(bytes: &[u8]) -> eyre::Result<Vec<TexLandEcTerrainProvenanceRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != TERRAIN_PROVENANCE_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if !(1..=TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION).contains(&version) { eyre::bail!("invalid version"); }
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
            primary_texture_id: if version >= 2 {
                cursor.read_u32::<LittleEndian>()?
            } else {
                MISSING_TEXTURE_ID
            },
            primary_layer_index: if version >= 2 {
                cursor.read_u32::<LittleEndian>()?
            } else {
                MISSING_TERRAIN_LAYER_INDEX
            },
            primary_selection_reason: if version >= 2 {
                cursor.read_u8()?
            } else {
                TERRAIN_PRIMARY_REASON_UNKNOWN
            },
            primary_selection_flags: if version >= 2 {
                cursor.read_u16::<LittleEndian>()?
            } else {
                0
            },
        });
    }
    Ok(records)
}

fn read_transcode_from_package(package: &UddpReader) -> Option<HashMap<u32, u32>> {
    let bytes = read_path_entry(package, UDDP_TRANSCODE_ENTRY_VPATH).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    // Minimal KDL-like parser for simple (cc_id, material_id) pairs
    let mut transcode = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with("#") {
            continue;
        }
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() >= 2 {
            if let (Ok(cc_id), Ok(mat_id)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                transcode.insert(cc_id, mat_id);
            }
        }
    }
    Some(transcode)
}

fn read_terrain_override_actions_from_package(
    package: &UddpReader,
) -> Option<HashMap<u32, TexLandEcTerrainOverrideActions>> {
    let bytes = read_path_entry(package, UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH).ok()?;
    parse_terrain_override_actions_metadata(&bytes).ok()
}

fn read_terrain_override_texture_refs_from_package(
    package: &UddpReader,
) -> Option<HashMap<u32, Vec<TexLandEcTerrainOverrideTextureRef>>> {
    let bytes = read_path_entry(package, UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH).ok()?;
    parse_terrain_override_texture_refs_metadata(&bytes).ok()
}

fn parse_terrain_override_actions_metadata(
    bytes: &[u8],
) -> eyre::Result<HashMap<u32, TexLandEcTerrainOverrideActions>> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes)?;
    let mut actions = HashMap::new();
    let Some(entries) = value.get("entries").and_then(|value| value.as_array()) else {
        return Ok(actions);
    };

    for entry in entries {
        let Some(material_id) = entry
            .get("material_id")
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok())
        else {
            continue;
        };
        let action_count = entry
            .get("active_action_count")
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        let mut action_flags = 0u16;
        if json_array_is_non_empty(entry, "policies") {
            action_flags |= TERRAIN_OVERRIDE_ACTION_POLICY;
        }
        if !entry.get("liquid").unwrap_or(&serde_json::Value::Null).is_null() {
            action_flags |= TERRAIN_OVERRIDE_ACTION_LIQUID;
        }
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
    }

    Ok(actions)
}

fn parse_terrain_override_texture_refs_metadata(
    bytes: &[u8],
) -> eyre::Result<HashMap<u32, Vec<TexLandEcTerrainOverrideTextureRef>>> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes)?;
    let mut refs_by_material = HashMap::<u32, Vec<TexLandEcTerrainOverrideTextureRef>>::new();
    let Some(entries) = value.get("entries").and_then(|value| value.as_array()) else {
        return Ok(refs_by_material);
    };

    for entry in entries {
        let Some(material_id) = entry
            .get("material_id")
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok())
        else {
            continue;
        };
        if let Some(layers) = entry.get("layers").and_then(|value| value.as_array()) {
            for layer in layers {
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
                    });
            }
        }
    }

    Ok(refs_by_material)
}

fn json_u32(value: &serde_json::Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
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

fn optional_texture_sort_key(value: u32, missing: u32) -> u32 {
    if value == missing {
        u32::MAX
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;
    use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

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
        bytes.write_u32::<LittleEndian>(101).unwrap();
        for art_id in 0..=100u32 {
            bytes.write_u32::<LittleEndian>(art_id).unwrap();
            if art_id == 77 || art_id == 100 {
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
        bytes.write_u32::<LittleEndian>(2).unwrap();
        for texture_id in [2000520, 2000510] {
            bytes.write_u32::<LittleEndian>(52).unwrap();
            bytes.write_i32::<LittleEndian>(0).unwrap();
            bytes.write_u32::<LittleEndian>(0).unwrap();
            bytes.write_u32::<LittleEndian>(77).unwrap();
            bytes.write_u64::<LittleEndian>(0).unwrap();
            bytes.write_u32::<LittleEndian>(texture_id).unwrap();
            bytes.write_u32::<LittleEndian>(100).unwrap();
            bytes.write_u32::<LittleEndian>(2000520).unwrap();
            bytes.write_u32::<LittleEndian>(0).unwrap();
            bytes.write_u8(TERRAIN_PRIMARY_REASON_NON_SUPPORT_PREFERRED_REPETITION).unwrap();
            bytes.write_u16::<LittleEndian>(TERRAIN_PRIMARY_FLAG_SELECTED_PREFERRED_REPETITION).unwrap();
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
      "active_action_count": 2,
      "policies": [],
      "liquid": null,
      "layers": [{"role": "t0", "texture_id": 2000510}],
      "textures": [],
      "ignore": null
    }
  ]
}"#
        .to_vec()
    }

    fn test_package() -> TexLandEcPackage {
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
        let bytes = builder.build().expect("build test package");
        TexLandEcPackage::from_uddp_package(UddpReader::open(bytes).expect("open package"))
            .expect("load test package")
    }

    #[test]
    fn terrain_provenance_parser_accepts_v1_records_with_unknown_primary_fields() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&TERRAIN_PROVENANCE_MAGIC);
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(12).unwrap();
        bytes.write_i32::<LittleEndian>(34).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_u32::<LittleEndian>(56).unwrap();
        bytes.write_u64::<LittleEndian>(0x55).unwrap();
        bytes.write_u32::<LittleEndian>(2_000_540).unwrap();
        bytes.write_u32::<LittleEndian>(56).unwrap();

        let records = parse_terrain_provenance_manifest(&bytes).expect("parse v1 provenance");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].material_id, 12);
        assert_eq!(records[0].selected_texture_id, 2_000_540);
        assert_eq!(records[0].primary_texture_id, MISSING_TEXTURE_ID);
        assert_eq!(records[0].primary_layer_index, MISSING_TERRAIN_LAYER_INDEX);
        assert_eq!(records[0].primary_selection_reason, TERRAIN_PRIMARY_REASON_UNKNOWN);
        assert_eq!(records[0].primary_selection_flags, 0);
    }

    #[test]
    fn terrain_override_metadata_parser_records_action_flags() {
        let actions = parse_terrain_override_actions_metadata(&terrain_overrides_json())
            .expect("parse override actions");
        let action = actions.get(&52).expect("material 52 override");

        assert_eq!(action.action_count, 2);
        assert!(action.has_layer());
        assert!(!action.has_policy());
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
        assert_eq!(decision.primary_selection_reason, TERRAIN_PRIMARY_REASON_NON_SUPPORT_PREFERRED_REPETITION);
        assert_eq!(
            decision.override_actions.map(|actions| actions.action_flags),
            Some(TERRAIN_OVERRIDE_ACTION_LAYER)
        );
        assert_eq!(override_slots.len(), 1);
        assert_eq!(override_slots[0].role, "t0");
        assert_eq!(override_slots[0].texture_id, 2000510);
        assert_eq!(override_slots[0].runtime_slot_id, Some(100));
    }
}
