//! The login server address built into Enhanced Client and Kingdom Reborn executables, located by instruction
//! structure ([`locate_login_sites`]); see `tools/uo-client-patcher-gui/docs/LOGIN_SERVER.md` for the traced code.
//!
//! `LoginManager::LoginAccount` fills a `{u32 ip (host order), u16 port}` pair on the stack at `[ESP+D]`:
//!
//! - the fixed address, `MOV DWORD [ESP+D],ip; MOV WORD [ESP+D+4],port; JMP L` where `L: LEA r32,[ESP+D]` hands the
//!   pair on. In the Enhanced Client it is the internal test address (172.16.17.41:7775, or 10.18.37.29:7775 before
//!   Time of Legends), used only with the client's internal flag set; in Kingdom Reborn it is the only address
//!   (159.153.196.41:7775).
//! - Enhanced Client only, after it: the public host strings and ports. Time of Legends and later draw `rand()`
//!   twice: an odd draw pushes host 1 (`JE +7; PUSH host1; JMP +5; PUSH host2`), and the second draw stores port
//!   7775 when odd, 7776 when even (`MOV [ESP+D],r32; MOV WORD [ESP+D+4],7775; JNE +7; MOV WORD [ESP+D+4],7776`).
//!   Earlier builds push one host and store one port. The host string is resolved by `ResolveHostIPv4`: dotted IPv4
//!   with `sscanf("%u.%u.%u.%u")`, anything else with `gethostbyname`, so a name works too, within the string's slot.
//!   The port immediates are little-endian u16, as every x86 immediate; the pair keeps the port in host order and
//!   swaps it when connecting.
//!
//! A patch ([`crate::client::patch::ExeEdit::LoginServer`], a [`LoginServerEdit`]) writes one host per site
//! and one port per site: in the Enhanced Client every public host slot (zero filled, each host within its own
//! slot) and every port immediate, the internal test address left alone; in Kingdom Reborn the fixed address's
//! imm32 (an IPv4 address only: no string is resolved there) and imm16. [`LoginServerEdit::uniform`] gives every
//! site the same host and port, so every random draw gives the one address.

crate::eyre_imports!();
use std::net::Ipv4Addr;

use crate::client::login_servers::validate_login_host;
use crate::client::facet_dimensions::{code_section, ClientFamily};
use crate::client::pe::PeImage;

/// The largest host slot: ECLO's 16-byte buffer and the spacing of the Time of Legends host strings. A slot is the
/// string, its NUL and the zero padding after it, up to this size.
pub const LOGIN_HOST_SLOT_SIZE: usize = 16;
/// The longest host any slot takes (one byte is the NUL).
pub const MAX_LOGIN_HOST_LENGTH: usize = LOGIN_HOST_SLOT_SIZE - 1;
/// The stock public ports, in draw order (odd, even); builds before Time of Legends store only the first.
pub const STOCK_LOGIN_PORTS: [u16; 2] = [7775, 7776];
/// The official login hosts of the traced builds: UOSA 4.0.102.3 and 4.0.29.97 (107.23.x), UOSA 4.0.115.0
/// (13.248.219.129, 75.2.70.184).
pub const STOCK_LOGIN_HOSTS: [&str; 4] = ["107.23.85.115", "107.23.176.74", "13.248.219.129", "75.2.70.184"];
/// How far after the fixed address the public hosts and ports are searched (the traced builds: 0x61..0x9b bytes).
pub const LOGIN_SEARCH_WINDOW: usize = 0x200;
/// The Enhanced Client's internal test addresses (the fixed store) of the traced builds: Time of Legends and later,
/// then 4.0.29.97 (TC).
pub const STOCK_INTERNAL_LOGIN_ADDRESSES: [(Ipv4Addr, u16); 2] =
    [(Ipv4Addr::new(172, 16, 17, 41), STOCK_LOGIN_PORTS[0]), (Ipv4Addr::new(10, 18, 37, 29), STOCK_LOGIN_PORTS[0])];
/// Kingdom Reborn's only address (the fixed store) in the traced build.
pub const STOCK_KR_LOGIN_ADDRESS: (Ipv4Addr, u16) = (Ipv4Addr::new(159, 153, 196, 41), STOCK_LOGIN_PORTS[0]);
/// Both built-in addresses found in the audited KR collection.
pub const STOCK_KR_LOGIN_ADDRESSES: [(Ipv4Addr, u16); 2] = [
    STOCK_KR_LOGIN_ADDRESS, (Ipv4Addr::new(159, 153, 196, 57), STOCK_LOGIN_PORTS[0]),
];

