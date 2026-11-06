//! The login server section: where each family reads its login server from (see
//! `lib/uocf/src/client/LOGIN_SERVER.md`), the executable's per-site host and port edit ([`LoginEdit`], one
//! more edit of the patch [`super::edits::EditState::build_patch`] builds) and the Classic `login.cfg` editor
//! ([`ClassicLogin`]), saved as a copy apart from the patch.
//!
//! - Enhanced Client: the address is built into the executable (a host per public slot, a port per immediate); the
//!   `LoginServers*.xml` files are never read.
//! - Kingdom Reborn: one built-in address, a dotted IPv4 number and a port, written by the patch.
//! - Classic: `uo.cfg`, then the file its `LoginServerFile` names or `login.cfg`; the executable holds no address.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use uocf::client::login_servers::{
    read_login_servers, validate_login_host, validate_login_port, write_login_cfg, CfgServerKind, HostKind, LoginCfg,
    LoginFileRole, LoginServerAddress, LoginServerEntry, LoginServersReport,
    LOGIN_CFG_FILE, MIN_LOGIN_PORT,
};
use uocf::client::facet_dimensions::{ClientExeInspection, ClientFamily};
use uocf::client::login::{
    LoginHost, LoginServerEdit, LoginSites, LoginValues, STOCK_INTERNAL_LOGIN_ADDRESSES, STOCK_KR_LOGIN_ADDRESS,
    STOCK_LOGIN_HOSTS, STOCK_LOGIN_PORTS,
};
use uocf::client::patch::{ExeEdit, ExeManifest};

use super::edits::Validation;

/// The parity of the `rand()` draw that picks host or port `i`, in draw order.
const DRAW_PARITY: [&str; 2] = ["odd", "even"];
const CLASSIC_NOT_IN_EXE: &str = "the Classic client reads its login server from login.cfg, not from the executable: \
                                  edit the list and save it with \"Save login.cfg as…\"";
const NOT_LOCATED: &str = "the login address code was not located in this build: its login server cannot be changed";
/// Why Kingdom Reborn takes no host name.
pub const KR_IPV4_ONLY: &str = "Kingdom Reborn takes a dotted IPv4 address only: UOKR.exe stores its login address as \
                                a 32-bit number in an instruction (MOV imm32), not as a string it resolves";
/// `host,port` in a `.cfg` value; `host:port` in the editor.
const CFG_PORT_SEPARATOR: char = ',';
const ADDRESS_PORT_SEPARATOR: char = ':';

/// What the login section can do for the opened executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMode {
    /// Enhanced Client with its public hosts and ports located: a host per slot and a port per immediate, written
    /// by the patch.
    PublicSites,
    /// Kingdom Reborn: the one built-in address (IPv4 imm32, port imm16), written by the patch.
    FixedAddress,
    /// Classic: the login server is in `login.cfg`, saved apart from the patch.
    ConfigFile,
    /// Enhanced Client or Kingdom Reborn without a located login address code.
    NotLocated,
}

pub fn login_mode(inspection: &ClientExeInspection) -> LoginMode {
    match (inspection.family, &inspection.login) {
        (ClientFamily::Classic, _) => LoginMode::ConfigFile,
        (ClientFamily::Enhanced, Some(sites)) if !sites.hosts.is_empty() && !sites.ports.is_empty() => {
            LoginMode::PublicSites
        }
        (ClientFamily::KingdomReborn, Some(sites)) if sites.uses_fixed_address() => LoginMode::FixedAddress,
        _ => LoginMode::NotLocated,
    }
}

/// The login values the build shipped with: the file's own when they are stock, else the ones the manifest of a
/// patched copy records. `None` when neither tells (a copy changed by another tool).
pub fn stock_login(inspection: &ClientExeInspection, manifest: Option<&ExeManifest>) -> Option<LoginValues> {
    let sites = inspection.login.as_ref()?;
    if sites.is_stock() {
        return Some(sites.values());
    }
    let manifest = manifest.filter(|manifest| manifest.patched_sha256 == inspection.sha256)?;
    manifest.original_values.as_ref()?.login.clone()
}

