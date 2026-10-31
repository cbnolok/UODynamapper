//! Synthetic client executables for the tests, built like the uocf `client_exe` tests build theirs (their helpers
//! are private to that crate): a PE32 image with `.text`, `.rdata` and `.data`; an Enhanced Client-like table with
//! the UOSA split sequence, a Kingdom Reborn-like one, and a two-copy Classic table with its file name table, startup
//! globals and wrap seam. The login variants add `LoginAccount`'s address code as the uocf `client_exe::login` tests
//! build it: the fixed store, then (Enhanced Client) the public host pushes and port stores, the hosts in `.rdata`.

use std::net::Ipv4Addr;

use uocf::classic::map::CLASSIC_FACET_HEIGHTS;
use uocf::client_exe::classic_facets::{
    ClassicFileRole, CLASSIC_FACET_RECORD_SIZE, CLASSIC_FILE_ID_COUNT, CLASSIC_RECORD_FILE_IDS_OFFSET,
    CLASSIC_RECORD_FLAG_OFFSET, CLASSIC_RECORD_WRAP_HEIGHT_OFFSET, CLASSIC_RECORD_WRAP_WIDTH_OFFSET,
    CLASSIC_STOCK_WIDTHS, CLASSIC_STOCK_WRAP,
};
use uocf::client_exe::facet_dimensions::{
    ENHANCED_CLIENT_MARKER, FACET_RECORD_HEIGHT_OFFSET, FACET_RECORD_WIDTH_OFFSET, SPLIT_IMMEDIATE_COUNT,
    STOCK_FACET_COUNT, STOCK_SPLIT_X,
};
use uocf::client_exe::login::{
    LOGIN_HOST_SLOT_SIZE, STOCK_INTERNAL_LOGIN_ADDRESSES, STOCK_KR_LOGIN_ADDRESS, STOCK_LOGIN_PORTS,
};
use uocf::client_exe::pe::{
    pe_checksum, PeImage, MACHINE_I386, OPTIONAL_CHECKSUM_OFFSET, SECTION_CODE, SECTION_EXECUTE,
    SECTION_INITIALIZED_DATA, SECTION_READ, SECTION_WRITE,
};
use uocf::enhanced::facet_sector::{FacetGeometry, CLIENT_FACET_DIMENSIONS};

const WORD: usize = size_of::<u32>();

// PE layout of the synthetic images.
const IMAGE_BASE: u32 = 0x0040_0000;
const E_LFANEW_OFFSET: usize = 0x3c;
const PE_OFFSET: usize = 0x80;
const PE_SIGNATURE: &[u8; 4] = b"PE\0\0";
const COFF_HEADER_SIZE: usize = 20;
const COFF_SECTION_COUNT_OFFSET: usize = 2;
const COFF_OPTIONAL_HEADER_SIZE_OFFSET: usize = 16;
const OPTIONAL_SIZE: usize = 0xe0;
const PE32_MAGIC: u16 = 0x10b;
const OPTIONAL_IMAGE_BASE_OFFSET: usize = 28;
const OPTIONAL_DIRECTORY_COUNT_OFFSET: usize = 92;
const OPTIONAL_DIRECTORIES_OFFSET: usize = 96;
const DIRECTORY_COUNT: u32 = 16;
const DATA_DIRECTORY_SIZE: usize = 8;
const SECURITY_DIRECTORY: usize = 4;
const SECTION_HEADER_SIZE: usize = 40;
const SECTION_VIRTUAL_SIZE_OFFSET: usize = 8;
const SECTION_VIRTUAL_ADDRESS_OFFSET: usize = 12;
const SECTION_RAW_SIZE_OFFSET: usize = 16;
const SECTION_RAW_OFFSET_OFFSET: usize = 20;
const SECTION_CHARACTERISTICS_OFFSET: usize = 36;
const SECTION_ALIGNMENT: u32 = 0x1000;
/// Any non-zero certificate table size marks a signed image.
const SIGNED_CERTIFICATE_SIZE: u32 = 1;
const NOP: u8 = 0x90;
const INT3: u8 = 0xcc;
const RET: u8 = 0xc3;

