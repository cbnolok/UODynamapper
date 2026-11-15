//! The network encryption of early Classic, Enhanced Client and Kingdom Reborn executables, located by instruction pattern
//! ([`locate_encryption_sites`]) so a patch ([`crate::client::patch::ExeEdit::NoEncryption`]) can make the client
//! talk plaintext. See `tools/uo-client-patcher-gui/docs/ENCRYPTION.md` and the traced code in
//! `tools/uo-client-patcher-gui/docs/research/uokr_login.md`.
//!
//! The client sends and receives through a chain of transforms per connection. The ones this patch disables:
//!
//! - [`CipherSiteKind::SharedLoginGameCipher`]: early Classic clients' one client-to-server rotation cipher,
//!   used by both login and game connections. The two-byte XOR becomes NOPs while mask stepping stays active.
//!   See `tools/uo-client-patcher-gui/docs/research/classic-rotation-cipher.md` for the verified layouts.
//! - [`CipherSiteKind::LoginCipher`]: the login connection's client-to-server cipher (`UOLoginCipher`, the seed's
//!   LFSR keys), installed right after the login seed. Its transform XORs each byte with the low key byte, then steps
//!   the keys; the patch replaces the `XOR [EAX],CL` with two NOPs, so the keys still step and the bytes stay as
//!   they are. Both families.
//! - [`CipherSiteKind::GameCipherSend`], [`CipherSiteKind::GameCipherReceive`]: the Enhanced Client's game
//!   connection ciphers (`UOGameCipher_C2S_Encrypt`, Twofish output XOR; `UOGameCipher_S2C_Decrypt`, MD5 pad
//!   XOR), installed when it connects to the game server. The patch replaces each XOR with NOPs. Kingdom Reborn
//!   still has the classes but never creates them (no reference to their constructors), and these patterns are not
//!   in its code.
//! - [`CipherSiteKind::KeyExchangeCipher`]: the ciphers the 0xE3 / 0xE4 key exchange installs on the game
//!   connection (`UOGameCipher2_C2S_Encrypt` and `UOGameCipher2_S2C_Decrypt`, both Crypto++ filters behind one
//!   `CryptoPPAdapter::Transform`). The adapter loops over the message's chunks and runs the filter on each in
//!   place; the patch turns the loop's entry `JZ` into a `JMP`, so the message passes through unchanged in both
//!   directions. Kingdom Reborn's only game ciphers; the exchange code is compiled into the traced Enhanced Client
//!   but is not used on its normal game-login route.
//! - [`CipherSiteKind::GameLoginHandshake`]: Kingdom Reborn's game connection setup. Send the relay auth key as
//!   the seed, enter the ready state, install Huffman and send 0x91 immediately, bypassing the 0xE3 / 0xE4 exchange.
//!
//! The Huffman compression of server-to-client game traffic is left alone: the server still compresses.
//!
//! Cipher sites use byte patterns with wildcards for call displacements. KR handshake resolution checks a known
//! complete instruction layout and independently validates its helper/RTTI/import/ownership graph; addresses are
//! decoded from the image. Stock, current and supported earlier replacements are recognized. Only a unique
//! validated site is exposed; unknown or ambiguous layouts are refused.

crate::eyre_imports!();

use std::borrow::Cow;

use crate::client::facet_dimensions::{code_section, ClientFamily};
use crate::client::pe::PeImage;

/// A wildcard in a [`CipherPattern`]: any byte (a call's rel32).
pub const PATTERN_WILDCARD: u16 = 0x100;
const ANY: u16 = PATTERN_WILDCARD;
/// `NOP`.
const OPCODE_NOP: u8 = 0x90;
/// `JZ rel8` and `JMP rel8`: the same displacement, taken always.
const OPCODE_JZ_REL8: u8 = 0x74;
const OPCODE_JMP_REL8: u8 = 0xeb;
/// `XOR [EAX],CL`: 30 /r, ModRM `00 001 000`.
const XOR_MEM_EAX_CL: [u8; 2] = [0x30, 0x08];
/// `XOR [EBP+0],AL`: 30 /r, ModRM `01 000 101`, disp8 0.
const XOR_MEM_EBP_AL: [u8; 3] = [0x30, 0x45, 0x00];
/// `XOR [EAX],DL`: 30 /r, ModRM `00 010 000`.
const XOR_MEM_EAX_DL: [u8; 2] = [0x30, 0x10];

/// Independently selectable parts of the executable's network encryption and KR connection setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncryptionPart {
    /// Early Classic clients share one cipher between their login and game connections.
    SharedLoginGame,
    /// The login connection's cipher.
    Login,
    /// All game send/receive ciphers, including the compiled-in key exchange ciphers.
    Game,
    /// KR's mandatory game key exchange setup, independent of cipher removal.
    KrHandshake,
}

