//! A command-line tool for working with Ultima Online UOP files.

use clap::{Parser, Subcommand};
use color_eyre::eyre::{self, Context};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use uocf_cli::parse_hex_u64;
use uocf::uop::file::{CompressionFlag, UopFile};
use uocf::uop::hash_bruteforce;
use uocf::uop::package::UopPackage;

/// UO Package Tool - A utility for inspecting, hashing, and modifying Ultima Online .uop files.
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
    /// Brute-force crack a UOP hash.
    Crack {
        /// The target hash (hex).
        #[arg(required = true, value_parser = parse_hex_u64)]
        hash: u64,

        /// Known prefix.
        #[arg(long, default_value = "")]
        prefix: String,

        /// Known suffix.
        #[arg(long, default_value = "")]
        suffix: String,

        /// Character set to use.
        #[arg(long, default_value = "abcdefghijklmnopqrstuvwxyz0123456789")]
        charset: String,

        /// Minimum length of the variable part.
        #[arg(long, default_value_t = 1)]
        min_len: usize,

        /// Maximum length of the variable part.
        #[arg(long, default_value_t = 8)]
        max_len: usize,

        /// Number of threads to use (0 for auto).
        #[arg(long, default_value_t = 0)]
        threads: usize,

        /// Cracking method (parallel-simd, parallel-scalar).
        #[arg(long, default_value = "parallel-simd")]
        method: String,
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

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    match &cli.subcommand {
        Commands::Hash { value } => {
            let hash = uocf::uop::hash::hash_file_name_single(value);
            println!("Hash for \"{value}\": 0x{:016x}", hash);
        }
        Commands::Crack {
            hash,
            prefix,
            suffix,
            charset,
            min_len,
            max_len,
            threads,
            method,
        } => {
            println!("Cracking hash 0x{:016x}...", hash);
            println!("  Prefix: \"{}\"", prefix);
            println!("  Suffix: \"{}\"", suffix);
            println!("  Charset: \"{}\"", charset);
            println!("  Length: {} to {}", min_len, max_len);
            println!("  Method: {}", method);

            let stop_signal = Arc::new(AtomicBool::new(false));
            let result = match method.as_str() {
                "parallel-simd" => hash_bruteforce::bruteforce_hash_simd(
                    *hash,
                    prefix,
                    suffix,
                    charset,
                    *min_len,
                    *max_len,
                    *threads,
                    stop_signal,
                ),
                "parallel-scalar" => hash_bruteforce::bruteforce_hash_recursive(
                    *hash,
                    prefix,
                    suffix,
                    charset,
                    *min_len,
                    *max_len,
                    *threads,
                    stop_signal,
                ),
                _ => return Err(eyre::eyre!("Invalid cracking method: {}. Use 'parallel-simd' or 'parallel-scalar'.", method)),
            };

            match result {
                Some(found) => println!("\nFound: {}", found),
                None => println!("\nFailed to find matching string."),
            }
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
