//! The network ciphers of every Enhanced Client and Kingdom Reborn build under the client roots: each cipher's
//! pattern matches exactly once and every build ships encrypted (Kingdom Reborn's at the traced offsets), and a
//! copy patched with a login address and no encryption differs from its executable only at the login sites, the
//! cipher bytes and the PE checksum, and reads back as plaintext.
//!
//! The roots come from `UO_CLIENT_ROOTS` (searched recursively, only read); copies go to `std::env::temp_dir()`
//! (set `TMPDIR`) and are deleted afterwards.

use crate::common;

use uocf::client::encryption::{CipherSiteKind, EncryptionPart, EncryptionState};
use uocf::client::facet_dimensions::{ClientExeInspection, ClientFamily};
use uocf::client::login::{LoginHost, LoginServerEdit};
use uocf::client::patch::{dry_run_client_exe, patch_client_exe, write_patched_client_exe, ExeEdit, ExePatch};

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