/// One cipher or connection setup the patch disables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CipherSiteKind {
    /// Early Classic clients' shared client-to-server rotation cipher.
    SharedLoginGameCipher,
    /// The login connection's client-to-server cipher (both families).
    LoginCipher,
    /// The Enhanced Client's game connection client-to-server cipher.
    GameCipherSend,
    /// The Enhanced Client's game connection server-to-client cipher.
    GameCipherReceive,
    /// The ciphers the 0xE3 / 0xE4 key exchange installs on the game connection, both directions.
    KeyExchangeCipher,
    /// KR's game seed and mandatory key exchange, replaced with immediate plaintext game login.
    GameLoginHandshake,
}

/// The sites each family has, in order. Only the recognized early Classic layouts expose a shared cipher.
const ENHANCED_SITES: [CipherSiteKind; 4] = [
    CipherSiteKind::LoginCipher,
    CipherSiteKind::GameCipherSend,
    CipherSiteKind::GameCipherReceive,
    CipherSiteKind::KeyExchangeCipher,
];
const KINGDOM_REBORN_SITES: [CipherSiteKind; 3] = [
    CipherSiteKind::LoginCipher,
    CipherSiteKind::KeyExchangeCipher,
    CipherSiteKind::GameLoginHandshake,
];
const EARLY_CLASSIC_SITES: [CipherSiteKind; 1] = [CipherSiteKind::SharedLoginGameCipher];

/// Where a cipher's bytes are and what the patch writes there.
#[derive(Debug, PartialEq, Eq)]
pub struct CipherPattern {
    /// The code around the patched bytes, a byte or [`PATTERN_WILDCARD`]; the patched bytes are at
    /// [`Self::patch_at`] in their stock form.
    pub bytes: &'static [u16],
    pub patch_at: usize,
    pub stock: &'static [u8],
    pub patched: &'static [u8],
    /// Required pattern VA for replacements containing build-specific calls or absolute operands.
    pub required_va: Option<u32>,
    /// An earlier replacement that can be recognized and upgraded to `patched`.
    pub previous: Option<&'static [u8]>,
}

impl CipherPattern {
    /// The stock code of the pattern, each wildcard as `fill` (for synthetic images).
    pub fn stock_code(&self, fill: u8) -> Vec<u8> {
        self.bytes.iter().map(|byte| u8::try_from(*byte).unwrap_or(fill)).collect()
    }
}

/// `UOLoginCipher_C2S_Encrypt::Transform`, the XOR at KR VA 0x5de19a, UOSA 4.0.102.3 0x63c7da:
/// `PUSH EBX; PUSH EAX; CALL byte_at; MOV CL,[ESI+4]; XOR [EAX],CL; MOV ECX,[ESI+4]; MOV EAX,[ESI+8]; MOV EDX,EAX;
/// SHR EAX,1; MOV EDI,ECX; SHL EDI,31; OR EAX,EDI; XOR EAX,imm32` (the key step).
const LOGIN_CIPHER: CipherPattern = CipherPattern {
    bytes: &[
        0x53, 0x50, 0xe8, ANY, ANY, ANY, ANY, 0x8a, 0x4e, 0x04, 0x30, 0x08, 0x8b, 0x4e, 0x04, 0x8b, 0x46, 0x08, 0x8b,
        0xd0, 0xd1, 0xe8, 0x8b, 0xf9, 0xc1, 0xe7, 0x1f, 0x0b, 0xc7, 0x35,
    ],
    patch_at: 10,
    stock: &XOR_MEM_EAX_CL,
    patched: &[OPCODE_NOP; XOR_MEM_EAX_CL.len()],
    required_va: None,
    previous: None,
};

/// `UOGameCipher_C2S_Encrypt::EncryptByte`, the XOR at UOSA 4.0.102.3 VA 0x639ff9: `MOV EDX,[EBX+0x124C];
/// MOV AL,[EDX+EBX+0x114C]; XOR [EBP+0],AL; ADD DWORD [EBX+0x124C],1` (the Twofish output table byte).
const GAME_CIPHER_SEND: CipherPattern = CipherPattern {
    bytes: &[
        0x8b, 0x93, 0x4c, 0x12, 0x00, 0x00, 0x8a, 0x84, 0x1a, 0x4c, 0x11, 0x00, 0x00, 0x30, 0x45, 0x00, 0x83, 0x83,
        0x4c, 0x12, 0x00, 0x00, 0x01,
    ],
    patch_at: 13,
    stock: &XOR_MEM_EBP_AL,
    patched: &[OPCODE_NOP; XOR_MEM_EBP_AL.len()],
    required_va: None,
    previous: None,
};

