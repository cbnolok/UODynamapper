//! # UO Enhanced Client Facet uop Decoder
//!
//! This module provides functionality to decode `.uop` map files (facets) from the Enhanced Client
//! back into the classic 8x8 `MapBlock` format and associated static tiles.
//!
//! ## Facet `.bin` File Format
//!
//! The `facet.uop` file is a standard UOP package containing Zlib-compressed `.bin` files.
//! Each `.bin` file represents a 64x64 tile chunk of the map. The data within each bin is structured
//! as follows, laid out in column-major order (X is the outer loop, Y is the inner loop):
//!
//! 1.  **Header** (3 bytes)
//!     - `BYTE`: Facet ID (the map index, e.g., 0 for Felucca/Trammel).
//!     - `WORD`: File ID (the index of the 64x64 block in the map).
//!
//! 2.  **Tile Data** (4096 entries for map0, one for each tile in the 64x64 grid)
//!     For each tile, the following data is present:
//!     - **Land Tile**
//!         - `BYTE`: Z-coordinate of the land tile.
//!         - `WORD`: Graphic ID of the land tile.
//!     - **Delimiters** (Variable size)
//!         - `BYTE`: `count` of delimiter entries.
//!         - For each entry:
//!             - `BYTE`: `direction` (0-7). If > 7, no further data for this entry.
//!             - `BYTE`: Z-coordinate of the adjacent tile.
//!             - `DWORD`: Graphic ID of the adjacent tile.
//!         *(This data is used by the Enhanced Client to stitch map chunks and is ignored by this decoder).*
//!     - **Statics** (Variable size)
//!         - `BYTE`: `count` of static items on this tile.
//!         - For each static:
//!             - `DWORD`: Graphic ID of the static item.
//!             - `BYTE`: Z-coordinate of the static item.
//!             - `DWORD`: Hue of the static item.

//#![allow(dead_code)]
crate::eyre_imports!();
use crate::classic::map::MapBlock;
use crate::uop_container as uop;
use crate::uop_container::package::UopPackage;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Seek, SeekFrom};

// region: --- Public API (Convenience & Application Use)

/// Represents a single static item on the map, including its graphic, position, and hue.
#[derive(Debug, Clone)]
pub struct StaticTile {
    /// The graphic ID of the static item.
    pub graphic_id: u32,
    /// The X-coordinate of the static within its 8x8 `MapBlock` (0-7).
    pub x: u8,
    /// The Y-coordinate of the static within its 8x8 `MapBlock` (0-7).
    pub y: u8,
    /// The Z-coordinate (altitude) of the static.
    pub z: i8,
    /// The hue of the static item.
    pub hue: u32,
}

/// Represents a fully decoded classic 8x8 map block and its associated statics.
pub struct DecodedBlockClassic {
    /// The classic 8x8 map block containing land tiles.
    pub block: MapBlock,
    /// A vector of static items located within this block.
    pub statics: Vec<StaticTile>,
}

/// Represents the complete decoded content of a single 64x64 facet `.bin` file.
pub struct DecodedFacet {
    /// A vector of 64 `DecodedBlock`s, representing the 8x8 grid of classic blocks.
    pub blocks: Vec<DecodedBlockClassic>,
}

/// Reads and decodes a single 64x64 block from a facet UOP file.
///
/// # Arguments
/// * `package` - A loaded `UOPackage` for the facet file.
/// * `map_index` - The index of the map (e.g., 0 for facet0.uop).
/// * `block_id` - The ID of the 64x64 block to read.
pub fn read_facet_block(
    package: &UopPackage,
    map_index: u8,
    block_id: u32,
) -> eyre::Result<DecodedFacet> {
    // Construct the internal path to the .bin file within the UOP package.
    use std::io::Write;
    let mut path_buf = [0u8; 64];
    let mut slice = &mut path_buf[..];
    write!(
        slice,
        "build/sectors/facet_0{}/{:08}.bin",
        map_index, block_id
    )
    .unwrap();
    let len = 64 - slice.len();
    let target_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
    let target_hash: u64 = uop::hash::hash_file_name_single(target_path);

    // Iterate through the files in the package to find the correct entry.
    for file in package.iter_files() {
        if file.filename_hash() == target_hash {
            let decompressed_data = file.unpack()?;
            return decode_facet_bin(&decompressed_data);
        }
    }

    Err(eyre!("File not found in package: {}", target_path))
}

