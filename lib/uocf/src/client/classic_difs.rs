//! Whether a Classic client (`client.exe`) applies the mapdif / stadif patches, told from its code. Traced in
//! `scripts/dev-helpers/ec-research/ghidra-classic-facets/results/classic_facets.md` ("Dif usage checker"); every
//! step is located by structure, never by address or hash:
//!
//! 1. **The loader.** The `.text` `PUSH imm32` of the `mapdifl%d.mul` (land) or `stadifl%d.mul` (statics) format
//!    string. The loader runs from the 0xBF/0x18 (and 0x11) handler and reads, per map, the block numbers of the list
//!    file into a std::map (block -> dif record).
//! 2. **The map array.** Within [`LOADER_SETUP_WINDOW`] bytes before the push, a `MOV r32,imm32` of a writable VA
//!    whose register the loop advances by the map size, `ADD r32,imm8` within [`LOADER_LOOP_WINDOW`] bytes after it
//!    (12 bytes in every build; the counts pointer moves by 4 and is not taken). The number of maps comes from the
//!    static constructor / destructor, `PUSH count; PUSH size; PUSH array` ([`MapReference::Construction`]).
//! 3. **The lookups.** A lookup reads the map of the current facet: a memory operand `[index*scale + disp32]` (SIB,
//!    no base) whose disp32 lies in the first map. Its function starts at the nearest 16-byte aligned address after a
//!    `RET` / `JMP` and the compiler's padding (`INT3` or `NOP` runs) ([`function_start`]).
//! 4. **The callers.** `CALL rel32` to the lookup, `JMP rel32` to it (a tail call, or an incremental-link thunk whose
//!    own callers count) and every stored pointer to it (its VA as a u32 anywhere in the sections). A lookup without
//!    any is dead code.
//!
//! The status ([`DifStatus`]) is told per side: land and statics are loaded and looked up separately. Any other
//! reference to the map array (one the steps above do not explain) makes a side without a called lookup
//! [`DifStatus::Unknown`]: the patches might be read another way. The status says what the code can do; the client
//! still applies nothing until the server sends the patch counts (0xBF/0x18).

use std::collections::HashMap;
use std::ops::{Range, RangeInclusive};

use crate::client::facet_dimensions::code_section;
use crate::client::pe::PeImage;

/// The list file name formats each loader passes to sprintf.
pub const LAND_LIST_FORMAT: &str = "mapdifl%d.mul";
pub const STATICS_LIST_FORMAT: &str = "stadifl%d.mul";

const OPCODE_PUSH_IMM32: u8 = 0x68;
const OPCODE_PUSH_IMM8: u8 = 0x6a;
const PUSH_IMM8_SIZE: usize = 2;
/// The `OR CL,AL` (2 bytes) between the count and the size pushes of an MSVC 6 guarded vector registration.
const CONSTRUCTION_INTERLEAVED_SIZE: usize = 2;
/// `MOV r32,imm32`: the register is the low 3 bits.
const OPCODES_MOV_R32_IMM32: RangeInclusive<u8> = 0xb8..=0xbf;
const REGISTER_MASK: u8 = 0b111;
const REGISTER_ESP: u8 = 4;
/// `83 /0 ib` with mod 11: `ADD r32,imm8`, ModRM `0xc0 + register`.
const OPCODE_GROUP1_RM32_IMM8: u8 = 0x83;
const MODRM_ADD_REGISTER: u8 = 0xc0;
const OPCODE_CALL_REL32: u8 = 0xe8;
const OPCODE_JMP_REL32: u8 = 0xe9;
const OPCODE_JMP_REL8: u8 = 0xeb;
const OPCODE_RET: u8 = 0xc3;
const OPCODE_RET_IMM16: u8 = 0xc2;
const REL32_INSTRUCTION_SIZE: usize = 5;
const RET_IMM16_SIZE: usize = 3;
const JMP_REL8_SIZE: usize = 2;
/// The padding between functions: `INT3` (MSVC 8 and later) or `NOP` (MSVC 6).
const FUNCTION_PADDING: [u8; 2] = [0xcc, 0x90];
const FUNCTION_ALIGNMENT: u32 = 16;
/// How far before a lookup's map read its function start is searched (the reads are at most 0x56 bytes in).
pub const MAX_FUNCTION_SCAN: usize = 0x800;
/// The map array is set up at most 25 bytes before the format push in the tested builds.
const LOADER_SETUP_WINDOW: usize = 0x40;
/// ... and advanced at most 0x117 bytes after it.
const LOADER_LOOP_WINDOW: usize = 0x200;
/// The size of a std::map object (12 in every tested build): more than the counts' 4 bytes, at most 32.
const MAP_SIZES: RangeInclusive<u8> = 8..=32;
/// The most maps a constructor count is taken for (5 up to 6.0.14.3, 6 from 7.0).
const MAX_DIF_MAPS: u8 = 16;
/// ModRM with mod 00 and rm 100: a SIB byte follows; SIB base 101 with mod 00: a disp32 and no base; SIB index 100:
/// no index.
const MODRM_MOD_RM_MASK: u8 = 0b1100_0111;
const MODRM_SIB_NO_DISPLACEMENT: u8 = 0b0000_0100;
const SIB_BASE_NONE: u8 = 0b101;
const SIB_INDEX_SHIFT: u8 = 3;
const SIB_INDEX_NONE: u8 = 0b100;
/// Callers listed in an evidence line before "...".
const MAX_LISTED_SITES: usize = 8;

