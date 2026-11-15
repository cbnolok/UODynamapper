//! The network ciphers of every Enhanced Client and Kingdom Reborn build under the client roots: each cipher's
//! pattern matches exactly once and every build ships encrypted (Kingdom Reborn's at the traced offsets), and a
//! copy patched with a login address and no encryption differs from its executable only at the login sites, the
//! cipher bytes and the PE checksum, and reads back as plaintext.
//!
//! The roots come from `UO_CLIENT_ROOTS` (searched recursively, only read); copies go to `std::env::temp_dir()`
//! (set `TMPDIR`) and are deleted afterwards.
//! The audited KR profile test instead reads only the executable paths in `UO_KR_EXES`; the previous-bypass
//! upgrade test reads `UO_KR_PREVIOUS_EXE` and its hash-matching manifest. Neither searches their folders.

use crate::common;

use uocf::client::encryption::{CipherSiteKind, EncryptionPart, EncryptionState};
use uocf::client::crypt_keys::inspect_login_crypt;
use uocf::client::facet_dimensions::{ClientExeInspection, ClientFamily};
use uocf::client::login::{LoginHost, LoginServerEdit};
use uocf::client::patch::{dry_run_client_exe, patch_client_exe, read_manifest, write_patched_client_exe, ExeEdit, ExePatch};
use uocf::client::pe::{PeImage, pe_checksum, OPTIONAL_CHECKSUM_OFFSET};

use super::client_exe_facet_dimensions::{allowed_ranges, client_exes, exe_kind, ExeKind};

/// Hashes and XOR offsets are regression evidence only, never a production locator allowlist.
#[test]
#[ignore = "requires UO_EARLY_CLASSIC_EXES naming the eight audited early Classic executables"]
fn early_classic_shared_encryption_round_trips_all_audited_executables() {
    const CASES: [(&str, usize, u32, u32); 8] = [
        ("e5ebe07148034b8d7f47e2828075ce93ef779c53fb3acf8edc7cdca5aac35195", 0x9790a, 0xf1a372d5, 0x3a1fd527),
        ("35c4903d50c56c5f8f1de5e5fb72ec310d0e5b074656be31a20ebf82946d6f02", 0x693f4, 0xf1a372d5, 0x3a1fd527),
        ("b36482dc091c0c0d333636271766a5ce8ef4be4002a2c7d44ff96b47fe2f09c4", 0x87784, 0x38ecedac, 0x025720c6),
        ("dc2c53d70ff1595d2a6f16a50794a4cd42c9a1823879a9fc7e46d15bb7297f57", 0x88a54, 0x383477bc, 0x02345cc6),
        ("8363693e3d8634985aa7ed2599566551423990fce8bc337af3bb1fe3ab2f059d", 0x889e4, 0x383477bc, 0x02345cc6),
        ("9dfe78e72ca03acb71c753729bbf8d3a4f9b7c71d74b8fabdc54c8d58ec40aa3", 0x88a04, 0x383477bc, 0x02345cc6),
        ("07944a089187e5fbcc8c14196898487135683fab0d1c101f4549d31273a6d601", 0x88a44, 0x383477bc, 0x02345cc6),
        ("60d62325b911b08bcf672f207cfba3f0bc35136a40e4318881b66630bb238ccb", 0x88748, 0x387fc5cc, 0x021510c6),
    ];
    let paths: Vec<_> = std::env::split_paths(&std::env::var_os("UO_EARLY_CLASSIC_EXES")
        .expect("set UO_EARLY_CLASSIC_EXES")).collect();
    assert_eq!(paths.len(), CASES.len());
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        let bytes = std::fs::read(&path).unwrap();
        let inspection = ClientExeInspection::inspect(&bytes).unwrap();
        let &(hash, offset, hi, lo) = CASES.iter().find(|case| case.0 == inspection.sha256)
            .unwrap_or_else(|| panic!("unexpected fixture: {}", path.display()));
        assert!(seen.insert(hash), "duplicate fixture");
        assert_eq!(inspection.family, ClientFamily::Classic);
        assert!(inspection.table().is_none() && inspection.values().facets.is_empty());
        let crypt = inspect_login_crypt(&bytes, &inspection).unwrap();
        assert_eq!((crypt.keys.key_1, crypt.keys.key_2), (hi, lo));
        assert_eq!(crypt.cipher_offset, offset);
        assert!(crypt.protocol_version.is_none() && !crypt.cipher_removed);
        let patch = ExePatch { edits: vec![ExeEdit::NoEncryption] };
        let patched = patch_client_exe(&bytes, &patch, None).unwrap();
        assert_eq!(patched.after.values().encryption, Some(EncryptionState::Removed));
        assert!(patched.after.checksum_valid());
        assert_eq!(&patched.bytes[offset..offset + 2], &[0x90, 0x90]);
        let checksum = inspection.pe.checksum_offset..inspection.pe.checksum_offset + WORD_SIZE;
        for (at, (old, new)) in bytes.iter().zip(&patched.bytes).enumerate() {
            assert!(old == new || (offset..offset + 2).contains(&at) || checksum.contains(&at),
                "unexpected change at {at:#x}");
        }
        let after = inspect_login_crypt(&patched.bytes, &patched.after).unwrap();
        assert_eq!(after.keys, crypt.keys);
        assert!(after.cipher_removed);
        assert!(patch_client_exe(&patched.bytes, &patch, Some(&patched.manifest)).is_err());
        for edit in [ExeEdit::NoLoginEncryption, ExeEdit::NoGameEncryption] {
            let error = patch_client_exe(&bytes, &ExePatch { edits: vec![edit] }, None).unwrap_err();
            assert!(error.to_string().contains("shares one cipher"));
        }
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "source changed");
    }
}

