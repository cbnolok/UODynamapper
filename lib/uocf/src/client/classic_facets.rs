//! The facet tables of the Classic client (`client.exe`, UO Gold to the current client) and the values derived from
//! them that the client also hard-codes. Traced in
//! `scripts/dev-helpers/ec-research/ghidra-classic-facets/results/classic_facets.md`; everything is located by
//! structure:
//!
//! - The table: a run of 48-byte `.rdata` records `{char* name, u32 width, u32 height, u32 wrap_width,
//!   u32 wrap_height, u32 file ids [statics, staidx, mapdif, stadif, stadifi, map], u32 felucca_rules}`. UO Gold to
//!   6.0.14.3 have one copy of 5 records; 7.0 and later two copies of 6 records (Ter Mur added). The second copy is
//!   read when the server sets 0xA9 flag 0x8000 and differs only in its file ids (the `x` variants). Both copies are
//!   required to agree on every other field.
//! - The file name table the ids index: entries of 56 bytes (up to 7.0.20) or 60 bytes (7.0.50 on), `{u32 record
//!   size, char name[40], .., u32 ordinal}` with the ordinal (index + 1) 8 bytes before the entry's end. Optional:
//!   without it the ids are reported raw.
//! - The startup copy of facet 0 ([`ClassicStartupFacet`]): `SetFacet` stores the wrap and size of the new facet in
//!   `.data` globals, `{wrap_width, wrap_height, width, height}`, and returns early when the facet does not change.
//!   The current facet starts at 0, so Felucca keeps the globals' initial values, which a static initializer also
//!   copies into the size globals and block counts. A facet 0 size or wrap edit writes them too. Located from the
//!   wrap width accessor (the function reading `table + 0x0c`): the call to it followed by `MOV [moffs32],EAX`
//!   into a writable section, with the stores of the other three fields after it.
//! - The wrap seam (the Classic split): two `CMP r32,imm32 (5120); [MOV r32,r/m32]; SETGE r8` near the reads of the
//!   startup globals ([`CLASSIC_SEAM_SIGNATURE`]): the radar (block x >= 5120, against the player's side
//!   `x >= wrap_width`) and the walk probe (no wrap at x >= 5120). They do not depend on the facet and must equal
//!   Felucca and Trammel's wrap width.
//!
//! No other facet size is hard-coded. The limits ([`validate_classic_size`]): block counts are i32 (signed shifts,
//! `IMUL`), a block's file offset is `196 * block` in 32 bits with an unsigned bound check, and the MUL files are
//! mapped whole into the 32-bit address space (2 GiB, 4 GiB with LARGEADDRESSAWARE: only 7.0.103 and later).

crate::eyre_imports!();

use crate::client::facet_dimensions::{
    code_section, decode_cmp_imm32, decode_modrm, facet_name_at, locate_record_runs, plausible_side, FacetSplit,
    FacetTable, FacetTableRecord, Operand, FACET_RECORD_HEIGHT_OFFSET, FACET_RECORD_WIDTH_OFFSET, FACET_SIDE_ALIGNMENT,
    MAX_FACET_SIDE, MOD_REGISTER, OPCODE_TWO_BYTE, STOCK_SPLIT_X,
};
use crate::client::pe::{read_u32, PeImage, PeSection};
use crate::classic::map::{MapBlock, CLASSIC_FACET_HEIGHTS};
use crate::enhanced::facet_sector::FacetGeometry;

/// Bytes of a Classic facet record.
pub const CLASSIC_FACET_RECORD_SIZE: usize = 48;
pub const CLASSIC_RECORD_WRAP_WIDTH_OFFSET: usize = 0x0c;
pub const CLASSIC_RECORD_WRAP_HEIGHT_OFFSET: usize = 0x10;
pub const CLASSIC_RECORD_FILE_IDS_OFFSET: usize = 0x14;
pub const CLASSIC_RECORD_FLAG_OFFSET: usize = 0x2c;
/// The file ids of a record, in record order ([`ClassicFileRole::ALL`]).
pub const CLASSIC_FILE_ID_COUNT: usize = 6;
/// `felucca_rules` is 1 for Felucca, 0 for the other facets.
const MAX_FELUCCA_RULES_FLAG: u32 = 1;
/// One table (UO Gold to 6.0.14.3) or two copies (7.0 and later).
pub const MAX_CLASSIC_TABLE_COPIES: usize = 2;
/// The bound of a file id when the file name table is not located: the shipped tables have 47 to 69 entries.
pub const UNLOCATED_FILE_ID_LIMIT: u32 = 256;

/// File name table entry strides (bytes): up to 7.0.20, and 7.0.50 on.
const FILE_NAME_STRIDES: [usize; 2] = [56, 60];
const FILE_NAME_OFFSET: usize = size_of::<u32>();
const FILE_NAME_FIELD_LEN: usize = 40;
/// The ordinal (entry index + 1) is this many bytes before the end of an entry.
const FILE_ORDINAL_FROM_END: usize = 8;
/// The fewest entries of a file name table: the six files of Felucca and a few more.
const MIN_FILE_NAME_ENTRIES: usize = 8;

