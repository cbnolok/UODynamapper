use std::collections::HashMap;
use std::io::{Cursor, Read};
use byteorder::{LittleEndian, ReadBytesExt};
// use udd_container::xxh64_virtual_path;

use crate::models::{AtlasPageInfo, AtlasPixelFormat, VirtualEntry, VirtualEntryData};

const PACKING_MODE_HEADER_VERSION: u32 = 3;

pub fn parse_atlas_page_manifest(data: &[u8]) -> HashMap<u32, AtlasPageInfo> {
    if data.len() < 25 {
        return HashMap::new();
    }

    let mut cursor = Cursor::new(data);
    let mut magic = [0u8; 4];
    if cursor.read_exact(&mut magic).is_err() {
        return HashMap::new();
    }

    if !matches!(&magic, b"CAPG" | b"EAPG" | b"ELPG" | b"CTXP") {
        return HashMap::new();
    }

    let version = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let atlas_width = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let atlas_height = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _gutter = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let pixel_format = match cursor.read_u8().ok() {
        Some(0) => AtlasPixelFormat::Rgba8888,
        Some(1) => AtlasPixelFormat::Bc7,
        _ => return HashMap::new(),
    };
    if version >= PACKING_MODE_HEADER_VERSION || magic == *b"CTXP" {
        match cursor.read_u8().ok() {
            Some(0 | 1) => {}
            _ => return HashMap::new(),
        }
    }
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
        b"CTXS" => Some("CC Texmaps"),
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

    let version = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _atlas_w = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _atlas_h = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _gutter = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    if version >= PACKING_MODE_HEADER_VERSION || magic == *b"CTXS" {
        match cursor.read_u8().ok() {
            Some(0 | 1) => {}
            _ => return Vec::new(),
        }
    }
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
        let (upscale_factor, upscale_algorithm) = if matches!(kind, "CC Art" | "EC Art") && version >= 4 {
            let Ok(upscale_factor) = cursor.read_u16::<LittleEndian>() else {
                break;
            };
            let upscale_algorithm = if version >= 5 {
                let Ok(upscale_algorithm) = cursor.read_u16::<LittleEndian>() else {
                    break;
                };
                upscale_algorithm
            } else {
                0
            };
            (upscale_factor.max(1), upscale_algorithm)
        } else {
            (1, 0)
        };

        if (flags & 1) != 0 {
            let summary = if matches!(kind, "CC Art" | "EC Art") {
                let logical_width = width as f32 / f32::from(upscale_factor);
                let logical_height = height as f32 / f32::from(upscale_factor);
                format!(
                    "{}x{} at {},{}; logical {:.1}x{:.1}; upscale {} {}x",
                    width,
                    height,
                    x,
                    y,
                    logical_width,
                    logical_height,
                    upscale_algorithm_name(upscale_algorithm),
                    upscale_factor
                )
            } else {
                format!("{}x{} at {},{}", width, height, x, y)
            };
            entries.push(VirtualEntry {
                id,
                _data_type: if kind == "EC Land" || kind == "CC Texmaps" { 9 } else { 1 },
                kind: kind.to_string(),
                summary,
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

fn upscale_algorithm_name(code: u16) -> &'static str {
    match code {
        0 => "None",
        1 => "Nearest",
        2 => "Bilinear",
        3 => "CatmullRom",
        4 => "Lanczos3",
        5 => "SuperSai",
        6 => "FsrEasu",
        7 => "FsrEasuRcas",
        8 => "Depixelize",
        9 => "Nedi",
        10 => "TwoSai",
        11 => "SuperEagle",
        12 => "Lq",
        13 => "Hq",
        14 => "HqTrue",
        15 => "Epx",
        16 => "Xbr",
        17 => "Mmpx",
        18 => "SuperXbr",
        19 => "Cut1",
        20 => "Cut2",
        21 => "Cut3",
        22 => "ScaleFx",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};

    fn build_slot_manifest(magic: &[u8; 4], version: u32, packing_mode: Option<u8>, flags: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(magic);
        bytes.write_u32::<LittleEndian>(version).unwrap();
        bytes.write_u32::<LittleEndian>(2048).unwrap();
        bytes.write_u32::<LittleEndian>(2048).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        if let Some(packing_mode) = packing_mode {
            bytes.write_u8(packing_mode).unwrap();
        }
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(42).unwrap();
        bytes.write_u32::<LittleEndian>(7).unwrap();
        bytes.write_u16::<LittleEndian>(3).unwrap();
        bytes.write_u16::<LittleEndian>(flags).unwrap();
        bytes.write_u16::<LittleEndian>(11).unwrap();
        bytes.write_u16::<LittleEndian>(22).unwrap();
        bytes.write_u16::<LittleEndian>(33).unwrap();
        bytes.write_u16::<LittleEndian>(44).unwrap();
        if matches!(magic, b"CASL" | b"EASL") && version >= 4 {
            bytes.write_u16::<LittleEndian>(2).unwrap();
            if version >= 5 {
                bytes.write_u16::<LittleEndian>(6).unwrap();
            }
        }
        bytes
    }

    fn build_page_manifest(
        magic: &[u8; 4],
        version: u32,
        pixel_format: u8,
        packing_mode: Option<u8>,
        used_width: u32,
        used_height: u32,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(magic);
        bytes.write_u32::<LittleEndian>(version).unwrap();
        bytes.write_u32::<LittleEndian>(4096).unwrap();
        bytes.write_u32::<LittleEndian>(2048).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u8(pixel_format).unwrap();
        if let Some(packing_mode) = packing_mode {
            bytes.write_u8(packing_mode).unwrap();
        }
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
        assert_eq!(slot_manifest_kind(b"CTXS"), Some("CC Texmaps"));
        assert_eq!(slot_manifest_kind(b"NOPE"), None);
    }

    #[test]
    fn parse_virtual_entries_accepts_tex_land_ec_manifests() {
        let manifest = build_slot_manifest(b"ELSL", 3, Some(1), 1);
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
    fn parse_virtual_entries_accepts_tex_land_cc_v2_manifests() {
        let manifest = build_slot_manifest(b"CTXS", 2, Some(1), 1);
        let entries = parse_virtual_entries_from_slot_manifest(&manifest);

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.id, 42);
        assert_eq!(entry.kind, "CC Texmaps");
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
        let manifest = build_slot_manifest(b"CASL", 3, Some(1), 0);
        let entries = parse_virtual_entries_from_slot_manifest(&manifest);

        assert!(entries.is_empty());
    }

    #[test]
    fn parse_virtual_entries_reports_art_upscale_metadata() {
        let manifest = build_slot_manifest(b"EASL", 5, Some(1), 1);
        let entries = parse_virtual_entries_from_slot_manifest(&manifest);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, "EC Art");
        assert!(entries[0].summary.contains("logical 16.5x22.0"));
        assert!(entries[0].summary.contains("upscale FsrEasu 2x"));
    }

    #[test]
    fn parse_atlas_page_manifest_reads_shared_dimensions_and_format() {
        let manifest = build_page_manifest(b"EAPG", 3, 1, Some(1), 3000, 1500);
        let pages = parse_atlas_page_manifest(&manifest);
        let info = pages.get(&7).unwrap();

        assert_eq!(info.atlas_width, 4096);
        assert_eq!(info.atlas_height, 2048);
        assert_eq!(info.used_width, 3000);
        assert_eq!(info.used_height, 1500);
        assert_eq!(info.pixel_format, AtlasPixelFormat::Bc7);
    }

    #[test]
    fn parse_atlas_page_manifest_accepts_tex_land_cc_v2_manifest() {
        let manifest = build_page_manifest(b"CTXP", 2, 0, Some(1), 3000, 1500);
        let pages = parse_atlas_page_manifest(&manifest);
        let info = pages.get(&7).unwrap();

        assert_eq!(info.atlas_width, 4096);
        assert_eq!(info.atlas_height, 2048);
        assert_eq!(info.used_width, 3000);
        assert_eq!(info.used_height, 1500);
        assert_eq!(info.pixel_format, AtlasPixelFormat::Rgba8888);
    }

    #[test]
    fn parse_virtual_entries_accepts_legacy_slot_manifests_without_packing_mode() {
        let manifest = build_slot_manifest(b"CASL", 2, None, 1);
        let entries = parse_virtual_entries_from_slot_manifest(&manifest);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, "CC Art");
    }

    #[test]
    fn parse_atlas_page_manifest_accepts_legacy_headers_without_packing_mode() {
        let manifest = build_page_manifest(b"CAPG", 2, 0, None, 512, 256);
        let pages = parse_atlas_page_manifest(&manifest);
        let info = pages.get(&7).unwrap();

        assert_eq!(info.atlas_width, 4096);
        assert_eq!(info.atlas_height, 2048);
        assert_eq!(info.used_width, 512);
        assert_eq!(info.used_height, 256);
        assert_eq!(info.pixel_format, AtlasPixelFormat::Rgba8888);
    }
}
