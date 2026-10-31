//! Census of the images in client UOP packages: every entry is decompressed and DDS files are found by their
//! magic, whatever the package; other image types (TGA, BMP, PNG, JPEG, GIF, PSD) are counted, and DDS files
//! embedded inside other payloads are listed. [`DdsCensus::to_markdown`] gives the report `uocf-cli dds census`
//! prints, which `ENHANCED_CLIENT_FILES.md` summarizes ("DDS formats in client packages").

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::enhanced::dds_writer::{full_mip_count, DdsDescription, DdsTextureFormat, DDS_MAGIC};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::package::{LoadMode, UopPackage};

crate::eyre_imports!();

const UOP_EXTENSION: &str = "uop";
/// A DDS header inside another payload: the magic, then the header size 124.
const EMBEDDED_DDS_PATTERN: [u8; 8] = *b"DDS \x7c\0\0\0";
/// Examples kept per header convention or quirk.
const EXAMPLES_KEPT: usize = 3;
/// NVIDIA Texture Tools write `NVTT` and their version in the reserved dwords 9 and 10.
const NVTT_TAG: u32 = u32::from_le_bytes(*b"NVTT");
const NVTT_TAG_DWORD: usize = 9;
const NVTT_VERSION_DWORD: usize = 10;

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG_MAGIC: &[u8] = &[0xff, 0xd8, 0xff];
const GIF_MAGIC: &[u8] = b"GIF8";
const PSD_MAGIC: &[u8] = b"8BPS";
const BMP_MAGIC: &[u8] = b"BM";
const BMP_INFO_HEADER_SIZES: [u32; 6] = [12, 40, 52, 56, 108, 124];
const BMP_LENGTH_OFFSET: usize = 2;
const BMP_RESERVED_OFFSET: usize = 6;
const BMP_INFO_SIZE_OFFSET: usize = 14;
const BMP_BIT_COUNT_OFFSET: usize = 28;
const BMP_CORE_BIT_COUNT_OFFSET: usize = 24;
const BMP_CORE_HEADER_SIZE: u32 = 12;
const TGA_HEADER_SIZE: usize = 18;
const TGA_EXTENSION: &str = ".tga";
const TGA_COLOR_MAPPED_TYPES: [u8; 2] = [1, 9];
const TGA_IMAGE_TYPES: [u8; 6] = [1, 2, 3, 9, 10, 11];
const TGA_RLE_TYPES: [u8; 3] = [9, 10, 11];
const TGA_PIXEL_DEPTHS: [u8; 5] = [8, 15, 16, 24, 32];
const TGA_COLOR_MAP_LENGTH_OFFSET: usize = 5;
const TGA_COLOR_MAP_DEPTH_OFFSET: usize = 7;
const TGA_WIDTH_OFFSET: usize = 12;
const TGA_HEIGHT_OFFSET: usize = 14;
const TGA_DEPTH_OFFSET: usize = 16;
const TGA_DESCRIPTOR_OFFSET: usize = 17;
/// Descriptor bits 6 and 7 are reserved (interleaving), zero in every real TGA.
const TGA_DESCRIPTOR_RESERVED: u8 = 0xc0;
const BITS_PER_BYTE: usize = 8;

/// One DDS payload of a package.
#[derive(Debug, Clone)]
pub struct DdsCensusRecord {
    pub filename_hash: u64,
    pub name: Option<String>,
    pub compression: CompressionFlag,
    pub payload_len: usize,
    pub description: DdsDescription,
}

impl DdsCensusRecord {
    /// Bytes after the last surface (negative when the payload is short); None for formats of unknown layout.
    pub fn trailing_bytes(&self) -> Option<i64> {
        let description = &self.description;
        description.data_len.map(|data| self.payload_len as i64 - description.header_len as i64 - data as i64)
    }

    fn label(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("{:016X}", self.filename_hash),
        }
    }
}