/// The Enhanced Client facet names.
const EC_NAMES: [&str; STOCK_FACET_COUNT] = ["Felucca", "Trammel", "Ilshenar", "Malas", "Tokuno", "Gargoyle", "Test"];
/// Offset of the 12-byte table inside `.data`.
const EC_TABLE_DATA_OFFSET: usize = 16;
/// `UOFacet::IsSameFeluccaTrammelHalf` from the facet compare to its end (UOSA.exe file 0x1df6d2); the split imm32
/// at [`SPLIT_IMMEDIATES`].
const UOSA_SPLIT_CODE: [u8; 51] = [
    0x83, 0xff, 0x01, 0x7f, 0x24, 0x81, 0x7c, 0x24, 0x08, 0x00, 0x14, 0x00, 0x00, 0x7d, 0x12, 0x81, 0xfb, 0x00, 0x14,
    0x00, 0x00, 0x7c, 0x12, 0x32, 0xc0, 0x5e, 0x5b, 0x83, 0xc4, 0x0c, 0xc2, 0x04, 0x00, 0x81, 0xfb, 0x00, 0x14, 0x00,
    0x00, 0x7c, 0xee, 0x5e, 0xb0, 0x01, 0x5b, 0x83, 0xc4, 0x0c, 0xc2, 0x04, 0x00,
];
const SPLIT_IMMEDIATES: [usize; SPLIT_IMMEDIATE_COUNT] = [9, 17, 35];
const SPLIT_CODE_PADDING: usize = 8;
const NOP_SLED_SIZE: usize = 64;

/// The Classic facet names and the synthetic file name table (`<role><facet>.mul`, then the `x` variants of facets
/// 0 and 1, read by the second copy).
pub const CLASSIC_NAMES: [&str; 6] = ["Felucca", "Trammel", "Ilshenar", "Malas", "Tokuno", "TerMur"];
const VARIANT_FACETS: usize = 2;
const FILE_NAME_STRIDE: usize = 60;
const FILE_NAME_OFFSET: usize = WORD;
const FILE_ORDINAL_FROM_END: usize = 8;
/// The record size field of a file name entry (any value).
const FILE_RECORD_SIZE_FIELD: u32 = 196;
const CLASSIC_COPIES: usize = 2;
/// Offsets inside the Classic `.data`: the startup globals `{wrap w, wrap h, width, height}`, then the tables.
const STARTUP_DATA_OFFSET: usize = 16;
const STARTUP_FIELDS: [usize; 4] = [0, WORD, 2 * WORD, 3 * WORD];
const TABLES_DATA_OFFSET: usize = 64;
/// Code layout: the wrap width accessor at 0, `SetFacet` at [`SET_FACET_AT`], the seam sites after it.
const SET_FACET_AT: usize = 0x100;
const SEAM_AT: [usize; 2] = [0x180, 0x200];
const CLASSIC_CODE_SIZE: usize = 0x300;
const OPCODE_CALL_REL32: u8 = 0xe8;
const CALL_REL32_SIZE: usize = 5;
const OPCODE_PUSH_ESI: u8 = 0x56;
const OPCODE_STORE_EAX_MOFFS32: u8 = 0xa3;
/// `MOV EAX,[ECX+disp32]`.
const MOV_EAX_ECX_DISP32: [u8; 2] = [0x8b, 0x81];
/// `CMP EDX,imm32`, `SETGE AL`; `CMP EAX,imm32`, `MOV ECX,[ESP+4]`, `SETGE CL`.
const CMP_EDX_IMM32: [u8; 2] = [0x81, 0xfa];
const SETGE_AL: [u8; 3] = [0x0f, 0x9d, 0xc0];
const CMP_EAX_IMM32: u8 = 0x3d;
const MOV_ECX_ESP4_SETGE_CL: [u8; 7] = [0x8b, 0x4c, 0x24, 0x04, 0x0f, 0x9d, 0xc1];

