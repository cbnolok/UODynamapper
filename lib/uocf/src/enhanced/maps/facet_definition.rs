//! Enhanced Client facet definitions (`facets.uop`): per facet, its name, size and the radar regions (names,
//! background colours, rectangles).
//!
//! Layout traced in `scripts/dev-helpers/ec-research/ghidra-facets/results/uosa_facets.md`
//! (`FacetDefinition_Read` 0x0083c933, `_ReadRegions` 0x0083ca5f, `_ReadRegionRects` 0x0083cb53). Entry
//! `build/facetdefinition/%08d.bin`, ids 0..6 (asset type 0x17), little endian, no padding:
//!
//! ```text
//! u8  facet id
//! u32 name cliloc
//! u32 width, u32 height        (squares)
//! u8  unknown[3]               (0x40 0x40 0x01 in all 7; stored but never read by the client)
//! u32 region count, then per region:
//!     u32 index                (equals the region's position in all 7; stored, never read)
//!     u32 name cliloc          (region 0 is 1112058, the whole facet)
//!     u8  r, g, b              (radar background of the region)
//!     u8  physical facet       (0xff: this facet; else the facet whose coordinates the rects use)
//!     u32 rect count, then rect count x {u32 x1, y1, x2, y2}, inclusive
//! ```
//!
//! [`FacetDefinition::to_bytes`] is the exact inverse of [`FacetDefinition::from_bytes`] (byte-identical on
//! all 7 shipped records); the unknown bytes and the region index are kept verbatim.

use std::io::{Cursor, Read};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{NewEntryCompression, UopPackage};

crate::eyre_imports!();

/// Entry name of facet `facet`'s definition.
pub fn facet_definition_entry_path(facet: u32) -> String {
    format!("build/facetdefinition/{facet:08}.bin")
}

/// The package holding the facet definitions.
pub const FACET_DEFINITIONS_UOP_NAME: &str = "facets.uop";
/// The facet ids the client requests (0..6; 6 is the Test facet).
pub const CLIENT_FACET_DEFINITION_IDS: std::ops::RangeInclusive<u32> = 0..=6;
/// [`FacetRegion::physical_facet`] meaning "this facet".
pub const REGION_SAME_FACET: u8 = 0xff;
/// Bytes after the height that the client stores and never reads.
pub const FACET_DEFINITION_UNKNOWN_SIZE: usize = 3;
/// Compression of a definition entry added to a package (an existing entry keeps its own).
pub const NEW_FACET_DEFINITION_COMPRESSION: CompressionFlag = CompressionFlag::None;

/// Reads facet `facet`'s definition from a `facets.uop` package; `None` when it has no such entry.
pub fn read_facet_definition(package: &UopPackage, facet: u32) -> eyre::Result<Option<FacetDefinition>> {
    let entry = facet_definition_entry_path(facet);
    match package.unpack_file_by_hash(hash_file_name_single(&entry))? {
        Some(bytes) => Ok(Some(FacetDefinition::from_bytes(&bytes).wrap_err_with(|| format!("decode {entry}"))?)),
        None => Ok(None),
    }
}

/// Writes `definition` into a `facets.uop` package as the entry of its facet id, keeping the compression of the
/// entry it replaces.
pub fn write_facet_definition(package: &mut UopPackage, definition: &FacetDefinition) -> eyre::Result<()> {
    let hash = hash_file_name_single(&facet_definition_entry_path(u32::from(definition.facet_id)));
    let payload = definition.to_bytes()?;
    match package.get_file_by_hash(hash).map(|file| file.compression()) {
        Some(compression) => {
            package.replace_file(hash, &payload, compression)?;
        }
        None => package.add_entry(hash, &payload, NewEntryCompression::Flag(NEW_FACET_DEFINITION_COMPRESSION))?,
    }
    Ok(())
}

/// An inclusive rectangle of squares: `x1 <= x <= x2`, `y1 <= y <= y2` (`Radar_IsLocationInArea`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FacetRect {
    pub x1: u32,
    pub y1: u32,
    pub x2: u32,
    pub y2: u32,
}

/// A radar region (area) of a facet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetRegion {
    /// Stored at object +0 and never read by the client; equals the region's position in every shipped record.
    pub index: u32,
    /// Area name cliloc (`UORadarGetAreaLabel`).
    pub name_cliloc: u32,
    /// Radar background colour of the region (`Radar_GetAreaColor` -> `FacetDds_CopyRegionBlocks`).
    pub rgb: [u8; 3],
    /// [`REGION_SAME_FACET`], or the facet whose map the rects are in (`Radar_SetView` switches to it).
    pub physical_facet: u8,
    pub rects: Vec<FacetRect>,
}

/// One `facets.uop` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetDefinition {
    pub facet_id: u8,
    /// Facet name cliloc (`UORadarGetFacetLabel`).
    pub name_cliloc: u32,
    /// Size in squares (`UORadarGetFacetDimensions`, used to copy the radar image's blocks).
    pub width: u32,
    pub height: u32,
    /// Stored at object +0x14..+0x16 (getter slots 7..9) and never read by the client; 0x40 0x40 0x01 in all 7
    /// shipped records (perhaps the sector side twice and a version).
    pub unknown: [u8; FACET_DEFINITION_UNKNOWN_SIZE],
    pub regions: Vec<FacetRegion>,
}

