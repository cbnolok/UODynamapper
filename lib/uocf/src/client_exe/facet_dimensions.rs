//! The facet size table of the client executables (`UOFacet::g_FacetDimensions`) and the hard-coded Felucca/Trammel
//! split at x = 5120: locate, report and patch them in a copy of the executable.
//!
//! Traced in `scripts/dev-helpers/ec-research/ghidra-facets/results/uosa_facets.md` ("Other dimension sites",
//! "Constraints", "Builds and the cross-build patch recipe") and `uokr_facets.md`. Both are located by structure,
//! in any build; the SHA-256 of a traced build ([`KNOWN_BUILDS`]) is only a label:
//!
//! - The table is a run of 12-byte records `{char* name, u32 width, u32 height}` in a data section: the name
//!   pointer resolves to a short NUL-terminated ASCII name in a readable section, and the sizes are legal facet
//!   sizes. Exactly one run of [`MIN_FACET_RECORDS`]..=[`MAX_FACET_RECORDS`] records is required (the shipped
//!   builds have 7: Felucca, Trammel, Ilshenar, Malas, Tokuno, Gargoyle, Test). The client sizes its sector grid
//!   and chunk bounds from it. Classic 2D clients keep their map sizes in another, 48-byte record table, which
//!   this locator does not match.
//! - `UOFacet::IsSameFeluccaTrammelHalf` (UOSA 0x005df670) returns false, for facets 0 and 1, when two x lie on
//!   different sides of 5120; `BuildChunkFromSector` then does not build the chunk. The 5120 is three imm32 in one
//!   `.text` sequence, found by decoding its instructions ([`SPLIT_SIGNATURE`]); 0x8000, above any legal width,
//!   disables the split. Kingdom Reborn has no split.
//! - The PE checksum is recomputed (the client does not verify it: `UOSA_eclo.exe` ships a wrong one and runs).
//!   A signed (Authenticode) build is refused, since any patch breaks its signature; no shipped build is signed.
//!
//! The executable alone is not enough: the facet data must have the new size too ([`super::facet_data`]).

crate::eyre_imports!();
use std::path::{Path, PathBuf};

use crate::client_exe::pe::{pe_checksum, read_u32, PeImage, PeSection, MACHINE_I386};
use crate::client_exe::sha256::sha256_hex;
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::facet_sector::{FacetGeometry, CLIENT_FACET_DIMENSIONS, SECTOR_SIDE};

/// Records of the shipped tables (Enhanced Client and Kingdom Reborn); every reader there rejects a facet >= 7.
pub const STOCK_FACET_COUNT: usize = CLIENT_FACET_DIMENSIONS.len();
/// The fewest records of a table: Felucca, Trammel, Ilshenar and Malas exist in every client since Age of Shadows.
pub const MIN_FACET_RECORDS: usize = 4;
/// The most records of a table: the facet nibble of the client's chunk key holds 16 facets.
pub const MAX_FACET_RECORDS: usize = 16;
/// The name pointer of a record (32-bit images only).
const NAME_POINTER_SIZE: usize = size_of::<u32>();
/// Bytes of a table record: `char* name, u32 width, u32 height`.
pub const FACET_RECORD_SIZE: usize = NAME_POINTER_SIZE + 2 * size_of::<u32>();
const FACET_RECORD_WIDTH_OFFSET: usize = NAME_POINTER_SIZE;
const FACET_RECORD_HEIGHT_OFFSET: usize = FACET_RECORD_WIDTH_OFFSET + size_of::<u32>();
/// The table is pointer aligned.
const TABLE_SCAN_STEP: usize = NAME_POINTER_SIZE;
/// Facet names are short ASCII words ("Felucca", "Ter Mur"): a letter, then letters, digits or
/// [`FACET_NAME_PUNCTUATION`], NUL terminated.
const MIN_FACET_NAME_LEN: usize = 2;
const MAX_FACET_NAME_LEN: usize = 31;
const FACET_NAME_PUNCTUATION: &[u8] = b" _'-";
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
/// The three imm32 of the split.
pub const SPLIT_IMMEDIATE_COUNT: usize = 3;
/// The split sequence the matcher decodes (`r`, `m`: any register or memory operand; `X`: the same operand in
/// both places; `jcc`: a short or near jump; the three imm32 must be equal):
///
/// ```text
/// CMP r/m,1      ; JG|JA   T       facet > 1: same half
/// CMP r/m,imm32  ; JGE|JAE B       x1 >= split
/// CMP X,imm32    ; JL|JB   T       x1 < split, x2 < split: same half
/// F: XOR AL,AL   ...               different halves
/// B: CMP X,imm32 ; JL|JB   F       x1 >= split, x2 < split: different halves
/// T: POP r ...   ; MOV AL,1        same half
/// ```
///
/// UOSA: `83 FF 01 7F 24 81 7C 24 08 [imm32] 7D 12 81 FB [imm32] 7C 12 32 C0 .. 81 FB [imm32] 7C EE 5E B0 01`.
pub const SPLIT_SIGNATURE: &str = "CMP r/m,1; JG T; CMP r/m,S; JGE B; CMP X,S; JL T; F: XOR AL,AL; .. \
                                   B: CMP X,S; JL F; .. T: POP*; MOV AL,1";

