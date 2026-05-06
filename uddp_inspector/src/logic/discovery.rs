use std::collections::HashMap;
use std::io::{Cursor, Read};
use byteorder::{LittleEndian, ReadBytesExt};
use uocf::udd::uddp::xxh64_virtual_path; 

use crate::models::{AtlasPageInfo, AtlasPixelFormat, VirtualEntry, VirtualEntryData};

pub fn parse_atlas_page_manifest(data: &[u8]) -> HashMap<u32, AtlasPageInfo> {
    if data.len() < 25 {
        return HashMap::new();
    }

    let mut cursor = Cursor::new(data);
    let mut magic = [0u8; 4];
    if cursor.read_exact(&mut magic).is_err() {
        return HashMap::new();
    }

    if !matches!(&magic, b"CAPG" | b"EAPG" | b"ELPG") {
        return HashMap::new();
    }

    let _version = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let atlas_width = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let atlas_height = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _gutter = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let pixel_format = match cursor.read_u8().ok() {
        Some(0) => AtlasPixelFormat::Rgba8888,
        Some(1) => AtlasPixelFormat::Bc7,
        _ => return HashMap::new(),
    };
    let page_count = cursor.read_u32::<LittleEndian>().unwrap_or(0);

    let mut pages = HashMap::new();
    for _ in 0..page_count {
        let Ok(page_index) = cursor.read_u32::<LittleEndian>() else {
            break;
        };
        let Ok(_tile_count) = cursor.read_u32::<LittleEndian>() else {
            break;
        };
        let Ok(used_width) = cursor.read_u32::<LittleEndian>() else {
            break;
        };
        let Ok(used_height) = cursor.read_u32::<LittleEndian>() else {
            break;
        };
        pages.insert(
            page_index,
            AtlasPageInfo {
                atlas_width,
                atlas_height,
                used_width,
                used_height,
                pixel_format,
            },
        );
    }

    pages
}

pub fn slot_manifest_kind(magic: &[u8; 4]) -> Option<&'static str> {
    match magic {
        b"CASL" => Some("CC Art"),
        b"EASL" => Some("EC Art"),
        b"ELSL" => Some("EC Land"),
        _ => None,
    }
}

pub fn parse_virtual_entries_from_slot_manifest(data: &[u8]) -> Vec<VirtualEntry> {
    if data.len() < 24 {
        return Vec::new();
    }

    let mut cursor = Cursor::new(data);
    let mut magic = [0u8; 4];
    if cursor.read_exact(&mut magic).is_err() {
        return Vec::new();
    }

    let Some(kind) = slot_manifest_kind(&magic) else {
        return Vec::new();
    };

    let _version = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _atlas_w = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _atlas_h = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _gutter = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let count = cursor.read_u32::<LittleEndian>().unwrap_or(0);

    let mut entries = Vec::new();
    for _ in 0..count {
        let Ok(id) = cursor.read_u32::<LittleEndian>() else {
            break;
        };
        let Ok(page_index) = cursor.read_u32::<LittleEndian>() else {
            break;
        };
        let Ok(_page_tile_idx) = cursor.read_u16::<LittleEndian>() else {
            break;
        };
        let Ok(flags) = cursor.read_u16::<LittleEndian>() else {
            break;
        };
        let Ok(x) = cursor.read_u16::<LittleEndian>() else {
            break;
        };
        let Ok(y) = cursor.read_u16::<LittleEndian>() else {
            break;
        };
        let Ok(width) = cursor.read_u16::<LittleEndian>() else {
            break;
        };
        let Ok(height) = cursor.read_u16::<LittleEndian>() else {
            break;
        };

        if (flags & 1) != 0 {
            entries.push(VirtualEntry {
                id,
                data_type: if kind == "EC Land" { 9 } else { 1 },
                kind: kind.to_string(),
                summary: format!("{}x{} at {},{}", width, height, x, y),
                location: format!("page {}", page_index),
                data: VirtualEntryData::AtlasRect {
                    page_index,
                    x,
                    y,
                    width,
                    height,
                    flags,
                },
            });
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};

    fn build_slot_manifest(magic: &[u8; 4], flags: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(magic);
        bytes.write_u32::<LittleEndian>(2).unwrap();
        bytes.write_u32::<LittleEndian>(2048).unwrap();
        bytes.write_u32::<LittleEndian>(2048).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(42).unwrap();
        bytes.write_u32::<LittleEndian>(7).unwrap();
        bytes.write_u16::<LittleEndian>(3).unwrap();
        bytes.write_u16::<LittleEndian>(flags).unwrap();
        bytes.write_u16::<LittleEndian>(11).unwrap();
        bytes.write_u16::<LittleEndian>(22).unwrap();
        bytes.write_u16::<LittleEndian>(33).unwrap();
        bytes.write_u16::<LittleEndian>(44).unwrap();
        bytes
    }

    fn build_page_manifest(
        magic: &[u8; 4],
        pixel_format: u8,
        used_width: u32,
        used_height: u32,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(magic);
        bytes.write_u32::<LittleEndian>(2).unwrap();
        bytes.write_u32::<LittleEndian>(4096).unwrap();
        bytes.write_u32::<LittleEndian>(2048).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u8(pixel_format).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(7).unwrap();
        bytes.write_u32::<LittleEndian>(3).unwrap();
        bytes.write_u32::<LittleEndian>(used_width).unwrap();
        bytes.write_u32::<LittleEndian>(used_height).unwrap();
        bytes
    }

    #[test]
    fn slot_manifest_kind_recognizes_current_package_types() {
        assert_eq!(slot_manifest_kind(b"CASL"), Some("CC Art"));
        assert_eq!(slot_manifest_kind(b"EASL"), Some("EC Art"));
        assert_eq!(slot_manifest_kind(b"ELSL"), Some("EC Land"));
        assert_eq!(slot_manifest_kind(b"NOPE"), None);
    }

    #[test]
    fn parse_virtual_entries_accepts_ec_land_manifests() {
        let manifest = build_slot_manifest(b"ELSL", 1);
        let entries = parse_virtual_entries_from_slot_manifest(&manifest);

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.id, 42);
        assert_eq!(entry.kind, "EC Land");
        match &entry.data {
            VirtualEntryData::AtlasRect {
                page_index,
                x,
                y,
                width,
                height,
                ..
            } => {
                assert_eq!(*page_index, 7);
                assert_eq!(*x, 11);
                assert_eq!(*y, 22);
                assert_eq!(*width, 33);
                assert_eq!(*height, 44);
            }
            other => panic!("expected atlas rect, got {other:?}"),
        }
    }

    #[test]
    fn parse_virtual_entries_skips_non_present_slots() {
        let manifest = build_slot_manifest(b"CASL", 0);
        let entries = parse_virtual_entries_from_slot_manifest(&manifest);

        assert!(entries.is_empty());
    }

    #[test]
    fn parse_atlas_page_manifest_reads_shared_dimensions_and_format() {
        let manifest = build_page_manifest(b"EAPG", 1, 3000, 1500);
        let pages = parse_atlas_page_manifest(&manifest);
        let info = pages.get(&7).unwrap();

        assert_eq!(info.atlas_width, 4096);
        assert_eq!(info.atlas_height, 2048);
        assert_eq!(info.used_width, 3000);
        assert_eq!(info.used_height, 1500);
        assert_eq!(info.pixel_format, AtlasPixelFormat::Bc7);
    }
}