/// The images of one package.
#[derive(Debug, Clone, Default)]
pub struct PackageCensus {
    /// The package path relative to the scanned directory.
    pub path: PathBuf,
    pub entries: usize,
    pub dds: Vec<DdsCensusRecord>,
    /// Other image payloads by type label (`TGA type 2, 32 bit`, `BMP 8 bit`, `PNG`, ...).
    pub other_images: BTreeMap<String, usize>,
    /// Payloads holding a DDS header after their start: entry label and offset.
    pub embedded_dds: Vec<(String, usize)>,
    /// Entries that failed to decompress: entry label and error.
    pub unreadable: Vec<(String, String)>,
}

/// The images of every package under a directory.
#[derive(Debug, Clone, Default)]
pub struct DdsCensus {
    pub packages: Vec<PackageCensus>,
}

/// Scans every `*.uop` under each of `roots` (recursively, in path order). A package is labelled with its path
/// relative to its root, after the root's folder name when there are several roots. `name_of` names an entry hash
/// when known.
pub fn census_directories(
    roots: &[PathBuf],
    name_of: &(dyn Fn(u64) -> Option<String> + Sync),
) -> eyre::Result<DdsCensus> {
    let mut packages = Vec::new();
    for root in roots {
        let paths = find_packages(root)?;
        let prefix = if roots.len() > 1 { root.file_name().map(PathBuf::from) } else { None };
        for path in paths {
            let mut package = census_package(&path, name_of).wrap_err_with(|| format!("scan {}", path.display()))?;
            let relative = path.strip_prefix(root).unwrap_or(&path);
            package.path = prefix.as_ref().map_or_else(|| relative.to_path_buf(), |prefix| prefix.join(relative));
            packages.push(package);
        }
    }
    Ok(DdsCensus { packages })
}

/// Every `*.uop` under `root`, recursively, in path order.
pub fn find_packages(root: &Path) -> eyre::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_packages(root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_packages(dir: &Path, out: &mut Vec<PathBuf>) -> eyre::Result<()> {
    for entry in std::fs::read_dir(dir).wrap_err_with(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_packages(&path, out)?;
        } else if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(UOP_EXTENSION)) {
            out.push(path);
        }
    }
    Ok(())
}

/// What one entry holds.
enum EntryImage {
    Dds(DdsCensusRecord),
    Other(String),
    Embedded(String, usize),
    Unreadable(String, String),
    None,
}

/// Scans the entries of one package.
pub fn census_package(path: &Path, name_of: &(dyn Fn(u64) -> Option<String> + Sync)) -> eyre::Result<PackageCensus> {
    let package = UopPackage::load_with_mode(path, LoadMode::Lazy)?;
    let files: Vec<_> = package.iter_files().filter(|file| file.has_size()).collect();
    let images: Vec<EntryImage> = files
        .par_iter()
        .map_init(
            || package.open_payload_reader(),
            |reader, file| {
                let hash = file.filename_hash();
                let name = name_of(hash);
                let label = name.clone().unwrap_or_else(|| format!("{hash:016X}"));
                let payload = match reader {
                    Ok(reader) => package.unpack_file_by_hash_from_reader(hash, reader),
                    Err(error) => Err(std::io::Error::new(error.kind(), error.to_string())),
                };
                let payload = match payload {
                    Ok(Some(payload)) => payload,
                    Ok(None) => return EntryImage::None,
                    Err(error) => return EntryImage::Unreadable(label, error.to_string()),
                };
                classify(&payload, hash, name, label, file.compression())
            },
        )
        .collect();
    let mut census = PackageCensus { path: path.to_path_buf(), entries: files.len(), ..Default::default() };
    for image in images {
        match image {
            EntryImage::Dds(record) => census.dds.push(record),
            EntryImage::Other(kind) => *census.other_images.entry(kind).or_default() += 1,
            EntryImage::Embedded(label, offset) => census.embedded_dds.push((label, offset)),
            EntryImage::Unreadable(label, error) => census.unreadable.push((label, error)),
            EntryImage::None => {}
        }
    }
    census.dds.sort_by(|a, b| a.label().cmp(&b.label()));
    Ok(census)
}

