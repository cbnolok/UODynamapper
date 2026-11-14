//! Where a client reads its login server from, and writers of the login server files. See
//! `tools/uo-client-patcher-gui/docs/LOGIN_SERVER.md` for the evidence.
//!
//! - Classic `client.exe`: `uo.cfg`, then the file its `LoginServerFile` key names, or `login.cfg`; both go through
//!   the same key table, so `LoginServer=host,port` and `BackupLoginServer=host,port` lines count in either.
//!   [`LoginCfg`] reads and edits such a file, keeping comments, unknown lines and line endings.
//! - Enhanced Client and Kingdom Reborn: the executable's built-in address
//!   ([`crate::client::login::LoginSites`]). The traced builds never call their XML loader;
//!   login XML files are not inspected or edited here.
//!
//! [`read_login_servers`] reports which files a client folder has and the servers the client would use.

crate::eyre_imports!();
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use crate::client::facet_dimensions::{ClientExeInspection, ClientFamily};
use crate::client::login::LoginSites;
use crate::utils::path::find_file_ignoring_case;

/// The Classic main configuration file, read first.
pub const UO_CFG_FILE: &str = "uo.cfg";
/// The Classic login server file read when `uo.cfg` names none.
pub const LOGIN_CFG_FILE: &str = "login.cfg";
/// The `uo.cfg` key naming another login server file.
pub const LOGIN_SERVER_FILE_KEY: &str = "LoginServerFile";
pub const LOGIN_SERVER_KEY: &str = "LoginServer";
pub const BACKUP_LOGIN_SERVER_KEY: &str = "BackupLoginServer";
pub const ENHANCED_EXE: &str = "UOSA.exe";
pub const KINGDOM_REBORN_EXE: &str = "UOKR.exe";

const CFG_COMMENT: char = ';';
const CFG_KEY_SEPARATOR: char = '=';
const CFG_PORT_SEPARATOR: char = ',';
const ADDRESS_PORT_SEPARATOR: char = ':';
const CRLF: &str = "\r\n";
const LF: &str = "\n";
/// A DNS name is at most 253 characters, in labels of at most 63.
pub const MAX_HOST_NAME_LENGTH: usize = 253;
const MAX_DNS_LABEL_LENGTH: usize = 63;
const IPV4_OCTET_COUNT: usize = 4;
/// The client's port field is 16 bits; 0 is not a port.
pub const MIN_LOGIN_PORT: u16 = 1;

/// What a host string is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    /// Dotted IPv4, `a.b.c.d`.
    Ipv4,
    /// A DNS name, resolved by the client.
    Name,
}

/// Checks a login host: a dotted IPv4 address (four octets 0..=255) or a DNS name (letters, digits, `-` and `.`,
/// labels of 1..=63 characters not starting or ending with `-`).
pub fn validate_login_host(host: &str) -> eyre::Result<HostKind> {
    eyre::ensure!(!host.is_empty(), "the host is empty");
    eyre::ensure!(
        host.len() <= MAX_HOST_NAME_LENGTH,
        "host `{host}`: longer than {MAX_HOST_NAME_LENGTH} characters"
    );
    if host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        let octets: Vec<&str> = host.split('.').collect();
        let valid = octets.len() == IPV4_OCTET_COUNT && octets.iter().all(|octet| !octet.is_empty() && octet.parse::<u8>().is_ok());
        eyre::ensure!(valid, "host `{host}`: not a dotted IPv4 address (four numbers 0..=255)");
        return Ok(HostKind::Ipv4);
    }
    for label in host.split('.') {
        let legal = !label.is_empty()
            && label.len() <= MAX_DNS_LABEL_LENGTH
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        eyre::ensure!(
            legal,
            "host `{host}`: not a dotted IPv4 address or a host name (letters, digits and `-` in dot-separated labels)"
        );
    }
    Ok(HostKind::Name)
}

pub fn validate_login_port(port: u16) -> eyre::Result<()> {
    eyre::ensure!(port >= MIN_LOGIN_PORT, "port {port}: must be {MIN_LOGIN_PORT}..={}", u16::MAX);
    Ok(())
}

