use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;
use color_eyre::eyre::{self, WrapErr};
use byteorder::{LittleEndian, ReadBytesExt};
use udd_container::UddpReader;
use crate::common::read_path_entry;
use crate::tex_art_cc::PagePixelFormat;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"ELPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"ELSL";
const TERRAIN_PROVENANCE_MAGIC: [u8; 4] = *b"ELTP";
const TEX_LAND_EC_METADATA_VERSION: u32 = 2;
const TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION: u32 = 1;

pub const UDDP_PAGE_MANIFEST_ENTRY_VPATH: &str = "metadata/pages.bin";
pub const UDDP_SLOT_MANIFEST_ENTRY_VPATH: &str = "metadata/slots.bin";
pub const UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH: &str = "metadata/terrain_provenance.bin";
pub const UDDP_TRANSCODE_ENTRY_VPATH: &str = "metadata/transcode.bin";

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const SLOT_FLAG_LAND: u16 = 1 << 1;
pub const SLOT_FLAG_STATIC: u16 = 1 << 2;
pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;
pub const MISSING_TEXTURE_ID: u32 = u32::MAX;
pub const MISSING_SLOT_ID: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexLandEcTerrainProvenanceRecord {
    pub material_id: u32,
    pub material_name_id: i32,
    pub alias_count_index: u32,
    pub alias_slot_id: u32,
    pub alias_tile_flags: u64,
    pub selected_texture_id: u32,
    pub canonical_slot_id: u32,
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
    pages: Vec<TexLandEcPageRecord>,
    slots: Vec<TexLandEcSlotRecord>,
    terrain_provenance: Vec<TexLandEcTerrainProvenanceRecord>,
    pub transcode: HashMap<u32, u32>,
}

impl TexLandEcPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load(path.as_ref())
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn load_in_memory(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load_in_memory(path.as_ref())
            .wrap_err_with(|| format!("load_in_memory {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        let page_manifest = read_path_entry(&package, UDDP_PAGE_MANIFEST_ENTRY_VPATH)
            .context("tex_land_ec.uddp missing metadata/pages.bin")?;
        let slot_manifest = read_path_entry(&package, UDDP_SLOT_MANIFEST_ENTRY_VPATH)
            .context("tex_land_ec.uddp missing metadata/slots.bin")?;
        let terrain_provenance_manifest =
            read_path_entry(&package, UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH)
                .context("tex_land_ec.uddp missing metadata/terrain_provenance.bin")?;

        let (page_width, page_height, page_gutter, pages) = parse_page_manifest(&page_manifest)?;
        let (slot_width, slot_height, slot_gutter, slots) = parse_slot_manifest(&slot_manifest)?;
        let terrain_provenance = parse_terrain_provenance_manifest(&terrain_provenance_manifest)?;

        if (page_width, page_height, page_gutter) != (slot_width, slot_height, slot_gutter) {
            eyre::bail!("tex_land_ec metadata headers disagree on atlas dimensions or gutter");
        }

        let transcode = read_transcode_from_package(&package).unwrap_or_default();
        Ok(Self {
            package,
            atlas_width: page_width,
            atlas_height: page_height,
            gutter: page_gutter,
            pages,
            slots,
            terrain_provenance,
            transcode,
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

    pub fn pages(&self) -> &[TexLandEcPageRecord] {
        &self.pages
    }

    pub fn slots(&self) -> &[TexLandEcSlotRecord] {
        &self.slots
    }

    pub fn terrain_provenance(&self) -> &[TexLandEcTerrainProvenanceRecord] {
        &self.terrain_provenance
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

    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .get(page_index as usize)
            .map(|p| p.pixel_format)
            .unwrap_or(PagePixelFormat::Rgba8888);
        read_path_entry(&self.package, &crate::tex_art_cc::page_entry_path(page_index, fmt))
            .wrap_err_with(|| format!("unpack atlas page {page_index}"))
    }
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<TexLandEcPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != TEX_LAND_EC_METADATA_VERSION { eyre::bail!("invalid version"); }
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?).unwrap();
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
    Ok((w, h, g, pages))
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<TexLandEcSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SLOT_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != TEX_LAND_EC_METADATA_VERSION { eyre::bail!("invalid version"); }
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
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
    Ok((w, h, g, slots))
}

fn parse_terrain_provenance_manifest(bytes: &[u8]) -> eyre::Result<Vec<TexLandEcTerrainProvenanceRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != TERRAIN_PROVENANCE_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION { eyre::bail!("invalid version"); }
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
