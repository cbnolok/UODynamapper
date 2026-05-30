use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::Parser;
use serde::Deserialize;
use uocf::uop_container::hash_dictionary::HashDictionary;
use uocf::uop_container::package::UopPackage;
use uocf::uop_container::template::UopTemplate;
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the TOML configuration file
    #[arg(short, long)]
    config: PathBuf,

    /// Path to the UOP files directory
    #[arg(short, long)]
    uop_dir: PathBuf,

    /// Path to the output dictionary file (.dic)
    #[arg(short, long, default_value = "Dictionary.dic")]
    output: PathBuf,

    /// Path to an existing .dic dictionary to load first (optional)
    #[arg(short, long)]
    input: Option<PathBuf>,
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

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    let _ = udd_logging::install_paris_logger();

    let args = Args::parse();

    let config_content = std::fs::read_to_string(&args.config)?;
    let config: PopulatorConfig = toml::from_str(&config_content)?;

    let mut hash_dictionary = if let Some(input_path) = &args.input {
        HashDictionary::load(input_path)?
    } else {
        HashDictionary::new()
    };

    let stop_signal = Arc::new(AtomicBool::new(false));

    for (uop_name, pkg_config) in config.packages {
        let uop_path = args.uop_dir.join(&uop_name);
        if !uop_path.exists() {
            log::warn!("UOP file not found: {}", uop_path.display());
            continue;
        }

        println!("Processing {}...", uop_name);
        let package = UopPackage::load(&uop_path)?;
        let missing_hashes: HashSet<u64> = package.iter_files()
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
                let count = range.as_ref().map(|r| r.clone().count()).unwrap_or_else(|| (template.infer_max_range() + 1) as usize);
                let pb = ProgressBar::new(count as u64);
                pb.set_style(ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")?
                    .progress_chars("#>-"));
                
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

    hash_dictionary.save(&args.output)?;
    println!("Saved dictionary to {}", args.output.display());

    Ok(())
}