/// The two kinds of patches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DifKind {
    /// `mapdifl` / `mapdif`: land blocks.
    Land,
    /// `stadifl` / `stadifi` / `stadif`: statics blocks.
    Statics,
}

impl DifKind {
    pub const ALL: [Self; 2] = [Self::Land, Self::Statics];

    pub fn label(self) -> &'static str {
        match self {
            Self::Land => "land (mapdif)",
            Self::Statics => "statics (stadif)",
        }
    }

    pub fn list_format(self) -> &'static str {
        match self {
            Self::Land => LAND_LIST_FORMAT,
            Self::Statics => STATICS_LIST_FORMAT,
        }
    }
}

/// What the client does with one kind of patches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DifStatus {
    /// The lists are loaded and a lookup the block readers call reads them: the patches show once the server sends
    /// their counts (0xBF/0x18).
    Applied,
    /// The lists are loaded, but no lookup reads them or no code calls the lookup: the patches never show.
    LoadedNotApplied,
    /// No loader: the client never reads the list files.
    Absent,
    /// The code does not have the shape the checker knows; the reason.
    Unknown(String),
}

impl DifStatus {
    /// `Some(true)` when the client shows the patches, `Some(false)` when it never does, `None` when unknown.
    pub fn applies(&self) -> Option<bool> {
        match self {
            Self::Applied => Some(true),
            Self::LoadedNotApplied | Self::Absent => Some(false),
            Self::Unknown(_) => None,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Applied => "applied (once the server sends the 0xBF/0x18 patch counts)".to_owned(),
            Self::LoadedNotApplied => "loaded, never applied".to_owned(),
            Self::Absent => "absent: no dif loader".to_owned(),
            Self::Unknown(reason) => format!("unknown: {reason}"),
        }
    }
}

/// The std::map array a loader fills, one map per facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DifMaps {
    pub va: u32,
    /// Bytes of a map object.
    pub map_size: u32,
    /// The maps the static constructor builds; `None` when no constructor was found.
    pub count: Option<u32>,
}

impl DifMaps {
    /// The VAs of the whole array (the first map when the count is unknown).
    fn range(&self) -> Range<u32> {
        self.va..self.va + self.map_size * self.count.unwrap_or(1)
    }
}

/// A loader: the code that pushes the list format and fills the maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DifLoader {
    pub kind: DifKind,
    /// The VA of the `PUSH` of the format string.
    pub push_va: u32,
    /// The start of its function, when found.
    pub function_va: Option<u32>,
    pub maps: DifMaps,
}

/// How a caller reaches a lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifCallKind {
    /// `CALL rel32`.
    Call,
    /// `JMP rel32` from inside another function (a tail call).
    TailJump,
    /// `CALL rel32` (or a stored pointer) to a thunk `JMP rel32` at `thunk_va`.
    Thunk { thunk_va: u32 },
    /// The lookup's VA stored as a u32 (a function pointer, a vtable entry, `PUSH` / `MOV` of the address).
    Pointer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DifCaller {
    /// The VA of the call, jump or stored pointer.
    pub site_va: u32,
    pub kind: DifCallKind,
}

/// A function that reads the maps by facet index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DifLookup {
    pub kind: DifKind,
    pub va: u32,
    /// The VAs of its map reads (the disp32 of each).
    pub read_vas: Vec<u32>,
    pub callers: Vec<DifCaller>,
}

/// What a Classic client does with its dif files ([`inspect_classic_difs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicDifUsage {
    pub land: DifStatus,
    pub statics: DifStatus,
    pub loaders: Vec<DifLoader>,
    pub lookups: Vec<DifLookup>,
    /// What was found, one human readable line each, prefixed with the kind.
    pub evidence: Vec<String>,
}

impl ClassicDifUsage {
    pub fn status(&self, kind: DifKind) -> &DifStatus {
        match kind {
            DifKind::Land => &self.land,
            DifKind::Statics => &self.statics,
        }
    }

    /// What a map converted for this client should do with the dif files: the client shows the patched map only
    /// when it applies them.
    pub fn advice(&self) -> &'static str {
        let applies: Vec<Option<bool>> = DifKind::ALL.into_iter().map(|kind| self.status(kind).applies()).collect();
        if applies.iter().all(|applies| *applies == Some(true)) {
            "convert the map with the dif files applied: this client shows the patched map"
        } else if applies.iter().all(|applies| *applies == Some(false)) {
            "convert the map without the dif files: this client never applies them"
        } else if applies.contains(&None) {
            "check the evidence: whether this client applies the dif files is not certain"
        } else {
            "apply only the dif files of the kind this client applies"
        }
    }
}

/// A reference into a map array, by the bytes before its u32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MapReference {
    /// The loader's `MOV r32,imm32`.
    Loader,
    /// `PUSH count; PUSH size; PUSH array`: the static constructor or destructor (`eh vector constructor iterator`);
    /// the guarded registrations of MSVC 6 put an `OR CL,AL` after the count.
    Construction { count: u8 },
    /// `[index*scale + disp32]`, no base: a read of the map of a facet.
    Indexed,
    Other,
}

fn read_code_u32(code: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(code.get(at..at + size_of::<u32>())?.try_into().ok()?))
}

