//! The facet size table of the client executables (`UOFacet::g_FacetDimensions`) and the hard-coded Felucca/Trammel
//! split at x = 5120: locate, report and patch them in a copy of the executable.
//!
//! Traced in `scripts/dev-helpers/ec-research/ghidra-facets/results/uosa_facets.md` ("Other dimension sites",
//! "Constraints", "Builds and the cross-build patch recipe") and `uokr_facets.md`:
//!
//! - The table is 7 records `{char* name, u32 width, u32 height}` in `.data`, located by its name pointers
//!   (Felucca, Trammel, Ilshenar, Malas, Tokuno, Gargoyle, Test). The client sizes its sector grid and chunk
//!   bounds from it.
//! - `UOFacet::IsSameFeluccaTrammelHalf` (UOSA 0x005df670) returns false, for facets 0 and 1, when two x lie on
//!   different sides of 5120; `BuildChunkFromSector` then does not build the chunk. The 5120 is three imm32 in one
//!   `.text` sequence; 0x8000, above any legal width, disables the split. Kingdom Reborn has no split.
//! - The PE checksum is recomputed (the client does not verify it: `UOSA_eclo.exe` ships a wrong one and runs).
//!   A signed (Authenticode) build is refused, since any patch breaks its signature; no shipped build is signed.
//!
//! The executable alone is not enough: the facet data must have the new size too ([`super::facet_data`]).

crate::eyre_imports!();
use std::path::{Path, PathBuf};

use crate::client_exe::pe::{pe_checksum, read_u32, PeImage, MACHINE_I386};
use crate::client_exe::sha256::sha256_hex;
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::facet_sector::{FacetGeometry, CLIENT_FACET_DIMENSIONS, SECTOR_SIDE};

/// Facets of the table; every client reader rejects a facet >= 7.
pub const FACET_COUNT: usize = CLIENT_FACET_DIMENSIONS.len();
/// The table's facet names, in record order.
pub const FACET_NAMES: [&str; FACET_COUNT] = ["Felucca", "Trammel", "Ilshenar", "Malas", "Tokuno", "Gargoyle", "Test"];
/// Bytes of a table record: `char* name, u32 width, u32 height`.
pub const FACET_RECORD_SIZE: usize = 12;
const FACET_RECORD_WIDTH_OFFSET: usize = 4;
const FACET_RECORD_HEIGHT_OFFSET: usize = 8;
/// The table is pointer aligned.
const TABLE_SCAN_STEP: usize = size_of::<u32>();
const TABLE_SECTION: &str = ".data";
const CODE_SECTION: &str = ".text";

/// The largest width or height: the land accessors reject x or y > 0x7fe0, and the side is a multiple of 64.
pub const MAX_FACET_SIDE: u32 = 32704;
/// Widths and heights must be multiples of this (Tokuno, 1448, is not a multiple of 64; DXT1 needs 4).
pub const FACET_SIDE_ALIGNMENT: u32 = 8;
/// Sector ids are u16.
pub const MAX_SECTOR_COUNT: u32 = u16::MAX as u32 + 1;

/// The stock split x of facets 0 and 1.
pub const STOCK_SPLIT_X: u32 = 5120;
/// A split x above any legal width: no chunk is split.
pub const DISABLED_SPLIT_X: u32 = 0x8000;
/// The facets the split applies to.
pub const SPLIT_FACETS: [u8; 2] = [0, 1];
/// `CMP EDI,1; JG r8; CMP [ESP+8],imm32; JGE r8; CMP EBX,imm32; JL r8; XOR AL,AL`.
const SPLIT_PATTERN: &str = "83 FF 01 7F ?? 81 7C 24 08 ?? ?? ?? ?? 7D ?? 81 FB ?? ?? ?? ?? 7C ?? 32 C0";
/// Offsets of the two imm32 inside [`SPLIT_PATTERN`].
const SPLIT_IMMEDIATE_OFFSETS: [usize; 2] = [9, 17];
/// `CMP EBX,imm32; JL -18; POP ESI; MOV AL,1`, within [`SPLIT_TAIL_WINDOW`] bytes after [`SPLIT_PATTERN`].
const SPLIT_TAIL_PATTERN: &str = "81 FB ?? ?? ?? ?? 7C EE 5E B0 01";
const SPLIT_TAIL_IMMEDIATE_OFFSET: usize = 2;
const SPLIT_TAIL_WINDOW: usize = 32;
/// The three imm32 of the split.
pub const SPLIT_IMMEDIATE_COUNT: usize = 3;
const PATTERN_WILDCARD: &str = "??";
const HEX_RADIX: u32 = 16;

