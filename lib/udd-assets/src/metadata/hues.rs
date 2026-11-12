use std::path::Path;
#[cfg(test)]
use std::cell::Cell;
#[cfg(all(debug_assertions, not(test)))]
use std::sync::atomic::{AtomicU64, Ordering};

use color_eyre::eyre::{self, Context};
use udd_container::UddpReader;

use crate::common::{ensure_manifest_version, read_path_entry, read_path_entry_cow};
use uocf::enhanced::hues::{EcHueLookup, EcHueTable, EcHueTables, HUE_TABLE_ENTRIES};

/// Version header of the package: `HUES_PACKAGE_MAGIC` then u32 `HUES_FORMAT_VERSION`.
pub const HUES_PACKAGE_ENTRY_PATH: &str = "metadata/package.bin";
pub const HUES_PACKAGE_MAGIC: [u8; 4] = *b"HUPK";
/// Version of the whole package: the header, the CSV columns and the texture layout.
pub const HUES_FORMAT_VERSION: u32 = 1;
const HUES_PACKAGE_HEADER_SIZE: usize = 8;
pub const HUES_METADATA_ENTRY_PATH: &str = "metadata/hues.csv";
pub const HUES_TEXTURE_ENTRY_PATH: &str = "textures/hues.rgba8888";

pub const HUE_FLAG_PRESENT: u32 = 1 << 0;

pub const MAX_HUE_ID: u16 = uocf::enhanced::hues::MAX_EC_HUES;
pub const HUE_STRIP_WIDTH: u32 = uocf::enhanced::hues::HUE_STRIP_WIDTH;
pub const HUES_TEXTURE_WIDTH: u32 = 1024;
pub const HUES_TEXTURE_HEIGHT: u32 = uocf::enhanced::hues::HUES_ATLAS_HEIGHT;
/// Bytes of one RGBA8888 texel of `HUES_TEXTURE_ENTRY_PATH`.
const TEXEL_BYTES: usize = 4;
// A strip is a whole hue table: texel `x` of a hue's strip is the colour of red byte `x`.
const _: () = assert!(HUE_STRIP_WIDTH as usize == HUE_TABLE_ENTRIES);

const HUES_CSV_HEADER: [&str; 8] = [
    "hue_id",
    "name",
    "table_start",
    "table_end",
    "texture_column",
    "texture_row",
    "palette_width_pixels",
    "flags",
];

#[cfg(all(debug_assertions, not(test)))]
static FROM_UDDP_PACKAGE_CALLS: AtomicU64 = AtomicU64::new(0);

// Under `cfg(test)` the counter is per-thread: the test harness runs sibling tests
// that load packages concurrently, and a shared counter would make absolute-count
// assertions depend on scheduling. Diagnostic (non-test) builds keep the process-wide
// atomic, which is read across threads by the application.
/// Bytes of `HUES_PACKAGE_ENTRY_PATH`.
pub fn encode_hues_package_header() -> [u8; HUES_PACKAGE_HEADER_SIZE] {
    let mut bytes = [0u8; HUES_PACKAGE_HEADER_SIZE];
    bytes[0..4].copy_from_slice(&HUES_PACKAGE_MAGIC);
    bytes[4..8].copy_from_slice(&HUES_FORMAT_VERSION.to_le_bytes());
    bytes
}

fn check_hues_package_header(bytes: &[u8]) -> eyre::Result<()> {
    eyre::ensure!(
        bytes.len() == HUES_PACKAGE_HEADER_SIZE && bytes[0..4] == HUES_PACKAGE_MAGIC,
        "{HUES_PACKAGE_ENTRY_PATH}: no {HUES_PACKAGE_HEADER_SIZE}-byte HUPK header; repack with the current udd-cli"
    );
    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("slice length checked"));
    ensure_manifest_version(HUES_PACKAGE_ENTRY_PATH, version, HUES_FORMAT_VERSION)
}

#[cfg(test)]
thread_local! {
    static FROM_UDDP_PACKAGE_CALLS: Cell<u64> = const { Cell::new(0) };
}

