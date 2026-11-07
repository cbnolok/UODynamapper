mod cmd;

use clap::{Parser, Subcommand};
use color_eyre::eyre;

#[derive(Parser, Debug)]
#[command(author, version, about = "UO client file research/diagnostic tools.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    FacetEvidence(cmd::facet_evidence::Args),
    KrEcTerrainDiff(cmd::kr_ec_terrain_diff::Args),
    KrLandMap(cmd::kr_land_map::Args),
    TextureScanner(cmd::texture_scanner::Args),
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let _ = uot_logging::install_tracing_indicatif_logger();
    let cli = Cli::parse();
    match cli.command {
        Command::FacetEvidence(args) => cmd::facet_evidence::run(args),
        Command::KrEcTerrainDiff(args) => cmd::kr_ec_terrain_diff::run(args),
        Command::KrLandMap(args) => cmd::kr_land_map::run(args),
        Command::TextureScanner(args) => cmd::texture_scanner::run(args),
    }
}