const OPCODE_MOV_RM32_IMM32: u8 = 0xc7;
const OPCODE_MOV_RM32_R32: u8 = 0x89;
const OPCODE_LEA: u8 = 0x8d;
const OPCODE_PUSH_IMM32: u8 = 0x68;
const OPCODE_JMP_REL32: u8 = 0xe9;
const OPCODE_JMP_REL8: u8 = 0xeb;
const OPCODE_JE_REL8: u8 = 0x74;
const OPCODE_JNE_REL8: u8 = 0x75;
const OPERAND_SIZE_PREFIX: u8 = 0x66;
/// ModRM `mod = 01` (disp8), `rm = 100` (SIB), reg 0: `[SIB + disp8]` with `/0`.
const MODRM_SIB_DISP8: u8 = 0x44;
/// The mod and rm fields of a ModRM byte.
const MODRM_MOD_RM_MASK: u8 = 0xc7;
/// SIB base ESP, no index.
const SIB_ESP: u8 = 0x24;
/// `MOV WORD [ESP+disp8],imm16`: 66 C7 44 24 disp8 imm16.
const MOV_WORD_ESP_LEN: usize = 7;
/// `MOV DWORD [ESP+disp8],imm32`: C7 44 24 disp8 imm32.
const MOV_DWORD_ESP_LEN: usize = 8;
/// `MOV [ESP+disp8],r32`: 89 modrm 24 disp8.
const MOV_ESP_R32_LEN: usize = 4;
const PUSH_IMM32_LEN: usize = 5;
const JMP_REL32_LEN: usize = 5;
const JCC_REL8_LEN: usize = 2;
/// The port follows the ip in the pair.
const PAIR_PORT_OFFSET: u8 = size_of::<u32>() as u8;
/// The longest host string read before giving up.
const MAX_HOST_SCAN: usize = 64;
const INLINE_KR_PREFIX: &[u8] = &[0x8d, 0x44, 0x24];
const INLINE_KR_AFTER_LEA: &[u8] = &[0x50, 0x8b, 0xc6];
const INLINE_KR_PREFIX_LEN: usize = INLINE_KR_PREFIX.len() + 1 + INLINE_KR_AFTER_LEA.len();
const INLINE_KR_TAIL: &[u8] = &[0x5f, 0x5e, 0x83, 0xc4, 0x08, 0xc2, 0x04, 0x00];
const OPCODE_CALL_REL32: u8 = 0xe8;
const CALL_REL32_LEN: usize = 5;
/// The 2.45 octet packer and active endpoint constructor. Only the endpoint operands vary.
const LEGACY_ADDRESS_CODE: &[u8] = &[
    0x33, 0xc0, 0x8a, 0x64, 0x24, 0x08, 0x0f, 0xb6, 0xc9, 0x8a, 0x44, 0x24, 0x0c, 0xc1, 0xe0, 0x08,
    0x0b, 0xc1, 0x0f, 0xb6, 0x4c, 0x24, 0x10, 0xc1, 0xe0, 0x08, 0x0b, 0xc1, 0x8b, 0x4c, 0x24, 0x04,
    0x89, 0x01, 0xc2, 0x10, 0x00,
    0x6a, 0x39, 0x68, 0x99, 0x00, 0x00, 0x00, 0x68, 0x9f, 0x00, 0x00, 0x00, 0x56, 0xb1, 0xc4, 0xe8,
    0xc7, 0xff, 0xff, 0xff, 0x66, 0xc7, 0x46, 0x04, 0x5f, 0x1e, 0xc3,
];
const LEGACY_PACKER_SIZE: usize = 37;
const LEGACY_IP_OCTET_OFFSETS: [usize; 4] = [8, 3, 14, 1];
const LEGACY_PORT_OFFSET: usize = 24;

/// The fixed address store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginAddressSite {
    /// Start of endpoint construction: a packed store or the earlier octet constructor.
    pub offset: usize,
    /// File offsets of the packed IP immediate (or first octet) and the port imm16.
    pub ip_offset: usize,
    pub port_offset: usize,
    /// Earlier KR stores four separate octet operands, in network order; otherwise the IP is one imm32.
    pub ip_byte_offsets: Option<[usize; 4]>,
    pub ip: Ipv4Addr,
    pub port: u16,
    /// The packed pair's stack displacement D; unused by the earlier octet constructor.
    pub displacement: u8,
}

impl LoginAddressSite {
    /// Exact operand writes for the address, preserving every instruction and immediate's unused bytes.
    pub fn ip_writes(&self, ip: Ipv4Addr) -> Vec<(usize, Vec<u8>)> {
        match self.ip_byte_offsets {
            Some(offsets) => offsets.into_iter().zip(ip.octets()).map(|(at, octet)| (at, vec![octet])).collect(),
            None => vec![(self.ip_offset, u32::from(ip).to_le_bytes().to_vec())],
        }
    }

    /// The active endpoint constructor's port operand.
    pub fn port_writes(&self, port: u16) -> Vec<(usize, Vec<u8>)> {
        vec![(self.port_offset, port.to_le_bytes().to_vec())]
    }
}

/// A public host string the code pushes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginHostSlot {
    /// File offset of the `PUSH imm32`.
    pub push_offset: usize,
    pub va: u32,
    /// File offset of the string.
    pub offset: usize,
    /// The string, its NUL and the zero bytes after it, at most [`LOGIN_HOST_SLOT_SIZE`].
    pub capacity: usize,
    pub host: String,
    /// How many times the image holds the slot's address as a u32: 1 when only the push uses it.
    pub references: usize,
}

/// A public port immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginPortSite {
    /// File offset of the imm16.
    pub offset: usize,
    pub port: u16,
}

