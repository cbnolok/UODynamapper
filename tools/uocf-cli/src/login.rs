use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use color_eyre::eyre::{self, eyre};
use uocf::client_config::login_servers::{
    read_login_servers, write_login_cfg, write_login_servers_xml, CfgServerKind, LoginCfg, LoginEntrySource,
    LoginFileRole, LoginServerAddress, LoginServerEntry, LoginServersXml, ENHANCED_EXE, KINGDOM_REBORN_EXE,
    LOGIN_CFG_FILE, LOGIN_SERVERS_USER_XML, UO_CFG_FILE,
};
use uocf::client_exe::facet_dimensions::ClientFamily;
use uocf::utils::path::find_file_ignoring_case;

/// `name=host:port`: an XML entry title and its address.
const NAME_SEPARATOR: char = '=';
const XML_EXTENSION: &str = "xml";
const CLASSIC_EXE: &str = "client.exe";

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum FamilyArg {
    Enhanced,
    KingdomReborn,
    Classic,
}

impl From<FamilyArg> for ClientFamily {
    fn from(family: FamilyArg) -> Self {
        match family {
            FamilyArg::Enhanced => ClientFamily::Enhanced,
            FamilyArg::KingdomReborn => ClientFamily::KingdomReborn,
            FamilyArg::Classic => ClientFamily::Classic,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum LoginCmd {
    /// Print where a client folder's client reads its login server from: the files looked for (found or not) and the
    /// servers. Classic: uo.cfg, then its LoginServerFile or login.cfg. Enhanced Client and Kingdom Reborn: the
    /// address built into UOSA.exe / UOKR.exe (their LoginServers*.xml loader is never called; the files are listed
    /// as unused). Only reads.
    Show {
        client_dir: PathBuf,
        /// The client family; by default told by the executable in the folder (UOSA.exe, UOKR.exe, client.exe).
        #[arg(long, value_enum)]
        family: Option<FamilyArg>,
    },
    /// Write a login server file with new servers, keeping the input's comments, unknown lines and other entries.
    /// The input is a client folder (Classic: the login.cfg or LoginServerFile it reads; Enhanced Client / Kingdom
    /// Reborn: LoginServersUser.xml, or a new one) or a file (`.xml`: LoginServers XML, else a .cfg). Only `--output`
    /// is written.
    Set {
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// A server, `host:port` (.cfg) or `name=host:port` (XML: the entry title). Repeat for more. In a .cfg file
        /// the servers replace the LoginServer lines.
        #[arg(long = "server", value_name = "[NAME=]HOST:PORT", required = true)]
        servers: Vec<String>,
        /// A .cfg BackupLoginServer, `host:port`; replaces the backup lines. Repeatable.
        #[arg(long = "backup-server", value_name = "HOST:PORT")]
        backup_servers: Vec<String>,
        /// The client family of a client folder input; by default told by its executable.
        #[arg(long, value_enum)]
        family: Option<FamilyArg>,
        /// Replace `--output` when it exists.
        #[arg(long)]
        overwrite: bool,
    },
}

pub fn run(cmd: LoginCmd) -> eyre::Result<()> {
    match cmd {
        LoginCmd::Show { client_dir, family } => show(&client_dir, family),
        LoginCmd::Set { input, output, servers, backup_servers, family, overwrite } => {
            set(&input, &output, &servers, &backup_servers, family, overwrite)
        }
    }
}

/// The family of the client in `dir`, by the executable it holds.
fn detect_family(dir: &Path, family: Option<FamilyArg>) -> eyre::Result<ClientFamily> {
    if let Some(family) = family {
        return Ok(family.into());
    }
    let has = |name: &str| find_file_ignoring_case(dir, name).is_some();
    if has(ENHANCED_EXE) {
        Ok(ClientFamily::Enhanced)
    } else if has(KINGDOM_REBORN_EXE) {
        Ok(ClientFamily::KingdomReborn)
    } else if has(CLASSIC_EXE) || has(LOGIN_CFG_FILE) || has(UO_CFG_FILE) {
        Ok(ClientFamily::Classic)
    } else {
        Err(eyre!(
            "{}: no {ENHANCED_EXE}, {KINGDOM_REBORN_EXE}, {CLASSIC_EXE}, {LOGIN_CFG_FILE} or {UO_CFG_FILE}: pass --family",
            dir.display()
        ))
    }
}

fn source_label(source: LoginEntrySource) -> &'static str {
    match source {
        LoginEntrySource::Cfg(CfgServerKind::Primary) => "LoginServer",
        LoginEntrySource::Cfg(CfgServerKind::Backup) => "BackupLoginServer",
        LoginEntrySource::Xml => "LoginServer (XML)",
        LoginEntrySource::BuiltIn => "built-in",
        LoginEntrySource::BuiltInInternal => "built-in internal",
    }
}

fn print_entry(entry: &LoginServerEntry) {
    let address = match &entry.address {
        Ok(address) => address.to_string(),
        Err(error) => format!("INVALID: {error}"),
    };
    let at = match entry.line {
        Some(line) => format!("{}:{line}", entry.file.display()),
        None => entry.file.display().to_string(),
    };
    let name = entry.name.as_deref().map(|name| format!(" {name}")).unwrap_or_default();
    println!("  {:<18} {address:<28}{name}  [{at}]", source_label(entry.source));
}

fn show(client_dir: &Path, family: Option<FamilyArg>) -> eyre::Result<()> {
    let family = detect_family(client_dir, family)?;
    let report = read_login_servers(client_dir, family)?;
    println!("family    {}", family.label());
    for file in &report.files {
        let role = match file.role {
            LoginFileRole::UoCfg => "uo.cfg",
            LoginFileRole::LoginCfg => "login file",
            LoginFileRole::LoginServersXml => "xml",
            LoginFileRole::Executable => "exe",
        };
        let state = match (file.found, file.read_by_client) {
            (false, _) => "absent",
            (true, true) => "read by the client",
            (true, false) => "present, NOT read by the client",
        };
        println!("file      {role:<10} {} ({state})", file.path.display());
    }
    println!("servers   {} used by the client:", report.entries.len());
    report.entries.iter().for_each(print_entry);
    if !report.unused_entries.is_empty() {
        println!("unused    {} entries of files the client does not read:", report.unused_entries.len());
        report.unused_entries.iter().for_each(print_entry);
    }
    for note in &report.notes {
        println!("note      {note}");
    }
    Ok(())
}

/// Parses `[name=]host:port`.
fn parse_named(text: &str) -> eyre::Result<(Option<String>, LoginServerAddress)> {
    match text.split_once(NAME_SEPARATOR) {
        Some((name, address)) => Ok((Some(name.trim().to_owned()), LoginServerAddress::parse(address)?)),
        None => Ok((None, LoginServerAddress::parse(text)?)),
    }
}

/// The file `set` edits for the input, and whether it is XML.
fn input_file(input: &Path, family: Option<FamilyArg>) -> eyre::Result<(Option<PathBuf>, bool)> {
    if input.is_file() {
        let xml = input.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(XML_EXTENSION));
        return Ok((Some(input.to_path_buf()), xml));
    }
    eyre::ensure!(input.is_dir(), "{}: not a file or folder", input.display());
    let family = detect_family(input, family)?;
    match family {
        ClientFamily::Classic => {
            let report = read_login_servers(input, family)?;
            let login = report.files.iter().find(|file| file.role == LoginFileRole::LoginCfg).filter(|file| file.found);
            Ok((login.map(|file| file.path.clone()), false))
        }
        ClientFamily::Enhanced | ClientFamily::KingdomReborn => {
            Ok((find_file_ignoring_case(input, LOGIN_SERVERS_USER_XML), true))
        }
    }
}

fn set(
    input: &Path,
    output: &Path,
    servers: &[String],
    backup_servers: &[String],
    family: Option<FamilyArg>,
    overwrite: bool,
) -> eyre::Result<()> {
    let (file, xml) = input_file(input, family)?;
    let servers = servers.iter().map(|server| parse_named(server)).collect::<eyre::Result<Vec<_>>>()?;
    match file.as_deref() {
        Some(path) => println!("input     {}", path.display()),
        None => println!("input     none found: a new file"),
    }
    if xml {
        eyre::ensure!(backup_servers.is_empty(), "--backup-server is a .cfg key; LoginServers XML has no backup list");
        let mut document = match &file {
            Some(path) => LoginServersXml::read(path)?,
            None => LoginServersXml::new_document(),
        };
        for (name, address) in &servers {
            let name = name.as_deref().ok_or_else(|| eyre!("`{address}`: an XML entry needs a title, name=host:port"))?;
            document.set_server(name, address)?;
        }
        write_login_servers_xml(output, &document, overwrite)?;
        println!("wrote     {} ({} LoginServer entries)", output.display(), document.servers()?.len());
        println!(
            "note      the traced Enhanced Client and Kingdom Reborn builds never load LoginServers*.xml: to change the \
             server, patch a copy of UOSA.exe or UOKR.exe (`uocf-cli exe patch --login-server`)"
        );
    } else {
        let mut cfg = match &file {
            Some(path) => LoginCfg::read(path)?,
            None => LoginCfg::default(),
        };
        if let Some((Some(name), _)) = servers.iter().find(|(name, _)| name.is_some()) {
            eyre::bail!("`{name}=`: login.cfg entries have no names; give host:port");
        }
        let primary: Vec<LoginServerAddress> = servers.into_iter().map(|(_, address)| address).collect();
        cfg.set_servers(CfgServerKind::Primary, &primary)?;
        if !backup_servers.is_empty() {
            let backup = backup_servers.iter().map(|text| LoginServerAddress::parse(text)).collect::<eyre::Result<Vec<_>>>()?;
            cfg.set_servers(CfgServerKind::Backup, &backup)?;
        }
        write_login_cfg(output, &cfg, overwrite)?;
        println!("wrote     {}", output.display());
        for server in cfg.servers() {
            let address = server.address.map(|address| address.to_string()).unwrap_or_else(|error| format!("INVALID: {error}"));
            println!("  line {:<4} {:<18} {address}", server.line, server.kind.key());
        }
    }
    Ok(())
}
