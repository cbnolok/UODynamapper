//! The network encryption of Enhanced Client and Kingdom Reborn executables, located by instruction pattern
//! ([`locate_encryption_sites`]) so a patch ([`crate::client::patch::ExeEdit::NoEncryption`]) can make the client
//! talk plaintext. See `tools/uo-client-patcher-gui/docs/ENCRYPTION.md` and the traced code in
//! `tools/uo-client-patcher-gui/docs/research/uokr_login.md`.
//!
//! The client sends and receives through a chain of transforms per connection. The ones this patch disables:
//!
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
//! Cipher sites use byte patterns with wildcards for call displacements; the KR handshake requires its exact
//! audited function and original VA because its replacement embeds build-specific calls. The bytes each patch writes
//! match either their stock or their patched form, so a patched copy is located too. A site counts only when
//! its pattern matches exactly once in `.text`; otherwise it is reported unlocated and the patch is refused.

crate::eyre_imports!();

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

/// The sites each family has, in order. Classic has none located here.
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
            Self::LoginCipher => EncryptionPart::Login,
            Self::GameCipherSend | Self::GameCipherReceive | Self::KeyExchangeCipher => EncryptionPart::Game,
            Self::GameLoginHandshake => EncryptionPart::KrHandshake,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
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
            Self::LoginCipher => "login cipher XOR (NOPs)",
            Self::GameCipherSend => "game send cipher XOR (NOPs)",
            Self::GameCipherReceive => "game receive cipher XOR (NOPs)",
            Self::KeyExchangeCipher => "key exchange cipher loop (JZ to JMP)",
            Self::GameLoginHandshake => "KR auth seed, Huffman and immediate 0x91 (skip 0xE3/0xE4)",
        }
    }

    pub fn pattern(self) -> &'static CipherPattern {
        match self {
            Self::LoginCipher => &LOGIN_CIPHER,
            Self::GameCipherSend => &GAME_CIPHER_SEND,
            Self::GameCipherReceive => &GAME_CIPHER_RECEIVE,
            Self::KeyExchangeCipher => &KEY_EXCHANGE_CIPHER,
            Self::GameLoginHandshake => &super::kr_game_login::PATTERN,
        }
    }

    /// The sites `family` has; empty for Classic.
    pub fn of_family(family: ClientFamily) -> &'static [Self] {
        match family {
            ClientFamily::Enhanced => &ENHANCED_SITES,
            ClientFamily::KingdomReborn => &KINGDOM_REBORN_SITES,
            ClientFamily::Classic => &[],
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
    pattern: &'static CipherPattern,
    previous: bool,
}

impl CipherSite {
    pub fn stock(&self) -> &'static [u8] {
        if self.previous { self.pattern.previous.expect("recognized earlier replacement") } else { self.pattern.stock }
    }

    pub fn patched(&self) -> &'static [u8] {
        self.pattern.patched
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
fn pattern_matches(code: &[u8], pattern: &CipherPattern) -> Vec<(usize, bool, bool)> {
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
            found.push((at, false, false));
        } else if written == pattern.patched {
            found.push((at, true, false));
        } else if pattern.previous == Some(written) {
            found.push((at, false, true));
        }
    }
    found
}