/// The class of the u32 at code offset `at` that points into an array of `map_size`-byte maps.
fn classify_reference(code: &[u8], at: usize, map_size: u32, loader_sites: &[usize]) -> MapReference {
    let before = |back: usize| at.checked_sub(back).map(|index| code[index]);
    if let (Some(modrm), Some(sib)) = (before(2), before(1))
        && modrm & MODRM_MOD_RM_MASK == MODRM_SIB_NO_DISPLACEMENT
        && sib & REGISTER_MASK == SIB_BASE_NONE
        && (sib >> SIB_INDEX_SHIFT) & REGISTER_MASK != SIB_INDEX_NONE
    {
        return MapReference::Indexed;
    }
    if before(1) == Some(OPCODE_PUSH_IMM32)
        && before(3) == Some(OPCODE_PUSH_IMM8)
        && before(2).map(u32::from) == Some(map_size)
        && let Some(count) = [0, CONSTRUCTION_INTERLEAVED_SIZE].into_iter().find_map(|between| {
            let push = PUSH_IMM8_SIZE * 2 + 1 + between;
            (before(push) == Some(OPCODE_PUSH_IMM8)).then(|| before(push - 1)).flatten()
        })
        && (1..=MAX_DIF_MAPS).contains(&count)
    {
        return MapReference::Construction { count };
    }
    if at.checked_sub(1).is_some_and(|mov| loader_sites.contains(&mov)) {
        return MapReference::Loader;
    }
    MapReference::Other
}

/// True when `va` lies in a writable section, file-backed or not (the maps are zero-filled data).
fn is_writable_va(pe: &PeImage, va: u32) -> bool {
    pe.sections.iter().any(|section| {
        let size = section.virtual_size.max(section.raw_size);
        section.is_writable() && va.checked_sub(section.virtual_address).is_some_and(|delta| delta < size)
    })
}

/// The map array the loader pushing the format at code offset `push` fills: the `MOV r32,imm32` site and the array.
fn loader_maps(code: &[u8], pe: &PeImage, push: usize) -> Option<(usize, DifMaps)> {
    let setup = push.saturating_sub(LOADER_SETUP_WINDOW)..push;
    let loop_end = (push + LOADER_LOOP_WINDOW).min(code.len());
    setup.rev().find_map(|mov| {
        let opcode = code[mov];
        let register = opcode & REGISTER_MASK;
        if !OPCODES_MOV_R32_IMM32.contains(&opcode) || register == REGISTER_ESP {
            return None;
        }
        let va = read_code_u32(code, mov + 1).filter(|va| is_writable_va(pe, *va))?;
        let add = code.get(push..loop_end)?.windows(3).find(|window| {
            window[0] == OPCODE_GROUP1_RM32_IMM8 && window[1] == MODRM_ADD_REGISTER + register && MAP_SIZES.contains(&window[2])
        })?;
        Some((mov, DifMaps { va, map_size: u32::from(add[2]), count: None }))
    })
}

/// True when a function may start at code offset `at`: aligned, after a `RET`, `RET imm16`, `JMP` and its padding
/// (or right after a `RET` when there is none).
fn is_function_start(code: &[u8], text_va: u32, at: usize) -> bool {
    if at == 0 {
        return true;
    }
    if !(text_va + at as u32).is_multiple_of(FUNCTION_ALIGNMENT) {
        return false;
    }
    let mut end = at;
    while end > 0 && FUNCTION_PADDING.contains(&code[end - 1]) {
        end -= 1;
    }
    let byte_before = |back: usize| end.checked_sub(back).map(|index| code[index]);
    let padded = end < at;
    byte_before(1) == Some(OPCODE_RET)
        || padded
            && (byte_before(RET_IMM16_SIZE) == Some(OPCODE_RET_IMM16)
                || byte_before(REL32_INSTRUCTION_SIZE) == Some(OPCODE_JMP_REL32)
                || byte_before(JMP_REL8_SIZE) == Some(OPCODE_JMP_REL8))
}

/// The start of the function holding the instruction that begins at or before code offset `inside`: the nearest
/// [`is_function_start`] at most [`MAX_FUNCTION_SCAN`] bytes before it.
pub fn function_start(code: &[u8], text_va: u32, inside: usize) -> Option<usize> {
    (inside.saturating_sub(MAX_FUNCTION_SCAN)..=inside).rev().find(|at| is_function_start(code, text_va, *at))
}

/// Every `CALL rel32` and `JMP rel32` of `.text`, by target.
struct Branches {
    calls: HashMap<usize, Vec<usize>>,
    jumps: HashMap<usize, Vec<usize>>,
}

impl Branches {
    fn scan(code: &[u8]) -> Self {
        let mut branches = Self { calls: HashMap::new(), jumps: HashMap::new() };
        for at in 0..code.len().saturating_sub(REL32_INSTRUCTION_SIZE - 1) {
            let map = match code[at] {
                OPCODE_CALL_REL32 => &mut branches.calls,
                OPCODE_JMP_REL32 => &mut branches.jumps,
                _ => continue,
            };
            let rel = read_code_u32(code, at + 1).expect("bounded by the loop") as i32;
            let target = (at + REL32_INSTRUCTION_SIZE) as i64 + i64::from(rel);
            if let Ok(target) = usize::try_from(target)
                && target < code.len()
            {
                map.entry(target).or_default().push(at);
            }
        }
        branches
    }
}

