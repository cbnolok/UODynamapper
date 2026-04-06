//! A command-line tool for working with Ultima Online UOP files.

use clap::{Parser, Subcommand};
use color_eyre::eyre::{self, Context};
use std::fs;
use std::path::PathBuf;
use uocf::uop::file::{CompressionFlag, UopFile};
use uocf::uop::package::UopPackage;

/// UOP Tool - A utility for interacting with UOP files.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    subcommand: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Calculate the UOP hash for a given string.
    Hash {
        /// The string to hash.
        #[arg(required = true)]
        value: String,
    },
    /// Replace a file in the UOP package.
    Replace {
        /// The path to the UOP file.
        #[arg(required = true)]
        uop_file: PathBuf,

        /// The hash of the file to replace.
        #[arg(required = true, value_parser = parse_hex_u64)]
        hash: u64,

        /// The path to the new file.
        #[arg(required = true)]
        new_file: PathBuf,
    },
    /// Rebuild the UOP package, recompressing all files.
    Rebuild {
        /// The path to the UOP file.
        #[arg(required = true)]
        uop_file: PathBuf,
    },
}

/// Parses a hexadecimal string into a u64.
fn parse_hex_u64(s: &str) -> Result<u64, std::num::ParseIntError> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(s, 16)
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    match &cli.subcommand {
        Commands::Hash { value } => {
            let hash = uocf::uop::hash::hash_file_name_single(value);
            println!("Hash for \"{value}\": 0x{:016x}", hash);
        }
        Commands::Replace {
            uop_file,
            hash,
            new_file,
        } => {
            println!(
                "Replacing file with hash 0x{:016x} in {}",
                *hash,
                uop_file.display()
            );

            let mut uop = UopPackage::load(uop_file)
                .with_context(|| format!("Failed to load UOP file: {}", uop_file.display()))?;

            let old_uop_file: &mut UopFile = uop.get_file_by_hash_mut(*hash).ok_or_else(|| {
                eyre::eyre!("File with hash 0x{:016x} not found in UOP package", *hash)
            })?;

            let mut new_file_data = fs::File::open(new_file)
                .with_context(|| format!("Failed to open new file: {}", new_file.display()))?;

            let new_uop_file =
                UopFile::new().create_file(&mut new_file_data, *hash, CompressionFlag::Zlib)?;

            *old_uop_file = new_uop_file;

            let temp_path = uop_file.with_extension("uop.temp");
            uop.finalize_and_save(&temp_path)
                .with_context(|| "Failed to save temporary UOP file")?;

            fs::rename(&temp_path, uop_file)
                .with_context(|| "Failed to replace old UOP file with the new one")?;

            println!("Successfully replaced file and saved the UOP package.");
        }
        Commands::Rebuild { uop_file } => {
            println!("Rebuilding UOP package: {}", uop_file.display());

            let mut uop = UopPackage::load(uop_file)
                .with_context(|| format!("Failed to load UOP file: {}", uop_file.display()))?;

            uop.recompress(flate2::Compression::best())
                .with_context(|| "Failed to recompress UOP package")?;

            let temp_path = uop_file.with_extension("uop.temp");
            uop.finalize_and_save(&temp_path)
                .with_context(|| "Failed to save temporary UOP file")?;

            fs::rename(&temp_path, uop_file)
                .with_context(|| "Failed to replace old UOP file with the new one")?;

            println!("Successfully rebuilt and saved the UOP package.");
        }
    }

    Ok(())
}