fn classify(payload: &[u8], hash: u64, name: Option<String>, label: String, compression: CompressionFlag) -> EntryImage {
    if payload.starts_with(DDS_MAGIC) {
        return match DdsDescription::parse(payload) {
            Ok(description) => EntryImage::Dds(DdsCensusRecord {
                filename_hash: hash,
                name,
                compression,
                payload_len: payload.len(),
                description,
            }),
            Err(error) => EntryImage::Unreadable(label, format!("{error}")),
        };
    }
    if let Some(kind) = other_image_kind(payload, name.as_deref()) {
        return EntryImage::Other(kind);
    }
    match payload.windows(EMBEDDED_DDS_PATTERN.len()).position(|window| window == EMBEDDED_DDS_PATTERN) {
        Some(offset) => EntryImage::Embedded(label, offset),
        None => EntryImage::None,
    }
}

/// The type of a non-DDS image payload.
fn other_image_kind(payload: &[u8], name: Option<&str>) -> Option<String> {
    for (magic, kind) in [(PNG_MAGIC, "PNG"), (JPEG_MAGIC, "JPEG"), (GIF_MAGIC, "GIF"), (PSD_MAGIC, "PSD")] {
        if payload.starts_with(magic) {
            return Some(kind.to_owned());
        }
    }
    bmp_kind(payload).or_else(|| tga_kind(payload, name))
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + size_of::<u32>()].try_into().expect("four bytes"))
}

fn bmp_kind(payload: &[u8]) -> Option<String> {
    if !payload.starts_with(BMP_MAGIC) || payload.len() < BMP_BIT_COUNT_OFFSET + size_of::<u16>() {
        return None;
    }
    let info_size = read_u32(payload, BMP_INFO_SIZE_OFFSET);
    if read_u32(payload, BMP_RESERVED_OFFSET) != 0
        || read_u32(payload, BMP_LENGTH_OFFSET) as usize > payload.len()
        || !BMP_INFO_HEADER_SIZES.contains(&info_size)
    {
        return None;
    }
    let bit_count_offset =
        if info_size == BMP_CORE_HEADER_SIZE { BMP_CORE_BIT_COUNT_OFFSET } else { BMP_BIT_COUNT_OFFSET };
    Some(format!("BMP {} bit", read_u16(payload, bit_count_offset)))
}

/// A TGA by its header; when the entry name does not end in `.tga`, it must also decode.
fn tga_kind(payload: &[u8], name: Option<&str>) -> Option<String> {
    if payload.len() < TGA_HEADER_SIZE {
        return None;
    }
    let (id_length, color_map_type, image_type) = (payload[0] as usize, payload[1], payload[2]);
    let depth = payload[TGA_DEPTH_OFFSET];
    let color_mapped = TGA_COLOR_MAPPED_TYPES.contains(&image_type);
    if color_map_type > 1
        || (color_map_type == 1) != color_mapped
        || !TGA_IMAGE_TYPES.contains(&image_type)
        || !TGA_PIXEL_DEPTHS.contains(&depth)
        || payload[TGA_DESCRIPTOR_OFFSET] & TGA_DESCRIPTOR_RESERVED != 0
    {
        return None;
    }
    let (width, height) = (read_u16(payload, TGA_WIDTH_OFFSET) as usize, read_u16(payload, TGA_HEIGHT_OFFSET) as usize);
    let color_map_bytes = read_u16(payload, TGA_COLOR_MAP_LENGTH_OFFSET) as usize
        * (payload[TGA_COLOR_MAP_DEPTH_OFFSET] as usize).div_ceil(BITS_PER_BYTE);
    let pixels_start = TGA_HEADER_SIZE + id_length + color_map_bytes;
    let named_tga = name.is_some_and(|name| name.to_ascii_lowercase().ends_with(TGA_EXTENSION));
    let plausible = if width == 0 || height == 0 {
        payload.len() <= pixels_start + TGA_FOOTER_ALLOWANCE
    } else if TGA_RLE_TYPES.contains(&image_type) {
        named_tga || (image::load_from_memory_with_format(payload, image::ImageFormat::Tga).is_ok())
    } else {
        let pixel_bytes = width * height * (depth as usize).div_ceil(BITS_PER_BYTE);
        payload.len() >= pixels_start + pixel_bytes && (named_tga || payload.len() <= pixels_start + pixel_bytes + TGA_FOOTER_ALLOWANCE)
    };
    plausible.then(|| format!("TGA type {image_type}, {depth} bit"))
}