/// The Time of Legends hosts of UOSA 4.0.102.3, in 16-byte slots.
pub const TOL_HOSTS: [&str; 2] = ["107.23.85.115", "107.23.176.74"];
/// A second host slot of 12 bytes (UOSA 4.0.115.0's 75.2.70.184).
pub const SHORT_SLOT_HOST: &str = "75.2.70.184";
/// The Enhanced Client's internal test address.
pub const INTERNAL_IP: Ipv4Addr = STOCK_INTERNAL_LOGIN_ADDRESSES[0].0;
/// The stack displacement of the `{ip, port}` pair.
const PAIR_DISPLACEMENT: u8 = 0x14;
const PAIR_PORT_OFFSET: u8 = WORD as u8;
/// After the last host: zero padding, then a non-zero byte that ends the slot.
const RDATA_TAIL_PADDING: usize = 4;
const RDATA_TAIL_TEXT: &[u8] = b"LoginManager";
const LOGIN_CODE_PADDING: usize = 16;
/// `MOV DWORD [ESP+disp8],imm32` and `MOV WORD [ESP+disp8],imm16` without their displacement.
const MOV_DWORD_ESP: [u8; 3] = [0xc7, 0x44, 0x24];
const MOV_WORD_ESP: [u8; 4] = [0x66, 0xc7, 0x44, 0x24];
/// The length of `MOV WORD [ESP+disp8],imm16`, the `JNE` skip over the second port store.
const MOV_WORD_ESP_LEN: u8 = 7;
/// `MOV [ESP+disp8],EDI` without its displacement.
const MOV_ESP_EDI: [u8; 3] = [0x89, 0x7c, 0x24];
/// `LEA EDX,[ESP+disp8]` without its displacement, then `PUSH EDX`.
const LEA_EDX_ESP: [u8; 3] = [0x8d, 0x54, 0x24];
const OPCODE_PUSH_EDX: u8 = 0x52;
const OPCODE_JMP_REL32: u8 = 0xe9;
const JMP_REL32_SIZE: usize = 5;
/// `CALL rand; AND EAX,0x80000001; LEA ECX,[ESP+1C]`.
const RAND_DRAW: [u8; 14] = [0xe8, 0, 0, 0, 0, 0x25, 0x01, 0, 0, 0x80, 0x8d, 0x4c, 0x24, 0x1c];
/// `CALL ResolveHostIPv4; MOV EDI,EAX`.
const RESOLVE_HOST: [u8; 7] = [0xe8, 0, 0, 0, 0, 0x8b, 0xf8];
const OPCODE_PUSH_IMM32: u8 = 0x68;
/// `JE +7` over `PUSH host1; JMP +5`, and that `JMP +5` over `PUSH host2`.
const JE_OVER_FIRST_PUSH: [u8; 2] = [0x74, 7];
const JMP_OVER_SECOND_PUSH: [u8; 2] = [0xeb, 5];
const OPCODE_JNE_REL8: u8 = 0x75;
const LOGIN_CODE_TAIL: usize = 16;

/// The login address code of a synthetic image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntheticLogin {
    /// Time of Legends: two hosts (the second in a 12-byte slot when `short_slot`) and two ports.
    TimeOfLegends { short_slot: bool },
    /// Before Time of Legends: one host and one port.
    OneHost,
}

fn put(buffer: &mut [u8], at: usize, value: u32) {
    buffer[at..at + WORD].copy_from_slice(&value.to_le_bytes());
}

fn characteristics(name: &str) -> u32 {
    match name {
        ".text" => SECTION_CODE | SECTION_EXECUTE | SECTION_READ,
        ".data" => SECTION_INITIALIZED_DATA | SECTION_READ | SECTION_WRITE,
        _ => SECTION_INITIALIZED_DATA | SECTION_READ,
    }
}

