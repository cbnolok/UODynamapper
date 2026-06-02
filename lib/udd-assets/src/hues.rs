use std::path::Path;

use color_eyre::eyre::{self, Context};
use udd_container::UddpReader;

use crate::common::{read_path_entry, read_path_entry_cow};

pub const HUES_METADATA_ENTRY_PATH: &str = "metadata/hues.csv";
pub const HUES_TEXTURE_ENTRY_PATH: &str = "textures/hues.rgba8888";

pub const HUE_FLAG_PRESENT: u32 = 1 << 0;

pub const MAX_HUE_ID: u16 = uocf::enhanced::hues::MAX_EC_HUES;
pub const HUE_STRIP_WIDTH: u32 = uocf::enhanced::hues::HUE_STRIP_WIDTH;
pub const HUES_TEXTURE_WIDTH: u32 = 1024;
pub const HUES_TEXTURE_HEIGHT: u32 = uocf::enhanced::hues::HUES_ATLAS_HEIGHT;

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
        let csv_bytes = read_path_entry_cow(&package, HUES_METADATA_ENTRY_PATH)
            .context("hues.uddp missing metadata/hues.csv")?;
        let slots = parse_hues_csv(&csv_bytes)?;

        let texture_bytes = read_path_entry(&package, HUES_TEXTURE_ENTRY_PATH)
            .context("hues.uddp missing textures/hues.rgba8888")?;
        validate_texture_len(&texture_bytes)?;

        Ok(Self { package, slots })
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

    pub fn read_texture_bytes(&self) -> eyre::Result<Vec<u8>> {
        let bytes = read_path_entry(&self.package, HUES_TEXTURE_ENTRY_PATH)?;
        validate_texture_len(&bytes)?;
        Ok(bytes)
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

    #[test]
    fn package_load_reads_csv_and_texture_payload() {
        let records = vec![record(1, "name", 0, 1)];
        let csv = encode_hues_csv(&records).expect("encode csv");
        let texture = vec![0u8; HUES_TEXTURE_WIDTH as usize * HUES_TEXTURE_HEIGHT as usize * 4];

        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
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

        assert_eq!(package.hue_name(1), Some("name"));
        assert_eq!(package.read_texture_bytes().expect("texture bytes").len(), texture.len());
    }
}