/// `host:port, host:port`.
pub fn servers_text(servers: &[(String, u16)]) -> String {
    servers.iter().map(|(host, port)| format!("{host}{ADDRESS_PORT_SEPARATOR}{port}")).collect::<Vec<_>>().join(", ")
}

/// Host `i` with port `i` (or the last one, when one list is shorter).
fn paired(values: &LoginValues) -> Vec<(String, u16)> {
    let count = values.hosts.len().max(values.ports.len());
    (0..count)
        .filter_map(|index| {
            let host = values.hosts.get(index).or(values.hosts.last())?;
            let port = values.ports.get(index).or(values.ports.last())?;
            Some((host.clone(), *port))
        })
        .collect()
}

/// The servers the build connects to: the Enhanced Client's public ones, Kingdom Reborn's fixed address.
pub fn built_in_servers(sites: &LoginSites) -> Vec<(String, u16)> {
    paired(&sites.values())
}

/// The login state, in words: stock, patched (and the stock values), or changed by another tool.
pub fn login_status(sites: &LoginSites, stock: Option<&LoginValues>) -> String {
    match stock {
        _ if sites.is_stock() && sites.uses_fixed_address() => "stock: the traced address".to_owned(),
        _ if sites.is_stock() => "stock: the official hosts and ports".to_owned(),
        Some(stock) => format!("patched; the manifest records the stock {}", servers_text(&paired(stock))),
        None => "not stock, and no manifest records the stock values (changed by another tool)".to_owned(),
    }
}

/// Which `rand()` draw picks item `index` of `count`; `draw` names the draw (hosts first, ports second).
fn drawn_by(index: usize, count: usize, draw: &str) -> String {
    match DRAW_PARITY.get(index) {
        _ if count == 1 => "every login".to_owned(),
        Some(parity) => format!("{parity} {draw} rand()"),
        None => format!("draw {}", index + 1),
    }
}

/// One login site (a public host slot or port immediate, or Kingdom Reborn's address) as the section shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSiteRow {
    pub what: String,
    pub drawn_by: String,
    pub offset: String,
    pub va: String,
    /// Hosts: the slot's bytes; ports: the immediate's.
    pub size: String,
    pub value: String,
    /// Stock, changed (with the stock value), or official / not official when the stock values are unknown.
    pub stock: String,
}

fn stock_cell(value: &str, stock: Option<&str>, official: bool) -> String {
    match stock {
        Some(stock) if stock == value => "stock".to_owned(),
        Some(stock) => format!("changed (stock {stock})"),
        None if official => "official".to_owned(),
        None => "not official".to_owned(),
    }
}

/// The login sites a login edit writes, in order: the public host slots then the port immediates, or Kingdom
/// Reborn's ip and port immediates; `stock` from [`stock_login`].
pub fn login_site_rows(inspection: &ClientExeInspection, stock: Option<&LoginValues>) -> Vec<LoginSiteRow> {
    let Some(sites) = &inspection.login else {
        return Vec::new();
    };
    let hex = |offset: usize| format!("{offset:#010x}");
    let va = |offset: usize| inspection.pe.offset_to_va(offset).map(|va| format!("{va:#010x}")).unwrap_or_default();
    let stock_host = |index: usize| stock.and_then(|stock| stock.hosts.get(index)).map(String::as_str);
    let stock_port = |index: usize| stock.and_then(|stock| stock.ports.get(index)).map(u16::to_string);
    if sites.uses_fixed_address() {
        let fixed = &sites.fixed;
        let (ip, port) = (fixed.ip.to_string(), fixed.port.to_string());
        let traced = (fixed.ip, fixed.port) == STOCK_KR_LOGIN_ADDRESS;
        return vec![
            LoginSiteRow {
                what: "address".to_owned(),
                drawn_by: drawn_by(0, 1, ""),
                offset: hex(fixed.ip_offset),
                va: va(fixed.ip_offset),
                size: format!("imm{} (IPv4 only)", u32::BITS),
                stock: stock_cell(&ip, stock_host(0), traced),
                value: ip,
            },
            LoginSiteRow {
                what: "port".to_owned(),
                drawn_by: drawn_by(0, 1, ""),
                offset: hex(fixed.port_offset),
                va: va(fixed.port_offset),
                size: format!("imm{}", u16::BITS),
                stock: stock_cell(&port, stock_port(0).as_deref(), traced),
                value: port,
            },
        ];
    }
    let mut rows = Vec::new();
    for (index, slot) in sites.hosts.iter().enumerate() {
        rows.push(LoginSiteRow {
            what: format!("host {}", index + 1),
            drawn_by: drawn_by(index, sites.hosts.len(), "1st"),
            offset: hex(slot.offset),
            va: format!("{:#010x}", slot.va),
            size: format!("{} bytes (host at most {})", slot.capacity, slot.capacity - 1),
            value: slot.host.clone(),
            stock: stock_cell(&slot.host, stock_host(index), STOCK_LOGIN_HOSTS.contains(&slot.host.as_str())),
        });
    }
    for (index, site) in sites.ports.iter().enumerate() {
        let official = STOCK_LOGIN_PORTS.get(index) == Some(&site.port);
        rows.push(LoginSiteRow {
            what: format!("port {}", index + 1),
            drawn_by: drawn_by(index, sites.ports.len(), "2nd"),
            offset: hex(site.offset),
            va: va(site.offset),
            size: format!("imm{}", u16::BITS),
            value: site.port.to_string(),
            stock: stock_cell(&site.port.to_string(), stock_port(index).as_deref(), official),
        });
    }
    rows
}