fn parse_port(text: &str) -> eyre::Result<u16> {
    let port = text.trim().parse::<u16>().map_err(|_| eyre!("port `{}`: not a number 1..={}", text.trim(), u16::MAX))?;
    validate_login_port(port)?;
    Ok(port)
}

/// A validated login server: host and port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoginServerAddress {
    pub host: String,
    pub port: u16,
}

impl LoginServerAddress {
    pub fn new(host: &str, port: u16) -> eyre::Result<Self> {
        validate_login_host(host)?;
        validate_login_port(port)?;
        Ok(Self { host: host.to_owned(), port })
    }

    /// Parses `host:port`.
    pub fn parse(text: &str) -> eyre::Result<Self> {
        let (host, port) = text
            .trim()
            .rsplit_once(ADDRESS_PORT_SEPARATOR)
            .ok_or_else(|| eyre!("`{text}`: expected host:port, e.g. 127.0.0.1:2593"))?;
        Self::new(host, parse_port(port)?)
    }

    pub fn kind(&self) -> HostKind {
        validate_login_host(&self.host).unwrap_or(HostKind::Name)
    }

    /// The IPv4 address, when the host is one.
    pub fn ipv4(&self) -> Option<Ipv4Addr> {
        self.host.parse().ok()
    }
}

impl std::fmt::Display for LoginServerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{ADDRESS_PORT_SEPARATOR}{}", self.host, self.port)
    }
}

impl std::str::FromStr for LoginServerAddress {
    type Err = eyre::Report;

    fn from_str(text: &str) -> eyre::Result<Self> {
        Self::parse(text)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Classic login.cfg / uo.cfg

/// Which list a `login.cfg` entry belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CfgServerKind {
    /// `LoginServer=host,port`.
    Primary,
    /// `BackupLoginServer=host,port`.
    Backup,
}

impl CfgServerKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Primary => LOGIN_SERVER_KEY,
            Self::Backup => BACKUP_LOGIN_SERVER_KEY,
        }
    }
}

/// One active `LoginServer` or `BackupLoginServer` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfgServerLine {
    pub kind: CfgServerKind,
    /// 1-based line number.
    pub line: usize,
    /// The value after `=`, trimmed.
    pub value: String,
    /// The value parsed as `host,port`, or why it does not parse.
    pub address: Result<LoginServerAddress, String>,
}

/// A Classic `.cfg` file (`login.cfg`, `uo.cfg`): `Key=value` lines and `;` comments. Kept line by line, so
/// [`Self::to_text`] gives back the read text exactly, with the edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCfg {
    lines: Vec<String>,
    newline: &'static str,
    final_newline: bool,
}

impl Default for LoginCfg {
    /// An empty file with the shipped CRLF line endings.
    fn default() -> Self {
        Self { lines: Vec::new(), newline: CRLF, final_newline: true }
    }
}

/// The key and value of an active (not comment, not blank) `Key=value` line.
fn cfg_key_value(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with(CFG_COMMENT) {
        return None;
    }
    let (key, value) = trimmed.split_once(CFG_KEY_SEPARATOR)?;
    Some((key.trim(), value.trim()))
}

/// Parses a `.cfg` server value, `host,port`.
fn parse_cfg_address(value: &str) -> Result<LoginServerAddress, String> {
    let (host, port) =
        value.rsplit_once(CFG_PORT_SEPARATOR).ok_or_else(|| format!("`{value}`: expected host,port"))?;
    let port = parse_port(port).map_err(|error| error.to_string())?;
    LoginServerAddress::new(host.trim(), port).map_err(|error| error.to_string())
}

