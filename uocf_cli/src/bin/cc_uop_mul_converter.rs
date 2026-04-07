use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre;
use uocf_cli::legacy_mul::{CompressionFlag, FileType, LegacyMulFileConverter};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Extracts known UOP files into MUL format.
    Extract {
        #[arg(name = "path")]
        path: PathBuf,
    },
    /// Packs known MUL files into UOP format.
    Pack {
        #[arg(name = "path")]
        path: PathBuf,
    },
}

fn print_results(success: u32, total: u32) {
    println!();
    if success < total {
        println!("Errors: {}", total - success);
    } else {
        println!("All actions completed successfully.");
    }
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    let mut success_count = 0;
    let mut total_count = 0;

    match &cli.command {
        Commands::Extract { path } => {
            println!("Mode: Extract from UOP.");
            println!();

            if !path.exists() || !path.is_dir() {
                eprintln!("Directory '{}' does not exist!", path.display());
                return Ok(());
            }

            let uop_dir = path;

            // Extract artLegacyMUL.uop
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("artLegacyMUL.uop"),
                &uop_dir.join("art.mul"),
                Some(&uop_dir.join("artidx.mul")),
                FileType::ArtLegacyMul,
                0,
                None,
            ) {
                eprintln!("Error extracting artLegacyMUL.uop: {}", e);
            } else {
                success_count += 1;
            }

            // Extract gumpartLegacyMUL.uop
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("gumpartLegacyMUL.uop"),
                &uop_dir.join("gumpart.mul"),
                Some(&uop_dir.join("gumpidx.mul")),
                FileType::GumpartLegacyMul,
                0,
                None,
            ) {
                eprintln!("Error extracting gumpartLegacyMUL.uop: {}", e);
            } else {
                success_count += 1;
            }

            // Extract MultiCollection.uop
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("MultiCollection.uop"),
                &uop_dir.join("multi.mul"),
                Some(&uop_dir.join("multiidx.mul")),
                FileType::MultiCollection,
                0,
                Some(&uop_dir.join("housing.bin")),
            ) {
                eprintln!("Error extracting MultiCollection.uop: {}", e);
            } else {
                success_count += 1;
            }

            // Extract soundLegacyMUL.uop
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("soundLegacyMUL.uop"),
                &uop_dir.join("sound.mul"),
                Some(&uop_dir.join("soundidx.mul")),
                FileType::SoundLegacyMul,
                0,
                None,
            ) {
                eprintln!("Error extracting soundLegacyMUL.uop: {}", e);
            } else {
                success_count += 1;
            }

            // Extract maps
            for i in 0..=5 {
                total_count += 1;
                let map_name = format!("map{}", i);
                if let Err(e) = LegacyMulFileConverter::from_uop(
                    &uop_dir.join(format!("{}LegacyMUL.uop", map_name)),
                    &uop_dir.join(format!("{}.mul", map_name)),
                    None,
                    FileType::MapLegacyMul,
                    i,
                    None,
                ) {
                    eprintln!("Error extracting {}LegacyMUL.uop: {}", map_name, e);
                } else {
                    success_count += 1;
                }

                total_count += 1;
                let map_x_name = format!("map{}x", i);
                if let Err(e) = LegacyMulFileConverter::from_uop(
                    &uop_dir.join(format!("{}LegacyMUL.uop", map_x_name)),
                    &uop_dir.join(format!("{}.mul", map_x_name)),
                    None,
                    FileType::MapLegacyMul,
                    i,
                    None,
                ) {
                    eprintln!("Error extracting {}LegacyMUL.uop: {}", map_x_name, e);
                } else {
                    success_count += 1;
                }
            }
        }
        Commands::Pack { path } => {
            println!("Mode: Pack to UOP.");
            println!();

            if !path.exists() || !path.is_dir() {
                eprintln!("Directory '{}' does not exist!", path.display());
                return Ok(());
            }

            let mul_dir = path;

            // Pack art.mul
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("art.mul"),
                Some(&mul_dir.join("artidx.mul")),
                &mul_dir.join("artLegacyMUL.uop"),
                FileType::ArtLegacyMul,
                0,
                CompressionFlag::Zlib,
            ) {
                eprintln!("Error packing art.mul: {}", e);
            } else {
                success_count += 1;
            }

            // Pack gumpart.mul
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("gumpart.mul"),
                Some(&mul_dir.join("gumpidx.mul")),
                &mul_dir.join("gumpartLegacyMUL.uop"),
                FileType::GumpartLegacyMul,
                0,
                CompressionFlag::Zlib,
            ) {
                eprintln!("Error packing gumpart.mul: {}", e);
            } else {
                success_count += 1;
            }

            // Pack multi.mul
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("multi.mul"),
                Some(&mul_dir.join("multiidx.mul")),
                &mul_dir.join("MultiCollection.uop"),
                FileType::MultiCollection,
                0,
                CompressionFlag::None, // housing.bin is not compressed
            ) {
                eprintln!("Error packing multi.mul: {}", e);
            } else {
                success_count += 1;
            }

            // Pack sound.mul
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("sound.mul"),
                Some(&mul_dir.join("soundidx.mul")),
                &mul_dir.join("soundLegacyMUL.uop"),
                FileType::SoundLegacyMul,
                0,
                CompressionFlag::Zlib,
            ) {
                eprintln!("Error packing sound.mul: {}", e);
            } else {
                success_count += 1;
            }

            // Pack maps
            for i in 0..=5 {
                total_count += 1;
                let map_name = format!("map{}", i);
                if let Err(e) = LegacyMulFileConverter::to_uop(
                    &mul_dir.join(format!("{}.mul", map_name)),
                    None,
                    &mul_dir.join(format!("{}LegacyMUL.uop", map_name)),
                    FileType::MapLegacyMul,
                    i,
                    CompressionFlag::None,
                ) {
                    eprintln!("Error packing {}.mul: {}", map_name, e);
                } else {
                    success_count += 1;
                }

                total_count += 1;
                let map_x_name = format!("map{}x", i);
                if let Err(e) = LegacyMulFileConverter::to_uop(
                    &mul_dir.join(format!("{}.mul", map_x_name)),
                    None,
                    &mul_dir.join(format!("{}LegacyMUL.uop", map_x_name)),
                    FileType::MapLegacyMul,
                    i,
                    CompressionFlag::None,
                ) {
                    eprintln!("Error packing {}.mul: {}", map_x_name, e);
                } else {
                    success_count += 1;
                }
            }
        }
    }

    print_results(success_count, total_count);

    Ok(())
}