/// x86 (32-bit) encodings the split matcher decodes.
const OPCODE_CMP_RM32_IMM32: u8 = 0x81;
const OPCODE_CMP_RM32_IMM8: u8 = 0x83;
const OPCODE_CMP_EAX_IMM32: u8 = 0x3d;
/// The ModRM reg field of CMP in the 0x81 / 0x83 opcode groups.
const MODRM_REG_CMP: u8 = 7;
const MODRM_MOD_SHIFT: u8 = 6;
const MODRM_REG_SHIFT: u8 = 3;
const MODRM_FIELD_MASK: u8 = 0b111;
const MODRM_REG_FIELD: u8 = MODRM_FIELD_MASK << MODRM_REG_SHIFT;
const MOD_INDIRECT: u8 = 0;
const MOD_DISP8: u8 = 1;
const MOD_DISP32: u8 = 2;
const MOD_REGISTER: u8 = 3;
/// rm = 100: a SIB byte follows; rm = 101 with mod 00 (or SIB base 101 with mod 00): a disp32 without base.
const RM_SIB: u8 = 0b100;
const RM_DISP32: u8 = 0b101;
const REGISTER_EAX: u8 = 0;
const OPCODE_JCC_SHORT: u8 = 0x70;
const OPCODE_TWO_BYTE: u8 = 0x0f;
const OPCODE_JCC_NEAR: u8 = 0x80;
const JCC_CONDITION_MASK: u8 = 0x0f;
const JCC_SHORT_SIZE: usize = 2;
const JCC_NEAR_SIZE: usize = 6;
/// Jcc conditions: below, above or equal, above, less, greater or equal, greater.
const CONDITION_B: u8 = 0x2;
const CONDITION_AE: u8 = 0x3;
const CONDITION_A: u8 = 0x7;
const CONDITION_L: u8 = 0xc;
const CONDITION_GE: u8 = 0xd;
const CONDITION_G: u8 = 0xf;
/// The facet compare: `facet > 1` is outside [`SPLIT_FACETS`].
const SPLIT_FACET_LIMIT: u8 = 1;
/// `XOR AL,AL` (both encodings) or `XOR EAX,EAX`: the false result.
const CLEAR_RESULT: [[u8; 2]; 3] = [[0x32, 0xc0], [0x30, 0xc0], [0x33, 0xc0]];
/// `MOV AL,1`: the true result, after at most [`MAX_POPS_BEFORE_RESULT`] one-byte `POP r32`.
const SET_RESULT: [u8; 2] = [0xb0, 0x01];
const POP_REGISTER_OPCODES: std::ops::RangeInclusive<u8> = 0x58..=0x5f;
const MAX_POPS_BEFORE_RESULT: usize = 4;

/// Appended to the file name of a patched copy for its manifest.
pub const MANIFEST_SUFFIX: &str = ".facet-dims.json";
pub const MANIFEST_FORMAT: &str = "uocf-facet-dims";
pub const MANIFEST_VERSION: u32 = 1;

/// A client build whose layout was traced. Only a label for reports and a reference for the tests: the table and
/// the split are always located by structure.
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

/// One record of a located table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetTableRecord {
    /// The name the record points to.
    pub name: String,
    pub geometry: FacetGeometry,
}

/// A run of facet records in a data section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetTable {
    pub section: String,
    /// File offset and virtual address of the first record.
    pub offset: usize,
    pub va: u32,
    pub records: Vec<FacetTableRecord>,
}