/// Appended to the file name of a patched copy for its manifest.
pub const MANIFEST_SUFFIX: &str = ".facet-dims.json";
pub const MANIFEST_FORMAT: &str = "uocf-facet-dims";
pub const MANIFEST_VERSION: u32 = 1;

/// A client build whose layout was traced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownBuild {
    pub name: &'static str,
    pub sha256: &'static str,
    pub dialect: ClientDialect,
    /// File offset of the table.
    pub table_offset: usize,
    /// File offsets of the three split imm32; `None` in Kingdom Reborn.
    pub split_offsets: Option<[usize; SPLIT_IMMEDIATE_COUNT]>,
    /// File offset of the PE checksum.
    pub checksum_offset: usize,
}

/// PE checksum offset of the Enhanced Client builds and of `UOKR.exe`.
const EC_CHECKSUM_OFFSET: usize = 0x158;
const KR_CHECKSUM_OFFSET: usize = 0x168;

/// The builds of the research (`uosa_facets.md`); the `fp` folder's `UOSA_TC.exe` and `UOSA_eclo.exe` are the same
/// files as the main folder's.
pub const KNOWN_BUILDS: [KnownBuild; 5] = [
    KnownBuild {
        name: "Enhanced Client UOSA.exe",
        sha256: "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50",
        dialect: ClientDialect::Enhanced,
        table_offset: 0x9a72a8,
        split_offsets: Some([0x1df6db, 0x1df6e3, 0x1df6f5]),
        checksum_offset: EC_CHECKSUM_OFFSET,
    },
    KnownBuild {
        name: "Enhanced Client UOSA_TC.exe",
        sha256: "4e38c1cb834083c18050ca9fc85f1ebf77cf4d8bcb1a39a1f37878ea314553d0",
        dialect: ClientDialect::Enhanced,
        table_offset: 0x98f870,
        split_offsets: Some([0x1cfceb, 0x1cfcf3, 0x1cfd05]),
        checksum_offset: EC_CHECKSUM_OFFSET,
    },
    KnownBuild {
        name: "Enhanced Client UOSA_eclo.exe",
        sha256: "5aaa654c517adc3aab88467f8bc869e9b048c78c0c195caf085723fecbb8fdf0",
        dialect: ClientDialect::Enhanced,
        table_offset: 0x9a72a8,
        split_offsets: Some([0x1df6db, 0x1df6e3, 0x1df6f5]),
        checksum_offset: EC_CHECKSUM_OFFSET,
    },
    KnownBuild {
        name: "Enhanced Client fp UOSA.exe",
        sha256: "9692f6ae95dd55338985caabf247f3a2490ba2ef4d0cfe2d75694b56ba65e3dd",
        dialect: ClientDialect::Enhanced,
        table_offset: 0x9ad2e0,
        split_offsets: Some([0x1e1a0b, 0x1e1a13, 0x1e1a25]),
        checksum_offset: EC_CHECKSUM_OFFSET,
    },
    KnownBuild {
        name: "Kingdom Reborn UOKR.exe",
        sha256: "1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43",
        dialect: ClientDialect::KingdomReborn,
        table_offset: 0x7fb688,
        split_offsets: None,
        checksum_offset: KR_CHECKSUM_OFFSET,
    },
];

/// The known build with SHA-256 `sha256`.
pub fn known_build(sha256: &str) -> Option<&'static KnownBuild> {
    KNOWN_BUILDS.iter().find(|build| build.sha256 == sha256)
}

/// The split of a build: the file offsets of its three imm32 and their (equal) value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FacetSplit {
    pub offsets: [usize; SPLIT_IMMEDIATE_COUNT],
    pub x: u32,
}

