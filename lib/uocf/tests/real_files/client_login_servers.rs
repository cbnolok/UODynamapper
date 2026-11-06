//! The login server of every client under the client roots: each client folder's configuration is read (Classic
//! uo.cfg and its login file; Enhanced Client and Kingdom Reborn: the built-in address), the login sites of every Enhanced Client build are located (each pattern once, one host slot per random
//! draw), Kingdom Reborn has its single built-in address, a login-patched copy (Enhanced Client: a different host
//! and port per site; Kingdom Reborn: its IPv4 address) differs from its executable only at the login sites and
//! the PE checksum, and `UOSA_eclo.exe` decodes to the ECLO-patched address. The survey prints each build's
//! version resource.
//!
//! The roots come from `UO_CLIENT_ROOTS` (searched recursively, only read); copies go to `std::env::temp_dir()`
//! (set `TMPDIR`) and are deleted afterwards.

use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use uocf::client::login_servers::{read_login_servers, LoginEntrySource, LoginFileRole};
use uocf::client::facet_dimensions::{ClientExeInspection, ClientFamily};
use uocf::client::login::{
    LoginHost, LoginServerEdit, LoginSites, LOGIN_HOST_SLOT_SIZE, MAX_LOGIN_HOST_LENGTH, STOCK_KR_LOGIN_ADDRESS,
    STOCK_LOGIN_HOSTS,
};
use uocf::client::patch::{
    dry_run_client_exe, plan_patch, write_patched_client_exe, ExeEdit, ExePatch, PatchWarningKind,
};

use super::client_exe_facet_dimensions::{allowed_ranges, client_exes, exe_kind, ExeKind};

