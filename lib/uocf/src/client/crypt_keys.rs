//! Embedded login master keys and the protocol version advertised by EC/KR's 0xEF seed writer.
//!
//! These are read from instructions, independently of the PE version resource. The version formula is a
//! comparison only: a patched or unusual executable may embed different keys. No connection seed is generated.

crate::eyre_imports!();

use super::encryption::CipherSiteKind;
use super::facet_dimensions::{code_section, ClientExeInspection, ClientFamily};
use super::pe::read_u32;

const ANY: u16 = super::encryption::PATTERN_WILDCARD;
const DWORD_SIZE: usize = size_of::<u32>();
/// From the login XOR onward, including the complete key step and loop back-edge. Key immediates vary.
const KEY_STEP: &[u16] = &[
    ANY, ANY, 0x8b, 0x4e, 0x04, 0x8b, 0x46, 0x08, 0x8b, 0xd0, 0xd1, 0xe8, 0x8b, 0xf9, 0xc1, 0xe7,
    0x1f, 0x0b, 0xc7, 0x35, ANY, ANY, ANY, ANY, 0xd1, 0xe8, 0xd1, 0xe9, 0xc1, 0xe2, 0x1f, 0x0b, 0xc7,
    0x35, ANY, ANY, ANY, ANY, 0x0b, 0xca, 0x83, 0xc3, 0x01, 0x81, 0xf1, ANY, ANY, ANY, ANY, 0x3b, 0x5c,
    0x24, 0x0c, 0x89, 0x46, 0x08, 0x8b, 0x45, 0x0c, 0x89, 0x4e, 0x04, 0x72, 0xb6,
];
const FIRST_ROUND_KEY_OFFSET: usize = 20;
const MASTER_KEY_1_OFFSET: usize = 34;
const MASTER_KEY_2_OFFSET: usize = 45;
const SEED_WRITER_PROLOGUE: &[u8] = &[0x83, 0xec, 0x24, 0x53, 0x55, 0x8b, 0x6c, 0x24, 0x34, 0x56, 0x57];
/// Both forms establish EBX = 0 before storing the seed and four protocol components.
const KR_SEED_SETUP: &[u8] = &[0x8d, 0x44, 0x24, 0x14, 0x33, 0xdb, 0x50];
const EC_SEED_SETUP: &[u8] = &[0x33, 0xdb, 0x6a, 0x20];
const SEED_STORE: &[u8] = &[0x89, 0x6c, 0x24, 0x20];
const MOV_STACK_IMMEDIATE: &[u8] = &[0xc7, 0x44, 0x24];
const MOV_STACK_ZERO_EBX: &[u8] = &[0x89, 0x5c, 0x24];
const STORE_HEADER_SIZE: usize = 4;
const VERSION_STACK_OFFSETS: [u8; 4] = [0x24, 0x28, 0x2c, 0x30];
const OPCODE_CALL_REL32: u8 = 0xe8;
const KEY_1_SHIFTS: [u32; 6] = [23, 14, 4, 9, 24, 19];
const KEY_2_SHIFTS: [u32; 6] = [22, 13, 3, 10, 23, 18];
const KEY_1_MINOR_FACTOR: u32 = 11;
const KEY_2_MINOR_FACTOR: u32 = 13;
const BUILD_FACTOR: u32 = 7;
const KEY_2_SQUARE_FACTOR: u32 = 3;
const KEY_1_XOR: u32 = 0x2c13_a5fd;
const KEY_2_XOR: u32 = 0xa31d_527f;

/// Actual embedded keys; the extra first-round constant matters when reproducing the byte transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginMasterKeys {
    pub key_1: u32,
    pub key_2: u32,
    pub first_round: u32,
}

/// The version fields passed to the 0xEF writer, not the PE resource version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolVersion(pub [u32; 4]);

impl std::fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [major, minor, third, fourth] = self.0;
        write!(f, "{major}.{minor}.{third}.{fourth}")
    }
}

