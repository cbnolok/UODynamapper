//! Enhanced Client facet sectors (`facetN.uop`): facet geometry, sector numbering and entry names, the sector
//! encoder (the exact inverse of [`decode_facet_bin_raw`](crate::enhanced::facet_decoder::decode_facet_bin_raw)),
//! the client's sector constraints and the shipped delimiter rule.
//!
//! The client rules are traced in
//! `scripts/dev-helpers/ec-research/ghidra-facets/results/uosa_facets.md`:
//!
//! - Sector `s` of facet `f` is the entry `build/sectors/facet_%02d/%08d.bin` (`f`, `s`). Sectors are 64x64
//!   squares numbered column-major, `s = ceil(height / 64) * (x >> 6) + (y >> 6)`. A facet whose size is not a
//!   multiple of 64 gets a partial last row and column, padded to 64x64 with `z 0, land 0` squares that hold no
//!   statics and no delimiters.
//! - The payload is `u8 facet, u16 sector id`, then 4096 squares (`x` outer, `y` inner), each
//!   `i8 z, u16 land id, u8 delimiter count, {u8 direction, i8 z, u32 land id}, u8 static count,
//!   {u32 graphic, i8 z, u32 hue}`.
//! - The client rejects a whole sector with a delimiter direction above 7, a read past the end or bytes left
//!   after the last square; a header whose facet or sector id differs from the requested one leaves a hole.
//! - Delimiters as shipped: only on a sector's edge squares, one per direction leaving the sector (an
//!   orthogonal direction on an edge, plus the diagonal on a corner), holding the neighbour's land z and id,
//!   none for a neighbour outside the map, in ascending direction order.
//! - The `build/sectors/facet_%02d/data.map` entry holds `u32 height, u32 width`, uncompressed. The client never
//!   reads it (it takes the sizes from a hard-coded table, [`CLIENT_FACET_DIMENSIONS`]), but shipped packages
//!   and offline tools have it.
//!
//! The Kingdom Reborn facets (`ClientDialect::KingdomReborn`) follow the same rules (numbering, header, padding,
//! delimiter rule and order, `data.map`; checked on every shipped sector), with 3 more bytes per square after the
//! land id ([`KrSquareLand`]); their padding squares are all zero bytes.

use std::path::{Path, PathBuf};

use byteorder::{LittleEndian, WriteBytesExt};

use crate::enhanced::client_dialect::ClientDialect;
use crate::map_edition::MapEdition;
use crate::utils::path::find_file_ignoring_case;
use crate::enhanced::facet_decoder::{DecodedEcFacet, EcFacetDelimiter, EcFacetTile, KrSquareLand};

crate::eyre_imports!();