/// The ECLO-patched build and the address its patch wrote to every host slot and port.
const ECLO_EXE_NAME: &str = "uosa_eclo.exe";
const ECLO_HOST: &str = "127.0.0.1";
const ECLO_PORT: u16 = 2593;
/// The hosts and ports of the patched copies, one per site in site order (a build with fewer sites takes the first
/// ones): short enough for every host slot (the shortest is 12 bytes); dotted IPv4, so Kingdom Reborn takes them.
const PATCHED_HOSTS: [&str; MAX_PUBLIC_SERVERS] = ["10.0.0.2", "10.0.0.4"];
const PATCHED_PORTS: [u16; MAX_PUBLIC_SERVERS] = [2594, 2596];
/// A host name, which Kingdom Reborn cannot take.
const NAMED_HOST: &str = "shard.example";
/// A host name of [`MAX_LOGIN_HOST_LENGTH`] letters, cut to a short slot's capacity (no room for the terminator).
const FULL_LENGTH_HOST: &str = "shardexamplenet";
/// The second patch of a chain, on the copy: one address on every site.
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
        match family {
            ClientFamily::Classic => {
                let login = report.files.iter().find(|file| file.role == LoginFileRole::LoginCfg).unwrap();
                if login.found {
                    assert!(report.entries.iter().any(|entry| entry.source == LoginEntrySource::Cfg(
                        uocf::client::login_servers::CfgServerKind::Primary
                    )), "{}: a login file without a LoginServer line", dir.display());
                    assert!(report.entries.iter().all(|entry| entry.address.is_ok()), "{}", dir.display());
                }
            }
            ClientFamily::Enhanced | ClientFamily::KingdomReborn => {
                assert_eq!(report.files.len(), 1);
                assert_eq!(report.files[0].role, LoginFileRole::Executable);
            }
        }
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_enhanced_client_build_has_its_login_sites_and_kingdom_reborn_its_address() {
    let exes = ec_kr_exes();
    assert!(!exes.is_empty(), "no UOSA*.exe or UOKR.exe under the roots");
    println!("| exe | version | fixed | hosts (slot file offset, capacity) | ports (imm16 file offset) |");
    for exe in exes {
        let inspection = read_inspection(&exe);
        let login = inspection.login.as_ref().unwrap_or_else(|| panic!("{}: login sites not located", exe.display()));
        let hosts: Vec<String> =
            login.hosts.iter().map(|slot| format!("{:#x} {} ({})", slot.offset, slot.host, slot.capacity)).collect();
        let ports: Vec<String> = login.ports.iter().map(|site| format!("{:#x} {}", site.offset, site.port)).collect();
        let version = inspection.version.map_or_else(|| "none".to_owned(), |version| version.to_string());
        println!(
            "| {} | {version} | {:#x} {}:{} | {} | {} |",
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
                assert!(login.uses_fixed_address() && login.is_stock(), "{}", exe.display());
                assert_eq!((login.fixed.ip, login.fixed.port), STOCK_KR_LOGIN_ADDRESS);
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

/// The per-site edit of the patched copies for `sites`.
fn per_site_edit(sites: &LoginSites) -> ExeEdit {
    let hosts = PATCHED_HOSTS.iter().take(sites.host_site_count()).map(|host| LoginHost::new(host).unwrap()).collect();
    let ports = PATCHED_PORTS.iter().take(sites.port_site_count()).copied().collect();
    ExeEdit::LoginServer(LoginServerEdit { hosts, ports })
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn login_patched_copies_change_only_the_login_sites_and_checksum() {
    let exes = ec_kr_exes();
    assert!(exes.iter().any(|exe| exe_kind(exe) == ExeKind::KingdomReborn), "no UOKR.exe under the roots");
    for exe in exes {
        let original = std::fs::read(&exe).unwrap();
        let before = ClientExeInspection::inspect(&original).unwrap();
        let before_sites = before.login.clone().unwrap();
        let edits = vec![per_site_edit(&before_sites)];
        let patch = ExePatch { edits: edits.clone() };
        let refused = |edit: LoginServerEdit| plan_patch(&original, &ExePatch { edits: vec![ExeEdit::LoginServer(edit)] }, None);
        if before.family == ClientFamily::KingdomReborn {
            let name = LoginServerEdit::uniform(&before_sites, LoginHost::new(NAMED_HOST).unwrap(), CHAINED_PORT);
            let error = refused(name).unwrap_err();
            assert!(format!("{error:#}").contains("dotted IPv4"), "{error:#}");
        }
        // A host as long as a short slot (no room for its terminator) is refused for that slot.
        for (index, slot) in before_sites.hosts.iter().enumerate().filter(|(_, slot)| slot.capacity <= MAX_LOGIN_HOST_LENGTH) {
            let mut edit = LoginServerEdit::uniform(&before_sites, LoginHost::new(PATCHED_HOSTS[0]).unwrap(), CHAINED_PORT);
            edit.hosts[index] = LoginHost::new(&FULL_LENGTH_HOST[..slot.capacity]).unwrap();
            let error = refused(edit).unwrap_err();
            assert!(format!("{error:#}").contains(&format!("slot {}", index + 1)), "{}: {error:#}", exe.display());
        }
        let dir = common::temp_dir("client_login_patch");
        let output = dir.join(exe.file_name().unwrap());
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
        println!("{}: {}", exe.display(), edits[0].describe());

        let copy = std::fs::read(&output).unwrap();
        let allowed = allowed_ranges(&before, &edits);
        let changed: Vec<usize> = (0..copy.len()).filter(|&offset| copy[offset] != original[offset]).collect();
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|offset| allowed.iter().any(|range| range.contains(offset))), "{}: {changed:x?}", exe.display());
        let after = ClientExeInspection::inspect(&copy).unwrap();
        assert!(after.checksum_valid(), "{}", exe.display());
        assert_eq!(after.tables, before.tables, "the facet tables are untouched");
        assert_eq!(after.version, before.version, "{}: the version resource is untouched", exe.display());
        assert_eq!(patched.manifest.exe_version, before.version);
        let sites = after.login.as_ref().unwrap();
        assert!(!sites.is_stock());
        let values = sites.values();
        let count = before_sites.host_site_count();
        assert_eq!(values.hosts, PATCHED_HOSTS[..count], "{}", exe.display());
        assert_eq!(values.ports, PATCHED_PORTS[..before_sites.port_site_count()], "{}", exe.display());
        assert_eq!(patched.manifest.values.login, Some(values));
        if before.family == ClientFamily::KingdomReborn {
            assert_eq!(sites.fixed.ip.to_string(), PATCHED_HOSTS[0]);
        } else {
            let expected: Vec<(String, u16)> =
                PATCHED_HOSTS.iter().zip(PATCHED_PORTS).take(count).map(|(host, port)| (host.to_string(), port)).collect();
            assert_eq!(sites.public_servers(), expected, "{}", exe.display());
            assert_eq!(sites.fixed, before_sites.fixed, "the internal address is not patched");
        }

        // The copy continues the chain through its manifest, with one address on every site.
        let second_dir = common::temp_dir("client_login_chain");
        let second = second_dir.join(exe.file_name().unwrap());
        let uniform = LoginServerEdit::uniform(sites, LoginHost::new(CHAINED_HOST).unwrap(), CHAINED_PORT);
        let chained = write_patched_client_exe(&output, &second, &ExePatch { edits: vec![ExeEdit::LoginServer(uniform)] }).unwrap();
        assert!(chained.warnings.iter().all(|warning| warning.kind != PatchWarningKind::Login), "{:?}", chained.warnings);
        assert_eq!(chained.manifest.steps.len(), 2);
        let chained = chained.after.login.unwrap().values();
        assert!(chained.hosts.iter().all(|host| host == CHAINED_HOST) && chained.ports.iter().all(|port| *port == CHAINED_PORT));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&second_dir);
    }
}
