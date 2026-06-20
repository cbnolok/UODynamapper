//! UO Package Tool — inspect, modify, and populate dictionaries for .uop files.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use color_eyre::eyre::{self, Context};
use serde::Deserialize;
use uocf::uop_container::file::{CompressionFlag, UopFile};
use uocf::uop_container::hash_bruteforce;
use uocf::uop_container::hash_dictionary::HashDictionary;
use uocf::uop_container::package::UopPackage;
use uocf::uop_container::template::UopTemplate;
use udd_logging::progress::{ProgressBar, ProgressStyle};

use uoppackage_cli::parse_hex_u64;

/// UO Package Tool — inspect, hash, modify, and populate dictionaries for Ultima Online .uop files.
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
    /// Extract all files from a UOP package.
    Extract {
        /// The path to the UOP file.
        #[arg(required = true)]
        uop_file: PathBuf,
        /// The directory to extract files into.
        #[arg(required = true)]
        out_dir: PathBuf,
        /// Optional path to a string dictionary UOP to resolve hashes.
        #[arg(long)]
        dictionary: Option<PathBuf>,
    },
    /// Merge raw UOP string/hash dictionaries in DIC format.
    MergeDic {
        /// The merged dictionary output path.
        #[arg(short, long)]
        output: PathBuf,
        /// Raw DIC dictionary files to merge.
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Populate a UOP hash dictionary using TOML-configured templates.
    PopulateDict {
        /// Path to the TOML configuration file.
        #[arg(short, long)]
        config: PathBuf,
        /// Path to the UOP files directory.
        #[arg(short, long)]
        uop_dir: PathBuf,
        /// Path to the output dictionary file (.dic).
        #[arg(short, long, default_value = "Dictionary.dic")]
        output: PathBuf,
        /// Path to an existing .dic dictionary to load first (optional).
        #[arg(short, long)]
        input: Option<PathBuf>,
    },
}

#[derive(Deserialize, Debug)]
struct PopulatorConfig {
    #[serde(flatten)]
    packages: HashMap<String, PackageConfig>,
}

#[derive(Deserialize, Debug)]
struct PackageConfig {
    candidates: Vec<String>,
    range: Option<[u64; 2]>,
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let _ = udd_logging::install_tracing_indicatif_logger();
    let cli = Cli::parse();