/// The facet sizes and split of a build, as stored in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FacetDimsValues {
    pub facets: Vec<FacetDimsRecord>,
    /// The split x of facets 0 and 1; `None` for a build without split (Kingdom Reborn).
    pub split_x: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FacetDimsRecord {
    pub facet: u8,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// What a client executable stores about the facet sizes.
#[derive(Debug, Clone)]
pub struct ClientExeFacets {
    pub sha256: String,
    pub file_size: usize,
    pub known: Option<&'static KnownBuild>,
    pub pe: PeImage,
    /// The checksum the file should store.
    pub computed_checksum: u32,
    pub table_offset: usize,
    pub table_va: u32,
    pub dimensions: [FacetGeometry; FACET_COUNT],
    pub split: Option<FacetSplit>,
}

impl ClientExeFacets {
    /// Locates the table and the split of the client executable `bytes` (PE32 i386) by their signatures, and
    /// checks them against the known build of the same SHA-256. Fails without exactly one table, with more than
    /// one split or a split whose three values differ.
    pub fn inspect(bytes: &[u8]) -> eyre::Result<Self> {
        let pe = PeImage::parse(bytes)?;
        eyre::ensure!(pe.machine == MACHINE_I386, "COFF machine {:#x}: not an i386 image", pe.machine);
        let sha256 = sha256_hex(bytes);
        let known = known_build(&sha256);
        let tables = find_tables(bytes, &pe)?;
        let [table_offset] = tables[..] else {
            eyre::bail!("{} facet tables found, expected exactly 1", tables.len());
        };
        let table_va = pe.offset_to_va(table_offset).expect("the table lies in .data");
        let mut dimensions = CLIENT_FACET_DIMENSIONS;
        for (facet, geometry) in dimensions.iter_mut().enumerate() {
            let record = table_offset + FACET_RECORD_SIZE * facet;
            *geometry = FacetGeometry::new(
                read_u32(bytes, record + FACET_RECORD_WIDTH_OFFSET)?,
                read_u32(bytes, record + FACET_RECORD_HEIGHT_OFFSET)?,
            );
        }
        let split = find_split(bytes, &pe)?;
        if let Some(build) = known {
            eyre::ensure!(
                build.table_offset == table_offset
                    && build.split_offsets == split.map(|split| split.offsets)
                    && build.checksum_offset == pe.checksum_offset,
                "{}: the located table, split or checksum differ from the traced build",
                build.name
            );
        }
        let computed_checksum = pe_checksum(bytes, pe.checksum_offset);
        Ok(Self { sha256, file_size: bytes.len(), known, pe, computed_checksum, table_offset, table_va, dimensions, split })
    }

    /// True when the stored PE checksum is the computed one.
    pub fn checksum_valid(&self) -> bool {
        self.pe.stored_checksum == self.computed_checksum
    }

    pub fn values(&self) -> FacetDimsValues {
        FacetDimsValues {
            facets: self
                .dimensions
                .iter()
                .zip(FACET_NAMES)
                .enumerate()
                .map(|(facet, (geometry, name))| FacetDimsRecord {
                    facet: facet as u8,
                    name: name.to_owned(),
                    width: geometry.width,
                    height: geometry.height,
                })
                .collect(),
            split_x: self.split.map(|split| split.x),
        }
    }

    /// True when the table has the shipped sizes and the split (if any) is at 5120.
    pub fn is_stock(&self) -> bool {
        self.dimensions == CLIENT_FACET_DIMENSIONS && self.split.is_none_or(|split| split.x == STOCK_SPLIT_X)
    }
}

/// `"83 FF ?? ..."` as bytes, `None` for a wildcard.
fn parse_pattern(pattern: &str) -> Vec<Option<u8>> {
    pattern
        .split_whitespace()
        .map(|token| (token != PATTERN_WILDCARD).then(|| u8::from_str_radix(token, HEX_RADIX).expect("hex byte")))
        .collect()
}

fn matches_at(bytes: &[u8], at: usize, pattern: &[Option<u8>]) -> bool {
    bytes
        .get(at..at + pattern.len())
        .is_some_and(|window| window.iter().zip(pattern).all(|(byte, expected)| expected.is_none_or(|e| e == *byte)))
}

/// File offsets in `.data`, at pointer steps, of 7 records whose name pointers resolve to [`FACET_NAMES`].
fn find_tables(bytes: &[u8], pe: &PeImage) -> eyre::Result<Vec<usize>> {
    let data = pe.section(TABLE_SECTION).ok_or_else(|| eyre!("no {TABLE_SECTION} section"))?;
    let range = data.raw_range();
    let table_size = FACET_RECORD_SIZE * FACET_COUNT;
    let names_at = |table: usize| {
        FACET_NAMES.iter().enumerate().all(|(facet, name)| {
            let Ok(pointer) = read_u32(bytes, table + FACET_RECORD_SIZE * facet) else {
                return false;
            };
            pe.va_to_offset(pointer).is_some_and(|offset| {
                bytes.get(offset..offset + name.len()) == Some(name.as_bytes()) && bytes.get(offset + name.len()) == Some(&0)
            })
        })
    };
    Ok((range.start..range.end.min(bytes.len()).saturating_sub(table_size))
        .step_by(TABLE_SCAN_STEP)
        .filter(|&table| names_at(table))
        .collect())
}

/// The split of the build: `None` without the signature (Kingdom Reborn); fails on more than one hit, a hit
/// without its tail, or differing values.
fn find_split(bytes: &[u8], pe: &PeImage) -> eyre::Result<Option<FacetSplit>> {
    let text = pe.section(CODE_SECTION).ok_or_else(|| eyre!("no {CODE_SECTION} section"))?;
    let range = text.raw_range();
    let (pattern, tail) = (parse_pattern(SPLIT_PATTERN), parse_pattern(SPLIT_TAIL_PATTERN));
    let hits: Vec<usize> = (range.start..range.end.min(bytes.len())).filter(|&at| matches_at(bytes, at, &pattern)).collect();
    let hit = match hits[..] {
        [] => return Ok(None),
        [hit] => hit,
        _ => eyre::bail!("{} x = 5120 split signatures found, expected at most 1", hits.len()),
    };
    let after = hit + pattern.len();
    let tail_at = (after..after + SPLIT_TAIL_WINDOW - tail.len() + 1)
        .find(|&at| matches_at(bytes, at, &tail))
        .ok_or_else(|| eyre!("the split signature at {hit:#x} has no tail compare"))?;
    let [first, second] = SPLIT_IMMEDIATE_OFFSETS.map(|offset| hit + offset);
    let offsets = [first, second, tail_at + SPLIT_TAIL_IMMEDIATE_OFFSET];
    let values = offsets.map(|offset| read_u32(bytes, offset).expect("inside the matched pattern"));
    eyre::ensure!(values.iter().all(|value| *value == values[0]), "the three split values differ: {values:?}");
    Ok(Some(FacetSplit { offsets, x: values[0] }))
}

/// What a patch does to the split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitEdit {
    Keep,
    /// Moves the split of facets 0 and 1 to this x.
    Set(u32),
    /// Sets the split to [`DISABLED_SPLIT_X`].
    Disable,
}

