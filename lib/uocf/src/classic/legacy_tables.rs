//! Text tables the Classic clients ship and **do not read**: `Tile256.txt`, `Tile1024.txt` and `Arttable.cfg`,
//! read as shipped and written back byte for byte.
//!
//! Neither the 2024 client nor UO Gold holds these file names (nor `gamegen.txt`, which holds only the text
//! `metrics.txt`); the client's `ArtTable` classes are the `art.def`, `sound.def` and `body.def` remaps
//! (`scripts/dev-helpers/ec-research/ghidra-classic-text-tables`). There is no client grammar to follow, so what
//! the shipped files hold is accepted and only what this model cannot hold is an error:
//!
//! - **`Tile256.txt`, `Tile1024.txt`** ([`TileTableRecord`]): a land tile id and a number per line, separated by a
//!   tab (ids up to 5000, numbers up to 162 in `Tile256.txt` and 10 in `Tile1024.txt`), and blank lines.
//! - **`Arttable.cfg`** ([`ArtTableFile`]): `ID {T1, T2, ...} VALUE` lines in the syntax of `art.def`, read by the
//!   [`TargetsRecord`](super::def_files::TargetsRecord) grammar.

use super::def_files::{DefDocument, DefRecord, DefSyntaxError, TargetsDefFile};

pub const TILE256_FILE: &str = "Tile256.txt";
pub const TILE1024_FILE: &str = "Tile1024.txt";
pub const ARTTABLE_FILE: &str = "Arttable.cfg";
/// Fields of a tile table line.
pub const TILE_TABLE_FIELDS: usize = 2;
const FIELD_SEPARATOR: char = '\t';

/// A line of `Tile256.txt` or `Tile1024.txt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileTableRecord {
    pub tile: u16,
    pub value: u32,
}

pub type TileTableFile = DefDocument<TileTableRecord>;
/// `Arttable.cfg`.
pub type ArtTableFile = TargetsDefFile;

impl DefRecord for TileTableRecord {
    fn parse(line: &str, line_number: usize) -> Result<Option<Self>, DefSyntaxError> {
        let fields: Vec<(usize, &str)> = line
            .split(|c: char| c.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .map(|field| (field.as_ptr() as usize - line.as_ptr() as usize, field))
            .collect();
        if fields.is_empty() {
            return Ok(None);
        }
        let error = |at: usize, message: String| DefSyntaxError { line: line_number, column: line[..at].chars().count() + 1, message };
        if fields.len() != TILE_TABLE_FIELDS {
            return Err(error(fields[0].0, format!("a line is a tile and a number, not {} fields", fields.len())));
        }
        let (tile_at, tile) = fields[0];
        let (value_at, value) = fields[1];
        let tile = tile.parse().map_err(|_| error(tile_at, format!("the tile `{tile}` is not a number from 0 to {}", u16::MAX)))?;
        let value = value.parse().map_err(|_| error(value_at, format!("the value `{value}` is not a number from 0 to {}", u32::MAX)))?;
        Ok(Some(Self { tile, value }))
    }

    fn format(&self) -> String {
        format!("{}{FIELD_SEPARATOR}{}", self.tile, self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_tables_write_back_and_reject_what_they_cannot_hold() {
        let text = "2\t0\r\n3\t17\r\n\r\n";
        let mut file = TileTableFile::from_bytes(text.as_bytes()).unwrap();
        assert_eq!(file.to_bytes(), text.as_bytes());
        assert_eq!(file.records().nth(1), Some(&TileTableRecord { tile: 3, value: 17 }));
        file.records_mut().next().unwrap().value = 4;
        assert_eq!(file.to_bytes(), b"2\t4\r\n3\t17\r\n\r\n");

        let error = TileTableFile::from_bytes(b"2\t0\r\n70000\t1\r\n").unwrap_err();
        assert_eq!((error.line, error.column), (2, 1));
        assert!(TileTableFile::from_bytes(b"2\t-1\r\n").is_err());
        assert!(TileTableFile::from_bytes(b"2\t1\t3\r\n").is_err());
    }
}