/// The traced UOKR.exe: the login cipher's `XOR [EAX],CL` and the key exchange adapter's `JZ`.
const KR_LOGIN_CIPHER_OFFSET: usize = 0x1d_d59a;
const KR_KEY_EXCHANGE_CIPHER_OFFSET: usize = 0x1d_a198;
const KR_GAME_LOGIN_HANDSHAKE_OFFSET: usize = 0x1b_5f9f;
/// The login address of the patched copies.
const PATCHED_HOST: &str = "127.0.0.1";
const PATCHED_PORT: u16 = 2593;
const SCOPED_EDITS: [(EncryptionPart, ExeEdit); 3] = [
    (EncryptionPart::Login, ExeEdit::NoLoginEncryption),
    (EncryptionPart::Game, ExeEdit::NoGameEncryption),
    (EncryptionPart::KrHandshake, ExeEdit::BypassKrHandshake),
];
const KR_EXES_ENV: &str = "UO_KR_EXES";
const KR_PREVIOUS_EXE_ENV: &str = "UO_KR_PREVIOUS_EXE";
const WORD_SIZE: usize = size_of::<u32>();
const REBASE_DELTA: u32 = 0x0010_0000;
const PE_IMAGE_BASE_OFFSET: usize = 28;
const BASE_RELOCATION_HEADER_SIZE: usize = WORD_SIZE * 2;
const BASE_RELOCATION_ENTRY_SIZE: usize = size_of::<u16>();
const BASE_RELOCATION_TYPE_SHIFT: u32 = 12;
const BASE_RELOCATION_OFFSET_MASK: u16 = 0x0fff;
const BASE_RELOCATION_HIGHLOW: u16 = 3;
const FIXED_FILE_INFO_SIGNATURE: u32 = 0xfeef_04bd;
const FIXED_FILE_VERSION_OFFSET: usize = WORD_SIZE * 2;
const UNTESTED_VERSION_MAJOR: u32 = 2;
const UNTESTED_VERSION_MINOR: u32 = 99;

struct AuditedKrProfile {
    version: &'static str,
    sha256: &'static str,
    connect_va: u32,
    handshake_va: u32,
}