/// A new size for one facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FacetDimsPatch {
    pub facet: u8,
    pub width: u32,
    pub height: u32,
    pub split: SplitEdit,
}

/// The record written next to a patched copy (`<copy>.facet-dims.json`): the stock build it derives from, the
/// build it was patched from and the result, and the values of the result. A later patch of the copy needs it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FacetDimsManifest {
    pub format: String,
    pub version: u32,
    /// The known build the chain started from, when it is one.
    pub build: Option<String>,
    /// SHA-256 of the stock executable the chain of patches started from.
    pub original_sha256: String,
    /// SHA-256 of the executable this patch was applied to.
    pub source_sha256: String,
    /// SHA-256 of the patched copy.
    pub patched_sha256: String,
    pub values: FacetDimsValues,
    /// The PE checksum written.
    pub checksum: u32,
}

/// A patched executable.
#[derive(Debug, Clone)]
pub struct PatchedClientExe {
    pub bytes: Vec<u8>,
    pub before: ClientExeFacets,
    pub after: ClientExeFacets,
    /// File offsets of the bytes that differ from the input.
    pub changed_offsets: Vec<usize>,
    /// Allowed but unusual requests.
    pub warnings: Vec<String>,
    pub manifest: FacetDimsManifest,
}

/// Fails on a size the client cannot hold; returns the warnings of an allowed but unusual one.
pub fn validate_facet_size(width: u32, height: u32) -> eyre::Result<Vec<String>> {
    for (what, side) in [("width", width), ("height", height)] {
        eyre::ensure!((1..=MAX_FACET_SIDE).contains(&side), "{what} {side} is outside 1..={MAX_FACET_SIDE}");
        eyre::ensure!(side % FACET_SIDE_ALIGNMENT == 0, "{what} {side} is not a multiple of {FACET_SIDE_ALIGNMENT}");
    }
    let sectors = FacetGeometry::new(width, height).sector_count();
    eyre::ensure!(sectors <= MAX_SECTOR_COUNT, "{width}x{height} needs {sectors} sectors, at most {MAX_SECTOR_COUNT}");
    Ok([("width", width), ("height", height)]
        .into_iter()
        .filter(|(_, side)| side % SECTOR_SIDE != 0)
        .map(|(what, side)| format!("{what} {side} is not a multiple of {SECTOR_SIDE}: the last sectors are padded"))
        .collect())
}