/// Standard vCrypter formula, with unsigned 32-bit wrapping arithmetic. The fourth component is unused.
/// This does not assert that the executable computes its keys from its version at runtime.
pub fn version_master_keys(version: ProtocolVersion) -> (u32, u32) {
    let [major, minor, third, _] = version.0;
    let square = third.wrapping_mul(third);
    let [a, b, c, d, e, f] = KEY_1_SHIFTS;
    let key_1 = ((major << a) | (minor << b) | (third << c))
        ^ (square << d) ^ minor.wrapping_mul(minor)
        ^ (minor.wrapping_mul(KEY_1_MINOR_FACTOR) << e) ^ (third.wrapping_mul(BUILD_FACTOR) << f) ^ KEY_1_XOR;
    let [a, b, c, d, e, f] = KEY_2_SHIFTS;
    let key_2 = ((major << a) | (third << b) | (minor << c))
        ^ (square.wrapping_mul(KEY_2_SQUARE_FACTOR) << d) ^ minor.wrapping_mul(minor)
        ^ (minor.wrapping_mul(KEY_2_MINOR_FACTOR) << e) ^ (third.wrapping_mul(BUILD_FACTOR) << f) ^ KEY_2_XOR;
    (key_1, key_2)
}

/// Located instructions and their extracted values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCryptInfo {
    pub keys: LoginMasterKeys,
    pub cipher_offset: usize,
    pub cipher_removed: bool,
    /// None unless exactly one supported seed-writer instruction sequence matches.
    pub protocol_version: Option<ProtocolVersion>,
    pub protocol_version_matches: usize,
}

fn protocol_version(code: &[u8], at: usize) -> Option<ProtocolVersion> {
    let after_prologue = at.checked_add(SEED_WRITER_PROLOGUE.len())?;
    let rest = code.get(after_prologue..)?;
    let setup = if rest.starts_with(KR_SEED_SETUP) { KR_SEED_SETUP } else { EC_SEED_SETUP };
    if !rest.starts_with(setup) {
        return None;
    }
    let mut cursor = after_prologue + setup.len();
    if !code.get(cursor..)?.starts_with(SEED_STORE) {
        return None;
    }
    cursor += SEED_STORE.len();
    let mut parts = [0; VERSION_STACK_OFFSETS.len()];
    for (index, stack_offset) in VERSION_STACK_OFFSETS.into_iter().enumerate() {
        let rest = code.get(cursor..)?;
        let value = if rest.starts_with(MOV_STACK_IMMEDIATE) && rest.get(MOV_STACK_IMMEDIATE.len()) == Some(&stack_offset) {
            cursor += STORE_HEADER_SIZE + DWORD_SIZE;
            u32::from_le_bytes(rest.get(STORE_HEADER_SIZE..STORE_HEADER_SIZE + DWORD_SIZE)?.try_into().ok()?)
        } else if rest.starts_with(MOV_STACK_ZERO_EBX) && rest.get(MOV_STACK_ZERO_EBX.len()) == Some(&stack_offset) {
            cursor += STORE_HEADER_SIZE;
            0
        } else {
            return None;
        };
        parts[index] = value;
    }
    (code.get(cursor) == Some(&OPCODE_CALL_REL32)).then_some(ProtocolVersion(parts))
}

fn legacy_login_keys(bytes: &[u8], cipher_offset: usize) -> Option<LoginMasterKeys> {
    let pattern = &super::kr_legacy_login::LOGIN_CIPHER;
    let start = cipher_offset.checked_sub(pattern.patch_at)?;
    let code = bytes.get(start..start + pattern.bytes.len())?;
    let patch = pattern.patch_at..pattern.patch_at + pattern.stock.len();
    if !code.iter().zip(pattern.bytes).enumerate().all(|(at, (byte, expected))| {
        patch.contains(&at) || *expected == ANY || u16::from(*byte) == *expected
    }) || (code[patch.clone()] != *pattern.stock && code[patch] != *pattern.patched) {
        return None;
    }
    let key_1 = read_u32(code, super::kr_legacy_login::KEY_1_OFFSET).ok()?;
    Some(LoginMasterKeys { key_1, key_2: read_u32(code, super::kr_legacy_login::KEY_2_OFFSET).ok()?, first_round: key_1 })
}