#[cfg(all(debug_assertions, not(test)))]
fn note_from_uddp_package_call() {
    FROM_UDDP_PACKAGE_CALLS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
fn note_from_uddp_package_call() {
    FROM_UDDP_PACKAGE_CALLS.with(|calls| calls.set(calls.get() + 1));
}

#[cfg(all(debug_assertions, not(test)))]
pub fn debug_from_uddp_package_call_count() -> u64 {
    FROM_UDDP_PACKAGE_CALLS.load(Ordering::Relaxed)
}

#[cfg(test)]
pub fn debug_from_uddp_package_call_count() -> u64 {
    FROM_UDDP_PACKAGE_CALLS.with(Cell::get)
}

#[cfg(test)]
pub fn debug_reset_package_load_counters() {
    FROM_UDDP_PACKAGE_CALLS.with(|calls| calls.set(0));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HueSlotRecord {
    pub hue_id: u16,
    pub name: String,
    pub table_start: u16,
    pub table_end: u16,
    pub texture_column: u32,
    pub texture_row: u32,
    pub palette_width_pixels: u32,
    pub flags: u32,
}

impl HueSlotRecord {
    pub fn is_present(&self) -> bool {
        (self.flags & HUE_FLAG_PRESENT) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HueTextureCoord {
    pub column: u32,
    pub row: u32,
    pub x: u32,
    pub y: u32,
    pub u: f32,
    pub v: f32,
}

pub struct HuesPackage {
    package: UddpReader,
    slots: Vec<Option<HueSlotRecord>>,
    texture_bytes: Vec<u8>,
}

impl HuesPackage {
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
        #[cfg(any(test, debug_assertions))]
        note_from_uddp_package_call();

        let header = read_path_entry_cow(&package, HUES_PACKAGE_ENTRY_PATH)
            .context("hues.uddp has no metadata/package.bin; repack with the current udd-cli")?;
        check_hues_package_header(&header)?;
        let csv_bytes = read_path_entry_cow(&package, HUES_METADATA_ENTRY_PATH)
            .context("hues.uddp missing metadata/hues.csv")?;
        let slots = parse_hues_csv(&csv_bytes)?;

        let texture_bytes = read_path_entry(&package, HUES_TEXTURE_ENTRY_PATH)
            .context("hues.uddp missing textures/hues.rgba8888")?;
        validate_texture_len(&texture_bytes)?;

        Ok(Self {
            package,
            slots,
            texture_bytes,
        })
    }

    pub fn package(&self) -> &UddpReader {
        &self.package
    }

    pub fn slots(&self) -> &[Option<HueSlotRecord>] {
        &self.slots
    }

    pub fn slot(&self, hue_id: u16) -> Option<&HueSlotRecord> {
        self.slots
            .get(hue_id as usize)
            .and_then(|entry| entry.as_ref())
            .filter(|entry| entry.is_present())
    }

    pub fn hue_name(&self, hue_id: u16) -> Option<&str> {
        self.slot(hue_id).map(|slot| slot.name.as_str())
    }

    pub fn texture_coord_for_hue(&self, hue_id: u16) -> Option<HueTextureCoord> {
        let slot = self.slot(hue_id)?;
        let x = slot.texture_column * HUE_STRIP_WIDTH;
        let y = slot.texture_row;
        Some(HueTextureCoord {
            column: slot.texture_column,
            row: slot.texture_row,
            x,
            y,
            u: x as f32 / HUES_TEXTURE_WIDTH as f32,
            v: y as f32 / HUES_TEXTURE_HEIGHT as f32,
        })
    }

    pub fn texture_bytes(&self) -> &[u8] {
        &self.texture_bytes
    }

    /// The table of a hue, its strip of the texture: entry `x` is the colour a hueable texel of red byte `x` takes
    /// (`uocf::enhanced::hues::EcHueTable`). `None` for a hue the package does not hold.
    pub fn hue_table(&self, hue_id: u16) -> Option<EcHueTable> {
        let coord = self.texture_coord_for_hue(hue_id)?;
        let start = (coord.y * HUES_TEXTURE_WIDTH + coord.x) as usize * TEXEL_BYTES;
        let strip = self.texture_bytes.get(start..start + HUE_TABLE_ENTRIES * TEXEL_BYTES)?;
        let mut entries = [[0u8; TEXEL_BYTES]; HUE_TABLE_ENTRIES];
        for (entry, texel) in entries.iter_mut().zip(strip.chunks_exact(TEXEL_BYTES)) {
            entry.copy_from_slice(texel);
        }
        Some(EcHueTable { entries })
    }

    /// The tables the server hues `hues` take (`EcHueLookup::from_hue`), keyed by hue number as `Hues.uop`'s bitmaps:
    /// what `EcHueTables::apply` needs for them. A hue without a table, or that the package does not hold, adds none.
    pub fn ec_hue_tables_of(&self, hues: impl IntoIterator<Item = u16>) -> EcHueTables {
        let tables = hues
            .into_iter()
            .filter_map(|hue| match EcHueLookup::from_hue(hue) {
                EcHueLookup::Bitmap(number) => Some((number, self.hue_table(number)?)),
                EcHueLookup::None | EcHueLookup::Shadow => None,
            })
            .collect();
        EcHueTables { tables }
    }

    /// The tables of every hue the package holds, keyed by hue number as `Hues.uop`'s bitmaps.
    pub fn ec_hue_tables(&self) -> EcHueTables {
        let tables = (1..=MAX_HUE_ID)
            .filter_map(|hue_id| Some((hue_id, self.hue_table(hue_id)?)))
            .collect();
        EcHueTables { tables }
    }

    pub fn read_texture_bytes(&self) -> eyre::Result<Vec<u8>> {
        Ok(self.texture_bytes.clone())
    }
}

pub fn parse_hues_csv(bytes: &[u8]) -> eyre::Result<Vec<Option<HueSlotRecord>>> {
    let text = std::str::from_utf8(bytes).wrap_err("hues.csv is not valid UTF-8")?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or_else(|| eyre::eyre!("hues.csv is empty"))?
        .trim_end_matches('\r');
    let header_fields = parse_csv_row(header)?;
    if header_fields.len() != HUES_CSV_HEADER.len()
        || header_fields
            .iter()
            .map(String::as_str)
            .ne(HUES_CSV_HEADER.iter().copied())
    {
        eyre::bail!("hues.csv header does not match expected schema");
    }

    let mut slots = vec![None; MAX_HUE_ID as usize + 1];
    for (line_index, raw_line) in lines.enumerate() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let fields = parse_csv_row(line)
            .wrap_err_with(|| format!("parse hues.csv row {}", line_index + 2))?;
        if fields.len() != HUES_CSV_HEADER.len() {
            eyre::bail!(
                "hues.csv row {} has {} fields, expected {}",
                line_index + 2,
                fields.len(),
                HUES_CSV_HEADER.len()
            );
        }

        let hue_id = fields[0]
            .parse::<u16>()
            .wrap_err_with(|| format!("invalid hue_id on row {}", line_index + 2))?;
        if hue_id == 0 || hue_id > MAX_HUE_ID {
            eyre::bail!("row {} hue_id {} out of range", line_index + 2, hue_id);
        }

        let record = HueSlotRecord {
            hue_id,
            name: fields[1].clone(),
            table_start: fields[2]
                .parse::<u16>()
                .wrap_err_with(|| format!("invalid table_start on row {}", line_index + 2))?,
            table_end: fields[3]
                .parse::<u16>()
                .wrap_err_with(|| format!("invalid table_end on row {}", line_index + 2))?,
            texture_column: fields[4]
                .parse::<u32>()
                .wrap_err_with(|| format!("invalid texture_column on row {}", line_index + 2))?,
            texture_row: fields[5]
                .parse::<u32>()
                .wrap_err_with(|| format!("invalid texture_row on row {}", line_index + 2))?,
            palette_width_pixels: fields[6].parse::<u32>().wrap_err_with(|| {
                format!("invalid palette_width_pixels on row {}", line_index + 2)
            })?,
            flags: fields[7]
                .parse::<u32>()
                .wrap_err_with(|| format!("invalid flags on row {}", line_index + 2))?,
        };

        if record.palette_width_pixels != HUE_STRIP_WIDTH {
            eyre::bail!(
                "row {} palette_width_pixels {} does not match expected {}",
                line_index + 2,
                record.palette_width_pixels,
                HUE_STRIP_WIDTH
            );
        }
        if record.texture_column > 2 {
            eyre::bail!(
                "row {} texture_column {} out of range",
                line_index + 2,
                record.texture_column
            );
        }
        if record.texture_row >= HUES_TEXTURE_HEIGHT {
            eyre::bail!(
                "row {} texture_row {} out of range",
                line_index + 2,
                record.texture_row
            );
        }
        if slots[hue_id as usize].is_some() {
            eyre::bail!("duplicate hue_id {} in hues.csv", hue_id);
        }

        slots[hue_id as usize] = Some(record);
    }

    Ok(slots)
}

pub fn encode_hues_csv(records: &[HueSlotRecord]) -> eyre::Result<Vec<u8>> {
    let mut out = String::new();
    out.push_str(&HUES_CSV_HEADER.join(","));
    out.push('\n');

    for record in records {
        out.push_str(&record.hue_id.to_string());
        out.push(',');
        out.push_str(&escape_csv_field(&record.name));
        out.push(',');
        out.push_str(&record.table_start.to_string());
        out.push(',');
        out.push_str(&record.table_end.to_string());
        out.push(',');
        out.push_str(&record.texture_column.to_string());
        out.push(',');
        out.push_str(&record.texture_row.to_string());
        out.push(',');
        out.push_str(&record.palette_width_pixels.to_string());
        out.push(',');
        out.push_str(&record.flags.to_string());
        out.push('\n');
    }

    Ok(out.into_bytes())
}

fn validate_texture_len(bytes: &[u8]) -> eyre::Result<()> {
    let expected_len = HUES_TEXTURE_WIDTH as usize * HUES_TEXTURE_HEIGHT as usize * 4;
    if bytes.len() != expected_len {
        eyre::bail!(
            "hues texture byte length {} does not match expected {}",
            bytes.len(),
            expected_len
        );
    }
    Ok(())
}

fn escape_csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        let mut escaped = String::with_capacity(value.len() + 2);
        escaped.push('"');
        for ch in value.chars() {
            if ch == '"' {
                escaped.push('"');
            }
            escaped.push(ch);
        }
        escaped.push('"');
        escaped
    } else {
        value.to_string()
    }
}