/// The Enhanced Client's internal test address, where it is and whether it is a traced stock value; `None` in
/// Kingdom Reborn (its fixed address is the login site itself, [`login_site_rows`]).
pub fn internal_address(inspection: &ClientExeInspection) -> Option<String> {
    let sites = inspection.login.as_ref().filter(|sites| !sites.uses_fixed_address())?;
    let fixed = &sites.fixed;
    let traced = STOCK_INTERNAL_LOGIN_ADDRESSES.contains(&(fixed.ip, fixed.port));
    let state = if traced { "the traced stock value" } else { "not a traced stock value (changed by another tool)" };
    Some(format!(
        "{}{ADDRESS_PORT_SEPARATOR}{} at file {:#010x} (ip imm32 {:#010x}, port imm16 {:#010x}): {state}",
        fixed.ip, fixed.port, fixed.offset, fixed.ip_offset, fixed.port_offset
    ))
}

/// One host or port site of the edit: what the file has there and the typed text (empty: keep the file's value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginField {
    /// "host 1", "port 2", "address".
    pub label: String,
    pub current: String,
    /// Hosts in the Enhanced Client: the slot's bytes (the host takes one less).
    pub capacity: Option<usize>,
    pub text: String,
}

impl LoginField {
    fn new(label: String, current: String, capacity: Option<usize>) -> Self {
        Self { label, current, capacity, text: String::new() }
    }

    /// The typed value, or the file's when nothing is typed.
    fn value(&self) -> &str {
        match self.text.trim() {
            "" => &self.current,
            text => text,
        }
    }

    pub fn max_host_length(&self) -> Option<usize> {
        self.capacity.map(|capacity| capacity.saturating_sub(1))
    }
}

/// The login server edit: one host per host site and one port per port site (the Enhanced Client's public slots
/// and immediates, Kingdom Reborn's one address). An empty field keeps the file's value; every field empty: no edit.
/// With `same_for_all`, a field typed is copied to the other sites of its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginEdit {
    pub mode: LoginMode,
    pub hosts: Vec<LoginField>,
    pub ports: Vec<LoginField>,
    pub same_for_all: bool,
}

impl LoginEdit {
    pub fn new(inspection: &ClientExeInspection) -> Self {
        let mode = login_mode(inspection);
        let (mut hosts, mut ports) = (Vec::new(), Vec::new());
        match (mode, &inspection.login) {
            (LoginMode::PublicSites, Some(sites)) => {
                for (index, slot) in sites.hosts.iter().enumerate() {
                    hosts.push(LoginField::new(format!("host {}", index + 1), slot.host.clone(), Some(slot.capacity)));
                }
                for (index, site) in sites.ports.iter().enumerate() {
                    ports.push(LoginField::new(format!("port {}", index + 1), site.port.to_string(), None));
                }
            }
            (LoginMode::FixedAddress, Some(sites)) => {
                hosts.push(LoginField::new("address".to_owned(), sites.fixed.ip.to_string(), None));
                ports.push(LoginField::new("port".to_owned(), sites.fixed.port.to_string(), None));
            }
            _ => {}
        }
        Self { mode, hosts, ports, same_for_all: true }
    }