/// The login address code of one build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSites {
    pub fixed: LoginAddressSite,
    /// Enhanced Client: the public hosts, in draw order (odd first); empty in Kingdom Reborn.
    pub hosts: Vec<LoginHostSlot>,
    /// Enhanced Client: the public ports, in draw order; empty in Kingdom Reborn.
    pub ports: Vec<LoginPortSite>,
}

/// The login values a login edit writes, as a manifest records them: the Enhanced Client's public hosts and ports,
/// or Kingdom Reborn's fixed address (one host, one port).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LoginValues {
    pub hosts: Vec<String>,
    pub ports: Vec<u16>,
}

impl LoginSites {
    /// True when the client connects to the fixed address (Kingdom Reborn: no public hosts), so a login edit writes
    /// it; the Enhanced Client's fixed address is the internal test one, never written.
    pub fn uses_fixed_address(&self) -> bool {
        self.hosts.is_empty()
    }

    /// The host sites a login edit writes: the public slots, or the one fixed address.
    pub fn host_site_count(&self) -> usize {
        if self.uses_fixed_address() { 1 } else { self.hosts.len() }
    }

    /// The port sites a login edit writes: the public port immediates, or the fixed address's.
    pub fn port_site_count(&self) -> usize {
        if self.uses_fixed_address() { 1 } else { self.ports.len() }
    }

    pub fn values(&self) -> LoginValues {
        if self.uses_fixed_address() {
            return LoginValues { hosts: vec![self.fixed.ip.to_string()], ports: vec![self.fixed.port] };
        }
        LoginValues {
            hosts: self.hosts.iter().map(|slot| slot.host.clone()).collect(),
            ports: self.ports.iter().map(|site| site.port).collect(),
        }
    }

    /// Enhanced Client: true when every public host is an official one and every port is the stock one of its
    /// draw. Kingdom Reborn: true at one of the audited [`STOCK_KR_LOGIN_ADDRESSES`].
    pub fn is_stock(&self) -> bool {
        if self.uses_fixed_address() {
            return STOCK_KR_LOGIN_ADDRESSES.contains(&(self.fixed.ip, self.fixed.port));
        }
        self.hosts.iter().all(|slot| STOCK_LOGIN_HOSTS.contains(&slot.host.as_str()))
            && self.ports.iter().zip(STOCK_LOGIN_PORTS).all(|(site, stock)| site.port == stock)
    }

    /// The Enhanced Client's public servers the client draws from: host `i` with port `i` (or every host with the
    /// one port). Empty in Kingdom Reborn (its address is [`Self::fixed`]).
    pub fn public_servers(&self) -> Vec<(String, u16)> {
        let count = self.hosts.len().max(self.ports.len());
        (0..count)
            .filter_map(|index| {
                let host = self.hosts.get(index).or(self.hosts.last())?;
                let port = self.ports.get(index).or(self.ports.last())?;
                Some((host.host.clone(), port.port))
            })
            .collect()
    }

    /// The smallest host slot, which bounds the host a patch writes.
    pub fn host_capacity(&self) -> Option<usize> {
        self.hosts.iter().map(|slot| slot.capacity).min()
    }

    /// The file offsets of the sites, to check a patched copy locates them at the same places.
    pub fn offsets(&self) -> Vec<usize> {
        let mut offsets = vec![self.fixed.offset];
        offsets.extend(self.hosts.iter().map(|slot| slot.offset));
        offsets.extend(self.ports.iter().map(|site| site.offset));
        offsets
    }
}

/// A host the patch writes: at most [`MAX_LOGIN_HOST_LENGTH`] bytes of a validated dotted IPv4 address or host
/// name, stored inline. Serialized as its string.
#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LoginHost {
    bytes: [u8; MAX_LOGIN_HOST_LENGTH],
    len: u8,
}

impl LoginHost {
    pub fn new(host: &str) -> eyre::Result<Self> {
        validate_login_host(host)?;
        eyre::ensure!(
            host.len() <= MAX_LOGIN_HOST_LENGTH,
            "host `{host}`: {} characters, the executable's host slots take at most {MAX_LOGIN_HOST_LENGTH}",
            host.len()
        );
        let mut bytes = [0; MAX_LOGIN_HOST_LENGTH];
        bytes[..host.len()].copy_from_slice(host.as_bytes());
        Ok(Self { bytes, len: host.len() as u8 })
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).expect("validated ASCII")
    }
}

impl std::fmt::Debug for LoginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl std::fmt::Display for LoginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for LoginHost {
    type Error = eyre::Report;

    fn try_from(host: String) -> eyre::Result<Self> {
        Self::new(&host)
    }
}

impl From<LoginHost> for String {
    fn from(host: LoginHost) -> Self {
        host.as_str().to_owned()
    }
}

/// A login server edit: one host per host site and one port per port site of the build, in the order of
/// [`LoginSites::hosts`] and [`LoginSites::ports`] (Kingdom Reborn: one each, the fixed address). The patch refuses
/// counts that differ from the located sites.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(from = "LoginServerEditRecord")]
pub struct LoginServerEdit {
    pub hosts: Vec<LoginHost>,
    pub ports: Vec<u16>,
}