/// Patches the size of one facet (and optionally the split) in a copy of the executable `bytes` and recomputes the
/// PE checksum. The input must have the stock values, or the values `previous` (the manifest of the copy) records
/// for it. Refuses a signed build. Only the record's width and height, the split imm32 and the checksum change.
pub fn patch_facet_dimensions(
    bytes: &[u8],
    patch: &FacetDimsPatch,
    previous: Option<&FacetDimsManifest>,
) -> eyre::Result<PatchedClientExe> {
    let facet = usize::from(patch.facet);
    eyre::ensure!(facet < FACET_COUNT, "facet {facet}: the client has facets 0..{FACET_COUNT}");
    let before = ClientExeFacets::inspect(bytes)?;
    eyre::ensure!(!before.pe.is_signed(), "the executable is Authenticode signed; a patch would break the signature");
    let previous = previous.filter(|manifest| manifest.patched_sha256 == before.sha256);
    let (original_sha256, build) = match previous {
        Some(manifest) => {
            eyre::ensure!(
                manifest.values == before.values(),
                "the executable's facet values differ from the ones its manifest records"
            );
            (manifest.original_sha256.clone(), manifest.build.clone())
        }
        None => {
            eyre::ensure!(
                before.is_stock(),
                "the executable's facet values are not the stock ones and no manifest of an earlier patch records them"
            );
            (before.sha256.clone(), before.known.map(|build| build.name.to_owned()))
        }
    };
    let mut warnings = validate_facet_size(patch.width, patch.height)?;
    let split_x = match patch.split {
        SplitEdit::Keep => None,
        SplitEdit::Set(x) => Some(x),
        SplitEdit::Disable => Some(DISABLED_SPLIT_X),
    };
    let mut patched = bytes.to_vec();
    let record = before.table_offset + FACET_RECORD_SIZE * facet;
    let mut allowed = vec![record + FACET_RECORD_WIDTH_OFFSET..record + FACET_RECORD_SIZE];
    patched[record + FACET_RECORD_WIDTH_OFFSET..][..size_of::<u32>()].copy_from_slice(&patch.width.to_le_bytes());
    patched[record + FACET_RECORD_HEIGHT_OFFSET..][..size_of::<u32>()].copy_from_slice(&patch.height.to_le_bytes());
    if let Some(x) = split_x {
        eyre::ensure!(
            SPLIT_FACETS.contains(&patch.facet),
            "the split concerns facets {SPLIT_FACETS:?}, not facet {facet}"
        );
        let split = before.split.ok_or_else(|| eyre!("this build has no x = 5120 split (Kingdom Reborn)"))?;
        eyre::ensure!((1..=DISABLED_SPLIT_X).contains(&x), "split x {x} is outside 1..={DISABLED_SPLIT_X}");
        if x != DISABLED_SPLIT_X && x >= patch.width {
            warnings.push(format!("split x {x} is not inside the new width {}: nothing is split", patch.width));
        }
        for offset in split.offsets {
            patched[offset..offset + size_of::<u32>()].copy_from_slice(&x.to_le_bytes());
            allowed.push(offset..offset + size_of::<u32>());
        }
    }
    let checksum_offset = before.pe.checksum_offset;
    let checksum = pe_checksum(&patched, checksum_offset);
    patched[checksum_offset..checksum_offset + size_of::<u32>()].copy_from_slice(&checksum.to_le_bytes());
    allowed.push(checksum_offset..checksum_offset + size_of::<u32>());

    let changed_offsets: Vec<usize> =
        bytes.iter().zip(&patched).enumerate().filter(|(_, (old, new))| old != new).map(|(offset, _)| offset).collect();
    let stray = changed_offsets.iter().find(|offset| !allowed.iter().any(|range| range.contains(offset)));
    eyre::ensure!(stray.is_none(), "internal error: byte {stray:?} changed outside the patched fields");
    let after = ClientExeFacets::inspect(&patched)?;
    eyre::ensure!(after.checksum_valid(), "internal error: the written checksum does not verify");
    let manifest = FacetDimsManifest {
        format: MANIFEST_FORMAT.to_owned(),
        version: MANIFEST_VERSION,
        build,
        original_sha256,
        source_sha256: before.sha256.clone(),
        patched_sha256: after.sha256.clone(),
        values: after.values(),
        checksum,
    };
    Ok(PatchedClientExe { bytes: patched, before, after, changed_offsets, warnings, manifest })
}