/// One match of [`SPLIT_SIGNATURE`]: the file offsets of the facet compare and of the three imm32, and their values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitCandidate {
    pub signature_offset: usize,
    pub offsets: [usize; SPLIT_IMMEDIATE_COUNT],
    pub values: [u32; SPLIT_IMMEDIATE_COUNT],
}

/// The split of a build: the file offsets of its three imm32 and their (equal) value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FacetSplit {
    /// File offset of the facet compare that starts [`SPLIT_SIGNATURE`].
    pub signature_offset: usize,
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
    /// The traced build of the same SHA-256, as a label.
    pub known: Option<&'static KnownBuild>,
    pub pe: PeImage,
    /// The checksum the file should store.
    pub computed_checksum: u32,
    pub table: FacetTable,
    pub split: Option<FacetSplit>,
}

impl ClientExeFacets {
    /// Locates the table and the split of the client executable `bytes` (PE32 i386) by their structure. Fails
    /// without exactly one table, with more than one split or a split whose three values differ. The SHA-256 only
    /// selects the [`KnownBuild`] label.
    pub fn inspect(bytes: &[u8]) -> eyre::Result<Self> {
        let pe = PeImage::parse(bytes)?;
        eyre::ensure!(pe.machine == MACHINE_I386, "COFF machine {:#x}: not an i386 image", pe.machine);
        let mut tables = locate_facet_tables(bytes, &pe);
        eyre::ensure!(
            tables.len() == 1,
            "{} facet size tables found, expected exactly 1 (a run of {MIN_FACET_RECORDS}..={MAX_FACET_RECORDS} \
             {FACET_RECORD_SIZE}-byte {{name pointer, width, height}} records; Classic 2D clients store their map \
             sizes differently and are not supported)",
            tables.len()
        );
        let table = tables.remove(0);
        let split = match locate_split_candidates(bytes, &pe)?[..] {
            [] => None,
            [candidate] => {
                let [first, ..] = candidate.values;
                eyre::ensure!(
                    candidate.values.iter().all(|value| *value == first),
                    "the three split values at {:#x} differ: {:?}",
                    candidate.signature_offset,
                    candidate.values
                );
                Some(FacetSplit { signature_offset: candidate.signature_offset, offsets: candidate.offsets, x: first })
            }
            ref candidates => eyre::bail!("{} x = 5120 split sequences found, expected at most 1", candidates.len()),
        };
        let sha256 = sha256_hex(bytes);
        let known = known_build(&sha256);
        let computed_checksum = pe_checksum(bytes, pe.checksum_offset);
        Ok(Self { sha256, file_size: bytes.len(), known, pe, computed_checksum, table, split })
    }

    /// Records of the table.
    pub fn facet_count(&self) -> usize {
        self.table.records.len()
    }

    pub fn dimensions(&self) -> Vec<FacetGeometry> {
        self.table.records.iter().map(|record| record.geometry).collect()
    }

    /// True when the stored PE checksum is the computed one.
    pub fn checksum_valid(&self) -> bool {
        self.pe.stored_checksum == self.computed_checksum
    }

    pub fn values(&self) -> FacetDimsValues {
        FacetDimsValues {
            facets: self
                .table
                .records
                .iter()
                .enumerate()
                .map(|(facet, record)| FacetDimsRecord {
                    facet: facet as u8,
                    name: record.name.clone(),
                    width: record.geometry.width,
                    height: record.geometry.height,
                })
                .collect(),
            split_x: self.split.map(|split| split.x),
        }
    }

    /// True when every record has the shipped size of its facet ([`CLIENT_FACET_DIMENSIONS`]) and the split (if
    /// any) is at 5120.
    pub fn is_stock(&self) -> bool {
        self.facet_count() <= STOCK_FACET_COUNT
            && self.table.records.iter().zip(CLIENT_FACET_DIMENSIONS).all(|(record, stock)| record.geometry == stock)
            && self.split.is_none_or(|split| split.x == STOCK_SPLIT_X)
    }
}

/// A legal facet width or height.
fn plausible_side(side: u32) -> bool {
    (1..=MAX_FACET_SIDE).contains(&side) && side % FACET_SIDE_ALIGNMENT == 0
}