/// `UOGameCipher_S2C_Decrypt::Transform`, the XOR at UOSA 4.0.102.3 VA 0x63becb: `MOV ECX,[ESI+0x18];
/// AND ECX,0x8000000F; JNS +5; DEC ECX; OR ECX,-16; INC ECX; MOV DL,[ECX+ESI+8]; XOR [EAX],DL;
/// ADD DWORD [ESI+0x18],1` (the MD5 pad byte at the position modulo 16).
const GAME_CIPHER_RECEIVE: CipherPattern = CipherPattern {
    bytes: &[
        0x8b, 0x4e, 0x18, 0x81, 0xe1, 0x0f, 0x00, 0x00, 0x80, 0x79, 0x05, 0x49, 0x83, 0xc9, 0xf0, 0x41, 0x8a, 0x54,
        0x31, 0x08, 0x30, 0x10, 0x83, 0x46, 0x18, 0x01,
    ],
    patch_at: 20,
    stock: &XOR_MEM_EAX_DL,
    patched: &[OPCODE_NOP; XOR_MEM_EAX_DL.len()],
    required_va: None,
    previous: None,
};

/// `CryptoPPAdapter::Transform` (KR 0x5dad60, UOSA 4.0.102.3 0x6390b0), after the message's chunk range is read:
/// `MOV EDI,[ESP+0x2C]; CMP EDI,[ESP+0x3C]; ...; JZ done; MOV ESI,[EDI]; ...; ADD EBX,0x1C; CALL EAX` (the first
/// chunk's data pointer). The `JZ` skips the loop for an empty message; as `JMP` it skips it always.
const KEY_EXCHANGE_CIPHER: CipherPattern = CipherPattern {
    bytes: &[
        0x8b, 0x7c, 0x24, 0x2c, 0x3b, 0x7c, 0x24, 0x3c, 0x8b, 0x4c, 0x24, 0x34, 0x8b, 0x54, 0x24, 0x38, 0x89, 0x4c,
        0x24, 0x1c, 0x89, 0x54, 0x24, 0x20, 0x74, 0x5d, 0x8b, 0x37, 0x8b, 0x44, 0x24, 0x0c, 0x8b, 0x40, 0x04, 0x8b,
        0x18, 0x8b, 0x16, 0x89, 0x44, 0x24, 0x10, 0x8b, 0x42, 0x04, 0x8b, 0xce, 0x83, 0xc3, 0x1c, 0xff, 0xd0,
    ],
    patch_at: 24,
    stock: &[OPCODE_JZ_REL8],
    patched: &[OPCODE_JMP_REL8],
    required_va: None,
    previous: None,
};

impl CipherSiteKind {
    pub fn part(self) -> EncryptionPart {
        match self {
            Self::SharedLoginGameCipher => EncryptionPart::SharedLoginGame,
            Self::LoginCipher => EncryptionPart::Login,
            Self::GameCipherSend | Self::GameCipherReceive | Self::KeyExchangeCipher => EncryptionPart::Game,
            Self::GameLoginHandshake => EncryptionPart::KrHandshake,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SharedLoginGameCipher => "shared login/game cipher",
            Self::LoginCipher => "login cipher",
            Self::GameCipherSend => "game cipher (send)",
            Self::GameCipherReceive => "game cipher (receive)",
            Self::KeyExchangeCipher => "0xE3 key exchange ciphers",
            Self::GameLoginHandshake => "KR game login handshake",
        }
    }

    /// What the patch writes, for a byte change's field name.
    pub fn patch_label(self) -> &'static str {
        match self {
            Self::SharedLoginGameCipher => "shared login/game cipher XOR (NOPs)",
            Self::LoginCipher => "login cipher XOR (NOPs)",
            Self::GameCipherSend => "game send cipher XOR (NOPs)",
            Self::GameCipherReceive => "game receive cipher XOR (NOPs)",
            Self::KeyExchangeCipher => "key exchange cipher loop (JZ to JMP)",
            Self::GameLoginHandshake => "KR auth seed, Huffman and immediate 0x91 (skip 0xE3/0xE4)",
        }
    }

    /// Static cipher signature; the KR handshake entry is a canonical fixture template, not a runtime address.
    /// Runtime patches use the bytes of the resolved [`CipherSite`].
    pub fn pattern(self) -> &'static CipherPattern {
        match self {
            Self::SharedLoginGameCipher => &super::classic_login::PATTERN,
            Self::LoginCipher => &LOGIN_CIPHER,
            Self::GameCipherSend => &GAME_CIPHER_SEND,
            Self::GameCipherReceive => &GAME_CIPHER_RECEIVE,
            Self::KeyExchangeCipher => &KEY_EXCHANGE_CIPHER,
            Self::GameLoginHandshake => &super::kr_game_login::PATTERN,
        }
    }

    /// The sites `family` has; Classic's shared site is conditional on recognizing an early cipher layout.
    pub fn of_family(family: ClientFamily) -> &'static [Self] {
        match family {
            ClientFamily::Enhanced => &ENHANCED_SITES,
            ClientFamily::KingdomReborn => &KINGDOM_REBORN_SITES,
            ClientFamily::Classic => &EARLY_CLASSIC_SITES,
        }
    }
}