/// Squares along each side of a sector.
pub const SECTOR_SIDE: u32 = 64;
/// `log2(SECTOR_SIDE)`: the shift from a square coordinate to a sector coordinate.
pub const SECTOR_SIDE_SHIFT: u32 = 6;
/// Squares in a sector.
pub const SECTOR_SQUARE_COUNT: usize = (SECTOR_SIDE * SECTOR_SIDE) as usize;
/// The most delimiters or statics a square can hold (u8 counts).
pub const MAX_SQUARE_RECORDS: usize = u8::MAX as usize;
/// Valid delimiter directions are `0..DELIMITER_DIRECTION_COUNT`; the client rejects a sector with any other.
pub const DELIMITER_DIRECTION_COUNT: u8 = 8;
/// The neighbour offset `(dx, dy)` of each delimiter direction (`Sector_ReadDelimiter` table).
pub const DELIMITER_OFFSETS: [(i32, i32); DELIMITER_DIRECTION_COUNT as usize] =
    [(-1, 0), (-1, -1), (0, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (1, -1)];
/// Land id of the padding squares outside the map in a partial sector.
pub const PADDING_LAND_ID: u16 = 0;
/// Land z of the padding squares outside the map in a partial sector.
pub const PADDING_LAND_Z: i8 = 0;
/// Bytes of the `data.map` payload: u32 height, u32 width.
pub const DATA_MAP_PAYLOAD_SIZE: usize = 2 * size_of::<u32>();
/// Bytes of the sector header: u8 facet, u16 sector id.
pub const SECTOR_HEADER_SIZE: usize = size_of::<u8>() + size_of::<u16>();
/// Bytes of a square without delimiters and statics: i8 z, u16 land id, u8 delimiter count, u8 static count.
const SQUARE_MIN_SIZE: usize = 5;
/// Bytes of a delimiter record: u8 direction, i8 z, u32 land id.
const DELIMITER_RECORD_SIZE: usize = 6;
/// Bytes of a static record: u32 graphic, i8 z, u32 hue.
const STATIC_RECORD_SIZE: usize = 9;
/// Bytes a Kingdom Reborn square adds after the land id: u8 `subtype`, u16 `classic_land`.
const KR_SQUARE_LAND_SIZE: usize = size_of::<u8>() + size_of::<u16>();
/// The Kingdom Reborn bytes of the padding squares outside the map (all zero in the shipped facets).
pub const PADDING_KR_SQUARE_LAND: KrSquareLand = KrSquareLand { subtype: 0, classic_land: PADDING_LAND_ID };

/// The client's facet sizes (`UOFacet::g_FacetDimensions`, 0x00da72a8): Felucca, Trammel, Ilshenar, Malas,
/// Tokuno, TerMur, Test.
pub const CLIENT_FACET_DIMENSIONS: [FacetGeometry; 7] = [
    FacetGeometry::new(7168, 4096),
    FacetGeometry::new(7168, 4096),
    FacetGeometry::new(2304, 1600),
    FacetGeometry::new(2560, 2048),
    FacetGeometry::new(1448, 1448),
    FacetGeometry::new(1280, 4096),
    FacetGeometry::new(256, 256),
];

/// The facets that ship a variant package `facetNx.uop`, mounted instead of `facetN.uop` by the character-list
/// flag 0x8000 (`UOFacet::SwapFacetPackageVariants`). A variant package uses the base facet's entry names and
/// sector ids.
pub const VARIANT_PACKAGE_FACETS: [u8; 4] = [0, 1, 2, 5];

/// The size of a facet in squares and its sector grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FacetGeometry {
    pub width: u32,
    pub height: u32,
}

impl FacetGeometry {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// The client's size for `facet`, from [`CLIENT_FACET_DIMENSIONS`].
    pub fn client_facet(facet: u8) -> Option<Self> {
        CLIENT_FACET_DIMENSIONS.get(usize::from(facet)).copied()
    }

    /// Sector columns, `ceil(width / 64)`.
    pub fn sector_columns(&self) -> u32 {
        self.width.div_ceil(SECTOR_SIDE)
    }

    /// Sector rows, `ceil(height / 64)`.
    pub fn sector_rows(&self) -> u32 {
        self.height.div_ceil(SECTOR_SIDE)
    }

    pub fn sector_count(&self) -> u32 {
        self.sector_columns() * self.sector_rows()
    }

    /// Whether the square `(x, y)` is on the map (not padding).
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height
    }

    /// The square `(x + dx, y + dy)` when it is on the map.
    pub fn offset(&self, x: u32, y: u32, dx: i32, dy: i32) -> Option<(u32, u32)> {
        let nx = x.checked_add_signed(dx)?;
        let ny = y.checked_add_signed(dy)?;
        self.contains(nx, ny).then_some((nx, ny))
    }

    /// The sector holding the square `(x, y)`: column-major, `ceil(height / 64) * (x >> 6) + (y >> 6)`
    /// (`UOFacet::SectorIndexFromXY`).
    pub fn sector_of(&self, x: u32, y: u32) -> u32 {
        self.sector_rows() * (x >> SECTOR_SIDE_SHIFT) + (y >> SECTOR_SIDE_SHIFT)
    }

    /// The map coordinates of square (0, 0) of `sector`.
    pub fn sector_origin(&self, sector: u32) -> (u32, u32) {
        let rows = self.sector_rows();
        ((sector / rows) * SECTOR_SIDE, (sector % rows) * SECTOR_SIDE)
    }

    /// The `data.map` payload: u32 height, u32 width (little endian).
    pub fn data_map_payload(&self) -> [u8; DATA_MAP_PAYLOAD_SIZE] {
        let mut payload = [0u8; DATA_MAP_PAYLOAD_SIZE];
        payload[..size_of::<u32>()].copy_from_slice(&self.height.to_le_bytes());
        payload[size_of::<u32>()..].copy_from_slice(&self.width.to_le_bytes());
        payload
    }

    /// Reads a `data.map` payload.
    pub fn from_data_map_payload(payload: &[u8]) -> eyre::Result<Self> {
        if payload.len() != DATA_MAP_PAYLOAD_SIZE {
            eyre::bail!("data.map has {} bytes, expected {DATA_MAP_PAYLOAD_SIZE}", payload.len());
        }
        let (height, width) = payload.split_at(size_of::<u32>());
        let height = u32::from_le_bytes(height.try_into().expect("split at u32 size"));
        let width = u32::from_le_bytes(width.try_into().expect("split at u32 size"));
        if width == 0 || height == 0 {
            eyre::bail!("data.map holds an empty facet ({width}x{height})");
        }
        Ok(Self { width, height })
    }
}