/// The facet name at virtual address `va`: NUL-terminated ASCII in a readable section (see [`MIN_FACET_NAME_LEN`]).
fn facet_name_at(bytes: &[u8], pe: &PeImage, va: u32) -> Option<String> {
    let section = pe.section_of_va(va).filter(|section| section.is_readable())?;
    let start = section.raw_offset as usize + (va - section.virtual_address) as usize;
    let end = section.raw_range().end.min(bytes.len()).min(start + MAX_FACET_NAME_LEN + 1);
    let text = bytes.get(start..end)?;
    let len = text.iter().position(|byte| *byte == 0)?;
    let name = &text[..len];
    let valid = len >= MIN_FACET_NAME_LEN
        && name[0].is_ascii_alphabetic()
        && name.iter().all(|byte| byte.is_ascii_alphanumeric() || FACET_NAME_PUNCTUATION.contains(byte));
    valid.then(|| String::from_utf8_lossy(name).into_owned())
}

/// The record at file offset `offset` of `section`, when it is one.
fn facet_record_at(bytes: &[u8], pe: &PeImage, section: &PeSection, offset: usize) -> Option<FacetTableRecord> {
    if offset + FACET_RECORD_SIZE > section.raw_range().end.min(bytes.len()) {
        return None;
    }
    let width = read_u32(bytes, offset + FACET_RECORD_WIDTH_OFFSET).ok()?;
    let height = read_u32(bytes, offset + FACET_RECORD_HEIGHT_OFFSET).ok()?;
    if !(plausible_side(width) && plausible_side(height)) {
        return None;
    }
    let name = facet_name_at(bytes, pe, read_u32(bytes, offset).ok()?)?;
    Some(FacetTableRecord { name, geometry: FacetGeometry::new(width, height) })
}

/// Every candidate table: the maximal runs of [`MIN_FACET_RECORDS`]..=[`MAX_FACET_RECORDS`] consecutive records, at
/// pointer steps, in the data sections.
pub fn locate_facet_tables(bytes: &[u8], pe: &PeImage) -> Vec<FacetTable> {
    let mut tables = Vec::new();
    for section in pe.sections.iter().filter(|section| section.is_data()) {
        let range = section.raw_range();
        for start in (range.start..range.end.min(bytes.len())).step_by(TABLE_SCAN_STEP) {
            let continues_a_run = start >= range.start + FACET_RECORD_SIZE
                && facet_record_at(bytes, pe, section, start - FACET_RECORD_SIZE).is_some();
            if continues_a_run {
                continue;
            }
            let records: Vec<FacetTableRecord> = (0..)
                .map_while(|index| facet_record_at(bytes, pe, section, start + FACET_RECORD_SIZE * index))
                .take(MAX_FACET_RECORDS + 1)
                .collect();
            if (MIN_FACET_RECORDS..=MAX_FACET_RECORDS).contains(&records.len()) {
                let va = pe.offset_to_va(start).expect("the table lies in its section");
                tables.push(FacetTable { section: section.name.clone(), offset: start, va, records });
            }
        }
    }
    tables
}

/// The operand of a decoded compare: a register, or the ModRM (reg field cleared), SIB and displacement bytes of a
/// memory operand. Equal operands are equal locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operand<'a> {
    Register(u8),
    Memory(u8, &'a [u8]),
}

/// The r/m operand of the ModRM byte at `at`: its ModRM reg field, the operand and the bytes ModRM, SIB and
/// displacement take.
fn decode_modrm(code: &[u8], at: usize) -> Option<(u8, Operand<'_>, usize)> {
    let modrm = *code.get(at)?;
    let (mode, reg, rm) =
        (modrm >> MODRM_MOD_SHIFT, (modrm >> MODRM_REG_SHIFT) & MODRM_FIELD_MASK, modrm & MODRM_FIELD_MASK);
    if mode == MOD_REGISTER {
        return Some((reg, Operand::Register(rm), 1));
    }
    let mut len = 1;
    let mut absolute = mode == MOD_INDIRECT && rm == RM_DISP32;
    if rm == RM_SIB {
        let base = *code.get(at + len)? & MODRM_FIELD_MASK;
        absolute = mode == MOD_INDIRECT && base == RM_DISP32;
        len += 1;
    }
    len += match mode {
        MOD_DISP8 => size_of::<u8>(),
        MOD_DISP32 => size_of::<u32>(),
        _ if absolute => size_of::<u32>(),
        _ => 0,
    };
    let rest = code.get(at + 1..at + len)?;
    Some((reg, Operand::Memory(modrm & !MODRM_REG_FIELD, rest), len))
}

/// A decoded `CMP r/m32, imm32` (or `CMP EAX, imm32`).
struct CompareImm32<'a> {
    operand: Operand<'a>,
    imm_offset: usize,
    imm: u32,
    end: usize,
}