/// A located cipher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CipherSite {
    pub kind: CipherSiteKind,
    /// File offset of the bytes the patch writes.
    pub offset: usize,
    /// True when they are in their patched form.
    pub removed: bool,
    stock_bytes: Cow<'static, [u8]>,
    patched_bytes: Cow<'static, [u8]>,
    absolute_operands: Vec<usize>,
    relocation_repair: bool,
}

impl CipherSite {
    /// Expected input bytes. Dynamic sites retain their actual input, including earlier replacements.
    pub fn stock(&self) -> &[u8] {
        &self.stock_bytes
    }

    pub fn patched(&self) -> &[u8] {
        &self.patched_bytes
    }

    pub(crate) fn absolute_operands(&self) -> &[usize] {
        &self.absolute_operands
    }

    /// Already patched code can still carry stale loader relocations from an earlier patcher.
    pub fn needs_relocation_repair(&self) -> bool {
        self.relocation_repair
    }
}

/// A cipher whose pattern did not match exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnlocatedCipherSite {
    pub kind: CipherSiteKind,
    pub matches: usize,
}

/// Whether the client encrypts, as a manifest records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncryptionState {
    /// Every cipher is the stock one.
    Encrypted,
    /// Every cipher is patched out.
    Removed,
    /// Some are patched out, some are not.
    Partial,
}

impl EncryptionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Encrypted => "on (stock)",
            Self::Removed => "removed (patched: plaintext)",
            Self::Partial => "PARTLY removed",
        }
    }
}

/// The ciphers of one build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionSites {
    /// The located sites, in [`CipherSiteKind::of_family`] order.
    pub sites: Vec<CipherSite>,
    /// The sites not located; a patch is refused unless this is empty.
    pub unlocated: Vec<UnlocatedCipherSite>,
}

impl EncryptionSites {
    /// The located and unlocated sites of one independently selectable part. An empty part is unsupported.
    pub fn for_part(&self, part: EncryptionPart) -> Self {
        Self {
            sites: self.sites.iter().filter(|site| site.kind.part() == part).cloned().collect(),
            unlocated: self.unlocated.iter().filter(|site| site.kind.part() == part).copied().collect(),
        }
    }

    /// True when every cipher of the family was located.
    pub fn is_complete(&self) -> bool {
        self.unlocated.is_empty() && !self.sites.is_empty()
    }

    /// The state of the located sites (`None` without any).
    pub fn state(&self) -> Option<EncryptionState> {
        let removed = self.sites.iter().filter(|site| site.removed).count();
        match removed {
            _ if self.sites.is_empty() => None,
            0 => Some(EncryptionState::Encrypted),
            count if count == self.sites.len() => Some(EncryptionState::Removed),
            _ => Some(EncryptionState::Partial),
        }
    }

    /// The state a manifest records: only when every site was located.
    pub fn values(&self) -> Option<EncryptionState> {
        self.state().filter(|_| self.is_complete())
    }

    /// The file offsets of the sites, to check a patched copy locates them at the same places.
    pub fn offsets(&self) -> Vec<usize> {
        self.sites.iter().map(|site| site.offset).collect()
    }

    /// The unlocated sites, in words: "login cipher (0 matches), ...".
    pub fn describe_unlocated(&self) -> String {
        let parts: Vec<String> = self
            .unlocated
            .iter()
            .map(|site| {
                let plural = if site.matches == 1 { "" } else { "es" };
                format!("{} ({} match{plural})", site.kind.label(), site.matches)
            })
            .collect();
        parts.join(", ")
    }
}