/// Read the real login transform's immediates. Classic or unrecognized code is refused rather than guessed
/// from the PE resource. Patched login XORs are supported because key stepping remains intact.
pub fn inspect_login_crypt(bytes: &[u8], inspection: &ClientExeInspection) -> eyre::Result<LoginCryptInfo> {
    eyre::ensure!(inspection.family != ClientFamily::Classic, "Classic login master-key extraction is not supported");
    let site = inspection.encryption.as_ref().and_then(|sites| {
        sites.sites.iter().find(|site| site.kind == CipherSiteKind::LoginCipher)
    }).ok_or_else(|| eyre!("the login cipher was not located uniquely; embedded keys cannot be extracted"))?;
    let keys = if let Some(keys) = legacy_login_keys(bytes, site.offset) {
        keys
    } else {
        let block = bytes.get(site.offset..site.offset + KEY_STEP.len())
            .ok_or_else(|| eyre!("the executable ends inside the login key step"))?;
        eyre::ensure!(block.iter().zip(KEY_STEP).all(|(byte, expected)| *expected == ANY || u16::from(*byte) == *expected),
            "the login key-step instructions are not recognized; keys are not guessed from the PE version");
        LoginMasterKeys {
            key_1: read_u32(block, MASTER_KEY_1_OFFSET)?,
            key_2: read_u32(block, MASTER_KEY_2_OFFSET)?,
            first_round: read_u32(block, FIRST_ROUND_KEY_OFFSET)?,
        }
    };
    let (code, _) = code_section(bytes, &inspection.pe)?;
    let versions: Vec<_> = code.windows(SEED_WRITER_PROLOGUE.len()).enumerate()
        .filter(|(_, window)| *window == SEED_WRITER_PROLOGUE)
        .filter_map(|(at, _)| protocol_version(code, at)).collect();
    Ok(LoginCryptInfo {
        keys,
        cipher_offset: site.offset,
        cipher_removed: site.removed,
        protocol_version: (versions.len() == 1).then(|| versions[0]),
        protocol_version_matches: versions.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::facet_dimensions::tests::synthetic_image_with;

    const FILLER: u8 = 0xcc;
    const FILLER_SIZE: usize = 32;
    const STOCK_XOR: [u8; 2] = [0x30, 0x08];
    const PATCHED_XOR: [u8; 2] = [0x90, 0x90];
    const KR_VERSION: ProtocolVersion = ProtocolVersion([66, 55, 52, 0]);
    const EC_VERSION: ProtocolVersion = ProtocolVersion([67, 0, 102, 0]);
    const KR_KEYS: LoginMasterKeys = LoginMasterKeys {
        key_1: 0x5b6b_4d6c, key_2: 0xd3d5_1816, first_round: 0x5b6b_4d6c,
    };

    fn seed_writer(version: ProtocolVersion) -> Vec<u8> {
        let mut bytes = SEED_WRITER_PROLOGUE.to_vec();
        bytes.extend(KR_SEED_SETUP);
        bytes.extend(SEED_STORE);
        for (part, stack) in version.0.into_iter().zip(VERSION_STACK_OFFSETS) {
            if part == 0 {
                bytes.extend(MOV_STACK_ZERO_EBX);
                bytes.push(stack);
            } else {
                bytes.extend(MOV_STACK_IMMEDIATE);
                bytes.push(stack);
                bytes.extend(part.to_le_bytes());
            }
        }
        bytes.push(OPCODE_CALL_REL32);
        bytes.extend([0; DWORD_SIZE]);
        bytes
    }

    fn image(keys: LoginMasterKeys, patched: bool, versions: &[ProtocolVersion], corrupt_step: bool) -> Vec<u8> {
        let pattern = CipherSiteKind::LoginCipher.pattern();
        let mut code = vec![FILLER; FILLER_SIZE];
        code.extend(&pattern.stock_code(0)[..pattern.patch_at]);
        let mut step: Vec<u8> = KEY_STEP.iter().map(|byte| u8::try_from(*byte).unwrap_or(0)).collect();
        step[..STOCK_XOR.len()].copy_from_slice(if patched { &PATCHED_XOR } else { &STOCK_XOR });
        for (at, value) in [(FIRST_ROUND_KEY_OFFSET, keys.first_round), (MASTER_KEY_1_OFFSET, keys.key_1), (MASTER_KEY_2_OFFSET, keys.key_2)] {
            step[at..at + DWORD_SIZE].copy_from_slice(&value.to_le_bytes());
        }
        if corrupt_step {
            *step.last_mut().unwrap() ^= 1;
        }
        code.extend(step);
        for &version in versions {
            code.extend([FILLER; FILLER_SIZE]);
            code.extend(seed_writer(version));
        }
        synthetic_image_with(code, Vec::new(), false)
    }

    #[test]
    fn version_formula_matches_embedded_kr_and_ec_keys_and_ignores_fourth_component() {
        assert_eq!(version_master_keys(KR_VERSION), (KR_KEYS.key_1, KR_KEYS.key_2));
        assert_eq!(version_master_keys(EC_VERSION), (0x1b92_eb9d, 0xb91e_227f));
        assert_eq!(version_master_keys(ProtocolVersion([66, 55, 52, u32::MAX])), version_master_keys(KR_VERSION));
    }

    #[test]
    fn embedded_keys_and_wire_version_are_read_with_either_login_xor_form() {
        for patched in [false, true] {
            let image = image(KR_KEYS, patched, &[KR_VERSION], false);
            let inspection = ClientExeInspection::inspect(&image).unwrap();
            let info = inspect_login_crypt(&image, &inspection).unwrap();
            assert_eq!(info.keys, KR_KEYS);
            assert_eq!(info.cipher_removed, patched);
            assert_eq!(info.protocol_version, Some(KR_VERSION));
            assert_eq!(info.protocol_version_matches, 1);
        }
    }

    #[test]
    fn embedded_constants_are_not_replaced_by_a_version_formula() {
        let keys = LoginMasterKeys { key_1: 0x1122_3344, key_2: 0x5566_7788, first_round: 0x99aa_bbcc };
        let image = image(keys, false, &[KR_VERSION], false);
        let info = inspect_login_crypt(&image, &ClientExeInspection::inspect(&image).unwrap()).unwrap();
        assert_eq!(info.keys, keys);
        assert_ne!(version_master_keys(KR_VERSION), (keys.key_1, keys.key_2));
    }

    #[test]
    fn missing_or_ambiguous_wire_versions_do_not_hide_the_real_keys() {
        for versions in [&[][..], &[KR_VERSION, EC_VERSION][..]] {
            let image = image(KR_KEYS, false, versions, false);
            let info = inspect_login_crypt(&image, &ClientExeInspection::inspect(&image).unwrap()).unwrap();
            assert_eq!(info.keys, KR_KEYS);
            assert_eq!(info.protocol_version, None);
            assert_eq!(info.protocol_version_matches, versions.len());
        }
    }

    #[test]
    fn unknown_key_steps_and_classic_are_refused() {
        let image = image(KR_KEYS, false, &[KR_VERSION], true);
        assert!(inspect_login_crypt(&image, &ClientExeInspection::inspect(&image).unwrap()).unwrap_err().to_string().contains("not recognized"));
        let classic = crate::client::classic_facets::tests::synthetic_classic(6, 2);
        assert!(inspect_login_crypt(&classic, &ClientExeInspection::inspect(&classic).unwrap()).unwrap_err().to_string().contains("Classic"));
    }

    #[test]
    fn earlier_kr_keys_are_actual_immediates_without_a_guessed_version() {
        let pattern = &super::super::kr_legacy_login::LOGIN_CIPHER;
        let keys = LoginMasterKeys { key_1: 0xd09c_9589, key_2: 0x1bd2_6d6b, first_round: 0xd09c_9589 };
        for removed in [false, true] {
            let mut code = pattern.stock_code(FILLER);
            for (at, value) in [(super::super::kr_legacy_login::KEY_1_OFFSET, keys.key_1), (super::super::kr_legacy_login::KEY_2_OFFSET, keys.key_2)] {
                code[at..][..DWORD_SIZE].copy_from_slice(&value.to_le_bytes());
            }
            if removed {
                code[pattern.patch_at..][..pattern.patched.len()].copy_from_slice(pattern.patched);
            }
            let bytes = synthetic_image_with(code, Vec::new(), false);
            let info = inspect_login_crypt(&bytes, &ClientExeInspection::inspect(&bytes).unwrap()).unwrap();
            assert_eq!(info.keys, keys);
            assert_eq!(info.cipher_removed, removed);
            assert_eq!(info.protocol_version, None);
            assert_eq!(info.protocol_version_matches, 0);
        }
    }
}