fn decode_cmp_imm32(code: &[u8], at: usize) -> Option<CompareImm32<'_>> {
    let (operand, imm_offset) = match *code.get(at)? {
        OPCODE_CMP_EAX_IMM32 => (Operand::Register(REGISTER_EAX), at + 1),
        OPCODE_CMP_RM32_IMM32 => {
            let (reg, operand, len) = decode_modrm(code, at + 1)?;
            (reg == MODRM_REG_CMP).then_some((operand, at + 1 + len))?
        }
        _ => return None,
    };
    let imm = u32::from_le_bytes(code.get(imm_offset..imm_offset + size_of::<u32>())?.try_into().ok()?);
    Some(CompareImm32 { operand, imm_offset, imm, end: imm_offset + size_of::<u32>() })
}

/// The end of a `CMP r/m32, imm8` with immediate `expected` at `at`.
fn decode_cmp_imm8(code: &[u8], at: usize, expected: u8) -> Option<usize> {
    (*code.get(at)? == OPCODE_CMP_RM32_IMM8).then_some(())?;
    let (reg, _, len) = decode_modrm(code, at + 1)?;
    let imm_at = at + 1 + len;
    (reg == MODRM_REG_CMP && *code.get(imm_at)? == expected).then_some(imm_at + size_of::<u8>())
}

/// A short or near conditional jump at `at`: its condition, target and end.
fn decode_jcc(code: &[u8], at: usize) -> Option<(u8, usize, usize)> {
    let opcode = *code.get(at)?;
    let (condition, displacement, end) = if opcode & !JCC_CONDITION_MASK == OPCODE_JCC_SHORT {
        (opcode & JCC_CONDITION_MASK, i64::from(*code.get(at + 1)? as i8), at + JCC_SHORT_SIZE)
    } else if opcode == OPCODE_TWO_BYTE && *code.get(at + 1)? & !JCC_CONDITION_MASK == OPCODE_JCC_NEAR {
        let rel = code.get(at + 2..at + JCC_NEAR_SIZE)?;
        let condition = code[at + 1] & JCC_CONDITION_MASK;
        (condition, i64::from(i32::from_le_bytes(rel.try_into().ok()?)), at + JCC_NEAR_SIZE)
    } else {
        return None;
    };
    let target = usize::try_from(end as i64 + displacement).ok().filter(|target| *target < code.len())?;
    Some((condition, target, end))
}

/// A jump at `at` with one of `conditions`: its target and end.
fn jump_if(code: &[u8], at: usize, conditions: [u8; 2]) -> Option<(usize, usize)> {
    let (condition, target, end) = decode_jcc(code, at)?;
    conditions.contains(&condition).then_some((target, end))
}

/// [`SPLIT_SIGNATURE`] at `at` of `code`: the three compares.
fn match_split(code: &[u8], at: usize) -> Option<[CompareImm32<'_>; SPLIT_IMMEDIATE_COUNT]> {
    let after_facet = decode_cmp_imm8(code, at, SPLIT_FACET_LIMIT)?;
    let (same_half, after) = jump_if(code, after_facet, [CONDITION_G, CONDITION_A])?;
    let first = decode_cmp_imm32(code, after)?;
    let (first_above, after) = jump_if(code, first.end, [CONDITION_GE, CONDITION_AE])?;
    let second = decode_cmp_imm32(code, after)?;
    let (target, different_halves) = jump_if(code, second.end, [CONDITION_L, CONDITION_B])?;
    (target == same_half).then_some(())?;
    let clears = code.get(different_halves..different_halves + CLEAR_RESULT[0].len())?;
    CLEAR_RESULT.iter().any(|clear| clear == clears).then_some(())?;
    let third = decode_cmp_imm32(code, first_above)?;
    (third.operand == second.operand).then_some(())?;
    let (target, _) = jump_if(code, third.end, [CONDITION_L, CONDITION_B])?;
    (target == different_halves).then_some(())?;
    let pops = code.get(same_half..)?.iter().take(MAX_POPS_BEFORE_RESULT);
    let result = same_half + pops.take_while(|opcode| POP_REGISTER_OPCODES.contains(opcode)).count();
    (code.get(result..result + SET_RESULT.len())? == SET_RESULT).then_some([first, second, third])
}