    match &cli.subcommand {
        Commands::Hash { value } => {
            let hash = uocf::uop_container::hash::hash_file_name_single(value);
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
            if charset.is_empty() {
                return Err(eyre::eyre!("Charset must not be empty."));
            }
            if min_len > max_len {
                return Err(eyre::eyre!(
                    "Minimum length ({}) must not be greater than maximum length ({}).",
                    min_len,
                    max_len
                ));
            }

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
                _ => {
                    return Err(eyre::eyre!(
                        "Invalid cracking method: {}. Use 'parallel-simd' or 'parallel-scalar'.",
                        method
                    ));
                }
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

            save_package_atomically(&mut uop, uop_file)?;

            println!("Successfully replaced file and saved the UOP package.");
        }
        Commands::Rebuild { uop_file } => {
            println!("Rebuilding UOP package: {}", uop_file.display());

            let mut uop = UopPackage::load(uop_file)
                .with_context(|| format!("Failed to load UOP file: {}", uop_file.display()))?;

            uop.recompress(flate2::Compression::best())
                .with_context(|| "Failed to recompress UOP package")?;

            save_package_atomically(&mut uop, uop_file)?;

            println!("Successfully rebuilt and saved the UOP package.");
        }
        Commands::Extract {
            uop_file,
            out_dir,
            dictionary,
        } => {
            println!(
                "Extracting {} to {}...",
                uop_file.display(),
                out_dir.display()
            );
            fs::create_dir_all(out_dir).with_context(|| "Failed to create output directory")?;

            let uop = UopPackage::load(uop_file)
                .with_context(|| format!("Failed to load UOP file: {}", uop_file.display()))?;

            let mut name_map = std::collections::HashMap::new();
            if let Some(dict_path) = dictionary {
                println!("Loading dictionary: {}...", dict_path.display());
                let dict = uocf::enhanced::string_dictionary::UoStringDictionary::load(dict_path)
                    .with_context(|| {
                        format!("Failed to load dictionary: {}", dict_path.display())
                    })?;
                for i in 0.. {
                    if let Some(s) = dict.get_string(i) {
                        let h = uocf::uop_container::hash::hash_file_name_single(s);
                        name_map.insert(h, s.to_string());
                    } else {
                        break;
                    }
                }
                println!("Loaded {} strings from dictionary.", name_map.len());
            }

            // TODO: add it only for Texture.uop
            // Add common EC guesses. TODO: review the string generating code with plausible bigger numerical strings.
            for i in 0..65536 {
                let s1 = format!("build/worldart/{:08}.dds", i);
                name_map.insert(uocf::uop_container::hash::hash_file_name_single(&s1), s1);
                let s2 = format!("build/worldart/{:08}.tga", i);
                name_map.insert(uocf::uop_container::hash::hash_file_name_single(&s2), s2);
                let s3 = format!("build/tileartlegacy/{:08}.dds", i);
                name_map.insert(uocf::uop_container::hash::hash_file_name_single(&s3), s3);
                let s4 = format!("build/tileartlegacy/{:08}.tga", i);
                name_map.insert(uocf::uop_container::hash::hash_file_name_single(&s4), s4);
            }

            let mut count = 0;
            for file in uop.iter_files() {
                let hash = file.filename_hash();
                if !file.has_size() {
                    continue;
                }

                let data = file
                    .unpack()
                    .with_context(|| format!("Failed to unpack file with hash 0x{:016x}", hash))?;

                // Detect extension
                let ext = if data.starts_with(b"DDS ") {
                    "dds"
                } else if data.starts_with(b"\x89PNG") {
                    "png"
                } else if data.starts_with(b"BM") {
                    "bmp"
                } else {
                    "dat"
                };

                let out_path = if let Some(name) = name_map.get(&hash) {
                    let p = safe_extract_path(out_dir, name).with_context(|| {
                        format!(
                            "Dictionary name for hash 0x{:016x} is not a safe relative path: {}",
                            hash, name
                        )
                    })?;
                    if let Some(parent) = p.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    p
                } else {
                    out_dir.join(format!("0x{:016x}.{}", hash, ext))
                };

                fs::write(&out_path, data)
                    .with_context(|| format!("Failed to write file: {}", out_path.display()))?;
                count += 1;
            }
            println!("Successfully extracted {} files.", count);
        }
        Commands::MergeDic { output, inputs } => {
            let mut dictionary = HashDictionary::new();
            let mut input_entries = 0usize;
            let mut new_hashes = 0usize;
            let mut new_file_names = 0usize;
            let mut duplicate_hashes = 0usize;
            for input in inputs {
                let incoming = HashDictionary::load(input)?;
                println!("Loaded {} entries from {}", incoming.len(), input.display());
                input_entries += incoming.len();
                let report = dictionary.merge(incoming);
                new_hashes += report.new_hashes;
                new_file_names += report.new_file_names;
                duplicate_hashes += report.duplicate_hashes;
            }

            dictionary.save(output)?;
            println!(
                "Merged {} input entries into {} unique entries at {} ({} new hashes, {} new names, {} duplicate hashes)",
                input_entries,
                dictionary.len(),
                output.display(),
                new_hashes,
                new_file_names,
                duplicate_hashes
            );
        }
        Commands::PopulateDict {
            config,
            uop_dir,
            output,
            input,
        } => {
            let config_content = std::fs::read_to_string(config)?;
            let cfg: PopulatorConfig = toml::from_str(&config_content)?;

            let mut hash_dictionary = if let Some(input_path) = input {
                HashDictionary::load(input_path)?
            } else {
                HashDictionary::new()
            };

            let stop_signal = Arc::new(AtomicBool::new(false));

            for (uop_name, pkg_config) in cfg.packages {
                let uop_path = uop_dir.join(&uop_name);
                if !uop_path.exists() {
                    log::warn!("UOP file not found: {}", uop_path.display());
                    continue;
                }

                println!("Processing {}...", uop_name);
                let package = UopPackage::load(&uop_path)?;
                let missing_hashes: HashSet<u64> = package
                    .iter_files()
                    .map(|f| f.filename_hash())
                    .filter(|h| !hash_dictionary.contains(*h))
                    .collect();

                if missing_hashes.is_empty() {
                    println!("  No missing hashes in {}.", uop_name);
                    continue;
                }

                println!("  Found {} missing hashes.", missing_hashes.len());

                for template_str in pkg_config.candidates {
                    println!("  Trying template: {}", template_str);
                    let range = pkg_config.range.map(|r| r[0]..=r[1]);
                    let template = UopTemplate::new(&template_str, range.clone());

                    if template.has_placeholders() {
                        let count = range
                            .as_ref()
                            .map(|r| r.clone().count())
                            .unwrap_or_else(|| (template.infer_max_range() + 1) as usize);
                        let pb = ProgressBar::new(count as u64);
                        pb.set_style(
                            ProgressStyle::default_bar()
                                .template(
                                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
                                )?
                                .progress_chars("#>-"),
                        );
                        let found = template.crack(&missing_hashes, &stop_signal);
                        for (h, s) in found {
                            println!("    Found match: 0x{:016X} -> {}", h, s);
                            hash_dictionary.set(h, s);
                        }
                        pb.finish_and_clear();
                    } else {
                        let found = template.crack(&missing_hashes, &stop_signal);
                        for (h, s) in found {
                            println!("    Found match: 0x{:016X} -> {}", h, s);
                            hash_dictionary.set(h, s);
                        }
                    }
                }
            }

            hash_dictionary.save(output)?;
            println!("Saved dictionary to {}", output.display());
        }
    }