/// The VAs of every u32 equal to `va` in the file bytes of the sections.
fn stored_pointers(bytes: &[u8], pe: &PeImage, va: u32) -> Vec<u32> {
    let needle = va.to_le_bytes();
    let mut sites = Vec::new();
    for section in &pe.sections {
        let range = section.raw_range();
        let Some(raw) = bytes.get(range.start..range.end.min(bytes.len())) else {
            continue;
        };
        sites.extend(
            raw.windows(needle.len())
                .enumerate()
                .filter(|(_, window)| *window == needle)
                .map(|(at, _)| section.virtual_address + at as u32),
        );
    }
    sites
}

/// Everything that reaches the function at code offset `entry`. A `JMP rel32` at a function start is a thunk: its
/// own callers count, and a thunk nothing reaches does not.
fn callers(bytes: &[u8], pe: &PeImage, code: &[u8], branches: &Branches, text_va: u32, entry: usize) -> Vec<DifCaller> {
    let va = |at: usize| text_va + at as u32;
    let mut found: Vec<DifCaller> = branches
        .calls
        .get(&entry)
        .into_iter()
        .flatten()
        .map(|site| DifCaller { site_va: va(*site), kind: DifCallKind::Call })
        .collect();
    for jump in branches.jumps.get(&entry).into_iter().flatten() {
        let thunk_va = va(*jump);
        let thunk_callers: Vec<u32> = branches
            .calls
            .get(jump)
            .into_iter()
            .flatten()
            .map(|site| va(*site))
            .chain(stored_pointers(bytes, pe, thunk_va))
            .collect();
        if thunk_callers.is_empty() {
            if !is_function_start(code, text_va, *jump) {
                found.push(DifCaller { site_va: thunk_va, kind: DifCallKind::TailJump });
            }
            continue;
        }
        found.extend(thunk_callers.into_iter().map(|site_va| DifCaller { site_va, kind: DifCallKind::Thunk { thunk_va } }));
    }
    found.extend(
        stored_pointers(bytes, pe, va(entry)).into_iter().map(|site_va| DifCaller { site_va, kind: DifCallKind::Pointer }),
    );
    found
}

/// Up to [`MAX_LISTED_SITES`] VAs, then "...".
fn list_sites(sites: impl Iterator<Item = u32>) -> String {
    let sites: Vec<u32> = sites.collect();
    let mut listed: Vec<String> = sites.iter().take(MAX_LISTED_SITES).map(|site| format!("{site:#010x}")).collect();
    if sites.len() > MAX_LISTED_SITES {
        listed.push("...".to_owned());
    }
    listed.join(", ")
}

fn describe_caller(caller: &DifCaller) -> String {
    match caller.kind {
        DifCallKind::Call => format!("CALL at {:#010x}", caller.site_va),
        DifCallKind::TailJump => format!("JMP at {:#010x}", caller.site_va),
        DifCallKind::Thunk { thunk_va } => format!("{:#010x} through the thunk {thunk_va:#010x}", caller.site_va),
        DifCallKind::Pointer => format!("pointer at {:#010x}", caller.site_va),
    }
}

/// The findings of one kind.
struct Side {
    status: DifStatus,
    loaders: Vec<DifLoader>,
    lookups: Vec<DifLookup>,
    evidence: Vec<String>,
}

impl Side {
    fn new(status: DifStatus, evidence: String) -> Self {
        Self { status, loaders: Vec::new(), lookups: Vec::new(), evidence: vec![evidence] }
    }
}