/// Every match of [`SPLIT_SIGNATURE`] in `.text`, whatever its registers, displacements, jump forms and values.
pub fn locate_split_candidates(bytes: &[u8], pe: &PeImage) -> eyre::Result<Vec<SplitCandidate>> {
    let text = pe.section(CODE_SECTION).ok_or_else(|| eyre!("no {CODE_SECTION} section"))?;
    let range = text.raw_range();
    let code = bytes.get(range.start..range.end.min(bytes.len())).unwrap_or_default();
    Ok((0..code.len())
        .filter(|&at| code[at] == OPCODE_CMP_RM32_IMM8)
        .filter_map(|at| {
            let compares = match_split(code, at)?;
            Some(SplitCandidate {
                signature_offset: range.start + at,
                offsets: compares.each_ref().map(|compare| range.start + compare.imm_offset),
                values: compares.each_ref().map(|compare| compare.imm),
            })
        })
        .collect())
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
    let before = ClientExeFacets::inspect(bytes)?;
    let facet_count = before.facet_count();
    eyre::ensure!(facet < facet_count, "facet {facet}: the table has facets 0..{facet_count}");
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
    if split_x.is_none() && before.split.is_none() && SPLIT_FACETS.contains(&patch.facet) {
        warnings.push(format!(
            "no x = {STOCK_SPLIT_X} split sequence found: right for Kingdom Reborn; in an Enhanced Client build, the \
             split code differs from the known sequence"
        ));
    }
    let mut patched = bytes.to_vec();
    let record = before.table.offset + FACET_RECORD_SIZE * facet;
    let mut allowed = vec![record + FACET_RECORD_WIDTH_OFFSET..record + FACET_RECORD_SIZE];
    patched[record + FACET_RECORD_WIDTH_OFFSET..][..size_of::<u32>()].copy_from_slice(&patch.width.to_le_bytes());
    patched[record + FACET_RECORD_HEIGHT_OFFSET..][..size_of::<u32>()].copy_from_slice(&patch.height.to_le_bytes());
    if let Some(x) = split_x {
        eyre::ensure!(
            SPLIT_FACETS.contains(&patch.facet),
            "the split concerns facets {SPLIT_FACETS:?}, not facet {facet}"
        );
        let split = before.split.ok_or_else(|| eyre!("no x = {STOCK_SPLIT_X} split sequence found in this build"))?;
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
    let same_split = after.split.map(|split| split.offsets) == before.split.map(|split| split.offsets);
    eyre::ensure!(
        after.table.offset == before.table.offset && same_split,
        "internal error: the patched copy's table or split is located elsewhere"
    );
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
    /// The shipped facet names.
    const STOCK_NAMES: [&str; STOCK_FACET_COUNT] =
        ["Felucca", "Trammel", "Ilshenar", "Malas", "Tokuno", "Gargoyle", "Test"];
    /// `UOFacet::IsSameFeluccaTrammelHalf` from the facet compare to its end (UOSA.exe file 0x1df6d2), with the
    /// split imm32 at [`SPLIT_IMMEDIATES`].
    const UOSA_SPLIT_CODE: [u8; 51] = [
        0x83, 0xff, 0x01, 0x7f, 0x24, 0x81, 0x7c, 0x24, 0x08, 0x00, 0x14, 0x00, 0x00, 0x7d, 0x12, 0x81, 0xfb, 0x00,
        0x14, 0x00, 0x00, 0x7c, 0x12, 0x32, 0xc0, 0x5e, 0x5b, 0x83, 0xc4, 0x0c, 0xc2, 0x04, 0x00, 0x81, 0xfb, 0x00,
        0x14, 0x00, 0x00, 0x7c, 0xee, 0x5e, 0xb0, 0x01, 0x5b, 0x83, 0xc4, 0x0c, 0xc2, 0x04, 0x00,
    ];
    const SPLIT_IMMEDIATES: [usize; SPLIT_IMMEDIATE_COUNT] = [9, 17, 35];
    /// Code before the split sequence.
    const SPLIT_CODE_START: usize = 8;
    /// Register and displacement variants of [`UOSA_SPLIT_CODE`]: (offset, byte). `CMP ESI,1`, `CMP [ESP+0x10]`,
    /// `CMP ECX` twice.
    const VARIANT_BYTES: [(usize, u8); 4] = [(1, 0xfe), (8, 0x10), (16, 0xf9), (34, 0xf9)];
    /// The second `CMP X` with another register than the first: not the split.
    const MISMATCHED_OPERAND: (usize, u8) = (34, 0xfa);

    fn split_code(x: u32, edits: &[(usize, u8)]) -> Vec<u8> {
        let mut code = vec![0x90; SPLIT_CODE_START];
        code.extend(UOSA_SPLIT_CODE);
        for offset in SPLIT_IMMEDIATES {
            code[SPLIT_CODE_START + offset..][..size_of::<u32>()].copy_from_slice(&x.to_le_bytes());
        }
        for (offset, byte) in edits {
            code[SPLIT_CODE_START + offset] = *byte;
        }
        code.extend([0xcc; SPLIT_CODE_START]);
        code
    }

    /// A client-like image: `.text` (`text`, or a nop sled), `.rdata` with the names, `.data` with `records`
    /// stock records.
    fn synthetic_image(text: Option<Vec<u8>>, records: usize, signed: bool) -> Vec<u8> {
        let text = text.unwrap_or_else(|| vec![0x90; 64]);
        // The sections are laid out at aligned offsets equal to their RVAs: .text, then .rdata, then .data.
        let rdata_rva = TEST_SECTION_ALIGNMENT * (1 + (text.len() as u32).div_ceil(TEST_SECTION_ALIGNMENT));
        let mut rdata = Vec::new();
        let mut pointers = Vec::new();
        for name in &STOCK_NAMES[..records] {
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
        data.extend(vec![0; FACET_RECORD_SIZE * STOCK_FACET_COUNT + TABLE_DATA_OFFSET]);
        let mut image = synthetic_pe(&[(".text", text), (".rdata", rdata), (".data", data)], signed);
        let pe = PeImage::parse(&image).unwrap();
        let checksum = pe_checksum(&image, pe.checksum_offset);
        image[pe.checksum_offset..pe.checksum_offset + 4].copy_from_slice(&checksum.to_le_bytes());
        image
    }

    /// A client-like image with the 7 stock records and, when `split` is set, the UOSA split sequence.
    pub fn synthetic_client(split: Option<u32>, signed: bool) -> Vec<u8> {
        synthetic_image(split.map(|x| split_code(x, &[])), STOCK_FACET_COUNT, signed)
    }

    #[test]
    fn table_and_split_are_located() {
        let image = synthetic_client(Some(STOCK_SPLIT_X), false);
        let facets = ClientExeFacets::inspect(&image).unwrap();
        assert!(facets.known.is_none());
        assert_eq!(facets.dimensions(), CLIENT_FACET_DIMENSIONS);
        assert_eq!(facets.table.records.iter().map(|record| record.name.as_str()).collect::<Vec<_>>(), STOCK_NAMES);
        assert_eq!(facets.split.unwrap().x, STOCK_SPLIT_X);
        assert!(facets.is_stock() && facets.checksum_valid());
        let data = facets.pe.section(".data").unwrap();
        assert_eq!(facets.table.offset, data.raw_offset as usize + TABLE_DATA_OFFSET);
        let text = facets.pe.section(CODE_SECTION).unwrap().raw_offset as usize + SPLIT_CODE_START;
        assert_eq!(facets.split.unwrap().signature_offset, text);
        assert_eq!(facets.split.unwrap().offsets, SPLIT_IMMEDIATES.map(|offset| text + offset));

        let no_split = ClientExeFacets::inspect(&synthetic_client(None, false)).unwrap();
        assert!(no_split.split.is_none() && no_split.is_stock());

        let mut unequal = image.clone();
        let offset = facets.split.unwrap().offsets[2];
        unequal[offset] ^= 1;
        assert!(ClientExeFacets::inspect(&unequal).is_err(), "differing split values");
        let mut two_tables = image.clone();
        let (table, table_size) = (facets.table.offset, FACET_RECORD_SIZE * STOCK_FACET_COUNT);
        two_tables.copy_within(table..table + table_size, table + table_size + TABLE_DATA_OFFSET);
        assert!(ClientExeFacets::inspect(&two_tables).is_err(), "two tables");
    }

    #[test]
    fn shorter_tables_are_located_and_too_short_runs_are_not() {
        for records in MIN_FACET_RECORDS..=STOCK_FACET_COUNT {
            let facets = ClientExeFacets::inspect(&synthetic_image(None, records, false)).unwrap();
            assert_eq!(facets.facet_count(), records);
            assert!(facets.is_stock());
        }
        assert!(ClientExeFacets::inspect(&synthetic_image(None, MIN_FACET_RECORDS - 1, false)).is_err());

        // A record with an illegal size ends a run: record 3 leaves two runs of 3 records, both too short.
        let image = synthetic_client(None, false);
        let facets = ClientExeFacets::inspect(&image).unwrap();
        let mut broken = image.clone();
        let width = facets.table.offset + FACET_RECORD_SIZE * 3 + FACET_RECORD_WIDTH_OFFSET;
        broken[width..width + 4].copy_from_slice(&(MAX_FACET_SIDE + FACET_SIDE_ALIGNMENT).to_le_bytes());
        assert!(locate_facet_tables(&broken, &facets.pe).is_empty());
        // A name that is not a word does not count either.
        let mut unnamed = image.clone();
        let name = facets.pe.va_to_offset(read_u32(&image, facets.table.offset).unwrap()).unwrap();
        unnamed[name] = b'#';
        let tables = locate_facet_tables(&unnamed, &facets.pe);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].records.len(), STOCK_FACET_COUNT - 1, "the run starts at Trammel");
    }

    #[test]
    fn the_split_matcher_takes_any_register_and_needs_the_same_x_operand() {
        let stock = ClientExeFacets::inspect(&synthetic_client(Some(STOCK_SPLIT_X), false)).unwrap();
        let variant = synthetic_image(Some(split_code(NEW_SPLIT, &VARIANT_BYTES)), STOCK_FACET_COUNT, false);
        let split = ClientExeFacets::inspect(&variant).unwrap().split.unwrap();
        assert_eq!(split.x, NEW_SPLIT);
        assert_eq!(split.offsets, stock.split.unwrap().offsets);

        let code = split_code(STOCK_SPLIT_X, &[MISMATCHED_OPERAND]);
        let mismatched = synthetic_image(Some(code), STOCK_FACET_COUNT, false);
        assert!(locate_split_candidates(&mismatched, &PeImage::parse(&mismatched).unwrap()).unwrap().is_empty());
        assert!(ClientExeFacets::inspect(&mismatched).unwrap().split.is_none());

        let mut twice = split_code(STOCK_SPLIT_X, &[]);
        twice.extend(split_code(STOCK_SPLIT_X, &VARIANT_BYTES));
        let image = synthetic_image(Some(twice), STOCK_FACET_COUNT, false);
        assert_eq!(locate_split_candidates(&image, &PeImage::parse(&image).unwrap()).unwrap().len(), 2);
        assert!(ClientExeFacets::inspect(&image).is_err(), "two split sequences");
    }

    #[test]
    fn a_patch_changes_only_the_record_the_split_and_the_checksum() {
        let image = synthetic_client(Some(STOCK_SPLIT_X), false);
        let patch = FacetDimsPatch { facet: 0, width: NEW_WIDTH, height: NEW_HEIGHT, split: SplitEdit::Set(NEW_SPLIT) };
        let patched = patch_facet_dimensions(&image, &patch, None).unwrap();
        assert_eq!(patched.after.dimensions()[0], FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT));
        assert_eq!(patched.after.dimensions()[1..], CLIENT_FACET_DIMENSIONS[1..]);
        assert_eq!(patched.after.split.unwrap().x, NEW_SPLIT);
        assert!(patched.after.checksum_valid() && !patched.after.is_stock());
        assert!(patched.warnings.is_empty());
        assert_eq!(patched.manifest.original_sha256, sha256_hex(&image));
        assert_eq!(patched.manifest.patched_sha256, sha256_hex(&patched.bytes));
        let record = patched.before.table.offset;
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

        // Without split code, resizing facet 0 warns that the seam was not found.
        let keep = FacetDimsPatch { split: SplitEdit::Keep, ..patch };
        assert_eq!(patch_facet_dimensions(&synthetic_client(None, false), &keep, None).unwrap().warnings.len(), 1);
    }

    #[test]
    fn bad_requests_and_builds_are_refused() {
        let image = synthetic_client(Some(STOCK_SPLIT_X), false);
        let good = FacetDimsPatch { facet: 3, width: NEW_WIDTH, height: NEW_HEIGHT, split: SplitEdit::Keep };
        for bad in [
            FacetDimsPatch { facet: STOCK_FACET_COUNT as u8, ..good },
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
        let short = synthetic_image(None, MIN_FACET_RECORDS, false);
        let past_the_table = FacetDimsPatch { facet: MIN_FACET_RECORDS as u8, ..good };
        assert!(patch_facet_dimensions(&short, &past_the_table, None).is_err(), "facet past a shorter table");
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
