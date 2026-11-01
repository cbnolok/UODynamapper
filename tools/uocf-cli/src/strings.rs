use std::path::{Path, PathBuf};

use clap::Subcommand;
use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::localized_strings::{
    is_uop_package, language_of_code, language_of_hash, localized_strings_entry_hash, read_classic_cliloc,
    write_classic_cliloc, write_tables, LocalizedStringTable,
};
use uocf::uop_container::package::UopPackage;

const KDL_EXTENSION: &str = "kdl";
/// Hex digits of a UOP hash used as a KDL file stem, for a table of no known language.
const HASH_STEM_DIGITS: usize = 16;
/// Bytes read to tell a UOP package from a Classic cliloc file.
const MAGIC_PROBE_BYTES: usize = 4;

#[derive(Subcommand, Debug)]
pub enum StringsCmd {
    /// Write string tables as KDL: one `<language>.kdl` per table of a LocalizedStrings.uop (`enu.kdl`,
    /// `jpn.kdl`, ...; `<hash>.kdl` for a table of no known language), or `<file name>.kdl` for a Classic
    /// `Cliloc.<lang>` file.
    Export {
        /// Source LocalizedStrings.uop or Classic Cliloc.<lang> file.
        input: PathBuf,
        /// Output folder, created when missing.
        #[arg(long)]
        output: PathBuf,
        /// Languages to export from a LocalizedStrings.uop (repeatable: `enu`, `jpn`, `kor`, `fra`, `deu`,
        /// `esp`, `chs`, `cht`); every table when omitted.
        #[arg(long = "language", value_name = "CODE")]
        languages: Vec<String>,
    },
    /// Read KDL tables back. Into a LocalizedStrings.uop: each `<language>.kdl` (or `<hash>.kdl`) replaces or adds
    /// its table, and a copy of the package is written with the other entries untouched. Into a Classic cliloc
    /// file: one KDL file, written as an uncompressed cliloc.
    Import {
        /// Source LocalizedStrings.uop or Classic Cliloc.<lang> file.
        input: PathBuf,
        /// KDL files.
        #[arg(required = true)]
        kdl: Vec<PathBuf>,
        /// Output file; never the source.
        #[arg(long)]
        output: PathBuf,
    },
}

pub fn run(cmd: StringsCmd) -> eyre::Result<()> {
    match cmd {
        StringsCmd::Export { input, output, languages } => export(&input, &output, &languages),
        StringsCmd::Import { input, kdl, output } => import(&input, &kdl, &output),
    }
}

fn is_uop(path: &Path) -> eyre::Result<bool> {
    use std::io::Read;
    let mut magic = Vec::with_capacity(MAGIC_PROBE_BYTES);
    std::fs::File::open(path)
        .wrap_err_with(|| format!("failed to open {}", path.display()))?
        .take(MAGIC_PROBE_BYTES as u64)
        .read_to_end(&mut magic)?;
    Ok(is_uop_package(&magic))
}

/// The KDL file stem of the table `hash`: its language code, else the hash.
fn table_stem(hash: u64) -> String {
    language_of_hash(hash).map_or_else(|| format!("{hash:0width$X}", width = HASH_STEM_DIGITS), |language| language.code.to_string())
}

/// The table hash a KDL file stem names: a language code or a 16-digit hash.
fn stem_hash(stem: &str) -> eyre::Result<u64> {
    if let Some(language) = language_of_code(stem) {
        return Ok(localized_strings_entry_hash(language.id));
    }
    if stem.len() == HASH_STEM_DIGITS {
        if let Ok(hash) = u64::from_str_radix(stem, 16) {
            return Ok(hash);
        }
    }
    eyre::bail!("`{stem}` is neither a language code (enu, jpn, kor, fra, deu, esp, chs, cht) nor a {HASH_STEM_DIGITS}-digit hash")
}

