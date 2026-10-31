use std::collections::HashMap;
use color_eyre::eyre;
use udd_assets::tex_art_cc::{MISSING_PAGE_INDEX, PagePixelFormat, SLOT_MANIFEST_ENTRY_PATH};
use udd_assets::{TexArtCcPackage, TexArtEcPackage, TexLandCcPackage, TexLandEcPackage};
use udd_container::{DataType, UddpReader, xxh64_virtual_path};

use crate::models::{AtlasPageInfo, AtlasPixelFormat, VirtualEntry, VirtualEntryData};

const KIND_CC_ART: &str = "CC Art";
const KIND_EC_ART: &str = "EC Art";
const KIND_EC_LAND: &str = "EC Land";
const KIND_CC_TEXMAPS: &str = "CC Texmaps";
const SLOT_MANIFEST_MAGIC_LEN: usize = 4;
/// Upscale factor and algorithm shown for land slots, which store neither.
const NOT_UPSCALED_FACTOR: u16 = 1;
const NOT_UPSCALED_ALGORITHM: u16 = 0;

pub fn slot_manifest_kind(magic: &[u8; 4]) -> Option<&'static str> {
    match magic {
        b"CASL" => Some(KIND_CC_ART),
        b"EASL" => Some(KIND_EC_ART),
        b"ELSL" => Some(KIND_EC_LAND),
        b"CTXS" => Some(KIND_CC_TEXMAPS),
        _ => None,
    }
}

/// An atlas package (art or land, Classic or Enhanced) read with its typed udd-assets reader.
pub enum AtlasPackage {
    CcArt(TexArtCcPackage),
    EcArt(TexArtEcPackage),
    EcLand(TexLandEcPackage),
    CcTexmaps(TexLandCcPackage),
}

/// Open `reader` as the atlas package its slot manifest names.
///
/// `None` when the package has no slot manifest of an atlas package; an error when it is an
/// atlas package the reader rejects, such as one packed in an older format version.
pub fn load_atlas_package(reader: &UddpReader) -> Option<eyre::Result<AtlasPackage>> {
    let slot_manifest = reader.read_file_by_path_hash(xxh64_virtual_path(SLOT_MANIFEST_ENTRY_PATH)).ok()?;
    let magic: [u8; SLOT_MANIFEST_MAGIC_LEN] = slot_manifest.get(..SLOT_MANIFEST_MAGIC_LEN)?.try_into().ok()?;
    let reader = reader.clone();
    Some(match slot_manifest_kind(&magic)? {
        KIND_CC_ART => TexArtCcPackage::from_uddp_package(reader).map(AtlasPackage::CcArt),
        KIND_EC_ART => TexArtEcPackage::from_uddp_package(reader).map(AtlasPackage::EcArt),
        KIND_EC_LAND => TexLandEcPackage::from_uddp_package(reader).map(AtlasPackage::EcLand),
        _ => TexLandCcPackage::from_uddp_package(reader).map(AtlasPackage::CcTexmaps),
    })
}

impl AtlasPackage {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::CcArt(_) => KIND_CC_ART,
            Self::EcArt(_) => KIND_EC_ART,
            Self::EcLand(_) => KIND_EC_LAND,
            Self::CcTexmaps(_) => KIND_CC_TEXMAPS,
        }
    }

    pub fn page_infos(&self) -> HashMap<u32, AtlasPageInfo> {
        macro_rules! pages {
            ($package:expr) => {{
                let (atlas_width, atlas_height) = ($package.atlas_width(), $package.atlas_height());
                $package
                    .pages()
                    .iter()
                    .map(|page| {
                        let info = AtlasPageInfo {
                            atlas_width,
                            atlas_height,
                            used_width: page.used_width,
                            used_height: page.used_height,
                            pixel_format: atlas_pixel_format(page.pixel_format),
                        };
                        (page.page_index, info)
                    })
                    .collect()
            }};
        }
        match self {
            Self::CcArt(package) => pages!(package),
            Self::EcArt(package) => pages!(package),
            Self::EcLand(package) => pages!(package),
            Self::CcTexmaps(package) => pages!(package),
        }
    }

    /// One entry per present slot.
    pub fn virtual_entries(&self) -> Vec<VirtualEntry> {
        let kind = self.kind();
        let slots: Vec<AtlasSlot> = match self {
            Self::CcArt(package) => package
                .slots()
                .iter()
                .filter(|slot| slot.is_present())
                .map(|slot| AtlasSlot {
                    id: slot.art_id,
                    page_index: slot.page_index,
                    page_tile_index: slot.page_tile_index,
                    flags: slot.flags,
                    x: slot.x,
                    y: slot.y,
                    width: slot.width,
                    height: slot.height,
                    upscale: Some((slot.upscale_factor.max(NOT_UPSCALED_FACTOR), slot.upscale_algorithm)),
                })
                .collect(),
            Self::EcArt(package) => package
                .slots()
                .iter()
                .filter(|slot| slot.is_present())
                .map(|slot| AtlasSlot {
                    id: slot.art_id,
                    page_index: slot.page_index,
                    page_tile_index: slot.page_tile_index,
                    flags: slot.flags,
                    x: slot.x,
                    y: slot.y,
                    width: slot.width,
                    height: slot.height,
                    upscale: Some((slot.upscale_factor.max(NOT_UPSCALED_FACTOR), slot.upscale_algorithm)),
                })
                .collect(),
            Self::EcLand(package) => package
                .slots()
                .iter()
                .filter(|slot| slot.is_present())
                .map(|slot| AtlasSlot {
                    id: slot.art_id,
                    page_index: slot.page_index,
                    page_tile_index: slot.page_tile_index,
                    flags: slot.flags,
                    x: slot.x,
                    y: slot.y,
                    width: slot.width,
                    height: slot.height,
                    upscale: None,
                })
                .collect(),
            Self::CcTexmaps(package) => package
                .slots()
                .iter()
                .filter(|slot| slot.is_present())
                .map(|slot| AtlasSlot {
                    id: slot.id,
                    page_index: slot.page_index,
                    page_tile_index: slot.page_tile_index,
                    flags: slot.flags,
                    x: slot.x,
                    y: slot.y,
                    width: slot.width,
                    height: slot.height,
                    upscale: None,
                })
                .collect(),
        };
        let data_type = match self {
            Self::CcArt(_) | Self::EcArt(_) => DataType::Art,
            Self::EcLand(_) | Self::CcTexmaps(_) => DataType::Texture,
        };
        slots.into_iter().map(|slot| atlas_virtual_entry(kind, data_type, slot)).collect()
    }
}

