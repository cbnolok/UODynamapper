use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, Context, ContextCompat};
use std::io::{Cursor, Read};
use std::path::Path;
use udd_container::{DataType, FileKey, UddpReader};

const GUMP_ATLAS_PAGE_MANIFEST_ID: u32 = 0xE000_0000;
const GUMP_ATLAS_SLOT_MANIFEST_ID: u32 = 0xE000_0001;
const GUMP_ATLAS_PAGE_ID_BASE: u32 = 0xF000_0000;
const GUMP_ATLAS_PAGE_MANIFEST_MAGIC: [u8; 4] = *b"GAPG";
const GUMP_ATLAS_SLOT_MANIFEST_MAGIC: [u8; 4] = *b"GASL";
const GUMP_ATLAS_METADATA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GumpAtlasPageRecord {
    pub page_index: u32,
    pub gump_count: u32,
    pub used_width: u32,
    pub used_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GumpAtlasSlotRecord {
    pub gump_id: u32,
    pub page_index: u32,
    pub page_gump_index: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub upscale_factor: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GumpImage {
    pub physical_width: u32,
    pub physical_height: u32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub upscale_factor: u16,
    pub rgba: Vec<u8>,
}

pub struct GumpsPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    atlas_gutter: u16,
    atlas_pages: Vec<GumpAtlasPageRecord>,
    atlas_slots: Vec<GumpAtlasSlotRecord>,
}

impl GumpsPackage {
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
        let (atlas_width, atlas_height, atlas_gutter, atlas_pages, atlas_slots) =
            match (
                package.find_by_sparse_id(GUMP_ATLAS_PAGE_MANIFEST_ID),
                package.find_by_sparse_id(GUMP_ATLAS_SLOT_MANIFEST_ID),
            ) {
                (Some(_), Some(_)) => {
                    let page_manifest = package
                        .read_file_by_sparse_id_cow(GUMP_ATLAS_PAGE_MANIFEST_ID)
                        .context("gumps package page manifest could not be read")?;
                    let slot_manifest = package
                        .read_file_by_sparse_id_cow(GUMP_ATLAS_SLOT_MANIFEST_ID)
                        .context("gumps package slot manifest could not be read")?;
                    let (page_width, page_height, page_gutter, pages) =
                        parse_page_manifest(&page_manifest)?;
                    let (slot_width, slot_height, slot_gutter, slots) =
                        parse_slot_manifest(&slot_manifest)?;
                    if (page_width, page_height, page_gutter)
                        != (slot_width, slot_height, slot_gutter)
                    {
                        eyre::bail!(
                            "gump atlas metadata headers disagree on dimensions or gutter"
                        );
                    }
                    (page_width, page_height, page_gutter, pages, slots)
                }
                _ => (0, 0, 0, Vec::new(), Vec::new()),
            };

        Ok(Self {
            package,
            atlas_width,
            atlas_height,
            atlas_gutter,
            atlas_pages,
            atlas_slots,
        })
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

    pub fn atlas_gutter(&self) -> u16 {
        self.atlas_gutter
    }

    pub fn atlas_pages(&self) -> &[GumpAtlasPageRecord] {
        &self.atlas_pages
    }

    pub fn atlas_slots(&self) -> &[GumpAtlasSlotRecord] {
        &self.atlas_slots
    }

    pub fn available_gump_ids(&self) -> Vec<u32> {
        let mut ids = self
            .package
            .records()
            .into_iter()
            .filter_map(|record| match record.key {
                FileKey::Id(id) => self
                    .package
                    .find_by_sparse_id(id)
                    .filter(|file| file.data_type == DataType::Gump as u8)
                    .map(|_| id),
                FileKey::PathHash(_) => None,
            })
            .collect::<Vec<_>>();

        ids.extend(self.atlas_slots.iter().map(|slot| slot.gump_id));
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    pub fn atlas_slot(&self, gump_id: u32) -> Option<&GumpAtlasSlotRecord> {
        self.atlas_slots
            .binary_search_by_key(&gump_id, |slot| slot.gump_id)
            .ok()
            .map(|index| &self.atlas_slots[index])
    }

    pub fn read_gump_rgba(&self, gump_id: u32) -> eyre::Result<(u32, u32, Vec<u8>)> {
        let image = self.read_gump_image(gump_id)?;
        Ok((image.physical_width, image.physical_height, image.rgba))
    }

    pub fn read_gump_image(&self, gump_id: u32) -> eyre::Result<GumpImage> {
        if let Some(file) = self.package.find_by_sparse_id(gump_id) {
            if file.data_type == DataType::Gump as u8 {
                return decode_single_gump_payload(&self.package.read_file_by_sparse_id(gump_id)?);
            }
        }

        let slot = self
            .atlas_slot(gump_id)
            .ok_or_else(|| eyre::eyre!("gump {gump_id} not found in package"))?;
        self.read_atlas_gump(slot)
    }

    fn read_atlas_gump(&self, slot: &GumpAtlasSlotRecord) -> eyre::Result<GumpImage> {
        let page = self
            .atlas_pages
            .get(slot.page_index as usize)
            .filter(|page| page.page_index == slot.page_index)
            .context("gump atlas slot references missing page metadata")?;
        let page_bytes = self
            .package
            .read_file_by_sparse_id(GUMP_ATLAS_PAGE_ID_BASE + slot.page_index)
            .wrap_err_with(|| format!("read gump atlas page {}", slot.page_index))?;
        let page_stride = page.used_width as usize * 4;
        let slot_width = slot.width as usize;
        let slot_height = slot.height as usize;
        let mut rgba = vec![0u8; slot_width * slot_height * 4];

        if u32::from(slot.x) + u32::from(slot.width) > page.used_width
            || u32::from(slot.y) + u32::from(slot.height) > page.used_height
        {
            eyre::bail!("gump atlas slot rectangle exceeds page used bounds");
        }

        for row in 0..slot_height {
            let src_start = ((slot.y as usize + row) * page_stride) + slot.x as usize * 4;
            let src_end = src_start + slot_width * 4;
            let dst_start = row * slot_width * 4;
            let src = page_bytes
                .get(src_start..src_end)
                .context("gump atlas page payload is shorter than metadata bounds")?;
            rgba[dst_start..dst_start + slot_width * 4].copy_from_slice(src);
        }

        let upscale_factor = slot.upscale_factor.max(1);
        Ok(GumpImage {
            physical_width: u32::from(slot.width),
            physical_height: u32::from(slot.height),
            logical_width: (u32::from(slot.width) / u32::from(upscale_factor)).max(1),
            logical_height: (u32::from(slot.height) / u32::from(upscale_factor)).max(1),
            upscale_factor,
            rgba,
        })
    }
}

fn decode_single_gump_payload(payload: &[u8]) -> eyre::Result<GumpImage> {
    let mut cursor = Cursor::new(payload);
    let width = cursor.read_u32::<LittleEndian>()?;
    let height = cursor.read_u32::<LittleEndian>()?;
    let expected_len = width as usize * height as usize * 4;
    let mut rgba = Vec::new();
    cursor.read_to_end(&mut rgba)?;
    if rgba.len() != expected_len {
        eyre::bail!(
            "invalid gump RGBA payload length for {width}x{height}: expected {expected_len}, got {}",
            rgba.len()
        );
    }
    Ok(GumpImage {
        physical_width: width,
        physical_height: height,
        logical_width: width,
        logical_height: height,
        upscale_factor: 1,
        rgba,
    })
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<GumpAtlasPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != GUMP_ATLAS_PAGE_MANIFEST_MAGIC {
        eyre::bail!("invalid gump atlas page manifest magic");
    }
    let (width, height, gutter, count) = parse_common_manifest_header(&mut cursor)?;
    let mut pages = Vec::with_capacity(count as usize);
    for _ in 0..count {
        pages.push(GumpAtlasPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            gump_count: cursor.read_u32::<LittleEndian>()?,
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
        });
    }
    Ok((width, height, gutter, pages))
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<GumpAtlasSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != GUMP_ATLAS_SLOT_MANIFEST_MAGIC {
        eyre::bail!("invalid gump atlas slot manifest magic");
    }
    let (width, height, gutter, count) = parse_common_manifest_header(&mut cursor)?;
    let mut slots = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let slot = GumpAtlasSlotRecord {
            gump_id: cursor.read_u32::<LittleEndian>()?,
            page_index: cursor.read_u32::<LittleEndian>()?,
            page_gump_index: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
            upscale_factor: cursor.read_u16::<LittleEndian>()?.max(1),
        };
        slots.push(slot);
    }
    slots.sort_by_key(|slot| slot.gump_id);
    Ok((width, height, gutter, slots))
}