/// A record the client would read, but that looks wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FacetDefinitionIssue {
    /// `x1 > x2` or `y1 > y2`: the rect matches nothing (facet 2 region 5 ships one).
    InvertedRect { region: usize, rect: usize },
    /// The region index differs from its position (never the case in shipped records; the client ignores it).
    RegionIndex { region: usize, index: u32 },
}

impl FacetDefinition {
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let facet_id = cursor.read_u8()?;
        let name_cliloc = cursor.read_u32::<LittleEndian>()?;
        let width = cursor.read_u32::<LittleEndian>()?;
        let height = cursor.read_u32::<LittleEndian>()?;
        let mut unknown = [0u8; FACET_DEFINITION_UNKNOWN_SIZE];
        cursor.read_exact(&mut unknown)?;
        let region_count = cursor.read_u32::<LittleEndian>()?;
        let mut regions = Vec::new();
        for region in 0..region_count {
            let index = cursor.read_u32::<LittleEndian>().wrap_err_with(|| format!("region {region}"))?;
            let name_cliloc = cursor.read_u32::<LittleEndian>()?;
            let mut rgb = [0u8; 3];
            cursor.read_exact(&mut rgb)?;
            let physical_facet = cursor.read_u8()?;
            let rect_count = cursor.read_u32::<LittleEndian>()?;
            let mut rects = Vec::new();
            for _ in 0..rect_count {
                rects.push(FacetRect {
                    x1: cursor.read_u32::<LittleEndian>()?,
                    y1: cursor.read_u32::<LittleEndian>()?,
                    x2: cursor.read_u32::<LittleEndian>()?,
                    y2: cursor.read_u32::<LittleEndian>()?,
                });
            }
            regions.push(FacetRegion { index, name_cliloc, rgb, physical_facet, rects });
        }
        let left = bytes.len() as u64 - cursor.position();
        if left != 0 {
            eyre::bail!("{left} bytes after the last region");
        }
        Ok(Self { facet_id, name_cliloc, width, height, unknown, regions })
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut out = Vec::new();
        out.write_u8(self.facet_id)?;
        out.write_u32::<LittleEndian>(self.name_cliloc)?;
        out.write_u32::<LittleEndian>(self.width)?;
        out.write_u32::<LittleEndian>(self.height)?;
        out.extend_from_slice(&self.unknown);
        out.write_u32::<LittleEndian>(u32::try_from(self.regions.len())?)?;
        for region in &self.regions {
            out.write_u32::<LittleEndian>(region.index)?;
            out.write_u32::<LittleEndian>(region.name_cliloc)?;
            out.extend_from_slice(&region.rgb);
            out.write_u8(region.physical_facet)?;
            out.write_u32::<LittleEndian>(u32::try_from(region.rects.len())?)?;
            for rect in &region.rects {
                for value in [rect.x1, rect.y1, rect.x2, rect.y2] {
                    out.write_u32::<LittleEndian>(value)?;
                }
            }
        }
        Ok(out)
    }

    /// Appends a region with the next index.
    pub fn push_region(&mut self, name_cliloc: u32, rgb: [u8; 3], physical_facet: u8, rects: Vec<FacetRect>) -> usize {
        let index = self.regions.len();
        self.regions.push(FacetRegion { index: index as u32, name_cliloc, rgb, physical_facet, rects });
        index
    }

    /// Removes region `position` and renumbers the indexes of the later regions to their positions.
    pub fn remove_region(&mut self, position: usize) -> eyre::Result<FacetRegion> {
        if position >= self.regions.len() {
            eyre::bail!("facet {} has {} regions, no region {position}", self.facet_id, self.regions.len());
        }
        let removed = self.regions.remove(position);
        for (at, region) in self.regions.iter_mut().enumerate().skip(position) {
            region.index = at as u32;
        }
        Ok(removed)
    }

    pub fn validate(&self) -> Vec<FacetDefinitionIssue> {
        let mut issues = Vec::new();
        for (position, region) in self.regions.iter().enumerate() {
            if region.index as usize != position {
                issues.push(FacetDefinitionIssue::RegionIndex { region: position, index: region.index });
            }
            for (rect, bounds) in region.rects.iter().enumerate() {
                if bounds.x1 > bounds.x2 || bounds.y1 > bounds.y2 {
                    issues.push(FacetDefinitionIssue::InvertedRect { region: position, rect });
                }
            }
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_validates() {
        let mut definition = FacetDefinition {
            facet_id: 6,
            name_cliloc: 1072851,
            width: 256,
            height: 256,
            unknown: [0x40, 0x40, 0x01],
            regions: Vec::new(),
        };
        definition.push_region(1112058, [0, 56, 80], REGION_SAME_FACET, vec![FacetRect { x1: 0, y1: 0, x2: 255, y2: 255 }]);
        definition.push_region(1, [1, 2, 3], 0, vec![FacetRect { x1: 9, y1: 0, x2: 4, y2: 1 }]);
        let bytes = definition.to_bytes().unwrap();
        assert_eq!(FacetDefinition::from_bytes(&bytes).unwrap(), definition);
        assert_eq!(definition.validate(), vec![FacetDefinitionIssue::InvertedRect { region: 1, rect: 0 }]);
        definition.remove_region(0).unwrap();
        assert_eq!(definition.regions[0].index, 0);
        let mut trailing = bytes;
        trailing.push(0);
        assert!(FacetDefinition::from_bytes(&trailing).is_err());
    }
}
