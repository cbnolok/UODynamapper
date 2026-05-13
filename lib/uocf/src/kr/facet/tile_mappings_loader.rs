use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io;
use std::path::Path;
use byteorder::{LittleEndian, ReadBytesExt};

// region: --- Public API (Convenience & Application Use)

/// Loads a tile dictionary from a binary file.
///
/// The file is expected to contain a sequence of `(key: u16, value: (u16, u8))` pairs.
pub fn load_tile_dictionary(path: &Path) -> io::Result<HashMap<u16, (u16, u8)>> {
    let file = File::open(path)?;
    let mut buffer = std::io::BufReader::new(file);
    let mut dictionary = HashMap::new();

    while let Ok(key) = buffer.read_u16::<LittleEndian>() {
        let graphic_id = buffer.read_u16::<LittleEndian>()?;
        let unknown_byte = buffer.read_u8()?;
        dictionary.insert(key, (graphic_id, unknown_byte));
    }

    Ok(dictionary)
}

/// Loads a static dictionary from a binary file.
///
/// The file is expected to contain a sequence of `u16` values (a whitelist).
pub fn load_static_dictionary(path: &Path) -> io::Result<HashSet<u16>> {
    let file = File::open(path)?;
    let mut buffer = std::io::BufReader::new(file);
    let mut dictionary = HashSet::new();

    while let Ok(value) = buffer.read_u16::<LittleEndian>() {
        dictionary.insert(value);
    }

    Ok(dictionary)
}

// endregion: --- Public API