/// Locates the ciphers of `family` in the `.text` section of `bytes`; `None` for Classic.
pub fn locate_encryption_sites(
    bytes: &[u8],
    pe: &PeImage,
    family: ClientFamily,
) -> eyre::Result<Option<EncryptionSites>> {
    let kinds = CipherSiteKind::of_family(family);
    if kinds.is_empty() {
        return Ok(None);
    }
    let (code, base) = code_section(bytes, pe)?;
    let mut located = EncryptionSites { sites: Vec::new(), unlocated: Vec::new() };
    for &kind in kinds {
        let patterns = if kind == CipherSiteKind::GameLoginHandshake {
            &super::kr_game_login::PATTERNS[..]
        } else {
            std::slice::from_ref(kind.pattern())
        };
        let mut matches: Vec<(usize, bool, bool, &CipherPattern)> = Vec::new();
        for pattern in patterns {
            for (at, removed, previous) in pattern_matches(code, pattern) {
                if pattern.required_va.is_none_or(|va| pe.offset_to_va(base + at) == Some(va)) {
                    // Different exact stock profiles can converge after their differing callback operands are
                    // replaced. They describe one physical patched site, with the same future write.
                    let equivalent_patched = removed && matches.iter().any(|(prior, patched, _, other)| {
                        *patched && at + pattern.patch_at == *prior + other.patch_at && pattern.patched == other.patched
                    });
                    if equivalent_patched {
                        continue;
                    }
                    matches.push((at, removed, previous, pattern));
                }
            }
        }
        match matches.as_slice() {
            [(at, removed, previous, pattern)] => {
                located.sites.push(CipherSite { kind, offset: base + at + pattern.patch_at, removed: *removed, pattern, previous: *previous })
            }
            matches => located.unlocated.push(UnlocatedCipherSite { kind, matches: matches.len() }),
        }
    }
    Ok(Some(located))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::client::facet_dimensions::tests::synthetic_image_with;
    use crate::client::facet_dimensions::ClientExeInspection;

    /// Filler between the patterns: `INT3`, which no pattern starts with.
    const FILLER: u8 = 0xcc;
    const FILLER_LEN: usize = 32;
    /// The bytes standing for a pattern's wildcards.
    const WILDCARD_FILL: u8 = 0x12;
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

    /// A Kingdom Reborn image with its ciphers and game handshake.
    pub fn synthetic_kr_encrypted_client() -> Vec<u8> {
        synthetic_image_with(cipher_code(&KINGDOM_REBORN_SITES), Vec::new(), false)
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

        let mut twice = cipher_code(&KINGDOM_REBORN_SITES);
        twice.extend(cipher_code(&[CipherSiteKind::LoginCipher]));
        let twice = sites(&synthetic_image_with(twice, Vec::new(), false));
        assert_eq!(twice.unlocated, [UnlocatedCipherSite { kind: CipherSiteKind::LoginCipher, matches: 2 }]);
        assert_eq!(twice.describe_unlocated(), "login cipher (2 matches)");
        assert_eq!(twice.values(), None, "not complete: not recorded");

        let mut other = cipher_code(&KINGDOM_REBORN_SITES);
        let at = FILLER_LEN + LOGIN_CIPHER.patch_at;
        other[at] = FILLER;
        let other = sites(&synthetic_image_with(other, Vec::new(), false));
        assert_eq!(other.unlocated[0].matches, 0);

        // The Kingdom Reborn sites only, in an Enhanced Client: its game ciphers are missing.
        let partial = sites(&synthetic_image_with(cipher_code(&KINGDOM_REBORN_SITES), Vec::new(), true));
        let kinds: Vec<CipherSiteKind> = partial.unlocated.iter().map(|site| site.kind).collect();
        assert_eq!(kinds, [CipherSiteKind::GameCipherSend, CipherSiteKind::GameCipherReceive]);
    }

    #[test]
    fn kr_handshake_requires_the_audited_function_at_its_original_address() {
        let pattern = CipherSiteKind::GameLoginHandshake.pattern();
        let code = cipher_code(&KINGDOM_REBORN_SITES);
        let at = (pattern.required_va.unwrap() - CODE_VA) as usize;
        // An altered cleanup is outside the written range but must still reject the entire handshake patch.
        let mut altered = code.clone();
        altered[at + pattern.bytes.len() - 1] = FILLER;
        let altered = sites(&synthetic_image_with(altered, Vec::new(), false));
        assert_eq!(altered.unlocated, [UnlocatedCipherSite { kind: CipherSiteKind::GameLoginHandshake, matches: 0 }]);

        // The exact instructions at a different VA would call the wrong helpers.
        let mut shifted = vec![FILLER; FILLER_LEN];
        shifted.extend(code);
        let shifted = sites(&synthetic_image_with(shifted, Vec::new(), false));
        assert_eq!(shifted.unlocated, altered.unlocated);
    }

    #[test]
    fn every_audited_kr_handshake_profile_locates_and_round_trips() {
        for pattern in &super::super::kr_game_login::PATTERNS {
            let mut code = cipher_code(&[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher]);
            let at = (pattern.required_va.unwrap() - CODE_VA) as usize;
            code.resize(at, FILLER);
            code.extend(pattern.stock_code(WILDCARD_FILL));
            let mut image = synthetic_image_with(code, Vec::new(), false);
            let before = sites(&image);
            assert!(before.is_complete());
            let handshake = before.sites.last().unwrap();
            assert_eq!(handshake.stock(), pattern.stock);
            assert_eq!(handshake.patched(), pattern.patched);
            image[handshake.offset..][..handshake.patched().len()].copy_from_slice(handshake.patched());
            let after = sites(&image);
            assert!(after.is_complete());
            assert!(after.sites.last().unwrap().removed);
            assert_eq!(after.offsets(), before.offsets());
            // An unrecognized helper operand must never be accepted as another build.
            image[handshake.offset - pattern.patch_at + 1] ^= 1;
            assert_eq!(sites(&image).unlocated, [UnlocatedCipherSite { kind: CipherSiteKind::GameLoginHandshake, matches: 0 }]);
        }
    }

    #[test]
    fn converging_kr_profiles_locate_one_patched_site_but_separate_physical_sites_stay_ambiguous() {
        const SHARED_VA: u32 = 0x005b_abe0;
        const EXPECTED_PEER_COUNTS: [usize; 5] = [2, 2, 3, 3, 3];
        let patterns = &super::super::kr_game_login::PATTERNS;
        let patched_code = |pattern: &CipherPattern| {
            let mut code = pattern.stock_code(WILDCARD_FILL);
            code[pattern.patch_at..][..pattern.patched.len()].copy_from_slice(pattern.patched);
            code
        };
        let mut peer_counts = Vec::new();
        for pattern in patterns.iter().filter(|pattern| pattern.required_va == Some(SHARED_VA)) {
            let patched = patched_code(pattern);
            peer_counts.push(patterns.iter().filter(|other| {
                other.required_va == Some(SHARED_VA) && patched_code(other) == patched
            }).count());
            let mut code = cipher_code(&[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher]);
            code.resize((SHARED_VA - CODE_VA) as usize, FILLER);
            code.extend(patched);
            let located = sites(&synthetic_image_with(code, Vec::new(), false));
            assert!(located.is_complete(), "{}", located.describe_unlocated());
            assert_eq!(located.sites.iter().filter(|site| site.kind == CipherSiteKind::GameLoginHandshake).count(), 1);
            assert!(located.sites.last().unwrap().removed);
        }
        peer_counts.sort_unstable();
        assert_eq!(peer_counts, EXPECTED_PEER_COUNTS);

        // Full functions at separate audited VAs remain two physical sites, even though both are patched.
        const SEPARATE_VAS: [u32; 2] = [0x005b_6b40, 0x005b_aeb0];
        let mut code = cipher_code(&[CipherSiteKind::LoginCipher, CipherSiteKind::KeyExchangeCipher]);
        for va in SEPARATE_VAS {
            let pattern = patterns.iter().find(|pattern| pattern.required_va == Some(va)).unwrap();
            code.resize((va - CODE_VA) as usize, FILLER);
            code.extend(patched_code(pattern));
        }
        let ambiguous = sites(&synthetic_image_with(code, Vec::new(), false));
        assert_eq!(ambiguous.unlocated, [UnlocatedCipherSite { kind: CipherSiteKind::GameLoginHandshake, matches: SEPARATE_VAS.len() }]);
    }

    #[test]
    fn previous_kr_handshake_replacement_is_located_for_upgrade() {
        let pattern = CipherSiteKind::GameLoginHandshake.pattern();
        let mut image = synthetic_kr_encrypted_client();
        let before = sites(&image);
        let handshake = before.sites.last().unwrap();
        image[handshake.offset..][..pattern.stock.len()].copy_from_slice(pattern.previous.unwrap());
        let previous = sites(&image);
        assert!(previous.is_complete());
        let upgrade = previous.sites.last().unwrap();
        assert!(!upgrade.removed);
        assert_eq!(upgrade.stock(), pattern.previous.unwrap());
        assert_eq!(upgrade.patched(), pattern.patched);
    }

    #[test]
    fn classic_has_no_encryption_sites_and_the_state_serializes_in_snake_case() {
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