/// The offsets in `code` where `pattern` matches, its patched bytes in either form, with whether they are patched.
fn pattern_matches(code: &[u8], pattern: &CipherPattern) -> Vec<(usize, bool)> {
    let length = pattern.bytes.len();
    let patched_range = pattern.patch_at..pattern.patch_at + pattern.stock.len();
    if code.len() < length {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (at, window) in code.windows(length).enumerate() {
        let surroundings = window.iter().zip(pattern.bytes).enumerate().all(|(index, (byte, expected))| {
            patched_range.contains(&index) || *expected == ANY || u16::from(*byte) == *expected
        });
        if !surroundings {
            continue;
        }
        let written = &window[patched_range.clone()];
        if written == pattern.stock {
            found.push((at, false));
        } else if written == pattern.patched {
            found.push((at, true));
        }
    }
    found
}

/// Locates the ciphers of `family` in `.text`; `None` for Classic without a supported early cipher graph.
pub fn locate_encryption_sites(
    bytes: &[u8],
    pe: &PeImage,
    family: ClientFamily,
) -> eyre::Result<Option<EncryptionSites>> {
    if family == ClientFamily::Classic {
        let mut matches = super::classic_login::locate(bytes, pe);
        if matches.is_empty() { return Ok(None); }
        let kind = CipherSiteKind::SharedLoginGameCipher;
        let mut located = EncryptionSites { sites: Vec::new(), unlocated: Vec::new() };
        if matches.len() == 1 {
            let site = matches.pop().expect("one validated Classic cipher");
            located.sites.push(CipherSite {
                kind, offset: site.offset, removed: site.removed,
                stock_bytes: Cow::Owned(site.stock.to_vec()), patched_bytes: Cow::Owned(vec![OPCODE_NOP; site.stock.len()]),
                absolute_operands: Vec::new(), relocation_repair: false,
            });
        } else { located.unlocated.push(UnlocatedCipherSite { kind, matches: matches.len() }); }
        return Ok(Some(located));
    }
    let kinds = CipherSiteKind::of_family(family);
    if kinds.is_empty() {
        return Ok(None);
    }
    let (code, base) = code_section(bytes, pe)?;
    let mut located = EncryptionSites { sites: Vec::new(), unlocated: Vec::new() };
    for &kind in kinds {
        if kind == CipherSiteKind::GameLoginHandshake {
            let image = super::kr_resolver::Image::new(bytes, pe);
            let mut matches = super::kr_game_login::resolve(&image);
            matches.extend(super::kr_game_login_legacy::resolve(&image));
            if matches.len() == 1 {
                let resolved = matches.pop().expect("one validated handshake");
                let span = resolved.offset..resolved.offset + resolved.patched.len();
                let relocation_repair = resolved.removed
                    && !super::pe_relocations::plan(bytes, pe, span, &resolved.absolute_operands)
                        .is_ok_and(|writes| writes.is_empty());
                located.sites.push(CipherSite {
                    kind, offset: resolved.offset, removed: resolved.removed,
                    stock_bytes: Cow::Owned(resolved.stock), patched_bytes: Cow::Owned(resolved.patched),
                    absolute_operands: resolved.absolute_operands,
                    relocation_repair,
                });
            } else {
                located.unlocated.push(UnlocatedCipherSite { kind, matches: matches.len() });
            }
            continue;
        }
        let (patterns, earlier): (&[CipherPattern], &[CipherPattern]) = match kind {
            CipherSiteKind::LoginCipher if family == ClientFamily::KingdomReborn => {
                (std::slice::from_ref(kind.pattern()), std::slice::from_ref(&super::kr_legacy_login::LOGIN_CIPHER))
            }
            CipherSiteKind::KeyExchangeCipher if family == ClientFamily::KingdomReborn => {
                (std::slice::from_ref(kind.pattern()), std::slice::from_ref(&super::kr_legacy_login::KEY_EXCHANGE_CIPHER))
            }
            _ => (std::slice::from_ref(kind.pattern()), &[]),
        };
        let mut matches: Vec<(usize, bool, &CipherPattern)> = Vec::new();
        for pattern in patterns.iter().chain(earlier) {
            for (at, removed) in pattern_matches(code, pattern) {
                matches.push((at, removed, pattern));
            }
        }
        match matches.as_slice() {
            [(at, removed, pattern)] => {
                located.sites.push(CipherSite {
                    kind, offset: base + at + pattern.patch_at, removed: *removed,
                    stock_bytes: Cow::Borrowed(pattern.stock), patched_bytes: Cow::Borrowed(pattern.patched),
                    absolute_operands: Vec::new(),
                    relocation_repair: false,
                })
            }
            matches => located.unlocated.push(UnlocatedCipherSite { kind, matches: matches.len() }),
        }
    }
    Ok(Some(located))
}

#[cfg(test)]
#[path = "../../tests/common/kr_handshake.rs"]
pub(crate) mod handshake_fixture;
#[cfg(test)]
#[path = "../../tests/common/kr_client_fixture.rs"]
pub(crate) mod client_fixture;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::client::facet_dimensions::tests::synthetic_image_with;
    use crate::client::facet_dimensions::ClientExeInspection;
    use crate::enhanced::facet_sector::CLIENT_FACET_DIMENSIONS;

    /// Filler between the patterns: `INT3`, which no pattern starts with.
    const FILLER: u8 = 0xcc;
    const FILLER_LEN: usize = 32;
    /// The bytes standing for a pattern's wildcards.
    const WILDCARD_FILL: u8 = 0x12;
    const FACET_NAMES: [&str; 7] = ["Felucca", "Trammel", "Ilshenar", "Malas", "Tokuno", "Gargoyle", "Test"];
    /// The synthetic PE's first section VA.
    const CODE_VA: u32 = super::super::pe::tests::TEST_IMAGE_BASE + super::super::pe::tests::TEST_SECTION_ALIGNMENT;

    /// Code with the stock patterns of `kinds`, each between filler.
    pub fn cipher_code(kinds: &[CipherSiteKind]) -> Vec<u8> {
        let mut code = vec![FILLER; FILLER_LEN];
        for kind in kinds {
            if let Some(va) = kind.pattern().required_va {
                code.resize((va - CODE_VA) as usize, FILLER);
            }
            code.extend(kind.pattern().stock_code(WILDCARD_FILL));
            code.extend([FILLER; FILLER_LEN]);
        }
        code
    }

    pub fn complete_kr_client(image: Vec<u8>, ciphers: &[CipherSiteKind]) -> Vec<u8> {
        let facets: Vec<_> = FACET_NAMES.into_iter().zip(CLIENT_FACET_DIMENSIONS)
            .map(|(name, geometry)| (name, geometry.width, geometry.height)).collect();
        client_fixture::client_image(image, &cipher_code(ciphers), &facets)
    }

    /// A Kingdom Reborn image with real helper relationships, imports, RTTI, ciphers and facet table.
    pub fn synthetic_kr_encrypted_client() -> Vec<u8> {
        complete_kr_client(handshake_fixture::modern(client_fixture::CONNECT_OFFSET).bytes,
            &[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher])
    }

    pub fn synthetic_kr_previous_client() -> Vec<u8> {
        let mut fixture = handshake_fixture::modern(client_fixture::CONNECT_OFFSET);
        handshake_fixture::previous_patch(&mut fixture);
        complete_kr_client(fixture.bytes, &[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher])
    }

    /// An Enhanced Client image with its four ciphers.
    pub fn synthetic_enhanced_encrypted_client() -> Vec<u8> {
        synthetic_image_with(cipher_code(&ENHANCED_SITES), Vec::new(), true)
    }

    fn sites(image: &[u8]) -> EncryptionSites {
        ClientExeInspection::inspect(image).unwrap().encryption.unwrap()
    }

    #[test]
    fn the_ciphers_are_located_in_either_form() {
        for (image, kinds) in [
            (synthetic_kr_encrypted_client(), &KINGDOM_REBORN_SITES[..]),
            (synthetic_enhanced_encrypted_client(), &ENHANCED_SITES[..]),
        ] {
            let mut located = sites(&image);
            assert!(located.is_complete());
            assert_eq!(located.state(), Some(EncryptionState::Encrypted));
            assert_eq!(located.sites.iter().map(|site| site.kind).collect::<Vec<_>>(), kinds);
            for site in &located.sites {
                assert_eq!(image[site.offset..][..site.stock().len()], *site.stock(), "{:?}", site.kind);
            }
            // The first site patched: partial; all patched: removed, at the same offsets.
            let mut patched = image.clone();
            let first = located.sites.remove(0);
            patched[first.offset..][..first.patched().len()].copy_from_slice(first.patched());
            let partial = sites(&patched);
            assert_eq!(partial.state(), Some(EncryptionState::Partial));
            assert!(partial.sites[0].removed && !partial.sites[1].removed);
            for site in &located.sites {
                patched[site.offset..][..site.patched().len()].copy_from_slice(site.patched());
            }
            let removed = sites(&patched);
            assert_eq!(removed.state(), Some(EncryptionState::Removed));
            assert_eq!(removed.values(), Some(EncryptionState::Removed));
            assert_eq!(removed.offsets(), sites(&image).offsets());
        }
    }

    #[test]
    fn missing_ambiguous_or_unknown_ciphers_are_not_located() {
        // No ciphers at all, a twice present login cipher, a login cipher XOR that is neither form.
        let none = sites(&synthetic_image_with(vec![FILLER; FILLER_LEN], Vec::new(), false));
        assert_eq!(none.unlocated.len(), KINGDOM_REBORN_SITES.len());
        assert!(!none.is_complete() && none.state().is_none() && none.values().is_none());

        let twice = complete_kr_client(handshake_fixture::modern(client_fixture::CONNECT_OFFSET).bytes,
            &[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher, CipherSiteKind::LoginCipher]);
        let twice = sites(&twice);
        assert_eq!(twice.unlocated, [UnlocatedCipherSite { kind: CipherSiteKind::LoginCipher, matches: 2 }]);
        assert_eq!(twice.describe_unlocated(), "login cipher (2 matches)");
        assert_eq!(twice.values(), None, "not complete: not recorded");

        let mut other = synthetic_kr_encrypted_client();
        let at = sites(&other).sites[0].offset;
        other[at] = FILLER;
        let other = sites(&other);
        assert_eq!(other.unlocated[0].matches, 0);

        // The Kingdom Reborn sites only, in an Enhanced Client: its game ciphers are missing.
        let partial = sites(&synthetic_image_with(cipher_code(&[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher]), Vec::new(), true));
        let kinds: Vec<CipherSiteKind> = partial.unlocated.iter().map(|site| site.kind).collect();
        assert_eq!(kinds, [CipherSiteKind::GameCipherSend, CipherSiteKind::GameCipherReceive]);
    }

    #[test]
    fn kr_handshake_requires_valid_cleanup_and_helper_relationships() {
        const CONNECT_SIZE: usize = 655;
        let fixture = handshake_fixture::modern(client_fixture::CONNECT_OFFSET);
        let mut image = complete_kr_client(fixture.bytes, &[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher]);
        let pe = PeImage::parse(&image).unwrap();
        let start = pe.va_to_offset(fixture.connect).unwrap();
        image[start + CONNECT_SIZE - 1] = FILLER;
        let altered = sites(&image);
        assert_eq!(altered.unlocated, [UnlocatedCipherSite { kind: CipherSiteKind::GameLoginHandshake, matches: 0 }]);

        // A function-shaped byte sequence without imports, RTTI and independently checked helpers is insufficient.
        let bare = synthetic_image_with(cipher_code(&KINGDOM_REBORN_SITES), Vec::new(), false);
        assert_eq!(sites(&bare).unlocated, altered.unlocated);
    }

    #[test]
    fn unseen_modern_and_legacy_graphs_locate_and_round_trip() {
        const CONNECT_OFFSETS: [usize; 2] = [0x700, 0x900];
        for connect in CONNECT_OFFSETS {
            for raw in [handshake_fixture::modern(connect).bytes, super::super::kr_game_login_legacy::fixture_image(connect)] {
                let mut image = complete_kr_client(raw, &[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher]);
                let before = sites(&image);
                assert!(before.is_complete(), "{}", before.describe_unlocated());
                let handshake = before.sites.last().unwrap();
                image[handshake.offset..][..handshake.patched().len()].copy_from_slice(handshake.patched());
                let after = sites(&image);
                assert!(after.is_complete());
                assert!(after.sites.last().unwrap().removed);
                assert_eq!(after.offsets(), before.offsets());
                assert_eq!(after.sites.last().unwrap().stock(), after.sites.last().unwrap().patched(),
                    "a removed dynamic site's input bytes are its current replacement");
                image[handshake.offset] = FILLER;
                assert_eq!(sites(&image).unlocated, [UnlocatedCipherSite { kind: CipherSiteKind::GameLoginHandshake, matches: 0 }]);
            }
        }
    }

    #[test]
    fn dynamically_resolved_helpers_may_use_an_unseen_image_base_and_section_layout() {
        const IMAGE_BASE: u32 = 0x0060_0000;
        const HELPER_START: usize = 0x1200;
        const RDATA_RVA: u32 = 0xa000;
        const DATA_RVA: u32 = 0x12000;
        let fixture = handshake_fixture::modern_layout(client_fixture::CONNECT_OFFSET,
            HELPER_START, IMAGE_BASE, RDATA_RVA, DATA_RVA);
        let mut image = complete_kr_client(fixture.bytes, &[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher]);
        let before = sites(&image);
        assert!(before.is_complete(), "{}", before.describe_unlocated());
        for site in before.sites {
            image[site.offset..][..site.patched().len()].copy_from_slice(site.patched());
        }
        assert_eq!(sites(&image).values(), Some(EncryptionState::Removed));
    }

    #[test]
    fn earlier_kr_ciphers_round_trip_and_remain_ambiguous_with_another_login_loop() {
        let earlier = [&super::super::kr_legacy_login::LOGIN_CIPHER, &super::super::kr_legacy_login::KEY_EXCHANGE_CIPHER];
        let mut code = vec![FILLER; FILLER_LEN];
        for pattern in earlier {
            code.extend(pattern.stock_code(WILDCARD_FILL));
            code.extend([FILLER; FILLER_LEN]);
        }
        let mut image = complete_kr_client(super::super::kr_game_login_legacy::fixture_image(client_fixture::CONNECT_OFFSET), &[]);
        let pe = PeImage::parse(&image).unwrap();
        let at = pe.section(".text").unwrap().raw_offset as usize + client_fixture::CIPHER_TEXT_OFFSET;
        image[at..at + code.len()].copy_from_slice(&code);
        let before = sites(&image);
        assert!(before.is_complete());
        assert_eq!(before.values(), Some(EncryptionState::Encrypted));
        for site in &before.sites {
            image[site.offset..][..site.patched().len()].copy_from_slice(site.patched());
        }
        let after = sites(&image);
        assert!(after.is_complete());
        assert_eq!(after.values(), Some(EncryptionState::Removed));
        assert_eq!(after.offsets(), before.offsets());
        const DUPLICATE_OFFSET: usize = 0x400;
        let at = pe.section(".text").unwrap().raw_offset as usize + DUPLICATE_OFFSET;
        let duplicate_code = LOGIN_CIPHER.stock_code(WILDCARD_FILL);
        image[at..at + duplicate_code.len()].copy_from_slice(&duplicate_code);
        let duplicate = sites(&image);
        assert_eq!(duplicate.unlocated, [UnlocatedCipherSite { kind: CipherSiteKind::LoginCipher, matches: 2 }]);
    }

    #[test]
    fn separate_valid_dynamic_functions_remain_ambiguous_in_stock_and_patched_forms() {
        const FIRST_CONNECT: usize = 0x700;
        const SECOND_CONNECT: usize = 0x400;
        const CONNECT_SIZE: usize = 655;
        for removed in [false, true] {
            let mut first = handshake_fixture::modern(FIRST_CONNECT);
            let mut second = handshake_fixture::modern(SECOND_CONNECT);
            if removed {
                handshake_fixture::current_patch(&mut first);
                handshake_fixture::current_patch(&mut second);
            }
            let second_pe = PeImage::parse(&second.bytes).unwrap();
            let source = second_pe.va_to_offset(second.connect).unwrap();
            let mut image = complete_kr_client(first.bytes, &[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher]);
            let pe = PeImage::parse(&image).unwrap();
            let destination = pe.va_to_offset(second.connect).unwrap();
            image[destination..destination + CONNECT_SIZE].copy_from_slice(&second.bytes[source..source + CONNECT_SIZE]);
            let ambiguous = sites(&image);
            assert_eq!(ambiguous.unlocated, [UnlocatedCipherSite { kind: CipherSiteKind::GameLoginHandshake, matches: 2 }]);
        }
    }

    #[test]
    fn previous_kr_handshake_replacement_is_located_for_upgrade() {
        let mut image = synthetic_kr_previous_client();
        let previous = sites(&image);
        assert!(previous.is_complete());
        let upgrade = previous.sites.last().unwrap();
        assert!(!upgrade.removed);
        assert_ne!(upgrade.stock(), upgrade.patched());
        assert_eq!(&image[upgrade.offset..][..upgrade.stock().len()], upgrade.stock());
        image[upgrade.offset..][..upgrade.patched().len()].copy_from_slice(upgrade.patched());
        assert!(sites(&image).sites.last().unwrap().removed);
    }

    #[test]
    fn later_classic_has_no_encryption_sites_and_the_state_serializes_in_snake_case() {
        let classic = crate::client::classic_facets::tests::synthetic_classic(6, 2);
        assert!(ClientExeInspection::inspect(&classic).unwrap().encryption.is_none());
        assert_eq!(serde_json::to_string(&EncryptionState::Removed).unwrap(), "\"removed\"");
        for pattern in [&LOGIN_CIPHER, &GAME_CIPHER_SEND, &GAME_CIPHER_RECEIVE, &KEY_EXCHANGE_CIPHER, &super::super::kr_game_login::PATTERN] {
            let stock = &pattern.bytes[pattern.patch_at..pattern.patch_at + pattern.stock.len()];
            assert!(stock.iter().zip(pattern.stock).all(|(byte, expected)| *byte == u16::from(*expected)));
            assert_eq!(pattern.stock.len(), pattern.patched.len());
        }
    }
}