/// Decodes a single raw, decompressed .bin file from a facet.uop.
/// Each .bin file represents a 64x64 tile area, which corresponds to an 8x8 grid of classic 8x8 MapBlocks.
pub fn decode_facet_bin(data: &[u8]) -> eyre::Result<DecodedFacet> {
    // TODO: comment divisions and reminders and implement them with bitwise operations for speed.

    let mut cursor: Cursor<&[u8]> = Cursor::new(data);

    // 1. Read Header
    let _facet_id: u8 = cursor.read_u8()?; // FacetID
    let _file_id: u16 = cursor.read_u16::<LittleEndian>()?; // FileID

    // Initialize the result structure. We will have 64 classic blocks.
    let mut decoded_blocks: Vec<DecodedBlockClassic> = (0..64)
        .map(|i| {
            let block_x = (i % 8) as u32;
            let block_y = (i / 8) as u32;
            let mut block = MapBlock::default();
            block.internal_coords.x = block_x;
            block.internal_coords.y = block_y;
            DecodedBlockClassic {
                block,
                statics: Vec::new(),
            }
        })
        .collect();

    // 2. Read Tile Data
    // The data is stored in column-major order (X is the outer loop).
    for x_64 in 0..64 {
        for y_64 in 0..64 {
            // --- Land Tile ---
            let z: i8 = cursor.read_i8()?;
            let land_graphic: u16 = cursor.read_u16::<LittleEndian>()?;

            // Determine which classic 8x8 block this tile belongs to.
            let block_x: u32 = x_64 / 8;
            let block_y: u32 = y_64 / 8;
            let block_index: usize = (block_y * 8 + block_x) as usize;

            // Determine the coordinates within that classic 8x8 block.
            let cell_x: u32 = x_64 % 8;
            let cell_y: u32 = y_64 % 8;

            // Update the land tile cell in the corresponding map block.
            if let Ok(cell) = decoded_blocks[block_index]
                .block
                .cell_as_mut(cell_x, cell_y)
            {
                cell.id = land_graphic;
                cell.z = z;
            }

            // --- Delimiters (Ignored) ---
            // This data is for stitching map chunks in the enhanced client. We skip over it.
            let delimiter_count: u8 = cursor.read_u8()?;
            for _ in 0..delimiter_count {
                let direction: u8 = cursor.read_u8()?;
                if direction <= 7 {
                    // If the direction is valid, skip the Z (1 byte) and Graphic (4 bytes).
                    cursor.seek(SeekFrom::Current(5))?;
                }
            }

            // --- Statics ---
            let statics_count: u8 = cursor.read_u8()?;
            if statics_count > 0 {
                for _ in 0..statics_count {
                    let graphic_id = cursor.read_u32::<LittleEndian>()?;
                    let static_z = cursor.read_i8()?;
                    let hue = cursor.read_u32::<LittleEndian>()?;

                    // Create the static tile with coordinates relative to its 8x8 block.
                    let static_tile = StaticTile {
                        graphic_id,
                        x: cell_x as u8,
                        y: cell_y as u8,
                        z: static_z,
                        hue,
                    };

                    // Add the static to the correct DecodedBlock.
                    decoded_blocks[block_index].statics.push(static_tile);
                }
            }
        }
    }
    Ok(DecodedFacet {
        blocks: decoded_blocks,
    })
}

// endregion: --- Public API