/// Audited complete game-connect functions and patch sites; these are independent of the locator's profiles.
const AUDITED_KR_PROFILES: [AuditedKrProfile; 19] = [
    AuditedKrProfile {
        version: "2.45.3.13",
        sha256: "e94190c52febc13585a159f44f976686835ee4140197d0363a4d00ce6ab6e25b",
        connect_va: 0x004fee41,
        handshake_va: 0x004fee9d,
    },
    AuditedKrProfile {
        version: "2.45.4.4",
        sha256: "d2eb90b428d29d83a235fc7d553a816128d95c780e4707b0a19a2b4617e9363b",
        connect_va: 0x00504766,
        handshake_va: 0x005047c2,
    },
    AuditedKrProfile {
        version: "2.45.5.6",
        sha256: "9d7e8257d2e5e7309ec7a8e3fc86863db087ce18228e6ebb8b7589695555e0e5",
        connect_va: 0x00509df4,
        handshake_va: 0x00509e50,
    },
    AuditedKrProfile {
        version: "2.45.6.3",
        sha256: "aebb584d6bcf73ec8b7821606fbe11832d223b3554bfe94a57507a226a48833e",
        connect_va: 0x0051323d,
        handshake_va: 0x00513299,
    },
    AuditedKrProfile {
        version: "2.46.0.3",
        sha256: "619c458eb70175c7ca389e2c31415d262155d188d7388608eb86efd89140667c",
        connect_va: 0x005a_a350,
        handshake_va: 0x005a_a3af,
    },
    AuditedKrProfile {
        version: "2.46.1.6",
        sha256: "cc664cc8550bf40877b597889e893409924e3a42cf78fb177ba4f68fb19a76f7",
        connect_va: 0x005b_08e0,
        handshake_va: 0x005b_093f,
    },
    AuditedKrProfile {
        version: "2.46.1.9",
        sha256: "fb5c60cd6ea81969b30a54b1978ce47c8c7a1d200a736f321602c61517c051ec",
        connect_va: 0x005b_08e0,
        handshake_va: 0x005b_093f,
    },
    AuditedKrProfile {
        version: "2.47.0.6",
        sha256: "36ea9fadbbb2d4c8e1180b7845aaba74d3bf5e7bd654735d5f023de69ade3059",
        connect_va: 0x005b_1080,
        handshake_va: 0x005b_10df,
    },
    AuditedKrProfile {
        version: "2.47.1.6",
        sha256: "3fde36e0ffa1920e5ac12c58e62d15647cb015b9b66ce1e31fe1d50678c2bd5b",
        connect_va: 0x005b_3240,
        handshake_va: 0x005b_329f,
    },
    AuditedKrProfile {
        version: "2.47.1.10",
        sha256: "fc5eea0a9aa99eb3984cf7d24406799b19e82567861f44dd0385cabae44e7a4e",
        connect_va: 0x005b_32b0,
        handshake_va: 0x005b_330f,
    },
    AuditedKrProfile {
        version: "2.48.0.3",
        sha256: "d88f0cd5a31c3cafe754b52424f615ab5c7b5f69d464fbb510e842e9610c43ea",
        connect_va: 0x005b_aeb0,
        handshake_va: 0x005b_af0f,
    },
    AuditedKrProfile {
        version: "2.48.0.7",
        sha256: "c9b4f6ce4a0763ca16bb3a1133d56a332be93e644db5c441506281632c57d77a",
        connect_va: 0x005b_ae70,
        handshake_va: 0x005b_aecf,
    },
    AuditedKrProfile {
        version: "2.50.0.3",
        sha256: "9547f6ccbaaca0a3695a7cbe31ec9a9e62dda4ac8ee627714061d38ecc70e8e8",
        connect_va: 0x005b_abe0,
        handshake_va: 0x005b_ac3f,
    },
    AuditedKrProfile {
        version: "2.51.0.3",
        sha256: "5f70b0aad9ccdafaea04b0883610de2e8a75937806b53faaa54cb03eb346a48a",
        connect_va: 0x005b_abe0,
        handshake_va: 0x005b_ac3f,
    },
    AuditedKrProfile {
        version: "2.52.0.4",
        sha256: "f4876dbbb5f4911cce12662e8e5b67d715946e33d0c1aff4ad2cd5f3c708e6ad",
        connect_va: 0x005b_abe0,
        handshake_va: 0x005b_ac3f,
    },
    AuditedKrProfile {
        version: "2.53.0.2",
        sha256: "dc498608e3c8dec517d2435ad5d442816214b33c430077a2a815fd761daa02e6",
        connect_va: 0x005b_abe0,
        handshake_va: 0x005b_ac3f,
    },
    AuditedKrProfile {
        version: "2.54.0.7",
        sha256: "35f6612cacd3c5f0f1cef6310ff1ceaf04e647ad8b0c0cfbf64684e1d81dc984",
        connect_va: 0x005b_abe0,
        handshake_va: 0x005b_ac3f,
    },
    AuditedKrProfile {
        version: "2.57.0.17",
        sha256: "b6ab0422c25e22c3e97d6220d3284723f2fb44039c4a4e7a0c3b7cc10acb1715",
        connect_va: 0x005b_6b60,
        handshake_va: 0x005b_6bbf,
    },
    AuditedKrProfile {
        version: "2.58.0.6",
        sha256: "1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43",
        connect_va: 0x005b_6b40,
        handshake_va: 0x005b_6b9f,
    },
];