/// The manifest path of the patched copy `exe`: its file name plus [`MANIFEST_SUFFIX`].
pub fn manifest_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(MANIFEST_SUFFIX);
    exe.with_file_name(name)
}

/// The manifest of `exe`, when it has one.
pub fn read_manifest(exe: &Path) -> eyre::Result<Option<FacetDimsManifest>> {
    let path = manifest_path(exe);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).wrap_err_with(|| format!("read {}", path.display()))?;
    let manifest: FacetDimsManifest =
        serde_json::from_str(&text).wrap_err_with(|| format!("parse {}", path.display()))?;
    eyre::ensure!(
        manifest.format == MANIFEST_FORMAT && manifest.version == MANIFEST_VERSION,
        "{}: not a {MANIFEST_FORMAT} version {MANIFEST_VERSION} manifest",
        path.display()
    );
    Ok(Some(manifest))
}

/// Fails unless `output` is a new file outside the folder of `input` (the client folder), and its manifest path is
/// free too.
fn check_output_path(input: &Path, output: &Path) -> eyre::Result<()> {
    let input = std::fs::canonicalize(input).wrap_err_with(|| format!("resolve {}", input.display()))?;
    let client_dir = input.parent().ok_or_else(|| eyre!("{} has no folder", input.display()))?;
    for path in [output.to_path_buf(), manifest_path(output)] {
        eyre::ensure!(path.symlink_metadata().is_err(), "{} exists: write to a new path", path.display());
    }
    let output_dir = match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let output_dir =
        std::fs::canonicalize(output_dir).wrap_err_with(|| format!("resolve the folder of {}", output.display()))?;
    eyre::ensure!(
        !output_dir.starts_with(client_dir),
        "refusing to write inside the folder of the input executable ({}): write the copy elsewhere",
        client_dir.display()
    );
    Ok(())
}