    Ok(())
}

fn safe_extract_path(out_dir: &Path, file_name: &str) -> eyre::Result<PathBuf> {
    let normalized = file_name.replace('\\', "/");
    let relative = Path::new(&normalized);
    let mut out_path = PathBuf::from(out_dir);
    let mut has_component = false;

    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                out_path.push(part);
                has_component = true;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(eyre::eyre!("path escapes output directory"));
            }
        }
    }

    if !has_component {
        return Err(eyre::eyre!("empty output path"));
    }

    Ok(out_path)
}

fn save_package_atomically(package: &mut UopPackage, target_path: &Path) -> eyre::Result<()> {
    let temp_path = unique_temp_path_for(target_path)?;
    package
        .finalize_and_save(&temp_path)
        .with_context(|| format!("Failed to save temporary UOP file: {}", temp_path.display()))?;

    if let Err(error) = fs::rename(&temp_path, target_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error).with_context(|| "Failed to replace old UOP file with the new one");
    }

    Ok(())
}

fn unique_temp_path_for(target_path: &Path) -> eyre::Result<PathBuf> {
    let parent = target_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = target_path
        .file_name()
        .ok_or_else(|| eyre::eyre!("target UOP path has no file name: {}", target_path.display()))?
        .to_string_lossy();

    for attempt in 0..100u32 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let candidate = parent.join(format!(
            ".{file_name}.tmp.{}.{}.{}",
            std::process::id(),
            nonce,
            attempt
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(eyre::eyre!(
        "failed to allocate a temporary UOP path beside {}",
        target_path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_extract_path_preserves_relative_subdirs() {
        let path = safe_extract_path(
            Path::new("/tmp/out"),
            "build\\worldart/00000042.dds",
        )
        .unwrap();

        assert_eq!(
            path,
            PathBuf::from("/tmp/out")
                .join("build")
                .join("worldart")
                .join("00000042.dds")
        );
    }

    #[test]
    fn safe_extract_path_rejects_parent_dir() {
        assert!(safe_extract_path(Path::new("/tmp/out"), "../outside.dds").is_err());
        assert!(safe_extract_path(Path::new("/tmp/out"), "build/../outside.dds").is_err());
    }

    #[test]
    fn safe_extract_path_rejects_absolute_path() {
        assert!(safe_extract_path(Path::new("/tmp/out"), "/tmp/outside.dds").is_err());
    }

    #[test]
    fn unique_temp_path_is_sibling_and_not_fixed_name() {
        let target = Path::new("/tmp/client.uop");
        let temp = unique_temp_path_for(target).unwrap();

        assert_eq!(temp.parent(), Some(Path::new("/tmp")));
        assert_ne!(temp, target.with_extension("uop.temp"));
        assert!(temp.file_name().unwrap().to_string_lossy().contains("client.uop"));
    }
}