impl LoginCfg {
    /// Splits `text` into lines, remembering the line ending (CRLF when any line has one) and whether the last line
    /// ends with one.
    pub fn parse(text: &str) -> Self {
        let newline = if text.contains(CRLF) || !text.contains(LF) { CRLF } else { LF };
        if text.is_empty() {
            return Self { lines: Vec::new(), newline, final_newline: true };
        }
        let final_newline = text.ends_with(LF);
        let body = text.strip_suffix(LF).map(|body| body.strip_suffix('\r').unwrap_or(body)).unwrap_or(text);
        let lines = body.split(LF).map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned()).collect();
        Self { lines, newline, final_newline }
    }

    pub fn read(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Ok(Self::parse(&String::from_utf8_lossy(&bytes)))
    }

    pub fn to_text(&self) -> String {
        let mut text = self.lines.join(self.newline);
        if self.final_newline && !self.lines.is_empty() {
            text.push_str(self.newline);
        }
        text
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The value of the first active line with `key` (keys compare ignoring ASCII case).
    pub fn value(&self, key: &str) -> Option<&str> {
        self.lines.iter().filter_map(|line| cfg_key_value(line)).find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    /// The active `LoginServer` and `BackupLoginServer` lines, in file order.
    pub fn servers(&self) -> Vec<CfgServerLine> {
        let mut servers = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            let Some((key, value)) = cfg_key_value(line) else { continue };
            let kind = [CfgServerKind::Primary, CfgServerKind::Backup].into_iter().find(|kind| key.eq_ignore_ascii_case(kind.key()));
            if let Some(kind) = kind {
                servers.push(CfgServerLine { kind, line: index + 1, value: value.to_owned(), address: parse_cfg_address(value) });
            }
        }
        servers
    }

    /// Replaces the active lines of `kind` with one line per server: the new lines take the place of the first old
    /// one (or go at the end), the other old lines are dropped. Comments, blank and other lines stay. A primary
    /// list must not be empty; an empty backup list removes the backup lines.
    pub fn set_servers(&mut self, kind: CfgServerKind, servers: &[LoginServerAddress]) -> eyre::Result<()> {
        eyre::ensure!(kind == CfgServerKind::Backup || !servers.is_empty(), "no login server given");
        let is_kind = |line: &String| cfg_key_value(line).is_some_and(|(key, _)| key.eq_ignore_ascii_case(kind.key()));
        let at = self.lines.iter().position(is_kind).unwrap_or(self.lines.len());
        let new: Vec<String> = servers
            .iter()
            .map(|server| format!("{}{CFG_KEY_SEPARATOR}{}{CFG_PORT_SEPARATOR}{}", kind.key(), server.host, server.port))
            .collect();
        let mut lines = Vec::with_capacity(self.lines.len() + new.len());
        for (index, line) in self.lines.iter().enumerate() {
            if index == at {
                lines.extend(new.iter().cloned());
            }
            if !is_kind(line) {
                lines.push(line.clone());
            }
        }
        if at == self.lines.len() {
            lines.extend(new);
        }
        self.lines = lines;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// What a client folder uses

/// What a checked file is to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginFileRole {
    /// Classic `uo.cfg`.
    UoCfg,
    /// Classic `login.cfg`, or the file `uo.cfg`'s `LoginServerFile` names.
    LoginCfg,
    /// The executable whose built-in address the client uses.
    Executable,
}

/// A file [`read_login_servers`] looked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginConfigFile {
    pub role: LoginFileRole,
    pub path: PathBuf,
    pub found: bool,
    /// The client reads it for the login server.
    pub read_by_client: bool,
}

/// Where an entry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginEntrySource {
    /// A `.cfg` `LoginServer` or `BackupLoginServer` line.
    Cfg(CfgServerKind),
    /// A public address built into the executable (Enhanced Client: one of the hosts and ports the client draws at
    /// random; Kingdom Reborn: its only address).
    BuiltIn,
    /// The Enhanced Client's internal test address, used only when the client's internal flag is set.
    BuiltInInternal,
}

/// One login server entry and the file holding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginServerEntry {
    pub source: LoginEntrySource,
    pub file: PathBuf,
    /// 1-based line (`.cfg` files only).
    pub line: Option<usize>,
    /// A label for built-in addresses.
    pub name: Option<String>,
    /// The address, or why the entry does not parse.
    pub address: Result<LoginServerAddress, String>,
}

/// Where a client folder's client reads its login server from.
#[derive(Debug, Clone)]
pub struct LoginServersReport {
    pub family: ClientFamily,
    pub client_dir: PathBuf,
    /// Every file looked for, found or not.
    pub files: Vec<LoginConfigFile>,
    /// The servers the client uses, in file order.
    pub entries: Vec<LoginServerEntry>,
    pub notes: Vec<String>,
}

fn locate(dir: &Path, name: &str) -> (PathBuf, bool) {
    match find_file_ignoring_case(dir, name) {
        Some(path) => (path, true),
        None => (dir.join(name), false),
    }
}