/// Fields of the startup copy of facet 0: `{wrap_width, wrap_height, width, height}`.
const STARTUP_WRAP_WIDTH_OFFSET: usize = 0;
const STARTUP_WRAP_HEIGHT_OFFSET: usize = 4;
const STARTUP_WIDTH_OFFSET: usize = 8;
const STARTUP_HEIGHT_OFFSET: usize = 12;
const STARTUP_FIELDS: [usize; 4] =
    [STARTUP_WRAP_WIDTH_OFFSET, STARTUP_WRAP_HEIGHT_OFFSET, STARTUP_WIDTH_OFFSET, STARTUP_HEIGHT_OFFSET];
pub const CLASSIC_STARTUP_SIZE: usize = 16;
const OPCODE_CALL_REL32: u8 = 0xe8;
const CALL_REL32_SIZE: usize = 5;
/// `MOV [moffs32],EAX`.
const OPCODE_STORE_EAX_MOFFS32: u8 = 0xa3;
/// The wrap width accessor reads `table + 0x0c` within this many bytes of its start (two tables in 7.0.x).
const ACCESSOR_MAX_LEN: usize = 0x80;
/// `SetFacet` stores the accessor's result within this many bytes after the call.
const STORE_AFTER_CALL_WINDOW: usize = 16;
/// ... and the other three startup fields within this many bytes of the call.
const STARTUP_STORES_WINDOW: usize = 0x60;

/// The wrap seam sequence the matcher decodes.
pub const CLASSIC_SEAM_SIGNATURE: &str = "CMP r32,S; [MOV r32,m]; SETGE r8 (within 0x800 bytes of a read of the startup \
                                          globals; two sites, equal S)";
const SEAM_SITE_COUNT: usize = 2;
/// The seam sites lie at most 1163 bytes from a read of the startup globals in the traced builds; the nearest
/// unrelated `CMP; SETGE` is 8151 bytes away.
const SEAM_WINDOW: usize = 0x800;
const OPCODE_MOV_R32_RM32: u8 = 0x8b;
const OPCODE_MOV_RM32_R32: u8 = 0x89;
/// `0F 9D /r`: SETGE r/m8.
const OPCODE_SETGE: u8 = 0x9d;

/// The stock widths by facet (Felucca, Trammel, Ilshenar, Malas, Tokuno, Ter Mur), from Mondain's Legacy on; the
/// heights are [`CLASSIC_FACET_HEIGHTS`].
pub const CLASSIC_STOCK_WIDTHS: [u32; 6] = [7168, 7168, 2304, 2560, 1448, 1280];
/// Felucca and Trammel's width up to Age of Shadows (UO Gold).
pub const CLASSIC_OLD_FELUCCA_WIDTH: u32 = 6144;
/// Felucca and Trammel: the facets with a strip past the wrap.
const CLASSIC_STOCK_SPLIT_FACETS: usize = 2;
/// The stock wrap of Felucca and Trammel; the other facets wrap at their size.
pub const CLASSIC_STOCK_WRAP: FacetGeometry = FacetGeometry::new(STOCK_SPLIT_X, CLASSIC_FACET_HEIGHTS[0]);
/// The largest stock map file (Felucca, 7168 x 4096): a larger map needs that much more address space.
pub const CLASSIC_STOCK_MAX_MAP: FacetGeometry = FacetGeometry::new(CLASSIC_STOCK_WIDTHS[0], CLASSIC_FACET_HEIGHTS[0]);
/// Map, `staidx` and the client's block offsets are below 2 GiB: the 32-bit offsets and the address space of a
/// client without LARGEADDRESSAWARE.
pub const CLASSIC_MAX_MAP_FILE_BYTES: u64 = 1 << 31;

/// What each file id of a record names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassicFileRole {
    Statics,
    Staidx,
    /// The dif files (`mapdifN`, `stadifN`, `stadifiN`): read only up to 5.0.9.1.
    Mapdif,
    Stadif,
    Stadifi,
    Map,
}

impl ClassicFileRole {
    pub const ALL: [Self; CLASSIC_FILE_ID_COUNT] =
        [Self::Statics, Self::Staidx, Self::Mapdif, Self::Stadif, Self::Stadifi, Self::Map];

    pub fn label(self) -> &'static str {
        match self {
            Self::Statics => "statics",
            Self::Staidx => "staidx",
            Self::Mapdif => "mapdif",
            Self::Stadif => "stadif",
            Self::Stadifi => "stadifi",
            Self::Map => "map",
        }
    }
}

/// The fields of a Classic record past the name and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicRecord {
    /// The x and y past which the facet does not wrap.
    pub wrap: FacetGeometry,
    /// Indexes into the file name table, by [`ClassicFileRole::ALL`].
    pub file_ids: [u32; CLASSIC_FILE_ID_COUNT],
    pub felucca_rules: u32,
}

impl ClassicRecord {
    pub fn file_id(&self, role: ClassicFileRole) -> u32 {
        self.file_ids[ClassicFileRole::ALL.iter().position(|each| *each == role).expect("every role has an id")]
    }
}

/// The client's file name table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicFileNames {
    pub offset: usize,
    pub va: u32,
    pub stride: usize,
    /// The names by file id.
    pub names: Vec<String>,
}

