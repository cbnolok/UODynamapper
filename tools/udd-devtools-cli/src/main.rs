mod gumps;
mod package_compare;
mod paperdoll;

use clap::{Parser, Subcommand};
use color_eyre::eyre;

#[derive(Parser, Debug)]
#[command(version, about = "Converted asset validation and research diagnostics")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Compare decoded package records and optionally time their texture reads.
    PackageCompare(package_compare::Arguments),
    /// Gump pixel equality, quality scores and storage statistics.
    Gumps(gumps::Arguments),
    /// Compare baked paperdolls with archived pre-bake composition baselines.
    PaperdollCompare(paperdoll::Arguments),
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    match Arguments::parse().command {
        Command::PackageCompare(args) => package_compare::run(args),
        Command::Gumps(args) => gumps::run(args),
        Command::PaperdollCompare(args) => paperdoll::run(args),
    }
}
