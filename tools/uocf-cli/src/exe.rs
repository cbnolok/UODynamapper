use std::path::{Path, PathBuf};

use clap::Subcommand;
use color_eyre::eyre::{self, eyre, WrapErr};
use uocf::client::classic_difs::DifKind;
use uocf::client::classic_facets::{ClassicFileRole, CLASSIC_FACET_RECORD_SIZE};
use uocf::client::crypt_keys::{inspect_login_crypt, version_master_keys};
use uocf::client::facet_data::{check_client_facet_data, FacetDataStatus};
use uocf::client::login_servers::LoginServerAddress;
use uocf::client::facet_dimensions::{
    ClientExeInspection, ClientFamily, FacetTable, DISABLED_SPLIT_X, FACET_RECORD_SIZE, STOCK_SPLIT_X,
};
use uocf::client::encryption::EncryptionState;
use uocf::client::login::{LoginHost, LoginServerEdit, LoginSites};
use uocf::client::patch::{
    dry_run_client_exe, manifest_path, read_manifest, write_patched_client_exe, ByteChange, ChangeCause, ExeEdit,
    ExePatch, SplitEdit,
};
use uocf::enhanced::facet_sector::CLIENT_FACET_DIMENSIONS;

/// `N=WxH`: a facet and a size.
const FACET_SEPARATOR: char = '=';
const SIZE_SEPARATOR: char = 'x';
/// `N=HOST`, `N=PORT`: a login site and its value.
const SITE_SEPARATOR: char = '=';
/// The login sites are numbered from 1, as `exe show` prints them.
const FIRST_SITE_NUMBER: usize = 1;

