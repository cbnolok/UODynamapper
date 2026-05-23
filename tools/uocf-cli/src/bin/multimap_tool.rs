use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre;
use uocf::classic::multimap_rle;

/// Classic Client multimap.rle converter.
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Decode multimap.rle to a BMP or PNG image.
    RleToImage {
        /// Input multimap.rle file.
        #[arg(long)]
        input: PathBuf,
        /// Output .bmp or .png file.
        #[arg(long)]
        output: PathBuf,
    },
    /// Encode a BMP or PNG image to multimap.rle.
    ImageToRle {
        /// Input .bmp or .png file.
        #[arg(long)]
        input: PathBuf,
        /// Output multimap.rle file.
        #[arg(long)]
        output: PathBuf,
    },
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    match Cli::parse().command {
        Commands::RleToImage { input, output } => {
            let image = multimap_rle::load_rle(&input)?;
            multimap_rle::save_bitmap_or_png(&output, &image)?;
            println!(
                "Decoded {}x{} multimap RLE '{}' to '{}'.",
                image.width,
                image.height,
                input.display(),
                output.display()
            );
        }
        Commands::ImageToRle { input, output } => {
            let image = multimap_rle::load_bitmap_or_png(&input)?;
            multimap_rle::save_rle(&output, &image)?;
            println!(
                "Encoded {}x{} multimap image '{}' to '{}'.",
                image.width,
                image.height,
                input.display(),
                output.display()
            );
        }
    }

    Ok(())
}