impl ClassicFileNames {
    pub fn name(&self, id: u32) -> Option<&str> {
        self.names.get(id as usize).map(String::as_str)
    }
}

/// The `.data` globals that hold facet 0's wrap and size until the first facet change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicStartupFacet {
    pub offset: usize,
    pub va: u32,
    pub wrap: FacetGeometry,
    pub size: FacetGeometry,
}

impl ClassicStartupFacet {
    /// File offsets of its wrap width, wrap height, width and height.
    pub fn field_offsets(&self) -> [usize; STARTUP_FIELDS.len()] {
        STARTUP_FIELDS.map(|field| self.offset + field)
    }
}

/// The Classic-only parts of an inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicLayout {
    /// `None` when not located: facet 0 cannot be patched then.
    pub startup: Option<ClassicStartupFacet>,
    pub file_names: Option<ClassicFileNames>,
}

/// The 48-byte record at file offset `offset` of `section`, when it is one; its file ids must be below `id_limit`.
fn classic_record_at(
    bytes: &[u8],
    pe: &PeImage,
    section: &PeSection,
    offset: usize,
    id_limit: u32,
) -> Option<FacetTableRecord> {
    if offset + CLASSIC_FACET_RECORD_SIZE > section.raw_range().end.min(bytes.len()) {
        return None;
    }
    let field = |at: usize| read_u32(bytes, offset + at).ok();
    let geometry = FacetGeometry::new(field(FACET_RECORD_WIDTH_OFFSET)?, field(FACET_RECORD_HEIGHT_OFFSET)?);
    let wrap = FacetGeometry::new(field(CLASSIC_RECORD_WRAP_WIDTH_OFFSET)?, field(CLASSIC_RECORD_WRAP_HEIGHT_OFFSET)?);
    let sides = [geometry.width, geometry.height, wrap.width, wrap.height];
    if !(sides.into_iter().all(plausible_side) && wrap.width <= geometry.width && wrap.height <= geometry.height) {
        return None;
    }
    let mut file_ids = [0; CLASSIC_FILE_ID_COUNT];
    for (index, id) in file_ids.iter_mut().enumerate() {
        *id = field(CLASSIC_RECORD_FILE_IDS_OFFSET + index * size_of::<u32>()).filter(|id| *id < id_limit)?;
    }
    let felucca_rules = field(CLASSIC_RECORD_FLAG_OFFSET).filter(|flag| *flag <= MAX_FELUCCA_RULES_FLAG)?;
    let name = facet_name_at(bytes, pe, field(0)?)?;
    Some(FacetTableRecord { name, geometry, classic: Some(ClassicRecord { wrap, file_ids, felucca_rules }) })
}

/// Every candidate Classic table; file ids must be below `id_limit` (the length of the file name table, or
/// [`UNLOCATED_FILE_ID_LIMIT`]).
pub fn locate_classic_facet_tables(bytes: &[u8], pe: &PeImage, id_limit: u32) -> Vec<FacetTable> {
    locate_record_runs(bytes, pe, CLASSIC_FACET_RECORD_SIZE, |section, offset| {
        classic_record_at(bytes, pe, section, offset, id_limit)
    })
}

/// Fails unless the copies have the same names, sizes, wraps and flags (only the file ids may differ).
pub fn classic_copies_agree(tables: &[FacetTable]) -> eyre::Result<()> {
    let strip = |table: &FacetTable| -> Vec<_> {
        table
            .records
            .iter()
            .map(|record| {
                let classic = record.classic.expect("a Classic table");
                (record.name.clone(), record.geometry, classic.wrap, classic.felucca_rules)
            })
            .collect()
    };
    let first = strip(&tables[0]);
    for table in &tables[1..] {
        eyre::ensure!(
            strip(table) == first,
            "the Classic facet tables at {:#x} and {:#x} differ in more than their file ids",
            tables[0].offset,
            table.offset
        );
    }
    Ok(())
}

/// The file name at `offset`: a `name.ext` of letters, digits and underscores in a 40-byte field.
fn file_name_at(bytes: &[u8], offset: usize) -> Option<String> {
    let field = bytes.get(offset..offset + FILE_NAME_FIELD_LEN)?;
    let len = field.iter().position(|byte| *byte == 0)?;
    let name = &field[..len];
    let dot = name.iter().position(|byte| *byte == b'.')?;
    let word = |part: &[u8]| !part.is_empty() && part.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
    (word(&name[..dot]) && word(&name[dot + 1..])).then(|| String::from_utf8_lossy(name).into_owned())
}