#[derive(Subcommand, Debug)]
pub enum ExeCmd {
    /// Print the facet size tables, the split (Enhanced Client) or wrap seam (Classic), the built-in login server
    /// and whether the encryption is on (Enhanced Client, Kingdom Reborn) and the PE checksum of a client executable
    /// (UOSA*.exe, UOKR.exe, Classic client.exe; any 32-bit build, located by structure). Classic: both table copies with wrap, file ids and flag,
    /// the startup copy of facet 0 and whether the code applies the mapdif / stadif patches. Only reads the file.
    Show {
        exe: PathBuf,
        /// Print the actual embedded login master keys and advertised 0xEF protocol version (EC/KR).
        /// Compare the standard version formula without assuming that the PE resource version is the wire version.
        #[arg(long)]
        crypt_keys: bool,
    },
    /// Write a copy of a client executable with new facet sizes, wraps and split in one step, and its manifest
    /// `<output>.facet-dims.json`. The output may be beside the input and replace a different destination;
    /// the input executable is never overwritten.
    /// Resize the facet data too (see `check-data`).
    Patch {
        exe: PathBuf,
        /// The patched copy (not needed with --dry-run).
        #[arg(long, required_unless_present = "dry_run")]
        output: Option<PathBuf>,
        /// A new facet size, `N=WxH` (facet index, width and height in squares: multiples of 8, at most 32704).
        /// Repeat for more facets. Classic: a wrap side equal to the old size follows the new one.
        #[arg(long = "size", value_name = "N=WxH", value_parser = parse_facet_pair)]
        sizes: Vec<(u8, u32, u32)>,
        /// Classic only: a new wrap rectangle, `N=WxH` (multiples of 8, within the facet's size). Repeatable.
        #[arg(long = "wrap", value_name = "N=WxH", value_parser = parse_facet_pair)]
        wraps: Vec<(u8, u32, u32)>,
        /// Move the split of facets 0 and 1 (Enhanced Client: chunks are not built across it) or the wrap seam
        /// (Classic: the radar and walk test's x = 5120) to this x.
        #[arg(long, conflicts_with = "no_split")]
        split: Option<u32>,
        /// Disable the split or seam (x = 0x8000).
        #[arg(long)]
        no_split: bool,
        /// The built-in login server, `HOST:PORT`, written to every login site (`exe show` lists them), so every
        /// random draw gives it. Enhanced Client: each public host slot (a dotted IPv4 address or a host name the
        /// client resolves, shorter than the slot: at most 15 characters, 11 in UOSA 4.0.115.0's second slot) and
        /// each public port. Kingdom Reborn: its one address, a dotted IPv4 only (stored as a number). Classic reads
        /// login.cfg (`login set`).
        #[arg(long, value_name = "HOST:PORT", value_parser = parse_login_server)]
        login_server: Option<(LoginHost, u16)>,
        /// The host of one login host site, `N=HOST` (N from 1, as `exe show` numbers them); repeat per site. It
        /// overrides --login-server's host for that site; without --login-server every site needs one.
        #[arg(long = "login-host", value_name = "N=HOST", value_parser = parse_login_host)]
        login_hosts: Vec<(usize, LoginHost)>,
        /// The port of one login port site, `N=PORT`, or of every site, `PORT`; repeatable. It overrides
        /// --login-server's port; without --login-server every port site needs one.
        #[arg(long = "login-port", value_name = "[N=]PORT", value_parser = parse_login_port)]
        login_ports: Vec<(Option<usize>, u16)>,
        /// Remove login and game ciphers and, for Kingdom Reborn, bypass 0xE3/0xE4. Shortcut for all three
        /// controls below. The server must take unencrypted 0x80 and 0x91 and send unencrypted,
        /// Huffman-compressed game packets.
        #[arg(long, conflicts_with_all = ["no_login_encryption", "no_game_encryption", "bypass_kr_handshake"])]
        no_encryption: bool,
        /// Remove only the login connection's send cipher (Enhanced Client, Kingdom Reborn). The 0xEF seed
        /// stays unchanged; 0x80 is sent unencrypted. Game ciphers and the KR handshake stay as they are.
        #[arg(long)]
        no_login_encryption: bool,
        /// Remove only game cipher transforms (Enhanced Client, Kingdom Reborn). KR still requires a valid
        /// 0xE3 and answers 0xE4 before sending 0x91. Server game packets must remain Huffman-compressed.
        #[arg(long)]
        no_game_encryption: bool,
        /// Kingdom Reborn only: skip 0xE3/0xE4, send the relay auth key as the game seed, install Huffman and
        /// send 0x91 immediately. Installs no game cipher, even when its transform code remains stock.
        #[arg(long)]
        bypass_kr_handshake: bool,
        /// Print the byte changes without writing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Check that the facet data of a client folder has the sizes the executable's table holds. Enhanced Client and
    /// Kingdom Reborn: facetN.uop / facetNx.uop sectors and data.map, facets.uop, facetNN.dds. Classic: the map,
    /// staidx and statics files of both table copies. Only reads. Fails when anything disagrees.
    CheckData {
        exe: PathBuf,
        client_dir: PathBuf,
    },
}

/// Parses `N=WxH`.
fn parse_facet_pair(text: &str) -> Result<(u8, u32, u32), String> {
    let malformed = || format!("`{text}`: expected N=WxH, e.g. 0=8192x4096");
    let (facet, size) = text.split_once(FACET_SEPARATOR).ok_or_else(malformed)?;
    let (width, height) = size.split_once(SIZE_SEPARATOR).ok_or_else(malformed)?;
    let number = |part: &str| part.trim().parse::<u32>().map_err(|_| malformed());
    let facet = u8::try_from(number(facet)?).map_err(|_| malformed())?;
    Ok((facet, number(width)?, number(height)?))
}

/// Parses `HOST:PORT` for `--login-server`.
fn parse_login_server(text: &str) -> Result<(LoginHost, u16), String> {
    let address = LoginServerAddress::parse(text).map_err(|error| error.to_string())?;
    let host = LoginHost::new(&address.host).map_err(|error| error.to_string())?;
    Ok((host, address.port))
}

/// Parses the site number of `N=VALUE`.
fn parse_site_number(number: &str, text: &str) -> Result<usize, String> {
    match number.trim().parse::<usize>() {
        Ok(number) if number >= FIRST_SITE_NUMBER => Ok(number),
        _ => Err(format!("`{text}`: the site number must be {FIRST_SITE_NUMBER} or more")),
    }
}

/// Parses `N=HOST` for `--login-host`.
fn parse_login_host(text: &str) -> Result<(usize, LoginHost), String> {
    let (number, host) =
        text.split_once(SITE_SEPARATOR).ok_or_else(|| format!("`{text}`: expected N=HOST, e.g. 2=10.0.0.2"))?;
    let host = LoginHost::new(host.trim()).map_err(|error| error.to_string())?;
    Ok((parse_site_number(number, text)?, host))
}

/// Parses `N=PORT` or `PORT` for `--login-port`.
fn parse_login_port(text: &str) -> Result<(Option<usize>, u16), String> {
    let (number, port) = match text.split_once(SITE_SEPARATOR) {
        Some((number, port)) => (Some(parse_site_number(number, text)?), port),
        None => (None, text),
    };
    let port = port.trim().parse::<u16>().map_err(|_| format!("`{text}`: expected [N=]PORT, e.g. 2=2594"))?;
    Ok((number, port))
}

/// The login options of `exe patch`, resolved against the inspected build's sites.
struct LoginOptions {
    server: Option<(LoginHost, u16)>,
    hosts: Vec<(usize, LoginHost)>,
    ports: Vec<(Option<usize>, u16)>,
}

/// One value per site `1..=count`: the last numbered value given for it, else `default`.
fn per_site<T: Clone>(count: usize, option: &str, numbered: &[(usize, T)], default: Option<T>) -> eyre::Result<Vec<T>> {
    for (number, _) in numbered {
        eyre::ensure!(
            *number < FIRST_SITE_NUMBER + count,
            "--{option} {number}=...: this build has {count} such site(s), numbered from {FIRST_SITE_NUMBER}"
        );
    }
    (FIRST_SITE_NUMBER..FIRST_SITE_NUMBER + count)
        .map(|number| {
            numbered.iter().rev().find(|(given, _)| *given == number).map(|(_, value)| value.clone()).or(default.clone()).ok_or_else(
                || eyre!("{} site {number} has no value: give --{option} {number}=..., or --login-server for every site", option.replace('-', " ")),
            )
        })
        .collect()
}

impl LoginOptions {
    fn is_empty(&self) -> bool {
        self.server.is_none() && self.hosts.is_empty() && self.ports.is_empty()
    }