/// The index of the square `(local_x, local_y)` in a sector's square list (`x * 64 + y`).
pub fn square_index(local_x: u32, local_y: u32) -> usize {
    (local_x * SECTOR_SIDE + local_y) as usize
}

/// The sector-local coordinates of a map square.
pub fn local_coords(x: u32, y: u32) -> (u32, u32) {
    (x % SECTOR_SIDE, y % SECTOR_SIDE)
}

/// The entry name of a sector: `build/sectors/facet_%02d/%08d.bin`.
pub fn sector_entry_path(facet: u8, sector: u32) -> String {
    format!("build/sectors/facet_{facet:02}/{sector:08}.bin")
}

/// The entry name of the facet header: `build/sectors/facet_%02d/data.map`.
pub fn data_map_entry_path(facet: u8) -> String {
    format!("build/sectors/facet_{facet:02}/data.map")
}

/// The package file name: `facetN.uop`, or `facetNx.uop` for a variant package.
pub fn facet_package_file_name(facet: u8, variant: bool) -> String {
    format!("facet{facet}{}.uop", if variant { "x" } else { "" })
}

/// The package of facet `facet` in `edition` in `dir` (the name matched ignoring ASCII case): `facetN.uop`, or for
/// the variant of a facet of [`VARIANT_PACKAGE_FACETS`] `facetNx.uop`. The other facets have no variant package and
/// read `facetN.uop` in both editions, as the client does. Fails naming the file when it is missing: the variant
/// never falls back to the standard package.
pub fn facet_package_path(dir: &Path, facet: u8, edition: MapEdition) -> eyre::Result<PathBuf> {
    let variant = edition.is_variant() && VARIANT_PACKAGE_FACETS.contains(&facet);
    let name = facet_package_file_name(facet, variant);
    find_file_ignoring_case(dir, &name).ok_or_else(|| match variant {
        true => eyre!(
            "Missing the variant edition of facet {facet}: no {name} in '{}'. The variant is never read from {}; \
             ask for the standard edition to read it.",
            dir.display(),
            facet_package_file_name(facet, false)
        ),
        false => eyre!("Missing facet {facet}: no {name} in '{}'", dir.display()),
    })
}

/// A sector the client would not use, as it would read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EcSectorIssue {
    /// The sector does not hold 4096 squares (an encoder input error; the client would overrun or leave bytes).
    SquareCount(usize),
    /// A delimiter direction above 7: the client rejects the whole sector.
    DelimiterDirection { square: usize, direction: u8 },
    /// More than 255 delimiters on a square (u8 count).
    TooManyDelimiters { square: usize, count: usize },
    /// More than 255 statics on a square (u8 count).
    TooManyStatics { square: usize, count: usize },
    /// The header facet or sector id differs from the requested one: the client leaves a hole.
    HeaderMismatch { facet: u8, sector_id: u16, expected_facet: u8, expected_sector: u32 },
    /// A square without the Kingdom Reborn bytes in a Kingdom Reborn sector, or with them in an Enhanced Client
    /// one (an encoder input error).
    SquareLayout { square: usize, dialect: ClientDialect },
}

impl std::fmt::Display for EcSectorIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SquareCount(count) => write!(f, "{count} squares, expected {SECTOR_SQUARE_COUNT}"),
            Self::DelimiterDirection { square, direction } => write!(
                f,
                "square {square}: delimiter direction {direction} is above {}; the client rejects the sector",
                DELIMITER_DIRECTION_COUNT - 1
            ),
            Self::TooManyDelimiters { square, count } => {
                write!(f, "square {square}: {count} delimiters, at most {MAX_SQUARE_RECORDS}")
            }
            Self::TooManyStatics { square, count } => {
                write!(f, "square {square}: {count} statics, at most {MAX_SQUARE_RECORDS}")
            }
            Self::HeaderMismatch { facet, sector_id, expected_facet, expected_sector } => write!(
                f,
                "header facet {facet} sector {sector_id}, expected facet {expected_facet} sector {expected_sector}; \
                 the client leaves a hole"
            ),
            Self::SquareLayout { square, dialect } => write!(
                f,
                "square {square}: {} the Kingdom Reborn square bytes in a {} sector",
                if dialect.is_kingdom_reborn() { "lacks" } else { "has" },
                dialect.label()
            ),
        }
    }
}