#[test]
#[ignore = "needs every audited KR executable: set UO_KR_EXES and run with --ignored"]
fn every_audited_kr_executable_patches_at_its_verified_profile() {
    let paths = std::env::var_os(KR_EXES_ENV).unwrap_or_else(|| panic!("set {KR_EXES_ENV}"));
    let mut seen = [false; AUDITED_KR_PROFILES.len()];
    for path in std::env::split_paths(&paths) {
        let original = std::fs::read(&path).unwrap();
        let before = ClientExeInspection::inspect(&original).unwrap();
        assert_eq!(before.family, ClientFamily::KingdomReborn, "{}", path.display());
        let index = AUDITED_KR_PROFILES.iter().position(|profile| profile.sha256 == before.sha256)
            .unwrap_or_else(|| panic!("{}: not an audited stock KR executable ({})", path.display(), before.sha256));
        seen[index] = true;
        let profile = &AUDITED_KR_PROFILES[index];
        assert_eq!(before.version.unwrap().file.to_string(), profile.version);
        let ciphers = before.encryption.as_ref().unwrap();
        assert!(ciphers.is_complete(), "{}: {}", path.display(), ciphers.describe_unlocated());
        assert_eq!(ciphers.values(), Some(EncryptionState::Encrypted));
        let kinds: Vec<_> = ciphers.sites.iter().map(|site| site.kind).collect();
        assert_eq!(kinds, CipherSiteKind::of_family(ClientFamily::KingdomReborn));
        let handshake = ciphers.sites.iter().find(|site| site.kind == CipherSiteKind::GameLoginHandshake).unwrap();
        assert_eq!(before.pe.offset_to_va(handshake.offset), Some(profile.handshake_va));
        let patch_at = usize::try_from(profile.handshake_va - profile.connect_va).unwrap();
        let function_offset = handshake.offset - patch_at;
        assert_eq!(before.pe.offset_to_va(function_offset), Some(profile.connect_va));

        let login = before.login.as_ref().expect("every audited KR executable has a supported login destination");
        assert!(login.is_stock(), "{}: native endpoint is recognized", path.display());
        let login_edit = LoginServerEdit::uniform(login, LoginHost::new(PATCHED_HOST).unwrap(), PATCHED_PORT);
        let patch = ExePatch { edits: vec![ExeEdit::LoginServer(login_edit), ExeEdit::NoEncryption] };
        let plan = dry_run_client_exe(&path, &patch).unwrap();
        let patched = patch_client_exe(&original, &patch, None).unwrap();
        assert_eq!(plan.apply(&original).unwrap(), patched.bytes, "{}: dry run differs", path.display());
        assert_eq!(plan.changes, patched.changes);
        let allowed = allowed_ranges(&before, &patch.edits);
        assert!(original.iter().zip(&patched.bytes).enumerate().all(|(at, (old, new))| {
            old == new || allowed.iter().any(|range| range.contains(&at))
        }), "{}: bytes changed outside the selected sites and checksum", path.display());
        assert!(patched.after.checksum_valid());
        assert_eq!(patched.after.encryption.as_ref().unwrap().values(), Some(EncryptionState::Removed));
        assert_eq!(patched.after.encryption.as_ref().unwrap().offsets(), ciphers.offsets());
        assert_eq!(patched.after.tables, before.tables);
        let after_login = patched.after.login.as_ref().unwrap();
        assert_eq!(after_login.values().hosts, [PATCHED_HOST]);
        assert_eq!(after_login.values().ports, [PATCHED_PORT]);
        for (at, expected) in login.fixed.port_writes(PATCHED_PORT) {
            assert_eq!(&patched.bytes[at..][..expected.len()], expected, "{}: the login port is changed", path.display());
        }
        let crypt_before = inspect_login_crypt(&original, &before).unwrap();
        let crypt_after = inspect_login_crypt(&patched.bytes, &patched.after).unwrap();
        assert_eq!(crypt_before.keys, crypt_after.keys, "{}: embedded keys stay intact", path.display());
        assert_eq!(crypt_before.protocol_version, crypt_after.protocol_version);
        assert!(crypt_after.cipher_removed);
        for (part, edit) in &SCOPED_EDITS {
            let scoped = ExePatch { edits: vec![edit.clone()] };
            let scoped_patch = patch_client_exe(&original, &scoped, None).unwrap();
            assert!(scoped_patch.after.checksum_valid());
            let allowed = allowed_ranges(&before, &scoped.edits);
            assert!(original.iter().zip(&scoped_patch.bytes).enumerate().all(|(at, (old, new))| {
                old == new || allowed.iter().any(|range| range.contains(&at))
            }), "{}: {part:?} touched unrelated bytes", path.display());
            let after = scoped_patch.after.encryption.as_ref().unwrap();
            assert!(after.is_complete());
            assert_eq!(after.values(), Some(EncryptionState::Partial));
            for site in &after.sites {
                assert_eq!(site.removed, site.kind.part() == *part, "{}: {part:?}", path.display());
            }
            assert_eq!(scoped_patch.after.login, before.login);
        }
        assert_eq!(std::fs::read(&path).unwrap(), original, "the source is unchanged");
    }
    assert!(seen.iter().all(|seen| *seen), "not all audited KR profiles were supplied: {seen:?}");
}