/// The data-section VAs of the NUL-terminated `text`.
fn string_vas(bytes: &[u8], pe: &PeImage, text: &str) -> Vec<u32> {
    let mut needle = text.as_bytes().to_vec();
    needle.push(0);
    pe.sections
        .iter()
        .filter(|section| section.is_data())
        .flat_map(|section| {
            let range = section.raw_range();
            let raw = bytes.get(range.start..range.end.min(bytes.len())).unwrap_or_default();
            raw.windows(needle.len())
                .enumerate()
                .filter(|(_, window)| *window == needle.as_slice())
                .map(|(at, _)| section.virtual_address + at as u32)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn inspect_side(bytes: &[u8], pe: &PeImage, code: &[u8], text_va: u32, branches: &mut Option<Branches>, kind: DifKind) -> Side {
    let label = kind.label();
    let format = kind.list_format();
    let strings = string_vas(bytes, pe, format);
    if strings.is_empty() {
        return Side::new(DifStatus::Absent, format!("{label}: no `{format}` string: the client never loads the list"));
    }
    let pushes: Vec<usize> = strings
        .iter()
        .flat_map(|va| {
            let mut push = vec![OPCODE_PUSH_IMM32];
            push.extend_from_slice(&va.to_le_bytes());
            code.windows(push.len()).enumerate().filter(|(_, window)| *window == push.as_slice()).map(|(at, _)| at).collect::<Vec<_>>()
        })
        .collect();
    if pushes.is_empty() {
        let at = list_sites(strings.iter().copied());
        return Side::new(
            DifStatus::Unknown(format!("the `{format}` string ({at}) is never pushed: the loader has another shape")),
            format!("{label}: `{format}` at {at}, no PUSH imm32 of it in .text"),
        );
    }
    let mut side = Side::new(DifStatus::LoadedNotApplied, String::new());
    side.evidence.clear();
    let mut loader_sites = Vec::new();
    for push in &pushes {
        let push_va = text_va + *push as u32;
        let function_va = function_start(code, text_va, *push).map(|start| text_va + start as u32);
        match loader_maps(code, pe, *push) {
            Some((mov, maps)) => {
                loader_sites.push(mov);
                side.loaders.push(DifLoader { kind, push_va, function_va, maps });
            }
            None => side.evidence.push(format!(
                "{label}: the `{format}` push at {push_va:#010x} has no MOV r32,imm32 of a writable array advanced by \
                 a map size"
            )),
        }
    }
    let Some(first) = side.loaders.first().map(|loader| loader.maps) else {
        side.status = DifStatus::Unknown(format!("the `{format}` loader's map array was not found"));
        return side;
    };
    if side.loaders.iter().any(|loader| loader.maps.va != first.va || loader.maps.map_size != first.map_size) {
        side.status = DifStatus::Unknown(format!("the `{format}` loaders fill different map arrays"));
        return side;
    }
    // Every u32 in the largest array the maps can be; the constructor's count bounds it.
    let widest = first.va..first.va.saturating_add(first.map_size * u32::from(MAX_DIF_MAPS));
    let references: Vec<(usize, u32, MapReference)> = (0..code.len().saturating_sub(size_of::<u32>() - 1))
        .filter_map(|at| {
            let value = read_code_u32(code, at)?;
            widest.contains(&value).then(|| (at, value, classify_reference(code, at, first.map_size, &loader_sites)))
        })
        .collect();
    let counts: Vec<u32> = references
        .iter()
        .filter_map(|(_, value, class)| match class {
            MapReference::Construction { count } if *value == first.va => Some(u32::from(*count)),
            _ => None,
        })
        .collect();
    let mut maps = first;
    maps.count = counts.first().copied().filter(|count| counts.iter().all(|each| each == count));
    for loader in &mut side.loaders {
        loader.maps = maps;
        let at = loader.function_va.map_or_else(|| "a function not delimited".to_owned(), |va| format!("{va:#010x}"));
        let count = maps.count.map_or_else(|| "maps (count not found)".to_owned(), |count| format!("{count} maps"));
        side.evidence.push(format!(
            "{label}: loader {at} pushes `{format}` at {:#010x} and fills {count} of {} bytes at {:#010x}",
            loader.push_va, maps.map_size, maps.va
        ));
    }
    let array = maps.range();
    let first_map = maps.va..maps.va + maps.map_size;
    let mut others = Vec::new();
    let mut reads: Vec<(usize, u32)> = Vec::new();
    for (at, value, class) in references.into_iter().filter(|(_, value, _)| array.contains(value)) {
        match class {
            MapReference::Indexed if first_map.contains(&value) => reads.push((at, text_va + at as u32)),
            MapReference::Loader | MapReference::Construction { .. } => {}
            MapReference::Indexed | MapReference::Other => others.push(text_va + at as u32),
        }
    }
    let branches = branches.get_or_insert_with(|| Branches::scan(code));
    let mut undelimited = Vec::new();
    for (at, read_va) in reads {
        // The ModRM and SIB precede the disp32; the opcode precedes them.
        let Some(start) = at.checked_sub(size_of::<u16>() + 1).and_then(|inside| function_start(code, text_va, inside)) else {
            undelimited.push(read_va);
            continue;
        };
        let va = text_va + start as u32;
        match side.lookups.iter_mut().find(|lookup| lookup.va == va) {
            Some(lookup) => lookup.read_vas.push(read_va),
            None => side.lookups.push(DifLookup {
                kind,
                va,
                read_vas: vec![read_va],
                callers: callers(bytes, pe, code, branches, text_va, start),
            }),
        }
    }
    for lookup in &side.lookups {
        let reads = list_sites(lookup.read_vas.iter().copied());
        side.evidence.push(if lookup.callers.is_empty() {
            format!(
                "{label}: lookup {:#010x} reads the maps by facet (at {reads}); no caller: no CALL or JMP rel32 to it \
                 and no stored pointer",
                lookup.va
            )
        } else {
            let listed: Vec<String> = lookup.callers.iter().take(MAX_LISTED_SITES).map(describe_caller).collect();
            let more = if lookup.callers.len() > MAX_LISTED_SITES { ", ..." } else { "" };
            format!(
                "{label}: lookup {:#010x} reads the maps by facet (at {reads}); {} callers: {}{more}",
                lookup.va,
                lookup.callers.len(),
                listed.join(", ")
            )
        });
    }
    if side.lookups.is_empty() && undelimited.is_empty() {
        side.evidence.push(format!(
            "{label}: no function reads the maps by facet index (only the loader and the static constructor / \
             destructor reference them)"
        ));
    }
    if !others.is_empty() {
        side.evidence.push(format!("{label}: other references to the maps: {}", list_sites(others.iter().copied())));
    }
    side.status = if side.lookups.iter().any(|lookup| !lookup.callers.is_empty()) {
        DifStatus::Applied
    } else if !undelimited.is_empty() {
        DifStatus::Unknown(format!(
            "no function start within {MAX_FUNCTION_SCAN:#x} bytes before the map reads at {}",
            list_sites(undelimited.into_iter())
        ))
    } else if !others.is_empty() {
        DifStatus::Unknown(format!(
            "no called lookup, but {} references to the maps the checker cannot explain ({})",
            others.len(),
            list_sites(others.into_iter())
        ))
    } else {
        DifStatus::LoadedNotApplied
    };
    side
}

/// Whether the client in `bytes` applies the land and statics patches (see the module docs). Meant for Classic
/// clients; any other executable reports [`DifStatus::Absent`].
pub fn inspect_classic_difs(bytes: &[u8], pe: &PeImage) -> ClassicDifUsage {
    let (code, start) = match code_section(bytes, pe) {
        Ok(found) => found,
        Err(error) => {
            let status = DifStatus::Unknown(format!("{error}"));
            return ClassicDifUsage {
                land: status.clone(),
                statics: status,
                loaders: Vec::new(),
                lookups: Vec::new(),
                evidence: vec![format!("{error}")],
            };
        }
    };
    let text_va = pe.offset_to_va(start).unwrap_or(pe.image_base);
    let mut branches = None;
    let [land, statics] = DifKind::ALL.map(|kind| inspect_side(bytes, pe, code, text_va, &mut branches, kind));
    let mut usage = ClassicDifUsage {
        land: land.status,
        statics: statics.status,
        loaders: land.loaders,
        lookups: land.lookups,
        evidence: land.evidence,
    };
    usage.loaders.extend(statics.loaders);
    usage.lookups.extend(statics.lookups);
    usage.evidence.extend(statics.evidence);
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::pe::tests::{synthetic_pe, TEST_IMAGE_BASE, TEST_SECTION_ALIGNMENT};

    const MAP_SIZE: u8 = 12;
    const MAP_COUNT: u8 = 5;
    const INT3: u8 = 0xcc;
    const PUSH_ESI: u8 = 0x56;
    /// `MOV ESI,imm32`, `ADD ESI,imm8`, `MOV ECX,[ESI*4+disp32]`, `MOV EAX,[moffs32]`.
    const MOV_ESI_IMM32: u8 = 0xbe;
    const ADD_ESI_IMM8: [u8; 2] = [OPCODE_GROUP1_RM32_IMM8, MODRM_ADD_REGISTER + 6];
    const MOV_ECX_INDEXED: [u8; 2] = [0x8b, 0x0c];
    const SIB_ESI_TIMES_4_NO_BASE: u8 = 0xb5;
    const MOV_EAX_MOFFS32: u8 = 0xa1;
    const OR_CL_AL: [u8; 2] = [0x0a, 0xc8];
    /// The synthetic sections: `.text` of [`CODE_SIZE`] bytes, then `.rdata` and `.data`, each at its own page.
    const CODE_SIZE: usize = 0x400;
    const TEXT_VA: u32 = TEST_IMAGE_BASE + TEST_SECTION_ALIGNMENT;
    const RDATA_VA: u32 = TEXT_VA + TEST_SECTION_ALIGNMENT;
    const DATA_VA: u32 = RDATA_VA + TEST_SECTION_ALIGNMENT;
    const DATA_SIZE: usize = 0x400;
    /// Per side: its code, its format string and its maps start here (land, statics).
    const SIDE_CODE: [usize; 2] = [0, 0x200];
    const SIDE_STRING: [usize; 2] = [0, 0x20];
    const SIDE_MAPS: [usize; 2] = [0x10, 0x100];
    const SIDE_POINTER: [usize; 2] = [0x300, 0x310];
    /// Functions inside a side's code.
    const LOADER_AT: usize = 0;
    const CONSTRUCTOR_AT: usize = 0x40;
    const GUARDED_REGISTRATION_AT: usize = 0x60;
    const LOOKUP_AT: usize = 0x80;
    const CALLER_AT: usize = 0xc0;
    const THUNK_CALLER_AT: usize = 0x100;
    const STRAY_AT: usize = 0x140;

    /// How a synthetic lookup is reached.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Reach {
        Nothing,
        Call,
        Thunk,
        Pointer,
        TailJump,
    }

    #[derive(Debug, Clone, Copy)]
    struct SideSpec {
        string: bool,
        loader: bool,
        lookup: Option<Reach>,
        /// A `MOV EAX,[maps + map_size]`: a reference the checker cannot explain.
        stray: bool,
    }

    /// Gold-like: loaded and called.
    const APPLIED: SideSpec = SideSpec { string: true, loader: true, lookup: Some(Reach::Call), stray: false };
    /// 7.0-like: the lookup has no caller.
    const DEAD: SideSpec = SideSpec { lookup: Some(Reach::Nothing), ..APPLIED };
    /// 6.0.14.3-like: no lookup.
    const NO_LOOKUP: SideSpec = SideSpec { lookup: None, ..APPLIED };

    fn put(buffer: &mut [u8], at: usize, value: u32) {
        buffer[at..at + size_of::<u32>()].copy_from_slice(&value.to_le_bytes());
    }

    fn rel32(code: &mut [u8], at: usize, opcode: u8, target: usize) {
        code[at] = opcode;
        put(code, at + 1, (target as i64 - (at + REL32_INSTRUCTION_SIZE) as i64) as i32 as u32);
        code[at + REL32_INSTRUCTION_SIZE] = OPCODE_RET;
    }

    fn synthetic(sides: [SideSpec; 2]) -> Vec<u8> {
        let mut code = vec![INT3; CODE_SIZE];
        let mut rdata = vec![0u8; TEST_SECTION_ALIGNMENT as usize / 4];
        let mut data = vec![0u8; DATA_SIZE];
        for (index, (spec, kind)) in sides.iter().zip(DifKind::ALL).enumerate() {
            let base = SIDE_CODE[index];
            let format_va = RDATA_VA + SIDE_STRING[index] as u32;
            let maps_va = DATA_VA + SIDE_MAPS[index] as u32;
            if spec.string {
                let format = kind.list_format().as_bytes();
                rdata[SIDE_STRING[index]..SIDE_STRING[index] + format.len()].copy_from_slice(format);
            }
            if spec.loader {
                let at = base + LOADER_AT;
                code[at] = MOV_ESI_IMM32;
                put(&mut code, at + 1, maps_va);
                code[at + 5] = OPCODE_PUSH_IMM32;
                put(&mut code, at + 6, format_va);
                code[at + 10..at + 12].copy_from_slice(&ADD_ESI_IMM8);
                code[at + 12] = MAP_SIZE;
                code[at + 13] = OPCODE_RET;
                let at = base + CONSTRUCTOR_AT;
                code[at..at + 5].copy_from_slice(&[OPCODE_PUSH_IMM8, MAP_COUNT, OPCODE_PUSH_IMM8, MAP_SIZE, OPCODE_PUSH_IMM32]);
                put(&mut code, at + 5, maps_va);
                code[at + 9] = OPCODE_RET;
                let at = base + GUARDED_REGISTRATION_AT;
                code[at..at + 7].copy_from_slice(&[
                    OPCODE_PUSH_IMM8,
                    MAP_COUNT,
                    OR_CL_AL[0],
                    OR_CL_AL[1],
                    OPCODE_PUSH_IMM8,
                    MAP_SIZE,
                    OPCODE_PUSH_IMM32,
                ]);
                put(&mut code, at + 7, maps_va);
                code[at + 11] = OPCODE_RET;
            }
            let lookup = base + LOOKUP_AT;
            if let Some(reach) = spec.lookup {
                code[lookup] = PUSH_ESI;
                code[lookup + 1..lookup + 3].copy_from_slice(&MOV_ECX_INDEXED);
                code[lookup + 3] = SIB_ESI_TIMES_4_NO_BASE;
                put(&mut code, lookup + 4, maps_va);
                code[lookup + 8] = OPCODE_RET;
                let caller = base + CALLER_AT;
                match reach {
                    Reach::Nothing => {}
                    Reach::Call => rel32(&mut code, caller, OPCODE_CALL_REL32, lookup),
                    Reach::Thunk => {
                        rel32(&mut code, caller, OPCODE_JMP_REL32, lookup);
                        rel32(&mut code, base + THUNK_CALLER_AT, OPCODE_CALL_REL32, caller);
                    }
                    Reach::Pointer => put(&mut data, SIDE_POINTER[index], TEXT_VA + lookup as u32),
                    Reach::TailJump => {
                        code[caller] = PUSH_ESI;
                        rel32(&mut code, caller + 1, OPCODE_JMP_REL32, lookup);
                    }
                }
            }
            if spec.stray {
                let at = base + STRAY_AT;
                code[at] = MOV_EAX_MOFFS32;
                put(&mut code, at + 1, maps_va + u32::from(MAP_SIZE));
                code[at + 5] = OPCODE_RET;
            }
        }
        synthetic_pe(&[(".text", code), (".rdata", rdata), (".data", data)], false)
    }

    fn inspect(sides: [SideSpec; 2]) -> ClassicDifUsage {
        let image = synthetic(sides);
        let pe = PeImage::parse(&image).unwrap();
        assert_eq!(pe.section(".text").unwrap().virtual_address, TEXT_VA);
        inspect_classic_difs(&image, &pe)
    }

    fn lookup_va(side: usize) -> u32 {
        TEXT_VA + (SIDE_CODE[side] + LOOKUP_AT) as u32
    }

    #[test]
    fn called_lookups_apply_the_patches() {
        let usage = inspect([APPLIED, APPLIED]);
        assert_eq!((&usage.land, &usage.statics), (&DifStatus::Applied, &DifStatus::Applied), "{:#?}", usage.evidence);
        assert_eq!(usage.loaders.len(), DifKind::ALL.len());
        for (index, loader) in usage.loaders.iter().enumerate() {
            assert_eq!(loader.kind, DifKind::ALL[index]);
            assert_eq!(loader.function_va, Some(TEXT_VA + (SIDE_CODE[index] + LOADER_AT) as u32));
            let maps = DifMaps { va: DATA_VA + SIDE_MAPS[index] as u32, map_size: MAP_SIZE.into(), count: Some(MAP_COUNT.into()) };
            assert_eq!(loader.maps, maps);
        }
        for (index, lookup) in usage.lookups.iter().enumerate() {
            assert_eq!((lookup.kind, lookup.va), (DifKind::ALL[index], lookup_va(index)));
            let site_va = TEXT_VA + (SIDE_CODE[index] + CALLER_AT) as u32;
            assert_eq!(lookup.callers, vec![DifCaller { site_va, kind: DifCallKind::Call }]);
        }
        assert!(usage.evidence.iter().any(|line| line.starts_with("land (mapdif): lookup") && line.contains("1 callers")));
        assert!(!usage.evidence.iter().any(|line| line.contains("other references")), "{:#?}", usage.evidence);
        assert_eq!(usage.status(DifKind::Statics).applies(), Some(true));
        assert!(usage.advice().starts_with("convert the map with"));
    }

    #[test]
    fn thunks_pointers_and_tail_jumps_are_callers() {
        for (reach, kind) in [
            (Reach::Thunk, DifCallKind::Thunk { thunk_va: TEXT_VA + CALLER_AT as u32 }),
            (Reach::Pointer, DifCallKind::Pointer),
            (Reach::TailJump, DifCallKind::TailJump),
        ] {
            let usage = inspect([SideSpec { lookup: Some(reach), ..APPLIED }, DEAD]);
            assert_eq!(usage.land, DifStatus::Applied, "{reach:?}: {:#?}", usage.evidence);
            assert_eq!(usage.statics, DifStatus::LoadedNotApplied, "the sides are told apart");
            assert!(usage.advice().starts_with("apply only"));
            let callers = &usage.lookups[0].callers;
            assert_eq!(callers.len(), 1, "{reach:?}: {callers:?}");
            assert_eq!(callers[0].kind, kind);
        }
        // A thunk nothing calls is no caller.
        let image = synthetic([SideSpec { lookup: Some(Reach::Thunk), ..APPLIED }, DEAD]);
        let mut orphan = image.clone();
        let text = PeImage::parse(&image).unwrap().section(".text").unwrap().raw_offset as usize;
        orphan[text + THUNK_CALLER_AT..text + THUNK_CALLER_AT + REL32_INSTRUCTION_SIZE + 1].fill(INT3);
        let usage = inspect_classic_difs(&orphan, &PeImage::parse(&orphan).unwrap());
        assert_eq!(usage.land, DifStatus::LoadedNotApplied, "{:#?}", usage.evidence);
    }

    #[test]
    fn dead_or_missing_lookups_load_without_applying() {
        let usage = inspect([DEAD, NO_LOOKUP]);
        assert_eq!((&usage.land, &usage.statics), (&DifStatus::LoadedNotApplied, &DifStatus::LoadedNotApplied));
        assert_eq!(usage.lookups.len(), 1);
        assert_eq!(usage.lookups[0].va, lookup_va(0));
        assert!(usage.lookups[0].callers.is_empty());
        assert!(usage.evidence.iter().any(|line| line.starts_with("land (mapdif): lookup") && line.contains("no caller")));
        assert!(usage.evidence.iter().any(|line| line.starts_with("statics (stadif): no function reads")));
        assert_eq!(usage.land.applies(), Some(false));
        assert!(usage.advice().starts_with("convert the map without"));
    }

    #[test]
    fn missing_loaders_and_unexplained_references() {
        let absent = SideSpec { string: false, loader: false, lookup: None, stray: false };
        let usage = inspect([absent, absent]);
        assert_eq!((&usage.land, &usage.statics), (&DifStatus::Absent, &DifStatus::Absent));
        assert!(usage.loaders.is_empty() && usage.lookups.is_empty());

        let usage = inspect([SideSpec { loader: false, ..NO_LOOKUP }, SideSpec { stray: true, ..DEAD }]);
        assert!(matches!(&usage.land, DifStatus::Unknown(reason) if reason.contains("never pushed")), "{:?}", usage.land);
        let stray_va = TEXT_VA + (SIDE_CODE[1] + STRAY_AT + 1) as u32;
        assert!(
            matches!(&usage.statics, DifStatus::Unknown(reason) if reason.contains(&format!("{stray_va:#010x}"))),
            "{:?}",
            usage.statics
        );
        assert_eq!(usage.statics.applies(), None);
        assert!(usage.advice().starts_with("check the evidence"));
        // A called lookup is enough, whatever else references the maps.
        assert_eq!(inspect([SideSpec { stray: true, ..APPLIED }, APPLIED]).land, DifStatus::Applied);
    }

    #[test]
    fn the_inspection_of_a_classic_client_carries_its_usage() {
        use crate::client::classic_facets::tests::synthetic_classic;
        use crate::client::facet_dimensions::ClientExeInspection;
        let difs = ClientExeInspection::inspect(&synthetic_classic(5, 1)).unwrap().difs.unwrap();
        assert_eq!((&difs.land, &difs.statics), (&DifStatus::Absent, &DifStatus::Absent));
    }

    #[test]
    fn function_starts_follow_the_padding_of_both_compilers() {
        const NOP: u8 = 0x90;
        const RET_8: [u8; RET_IMM16_SIZE] = [OPCODE_RET_IMM16, 8, 0];
        const START: usize = 0x20;
        let text_va = TEXT_VA;
        // MSVC 6: NOP padding after a RET; MSVC 8: INT3 after a RET 8; a JMP before the padding.
        for tail in [vec![OPCODE_RET, NOP, NOP], RET_8.iter().copied().chain([INT3]).collect(), vec![OPCODE_JMP_REL8, 0, INT3]] {
            let mut code = vec![PUSH_ESI; START + 0x10];
            code[START - tail.len()..START].copy_from_slice(&tail);
            assert_eq!(function_start(&code, text_va, START + 8), Some(START), "{tail:x?}");
        }
        // An aligned address after an instruction that does not end a function is not a start.
        let mut code = vec![PUSH_ESI; START + 0x10];
        code[START - 1] = INT3;
        assert_eq!(function_start(&code, text_va, START + 8), Some(0));
        // Right after a RET with no padding.
        code[START - 1] = OPCODE_RET;
        assert_eq!(function_start(&code, text_va, START + 8), Some(START));
    }
}