/// Every candidate file name table: runs of at least [`MIN_FILE_NAME_ENTRIES`] entries whose ordinals count from 1,
/// plus a last entry with a name whose ordinal field is overwritten (UO Gold, 7.0.103).
pub fn locate_classic_file_names(bytes: &[u8], pe: &PeImage) -> Vec<ClassicFileNames> {
    let mut tables = Vec::new();
    for section in pe.sections.iter().filter(|section| section.is_data()) {
        let end = section.raw_range().end.min(bytes.len());
        for stride in FILE_NAME_STRIDES {
            let ordinal = |entry: usize| read_u32(bytes, entry + stride - FILE_ORDINAL_FROM_END).ok();
            for start in (section.raw_range().start..end.saturating_sub(stride)).step_by(size_of::<u32>()) {
                if ordinal(start) != Some(1) {
                    continue;
                }
                let entry = |index: usize| Some(start + stride * index).filter(|entry| entry + stride <= end);
                let mut names: Vec<String> = (0..)
                    .map_while(|index| {
                        let at = entry(index)?;
                        (ordinal(at)? == index as u32 + 1).then_some(())?;
                        file_name_at(bytes, at + FILE_NAME_OFFSET)
                    })
                    .collect();
                if names.len() < MIN_FILE_NAME_ENTRIES {
                    continue;
                }
                if let Some(last) = entry(names.len()).and_then(|at| file_name_at(bytes, at + FILE_NAME_OFFSET)) {
                    names.push(last);
                }
                let va = pe.offset_to_va(start).expect("the table lies in its section");
                tables.push(ClassicFileNames { offset: start, va, stride, names });
            }
        }
    }
    tables
}

/// Code offsets of the little-endian `value` in `code`.
fn find_u32(code: &[u8], value: u32) -> Vec<usize> {
    let needle = value.to_le_bytes();
    code.windows(needle.len()).enumerate().filter(|(_, window)| *window == needle).map(|(at, _)| at).collect()
}

/// The startup copy of facet 0 (see the module docs), located from `table`'s wrap width accessor. `None` when not
/// found; fails when several are.
pub fn locate_classic_startup_facet(
    bytes: &[u8],
    pe: &PeImage,
    table: &FacetTable,
) -> eyre::Result<Option<ClassicStartupFacet>> {
    let (code, _) = code_section(bytes, pe)?;
    let reads = find_u32(code, table.va + CLASSIC_RECORD_WRAP_WIDTH_OFFSET as u32);
    let is_accessor = |target: usize| reads.iter().any(|read| (target..target + ACCESSOR_MAX_LEN).contains(read));
    let mut found: Vec<ClassicStartupFacet> = Vec::new();
    for at in (0..code.len().saturating_sub(CALL_REL32_SIZE)).filter(|&at| code[at] == OPCODE_CALL_REL32) {
        let after = at + CALL_REL32_SIZE;
        let rel = i32::from_le_bytes(code[at + 1..after].try_into().expect("four bytes"));
        let target = usize::try_from(after as i64 + i64::from(rel)).ok().filter(|target| *target < code.len());
        if !target.is_some_and(is_accessor) {
            continue;
        }
        let window = &code[after..(after + STORE_AFTER_CALL_WINDOW).min(code.len())];
        for store in (0..window.len()).filter(|&store| window[store] == OPCODE_STORE_EAX_MOFFS32) {
            let Some(va) = window.get(store + 1..store + 1 + size_of::<u32>()).map(|moffs| {
                u32::from_le_bytes(moffs.try_into().expect("four bytes"))
            }) else {
                continue;
            };
            if let Some(startup) = startup_facet_at(bytes, pe, code, at, va)
                && !found.contains(&startup)
            {
                found.push(startup);
            }
        }
    }
    match found.len() {
        0 | 1 => Ok(found.pop()),
        count => eyre::bail!("{count} candidate startup copies of facet 0 found, expected 1"),
    }
}

/// The startup globals at `va`, stored by the `SetFacet` call at code offset `call`: a writable file-backed block
/// whose other three fields are stored near the call and whose values are a legal wrap and size.
fn startup_facet_at(bytes: &[u8], pe: &PeImage, code: &[u8], call: usize, va: u32) -> Option<ClassicStartupFacet> {
    let section = pe.section_of_va(va).filter(|section| section.is_writable() && section.is_data())?;
    let offset = pe.va_to_offset(va)?;
    (offset + CLASSIC_STARTUP_SIZE <= section.raw_range().end.min(bytes.len())).then_some(())?;
    let nearby = &code[call..(call + STARTUP_STORES_WINDOW).min(code.len())];
    let stored = |field: usize| {
        let needle = (va + field as u32).to_le_bytes();
        nearby.windows(needle.len()).any(|window| window == needle)
    };
    STARTUP_FIELDS[1..].iter().all(|field| stored(*field)).then_some(())?;
    let field = |at: usize| read_u32(bytes, offset + at).ok();
    let wrap = FacetGeometry::new(field(STARTUP_WRAP_WIDTH_OFFSET)?, field(STARTUP_WRAP_HEIGHT_OFFSET)?);
    let size = FacetGeometry::new(field(STARTUP_WIDTH_OFFSET)?, field(STARTUP_HEIGHT_OFFSET)?);
    let sides = [wrap.width, wrap.height, size.width, size.height];
    (sides.into_iter().all(plausible_side) && wrap.width <= size.width && wrap.height <= size.height)
        .then_some(ClassicStartupFacet { offset, va, wrap, size })
}

/// The `CMP r32,imm32; [MOV r32,m]; SETGE r8` at `at`: the offset and value of its imm32.
fn match_seam(code: &[u8], at: usize) -> Option<(usize, u32)> {
    let compare = decode_cmp_imm32(code, at)?;
    matches!(compare.operand, Operand::Register(_)).then_some(())?;
    let mut next = compare.end;
    if matches!(*code.get(next)?, OPCODE_MOV_R32_RM32 | OPCODE_MOV_RM32_R32)
        && let Some((_, Operand::Memory(..), len)) = decode_modrm(code, next + 1)
    {
        next += 1 + len;
    }
    let setge = code.get(next..next + 3)?;
    (setge[0] == OPCODE_TWO_BYTE && setge[1] == OPCODE_SETGE && setge[2] >> 6 == MOD_REGISTER)
        .then_some((compare.imm_offset, compare.imm))
}