/// Patches a copy of `input` to the new file `output` (outside the input's folder) and writes its manifest
/// `<output>.facet-dims.json`. The input is only read; when it is an earlier patched copy, its manifest is read too.
pub fn write_patched_client_exe(input: &Path, output: &Path, patch: &FacetDimsPatch) -> eyre::Result<PatchedClientExe> {
    check_output_path(input, output)?;
    let bytes = std::fs::read(input).wrap_err_with(|| format!("read {}", input.display()))?;
    let previous = read_manifest(input)?;
    let patched = patch_facet_dimensions(&bytes, patch, previous.as_ref())?;
    let manifest = serde_json::to_string_pretty(&patched.manifest)?;
    std::fs::write(output, &patched.bytes).wrap_err_with(|| format!("write {}", output.display()))?;
    let manifest_file = manifest_path(output);
    std::fs::write(&manifest_file, manifest).wrap_err_with(|| format!("write {}", manifest_file.display()))?;
    Ok(patched)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::client_exe::pe::tests::{synthetic_pe, TEST_IMAGE_BASE, TEST_SECTION_ALIGNMENT};

    /// Offset of the table inside the synthetic `.data`.
    const TABLE_DATA_OFFSET: usize = 16;
    const NEW_WIDTH: u32 = 8192;
    const NEW_HEIGHT: u32 = 4608;
    const NEW_SPLIT: u32 = 6144;

    fn split_code(x: u32) -> Vec<u8> {
        let mut code = vec![0x90; 8];
        let pattern = parse_pattern(SPLIT_PATTERN);
        let start = code.len();
        code.extend(pattern.iter().map(|byte| byte.unwrap_or(0)));
        for offset in SPLIT_IMMEDIATE_OFFSETS {
            code[start + offset..start + offset + 4].copy_from_slice(&x.to_le_bytes());
        }
        code.extend([0x90; 3]);
        let tail = code.len();
        code.extend(parse_pattern(SPLIT_TAIL_PATTERN).iter().map(|byte| byte.unwrap_or(0)));
        code[tail + SPLIT_TAIL_IMMEDIATE_OFFSET..][..4].copy_from_slice(&x.to_le_bytes());
        code.extend([0xc3; 8]);
        code
    }

    /// A client-like image: `.text` (with the split when `split` is set), `.rdata` with the names, `.data` with
    /// the stock table.
    pub fn synthetic_client(split: Option<u32>, signed: bool) -> Vec<u8> {
        let text = split.map_or_else(|| vec![0x90; 64], split_code);
        // The sections are laid out at aligned offsets equal to their RVAs: .text, then .rdata, then .data.
        let rdata_rva = TEST_SECTION_ALIGNMENT * (1 + (text.len() as u32).div_ceil(TEST_SECTION_ALIGNMENT));
        let mut rdata = Vec::new();
        let mut pointers = Vec::new();
        for name in FACET_NAMES {
            pointers.push(TEST_IMAGE_BASE + rdata_rva + rdata.len() as u32);
            rdata.extend_from_slice(name.as_bytes());
            rdata.push(0);
        }
        let mut data = vec![0u8; TABLE_DATA_OFFSET];
        for (pointer, geometry) in pointers.iter().zip(CLIENT_FACET_DIMENSIONS) {
            for value in [*pointer, geometry.width, geometry.height] {
                data.extend_from_slice(&value.to_le_bytes());
            }
        }
        // Room for a second table, so a test can copy one there.
        data.extend(vec![0; FACET_RECORD_SIZE * FACET_COUNT + TABLE_DATA_OFFSET]);
        let mut image = synthetic_pe(&[(".text", text), (".rdata", rdata), (".data", data)], signed);
        let pe = PeImage::parse(&image).unwrap();
        let checksum = pe_checksum(&image, pe.checksum_offset);
        image[pe.checksum_offset..pe.checksum_offset + 4].copy_from_slice(&checksum.to_le_bytes());
        image
    }

    #[test]
    fn table_and_split_are_located() {
        let image = synthetic_client(Some(STOCK_SPLIT_X), false);
        let facets = ClientExeFacets::inspect(&image).unwrap();
        assert!(facets.known.is_none());
        assert_eq!(facets.dimensions, CLIENT_FACET_DIMENSIONS);
        assert_eq!(facets.split.unwrap().x, STOCK_SPLIT_X);
        assert!(facets.is_stock() && facets.checksum_valid());
        assert_eq!(facets.table_offset, facets.pe.section(TABLE_SECTION).unwrap().raw_offset as usize + TABLE_DATA_OFFSET);

        let no_split = ClientExeFacets::inspect(&synthetic_client(None, false)).unwrap();
        assert!(no_split.split.is_none() && no_split.is_stock());

        let mut unequal = image.clone();
        let offset = facets.split.unwrap().offsets[2];
        unequal[offset] ^= 1;
        assert!(ClientExeFacets::inspect(&unequal).is_err(), "differing split values");
        let mut two_tables = image.clone();
        let (table, table_size) = (facets.table_offset, FACET_RECORD_SIZE * FACET_COUNT);
        two_tables.copy_within(table..table + table_size, table + table_size);
        assert!(ClientExeFacets::inspect(&two_tables).is_err(), "two tables");
    }

    #[test]
    fn a_patch_changes_only_the_record_the_split_and_the_checksum() {
        let image = synthetic_client(Some(STOCK_SPLIT_X), false);
        let patch = FacetDimsPatch { facet: 0, width: NEW_WIDTH, height: NEW_HEIGHT, split: SplitEdit::Set(NEW_SPLIT) };
        let patched = patch_facet_dimensions(&image, &patch, None).unwrap();
        assert_eq!(patched.after.dimensions[0], FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT));
        assert_eq!(patched.after.dimensions[1..], CLIENT_FACET_DIMENSIONS[1..]);
        assert_eq!(patched.after.split.unwrap().x, NEW_SPLIT);
        assert!(patched.after.checksum_valid() && !patched.after.is_stock());
        assert!(patched.warnings.is_empty());
        assert_eq!(patched.manifest.original_sha256, sha256_hex(&image));
        assert_eq!(patched.manifest.patched_sha256, sha256_hex(&patched.bytes));
        let record = patched.before.table_offset;
        assert!(patched.changed_offsets.iter().all(|offset| {
            (record + FACET_RECORD_WIDTH_OFFSET..record + FACET_RECORD_SIZE).contains(offset)
                || patched.before.split.unwrap().offsets.iter().any(|imm| (*imm..*imm + 4).contains(offset))
                || (patched.before.pe.checksum_offset..patched.before.pe.checksum_offset + 4).contains(offset)
        }));

        // A patched copy needs its manifest; with it, it patches again and keeps the original hash.
        let again = FacetDimsPatch { facet: 2, width: 2304, height: 1608, split: SplitEdit::Keep };
        assert!(patch_facet_dimensions(&patched.bytes, &again, None).is_err());
        let chained = patch_facet_dimensions(&patched.bytes, &again, Some(&patched.manifest)).unwrap();
        assert_eq!(chained.manifest.original_sha256, patched.manifest.original_sha256);
        assert_eq!(chained.manifest.source_sha256, patched.manifest.patched_sha256);
        assert_eq!(chained.warnings.len(), 1, "1608 is not a multiple of 64");
        let disabled = FacetDimsPatch { split: SplitEdit::Disable, ..patch };
        assert_eq!(patch_facet_dimensions(&image, &disabled, None).unwrap().after.split.unwrap().x, DISABLED_SPLIT_X);
    }

    #[test]
    fn bad_requests_and_builds_are_refused() {
        let image = synthetic_client(Some(STOCK_SPLIT_X), false);
        let good = FacetDimsPatch { facet: 3, width: NEW_WIDTH, height: NEW_HEIGHT, split: SplitEdit::Keep };
        for bad in [
            FacetDimsPatch { facet: FACET_COUNT as u8, ..good },
            FacetDimsPatch { width: MAX_FACET_SIDE + SECTOR_SIDE, ..good },
            FacetDimsPatch { width: NEW_WIDTH + FACET_SIDE_ALIGNMENT / 2, ..good },
            FacetDimsPatch { width: 0, ..good },
            FacetDimsPatch { width: MAX_FACET_SIDE, height: MAX_FACET_SIDE, ..good },
            FacetDimsPatch { split: SplitEdit::Set(NEW_SPLIT), ..good },
            FacetDimsPatch { facet: 0, split: SplitEdit::Set(DISABLED_SPLIT_X + 1), ..good },
        ] {
            assert!(patch_facet_dimensions(&image, &bad, None).is_err(), "{bad:?}");
        }
        assert!(patch_facet_dimensions(&image, &good, None).is_ok());
        let kr_like = synthetic_client(None, false);
        let split = FacetDimsPatch { facet: 0, split: SplitEdit::Disable, ..good };
        assert!(patch_facet_dimensions(&kr_like, &split, None).is_err(), "no split to patch");
        assert!(patch_facet_dimensions(&synthetic_client(Some(STOCK_SPLIT_X), true), &good, None).is_err(), "signed");
    }

    #[test]
    fn the_copy_goes_to_a_new_path_outside_the_client_folder() {
        let root = std::env::temp_dir().join(format!("uocf_facet_dims_{}", std::process::id()));
        let (client, copies) = (root.join("client"), root.join("copies"));
        std::fs::create_dir_all(&client).unwrap();
        std::fs::create_dir_all(&copies).unwrap();
        let exe = client.join("UOSA.exe");
        std::fs::write(&exe, synthetic_client(Some(STOCK_SPLIT_X), false)).unwrap();
        let patch = FacetDimsPatch { facet: 5, width: 1280, height: 4160, split: SplitEdit::Keep };
        assert!(write_patched_client_exe(&exe, &exe, &patch).is_err());
        assert!(write_patched_client_exe(&exe, &client.join("copy.exe"), &patch).is_err());
        let output = copies.join("UOSA.exe");
        let patched = write_patched_client_exe(&exe, &output, &patch).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), patched.bytes);
        assert_eq!(read_manifest(&output).unwrap().unwrap(), patched.manifest);
        assert!(write_patched_client_exe(&exe, &output, &patch).is_err(), "the copy exists");
        let second = root.join("second.exe");
        let again = write_patched_client_exe(&output, &second, &FacetDimsPatch { facet: 6, width: 512, height: 512, ..patch });
        assert_eq!(again.unwrap().manifest.original_sha256, patched.manifest.original_sha256);
        let _ = std::fs::remove_dir_all(root);
    }
}
