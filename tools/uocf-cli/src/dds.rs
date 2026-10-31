use std::path::PathBuf;

use clap::Subcommand;
use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::dds_census::census_directories;
use uocf::uop_container::hash_dictionary::HashDictionary;

#[derive(Subcommand, Debug)]
pub enum DdsCmd {
    /// List the DDS payloads of every UOP under the directories (found by their magic after decompression, in every
    /// entry): formats, mip counts, header conventions and quirks per package, plus the other images.
    Census {
        /// Directories searched recursively for *.uop.
        #[arg(required = true)]
        dirs: Vec<PathBuf>,
        /// Hash dictionary (.dic) naming the entries; repeat to merge several.
        #[arg(long)]
        dictionary: Vec<PathBuf>,
        /// Write the Markdown report here instead of printing it.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Also write every DDS header, one tab-separated line each, to this file.
        #[arg(long)]
        records: Option<PathBuf>,
    },
}

pub fn run(cmd: DdsCmd) -> eyre::Result<()> {
    match cmd {
        DdsCmd::Census { dirs, dictionary, output, records } => {
            let mut names = HashDictionary::new();
            for path in &dictionary {
                names.merge(HashDictionary::load(path)?);
            }
            let census = census_directories(&dirs, &|hash| names.resolve(hash).map(str::to_owned))
                .wrap_err("failed to scan the client packages")?;
            if let Some(path) = &records {
                std::fs::write(path, census.records_tsv())
                    .wrap_err_with(|| format!("failed to write {}", path.display()))?;
            }
            let report = census.to_markdown();
            match output {
                Some(path) => {
                    std::fs::write(&path, report).wrap_err_with(|| format!("failed to write {}", path.display()))?;
                    println!(
                        "Wrote {}: {} DDS payloads in {} packages.",
                        path.display(),
                        census.dds_count(),
                        census.packages.len()
                    );
                }
                None => print!("{report}"),
            }
        }
    }
    Ok(())
}