fn cfg_entries(path: &Path, cfg: &LoginCfg) -> Vec<LoginServerEntry> {
    cfg.servers()
        .into_iter()
        .map(|server| LoginServerEntry {
            source: LoginEntrySource::Cfg(server.kind),
            file: path.to_path_buf(),
            line: Some(server.line),
            name: None,
            address: server.address,
        })
        .collect()
}

/// The path a `LoginServerFile` value names, relative to the client folder. A Windows absolute path (`C:\..`)
/// cannot be resolved here.
fn resolve_login_server_file(client_dir: &Path, value: &str) -> Option<PathBuf> {
    let value = value.trim().trim_matches('"');
    let windows_absolute = value.as_bytes().get(1) == Some(&b':') || value.starts_with('\\');
    if value.is_empty() || windows_absolute {
        return None;
    }
    let relative: PathBuf = value.split(['\\', '/']).filter(|part| !part.is_empty()).collect();
    let (dir, name) = match (relative.parent(), relative.file_name()) {
        (Some(parent), Some(name)) => (client_dir.join(parent), name.to_string_lossy().into_owned()),
        _ => return None,
    };
    Some(locate(&dir, &name).0)
}

/// The built-in entries of an Enhanced Client or Kingdom Reborn executable.
pub fn built_in_entries(exe: &Path, family: ClientFamily, sites: &LoginSites) -> Vec<LoginServerEntry> {
    let entry = |source, name: String, host: String, port: u16| LoginServerEntry {
        source,
        file: exe.to_path_buf(),
        line: None,
        name: Some(name),
        address: LoginServerAddress::new(&host, port).map_err(|error| error.to_string()),
    };
    let mut entries = Vec::new();
    for (index, (host, port)) in sites.public_servers().into_iter().enumerate() {
        let name = match sites.hosts.len().max(sites.ports.len()) {
            1 => "built-in".to_owned(),
            _ => format!("built-in {} (rand {})", index + 1, if index == 0 { "odd" } else { "even" }),
        };
        entries.push(entry(LoginEntrySource::BuiltIn, name, host, port));
    }
    let fixed = &sites.fixed;
    let (source, name) = match family {
        ClientFamily::KingdomReborn => (LoginEntrySource::BuiltIn, "built-in".to_owned()),
        _ => (LoginEntrySource::BuiltInInternal, "internal test (internal flag only)".to_owned()),
    };
    entries.push(entry(source, name, fixed.ip.to_string(), fixed.port));
    entries
}

/// Where the client of `client_dir` reads its login server from: the files it looks for (found or not) and the
/// entries it would use. Classic: `uo.cfg`, then the `LoginServerFile` it names or `login.cfg` (file names compare
/// ignoring case). Enhanced Client and Kingdom Reborn: the built-in address of `UOSA.exe` / `UOKR.exe`; the
/// dormant XML login files are ignored. Only reads.
pub fn read_login_servers(client_dir: &Path, family: ClientFamily) -> eyre::Result<LoginServersReport> {
    eyre::ensure!(client_dir.is_dir(), "{} is not a folder", client_dir.display());
    let mut report = LoginServersReport {
        family,
        client_dir: client_dir.to_path_buf(),
        files: Vec::new(),
        entries: Vec::new(),
        notes: Vec::new(),
    };
    match family {
        ClientFamily::Classic => {
            let (uo_cfg, found) = locate(client_dir, UO_CFG_FILE);
            report.files.push(LoginConfigFile { role: LoginFileRole::UoCfg, path: uo_cfg.clone(), found, read_by_client: found });
            let mut login_file = None;
            if found {
                let cfg = LoginCfg::read(&uo_cfg)?;
                report.entries.extend(cfg_entries(&uo_cfg, &cfg));
                if let Some(value) = cfg.value(LOGIN_SERVER_FILE_KEY) {
                    match resolve_login_server_file(client_dir, value) {
                        Some(path) => login_file = Some(path),
                        None => report.notes.push(format!(
                            "{LOGIN_SERVER_FILE_KEY}={value} in {}: not a path inside the client folder; it is not read here",
                            uo_cfg.display()
                        )),
                    }
                }
            }
            let path = login_file.unwrap_or_else(|| locate(client_dir, LOGIN_CFG_FILE).0);
            let found = path.is_file();
            report.files.push(LoginConfigFile { role: LoginFileRole::LoginCfg, path: path.clone(), found, read_by_client: found });
            if found {
                report.entries.extend(cfg_entries(&path, &LoginCfg::read(&path)?));
            }
            if report.entries.is_empty() {
                report.notes.push("no LoginServer line found: the client has no login server".to_owned());
            }
        }
        ClientFamily::Enhanced | ClientFamily::KingdomReborn => {
            let exe_name = if family == ClientFamily::Enhanced { ENHANCED_EXE } else { KINGDOM_REBORN_EXE };
            report.notes.push(format!(
                "the {} builds traced never call their LoginServers*.xml loader: the client connects to the address \
                 built into {exe_name}",
                family.label()
            ));
            let (exe, found) = locate(client_dir, exe_name);
            report.files.push(LoginConfigFile { role: LoginFileRole::Executable, path: exe.clone(), found, read_by_client: found });
            if found {
                let bytes = std::fs::read(&exe).wrap_err_with(|| format!("read {}", exe.display()))?;
                let inspection = ClientExeInspection::inspect(&bytes).wrap_err_with(|| format!("inspect {}", exe.display()))?;
                match &inspection.login {
                    Some(sites) => report.entries.extend(built_in_entries(&exe, inspection.family, sites)),
                    None => report.notes.push(format!("{}: the login address code was not located", exe.display())),
                }
            }
        }
    }
    Ok(report)
}