    pub fn is_edited(&self) -> bool {
        self.hosts.iter().chain(&self.ports).any(|field| !field.text.trim().is_empty())
    }

    /// True when the build has more than one site of a kind, so `same_for_all` matters.
    pub fn has_several_sites(&self) -> bool {
        self.hosts.len() > 1 || self.ports.len() > 1
    }

    pub fn clear(&mut self) {
        for field in self.hosts.iter_mut().chain(&mut self.ports) {
            field.text.clear();
        }
    }

    /// With `same_for_all`, copies host `index`'s text to every host field (`port: false`) or port `index`'s to
    /// every port field.
    pub fn spread(&mut self, port: bool, index: usize) {
        if !self.same_for_all {
            return;
        }
        let fields = if port { &mut self.ports } else { &mut self.hosts };
        let Some(text) = fields.get(index).map(|field| field.text.clone()) else {
            return;
        };
        for field in fields.iter_mut() {
            field.text.clone_from(&text);
        }
    }

    /// Turns `same_for_all` on or off; on, the first host and port fields are copied to the others.
    pub fn set_same_for_all(&mut self, same: bool) {
        self.same_for_all = same;
        self.spread(false, 0);
        self.spread(true, 0);
    }

    /// Validates the fields live: the family, each host (IPv4 or name, which warns; Kingdom Reborn: IPv4 only)
    /// against its own slot, each port's range. Returns the edit, if any (none when the file already has these
    /// values).
    pub fn check(&self) -> (Option<ExeEdit>, Validation) {
        let mut validation = Validation::default();
        if !self.is_edited() {
            return (None, validation);
        }
        let refusal = match self.mode {
            LoginMode::PublicSites | LoginMode::FixedAddress => None,
            LoginMode::ConfigFile => Some(CLASSIC_NOT_IN_EXE),
            LoginMode::NotLocated => Some(NOT_LOCATED),
        };
        if let Some(refusal) = refusal {
            validation.errors.push(refusal.to_owned());
            return (None, validation);
        }
        let mut hosts = Vec::new();
        let mut names: Vec<&str> = Vec::new();
        for field in &self.hosts {
            let host = field.value();
            match validate_login_host(host) {
                Ok(HostKind::Ipv4) => {}
                Ok(HostKind::Name) if self.mode == LoginMode::FixedAddress => {
                    validation.errors.push(format!("{}: `{host}`: {KR_IPV4_ONLY}", field.label));
                }
                Ok(HostKind::Name) => {
                    if !names.contains(&host) {
                        names.push(host);
                        validation
                            .warnings
                            .push(format!("`{host}` is a host name: the client resolves it (gethostbyname) at each login"));
                    }
                }
                Err(error) => validation.errors.push(format!("{}: {error:#}", field.label)),
            }
            if let Some((capacity, max)) = field.capacity.zip(field.max_host_length())
                && host.len() > max
            {
                validation.errors.push(format!(
                    "{}: `{host}` has {} characters: its slot is {capacity} bytes, so it takes at most {max}",
                    field.label,
                    host.len()
                ));
            }
            match LoginHost::new(host) {
                Ok(host) => hosts.push(host),
                Err(error) => validation.errors.push(format!("{}: {error:#}", field.label)),
            }
        }
        let mut ports = Vec::new();
        for field in &self.ports {
            let text = field.value();
            match text.parse::<u16>() {
                Ok(port) => match validate_login_port(port) {
                    Ok(()) => ports.push(port),
                    Err(error) => validation.errors.push(format!("{}: {error:#}", field.label)),
                },
                Err(_) => validation
                    .errors
                    .push(format!("{}: `{text}` is not a number {MIN_LOGIN_PORT}..={}", field.label, u16::MAX)),
            }
        }
        let unchanged = self.hosts.iter().chain(&self.ports).all(|field| field.value() == field.current);
        if !validation.is_ok() || unchanged {
            return (None, validation);
        }
        (Some(ExeEdit::LoginServer(LoginServerEdit { hosts, ports })), validation)
    }
}