/// What a manifest step holds: the per-site form, or the one host and port of a manifest written before per-site
/// edits (read as one host and one port).
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum LoginServerEditRecord {
    PerSite { hosts: Vec<LoginHost>, ports: Vec<u16> },
    Single { host: LoginHost, port: u16 },
}

impl From<LoginServerEditRecord> for LoginServerEdit {
    fn from(record: LoginServerEditRecord) -> Self {
        match record {
            LoginServerEditRecord::PerSite { hosts, ports } => Self { hosts, ports },
            LoginServerEditRecord::Single { host, port } => Self { hosts: vec![host], ports: vec![port] },
        }
    }
}

impl LoginServerEdit {
    /// `host` on every host site and `port` on every port site of `sites`.
    pub fn uniform(sites: &LoginSites, host: LoginHost, port: u16) -> Self {
        Self { hosts: vec![host; sites.host_site_count()], ports: vec![port; sites.port_site_count()] }
    }

    pub fn describe(&self) -> String {
        let uniform_host = self.hosts.first().filter(|first| self.hosts.iter().all(|host| host == *first));
        let uniform_port = self.ports.first().filter(|first| self.ports.iter().all(|port| port == *first));
        if let (Some(host), Some(port)) = (uniform_host, uniform_port) {
            return format!("login server {host}:{port}");
        }
        let numbered = |items: Vec<String>| {
            items.iter().enumerate().map(|(index, item)| format!("{} {item}", index + 1)).collect::<Vec<_>>().join(", ")
        };
        format!(
            "login hosts {}; ports {}",
            numbered(self.hosts.iter().map(LoginHost::to_string).collect()),
            numbered(self.ports.iter().map(u16::to_string).collect())
        )
    }
}

