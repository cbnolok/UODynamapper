use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use color_eyre::eyre::{self, eyre};
use uocf::client::login_servers::{
    read_login_servers, write_login_cfg, CfgServerKind, LoginCfg, LoginEntrySource,
    LoginFileRole, LoginServerAddress, LoginServerEntry, ENHANCED_EXE, KINGDOM_REBORN_EXE,
    LOGIN_CFG_FILE, UO_CFG_FILE,
};
use uocf::client::facet_dimensions::ClientFamily;
use uocf::utils::path::find_file_ignoring_case;

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
    /// address built into UOSA.exe / UOKR.exe. Only reads.
    Show {
        client_dir: PathBuf,
        /// The client family; by default told by the executable in the folder (UOSA.exe, UOKR.exe, client.exe).
        #[arg(long, value_enum)]
        family: Option<FamilyArg>,
    },
    /// Write a login server file with new servers, keeping the input's comments, unknown lines and other entries.
    /// The input is a Classic client folder (the login.cfg or LoginServerFile it reads) or a .cfg file.
    /// Enhanced Client and Kingdom Reborn addresses are changed with `exe patch`. Only `--output` is written.
    Set {
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// A server, `host:port`. Repeat for more; replaces the LoginServer lines.
        #[arg(long = "server", value_name = "HOST:PORT", required = true)]
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
    for note in &report.notes {
        println!("note      {note}");
    }
    Ok(())
}

/// The Classic configuration file `set` edits for the input.
fn input_file(input: &Path, family: Option<FamilyArg>) -> eyre::Result<Option<PathBuf>> {
    eyre::ensure!(
        !input.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("xml")),
        "LoginServers XML is unsupported: use `uocf-cli exe patch --login-server` for Enhanced Client or Kingdom Reborn"
    );
    eyre::ensure!(
        family.is_none_or(|family| matches!(family, FamilyArg::Classic)),
        "login set edits Classic configuration only: use `uocf-cli exe patch --login-server` for this client"
    );
    if input.is_file() {
        return Ok(Some(input.to_path_buf()));
    }
    eyre::ensure!(input.is_dir(), "{}: not a file or folder", input.display());
    let family = detect_family(input, family)?;
    match family {
        ClientFamily::Classic => {
            let report = read_login_servers(input, family)?;
            let login = report.files.iter().find(|file| file.role == LoginFileRole::LoginCfg).filter(|file| file.found);
            Ok(login.map(|file| file.path.clone()))
        }
        ClientFamily::Enhanced | ClientFamily::KingdomReborn => {
            eyre::bail!("login set edits Classic configuration only: use `uocf-cli exe patch --login-server` for this client")
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
    let file = input_file(input, family)?;
    eyre::ensure!(
        !output.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("xml")),
        "login set writes Classic configuration files; choose a .cfg output"
    );
    let servers = servers.iter().map(|server| LoginServerAddress::parse(server)).collect::<eyre::Result<Vec<_>>>()?;
    match file.as_deref() {
        Some(path) => println!("input     {}", path.display()),
        None => println!("input     none found: a new file"),
    }
    let mut cfg = match &file {
        Some(path) => LoginCfg::read(path)?,
        None => LoginCfg::default(),
    };
    cfg.set_servers(CfgServerKind::Primary, &servers)?;
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_edits_classic_files_and_refuses_xml_and_executable_clients() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("uocf_cli_login_{nonce}"));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("login.cfg");
        let output = dir.join("edited.cfg");
        let servers = ["shard.example:2593".to_owned()];
        std::fs::write(&input, ";keep\r\nUnknown=1\r\nLoginServer=old.example,7775\r\n").unwrap();
        set(&input, &output, &servers, &[], None, false).unwrap();
        assert_eq!(std::fs::read_to_string(&output).unwrap(), ";keep\r\nUnknown=1\r\nLoginServer=shard.example,2593\r\n");

        let xml = dir.join("LoginServersUser.xml");
        std::fs::write(&xml, "<invalid").unwrap();
        let refused = dir.join("refused.cfg");
        assert!(set(&xml, &refused, &servers, &[], None, false).unwrap_err().to_string().contains("XML"));
        for family in [FamilyArg::Enhanced, FamilyArg::KingdomReborn] {
            assert!(set(&dir, &refused, &servers, &[], Some(family), false).unwrap_err().to_string().contains("exe patch"));
        }
        assert!(!refused.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