    /// The edit for `sites`; without located sites, the one host and port given, which the patch refuses with the
    /// family's reason.
    fn edit(&self, sites: Option<&LoginSites>) -> eyre::Result<LoginServerEdit> {
        let default_port =
            self.ports.iter().rev().find_map(|(number, port)| number.is_none().then_some(*port)).or(self.server.map(|(_, port)| port));
        let default_host = self.server.map(|(host, _)| host);
        let Some(sites) = sites else {
            let host = default_host.or(self.hosts.first().map(|(_, host)| *host));
            let port = default_port.or(self.ports.first().map(|(_, port)| *port));
            return Ok(LoginServerEdit { hosts: host.into_iter().collect(), ports: port.into_iter().collect() });
        };
        let numbered_ports: Vec<(usize, u16)> =
            self.ports.iter().filter_map(|(number, port)| number.map(|number| (number, *port))).collect();
        Ok(LoginServerEdit {
            hosts: per_site(sites.host_site_count(), "login-host", &self.hosts, default_host)?,
            ports: per_site(sites.port_site_count(), "login-port", &numbered_ports, default_port)?,
        })
    }
}

pub fn run(cmd: ExeCmd) -> eyre::Result<()> {
    match cmd {
        ExeCmd::Show { exe, crypt_keys } => show(&exe, crypt_keys),
        ExeCmd::Patch {
            exe,
            output,
            sizes,
            wraps,
            split,
            no_split,
            login_server,
            login_hosts,
            login_ports,
            no_encryption,
            no_login_encryption,
            no_game_encryption,
            bypass_kr_handshake,
            dry_run,
        } => {
            let mut edits: Vec<ExeEdit> =
                sizes.into_iter().map(|(facet, width, height)| ExeEdit::FacetSize { facet, width, height }).collect();
            edits.extend(wraps.into_iter().map(|(facet, wrap_width, wrap_height)| ExeEdit::ClassicWrap {
                facet,
                wrap_width,
                wrap_height,
            }));
            match (split, no_split) {
                (Some(x), _) => edits.push(ExeEdit::Split(SplitEdit::Set(x))),
                (None, true) => edits.push(ExeEdit::Split(SplitEdit::Disable)),
                (None, false) => {}
            }
            let login = LoginOptions { server: login_server, hosts: login_hosts, ports: login_ports };
            if !login.is_empty() {
                let inspection = inspect(&exe)?;
                edits.push(ExeEdit::LoginServer(login.edit(inspection.login.as_ref())?));
            }
            if no_encryption {
                edits.push(ExeEdit::NoEncryption);
            }
            if no_login_encryption {
                edits.push(ExeEdit::NoLoginEncryption);
            }
            if no_game_encryption {
                edits.push(ExeEdit::NoGameEncryption);
            }
            if bypass_kr_handshake {
                edits.push(ExeEdit::BypassKrHandshake);
            }
            let patch = ExePatch { edits };
            match (dry_run, output) {
                (true, _) => dry_run_patch(&exe, &patch),
                (false, Some(output)) => write_patch(&exe, &output, &patch),
                (false, None) => Err(eyre!("--output is required without --dry-run")),
            }
        }
        ExeCmd::CheckData { exe, client_dir } => check_data(&exe, &client_dir),
    }
}

fn inspect(exe: &Path) -> eyre::Result<ClientExeInspection> {
    let bytes = std::fs::read(exe).wrap_err_with(|| format!("read {}", exe.display()))?;
    ClientExeInspection::inspect(&bytes).wrap_err_with(|| format!("inspect {}", exe.display()))
}

fn split_label(x: u32) -> &'static str {
    match x {
        STOCK_SPLIT_X => "stock",
        DISABLED_SPLIT_X => "disabled",
        _ => "custom",
    }
}