fn export(input: &Path, output: &Path, languages: &[String]) -> eyre::Result<()> {
    std::fs::create_dir_all(output).wrap_err_with(|| format!("failed to create {}", output.display()))?;
    if !is_uop(input)? {
        eyre::ensure!(languages.is_empty(), "--language selects tables of a LocalizedStrings.uop, not of a cliloc file");
        let table = read_classic_cliloc(input)?;
        let name = input.file_name().ok_or_else(|| eyre::eyre!("{} has no file name", input.display()))?;
        let path = output.join(format!("{}.{KDL_EXTENSION}", name.to_string_lossy()));
        write_kdl(&path, &table)?;
        println!("Wrote {} ({} strings).", path.display(), table.len());
        return Ok(());
    }

    let wanted = languages
        .iter()
        .map(|code| language_of_code(code).map(|language| localized_strings_entry_hash(language.id)).ok_or_else(|| eyre::eyre!("unknown language `{code}`")))
        .collect::<eyre::Result<Vec<u64>>>()?;
    let package = UopPackage::load(input).wrap_err_with(|| format!("failed to load {}", input.display()))?;
    let mut written = 0usize;
    for file in package.iter_files().filter(|file| file.has_size()) {
        let hash = file.filename_hash();
        if !wanted.is_empty() && !wanted.contains(&hash) {
            continue;
        }
        let bytes = file.unpack().wrap_err_with(|| format!("failed to unpack {hash:016X}"))?;
        let table = LocalizedStringTable::from_payload_bytes(&bytes).wrap_err_with(|| format!("failed to parse table {hash:016X}"))?;
        let path = output.join(format!("{}.{KDL_EXTENSION}", table_stem(hash)));
        write_kdl(&path, &table)?;
        println!("{}: {} strings", path.display(), table.len());
        written += 1;
    }
    eyre::ensure!(wanted.is_empty() || written == wanted.len(), "{} has {written} of the {} requested tables", input.display(), wanted.len());
    println!("Wrote {written} KDL files to {}.", output.display());
    Ok(())
}

fn write_kdl(path: &Path, table: &LocalizedStringTable) -> eyre::Result<()> {
    std::fs::write(path, table.to_kdl_string()).wrap_err_with(|| format!("failed to write {}", path.display()))
}

fn read_kdl(path: &Path) -> eyre::Result<LocalizedStringTable> {
    let text = std::fs::read_to_string(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    LocalizedStringTable::from_kdl_str(&path.display().to_string(), &text)
}

fn import(input: &Path, kdl: &[PathBuf], output: &Path) -> eyre::Result<()> {
    if !is_uop(input)? {
        let [path] = kdl else {
            eyre::bail!("a Classic cliloc file takes one KDL file, got {}", kdl.len());
        };
        read_classic_cliloc(input)?;
        let table = read_kdl(path)?;
        let changed = write_classic_cliloc(input, output, &table)?;
        println!(
            "Wrote {} ({} strings){}.",
            output.display(),
            table.len(),
            if changed { "" } else { ", identical to the source" }
        );
        return Ok(());
    }

    let mut tables = Vec::with_capacity(kdl.len());
    for path in kdl {
        let stem = path.file_stem().and_then(|stem| stem.to_str()).ok_or_else(|| eyre::eyre!("{} has no file stem", path.display()))?;
        let hash = stem_hash(stem).wrap_err_with(|| format!("{}", path.display()))?;
        eyre::ensure!(!tables.iter().any(|(other, _)| *other == hash), "{}: a second file for table {stem}", path.display());
        tables.push((hash, read_kdl(path)?));
    }
    let refs: Vec<(u64, &LocalizedStringTable)> = tables.iter().map(|(hash, table)| (*hash, table)).collect();
    let write = write_tables(input, output, &refs)?;
    println!(
        "Read {} KDL files; wrote {}: {} tables replaced, {} added, {} unchanged.",
        kdl.len(),
        output.display(),
        write.replaced,
        write.added,
        write.unchanged
    );
    Ok(())
}