fn parse_csv_row(line: &str) -> eyre::Result<Vec<String>> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes {
                    if chars.peek() == Some(&'"') {
                        current.push('"');
                        chars.next();
                    } else {
                        in_quotes = false;
                    }
                } else if current.is_empty() {
                    in_quotes = true;
                } else {
                    eyre::bail!("unexpected quote inside unquoted CSV field");
                }
            }
            ',' if !in_quotes => {
                fields.push(current);
                current = String::new();
            }
            _ => current.push(ch),
        }
    }

    if in_quotes {
        eyre::bail!("unterminated quoted CSV field");
    }

    fields.push(current);
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

    fn record(hue_id: u16, name: &str, texture_column: u32, texture_row: u32) -> HueSlotRecord {
        HueSlotRecord {
            hue_id,
            name: name.to_string(),
            table_start: 3,
            table_end: 27,
            texture_column,
            texture_row,
            palette_width_pixels: HUE_STRIP_WIDTH,
            flags: HUE_FLAG_PRESENT,
        }
    }

    #[test]
    fn csv_roundtrip_preserves_names_and_fields() {
        let records = vec![
            record(1, "plain", 0, 1),
            record(1024, "comma,quote\"name", 1, 0),
        ];

        let encoded = encode_hues_csv(&records).expect("encode csv");
        let decoded = parse_hues_csv(&encoded).expect("parse csv");

        assert_eq!(decoded[1].as_ref().expect("hue 1"), &records[0]);
        assert_eq!(decoded[1024].as_ref().expect("hue 1024"), &records[1]);
    }

    #[test]
    fn texture_coords_follow_ec_layout() {
        let records = vec![
            record(1, "first", 0, 1),
            record(1023, "edge", 0, 1023),
            record(1024, "col1", 1, 0),
            record(2048, "col2", 2, 0),
            record(3000, "last", 2, 952),
        ];
        let encoded = encode_hues_csv(&records).expect("encode csv");
        let slots = parse_hues_csv(&encoded).expect("parse csv");
        let package = HuesPackage {
            package: UddpReader::open(
                UddpBuilder::new(LookupMode::VirtualPathHash)
                    .build()
                    .expect("empty package"),
            )
            .expect("open empty package"),
            slots,
            texture_bytes: vec![0; HUES_TEXTURE_WIDTH as usize * HUES_TEXTURE_HEIGHT as usize * 4],
        };

        assert_eq!(
            package.texture_coord_for_hue(1),
            Some(HueTextureCoord {
                column: 0,
                row: 1,
                x: 0,
                y: 1,
                u: 0.0,
                v: 1.0 / HUES_TEXTURE_HEIGHT as f32,
            })
        );
        assert_eq!(package.texture_coord_for_hue(0), None);
        assert_eq!(package.texture_coord_for_hue(3001), None);
        assert_eq!(
            package.texture_coord_for_hue(1024).expect("hue 1024").x,
            HUE_STRIP_WIDTH
        );
        assert_eq!(
            package.texture_coord_for_hue(2048).expect("hue 2048").x,
            HUE_STRIP_WIDTH * 2
        );
    }

    /// The tables of server hues go by the client's lookup: a partial hue takes its number's table; no hue, the shadow
    /// and a hue the package does not hold take none.
    #[test]
    fn server_hues_take_the_tables_of_their_numbers() {
        use uocf::enhanced::hues::{HUE_PARTIAL_FLAG, SHADOW_HUE};
        const DYE: u16 = 38;
        const MISSING: u16 = 77;
        let records = vec![record(DYE, "dye", 0, u32::from(DYE))];
        let slots = parse_hues_csv(&encode_hues_csv(&records).expect("encode csv")).expect("parse csv");
        let mut texture_bytes = vec![0; HUES_TEXTURE_WIDTH as usize * HUES_TEXTURE_HEIGHT as usize * TEXEL_BYTES];
        let start = usize::from(DYE) * HUES_TEXTURE_WIDTH as usize * TEXEL_BYTES;
        for (index, texel) in texture_bytes[start..].chunks_exact_mut(TEXEL_BYTES).take(HUE_TABLE_ENTRIES).enumerate() {
            let key = index as u8;
            texel.copy_from_slice(&[key, u8::MAX - key, key / 2, u8::MAX]);
        }
        let package = HuesPackage {
            package: UddpReader::open(UddpBuilder::new(LookupMode::VirtualPathHash).build().expect("empty package"))
                .expect("open empty package"),
            slots,
            texture_bytes,
        };
        let tables = package.ec_hue_tables_of([0, DYE, DYE | HUE_PARTIAL_FLAG, SHADOW_HUE, MISSING]);
        assert_eq!(tables.tables.keys().copied().collect::<Vec<_>>(), [DYE]);
        assert_eq!(tables.tables.get(&DYE).cloned(), package.hue_table(DYE));
        assert_eq!(tables.tables[&DYE].entries[usize::from(DYE)], [DYE as u8, u8::MAX - DYE as u8, DYE as u8 / 2, u8::MAX]);
    }

    #[test]
    fn package_load_reads_csv_and_texture_payload() {
        debug_reset_package_load_counters();
        assert_eq!(debug_from_uddp_package_call_count(), 0);

        let records = vec![record(1, "name", 0, 1)];
        let csv = encode_hues_csv(&records).expect("encode csv");
        let texture = vec![0u8; HUES_TEXTURE_WIDTH as usize * HUES_TEXTURE_HEIGHT as usize * 4];

        let header = encode_hues_package_header();
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::None,
                width: 0,
                height: 0,
                virtual_path: Some(HUES_PACKAGE_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: &header,
            })
            .expect("add header");
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::None,
                width: 0,
                height: 0,
                virtual_path: Some(HUES_METADATA_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: &csv,
            })
            .expect("add csv");
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Texture as u8,
                compression: CompressionFlag::None,
                width: HUES_TEXTURE_WIDTH,
                height: HUES_TEXTURE_HEIGHT,
                virtual_path: Some(HUES_TEXTURE_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: &texture,
            })
            .expect("add texture");

        let package = HuesPackage::from_uddp_package(
            UddpReader::open(builder.build().expect("build package")).expect("open package"),
        )
        .expect("load hues package");

        assert_eq!(debug_from_uddp_package_call_count(), 1);
        assert_eq!(package.hue_name(1), Some("name"));
        assert_eq!(package.texture_bytes().len(), texture.len());
        assert_eq!(package.read_texture_bytes().expect("texture bytes").len(), texture.len());
    }

    #[test]
    fn the_package_header_rejects_other_versions_with_the_repack_message() {
        assert!(check_hues_package_header(&encode_hues_package_header()).is_ok());
        let mut other = encode_hues_package_header();
        other[4..8].copy_from_slice(&(HUES_FORMAT_VERSION + 1).to_le_bytes());
        for bytes in [&other[..], &other[..4]] {
            let error = check_hues_package_header(bytes).unwrap_err().to_string();
            assert!(error.contains("repack with the current udd-cli"), "{error}");
        }
    }
}