/// Bytes an uncompressed TGA may hold after its pixels (the TGA 2.0 extension area and footer) for an unnamed
/// entry to count as one.
const TGA_FOOTER_ALLOWANCE: usize = 1024;

// region: --- Report

/// How the mip count field relates to the image size: 0 (one level), 1, the full chain, the chain down to the
/// smaller side at 1, or another count.
const MIP_RULES: [&str; 5] = ["0", "1", "full", "to min side", "other"];

fn mip_rule(record: &DdsCensusRecord) -> &'static str {
    let description = &record.description;
    let (width, height) = (description.width(), description.height());
    match description.header.mipmap_count {
        0 => "0",
        1 => "1",
        count if count == full_mip_count(width, height) => "full",
        count if count == full_mip_count(width.min(height), width.min(height)) => "to min side",
        _ => "other",
    }
}

/// How `pitchOrLinearSize` relates to mip 0.
fn pitch_rule(record: &DdsCensusRecord) -> String {
    let description = &record.description;
    let value = description.header.pitch_or_linear_size;
    let (width, height) = (description.width(), description.height());
    let surface = description.format.surface_bytes(width, height);
    let row = description.format.surface_bytes(width, 1);
    if value == 0 {
        "0".to_owned()
    } else if description.format.is_block_compressed() && surface == Some(u64::from(value)) {
        "mip 0 bytes".to_owned()
    } else if !description.format.is_block_compressed() && row == Some(u64::from(value)) {
        "row bytes".to_owned()
    } else if surface == Some(u64::from(value)) {
        "mip 0 bytes".to_owned()
    } else {
        "other".to_owned()
    }
}