/// The issues that keep the client from using `sector` as facet `facet`, sector `sector_id`
/// (`None` skips the header check).
pub fn validate_ec_sector(sector: &DecodedEcFacet, expected: Option<(u8, u32)>) -> Vec<EcSectorIssue> {
    validate_sector(sector, expected, ClientDialect::Enhanced)
}

/// [`validate_ec_sector`] for a sector in the square layout of `dialect`.
pub fn validate_sector(
    sector: &DecodedEcFacet,
    expected: Option<(u8, u32)>,
    dialect: ClientDialect,
) -> Vec<EcSectorIssue> {
    let mut issues = Vec::new();
    if sector.tiles.len() != SECTOR_SQUARE_COUNT {
        issues.push(EcSectorIssue::SquareCount(sector.tiles.len()));
    }
    for (square, tile) in sector.tiles.iter().enumerate() {
        if tile.kr.is_some() != dialect.is_kingdom_reborn() {
            issues.push(EcSectorIssue::SquareLayout { square, dialect });
        }
        if tile.delimiters.len() > MAX_SQUARE_RECORDS {
            issues.push(EcSectorIssue::TooManyDelimiters { square, count: tile.delimiters.len() });
        }
        if tile.statics.len() > MAX_SQUARE_RECORDS {
            issues.push(EcSectorIssue::TooManyStatics { square, count: tile.statics.len() });
        }
        for delimiter in &tile.delimiters {
            if delimiter.direction >= DELIMITER_DIRECTION_COUNT {
                issues.push(EcSectorIssue::DelimiterDirection { square, direction: delimiter.direction });
            }
        }
    }
    if let Some((expected_facet, expected_sector)) = expected {
        if sector.facet_id != expected_facet || u32::from(sector.file_id) != expected_sector {
            issues.push(EcSectorIssue::HeaderMismatch {
                facet: sector.facet_id,
                sector_id: sector.file_id,
                expected_facet,
                expected_sector,
            });
        }
    }
    issues
}

/// Encodes a sector payload, the exact inverse of `decode_facet_bin_raw` (byte-identical on every shipped
/// sector). Fails on anything [`validate_ec_sector`] reports, instead of writing a sector the client rejects
/// (a u8 count would wrap above 255).
pub fn encode_ec_sector(sector: &DecodedEcFacet) -> eyre::Result<Vec<u8>> {
    encode_sector(sector, ClientDialect::Enhanced)
}

/// Encodes a sector payload in the square layout of `dialect`, the exact inverse of
/// [`decode_sector`](crate::enhanced::facet_decoder::decode_sector) (byte-identical on every shipped EC and KR
/// sector). Fails on anything [`validate_sector`] reports.
pub fn encode_sector(sector: &DecodedEcFacet, dialect: ClientDialect) -> eyre::Result<Vec<u8>> {
    if let Some(issue) = validate_sector(sector, None, dialect).first() {
        eyre::bail!("facet {} sector {}: {issue}", sector.facet_id, sector.file_id);
    }
    let square_land_size = if dialect.is_kingdom_reborn() { KR_SQUARE_LAND_SIZE } else { 0 };
    let size = SECTOR_HEADER_SIZE
        + sector
            .tiles
            .iter()
            .map(|tile| {
                SQUARE_MIN_SIZE
                    + square_land_size
                    + tile.delimiters.len() * DELIMITER_RECORD_SIZE
                    + tile.statics.len() * STATIC_RECORD_SIZE
            })
            .sum::<usize>();
    let mut out = Vec::with_capacity(size);
    out.write_u8(sector.facet_id)?;
    out.write_u16::<LittleEndian>(sector.file_id)?;
    for tile in &sector.tiles {
        out.write_i8(tile.z)?;
        out.write_u16::<LittleEndian>(tile.land_graphic)?;
        if let Some(kr) = tile.kr {
            out.write_u8(kr.subtype)?;
            out.write_u16::<LittleEndian>(kr.classic_land)?;
        }
        out.write_u8(tile.delimiters.len() as u8)?;
        for delimiter in &tile.delimiters {
            out.write_u8(delimiter.direction)?;
            out.write_i8(delimiter.z)?;
            out.write_u32::<LittleEndian>(delimiter.graphic_id)?;
        }
        out.write_u8(tile.statics.len() as u8)?;
        for item in &tile.statics {
            out.write_u32::<LittleEndian>(item.graphic_id)?;
            out.write_i8(item.z)?;
            out.write_u32::<LittleEndian>(item.hue)?;
        }
    }
    debug_assert_eq!(out.len(), size);
    Ok(out)
}