/// A PE32 image with `sections`, each at an aligned RVA equal to its raw offset, and a valid checksum.
fn synthetic_pe(sections: &[(&str, Vec<u8>)], signed: bool) -> Vec<u8> {
    let mut bytes = vec![0u8; SECTION_ALIGNMENT as usize];
    bytes[..2].copy_from_slice(b"MZ");
    put(&mut bytes, E_LFANEW_OFFSET, PE_OFFSET as u32);
    bytes[PE_OFFSET..PE_OFFSET + PE_SIGNATURE.len()].copy_from_slice(PE_SIGNATURE);
    let coff = PE_OFFSET + PE_SIGNATURE.len();
    bytes[coff..coff + 2].copy_from_slice(&MACHINE_I386.to_le_bytes());
    bytes[coff + COFF_SECTION_COUNT_OFFSET..][..2].copy_from_slice(&(sections.len() as u16).to_le_bytes());
    bytes[coff + COFF_OPTIONAL_HEADER_SIZE_OFFSET..][..2].copy_from_slice(&(OPTIONAL_SIZE as u16).to_le_bytes());
    let optional = coff + COFF_HEADER_SIZE;
    bytes[optional..optional + 2].copy_from_slice(&PE32_MAGIC.to_le_bytes());
    put(&mut bytes, optional + OPTIONAL_IMAGE_BASE_OFFSET, IMAGE_BASE);
    put(&mut bytes, optional + OPTIONAL_DIRECTORY_COUNT_OFFSET, DIRECTORY_COUNT);
    if signed {
        let entry = optional + OPTIONAL_DIRECTORIES_OFFSET + DATA_DIRECTORY_SIZE * SECURITY_DIRECTORY;
        put(&mut bytes, entry + WORD, SIGNED_CERTIFICATE_SIZE);
    }
    let mut raw_offset = SECTION_ALIGNMENT;
    for (index, (name, contents)) in sections.iter().enumerate() {
        let header = optional + OPTIONAL_SIZE + SECTION_HEADER_SIZE * index;
        bytes[header..header + name.len()].copy_from_slice(name.as_bytes());
        let size = contents.len() as u32;
        for (field, value) in [
            (SECTION_VIRTUAL_SIZE_OFFSET, size),
            (SECTION_VIRTUAL_ADDRESS_OFFSET, raw_offset),
            (SECTION_RAW_SIZE_OFFSET, size),
            (SECTION_RAW_OFFSET_OFFSET, raw_offset),
            (SECTION_CHARACTERISTICS_OFFSET, characteristics(name)),
        ] {
            put(&mut bytes, header + field, value);
        }
        bytes.resize(raw_offset as usize, 0);
        bytes.extend_from_slice(contents);
        raw_offset = (bytes.len() as u32).next_multiple_of(SECTION_ALIGNMENT);
    }
    let checksum_offset = optional + OPTIONAL_CHECKSUM_OFFSET;
    debug_assert_eq!(PeImage::parse(&bytes).unwrap().checksum_offset, checksum_offset);
    let checksum = pe_checksum(&bytes, checksum_offset);
    put(&mut bytes, checksum_offset, checksum);
    bytes
}

/// The RVA of the section after one of `previous_size` bytes at `previous_rva`.
fn next_rva(previous_rva: u32, previous_size: usize) -> u32 {
    previous_rva + SECTION_ALIGNMENT * (previous_size as u32).div_ceil(SECTION_ALIGNMENT)
}

fn split_code(x: u32) -> Vec<u8> {
    let mut code = vec![NOP; SPLIT_CODE_PADDING];
    code.extend(UOSA_SPLIT_CODE);
    for offset in SPLIT_IMMEDIATES {
        put(&mut code, SPLIT_CODE_PADDING + offset, x);
    }
    code.extend([INT3; SPLIT_CODE_PADDING]);
    code
}

/// The `.rdata` prefix with the host strings, each in a 16-byte slot (the last one only as long as its string and
/// NUL when `short_last`), then the tail. Returns it and the offsets of the strings.
fn host_rdata(hosts: &[&str], short_last: bool) -> (Vec<u8>, Vec<usize>) {
    let mut rdata = Vec::new();
    let mut offsets = Vec::new();
    for (index, host) in hosts.iter().enumerate() {
        offsets.push(rdata.len());
        rdata.extend_from_slice(host.as_bytes());
        let slot = if short_last && index + 1 == hosts.len() { host.len() + 1 } else { LOGIN_HOST_SLOT_SIZE };
        rdata.resize(offsets[index] + slot, 0);
    }
    if !short_last {
        rdata.extend([0; RDATA_TAIL_PADDING]);
    }
    rdata.extend_from_slice(RDATA_TAIL_TEXT);
    rdata.push(0);
    (rdata, offsets)
}