/// The wrap seam: the two [`CLASSIC_SEAM_SIGNATURE`] sites near the reads of `startup`. `None` when none is found;
/// fails on another count or on different values.
pub fn locate_classic_seam(bytes: &[u8], pe: &PeImage, startup: &ClassicStartupFacet) -> eyre::Result<Option<FacetSplit>> {
    let (code, start) = code_section(bytes, pe)?;
    let reads: Vec<usize> = STARTUP_FIELDS.iter().flat_map(|field| find_u32(code, startup.va + *field as u32)).collect();
    let near = |at: usize| reads.iter().any(|read| read.abs_diff(at) < SEAM_WINDOW);
    let sites: Vec<(usize, usize, u32)> = (0..code.len())
        .filter_map(|at| match_seam(code, at).map(|(imm_offset, imm)| (at, imm_offset, imm)))
        .filter(|(at, ..)| near(*at))
        .collect();
    match sites[..] {
        [] => Ok(None),
        [(_, _, x), ..] if sites.len() == SEAM_SITE_COUNT && sites.iter().all(|(.., imm)| *imm == x) => {
            Ok(Some(FacetSplit {
                signature_offsets: sites.iter().map(|(at, ..)| start + at).collect(),
                offsets: sites.iter().map(|(_, imm_offset, _)| start + imm_offset).collect(),
                x,
            }))
        }
        _ => eyre::bail!(
            "{} wrap seam compares found ({:?}), expected {SEAM_SITE_COUNT} with the same value",
            sites.len(),
            sites.iter().map(|(.., imm)| *imm).collect::<Vec<_>>()
        ),
    }
}

/// True when Trammel reads Felucca's map file (UO Gold to 5.x: Trammel is map0 plus the mapdif1 patches); from
/// 6.0.14.3 on it reads map1. Told by the record file ids, so it holds for patched copies too.
pub fn classic_trammel_reads_map0(table: &FacetTable) -> bool {
    let map = |facet: usize| table.records.get(facet).and_then(|record| record.classic).map(|classic| classic.file_id(ClassicFileRole::Map));
    map(0).is_some() && map(0) == map(1)
}

/// The stock width of Felucca and Trammel a Classic build may have: [`CLASSIC_OLD_FELUCCA_WIDTH`] (UO Gold) only
/// where Trammel still reads map0, else [`CLASSIC_STOCK_WIDTHS`]'s.
pub fn classic_split_widths(table: &FacetTable) -> &'static [u32] {
    const BOTH: [u32; 2] = [CLASSIC_STOCK_WIDTHS[0], CLASSIC_OLD_FELUCCA_WIDTH];
    if classic_trammel_reads_map0(table) { &BOTH } else { &CLASSIC_STOCK_WIDTHS[..1] }
}

/// The stock size and wrap of Classic facet `facet` when Felucca and Trammel are `split_width` wide.
pub fn classic_stock_facet(facet: usize, split_width: u32) -> Option<(FacetGeometry, FacetGeometry)> {
    let height = *CLASSIC_FACET_HEIGHTS.get(facet)?;
    let width = if facet < CLASSIC_STOCK_SPLIT_FACETS { split_width } else { *CLASSIC_STOCK_WIDTHS.get(facet)? };
    let size = FacetGeometry::new(width, height);
    Some((size, if facet < CLASSIC_STOCK_SPLIT_FACETS { CLASSIC_STOCK_WRAP } else { size }))
}

/// True when every record of every copy has its stock size and wrap ([`classic_stock_facet`]: Felucca and Trammel
/// both 7168 wide, or both 6144 in a build whose Trammel reads map0 (UO Gold), [`CLASSIC_FACET_HEIGHTS`],
/// [`CLASSIC_STOCK_WRAP`] for Felucca and Trammel and the size for the others) and the startup copy equals facet 0.
pub fn is_classic_stock(tables: &[FacetTable], layout: &ClassicLayout) -> bool {
    let split_width = tables[0].records[0].geometry.width;
    let records_stock = classic_split_widths(&tables[0]).contains(&split_width)
        && tables.iter().all(|table| {
            table.records.len() <= CLASSIC_STOCK_WIDTHS.len()
                && table.records.iter().enumerate().all(|(facet, record)| {
                    let wrap = record.classic.expect("a Classic table").wrap;
                    classic_stock_facet(facet, split_width) == Some((record.geometry, wrap))
                })
        });
    let first = &tables[0].records[0];
    let startup_stock = layout.startup.is_none_or(|startup| {
        startup.size == first.geometry && Some(startup.wrap) == first.classic.map(|classic| classic.wrap)
    });
    records_stock && startup_stock
}