fn parse_common_manifest_header<R: Read>(reader: &mut R) -> eyre::Result<(u32, u32, u16, u32)> {
    let version = reader.read_u32::<LittleEndian>()?;
    if version != GUMP_ATLAS_METADATA_VERSION {
        eyre::bail!("unsupported gump atlas metadata version {version}");
    }
    let width = reader.read_u32::<LittleEndian>()?;
    let height = reader.read_u32::<LittleEndian>()?;
    let gutter = reader.read_u32::<LittleEndian>()? as u16;
    let count = reader.read_u32::<LittleEndian>()?;
    Ok((width, height, gutter, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;
    use udd_container::{AddFileRequest, CompressionFlag, LookupMode, UddpBuilder};

    #[test]
    fn package_reads_single_gump_payload() {
        let package = GumpsPackage::from_uddp_package(build_test_package(false)).unwrap();

        let (width, height, rgba) = package.read_gump_rgba(42).unwrap();

        assert_eq!((width, height), (2, 1));
        assert_eq!(rgba, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn package_reads_atlas_gump_payload() {
        let package = GumpsPackage::from_uddp_package(build_test_package(true)).unwrap();

        let image = package.read_gump_image(50_001).unwrap();

        assert_eq!((image.physical_width, image.physical_height), (2, 1));
        assert_eq!((image.logical_width, image.logical_height), (1, 1));
        assert_eq!(image.upscale_factor, 2);
        assert_eq!(image.rgba, vec![9, 10, 11, 12, 13, 14, 15, 16]);
    }

    fn build_test_package(include_atlas: bool) -> UddpReader {
        let mut builder = UddpBuilder::new(LookupMode::SparseId);
        let single = single_payload(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]);
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Gump as u8,
                compression: CompressionFlag::None,
                width: 2,
                height: 1,
                virtual_path: None,
                path_hash64: None,
                id: Some(42),
                data: &single,
            })
            .unwrap();

        if include_atlas {
            let page_manifest = page_manifest();
            let slot_manifest = slot_manifest();
            let page_pixels = vec![
                0, 0, 0, 0, 9, 10, 11, 12, 13, 14, 15, 16,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ];
            for (id, data_type, data, width, height) in [
                (GUMP_ATLAS_PAGE_MANIFEST_ID, DataType::Metadata as u8, page_manifest.as_slice(), 0, 0),
                (GUMP_ATLAS_SLOT_MANIFEST_ID, DataType::Metadata as u8, slot_manifest.as_slice(), 0, 0),
                (GUMP_ATLAS_PAGE_ID_BASE, DataType::Texture as u8, page_pixels.as_slice(), 3, 2),
            ] {
                builder
                    .add_file(AddFileRequest {
                        data_type,
                        compression: CompressionFlag::None,
                        width,
                        height,
                        virtual_path: None,
                        path_hash64: None,
                        id: Some(id),
                        data,
                    })
                    .unwrap();
            }
        }

        UddpReader::open(builder.build().unwrap()).unwrap()
    }

    fn single_payload(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.write_u32::<LittleEndian>(width).unwrap();
        payload.write_u32::<LittleEndian>(height).unwrap();
        payload.extend_from_slice(rgba);
        payload
    }

    fn page_manifest() -> Vec<u8> {
        let mut bytes = common_manifest_header(&GUMP_ATLAS_PAGE_MANIFEST_MAGIC, 3, 2, 1, 1);
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(3).unwrap();
        bytes.write_u32::<LittleEndian>(2).unwrap();
        bytes
    }

    fn slot_manifest() -> Vec<u8> {
        let mut bytes = common_manifest_header(&GUMP_ATLAS_SLOT_MANIFEST_MAGIC, 3, 2, 1, 1);
        bytes.write_u32::<LittleEndian>(50_001).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_u16::<LittleEndian>(0).unwrap();
        bytes.write_u16::<LittleEndian>(1).unwrap();
        bytes.write_u16::<LittleEndian>(0).unwrap();
        bytes.write_u16::<LittleEndian>(2).unwrap();
        bytes.write_u16::<LittleEndian>(1).unwrap();
        bytes.write_u16::<LittleEndian>(2).unwrap();
        bytes
    }

    fn common_manifest_header(
        magic: &[u8; 4],
        width: u32,
        height: u32,
        gutter: u32,
        count: u32,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(magic);
        bytes.write_u32::<LittleEndian>(GUMP_ATLAS_METADATA_VERSION).unwrap();
        bytes.write_u32::<LittleEndian>(width).unwrap();
        bytes.write_u32::<LittleEndian>(height).unwrap();
        bytes.write_u32::<LittleEndian>(gutter).unwrap();
        bytes.write_u32::<LittleEndian>(count).unwrap();
        bytes
    }
}