/// A sector of padding squares (`z 0, land 0`, no delimiters, no statics), to fill in.
pub fn empty_ec_sector(facet: u8, sector: u32) -> eyre::Result<DecodedEcFacet> {
    empty_sector(facet, sector, ClientDialect::Enhanced)
}

/// [`empty_ec_sector`] in the square layout of `dialect` (Kingdom Reborn squares get
/// [`PADDING_KR_SQUARE_LAND`]).
pub fn empty_sector(facet: u8, sector: u32, dialect: ClientDialect) -> eyre::Result<DecodedEcFacet> {
    let file_id = u16::try_from(sector).map_err(|_| eyre!("sector id {sector} does not fit the u16 header"))?;
    Ok(DecodedEcFacet {
        facet_id: facet,
        file_id,
        tiles: vec![
            EcFacetTile {
                z: PADDING_LAND_Z,
                land_graphic: PADDING_LAND_ID,
                kr: dialect.is_kingdom_reborn().then_some(PADDING_KR_SQUARE_LAND),
                delimiters: Vec::new(),
                statics: Vec::new(),
            };
            SECTOR_SQUARE_COUNT
        ],
    })
}

/// The delimiter directions the square `(x, y)` gets as shipped, with the neighbour each one points at: a
/// direction is kept when every non-zero component of its offset leaves the sector (so the diagonals only on
/// corners) and the neighbour is on the map. Padding squares get none.
pub fn delimiter_targets(geometry: &FacetGeometry, x: u32, y: u32) -> Vec<(u8, u32, u32)> {
    if !geometry.contains(x, y) {
        return Vec::new();
    }
    let (local_x, local_y) = local_coords(x, y);
    let leaves = |local: u32, delta: i32| {
        delta == 0 || local.checked_add_signed(delta).is_none_or(|moved| moved >= SECTOR_SIDE)
    };
    let mut targets = Vec::new();
    for (direction, &(dx, dy)) in DELIMITER_OFFSETS.iter().enumerate() {
        if !leaves(local_x, dx) || !leaves(local_y, dy) {
            continue;
        }
        if let Some((nx, ny)) = geometry.offset(x, y, dx, dy) {
            targets.push((direction as u8, nx, ny));
        }
    }
    targets
}