/// `LoginAccount`'s address code: the fixed store of `fixed`, a `JMP` to the `LEA` of the pair, and between them the
/// pushes of the hosts at `host_vas` (none for Kingdom Reborn) and the stores of `ports`.
fn login_code(host_vas: &[u32], fixed: (Ipv4Addr, u16), ports: &[u16]) -> Vec<u8> {
    let disp = PAIR_DISPLACEMENT;
    let port_disp = disp + PAIR_PORT_OFFSET;
    let mut code = vec![NOP; LOGIN_CODE_PADDING];
    code.extend(MOV_DWORD_ESP);
    code.push(disp);
    code.extend(u32::from(fixed.0).to_le_bytes());
    code.extend(MOV_WORD_ESP);
    code.push(port_disp);
    code.extend(fixed.1.to_le_bytes());
    let jump = code.len();
    code.extend([OPCODE_JMP_REL32, 0, 0, 0, 0]);
    code.extend(RAND_DRAW);
    match host_vas {
        [one] => {
            code.push(OPCODE_PUSH_IMM32);
            code.extend(one.to_le_bytes());
        }
        [first, second] => {
            code.extend(JE_OVER_FIRST_PUSH);
            code.push(OPCODE_PUSH_IMM32);
            code.extend(first.to_le_bytes());
            code.extend(JMP_OVER_SECOND_PUSH);
            code.push(OPCODE_PUSH_IMM32);
            code.extend(second.to_le_bytes());
        }
        _ => {}
    }
    code.extend(RESOLVE_HOST);
    code.extend(MOV_ESP_EDI);
    code.push(disp);
    code.extend(MOV_WORD_ESP);
    code.push(port_disp);
    code.extend(ports[0].to_le_bytes());
    if let Some(second) = ports.get(1) {
        code.extend([OPCODE_JNE_REL8, MOV_WORD_ESP_LEN]);
        code.extend(MOV_WORD_ESP);
        code.push(port_disp);
        code.extend(second.to_le_bytes());
    }
    let target = code.len();
    code.extend(LEA_EDX_ESP);
    code.extend([disp, OPCODE_PUSH_EDX]);
    let rel = (target - (jump + JMP_REL32_SIZE)) as i32;
    code[jump + 1..jump + JMP_REL32_SIZE].copy_from_slice(&rel.to_le_bytes());
    code.extend([INT3; LOGIN_CODE_TAIL]);
    code
}

/// The `.text` tail and `.rdata` prefix of `login`, for a `.text` whose first `head_len` bytes are other code.
fn enhanced_login_parts(head_len: usize, login: SyntheticLogin) -> (Vec<u8>, Vec<u8>) {
    let (hosts, short_slot, ports): (Vec<&str>, bool, &[u16]) = match login {
        SyntheticLogin::TimeOfLegends { short_slot: false } => (TOL_HOSTS.to_vec(), false, &STOCK_LOGIN_PORTS),
        SyntheticLogin::TimeOfLegends { short_slot: true } => {
            (vec![TOL_HOSTS[0], SHORT_SLOT_HOST], true, &STOCK_LOGIN_PORTS)
        }
        SyntheticLogin::OneHost => (vec![TOL_HOSTS[0]], false, &STOCK_LOGIN_PORTS[..1]),
    };
    let (rdata, offsets) = host_rdata(&hosts, short_slot);
    let fixed = (INTERNAL_IP, STOCK_LOGIN_PORTS[0]);
    // The code's length does not depend on the host addresses.
    let code_len = login_code(&vec![0; hosts.len()], fixed, ports).len();
    let rdata_va = IMAGE_BASE + next_rva(SECTION_ALIGNMENT, head_len + code_len);
    let vas: Vec<u32> = offsets.iter().map(|offset| rdata_va + *offset as u32).collect();
    (login_code(&vas, fixed, ports), rdata)
}