fn located(inspection: &ClientExeInspection, offset: usize) -> String {
    format!("file {offset:#x} (VA {:#x})", inspection.pe.offset_to_va(offset).unwrap_or_default())
}

fn print_table(inspection: &ClientExeInspection, index: usize, table: &FacetTable) {
    let classic = inspection.family == ClientFamily::Classic;
    let layout = if classic {
        format!("{CLASSIC_FACET_RECORD_SIZE} bytes {{name, width, height, wrap, 6 file ids, felucca_rules}}")
    } else {
        format!("{FACET_RECORD_SIZE} bytes {{name pointer, width, height}}")
    };
    let copy = match (inspection.tables.len(), index) {
        (1, _) => String::new(),
        (_, 0) => " (copy 1: default)".to_owned(),
        _ => format!(" (copy {}: 0xA9 flag 0x8000)", index + 1),
    };
    println!(
        "table     located by structure: {} records of {layout} in {}, file {:#x}, VA {:#x}{copy}",
        table.records.len(),
        table.section,
        table.offset,
        table.va,
    );
    let names = inspection.classic.as_ref().and_then(|layout| layout.file_names.as_ref());
    for (facet, record) in table.records.iter().enumerate() {
        let (geometry, name) = (record.geometry, &record.name);
        match record.classic {
            None => {
                let stock = if CLIENT_FACET_DIMENSIONS.get(facet) == Some(&geometry) { "" } else { "  (not stock)" };
                println!(
                    "  {facet} {name:<14} {:>5} x {:<5} {:>3} x {:<3} = {:>5} sectors{stock}",
                    geometry.width,
                    geometry.height,
                    geometry.sector_columns(),
                    geometry.sector_rows(),
                    geometry.sector_count()
                );
            }
            Some(classic) => {
                let files = ClassicFileRole::ALL
                    .iter()
                    .zip(classic.file_ids)
                    .map(|(role, id)| match names.and_then(|names| names.name(id)) {
                        Some(file) => format!("{} {id} {file}", role.label()),
                        None => format!("{} {id}", role.label()),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "  {facet} {name:<14} {:>5} x {:<5} wrap {:>5} x {:<5} felucca_rules {}  {files}",
                    geometry.width, geometry.height, classic.wrap.width, classic.wrap.height, classic.felucca_rules
                );
            }
        }
    }
}

fn print_inspection(inspection: &ClientExeInspection) {
    println!("sha256    {}", inspection.sha256);
    if let Some(build) = inspection.known {
        println!("build     known: {}", build.name);
    }
    println!("family    {} (located by structure)", inspection.family.label());
    println!("size      {} bytes", inspection.file_size);
    match &inspection.version {
        Some(version) => println!("version   {version} (version resource)"),
        None => println!("version   none (no version resource)"),
    }
    let state = if inspection.checksum_valid() { "valid" } else { "INVALID" };
    println!(
        "checksum  {:#010x} at {:#x}, computed {:#010x}: {state}",
        inspection.pe.stored_checksum, inspection.pe.checksum_offset, inspection.computed_checksum
    );
    let signed = if inspection.pe.is_signed() { "Authenticode signed (cannot be patched)" } else { "not signed" };
    println!("signature {signed}");
    println!("values    {}", if inspection.is_stock() { "stock" } else { "not stock" });
    for (index, table) in inspection.tables.iter().enumerate() {
        print_table(inspection, index, table);
    }
    if let Some(layout) = &inspection.classic {
        match &layout.file_names {
            Some(names) => println!(
                "files     file name table: {} entries of {} bytes at {}",
                names.names.len(),
                names.stride,
                located(inspection, names.offset)
            ),
            None => println!("files     file name table not located: ids are shown raw"),
        }
        match layout.startup {
            Some(startup) => println!(
                "startup   facet 0 globals at {}: wrap {} x {}, size {} x {}",
                located(inspection, startup.offset),
                startup.wrap.width,
                startup.wrap.height,
                startup.size.width,
                startup.size.height
            ),
            None => println!("startup   not located: facet 0 cannot be patched"),
        }
    }
    print_login(inspection);
    print_encryption(inspection);
    print_difs(inspection);
    let what = if inspection.family == ClientFamily::Classic { "wrap seam" } else { "split" };
    match &inspection.split {
        None => match inspection.family {
            ClientFamily::KingdomReborn => println!("split     none (Kingdom Reborn has no split)"),
            ClientFamily::Enhanced => println!(
                "split     not found: no x = {STOCK_SPLIT_X} Felucca/Trammel split sequence (the split code has \
                 another shape: check it by hand)"
            ),
            ClientFamily::Classic => println!("seam      not found: no x = {STOCK_SPLIT_X} wrap seam compares"),
        },
        Some(split) => {
            let at = split.offsets.iter().map(|offset| located(inspection, *offset)).collect::<Vec<_>>().join(", ");
            let sites = split.signature_offsets.iter().map(|offset| format!("{offset:#x}")).collect::<Vec<_>>().join(", ");
            println!(
                "{what:<9} located by instruction pattern at file {sites}: x = {} ({}), imm32 at {at}",
                split.x,
                split_label(split.x)
            );
        }
    }
}

/// Classic: whether the code applies the mapdif / stadif patches, and why.
fn print_difs(inspection: &ClientExeInspection) {
    let Some(difs) = &inspection.difs else {
        return;
    };
    for (index, kind) in DifKind::ALL.into_iter().enumerate() {
        let head = if index == 0 { "difs" } else { "" };
        println!("{head:<9} {}: {}", kind.label(), difs.status(kind).describe());
    }
    println!("{:<9} {}", "", difs.advice());
    for line in &difs.evidence {
        println!("{:<9}   {line}", "");
    }
}

fn print_login(inspection: &ClientExeInspection) {
    let Some(login) = &inspection.login else {
        match inspection.family {
            ClientFamily::Classic => println!("login     not in the executable: Classic reads login.cfg (`uocf-cli login show`)"),
            _ => println!("login     the login address code was not located"),
        }
        return;
    };
    let fixed = &login.fixed;
    let fixed_at = located(inspection, fixed.offset);
    if inspection.family == ClientFamily::KingdomReborn {
        let state = if login.is_stock() { "stock" } else { "NOT the stock address: patched" };
        println!("login     built-in {}:{} ({state}; the only address; MOV imm32/imm16 at {fixed_at})", fixed.ip, fixed.port);
        println!(
            "  ip imm32 at {}, port imm16 at {}: --login-server takes a dotted IPv4 address only",
            located(inspection, fixed.ip_offset),
            located(inspection, fixed.port_offset)
        );
        return;
    }
    let state = if login.is_stock() { "stock" } else { "NOT the stock address: patched" };
    println!("login     built-in, located by instruction pattern ({state}):");
    let draw = |index: usize, count: usize| match count {
        1 => String::new(),
        _ => format!(" (rand {})", if index == 0 { "odd" } else { "even" }),
    };
    for (index, slot) in login.hosts.iter().enumerate() {
        println!(
            "  host {} {:<16} {}-byte slot at {}, pushed at file {:#x}{}",
            index + 1,
            slot.host,
            slot.capacity,
            located(inspection, slot.offset),
            slot.push_offset,
            draw(index, login.hosts.len())
        );
    }
    for (index, site) in login.ports.iter().enumerate() {
        println!(
            "  port {} {:<16} imm16 at {}{}",
            index + 1,
            site.port,
            located(inspection, site.offset),
            draw(index, login.ports.len())
        );
    }
    println!("  internal {}:{} (internal flag only, not patched) at {fixed_at}", fixed.ip, fixed.port);
}

/// Enhanced Client and Kingdom Reborn: whether the network ciphers are on, and where each is.
fn print_encryption(inspection: &ClientExeInspection) {
    let Some(ciphers) = &inspection.encryption else {
        return;
    };
    let state = match ciphers.state() {
        Some(state) if ciphers.is_complete() => state.label().to_owned(),
        Some(state) => format!("{} where located; NOT all ciphers located", state.label()),
        None => "the cipher code was not located".to_owned(),
    };
    println!("crypto    {state}");
    for site in &ciphers.sites {
        let form = if site.removed { hex(site.patched()) } else { hex(site.stock()) };
        let what = if site.removed { "patched" } else { "stock" };
        println!("  {:<26} {form} ({what}) at {}", site.kind.label(), located(inspection, site.offset));
    }
    if !ciphers.unlocated.is_empty() {
        println!("  not located (pattern not matched exactly once): {}", ciphers.describe_unlocated());
    }
    if ciphers.values() == Some(EncryptionState::Encrypted) {
        println!("  --no-login-encryption and --no-game-encryption select cipher groups");
        if inspection.family == ClientFamily::KingdomReborn {
            println!("  --bypass-kr-handshake independently enables immediate plaintext game login");
        }
        println!("  --no-encryption selects all applicable changes");
    }
}

fn show(exe: &Path, crypt_keys: bool) -> eyre::Result<()> {
    let inspection = inspect(exe)?;
    print_inspection(&inspection);
    if crypt_keys {
        let bytes = std::fs::read(exe).wrap_err_with(|| format!("read {}", exe.display()))?;
        print_crypt_keys(&bytes, &inspection)?;
    }
    if let Some(manifest) = read_manifest(exe)? {
        let current =
            if manifest.patched_sha256 == inspection.sha256 { "matches this file" } else { "STALE: another file" };
        println!("manifest  {} (version {}, {current})", manifest_path(exe).display(), manifest.version);
        println!("  original {}", manifest.original_sha256);
        if let Some(build) = &manifest.build {
            println!("  build    known: {build}");
        }
        println!("  source   {}", manifest.source_sha256);
        for (number, step) in manifest.steps.iter().enumerate() {
            let edits = step.edits.iter().map(ExeEdit::describe).collect::<Vec<_>>().join("; ");
            let edits = if edits.is_empty() { "(not recorded)".to_owned() } else { edits };
            println!("  step {} {} -> {}: {edits}", number + 1, step.source_sha256, step.patched_sha256);
        }
    }
    Ok(())
}

fn print_crypt_keys(bytes: &[u8], inspection: &ClientExeInspection) -> eyre::Result<()> {
    let info = inspect_login_crypt(bytes, inspection)?;
    let keys = info.keys;
    println!("cryptkeys embedded {:#010x} {:#010x} (login master keys)", keys.key_1, keys.key_2);
    println!("  first-round XOR {:#010x}; {}", keys.first_round, located(inspection, info.cipher_offset));
    println!("  login XOR {}", if info.cipher_removed { "patched out; keys are still embedded" } else { "stock" });
    match info.protocol_version {
        Some(version) => {
            println!("  wire version {version} (0xEF writer instructions)");
            if let Some(resource) = inspection.version {
                let resource_parts = resource.file.0.map(u32::from);
                let relation = if version.0 == resource_parts { "same" } else { "DIFFERS" };
                println!("  PE file version {}: {relation} from wire version", resource.file);
            }
            let (key_1, key_2) = version_master_keys(version);
            let relation = if (key_1, key_2) == (keys.key_1, keys.key_2) { "matches" } else { "DIFFERS from" };
            println!("  version formula {key_1:#010x} {key_2:#010x}: {relation} embedded keys (fourth component ignored)");
        }
        None => println!("  wire version not located uniquely ({} matches); no PE-version substitution", info.protocol_version_matches),
    }
    println!("  These are login master keys; the connection seed and DH game keys are separate.");
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(" ")
}

fn print_changes(changes: &[ByteChange], patch: &ExePatch) {
    println!("changes   {} fields:", changes.len());
    for change in changes {
        let va = change.va.map(|va| format!("VA {va:#010x}")).unwrap_or_else(|| "header".to_owned());
        let cause = match change.cause {
            ChangeCause::Edit(index) => format!("edit {}: {}", index + 1, patch.edits[index].describe()),
            ChangeCause::Checksum => "checksum".to_owned(),
        };
        println!(
            "  {:#010x} {va:<14} {} -> {}  {}  [{cause}]",
            change.offset,
            hex(&change.old),
            hex(&change.new),
            change.field
        );
    }
}

fn dry_run_patch(exe: &Path, patch: &ExePatch) -> eyre::Result<()> {
    let plan = dry_run_client_exe(exe, patch)?;
    for warning in &plan.warnings {
        println!("warning: {warning}");
    }
    print_changes(&plan.changes, patch);
    println!("dry run   nothing written");
    Ok(())
}

fn write_patch(exe: &Path, output: &Path, patch: &ExePatch) -> eyre::Result<()> {
    let patched = write_patched_client_exe(exe, output, patch)?;
    for warning in &patched.warnings {
        println!("warning: {warning}");
    }
    print_inspection(&patched.after);
    print_changes(&patched.changes, patch);
    println!("wrote     {}", output.display());
    println!("manifest  {}", manifest_path(output).display());
    let resized = patch.edits.iter().any(|edit| matches!(edit, ExeEdit::FacetSize { .. }));
    if resized {
        let data = match patched.after.family {
            ClientFamily::Classic => "the map, staidx and statics files",
            _ => "facetN.uop sectors and data.map, facets.uop, facetNN.dds",
        };
        println!("Resize the facet data too: {data} (`uocf-cli exe check-data`).");
    }
    Ok(())
}

fn check_data(exe: &Path, client_dir: &Path) -> eyre::Result<()> {
    let inspection = inspect(exe)?;
    let reports = check_client_facet_data(&inspection, client_dir)?;
    let mut mismatches = 0;
    for report in &reports {
        let geometry = report.geometry;
        let name = &inspection.table().records[usize::from(report.facet)].name;
        match inspection.family {
            ClientFamily::Classic => println!("facet {} {name} {}x{}", report.facet, geometry.width, geometry.height),
            _ => println!(
                "facet {} {name} {}x{} ({} sectors)",
                report.facet,
                geometry.width,
                geometry.height,
                geometry.sector_count()
            ),
        }
        for item in &report.items {
            let status = match item.status {
                FacetDataStatus::Ok => "ok",
                FacetDataStatus::Mismatch => "MISMATCH",
                FacetDataStatus::Absent => "absent",
            };
            println!("  {status:<8} {:<28} {}", item.source, item.detail);
        }
        mismatches += report.mismatches().count();
    }
    eyre::ensure!(mismatches == 0, "{mismatches} files disagree with the executable's facet table");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: ExeCmd,
    }

    #[test]
    fn scoped_encryption_flags_combine_and_default_off() {
        let base = ["uocf-cli", "patch", "UOKR.exe", "--dry-run"];
        let flags = ["--no-login-encryption", "--no-game-encryption", "--bypass-kr-handshake"];
        for selected in [&[][..], &flags[..]] {
            let args = base.iter().chain(selected).copied();
            let cli = TestCli::try_parse_from(args).unwrap();
            let ExeCmd::Patch { no_encryption, no_login_encryption, no_game_encryption, bypass_kr_handshake, .. } = cli.command else {
                panic!("expected patch command");
            };
            assert!(!no_encryption);
            let enabled = !selected.is_empty();
            assert_eq!((no_login_encryption, no_game_encryption, bypass_kr_handshake), (enabled, enabled, enabled));
        }
    }

    #[test]
    fn all_encryption_shortcut_conflicts_with_each_scoped_flag() {
        for flag in ["--no-login-encryption", "--no-game-encryption", "--bypass-kr-handshake"] {
            let error = TestCli::try_parse_from(["uocf-cli", "patch", "UOKR.exe", "--dry-run", "--no-encryption", flag])
                .err().expect("conflicting flags");
            assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }
}
