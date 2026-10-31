use std::path::PathBuf;

use clap::Subcommand;
use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::image_census::census_directories;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::hash_dictionary::HashDictionary;

#[derive(Subcommand, Debug)]
pub enum ImageCmd {
    /// Classify every entry of every UOP under the directories by its decompressed content alone (a magic, or a
    /// strict TGA validation), set the names' extensions against it, and describe the DDS payloads: formats, mip
    /// counts, header conventions and quirks per package.
    Census {
        /// Directories searched recursively for *.uop.
        #[arg(required = true)]
        dirs: Vec<PathBuf>,
        /// Hash dictionary (.dic) naming the entries; repeat to merge several.
        #[arg(long)]
        dictionary: Vec<PathBuf>,
        /// A client's string_dictionary.uop, whose strings name the entries they hash to; repeat to merge several.
        #[arg(long)]
        string_dictionary: Vec<PathBuf>,
        /// Write the Markdown report here instead of printing it.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Also write every DDS header, one tab-separated line each, to this file.
        #[arg(long)]
        records: Option<PathBuf>,
        /// Also write every entry that is not plain binary (with its name, size, content and TGA verdict), one
        /// tab-separated line each, to this file.
        #[arg(long)]
        contents: Option<PathBuf>,
    },
}

pub fn run(cmd: ImageCmd) -> eyre::Result<()> {
    match cmd {
        ImageCmd::Census { dirs, dictionary, string_dictionary, output, records, contents } => {
            let mut names = HashDictionary::new();
            for path in &dictionary {
                names.merge(HashDictionary::load(path)?);
            }
            for path in &string_dictionary {
                let strings = UoStringDictionary::load(path)
                    .wrap_err_with(|| format!("failed to load {}", path.display()))?;
                for (_, string) in strings.iter() {
                    let hash = hash_file_name_single(string);
                    if names.resolve(hash).is_none() {
                        names.set(hash, string);
                    }
                }
            }
            let census = census_directories(&dirs, &|hash| names.resolve(hash).map(str::to_owned))
                .wrap_err("failed to scan the client packages")?;
            if let Some(path) = &records {
                std::fs::write(path, census.records_tsv())
                    .wrap_err_with(|| format!("failed to write {}", path.display()))?;
            }
            if let Some(path) = &contents {
                std::fs::write(path, census.contents_tsv())
                    .wrap_err_with(|| format!("failed to write {}", path.display()))?;
            }
            let report = census.to_markdown();
            match output {
                Some(path) => {
                    std::fs::write(&path, report).wrap_err_with(|| format!("failed to write {}", path.display()))?;
                    println!(
                        "Wrote {}: {} packages, {} DDS payloads, {} named entries disagreeing with their extension.",
                        path.display(),
                        census.packages.len(),
                        census.dds_count(),
                        census.mismatch_count()
                    );
                }
                None => print!("{report}"),
            }
        }
    }
    Ok(())
}