/// The shipped delimiter list of the square `(x, y)`; `land_at(nx, ny)` gives a neighbour's land `(z, id)`.
pub fn expected_delimiters(
    geometry: &FacetGeometry,
    x: u32,
    y: u32,
    mut land_at: impl FnMut(u32, u32) -> (i8, u16),
) -> Vec<EcFacetDelimiter> {
    delimiter_targets(geometry, x, y)
        .into_iter()
        .map(|(direction, nx, ny)| {
            let (z, land) = land_at(nx, ny);
            EcFacetDelimiter { direction, z, graphic_id: u32::from(land) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::facet_decoder::{decode_facet_bin_raw, StaticTile};

    const FELUCCA: u8 = 0;
    const TOKUNO: u8 = 4;

    #[test]
    fn sectors_are_numbered_column_major_with_partial_rows() {
        let felucca = FacetGeometry::client_facet(FELUCCA).unwrap();
        assert_eq!(felucca.sector_rows(), 64);
        assert_eq!(felucca.sector_count(), 7168);
        assert_eq!(felucca.sector_of(0, 64), 1);
        assert_eq!(felucca.sector_of(64, 0), 64);
        assert_eq!(felucca.sector_origin(65), (64, 64));

        let tokuno = FacetGeometry::client_facet(TOKUNO).unwrap();
        assert_eq!(tokuno.sector_rows(), 23);
        assert_eq!(tokuno.sector_count(), 529);
        assert_eq!(tokuno.sector_of(1447, 1447), 528);
        assert_eq!(tokuno.sector_origin(23), (64, 0));
    }

    #[test]
    fn entry_names_are_zero_padded() {
        assert_eq!(sector_entry_path(3, 17), "build/sectors/facet_03/00000017.bin");
        assert_eq!(data_map_entry_path(5), "build/sectors/facet_05/data.map");
        assert_eq!(facet_package_file_name(1, true), "facet1x.uop");
    }

    #[test]
    fn the_variant_edition_picks_facet_nx_and_never_falls_back() {
        let dir = std::env::temp_dir().join(format!("uocf_facet_package_path_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["facet0.uop", "facet0x.uop", "facet1.uop", "facet4.uop"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        assert_eq!(facet_package_path(&dir, FELUCCA, MapEdition::Standard).unwrap(), dir.join("facet0.uop"));
        assert_eq!(facet_package_path(&dir, FELUCCA, MapEdition::Variant).unwrap(), dir.join("facet0x.uop"));
        // Tokuno has no variant package: both editions read facet4.uop.
        assert_eq!(facet_package_path(&dir, TOKUNO, MapEdition::Variant).unwrap(), dir.join("facet4.uop"));
        let message = format!("{:#}", facet_package_path(&dir, 1, MapEdition::Variant).unwrap_err());
        assert!(message.contains("no facet1x.uop") && message.contains("never read from facet1.uop"), "{message}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn data_map_holds_height_then_width() {
        let geometry = FacetGeometry::new(7168, 4096);
        let payload = geometry.data_map_payload();
        assert_eq!(&payload[..4], &4096u32.to_le_bytes());
        assert_eq!(FacetGeometry::from_data_map_payload(&payload).unwrap(), geometry);
    }

    #[test]
    fn delimiters_follow_the_shipped_rule() {
        let geometry = FacetGeometry::new(1448, 1448);
        let dirs = |x, y| delimiter_targets(&geometry, x, y).into_iter().map(|t| t.0).collect::<Vec<_>>();
        assert!(dirs(10, 10).is_empty());
        assert_eq!(dirs(64, 10), vec![0]);
        assert_eq!(dirs(64, 64), vec![0, 1, 2]);
        assert_eq!(dirs(127, 64), vec![2, 3, 7]);
        assert_eq!(dirs(64, 127), vec![0, 5, 6]);
        assert_eq!(dirs(127, 127), vec![3, 4, 5]);
        // Map border: no delimiter points off the map, and padding squares get none.
        assert_eq!(dirs(0, 63), vec![5]);
        assert!(dirs(1447, 10).is_empty());
        assert!(dirs(1471, 1471).is_empty());
    }

    #[test]
    fn encoder_inverts_the_decoder_and_rejects_what_the_client_rejects() {
        let mut sector = empty_ec_sector(2, 9).unwrap();
        sector.tiles[0].delimiters.push(EcFacetDelimiter { direction: 3, z: -4, graphic_id: 168 });
        sector.tiles[4095].statics.push(StaticTile { graphic_id: 0x1234, x: 63, y: 63, z: 7, hue: 33 });
        let bytes = encode_ec_sector(&sector).unwrap();
        let decoded = decode_facet_bin_raw(&bytes).unwrap();
        assert_eq!(encode_ec_sector(&decoded).unwrap(), bytes);

        let mut bad = bytes.clone();
        // Square 0 starts after the header: z, u16 land, delimiter count, then the direction byte.
        bad[SECTOR_HEADER_SIZE + 4] = DELIMITER_DIRECTION_COUNT;
        assert!(decode_facet_bin_raw(&bad).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_facet_bin_raw(&trailing).is_err());

        sector.tiles[1].statics = vec![StaticTile { graphic_id: 1, x: 0, y: 1, z: 0, hue: 0 }; MAX_SQUARE_RECORDS + 1];
        assert!(encode_ec_sector(&sector).is_err());
        sector.tiles[1].statics.clear();
        sector.tiles[2].delimiters.push(EcFacetDelimiter { direction: 9, z: 0, graphic_id: 0 });
        assert!(matches!(
            validate_ec_sector(&sector, Some((2, 10))).as_slice(),
            [EcSectorIssue::DelimiterDirection { square: 2, direction: 9 }, EcSectorIssue::HeaderMismatch { .. }]
        ));
    }
}
