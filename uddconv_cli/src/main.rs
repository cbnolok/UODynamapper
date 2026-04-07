use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre;
use uddconv::{
    cc_art::{
        CcArtAtlasOptions, DEFAULT_ATLAS_GUTTER, DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH, convert_art_mul_to_cc_art_uddp,
    },
    cc_tiledata::convert_tiledata_mul_to_cc_tiledata_uddp,
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Packs classic art.mul/artidx.mul into cc_art.uddp atlas pages.
    PackArt {
        #[arg(name = "path")]
        path: PathBuf,
        #[arg(long, default_value = "cc_art.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
    },
    /// Packs classic tiledata.mul into cc_tiledata.uddp metadata payloads.
    PackTiledata {
        #[arg(name = "path")]
        path: PathBuf,
        #[arg(long, default_value = "cc_tiledata.uddp")]
        output: PathBuf,
    },
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    match Cli::parse().command {
        Commands::PackArt {
            path,
            output,
            atlas_width,
            atlas_height,
            gutter,
        } => {
            let out_file = if output.is_absolute() {
                output
            } else {
                path.join(output)
            };
            let summary = convert_art_mul_to_cc_art_uddp(
                &path,
                &out_file,
                &CcArtAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                },
            )?;
            println!(
                "Wrote {} pages for {} populated slots out of {} total slots to '{}'.",
                summary.page_count,
                summary.populated_slot_count,
                summary.slot_count,
                out_file.display()
            );
        }
        Commands::PackTiledata { path, output } => {
            let out_file = if output.is_absolute() {
                output
            } else {
                path.join(output)
            };
            let summary = convert_tiledata_mul_to_cc_tiledata_uddp(&path, &out_file)?;
            println!(
                "Wrote cc_tiledata.uddp with {} land tiles and {} item tiles to '{}'.",
                summary.land_tile_count,
                summary.item_tile_count,
                out_file.display()
            );
        }
    }

    Ok(())
}