/// Fails unless `output` may be written: a new path, or an existing file when `overwrite`.
fn check_config_output(output: &Path, overwrite: bool) -> eyre::Result<()> {
    if output.symlink_metadata().is_ok() {
        eyre::ensure!(overwrite && output.is_file(), "{} exists: write to a new path or allow overwriting", output.display());
    }
    Ok(())
}

/// Writes `cfg` to `output` (a new file, or an existing one when `overwrite`).
pub fn write_login_cfg(output: &Path, cfg: &LoginCfg, overwrite: bool) -> eyre::Result<()> {
    check_config_output(output, overwrite)?;
    std::fs::write(output, cfg.to_text()).wrap_err_with(|| format!("write {}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIPPED_LOGIN_CFG: &str = ";Loginservers for Ultima Online\r\n;Do not edit this file or patching will fail!  \
        Always save a backup.\r\nLoginServer=127.0.0.1,2593\r\n;LoginServer=107.23.176.74,7775\r\nUnknownKey=1\r\n\
        BackupLoginServer=backup.example.com,7776\r\nLoginServer=10.0.0.2,2594";

    fn address(text: &str) -> LoginServerAddress {
        LoginServerAddress::parse(text).unwrap()
    }

    #[test]
    fn hosts_and_ports_are_validated() {
        for good in ["127.0.0.1", "255.255.255.255", "login.owo.com", "a-b.c", "localhost"] {
            assert!(validate_login_host(good).is_ok(), "{good}");
        }
        assert_eq!(validate_login_host("1.2.3.4").unwrap(), HostKind::Ipv4);
        assert_eq!(validate_login_host("login.owo.com").unwrap(), HostKind::Name);
        for bad in ["", "1.2.3", "1.2.3.256", "1..2.3", "-a.com", "a_b.com", "a b", "x.-y", &"a".repeat(MAX_HOST_NAME_LENGTH + 1)] {
            assert!(validate_login_host(bad).is_err(), "{bad}");
        }
        assert!(validate_login_port(0).is_err() && validate_login_port(u16::MAX).is_ok());
        assert_eq!(address("example.com:2593"), LoginServerAddress { host: "example.com".into(), port: 2593 });
        for bad in ["example.com", "example.com:0", "example.com:65536", ":2593", "a b:1"] {
            assert!(LoginServerAddress::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(address("1.2.3.4:5").to_string(), "1.2.3.4:5");
        assert_eq!(address("1.2.3.4:5").ipv4(), Some(Ipv4Addr::new(1, 2, 3, 4)));
    }

    #[test]
    fn login_cfg_round_trips_and_keeps_unknown_lines() {
        let cfg = LoginCfg::parse(SHIPPED_LOGIN_CFG);
        assert_eq!(cfg.to_text(), SHIPPED_LOGIN_CFG, "read and written back unchanged, no final newline");
        let servers = cfg.servers();
        assert_eq!(servers.iter().map(|server| (server.kind, server.line)).collect::<Vec<_>>(), [
            (CfgServerKind::Primary, 3),
            (CfgServerKind::Backup, 6),
            (CfgServerKind::Primary, 7)
        ]);
        assert_eq!(servers[1].address.as_ref().unwrap(), &address("backup.example.com:7776"));
        for text in ["", "A=1\n", "A=1\r\nB=2\r\n", "\r\n"] {
            assert_eq!(LoginCfg::parse(text).to_text(), text, "{text:?}");
        }

        let mut edited = cfg.clone();
        edited.set_servers(CfgServerKind::Primary, &[address("192.168.1.2:2593"), address("shard.example.org:2594")]).unwrap();
        let text = edited.to_text();
        assert_eq!(
            text,
            ";Loginservers for Ultima Online\r\n;Do not edit this file or patching will fail!  Always save a backup.\r\n\
             LoginServer=192.168.1.2,2593\r\nLoginServer=shard.example.org,2594\r\n;LoginServer=107.23.176.74,7775\r\n\
             UnknownKey=1\r\nBackupLoginServer=backup.example.com,7776"
        );
        edited.set_servers(CfgServerKind::Backup, &[]).unwrap();
        assert!(edited.servers().iter().all(|server| server.kind == CfgServerKind::Primary));
        assert!(edited.set_servers(CfgServerKind::Primary, &[]).is_err());

        let mut empty = LoginCfg::default();
        empty.set_servers(CfgServerKind::Primary, &[address("127.0.0.1:2593")]).unwrap();
        assert_eq!(empty.to_text(), "LoginServer=127.0.0.1,2593\r\n");
        let mut unix = LoginCfg::parse("LoginServerFile=other.cfg\n");
        assert_eq!(unix.value("loginserverfile"), Some("other.cfg"));
        unix.set_servers(CfgServerKind::Backup, &[address("1.1.1.1:1")]).unwrap();
        assert_eq!(unix.to_text(), "LoginServerFile=other.cfg\nBackupLoginServer=1.1.1.1,1\n");
        let bad = LoginCfg::parse("LoginServer=nohost\r\nLoginServer=a.b,0\r\n");
        assert!(bad.servers().iter().all(|server| server.address.is_err()));
    }

    #[test]
    fn a_classic_folder_reads_uo_cfg_then_its_login_server_file() {
        let dir = std::env::temp_dir().join(format!("uocf_login_servers_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cfg")).unwrap();
        std::fs::write(dir.join("Uo.cfg"), "CacheSize=25\r\n").unwrap();
        std::fs::write(dir.join("Login.cfg"), SHIPPED_LOGIN_CFG).unwrap();
        let report = read_login_servers(&dir, ClientFamily::Classic).unwrap();
        assert_eq!(report.files.len(), 2);
        assert!(report.files.iter().all(|file| file.found && file.read_by_client));
        assert_eq!(report.files[1].path, dir.join("Login.cfg"), "found ignoring case");
        assert_eq!(report.entries.len(), 3);

        std::fs::write(dir.join("Uo.cfg"), "LoginServerFile=cfg\\Shard.cfg\r\n").unwrap();
        std::fs::write(dir.join("cfg").join("shard.cfg"), "LoginServer=shard.example.org,2593\r\n").unwrap();
        let report = read_login_servers(&dir, ClientFamily::Classic).unwrap();
        assert_eq!(report.files[1].path, dir.join("cfg").join("shard.cfg"));
        assert_eq!(report.entries[0].address.as_ref().unwrap(), &address("shard.example.org:2593"));

        // Dormant XML files, including malformed ones, are never inspected.
        std::fs::write(dir.join("LoginServersUser.xml"), "<invalid").unwrap();
        let report = read_login_servers(&dir, ClientFamily::Enhanced).unwrap();
        assert!(report.entries.is_empty());
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].role, LoginFileRole::Executable);
        assert!(report.files.iter().all(|file| !file.read_by_client));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