fn read_word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..][..WORD_SIZE].try_into().unwrap())
}

fn fix_checksum(bytes: &mut [u8]) {
    let pe = PeImage::parse(bytes).unwrap();
    let checksum = pe_checksum(bytes, pe.checksum_offset);
    bytes[pe.checksum_offset..][..WORD_SIZE].copy_from_slice(&checksum.to_le_bytes());
}

/// Simulate the PE loader: update the preferred base and every HIGHLOW target without changing RVAs.
fn rebase(bytes: &[u8], delta: u32) -> Vec<u8> {
    let pe = PeImage::parse(bytes).unwrap();
    let (rva, size) = pe.base_relocation_directory;
    assert!(rva != 0 && size != 0);
    let start = pe.va_to_offset(pe.image_base + rva).unwrap();
    let mut cursor = start;
    let end = start + size as usize;
    let mut targets = Vec::new();
    while cursor < end && bytes[cursor..end].iter().any(|byte| *byte != 0) {
        let page = read_word(bytes, cursor);
        let length = read_word(bytes, cursor + WORD_SIZE) as usize;
        assert!(length >= BASE_RELOCATION_HEADER_SIZE && cursor + length <= end);
        for at in (cursor + BASE_RELOCATION_HEADER_SIZE..cursor + length).step_by(BASE_RELOCATION_ENTRY_SIZE) {
            let word = u16::from_le_bytes(bytes[at..][..BASE_RELOCATION_ENTRY_SIZE].try_into().unwrap());
            match word >> BASE_RELOCATION_TYPE_SHIFT {
                0 => {}
                BASE_RELOCATION_HIGHLOW => {
                    let va = pe.image_base + page + u32::from(word & BASE_RELOCATION_OFFSET_MASK);
                    targets.push(pe.va_to_offset(va).unwrap());
                }
                kind => panic!("unexpected fixture relocation type {kind}"),
            }
        }
        cursor += length;
    }
    let mut rebased = bytes.to_vec();
    for at in targets {
        let value = read_word(bytes, at).wrapping_add(delta);
        rebased[at..][..WORD_SIZE].copy_from_slice(&value.to_le_bytes());
    }
    let base_at = pe.checksum_offset - OPTIONAL_CHECKSUM_OFFSET + PE_IMAGE_BASE_OFFSET;
    rebased[base_at..][..WORD_SIZE].copy_from_slice(&pe.image_base.checked_add(delta).unwrap().to_le_bytes());
    fix_checksum(&mut rebased);
    rebased
}