/// A row of the Classic editor: `host:port`.
pub fn check_server_row(text: &str) -> Result<LoginServerAddress, String> {
    LoginServerAddress::parse(text).map_err(|error| format!("{error:#}"))
}

/// The editor rows of the `kind` lines of `cfg`: `host:port`, the raw value (its comma as a colon) when it does not
/// parse.
fn cfg_rows(cfg: &LoginCfg, kind: CfgServerKind) -> Vec<String> {
    cfg.servers()
        .into_iter()
        .filter(|line| line.kind == kind)
        .map(|line| match &line.address {
            Ok(address) => address.to_string(),
            Err(_) => match line.value.rsplit_once(CFG_PORT_SEPARATOR) {
                Some((host, port)) => format!("{}{ADDRESS_PORT_SEPARATOR}{}", host.trim(), port.trim()),
                None => line.value.clone(),
            },
        })
        .collect()
}

pub const CFG_KINDS: [CfgServerKind; 2] = [CfgServerKind::Primary, CfgServerKind::Backup];

pub fn kind_label(kind: CfgServerKind) -> &'static str {
    match kind {
        CfgServerKind::Primary => "primary",
        CfgServerKind::Backup => "backup",
    }
}

/// Fails when `output`'s folder lies inside one of `client_dirs` (the rule of the executable's "Save as": copies go
/// elsewhere). Folders that do not resolve are skipped.
pub fn check_outside_client_folders(output: &Path, client_dirs: &[PathBuf]) -> eyre::Result<()> {
    let output_dir = match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let output_dir =
        std::fs::canonicalize(output_dir).wrap_err_with(|| format!("resolve the folder of {}", output.display()))?;
    for dir in client_dirs {
        let Ok(dir) = std::fs::canonicalize(dir) else {
            continue;
        };
        eyre::ensure!(
            !output_dir.starts_with(&dir),
            "refusing to write inside the client folder ({}): write the copy elsewhere",
            dir.display()
        );
    }
    Ok(())
}

/// The Classic login configuration of a client folder and its editor: the primary and backup lists of the login
/// file (`login.cfg`, or the file `uo.cfg`'s `LoginServerFile` names), saved as a copy with [`Self::save_copy`].
#[derive(Debug, Clone)]
pub struct ClassicLogin {
    pub dir: PathBuf,
    pub report: Result<LoginServersReport, String>,
    /// The login file the copy is made of, and whether it exists (a missing one starts empty).
    pub cfg_path: PathBuf,
    pub cfg_found: bool,
    cfg: Result<LoginCfg, String>,
    pub primary: Vec<String>,
    pub backup: Vec<String>,
    original: [Vec<String>; 2],
    /// An existing file chosen as the copy, waiting for the overwrite confirmation.
    pub pending_overwrite: Option<PathBuf>,
    pub last_save: Option<Result<PathBuf, String>>,
}

impl ClassicLogin {
    /// Reads where the client of `dir` reads its login server from, and the login file. Only reads.
    pub fn load(dir: &Path) -> Self {
        let report = read_login_servers(dir, ClientFamily::Classic).map_err(|error| format!("{error:#}"));
        let login_file = report.as_ref().ok().and_then(|report| report.files.iter().find(|file| file.role == LoginFileRole::LoginCfg));
        let (cfg_path, cfg_found) = match login_file {
            Some(file) => (file.path.clone(), file.found),
            None => (dir.join(LOGIN_CFG_FILE), false),
        };
        let cfg = if cfg_found {
            LoginCfg::read(&cfg_path).map_err(|error| format!("{error:#}"))
        } else {
            Ok(LoginCfg::default())
        };
        let rows = |kind| cfg.as_ref().map(|cfg| cfg_rows(cfg, kind)).unwrap_or_default();
        let original = CFG_KINDS.map(rows);
        Self {
            dir: dir.to_path_buf(),
            report,
            cfg_path,
            cfg_found,
            primary: original[0].clone(),
            backup: original[1].clone(),
            original,
            cfg,
            pending_overwrite: None,
            last_save: None,
        }
    }