/// A 12-byte-table client with the 7 stock records: Enhanced Client-like (marker, and the split sequence when
/// `split` is set) or Kingdom Reborn-like; `login` adds the login address code and `.rdata` prefix.
fn twelve_byte_client(split: Option<u32>, enhanced: bool, signed: bool, login: Option<(Vec<u8>, Vec<u8>)>) -> Vec<u8> {
    let mut text = split.map(split_code).unwrap_or_else(|| vec![NOP; NOP_SLED_SIZE]);
    let (login_text, mut rdata) = login.unwrap_or_default();
    text.extend(login_text);
    let rdata_rva = next_rva(SECTION_ALIGNMENT, text.len());
    let mut pointers = Vec::new();
    for name in EC_NAMES {
        pointers.push(IMAGE_BASE + rdata_rva + rdata.len() as u32);
        rdata.extend_from_slice(name.as_bytes());
        rdata.push(0);
    }
    if enhanced {
        rdata.extend_from_slice(ENHANCED_CLIENT_MARKER);
        rdata.push(0);
    }
    let mut data = vec![0u8; EC_TABLE_DATA_OFFSET];
    for (pointer, geometry) in pointers.iter().zip(CLIENT_FACET_DIMENSIONS) {
        for value in [*pointer, geometry.width, geometry.height] {
            data.extend_from_slice(&value.to_le_bytes());
        }
    }
    data.extend(vec![0; EC_TABLE_DATA_OFFSET]);
    synthetic_pe(&[(".text", text), (".rdata", rdata), (".data", data)], signed)
}

/// An Enhanced Client-like image with the stock split at x = 5120.
pub fn synthetic_enhanced(signed: bool) -> Vec<u8> {
    twelve_byte_client(Some(STOCK_SPLIT_X), true, signed, None)
}

/// [`synthetic_enhanced`] with the login address code of `login` and the stock hosts and ports.
pub fn synthetic_enhanced_login(login: SyntheticLogin) -> Vec<u8> {
    let parts = enhanced_login_parts(split_code(STOCK_SPLIT_X).len(), login);
    twelve_byte_client(Some(STOCK_SPLIT_X), true, false, Some(parts))
}

/// A Kingdom Reborn-like image: no marker, no split.
pub fn synthetic_kingdom_reborn() -> Vec<u8> {
    twelve_byte_client(None, false, false, None)
}

/// [`synthetic_kingdom_reborn`] with its one built-in address, [`STOCK_KR_LOGIN_ADDRESS`].
pub fn synthetic_kingdom_reborn_login() -> Vec<u8> {
    let code = login_code(&[], STOCK_KR_LOGIN_ADDRESS, &STOCK_LOGIN_PORTS[..1]);
    twelve_byte_client(None, false, false, Some((code, Vec::new())))
}

pub fn classic_stock_size(facet: usize) -> FacetGeometry {
    FacetGeometry::new(CLASSIC_STOCK_WIDTHS[facet], CLASSIC_FACET_HEIGHTS[facet])
}

pub fn classic_stock_wrap(facet: usize) -> FacetGeometry {
    if facet < VARIANT_FACETS { CLASSIC_STOCK_WRAP } else { classic_stock_size(facet) }
}

