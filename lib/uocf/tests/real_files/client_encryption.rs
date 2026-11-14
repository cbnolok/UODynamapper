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
use uocf::client::facet_dimensions::{ClientExeInspection, ClientFamily};
use uocf::client::login::{LoginHost, LoginServerEdit};
use uocf::client::patch::{dry_run_client_exe, patch_client_exe, read_manifest, write_patched_client_exe, ExeEdit, ExePatch};

use super::client_exe_facet_dimensions::{allowed_ranges, client_exes, exe_kind, ExeKind};

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

struct AuditedKrProfile {
    version: &'static str,
    sha256: &'static str,
    connect_va: u32,
    handshake_va: u32,
}

/// Audited complete game-connect functions and patch sites; these are independent of the locator's profiles.
const AUDITED_KR_PROFILES: [AuditedKrProfile; 9] = [
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
    let patch = ExePatch { edits: vec![ExeEdit::NoEncryption] };
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
        let function_offset = handshake.offset - handshake.kind.pattern().patch_at;
        assert_eq!(before.pe.offset_to_va(function_offset), Some(profile.connect_va));

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
        assert_eq!(std::fs::read(&path).unwrap(), original, "the source is unchanged");
    }
    assert!(seen.iter().all(|seen| *seen), "not all audited KR profiles were supplied: {seen:?}");
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
