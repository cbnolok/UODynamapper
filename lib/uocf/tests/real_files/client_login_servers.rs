//! The login server of every client under the client roots: each client folder's configuration is read (Classic
//! uo.cfg and its login file; Enhanced Client and Kingdom Reborn: the built-in address, the XML files listed as
//! unused), the login sites of every Enhanced Client build are located (each pattern once, one host slot per random
//! draw), Kingdom Reborn has its single built-in address, a login-patched copy differs from its executable only in
//! the host slots, ports and PE checksum, and `UOSA_eclo.exe` decodes to the ECLO-patched address.
//!
//! The roots come from `UO_CLIENT_ROOTS` (searched recursively, only read); copies go to `std::env::temp_dir()`
//! (set `TMPDIR`) and are deleted afterwards.

use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use uocf::client_config::login_servers::{read_login_servers, LoginEntrySource, LoginFileRole};
use uocf::client_exe::facet_dimensions::{ClientExeInspection, ClientFamily};
use uocf::client_exe::login::{LoginHost, LOGIN_HOST_SLOT_SIZE, STOCK_LOGIN_HOSTS, STOCK_LOGIN_PORTS};
use uocf::client_exe::patch::{
    dry_run_client_exe, plan_patch, write_patched_client_exe, ExeEdit, ExePatch, PatchWarningKind,
};

use super::client_exe_facet_dimensions::{allowed_ranges, client_exes, exe_kind, ExeKind};

/// The ECLO-patched build and the address its patch wrote to every host slot and port.
const ECLO_EXE_NAME: &str = "uosa_eclo.exe";
const ECLO_HOST: &str = "127.0.0.1";
const ECLO_PORT: u16 = 2593;
/// The address of the patched copies: short enough for every host slot (the shortest is 12 bytes).
const PATCHED_HOST: &str = "10.0.0.2";
const PATCHED_PORT: u16 = 2594;
/// The second patch of a chain, on the copy.
const CHAINED_HOST: &str = "10.0.0.3";
const CHAINED_PORT: u16 = 2595;
/// Enhanced Client builds draw between two public servers (TOL and later) or have one (older builds).
const MAX_PUBLIC_SERVERS: usize = 2;

fn read_inspection(path: &Path) -> ClientExeInspection {
    ClientExeInspection::inspect(&std::fs::read(path).unwrap())
        .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()))
}

fn is_eclo(path: &Path) -> bool {
    path.file_name().unwrap_or_default().to_string_lossy().eq_ignore_ascii_case(ECLO_EXE_NAME)
}