/// Fails on a Classic size the client cannot hold; returns the warnings of an allowed but unusual one.
pub fn validate_classic_size(width: u32, height: u32) -> eyre::Result<Vec<String>> {
    for (what, side) in [("width", width), ("height", height)] {
        eyre::ensure!(
            (FACET_SIDE_ALIGNMENT..=MAX_FACET_SIDE).contains(&side),
            "{what} {side} is outside {FACET_SIDE_ALIGNMENT}..={MAX_FACET_SIDE}"
        );
        eyre::ensure!(side % FACET_SIDE_ALIGNMENT == 0, "{what} {side} is not a multiple of {FACET_SIDE_ALIGNMENT}");
    }
    let map_bytes = classic_map_bytes(FacetGeometry::new(width, height));
    eyre::ensure!(
        map_bytes < CLASSIC_MAX_MAP_FILE_BYTES,
        "{width}x{height} needs a {map_bytes}-byte map file; the client's 32-bit block offsets and mapping need it \
         below {CLASSIC_MAX_MAP_FILE_BYTES}"
    );
    let stock = classic_map_bytes(CLASSIC_STOCK_MAX_MAP);
    Ok(if map_bytes > stock {
        vec![format!(
            "{width}x{height} needs a {map_bytes}-byte map file, more than stock Felucca's {stock}: the client maps \
             every MUL file whole into its address space"
        )]
    } else {
        Vec::new()
    })
}

/// Fails on a wrap the facet of size `size` cannot have: multiples of 8, within the size.
pub fn validate_classic_wrap(wrap: FacetGeometry, size: FacetGeometry) -> eyre::Result<()> {
    for (what, side, limit) in [("wrap width", wrap.width, size.width), ("wrap height", wrap.height, size.height)] {
        eyre::ensure!(
            (FACET_SIDE_ALIGNMENT..=limit).contains(&side),
            "{what} {side} is outside {FACET_SIDE_ALIGNMENT}..={limit} (the facet's size)"
        );
        eyre::ensure!(side % FACET_SIDE_ALIGNMENT == 0, "{what} {side} is not a multiple of {FACET_SIDE_ALIGNMENT}");
    }
    Ok(())
}

/// The blocks of a map of size `size`.
pub fn classic_map_blocks(size: FacetGeometry) -> u64 {
    u64::from(size.width / MapBlock::CELLS_PER_ROW) * u64::from(size.height / MapBlock::CELLS_PER_COLUMN)
}