fn u16_at(code: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(code.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(code: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(code.get(at..at + 4)?.try_into().ok()?))
}

/// `MOV WORD [ESP+disp],imm16` at `at`: the imm16 position.
fn mov_word_esp(code: &[u8], at: usize, disp: u8) -> Option<usize> {
    (code.get(at..at + 5)? == [OPERAND_SIZE_PREFIX, OPCODE_MOV_RM32_IMM32, MODRM_SIB_DISP8, SIB_ESP, disp]).then_some(at + 5)
}

/// `LEA r32,[ESP+disp]` at `at`.
fn lea_esp(code: &[u8], at: usize, disp: u8) -> bool {
    code.get(at..at + 4).is_some_and(|bytes| {
        bytes[0] == OPCODE_LEA && bytes[1] & MODRM_MOD_RM_MASK == MODRM_SIB_DISP8 && bytes[2] == SIB_ESP && bytes[3] == disp
    })
}

/// The fixed address store at code index `at`: `(D, ip imm index, port imm index)`.
fn match_fixed(code: &[u8], at: usize) -> Option<(u8, usize, usize)> {
    if code.get(at..at + 3)? != [OPCODE_MOV_RM32_IMM32, MODRM_SIB_DISP8, SIB_ESP] {
        return None;
    }
    let disp = code[at + 3];
    let port = mov_word_esp(code, at + MOV_DWORD_ESP_LEN, disp.checked_add(PAIR_PORT_OFFSET)?)?;
    let jump = at + MOV_DWORD_ESP_LEN + MOV_WORD_ESP_LEN;
    let target = match *code.get(jump)? {
        OPCODE_JMP_REL32 => (jump + JMP_REL32_LEN).checked_add_signed(i32::from_le_bytes(code.get(jump + 1..jump + 5)?.try_into().ok()?) as isize)?,
        OPCODE_JMP_REL8 => (jump + JCC_REL8_LEN).checked_add_signed(*code.get(jump + 1)? as i8 as isize)?,
        _ => return None,
    };
    lea_esp(code, target, disp).then_some((disp, at + 4, port))
}

/// Code indices of every fixed address store (`MOV DWORD [ESP+D],ip; MOV WORD [ESP+D+4],port; JMP` to
/// `LEA r32,[ESP+D]`). The traced builds have one.
pub fn locate_login_fixed_candidates(bytes: &[u8], pe: &PeImage) -> eyre::Result<Vec<LoginAddressSite>> {
    let (code, base) = code_section(bytes, pe)?;
    Ok((0..code.len())
        .filter_map(|at| {
            let (displacement, ip, port) = match_fixed(code, at)?;
            Some(LoginAddressSite {
                offset: base + at,
                ip_offset: base + ip,
                port_offset: base + port,
                ip_byte_offsets: None,
                ip: Ipv4Addr::from(u32_at(code, ip)?),
                port: u16_at(code, port)?,
                displacement,
            })
        })
        .collect())
}

fn earlier_kr_candidates(code: &[u8], base: usize) -> Vec<LoginAddressSite> {
    let mut found = Vec::new();
    for at in 0..code.len() {
        // 2.46: the pair is built immediately before the direct connect call, after its pointer is pushed.
        if at >= INLINE_KR_PREFIX_LEN && code.get(at..at + 3) == Some(&[OPCODE_MOV_RM32_IMM32, MODRM_SIB_DISP8, SIB_ESP])
            && let Some(&disp) = code.get(at + 3)
        {
            let prefix = at - INLINE_KR_PREFIX_LEN;
            let port = disp.checked_add(PAIR_PORT_OFFSET).and_then(|port_disp| mov_word_esp(code, at + MOV_DWORD_ESP_LEN, port_disp));
            let call = at + MOV_DWORD_ESP_LEN + MOV_WORD_ESP_LEN;
            let tail = call + CALL_REL32_LEN;
            let prefix_matches = code.get(prefix..prefix + INLINE_KR_PREFIX.len()) == Some(INLINE_KR_PREFIX)
                && code.get(prefix + INLINE_KR_PREFIX.len()) == disp.checked_sub(PAIR_PORT_OFFSET).as_ref()
                && code.get(prefix + INLINE_KR_PREFIX.len() + 1..at) == Some(INLINE_KR_AFTER_LEA);
            if prefix_matches && code.get(call) == Some(&OPCODE_CALL_REL32) && code.get(tail..tail + INLINE_KR_TAIL.len()) == Some(INLINE_KR_TAIL)
                && let Some(port) = port
                && let (Some(ip), Some(port_value)) = (u32_at(code, at + 4), u16_at(code, port))
            {
                found.push(LoginAddressSite {
                    offset: base + at, ip_offset: base + at + 4, port_offset: base + port,
                    ip_byte_offsets: None,
                    ip: Ipv4Addr::from(ip), port: port_value, displacement: disp,
                });
            }
        }
        // 2.45: verify the complete octet packer, its exact backward call, and the active constructor.
        let Some(window) = code.get(at..at + LEGACY_ADDRESS_CODE.len()) else { continue };
        let operands = |index: usize| {
            index.checked_sub(LEGACY_PACKER_SIZE).is_some_and(|index| {
                LEGACY_IP_OCTET_OFFSETS.contains(&index)
                    || (LEGACY_PORT_OFFSET..LEGACY_PORT_OFFSET + size_of::<u16>()).contains(&index)
            })
        };
        if !window.iter().zip(LEGACY_ADDRESS_CODE).enumerate().all(|(index, (actual, expected))| operands(index) || actual == expected) {
            continue;
        }
        let ctor = at + LEGACY_PACKER_SIZE;
        let port = u16_at(code, ctor + LEGACY_PORT_OFFSET).expect("matched constructor");
        let offsets = LEGACY_IP_OCTET_OFFSETS.map(|at| base + ctor + at);
        found.push(LoginAddressSite {
            offset: base + ctor, ip_offset: offsets[0], port_offset: base + ctor + LEGACY_PORT_OFFSET,
            ip_byte_offsets: Some(offsets),
            ip: Ipv4Addr::from(LEGACY_IP_OCTET_OFFSETS.map(|at| code[ctor + at])), port, displacement: 0,
        });
    }
    found
}

/// The host string at `va`: in a data section, 1..=[`MAX_LOGIN_HOST_LENGTH`] host characters and a NUL.
fn host_slot_at(bytes: &[u8], pe: &PeImage, va: u32) -> Option<(usize, String, usize)> {
    if !pe.section_of_va(va)?.is_data() {
        return None;
    }
    let offset = pe.va_to_offset(va)?;
    let window = bytes.get(offset..(offset + MAX_HOST_SCAN).min(bytes.len()))?;
    let len = window.iter().position(|byte| *byte == 0)?;
    let host = std::str::from_utf8(&window[..len]).ok()?;
    if len > MAX_LOGIN_HOST_LENGTH || validate_login_host(host).is_err() {
        return None;
    }
    let capacity = len + window[len..].iter().take(LOGIN_HOST_SLOT_SIZE - len).take_while(|byte| **byte == 0).count();
    Some((offset, host.to_owned(), capacity))
}

fn count_references(bytes: &[u8], va: u32) -> usize {
    let needle = va.to_le_bytes();
    bytes.windows(needle.len()).filter(|window| *window == needle).count()
}

/// The public port stores after code index `from`: `MOV [ESP+D],r32; MOV WORD [ESP+D+4],port` and, Time of Legends
/// on, `JNE +7; MOV WORD [ESP+D+4],port2`. Returns the index of the first store and the imm16 indices.
fn locate_ports(code: &[u8], from: usize, disp: u8) -> Option<(usize, Vec<usize>)> {
    let port_disp = disp.checked_add(PAIR_PORT_OFFSET)?;
    let end = (from + LOGIN_SEARCH_WINDOW).min(code.len());
    (from..end).find_map(|at| {
        let store = code.get(at..at + MOV_ESP_R32_LEN)?;
        if store[0] != OPCODE_MOV_RM32_R32 || store[1] & MODRM_MOD_RM_MASK != MODRM_SIB_DISP8 || store[2] != SIB_ESP || store[3] != disp {
            return None;
        }
        let first = mov_word_esp(code, at + MOV_ESP_R32_LEN, port_disp)?;
        let mut ports = vec![first];
        let branch = at + MOV_ESP_R32_LEN + MOV_WORD_ESP_LEN;
        if code.get(branch..branch + JCC_REL8_LEN) == Some(&[OPCODE_JNE_REL8, MOV_WORD_ESP_LEN as u8])
            && let Some(second) = mov_word_esp(code, branch + JCC_REL8_LEN, port_disp)
        {
            ports.push(second);
        }
        Some((at, ports))
    })
}

/// The login address code of an Enhanced Client or Kingdom Reborn build (`None` for Classic, or when the fixed
/// address is not found exactly once, or an Enhanced Client build lacks its public hosts or ports after it).
pub fn locate_login_sites(bytes: &[u8], pe: &PeImage, family: ClientFamily) -> eyre::Result<Option<LoginSites>> {
    if family == ClientFamily::Classic {
        return Ok(None);
    }
    let mut candidates = locate_login_fixed_candidates(bytes, pe)?;
    if family == ClientFamily::KingdomReborn {
        let (code, base) = code_section(bytes, pe)?;
        candidates.extend(earlier_kr_candidates(code, base));
    }
    if candidates.len() != 1 {
        return Ok(None);
    }
    let fixed = candidates.pop().expect("one candidate");
    if family == ClientFamily::KingdomReborn {
        return Ok(Some(LoginSites { fixed, hosts: Vec::new(), ports: Vec::new() }));
    }
    let (code, base) = code_section(bytes, pe)?;
    let after = fixed.port_offset + size_of::<u16>() - base;
    let Some((port_store, port_imms)) = locate_ports(code, after, fixed.displacement) else {
        return Ok(None);
    };
    let ports = port_imms
        .into_iter()
        .map(|imm| LoginPortSite { offset: base + imm, port: u16_at(code, imm).expect("matched imm16") })
        .collect();
    let mut hosts = Vec::new();
    for at in after..port_store.saturating_sub(PUSH_IMM32_LEN - 1) {
        if code[at] != OPCODE_PUSH_IMM32 {
            continue;
        }
        let Some(va) = u32_at(code, at + 1) else { continue };
        if let Some((offset, host, capacity)) = host_slot_at(bytes, pe, va) {
            hosts.push(LoginHostSlot { push_offset: base + at, va, offset, capacity, host, references: count_references(bytes, va) });
        }
    }
    // Time of Legends on: the two pushes are the arms of `JE +7; PUSH h1; JMP +5; PUSH h2`.
    let two_arms = hosts.len() == 2 && {
        let first = hosts[0].push_offset - base;
        code.get(first - JCC_REL8_LEN..first) == Some(&[OPCODE_JE_REL8, (PUSH_IMM32_LEN + JCC_REL8_LEN) as u8])
            && code.get(first + PUSH_IMM32_LEN..first + PUSH_IMM32_LEN + JCC_REL8_LEN) == Some(&[OPCODE_JMP_REL8, PUSH_IMM32_LEN as u8])
            && hosts[1].push_offset == hosts[0].push_offset + PUSH_IMM32_LEN + JCC_REL8_LEN
    };
    if !(hosts.len() == 1 || two_arms) {
        return Ok(None);
    }
    Ok(Some(LoginSites { fixed, hosts, ports }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::client::facet_dimensions::tests::{synthetic_image_with, synthetic_rdata_va};
    use crate::client::facet_dimensions::ClientExeInspection;

    /// The Time of Legends hosts of UOSA 4.0.102.3, 16 bytes apart, then 4 zero bytes and other data.
    pub const TOL_HOSTS: [&str; 2] = ["107.23.85.115", "107.23.176.74"];
    pub const INTERNAL_IP: Ipv4Addr = STOCK_INTERNAL_LOGIN_ADDRESSES[0].0;
    const KR_IP: Ipv4Addr = STOCK_KR_LOGIN_ADDRESS.0;
    const PAIR_DISPLACEMENT: u8 = 0x14;
    /// After the last host: a non-zero byte ends the zero padding of the second slot.
    const RDATA_TAIL: &[u8] = b"\0\0\0\0LoginManager";
    /// A second host slot of 12 bytes (UOSA 4.0.115.0's 75.2.70.184).
    pub const SHORT_SLOT: &str = "75.2.70.184";

    /// The `.rdata` prefix with the host strings: each in a 16-byte slot, the last followed by [`RDATA_TAIL`].
    fn host_rdata(hosts: &[&str], short_last: bool) -> (Vec<u8>, Vec<usize>) {
        let mut rdata = Vec::new();
        let mut offsets = Vec::new();
        for (index, host) in hosts.iter().enumerate() {
            offsets.push(rdata.len());
            rdata.extend_from_slice(host.as_bytes());
            let slot = if short_last && index + 1 == hosts.len() { host.len() + 1 } else { LOGIN_HOST_SLOT_SIZE };
            rdata.resize(offsets[index] + slot, 0);
        }
        rdata.extend_from_slice(if short_last { &RDATA_TAIL[4..] } else { RDATA_TAIL });
        (rdata, offsets)
    }

    /// `LoginAccount`'s address code as UOSA 4.0.102.3 has it (`tol`) or as 4.0.29.97 has it, with the hosts at
    /// `host_vas`. Returns the code and the index of the fixed store.
    fn login_code(host_vas: &[u32], fixed: (Ipv4Addr, u16), ports: &[u16]) -> (Vec<u8>, usize) {
        let d = PAIR_DISPLACEMENT;
        let mut code = vec![0x90; 16];
        let fixed_at = code.len();
        code.extend([OPCODE_MOV_RM32_IMM32, MODRM_SIB_DISP8, SIB_ESP, d]);
        code.extend(u32::from(fixed.0).to_le_bytes());
        code.extend([OPERAND_SIZE_PREFIX, OPCODE_MOV_RM32_IMM32, MODRM_SIB_DISP8, SIB_ESP, d + 4]);
        code.extend(fixed.1.to_le_bytes());
        let jump = code.len();
        code.extend([OPCODE_JMP_REL32, 0, 0, 0, 0]);
        // rand; AND EAX,0x80000001; ...; LEA ECX,[ESP+1C]
        code.extend([0xe8, 0, 0, 0, 0, 0x25, 0x01, 0, 0, 0x80, 0x8d, 0x4c, 0x24, 0x1c]);
        match host_vas {
            [one] => {
                code.push(OPCODE_PUSH_IMM32);
                code.extend(one.to_le_bytes());
            }
            [first, second] => {
                code.extend([OPCODE_JE_REL8, 7, OPCODE_PUSH_IMM32]);
                code.extend(first.to_le_bytes());
                code.extend([OPCODE_JMP_REL8, 5, OPCODE_PUSH_IMM32]);
                code.extend(second.to_le_bytes());
            }
            _ => {}
        }
        // call ResolveHostIPv4; MOV EDI,EAX
        code.extend([0xe8, 0, 0, 0, 0, 0x8b, 0xf8]);
        code.extend([OPCODE_MOV_RM32_R32, 0x7c, SIB_ESP, d, OPERAND_SIZE_PREFIX, OPCODE_MOV_RM32_IMM32, MODRM_SIB_DISP8, SIB_ESP, d + 4]);
        code.extend(ports[0].to_le_bytes());
        if let Some(second) = ports.get(1) {
            code.extend([OPCODE_JNE_REL8, MOV_WORD_ESP_LEN as u8, OPERAND_SIZE_PREFIX, OPCODE_MOV_RM32_IMM32, MODRM_SIB_DISP8, SIB_ESP, d + 4]);
            code.extend(second.to_le_bytes());
        }
        let target = code.len();
        code.extend([OPCODE_LEA, 0x54, SIB_ESP, d, 0x52]);
        let rel = (target - (jump + JMP_REL32_LEN)) as i32;
        code[jump + 1..jump + 5].copy_from_slice(&rel.to_le_bytes());
        code.extend([0xcc; 16]);
        (code, fixed_at)
    }

    /// An Enhanced Client image with the Time of Legends login code (or the earlier one-host code when `!tol`); the
    /// second slot is 12 bytes when `short_slot`.
    pub fn synthetic_login_client(tol: bool, short_slot: bool) -> Vec<u8> {
        let hosts: Vec<&str> = match (tol, short_slot) {
            (true, false) => TOL_HOSTS.to_vec(),
            (true, true) => vec![TOL_HOSTS[0], SHORT_SLOT],
            (false, _) => vec![TOL_HOSTS[0]],
        };
        let (rdata, offsets) = host_rdata(&hosts, short_slot);
        let ports: &[u16] = if tol { &STOCK_LOGIN_PORTS } else { &STOCK_LOGIN_PORTS[..1] };
        let text_len = login_code(&[0; 2], (INTERNAL_IP, STOCK_LOGIN_PORTS[0]), ports).0.len();
        let rdata_va = synthetic_rdata_va(text_len);
        let vas: Vec<u32> = offsets.iter().map(|offset| rdata_va + *offset as u32).collect();
        let (code, _) = login_code(&vas, (INTERNAL_IP, STOCK_LOGIN_PORTS[0]), ports);
        synthetic_image_with(code, rdata, true)
    }

    pub fn synthetic_kr_login_client() -> Vec<u8> {
        let (code, _) = login_code(&[], (KR_IP, STOCK_LOGIN_PORTS[0]), &[STOCK_LOGIN_PORTS[0]]);
        synthetic_image_with(code, Vec::new(), false)
    }

    pub fn synthetic_kr_legacy_login_client() -> Vec<u8> {
        synthetic_image_with(LEGACY_ADDRESS_CODE.to_vec(), Vec::new(), false)
    }

    #[test]
    fn earlier_kr_octet_address_is_guarded_and_relocatable() {
        let image = synthetic_kr_legacy_login_client();
        let before = ClientExeInspection::inspect(&image).unwrap();
        let login = before.login.unwrap();
        assert!(login.is_stock());
        assert_eq!(login.fixed.ip, STOCK_KR_LOGIN_ADDRESSES[1].0);
        let ip = Ipv4Addr::new(255, 128, 254, 192);
        const PORT: u16 = 2593;
        let mut patched = image.clone();
        for (at, bytes) in login.fixed.ip_writes(ip).into_iter().chain(login.fixed.port_writes(PORT)) {
            patched[at..][..bytes.len()].copy_from_slice(&bytes);
        }
        let after = ClientExeInspection::inspect(&patched).unwrap().login.unwrap();
        assert_eq!((after.fixed.ip, after.fixed.port), (ip, PORT));
        assert_eq!(after.offsets(), login.offsets());
        assert_eq!(after.fixed.ip_writes(ip).len(), ip.octets().len());
        assert_eq!(after.fixed.port_writes(PORT).len(), 1);
        let mut bad_packer = image;
        let (code, base) = code_section(&bad_packer, &before.pe).unwrap();
        assert_eq!(code[0], LEGACY_ADDRESS_CODE[0]);
        bad_packer[base] ^= 1;
        assert!(ClientExeInspection::inspect(&bad_packer).unwrap().login.is_none());
        let mut twice = LEGACY_ADDRESS_CODE.to_vec();
        twice.extend(LEGACY_ADDRESS_CODE);
        assert!(ClientExeInspection::inspect(&synthetic_image_with(twice, Vec::new(), false)).unwrap().login.is_none());
    }

    #[test]
    fn earlier_kr_inline_pair_requires_its_connect_context() {
        const IP: Ipv4Addr = Ipv4Addr::new(159, 153, 196, 57);
        const DISP: u8 = 0x0c;
        let mut code = vec![0x8d, 0x44, 0x24, DISP - PAIR_PORT_OFFSET, 0x50, 0x8b, 0xc6, 0xc7, 0x44, 0x24, DISP];
        code.extend(u32::from(IP).to_le_bytes());
        code.extend([0x66, 0xc7, 0x44, 0x24, DISP + PAIR_PORT_OFFSET]);
        code.extend(STOCK_LOGIN_PORTS[0].to_le_bytes());
        code.extend([OPCODE_CALL_REL32, 0, 0, 0, 0]);
        code.extend(INLINE_KR_TAIL);
        let image = synthetic_image_with(code.clone(), Vec::new(), false);
        let fixed = ClientExeInspection::inspect(&image).unwrap().login.unwrap().fixed;
        assert_eq!((fixed.ip, fixed.port), (IP, STOCK_LOGIN_PORTS[0]));
        assert!(fixed.ip_byte_offsets.is_none());
        code[0] ^= 1;
        assert!(ClientExeInspection::inspect(&synthetic_image_with(code, Vec::new(), false)).unwrap().login.is_none());
        for length in 0..INLINE_KR_PREFIX_LEN + MOV_DWORD_ESP_LEN {
            let truncated = vec![OPCODE_MOV_RM32_IMM32; length];
            assert!(earlier_kr_candidates(&truncated, 0).is_empty());
        }
    }

    #[test]
    fn the_login_sites_are_located() {
        let tol = ClientExeInspection::inspect(&synthetic_login_client(true, false)).unwrap();
        let login = tol.login.as_ref().unwrap();
        assert_eq!((login.fixed.ip, login.fixed.port), (INTERNAL_IP, STOCK_LOGIN_PORTS[0]));
        assert_eq!(login.values(), LoginValues { hosts: TOL_HOSTS.map(str::to_owned).to_vec(), ports: STOCK_LOGIN_PORTS.to_vec() });
        assert!(login.hosts.iter().all(|slot| slot.capacity == LOGIN_HOST_SLOT_SIZE && slot.references == 1));
        assert!(login.is_stock());
        assert_eq!(login.public_servers(), [(TOL_HOSTS[0].to_owned(), 7775), (TOL_HOSTS[1].to_owned(), 7776)]);

        let old = ClientExeInspection::inspect(&synthetic_login_client(false, false)).unwrap();
        let login = old.login.unwrap();
        assert_eq!((login.hosts.len(), login.ports.len()), (1, 1));

        let short = ClientExeInspection::inspect(&synthetic_login_client(true, true)).unwrap();
        let slots: Vec<usize> = short.login.unwrap().hosts.iter().map(|slot| slot.capacity).collect();
        assert_eq!(slots, [LOGIN_HOST_SLOT_SIZE, SHORT_SLOT.len() + 1]);

        let kr = ClientExeInspection::inspect(&synthetic_kr_login_client()).unwrap();
        assert_eq!(kr.family, ClientFamily::KingdomReborn);
        let login = kr.login.unwrap();
        assert_eq!(login.fixed.ip, KR_IP);
        assert!(login.hosts.is_empty() && login.ports.is_empty());

        // Two fixed stores: not located.
        let (mut code, _) = login_code(&[], (KR_IP, STOCK_LOGIN_PORTS[0]), &[STOCK_LOGIN_PORTS[0]]);
        code.extend(code.clone());
        let image = synthetic_image_with(code, Vec::new(), false);
        assert!(ClientExeInspection::inspect(&image).unwrap().login.is_none());
    }

    #[test]
    fn hosts_are_copy_values_with_a_string_form() {
        let host = LoginHost::new("127.0.0.1").unwrap();
        assert_eq!(host.as_str(), "127.0.0.1");
        assert_eq!(serde_json::to_string(&host).unwrap(), "\"127.0.0.1\"");
        assert_eq!(serde_json::from_str::<LoginHost>("\"login.owo.com\"").unwrap().as_str(), "login.owo.com");
        assert!(serde_json::from_str::<LoginHost>("\"bad host\"").is_err());
        assert!(LoginHost::new("a-very-long.example.com").is_err(), "longer than a slot");
        assert!(LoginHost::new("1.2.3.999").is_err());
    }
}