/// The reserved dwords: zero, an NVTT tag with its version, or the non-zero dwords.
fn reserved_rule(record: &DdsCensusRecord) -> String {
    let reserved = &record.description.header.reserved1;
    if reserved.iter().all(|&dword| dword == 0) {
        return "0".to_owned();
    }
    if reserved[NVTT_TAG_DWORD] == NVTT_TAG
        && reserved.iter().enumerate().all(|(index, &dword)| index >= NVTT_TAG_DWORD || dword == 0)
    {
        return format!("NVTT {:#x}", reserved[NVTT_VERSION_DWORD]);
    }
    reserved
        .iter()
        .enumerate()
        .filter(|(_, dword)| **dword != 0)
        .map(|(index, dword)| format!("[{index}]={dword:#x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Header fields beyond format, flags, caps, pitch and reserved dwords that are not zero.
fn extra_fields(record: &DdsCensusRecord) -> String {
    let header = &record.description.header;
    let pixel_format = &header.pixel_format;
    let mut extra = Vec::new();
    if header.depth != 0 && !record.description.is_volume() {
        extra.push(format!("depth {}", header.depth));
    }
    if pixel_format.flags.contains(dds::header::PixelFormatFlags::FOURCC)
        && (pixel_format.rgb_bit_count != 0
            || [pixel_format.r_bit_mask, pixel_format.g_bit_mask, pixel_format.b_bit_mask, pixel_format.a_bit_mask]
                .iter()
                .any(|&mask| mask != 0))
    {
        extra.push(format!(
            "FourCC with bit count {} masks {:x}/{:x}/{:x}/{:x}",
            pixel_format.rgb_bit_count,
            pixel_format.r_bit_mask,
            pixel_format.g_bit_mask,
            pixel_format.b_bit_mask,
            pixel_format.a_bit_mask
        ));
    }
    if pixel_format.size != crate::enhanced::dds_writer::DDS_PIXEL_FORMAT_STRUCT_SIZE {
        extra.push(format!("pixel format size {}", pixel_format.size));
    }
    if header.size != crate::enhanced::dds_writer::DDS_HEADER_STRUCT_SIZE {
        extra.push(format!("header size {}", header.size));
    }
    for (field, value) in [("caps3", header.caps3), ("caps4", header.caps4), ("reserved2", header.reserved2)] {
        if value != 0 {
            extra.push(format!("{field} {value:#x}"));
        }
    }
    if let Some(dx10) = &header.dx10 {
        extra.push(format!(
            "DX10 dim {} misc {:#x} array {} misc2 {:#x}",
            dx10.resource_dimension,
            dx10.misc_flag.bits(),
            dx10.array_size,
            dx10.misc_flags2
        ));
    }
    extra.join(", ")
}

fn is_power_of_two_image(record: &DdsCensusRecord) -> bool {
    record.description.width().is_power_of_two() && record.description.height().is_power_of_two()
}

/// One header convention: everything in a header but the size-dependent fields.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ConventionKey {
    format: String,
    flags: u32,
    caps: u32,
    caps2: u32,
    pixel_format_flags: u32,
    pitch: String,
    mips: &'static str,
    reserved: String,
    extra: String,
}

impl ConventionKey {
    fn of(record: &DdsCensusRecord) -> Self {
        let header = &record.description.header;
        Self {
            format: record.description.format.label(),
            flags: header.flags.bits(),
            caps: header.caps.bits(),
            caps2: header.caps2.bits(),
            pixel_format_flags: header.pixel_format.flags.bits(),
            pitch: pitch_rule(record),
            mips: mip_rule(record),
            reserved: reserved_rule(record),
            extra: extra_fields(record),
        }
    }
}

#[derive(Default)]
struct FormatStats {
    count: usize,
    mips: BTreeMap<&'static str, usize>,
    cube: usize,
    volume: usize,
    non_power_of_two: usize,
    min_size: Option<(u32, u32)>,
    max_size: Option<(u32, u32)>,
    trailing: usize,
    compression: BTreeMap<String, usize>,
}

#[derive(Default)]
struct ConventionStats {
    count: usize,
    packages: BTreeSet<String>,
    examples: Vec<String>,
}

impl DdsCensus {
    pub fn dds_count(&self) -> usize {
        self.packages.iter().map(|package| package.dds.len()).sum()
    }

    /// Every DDS record with its package path.
    pub fn records(&self) -> impl Iterator<Item = (&Path, &DdsCensusRecord)> {
        self.packages.iter().flat_map(|package| package.dds.iter().map(move |record| (package.path.as_path(), record)))
    }

    /// One tab-separated line per DDS: package, entry hash and name, compression, payload length, format, width,
    /// height, then the 31 header dwords after the magic (and the 5 DX10 dwords) in hex.
    pub fn records_tsv(&self) -> String {
        let mut out = String::from("package\thash\tname\tcompression\tpayload_len\tformat\twidth\theight\theader_dwords\n");
        for (path, record) in self.records() {
            let description = &record.description;
            let mut header = Vec::new();
            description.header.write(&mut header).expect("writing to memory");
            let dwords = header
                .chunks_exact(size_of::<u32>())
                .map(|dword| format!("{:08x}", u32::from_le_bytes(dword.try_into().expect("four bytes"))))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = writeln!(
                out,
                "{}\t{:016X}\t{}\t{:?}\t{}\t{}\t{}\t{}\t{dwords}",
                path.display(),
                record.filename_hash,
                record.name.as_deref().unwrap_or(""),
                record.compression,
                record.payload_len,
                description.format.label(),
                description.width(),
                description.height()
            );
        }
        out
    }

    /// The census report: totals, formats per package, header conventions, quirks and other images.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let entries: usize = self.packages.iter().map(|package| package.entries).sum();
        let _ = writeln!(out, "# DDS census\n");
        let _ = writeln!(
            out,
            "{} packages, {} entries, {} DDS payloads (by the `DDS ` magic after decompression).\n",
            self.packages.len(),
            entries,
            self.dds_count()
        );

        let _ = writeln!(out, "## Formats per package\n");
        let _ = writeln!(
            out,
            "| Package | Entries | Format | Count | Mip count field (0 / 1 / full / to min side / other) | Cube | Volume | Non-pow2 | Smallest | Largest | Trailing bytes | Compression |"
        );
        let _ = writeln!(out, "|---|---:|---|---:|---|---:|---:|---:|---|---|---:|---|");
        for package in &self.packages {
            if package.dds.is_empty() {
                continue;
            }
            let mut formats: BTreeMap<String, FormatStats> = BTreeMap::new();
            for record in &package.dds {
                let stats = formats.entry(record.description.format.label()).or_default();
                let description = &record.description;
                let size = (description.width(), description.height());
                let area = |size: (u32, u32)| u64::from(size.0) * u64::from(size.1);
                stats.count += 1;
                *stats.mips.entry(mip_rule(record)).or_default() += 1;
                stats.cube += usize::from(description.is_cube_map());
                stats.volume += usize::from(description.is_volume());
                stats.non_power_of_two += usize::from(!is_power_of_two_image(record));
                if stats.min_size.is_none_or(|min| area(size) < area(min)) {
                    stats.min_size = Some(size);
                }
                if stats.max_size.is_none_or(|max| area(size) > area(max)) {
                    stats.max_size = Some(size);
                }
                stats.trailing += usize::from(record.trailing_bytes().is_some_and(|bytes| bytes != 0));
                *stats.compression.entry(format!("{:?}", record.compression)).or_default() += 1;
            }
            for (format, stats) in formats {
                let mips = MIP_RULES
                    .map(|rule| stats.mips.get(rule).copied().unwrap_or(0).to_string())
                    .join(" / ");
                let size = |size: Option<(u32, u32)>| size.map_or(String::new(), |(w, h)| format!("{w}x{h}"));
                let compression =
                    stats.compression.iter().map(|(name, count)| format!("{name} {count}")).collect::<Vec<_>>().join(", ");
                let _ = writeln!(
                    out,
                    "| {} | {} | {format} | {} | {mips} | {} | {} | {} | {} | {} | {} | {compression} |",
                    package.path.display(),
                    package.entries,
                    stats.count,
                    stats.cube,
                    stats.volume,
                    stats.non_power_of_two,
                    size(stats.min_size),
                    size(stats.max_size),
                    stats.trailing
                );
            }
        }

        let _ = writeln!(out, "\n## Header conventions\n");
        let _ = writeln!(
            out,
            "Every header field but width, height, the mip count and `pitchOrLinearSize` (given as its relation to \
             mip 0). Flags, caps and caps2 in hex.\n"
        );
        let _ = writeln!(out, "| Format | Flags | Caps | Caps2 | PF flags | Pitch | Mips | Reserved1 | Other fields | Count | Packages | Examples |");
        let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---:|---|---|");
        let mut conventions: BTreeMap<ConventionKey, ConventionStats> = BTreeMap::new();
        for (path, record) in self.records() {
            let stats = conventions.entry(ConventionKey::of(record)).or_default();
            stats.count += 1;
            stats.packages.insert(package_label(path));
            if stats.examples.len() < EXAMPLES_KEPT {
                let description = &record.description;
                stats.examples.push(format!(
                    "{} {}x{} m{}",
                    record.label(),
                    description.width(),
                    description.height(),
                    description.mip_count
                ));
            }
        }
        for (key, stats) in &conventions {
            let _ = writeln!(
                out,
                "| {} | {:#x} | {:#x} | {:#x} | {:#x} | {} | {} | {} | {} | {} | {} | {} |",
                key.format,
                key.flags,
                key.caps,
                key.caps2,
                key.pixel_format_flags,
                key.pitch,
                key.mips,
                key.reserved,
                key.extra,
                stats.count,
                stats.packages.iter().cloned().collect::<Vec<_>>().join(", "),
                stats.examples.join("; ")
            );
        }

        let _ = writeln!(out, "\n## Payload length vs header\n");
        let mut mismatches: BTreeMap<(String, i64), ConventionStats> = BTreeMap::new();
        let mut unknown_layout = 0usize;
        for (path, record) in self.records() {
            match record.trailing_bytes() {
                None => unknown_layout += 1,
                Some(0) => {}
                Some(bytes) => {
                    let stats = mismatches.entry((record.description.format.label(), bytes)).or_default();
                    stats.count += 1;
                    stats.packages.insert(package_label(path));
                    if stats.examples.len() < EXAMPLES_KEPT {
                        stats.examples.push(record.label());
                    }
                }
            }
        }
        if mismatches.is_empty() {
            let _ = writeln!(out, "Every payload ends with its last surface.");
        } else {
            let _ = writeln!(out, "| Format | Bytes after the last surface | Count | Packages | Examples |");
            let _ = writeln!(out, "|---|---:|---:|---|---|");
            for ((format, bytes), stats) in &mismatches {
                let _ = writeln!(
                    out,
                    "| {format} | {bytes} | {} | {} | {} |",
                    stats.count,
                    stats.packages.iter().cloned().collect::<Vec<_>>().join(", "),
                    stats.examples.join("; ")
                );
            }
        }
        if unknown_layout > 0 {
            let _ = writeln!(out, "\n{unknown_layout} payloads have a format of unknown layout.");
        }

        let _ = writeln!(out, "\n## Other images\n");
        let _ = writeln!(out, "| Package | Type | Count |");
        let _ = writeln!(out, "|---|---|---:|");
        for package in &self.packages {
            for (kind, count) in &package.other_images {
                let _ = writeln!(out, "| {} | {kind} | {count} |", package.path.display());
            }
        }

        let _ = writeln!(out, "\n## DDS inside other payloads\n");
        let embedded: Vec<_> = self.packages.iter().filter(|package| !package.embedded_dds.is_empty()).collect();
        if embedded.is_empty() {
            let _ = writeln!(out, "None.");
        } else {
            let _ = writeln!(out, "| Package | Count | Examples (entry @ offset) |");
            let _ = writeln!(out, "|---|---:|---|");
            for package in embedded {
                let examples = package
                    .embedded_dds
                    .iter()
                    .take(EXAMPLES_KEPT)
                    .map(|(label, offset)| format!("{label} @ {offset}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                let _ = writeln!(out, "| {} | {} | {examples} |", package.path.display(), package.embedded_dds.len());
            }
        }

        let unreadable: Vec<_> = self.packages.iter().filter(|package| !package.unreadable.is_empty()).collect();
        if !unreadable.is_empty() {
            let _ = writeln!(out, "\n## Unreadable entries\n");
            for package in unreadable {
                for (label, error) in package.unreadable.iter().take(EXAMPLES_KEPT) {
                    let _ = writeln!(out, "- {} {label}: {error}", package.path.display());
                }
                let _ = writeln!(out, "- {}: {} in all", package.path.display(), package.unreadable.len());
            }
        }
        out
    }
}

/// `folder/file.uop` of a package path relative to the scanned directory.
fn package_label(path: &Path) -> String {
    path.display().to_string()
}

// endregion: --- Report