/// The bytes of a map file of size `size`.
pub fn classic_map_bytes(size: FacetGeometry) -> u64 {
    classic_map_blocks(size) * MapBlock::PACKED_SIZE as u64
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::client::facet_dimensions::tests::fix_checksum;
    use crate::client::facet_dimensions::{ClientExeInspection, ClientFamily};
    use crate::client::pe::tests::{synthetic_pe, TEST_IMAGE_BASE, TEST_SECTION_ALIGNMENT};

    pub const CLASSIC_NAMES: [&str; 6] = ["Felucca", "Trammel", "Ilshenar", "Malas", "Tokuno", "TerMur"];
    /// The synthetic file name table: `<role><facet>.mul`, then the `x` variants of facets 0 and 1.
    const FILE_NAME_STRIDE: usize = 60;
    const VARIANT_FACETS: usize = 2;
    /// Offsets inside the synthetic `.data`: the startup globals, then the tables.
    const STARTUP_DATA_OFFSET: usize = 16;
    const TABLES_DATA_OFFSET: usize = 64;
    /// The gap between two copies (a zero record).
    const COPY_GAP: usize = CLASSIC_FACET_RECORD_SIZE;
    /// Code layout: the wrap width accessor at 0, `SetFacet` at [`SET_FACET_AT`], the seam sites after it.
    const SET_FACET_AT: usize = 0x100;
    const SEAM_AT: [usize; SEAM_SITE_COUNT] = [0x180, 0x200];
    const CODE_SIZE: usize = 0x300;

    fn file_id(role: usize, facet: usize, variant: bool) -> u32 {
        let base = role * CLASSIC_NAMES.len() + facet;
        if variant && facet < VARIANT_FACETS {
            (ClassicFileRole::ALL.len() * CLASSIC_NAMES.len() + role * VARIANT_FACETS + facet) as u32
        } else {
            base as u32
        }
    }

    fn file_names() -> Vec<String> {
        let mut names = Vec::new();
        for role in ClassicFileRole::ALL {
            names.extend((0..CLASSIC_NAMES.len()).map(|facet| format!("{}{facet}.mul", role.label())));
        }
        for role in ClassicFileRole::ALL {
            names.extend((0..VARIANT_FACETS).map(|facet| format!("{}{facet}x.mul", role.label())));
        }
        names
    }

    fn put(buffer: &mut [u8], at: usize, value: u32) {
        buffer[at..at + size_of::<u32>()].copy_from_slice(&value.to_le_bytes());
    }

    pub fn stock_size(facet: usize) -> FacetGeometry {
        FacetGeometry::new(CLASSIC_STOCK_WIDTHS[facet], CLASSIC_FACET_HEIGHTS[facet])
    }

    pub fn stock_wrap(facet: usize) -> FacetGeometry {
        if facet < CLASSIC_STOCK_SPLIT_FACETS { CLASSIC_STOCK_WRAP } else { stock_size(facet) }
    }

    /// The synthetic Classic client: `copies` tables of `records` stock records, file name table, startup globals,
    /// wrap width accessor, `SetFacet` stores and two seam compares.
    pub fn synthetic_classic(records: usize, copies: usize) -> Vec<u8> {
        let rdata_rva = TEST_SECTION_ALIGNMENT * (1 + (CODE_SIZE as u32).div_ceil(TEST_SECTION_ALIGNMENT));
        let mut rdata = Vec::new();
        let mut pointers = Vec::new();
        for name in &CLASSIC_NAMES[..records] {
            pointers.push(TEST_IMAGE_BASE + rdata_rva + rdata.len() as u32);
            rdata.extend_from_slice(name.as_bytes());
            rdata.push(0);
        }
        rdata.resize(rdata.len().next_multiple_of(size_of::<u32>()), 0);
        for (index, name) in file_names().iter().enumerate() {
            let mut entry = vec![0u8; FILE_NAME_STRIDE];
            put(&mut entry, 0, MapBlock::PACKED_SIZE as u32);
            entry[FILE_NAME_OFFSET..FILE_NAME_OFFSET + name.len()].copy_from_slice(name.as_bytes());
            put(&mut entry, FILE_NAME_STRIDE - FILE_ORDINAL_FROM_END, index as u32 + 1);
            rdata.extend(entry);
        }
        rdata.extend([0; FILE_NAME_STRIDE]);
        // The data section follows .rdata.
        let data_rva = rdata_rva + TEST_SECTION_ALIGNMENT * (rdata.len() as u32).div_ceil(TEST_SECTION_ALIGNMENT);
        let data_va = TEST_IMAGE_BASE + data_rva;
        let mut data = vec![0u8; TABLES_DATA_OFFSET];
        for (field, value) in STARTUP_FIELDS.iter().zip([
            stock_wrap(0).width,
            stock_wrap(0).height,
            stock_size(0).width,
            stock_size(0).height,
        ]) {
            put(&mut data, STARTUP_DATA_OFFSET + field, value);
        }
        for copy in 0..copies {
            for (facet, pointer) in pointers.iter().enumerate() {
                let mut record = vec![0u8; CLASSIC_FACET_RECORD_SIZE];
                let (size, wrap) = (stock_size(facet), stock_wrap(facet));
                put(&mut record, 0, *pointer);
                put(&mut record, FACET_RECORD_WIDTH_OFFSET, size.width);
                put(&mut record, FACET_RECORD_HEIGHT_OFFSET, size.height);
                put(&mut record, CLASSIC_RECORD_WRAP_WIDTH_OFFSET, wrap.width);
                put(&mut record, CLASSIC_RECORD_WRAP_HEIGHT_OFFSET, wrap.height);
                for role in 0..CLASSIC_FILE_ID_COUNT {
                    let id = file_id(role, facet, copy > 0);
                    put(&mut record, CLASSIC_RECORD_FILE_IDS_OFFSET + role * size_of::<u32>(), id);
                }
                put(&mut record, CLASSIC_RECORD_FLAG_OFFSET, u32::from(facet == 0));
                data.extend(record);
            }
            data.extend([0; COPY_GAP]);
        }
        let table_va = data_va + TABLES_DATA_OFFSET as u32;
        let startup_va = data_va + STARTUP_DATA_OFFSET as u32;

        let mut code = vec![0x90u8; CODE_SIZE];
        // Wrap width accessor: MOV EAX,[ECX+table+0x0c]; RET.
        code[0..2].copy_from_slice(&[0x8b, 0x81]);
        put(&mut code, 2, table_va + CLASSIC_RECORD_WRAP_WIDTH_OFFSET as u32);
        code[6] = 0xc3;
        // SetFacet: CALL accessor; PUSH ESI; MOV [startup],EAX; then MOV [startup+field],EAX for the others.
        let mut at = SET_FACET_AT;
        code[at] = OPCODE_CALL_REL32;
        put(&mut code, at + 1, (-((at + CALL_REL32_SIZE) as i32)) as u32);
        at += CALL_REL32_SIZE;
        code[at] = 0x56;
        at += 1;
        for field in STARTUP_FIELDS {
            code[at] = OPCODE_STORE_EAX_MOFFS32;
            put(&mut code, at + 1, startup_va + field as u32);
            at += 1 + size_of::<u32>();
        }
        // Seam sites: CMP EDX,5120; SETGE AL, and CMP EAX,5120; MOV ECX,[ESP+4]; SETGE CL.
        let first = SEAM_AT[0];
        code[first..first + 2].copy_from_slice(&[0x81, 0xfa]);
        put(&mut code, first + 2, STOCK_SPLIT_X);
        code[first + 6..first + 9].copy_from_slice(&[0x0f, 0x9d, 0xc0]);
        let second = SEAM_AT[1];
        code[second] = 0x3d;
        put(&mut code, second + 1, STOCK_SPLIT_X);
        code[second + 5..second + 13].copy_from_slice(&[0x8b, 0x4c, 0x24, 0x04, 0x0f, 0x9d, 0xc1, 0x90]);

        let mut image = synthetic_pe(&[(".text", code), (".rdata", rdata), (".data", data)], false);
        fix_checksum(&mut image);
        image
    }

    /// File offset of the imm32 of the seam sites in a synthetic image.
    pub fn seam_imm_offsets(inspection: &ClientExeInspection) -> Vec<usize> {
        let text = inspection.pe.section(".text").unwrap().raw_offset as usize;
        vec![text + SEAM_AT[0] + 2, text + SEAM_AT[1] + 1]
    }

    #[test]
    fn gold_widths_are_stock_only_where_trammel_reads_map0() {
        let image = synthetic_classic(5, 1);
        let inspection = ClientExeInspection::inspect(&image).unwrap();
        let mut gold = image.clone();
        let table = inspection.table();
        for facet in 0..CLASSIC_STOCK_SPLIT_FACETS {
            put(&mut gold, table.record_offset(facet) + FACET_RECORD_WIDTH_OFFSET, CLASSIC_OLD_FELUCCA_WIDTH);
        }
        let startup = inspection.classic.as_ref().unwrap().startup.unwrap();
        put(&mut gold, startup.field_offsets()[2], CLASSIC_OLD_FELUCCA_WIDTH);
        assert!(!ClientExeInspection::inspect(&gold).unwrap().is_stock(), "6144 with Trammel on map1 is not stock");
        let map_role = ClassicFileRole::ALL.iter().position(|role| *role == ClassicFileRole::Map).unwrap();
        let felucca_map = table.records[0].classic.unwrap().file_ids[map_role];
        put(&mut gold, table.record_offset(1) + CLASSIC_RECORD_FILE_IDS_OFFSET + map_role * size_of::<u32>(), felucca_map);
        let gold = ClientExeInspection::inspect(&gold).unwrap();
        assert!(classic_trammel_reads_map0(gold.table()));
        assert!(gold.is_stock(), "UO Gold: Felucca and Trammel 6144 wide on map0");
        let mut uneven = image.clone();
        put(&mut uneven, table.record_offset(1) + FACET_RECORD_WIDTH_OFFSET, CLASSIC_OLD_FELUCCA_WIDTH);
        assert!(!ClientExeInspection::inspect(&uneven).unwrap().is_stock());
    }

    #[test]
    fn one_and_two_copy_tables_are_located_with_their_layout() {
        for (records, copies) in [(5, 1), (6, 2)] {
            let image = synthetic_classic(records, copies);
            let inspection = ClientExeInspection::inspect(&image).unwrap();
            assert_eq!(inspection.family, ClientFamily::Classic);
            assert_eq!(inspection.tables.len(), copies);
            assert_eq!(inspection.facet_count(), records);
            assert!(inspection.is_stock() && inspection.checksum_valid());
            let classic = inspection.classic.as_ref().unwrap();
            let names = classic.file_names.as_ref().unwrap();
            assert_eq!(names.names, file_names());
            let startup = classic.startup.unwrap();
            assert_eq!((startup.size, startup.wrap), (stock_size(0), stock_wrap(0)));
            let seam = inspection.split.clone().unwrap();
            assert_eq!(seam.x, STOCK_SPLIT_X);
            assert_eq!(seam.offsets, seam_imm_offsets(&inspection));
            let felucca = inspection.tables.last().unwrap().records[0].classic.unwrap();
            let map = names.name(felucca.file_id(ClassicFileRole::Map)).unwrap();
            assert_eq!(map, if copies > 1 { "map0x.mul" } else { "map0.mul" });
            assert_eq!(felucca.felucca_rules, 1);
            assert_eq!(inspection.values().facets[2].wrap_width, Some(stock_size(2).width));
        }
    }

    #[test]
    fn mismatched_copies_and_impossible_wraps_are_refused() {
        let image = synthetic_classic(6, 2);
        let inspection = ClientExeInspection::inspect(&image).unwrap();
        let second = &inspection.tables[1];
        let mut mismatched = image.clone();
        put(&mut mismatched, second.record_offset(3) + FACET_RECORD_WIDTH_OFFSET, stock_size(3).width + 8);
        assert!(ClientExeInspection::inspect(&mismatched).is_err(), "copies with different sizes");

        // A wrap larger than the size is not a record: the run breaks into two runs too short to count.
        let mut wide_wrap = image.clone();
        for table in &inspection.tables {
            put(&mut wide_wrap, table.record_offset(2) + CLASSIC_RECORD_WRAP_WIDTH_OFFSET, stock_size(2).width + 8);
        }
        assert!(locate_classic_facet_tables(&wide_wrap, &inspection.pe, UNLOCATED_FILE_ID_LIMIT).is_empty());

        assert!(validate_classic_wrap(FacetGeometry::new(5128, 4096), stock_size(1)).is_ok());
        assert!(validate_classic_wrap(FacetGeometry::new(7176, 4096), stock_size(1)).is_err());
        assert!(validate_classic_wrap(FacetGeometry::new(5121, 4096), stock_size(1)).is_err());
        assert!(validate_classic_size(MAX_FACET_SIDE, MAX_FACET_SIDE).is_err(), "a map file of 3 GiB");
        assert_eq!(validate_classic_size(8192, 4096).unwrap().len(), 1, "larger than stock Felucca");
        assert!(validate_classic_size(4096, 4096).unwrap().is_empty());
    }
}