    pub fn cfg_error(&self) -> Option<&str> {
        self.cfg.as_ref().err().map(String::as_str)
    }

    pub fn list(&self, kind: CfgServerKind) -> &[String] {
        match kind {
            CfgServerKind::Primary => &self.primary,
            CfgServerKind::Backup => &self.backup,
        }
    }

    pub fn list_mut(&mut self, kind: CfgServerKind) -> &mut Vec<String> {
        match kind {
            CfgServerKind::Primary => &mut self.primary,
            CfgServerKind::Backup => &mut self.backup,
        }
    }

    fn original(&self, kind: CfgServerKind) -> &[String] {
        &self.original[CFG_KINDS.iter().position(|candidate| *candidate == kind).expect("a listed kind")]
    }

    fn kind_edited(&self, kind: CfgServerKind) -> bool {
        self.list(kind) != self.original(kind)
    }

    pub fn is_edited(&self) -> bool {
        CFG_KINDS.into_iter().any(|kind| self.kind_edited(kind))
    }

    /// Back to the login file's lists.
    pub fn reset(&mut self) {
        let [primary, backup] = self.original.clone();
        self.primary = primary;
        self.backup = backup;
    }

    /// The entries the client also reads from other files (`uo.cfg`'s own `LoginServer` lines).
    pub fn other_entries(&self) -> Vec<&LoginServerEntry> {
        let Ok(report) = &self.report else {
            return Vec::new();
        };
        report.entries.iter().filter(|entry| entry.file != self.cfg_path).collect()
    }

    pub fn file_name(&self) -> String {
        self.cfg_path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| LOGIN_CFG_FILE.to_owned())
    }

    /// The login file with the edited lists: a changed list replaces the lines of its kind (`set_servers`), an
    /// unchanged one keeps its lines as they are; comments and other lines stay. The row errors otherwise.
    pub fn edited_cfg(&self) -> Result<LoginCfg, Vec<String>> {
        let mut cfg = self.cfg.clone().map_err(|error| vec![error])?;
        let mut errors = Vec::new();
        for kind in CFG_KINDS.into_iter().filter(|kind| self.kind_edited(*kind)) {
            let mut servers = Vec::new();
            for (index, row) in self.list(kind).iter().enumerate() {
                match check_server_row(row) {
                    Ok(address) => servers.push(address),
                    Err(error) => errors.push(format!("{} {}: {error}", kind_label(kind), index + 1)),
                }
            }
            if errors.is_empty()
                && let Err(error) = cfg.set_servers(kind, &servers)
            {
                errors.push(format!("{}: {error:#}", kind_label(kind)));
            }
        }
        if errors.is_empty() { Ok(cfg) } else { Err(errors) }
    }

    /// Writes the edited login file to `output`, outside `client_dirs` (an existing file only when `overwrite`).
    pub fn save_copy(&self, output: &Path, overwrite: bool, client_dirs: &[PathBuf]) -> eyre::Result<()> {
        check_outside_client_folders(output, client_dirs)?;
        let cfg = self.edited_cfg().map_err(|errors| eyre::eyre!("{}", errors.join("; ")))?;
        write_login_cfg(output, &cfg, overwrite)
    }
}

/// The login files of the client folder the section reads (the data check's folder).
#[derive(Debug, Clone)]
pub enum LoginFolder {
    /// Enhanced Client and Kingdom Reborn use only their executable.
    Executable { dir: PathBuf },
    Classic(Box<ClassicLogin>),
}

impl LoginFolder {
    /// Only reads.
    pub fn load(dir: &Path, family: ClientFamily) -> Self {
        match family {
            ClientFamily::Classic => Self::Classic(Box::new(ClassicLogin::load(dir))),
            ClientFamily::Enhanced | ClientFamily::KingdomReborn => {
                Self::Executable { dir: dir.to_path_buf() }
            }
        }
    }

    pub fn dir(&self) -> &Path {
        match self {
            Self::Executable { dir } => dir,
            Self::Classic(login) => &login.dir,
        }
    }
}
