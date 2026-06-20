use std::path::PathBuf;

use clap::Subcommand;
use color_eyre::eyre;
use crate::legacy_mul::{CompressionFlag, FileType, LegacyMulFileConverter};

/// UO Legacy MUL/UOP Converter - convert between legacy .mul/.idx files and .uop packages.
#[derive(Subcommand, Debug)]
pub enum MulConvertCmd {
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

pub fn run(cmd: MulConvertCmd) -> eyre::Result<()> {
    let mut success_count = 0;
    let mut total_count = 0;

    match &cmd {
        MulConvertCmd::Extract { path } => {
            println!("Mode: Extract from UOP.");
            println!();

            if !path.exists() || !path.is_dir() {
                return Err(eyre::eyre!("Directory '{}' does not exist", path.display()));
            }

            let uop_dir = path;

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

            for i in 0..=5 {
                let map_variants = [
                    format!("map{}", i),
                    format!("map{}x", i),
                ];

                for map_name in map_variants {
                    let uop_names = [
                        format!("{}LegacyMUL.uop", map_name),
                        format!("{}.uop", map_name),
                    ];

                    let mut found_path = None;
                    for uop_name in &uop_names {
                        let p = uop_dir.join(uop_name);
                        if p.exists() {
                            found_path = Some(p);
                            break;
                        }
                    }

                    if let Some(p) = found_path {
                        total_count += 1;
                        if let Err(e) = LegacyMulFileConverter::from_uop(
                            &p,
                            &uop_dir.join(format!("{}.mul", map_name)),
                            None,
                            FileType::MapLegacyMul,
                            i,
                            None,
                        ) {
                            eprintln!("Error extracting {}: {}", p.display(), e);
                        } else {
                            success_count += 1;
                        }
                    }
                }
            }
        }
        MulConvertCmd::Pack { path } => {
            println!("Mode: Pack to UOP.");
            println!();

            if !path.exists() || !path.is_dir() {
                return Err(eyre::eyre!("Directory '{}' does not exist", path.display()));
            }

            let mul_dir = path;

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

            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("multi.mul"),
                Some(&mul_dir.join("multiidx.mul")),
                &mul_dir.join("MultiCollection.uop"),
                FileType::MultiCollection,
                0,
                CompressionFlag::None,
            ) {
                eprintln!("Error packing multi.mul: {}", e);
            } else {
                success_count += 1;
            }

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

    println!();
    if success_count < total_count {
        println!("Errors: {}", total_count - success_count);
        return Err(eyre::eyre!(
            "{} of {} actions failed",
            total_count - success_count,
            total_count
        ));
    } else {
        println!("All actions completed successfully.");
    }

    Ok(())
}