#[test]
#[ignore = "needs the audited KR files: set UO_KR_EXES and run with --ignored"]
fn dynamic_kr_patching_commutes_with_loader_rebasing_and_repairs_previous_relocations() {
    let paths = std::env::var_os(KR_EXES_ENV).unwrap_or_else(|| panic!("set {KR_EXES_ENV}"));
    let expected = AUDITED_KR_PROFILES.iter().find(|profile| profile.version == "2.46.0.3").unwrap();
    let original = std::env::split_paths(&paths).find_map(|path| {
        let bytes = std::fs::read(path).unwrap();
        let before = ClientExeInspection::inspect(&bytes).unwrap();
        (before.sha256 == expected.sha256).then_some(bytes)
    }).expect("supply the 2.46.0.3 relocation-bearing fixture");
    let before = ClientExeInspection::inspect(&original).unwrap();
    let patch = ExePatch { edits: vec![ExeEdit::BypassKrHandshake] };
    let patched = patch_client_exe(&original, &patch, None).unwrap();
    let after_rebase = rebase(&patched.bytes, REBASE_DELTA);
    let relocated_source = rebase(&original, REBASE_DELTA);
    let rebased_then_patched = patch_client_exe(&relocated_source, &patch, None).unwrap();
    assert_eq!(after_rebase, rebased_then_patched.bytes, "patching and loader rebasing produce identical code and relocation tables");
    assert_eq!(rebased_then_patched.before.pe.image_base, before.pe.image_base + REBASE_DELTA);
    assert!(rebased_then_patched.after.encryption.as_ref().unwrap().is_complete());

    let mut unknown_version = relocated_source;
    let pe = PeImage::parse(&unknown_version).unwrap();
    let resources = pe.va_to_offset(pe.image_base + pe.resource_directory.0).unwrap();
    let end = resources + pe.resource_directory.1 as usize;
    let signature = FIXED_FILE_INFO_SIGNATURE.to_le_bytes();
    let records: Vec<_> = unknown_version[resources..end].windows(WORD_SIZE).enumerate()
        .filter(|(_, value)| *value == signature).map(|(at, _)| resources + at).collect();
    assert_eq!(records.len(), 1);
    let version_at = records[0] + FIXED_FILE_VERSION_OFFSET;
    let version = (UNTESTED_VERSION_MAJOR << u16::BITS) | UNTESTED_VERSION_MINOR;
    unknown_version[version_at..][..WORD_SIZE].copy_from_slice(&version.to_le_bytes());
    unknown_version[version_at + WORD_SIZE..][..WORD_SIZE].fill(0);
    fix_checksum(&mut unknown_version);
    let unknown = patch_client_exe(&unknown_version, &patch, None).unwrap();
    assert_eq!(unknown.before.version.unwrap().file.to_string(), "2.99.0.0");
    assert!(unknown.after.encryption.as_ref().unwrap().is_complete());

    // Earlier patchers changed only the function and left the original relocation entries in place.
    let site = patched.after.encryption.as_ref().unwrap().sites.iter()
        .find(|site| site.kind == CipherSiteKind::GameLoginHandshake).unwrap();
    let mut stale = original.clone();
    stale[site.offset..][..site.patched().len()].copy_from_slice(site.patched());
    fix_checksum(&mut stale);
    let inspection = ClientExeInspection::inspect(&stale).unwrap();
    let stale_site = inspection.encryption.as_ref().unwrap().sites.iter()
        .find(|site| site.kind == CipherSiteKind::GameLoginHandshake).unwrap();
    assert!(stale_site.removed && stale_site.needs_relocation_repair());
    let repaired = patch_client_exe(&stale, &patch, None).unwrap();
    assert_eq!(repaired.bytes, patched.bytes);
    assert!(repaired.changes.iter().filter(|change| change.cause == uocf::client::patch::ChangeCause::Edit(0))
        .all(|change| change.field.contains("base relocation")));
    assert!(!repaired.after.encryption.as_ref().unwrap().sites.iter().any(|site| site.needs_relocation_repair()));
    assert!(patch_client_exe(&repaired.bytes, &patch, None).unwrap_err().to_string().contains("changes nothing"));
}