fn file_id(role: usize, facet: usize, variant: bool) -> u32 {
    if variant && facet < VARIANT_FACETS {
        (CLASSIC_FILE_ID_COUNT * CLASSIC_NAMES.len() + role * VARIANT_FACETS + facet) as u32
    } else {
        (role * CLASSIC_NAMES.len() + facet) as u32
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

/// A Classic 7.0-like client: two copies of the 6 stock records, the file name table, the startup globals, the wrap
/// width accessor, the `SetFacet` stores and the two seam compares at x = 5120.
pub fn synthetic_classic() -> Vec<u8> {
    let rdata_rva = next_rva(SECTION_ALIGNMENT, CLASSIC_CODE_SIZE);
    let mut rdata = Vec::new();
    let mut pointers = Vec::new();
    for name in CLASSIC_NAMES {
        pointers.push(IMAGE_BASE + rdata_rva + rdata.len() as u32);
        rdata.extend_from_slice(name.as_bytes());
        rdata.push(0);
    }
    rdata.resize(rdata.len().next_multiple_of(WORD), 0);
    for (index, name) in file_names().iter().enumerate() {
        let mut entry = vec![0u8; FILE_NAME_STRIDE];
        put(&mut entry, 0, FILE_RECORD_SIZE_FIELD);
        entry[FILE_NAME_OFFSET..FILE_NAME_OFFSET + name.len()].copy_from_slice(name.as_bytes());
        put(&mut entry, FILE_NAME_STRIDE - FILE_ORDINAL_FROM_END, index as u32 + 1);
        rdata.extend(entry);
    }
    rdata.extend([0; FILE_NAME_STRIDE]);
    let data_va = IMAGE_BASE + next_rva(rdata_rva, rdata.len());
    let mut data = vec![0u8; TABLES_DATA_OFFSET];
    let (wrap, size) = (classic_stock_wrap(0), classic_stock_size(0));
    for (field, value) in STARTUP_FIELDS.iter().zip([wrap.width, wrap.height, size.width, size.height]) {
        put(&mut data, STARTUP_DATA_OFFSET + field, value);
    }
    for copy in 0..CLASSIC_COPIES {
        for (facet, pointer) in pointers.iter().enumerate() {
            let mut record = vec![0u8; CLASSIC_FACET_RECORD_SIZE];
            let (size, wrap) = (classic_stock_size(facet), classic_stock_wrap(facet));
            put(&mut record, 0, *pointer);
            put(&mut record, FACET_RECORD_WIDTH_OFFSET, size.width);
            put(&mut record, FACET_RECORD_HEIGHT_OFFSET, size.height);
            put(&mut record, CLASSIC_RECORD_WRAP_WIDTH_OFFSET, wrap.width);
            put(&mut record, CLASSIC_RECORD_WRAP_HEIGHT_OFFSET, wrap.height);
            for role in 0..CLASSIC_FILE_ID_COUNT {
                put(&mut record, CLASSIC_RECORD_FILE_IDS_OFFSET + role * WORD, file_id(role, facet, copy > 0));
            }
            put(&mut record, CLASSIC_RECORD_FLAG_OFFSET, u32::from(facet == 0));
            data.extend(record);
        }
        // A zero record between the copies.
        data.extend([0; CLASSIC_FACET_RECORD_SIZE]);
    }
    let table_va = data_va + TABLES_DATA_OFFSET as u32;
    let startup_va = data_va + STARTUP_DATA_OFFSET as u32;

    let mut code = vec![NOP; CLASSIC_CODE_SIZE];
    code[..MOV_EAX_ECX_DISP32.len()].copy_from_slice(&MOV_EAX_ECX_DISP32);
    put(&mut code, MOV_EAX_ECX_DISP32.len(), table_va + CLASSIC_RECORD_WRAP_WIDTH_OFFSET as u32);
    code[MOV_EAX_ECX_DISP32.len() + WORD] = RET;
    let mut at = SET_FACET_AT;
    code[at] = OPCODE_CALL_REL32;
    put(&mut code, at + 1, (-((at + CALL_REL32_SIZE) as i32)) as u32);
    at += CALL_REL32_SIZE;
    code[at] = OPCODE_PUSH_ESI;
    at += 1;
    for field in STARTUP_FIELDS {
        code[at] = OPCODE_STORE_EAX_MOFFS32;
        put(&mut code, at + 1, startup_va + field as u32);
        at += 1 + WORD;
    }
    let first = SEAM_AT[0];
    code[first..first + CMP_EDX_IMM32.len()].copy_from_slice(&CMP_EDX_IMM32);
    put(&mut code, first + CMP_EDX_IMM32.len(), STOCK_SPLIT_X);
    let after = first + CMP_EDX_IMM32.len() + WORD;
    code[after..after + SETGE_AL.len()].copy_from_slice(&SETGE_AL);
    let second = SEAM_AT[1];
    code[second] = CMP_EAX_IMM32;
    put(&mut code, second + 1, STOCK_SPLIT_X);
    let after = second + 1 + WORD;
    code[after..after + MOV_ECX_ESP4_SETGE_CL.len()].copy_from_slice(&MOV_ECX_ESP4_SETGE_CL);

    synthetic_pe(&[(".text", code), (".rdata", rdata), (".data", data)], false)
}