/// The Enhanced Client and Kingdom Reborn builds under the roots.
fn ec_kr_exes() -> Vec<PathBuf> {
    client_exes().into_iter().filter(|exe| matches!(exe_kind(exe), ExeKind::Enhanced | ExeKind::KingdomReborn)).collect()
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_client_folder_tells_where_its_login_server_comes_from() {
    let mut folders = BTreeMap::new();
    for exe in client_exes() {
        let family = match exe_kind(&exe) {
            ExeKind::Enhanced => ClientFamily::Enhanced,
            ExeKind::KingdomReborn => ClientFamily::KingdomReborn,
            ExeKind::Classic => ClientFamily::Classic,
            ExeKind::Other => continue,
        };
        folders.entry(exe.parent().unwrap().to_path_buf()).or_insert(family);
    }
    assert!(!folders.is_empty());
    for (dir, family) in folders {
        let report = read_login_servers(&dir, family).unwrap_or_else(|error| panic!("{}: {error:#}", dir.display()));
        println!("{} ({}):", dir.display(), family.label());
        for file in report.files.iter().filter(|file| file.found) {
            println!("  file {:?} {} read={}", file.role, file.path.display(), file.read_by_client);
        }
        for entry in &report.entries {
            println!("  {:?} {:?} {:?}", entry.source, entry.name, entry.address);
        }
        for entry in &report.unused_entries {
            println!("  unused {:?} {:?} {:?}", entry.source, entry.name, entry.address);
        }
        match family {
            ClientFamily::Classic => {
                let login = report.files.iter().find(|file| file.role == LoginFileRole::LoginCfg).unwrap();
                if login.found {
                    assert!(report.entries.iter().any(|entry| entry.source == LoginEntrySource::Cfg(
                        uocf::client_config::login_servers::CfgServerKind::Primary
                    )), "{}: a login file without a LoginServer line", dir.display());
                    assert!(report.entries.iter().all(|entry| entry.address.is_ok()), "{}", dir.display());
                }
            }
            ClientFamily::Enhanced | ClientFamily::KingdomReborn => {
                assert!(report.files.iter().filter(|file| file.role == LoginFileRole::LoginServersXml).all(|file| !file.read_by_client));
                assert!(report.unused_entries.iter().all(|entry| entry.source == LoginEntrySource::Xml));
            }
        }
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_enhanced_client_build_has_its_login_sites_and_kingdom_reborn_its_address() {
    let exes = ec_kr_exes();
    assert!(!exes.is_empty(), "no UOSA*.exe or UOKR.exe under the roots");
    println!("| exe | fixed | hosts (slot file offset, capacity) | ports (imm16 file offset) |");
    for exe in exes {
        let inspection = read_inspection(&exe);
        let login = inspection.login.as_ref().unwrap_or_else(|| panic!("{}: login sites not located", exe.display()));
        let hosts: Vec<String> =
            login.hosts.iter().map(|slot| format!("{:#x} {} ({})", slot.offset, slot.host, slot.capacity)).collect();
        let ports: Vec<String> = login.ports.iter().map(|site| format!("{:#x} {}", site.offset, site.port)).collect();
        println!(
            "| {} | {:#x} {}:{} | {} | {} |",
            exe.display(),
            login.fixed.offset,
            login.fixed.ip,
            login.fixed.port,
            hosts.join(", "),
            ports.join(", ")
        );
        match inspection.family {
            ClientFamily::KingdomReborn => {
                assert!(login.hosts.is_empty() && login.ports.is_empty(), "{}", exe.display());
                assert_eq!(login.fixed.port, STOCK_LOGIN_PORTS[0]);
            }
            ClientFamily::Enhanced => {
                assert!((1..=MAX_PUBLIC_SERVERS).contains(&login.hosts.len()), "{}", exe.display());
                assert_eq!(login.hosts.len(), login.ports.len(), "{}: one port per host", exe.display());
                for slot in &login.hosts {
                    assert_eq!(slot.references, 1, "{}: host slot {:#x}", exe.display(), slot.va);
                    assert!(slot.host.len() < slot.capacity && slot.capacity <= LOGIN_HOST_SLOT_SIZE);
                }
                if is_eclo(&exe) {
                    assert!(!login.is_stock(), "{}: ECLO-patched", exe.display());
                    assert!(login.public_servers().iter().all(|(host, port)| host == ECLO_HOST && *port == ECLO_PORT));
                } else {
                    assert!(login.is_stock(), "{}: {:?}", exe.display(), login.values());
                    assert!(login.hosts.iter().all(|slot| STOCK_LOGIN_HOSTS.contains(&slot.host.as_str())));
                }
            }
            ClientFamily::Classic => unreachable!(),
        }
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn login_patched_copies_change_only_the_host_slots_ports_and_checksum() {
    let login = |host: &str, port: u16| ExeEdit::LoginServer { host: LoginHost::new(host).unwrap(), port };
    for exe in ec_kr_exes() {
        let original = std::fs::read(&exe).unwrap();
        let before = ClientExeInspection::inspect(&original).unwrap();
        let edits = vec![login(PATCHED_HOST, PATCHED_PORT)];
        let patch = ExePatch { edits: edits.clone() };
        let dir = common::temp_dir("client_login_patch");
        let output = dir.join(exe.file_name().unwrap());
        if before.family == ClientFamily::KingdomReborn {
            let error = plan_patch(&original, &patch, None).unwrap_err();
            assert!(format!("{error:#}").contains("Enhanced Client only"), "{error:#}");
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        }
        let plan = dry_run_client_exe(&exe, &patch).unwrap();
        let patched = write_patched_client_exe(&exe, &output, &patch).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), original, "the input is unchanged");
        assert_eq!(plan.changes, patched.changes, "the dry run is the write");
        assert_eq!(
            patched.warnings.iter().any(|warning| warning.kind == PatchWarningKind::Login),
            is_eclo(&exe),
            "{}: only a build not at the stock address warns: {:?}",
            exe.display(),
            patched.warnings
        );

        let copy = std::fs::read(&output).unwrap();
        let allowed = allowed_ranges(&before, &edits);
        let changed: Vec<usize> = (0..copy.len()).filter(|&offset| copy[offset] != original[offset]).collect();
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|offset| allowed.iter().any(|range| range.contains(offset))), "{}: {changed:x?}", exe.display());
        let after = ClientExeInspection::inspect(&copy).unwrap();
        assert!(after.checksum_valid(), "{}", exe.display());
        assert_eq!(after.tables, before.tables, "the facet tables are untouched");
        let sites = after.login.as_ref().unwrap();
        assert!(!sites.is_stock());
        assert!(sites.public_servers().iter().all(|(host, port)| host == PATCHED_HOST && *port == PATCHED_PORT));
        assert_eq!(sites.fixed, before.login.as_ref().unwrap().fixed, "the internal address is not patched");

        // The copy continues the chain through its manifest.
        let second_dir = common::temp_dir("client_login_chain");
        let second = second_dir.join(exe.file_name().unwrap());
        let chained = write_patched_client_exe(&output, &second, &ExePatch { edits: vec![login(CHAINED_HOST, CHAINED_PORT)] }).unwrap();
        assert!(chained.warnings.iter().all(|warning| warning.kind != PatchWarningKind::Login), "{:?}", chained.warnings);
        let chained = chained.after.login.unwrap();
        assert!(chained.public_servers().iter().all(|(host, port)| host == CHAINED_HOST && *port == CHAINED_PORT));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&second_dir);
    }
}