/// A present slot of any atlas package; `upscale` is `(factor, algorithm)` for art slots.
#[derive(Debug, Clone, Copy)]
struct AtlasSlot {
    id: u32,
    page_index: u32,
    page_tile_index: u16,
    flags: u16,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    upscale: Option<(u16, u16)>,
}

fn atlas_pixel_format(format: PagePixelFormat) -> AtlasPixelFormat {
    match format {
        PagePixelFormat::Rgba8888 => AtlasPixelFormat::Rgba8888,
        PagePixelFormat::Bc7 => AtlasPixelFormat::Bc7,
    }
}

fn atlas_virtual_entry(kind: &str, data_type: DataType, slot: AtlasSlot) -> VirtualEntry {
    let AtlasSlot { id, page_index, page_tile_index, flags, x, y, width, height, upscale } = slot;
    let (upscale_factor, upscale_algorithm) = upscale.unwrap_or((NOT_UPSCALED_FACTOR, NOT_UPSCALED_ALGORITHM));
    let is_empty = page_index == MISSING_PAGE_INDEX || width == 0 || height == 0;
    let summary = if is_empty {
        "empty".to_string()
    } else if upscale.is_some() {
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
    let location = if page_index == MISSING_PAGE_INDEX {
        "missing page".to_string()
    } else {
        format!("page {}", page_index)
    };
    VirtualEntry {
        id,
        _data_type: data_type as u8,
        kind: kind.to_string(),
        summary,
        location,
        data: VirtualEntryData::AtlasRect {
            page_index,
            page_tile_idx: page_tile_index,
            x,
            y,
            width,
            height,
            flags,
            upscale_factor,
            upscale_algorithm,
        },
    }
}

pub(crate) fn upscale_algorithm_name(code: u16) -> &'static str {
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
        23 => "OmniScale",
        24 => "Jinc2",
        25 => "Jinc2Sharp",
        26 => "Jinc2Sharper",
        27 => "Jinc2Sharpest",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(width: u16, upscale: Option<(u16, u16)>) -> AtlasSlot {
        AtlasSlot {
            id: 42,
            page_index: 7,
            page_tile_index: 3,
            flags: 1,
            x: 11,
            y: 22,
            width,
            height: 44,
            upscale,
        }
    }

    #[test]
    fn slot_manifest_kind_recognizes_current_package_types() {
        assert_eq!(slot_manifest_kind(b"CASL"), Some(KIND_CC_ART));
        assert_eq!(slot_manifest_kind(b"EASL"), Some(KIND_EC_ART));
        assert_eq!(slot_manifest_kind(b"ELSL"), Some(KIND_EC_LAND));
        assert_eq!(slot_manifest_kind(b"CTXS"), Some(KIND_CC_TEXMAPS));
        assert_eq!(slot_manifest_kind(b"NOPE"), None);
    }

    #[test]
    fn atlas_entries_report_slot_rects() {
        let entry = atlas_virtual_entry(KIND_EC_LAND, DataType::Texture, slot(33, None));

        assert_eq!(entry.id, 42);
        assert_eq!(entry.kind, KIND_EC_LAND);
        assert_eq!(entry.summary, "33x44 at 11,22");
        assert_eq!(entry.location, "page 7");
        match &entry.data {
            VirtualEntryData::AtlasRect { page_index, x, y, width, height, upscale_factor, .. } => {
                assert_eq!((*page_index, *x, *y, *width, *height), (7, 11, 22, 33, 44));
                assert_eq!(*upscale_factor, NOT_UPSCALED_FACTOR);
            }
            other => panic!("expected atlas rect, got {other:?}"),
        }
    }

    #[test]
    fn atlas_entries_keep_present_slots_without_displayable_rects() {
        let entry = atlas_virtual_entry(KIND_EC_LAND, DataType::Texture, slot(0, None));

        assert_eq!(entry.summary, "empty");
    }

    #[test]
    fn atlas_entries_report_art_upscale_metadata() {
        let fsr_easu = 6;
        let entry = atlas_virtual_entry(KIND_EC_ART, DataType::Art, slot(33, Some((2, fsr_easu))));

        assert!(entry.summary.contains("logical 16.5x22.0"));
        assert!(entry.summary.contains("upscale FsrEasu 2x"));
    }
}