#[test]
#[ignore = "needs a previous KR bypass copy and sidecar: set UO_KR_PREVIOUS_EXE and run with --ignored"]
fn the_previous_kr_bypass_copy_upgrades_with_its_matching_sidecar() {
    let path = std::path::PathBuf::from(std::env::var_os(KR_PREVIOUS_EXE_ENV).unwrap_or_else(|| panic!("set {KR_PREVIOUS_EXE_ENV}")));
    let original = std::fs::read(&path).unwrap();
    let before = ClientExeInspection::inspect(&original).unwrap();
    let manifest = read_manifest(&path).unwrap().expect("the previous copy needs its sidecar");
    assert_eq!(manifest.patched_sha256, before.sha256, "the sidecar must record this exact copy");
    assert_eq!(before.family, ClientFamily::KingdomReborn);
    let ciphers = before.encryption.as_ref().unwrap();
    assert!(ciphers.is_complete(), "{}", ciphers.describe_unlocated());
    let handshake = ciphers.sites.iter().find(|site| site.kind == CipherSiteKind::GameLoginHandshake).unwrap();
    let previous = handshake.kind.pattern().previous.expect("the 2.58 profile has a previous callback");
    assert_eq!(handshake.stock(), previous, "the located stock bytes are the actual previous replacement");
    assert_eq!(&original[handshake.offset..][..previous.len()], previous);
    assert!(!handshake.removed, "the previous callback requires an explicit upgrade");
    let patch = ExePatch { edits: vec![ExeEdit::BypassKrHandshake] };
    let plan = dry_run_client_exe(&path, &patch).unwrap();
    let patched = patch_client_exe(&original, &patch, Some(&manifest)).unwrap();
    assert_eq!(plan.apply(&original).unwrap(), patched.bytes);
    assert_eq!(plan.changes, patched.changes);
    let allowed = allowed_ranges(&before, &patch.edits);
    assert!(original.iter().zip(&patched.bytes).enumerate().all(|(at, (old, new))| {
        old == new || allowed.iter().any(|range| range.contains(&at))
    }), "the upgrade changed bytes outside the handshake and checksum");
    assert!(patched.after.checksum_valid());
    assert_eq!(patched.after.encryption.as_ref().unwrap().values(), Some(EncryptionState::Removed));
    assert_eq!(patched.manifest.steps[..manifest.steps.len()], manifest.steps);
    assert_eq!(patched.manifest.steps.last().unwrap().edits, patch.edits);
    let writes: Vec<_> = patched.changes.iter().filter(|change| change.cause == uocf::client::patch::ChangeCause::Edit(0)).collect();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].old, previous);
    assert_eq!(writes[0].new, handshake.patched());
    assert_eq!(std::fs::read(&path).unwrap(), original, "the previous copy is unchanged");
    assert_eq!(read_manifest(&path).unwrap().unwrap(), manifest, "the original sidecar is unchanged");
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_client_patches_login_game_and_kr_handshake_independently() {
    let exes: Vec<_> =
        client_exes().into_iter().filter(|exe| matches!(exe_kind(exe), ExeKind::Enhanced | ExeKind::KingdomReborn)).collect();
    assert!(exes.iter().any(|exe| exe_kind(exe) == ExeKind::KingdomReborn), "no UOKR.exe under the roots");
    for exe in exes {
        let original = std::fs::read(&exe).unwrap();
        let before = ClientExeInspection::inspect(&original).unwrap();
        let ciphers = before.encryption.as_ref().unwrap();
        assert!(ciphers.is_complete(), "{}", exe.display());
        for (part, edit) in &SCOPED_EDITS {
            let selected = ciphers.for_part(*part);
            let patch = ExePatch { edits: vec![edit.clone()] };
            if selected.sites.is_empty() {
                assert!(patch_client_exe(&original, &patch, None).is_err(), "{}: {part:?}", exe.display());
                continue;
            }
            let patched = patch_client_exe(&original, &patch, None).unwrap();
            assert!(patched.after.checksum_valid(), "{}: {part:?}", exe.display());
            assert_eq!(patched.manifest.values.encryption, Some(EncryptionState::Partial));
            let allowed = allowed_ranges(&before, &patch.edits);
            assert!(original.iter().zip(&patched.bytes).enumerate().all(|(at, (old, new))| {
                old == new || allowed.iter().any(|range| range.contains(&at))
            }), "{}: {part:?} changed unrelated bytes", exe.display());
            for site in &patched.after.encryption.as_ref().unwrap().sites {
                assert_eq!(site.removed, site.kind.part() == *part, "{}: {part:?}: {:?}", exe.display(), site.kind);
            }
            assert_eq!(patched.manifest.steps.last().unwrap().edits, patch.edits);
        }
        assert_eq!(std::fs::read(&exe).unwrap(), original, "the input is unchanged");
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_enhanced_client_and_kingdom_reborn_build_patches_its_encryption_out() {
    let exes: Vec<_> =
        client_exes().into_iter().filter(|exe| matches!(exe_kind(exe), ExeKind::Enhanced | ExeKind::KingdomReborn)).collect();
    assert!(exes.iter().any(|exe| exe_kind(exe) == ExeKind::KingdomReborn), "no UOKR.exe under the roots");
    for exe in exes {
        let original = std::fs::read(&exe).unwrap();
        let before = ClientExeInspection::inspect(&original).unwrap();
        let ciphers = before.encryption.clone().unwrap();
        assert!(ciphers.is_complete(), "{}: not located: {}", exe.display(), ciphers.describe_unlocated());
        assert_eq!(ciphers.values(), Some(EncryptionState::Encrypted), "{}", exe.display());
        let kinds: Vec<CipherSiteKind> = ciphers.sites.iter().map(|site| site.kind).collect();
        assert_eq!(kinds, CipherSiteKind::of_family(before.family), "{}", exe.display());
        if before.family == ClientFamily::KingdomReborn {
            assert_eq!(ciphers.offsets(), [KR_LOGIN_CIPHER_OFFSET, KR_KEY_EXCHANGE_CIPHER_OFFSET, KR_GAME_LOGIN_HANDSHAKE_OFFSET]);
        }
        let sites = before.login.as_ref().unwrap();
        let login = LoginServerEdit::uniform(sites, LoginHost::new(PATCHED_HOST).unwrap(), PATCHED_PORT);
        let edits = vec![ExeEdit::LoginServer(login), ExeEdit::NoEncryption];
        let patch = ExePatch { edits: edits.clone() };

        let dir = common::temp_dir("client_encryption_patch");
        let output = dir.join(exe.file_name().unwrap());
        let plan = dry_run_client_exe(&exe, &patch).unwrap();
        let patched = write_patched_client_exe(&exe, &output, &patch).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), original, "the input is unchanged");
        assert_eq!(plan.changes, patched.changes, "the dry run is the write");
        let copy = std::fs::read(&output).unwrap();
        let allowed = allowed_ranges(&before, &edits);
        let changed: Vec<usize> = (0..copy.len()).filter(|&offset| copy[offset] != original[offset]).collect();
        assert!(changed.iter().all(|offset| allowed.iter().any(|range| range.contains(offset))), "{}: {changed:x?}", exe.display());
        for site in &ciphers.sites {
            assert_eq!(copy[site.offset..][..site.patched().len()], *site.patched(), "{}: {:?}", exe.display(), site.kind);
        }
        let after = ClientExeInspection::inspect(&copy).unwrap();
        assert!(after.checksum_valid(), "{}", exe.display());
        assert_eq!(after.encryption.as_ref().unwrap().values(), Some(EncryptionState::Removed));
        assert_eq!(after.encryption.as_ref().unwrap().offsets(), ciphers.offsets());
        assert_eq!(patched.manifest.values.encryption, Some(EncryptionState::Removed));
        assert_eq!(after.tables, before.tables, "the facet tables are untouched");
        println!("{}: {} bytes changed", exe.display(), changed.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
