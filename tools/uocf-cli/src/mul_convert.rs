use std::path::{Path, PathBuf};

use clap::Subcommand;
use color_eyre::eyre::{self, WrapErr};
use log::{info, error};
use uocf::classic::map::MapSizeCells;
use uocf::classic::map_write::ClassicMapBlocks;
use uocf::classic::target_client::TargetClient;
use uocf::uop_container::package::UopPackage;
use crate::legacy_mul::{CompressionFlag, FileType, LegacyMulFileConverter};

/// The Classic map indices and editions converted.
const MAP_INDICES: std::ops::RangeInclusive<u32> = 0..=5;
const MAP_EDITION_SUFFIXES: [&str; 2] = ["", "x"];

/// UO Legacy MUL/UOP Converter - convert between legacy .mul/.idx files and .uop packages.
#[derive(Subcommand, Debug)]
pub enum MulConvertCmd {
    /// Extracts known UOP files into MUL format. Maps (mapN[x]LegacyMUL.uop) become mapN[x].mul of the size the
    /// folder's client.exe size table gives (else the size the blocks hold), without the UOP's trailing block.
    Extract {
        #[arg(name = "path")]
        path: PathBuf,
        /// Folder the maps are written to (default: the input folder).
        #[arg(long)]
        output_dir: Option<PathBuf>,
    },
    /// Packs known MUL files into UOP format. Maps (mapN[x].mul) become mapN[x]LegacyMUL.uop as the client ships
    /// them (4,096 blocks per entry and the trailing block); with --uop-template-dir, each map keeps the layout of
    /// the package of the same name there, so unchanged entries stay byte for byte.
    Pack {
        #[arg(name = "path")]
        path: PathBuf,
        /// Folder the maps are written to (default: the input folder).
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// Folder of the mapN[x]LegacyMUL.uop whose layouts the packed maps keep.
        #[arg(long)]
        uop_template_dir: Option<PathBuf>,
    },
}

/// The size of map `map_index` in the size table of the client in `dir`, when there is one.
fn client_map_size(dir: &Path, map_index: u32) -> Option<MapSizeCells> {
    TargetClient::open(dir).ok().and_then(|client| client.facet_size(map_index).ok())
}

fn extract_map(uop: &Path, mul: &Path, map_index: u32, size: Option<MapSizeCells>) -> eyre::Result<()> {
    let map = ClassicMapBlocks::read_uop(uop, map_index, size)?;
    map.write_mul(mul)?;
    let size = map.size_cells();
    info!("{} -> {} ({}x{})", uop.display(), mul.display(), size.width, size.height);
    Ok(())
}

fn pack_map(mul: &Path, uop: &Path, map_index: u32, size: Option<MapSizeCells>, template: Option<&Path>) -> eyre::Result<()> {
    let map = ClassicMapBlocks::read_mul(mul, map_index, size)?;
    let template = match template {
        Some(path) if path.is_file() => {
            if path.canonicalize().ok() == uop.canonicalize().ok() {
                eyre::bail!("the template {} is the output; choose another output folder", path.display());
            }
            Some(UopPackage::load(path).wrap_err_with(|| format!("load {}", path.display()))?)
        }
        _ => None,
    };
    let kept = template.is_some();
    let report = map.write_uop(uop, map_index, template)?;
    let size = map.size_cells();
    info!(
        "{} -> {} ({}x{}; {} entries written, {} unchanged, {} removed{})",
        mul.display(),
        uop.display(),
        size.width,
        size.height,
        report.written,
        report.unchanged,
        report.removed,
        if kept { ", template layout kept" } else { "" }
    );
    Ok(())
}

pub fn run(cmd: MulConvertCmd) -> eyre::Result<()> {
    let mut success_count = 0;
    let mut total_count = 0;

    match &cmd {
        MulConvertCmd::Extract { path, output_dir } => {
            info!("Mode: Extract from UOP.");
            info!("");

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
                                None,
            ) {
                error!("Error extracting artLegacyMUL.uop: {}", e);
            } else {
                success_count += 1;
            }

            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("gumpartLegacyMUL.uop"),
                &uop_dir.join("gumpart.mul"),
                Some(&uop_dir.join("gumpidx.mul")),
                FileType::GumpartLegacyMul,
                                None,
            ) {
                error!("Error extracting gumpartLegacyMUL.uop: {}", e);
            } else {
                success_count += 1;
            }

            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("MultiCollection.uop"),
                &uop_dir.join("multi.mul"),
                Some(&uop_dir.join("multiidx.mul")),
                FileType::MultiCollection,
                                Some(&uop_dir.join("housing.bin")),
            ) {
                error!("Error extracting MultiCollection.uop: {}", e);
            } else {
                success_count += 1;
            }

            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("soundLegacyMUL.uop"),
                &uop_dir.join("sound.mul"),
                Some(&uop_dir.join("soundidx.mul")),
                FileType::SoundLegacyMul,
                                None,
            ) {
                error!("Error extracting soundLegacyMUL.uop: {}", e);
            } else {
                success_count += 1;
            }

            let output_dir = output_dir.as_deref().unwrap_or(uop_dir);
            for map_index in MAP_INDICES {
                for x in MAP_EDITION_SUFFIXES {
                    let uop = uop_dir.join(format!("map{map_index}{x}LegacyMUL.uop"));
                    if !uop.is_file() {
                        continue;
                    }
                    total_count += 1;
                    let mul = output_dir.join(format!("map{map_index}{x}.mul"));
                    match extract_map(&uop, &mul, map_index, client_map_size(uop_dir, map_index)) {
                        Ok(()) => success_count += 1,
                        Err(e) => error!("Error extracting {}: {e:#}", uop.display()),
                    }
                }
            }
        }
        MulConvertCmd::Pack { path, output_dir, uop_template_dir } => {
            info!("Mode: Pack to UOP.");
            info!("");

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
                                CompressionFlag::Zlib,
            ) {
                error!("Error packing art.mul: {}", e);
            } else {
                success_count += 1;
            }

            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("gumpart.mul"),
                Some(&mul_dir.join("gumpidx.mul")),
                &mul_dir.join("gumpartLegacyMUL.uop"),
                FileType::GumpartLegacyMul,
                                CompressionFlag::Zlib,
            ) {
                error!("Error packing gumpart.mul: {}", e);
            } else {
                success_count += 1;
            }

            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("multi.mul"),
                Some(&mul_dir.join("multiidx.mul")),
                &mul_dir.join("MultiCollection.uop"),
                FileType::MultiCollection,
                                CompressionFlag::None,
            ) {
                error!("Error packing multi.mul: {}", e);
            } else {
                success_count += 1;
            }

            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("sound.mul"),
                Some(&mul_dir.join("soundidx.mul")),
                &mul_dir.join("soundLegacyMUL.uop"),
                FileType::SoundLegacyMul,
                                CompressionFlag::Zlib,
            ) {
                error!("Error packing sound.mul: {}", e);
            } else {
                success_count += 1;
            }

            let output_dir = output_dir.as_deref().unwrap_or(mul_dir);
            for map_index in MAP_INDICES {
                for x in MAP_EDITION_SUFFIXES {
                    let mul = mul_dir.join(format!("map{map_index}{x}.mul"));
                    if !mul.is_file() {
                        continue;
                    }
                    total_count += 1;
                    let name = format!("map{map_index}{x}LegacyMUL.uop");
                    let template = uop_template_dir.as_ref().map(|dir| dir.join(&name));
                    let size = client_map_size(mul_dir, map_index);
                    match pack_map(&mul, &output_dir.join(&name), map_index, size, template.as_deref()) {
                        Ok(()) => success_count += 1,
                        Err(e) => error!("Error packing {}: {e:#}", mul.display()),
                    }
                }
            }
        }
    }

    info!("");
    if success_count < total_count {
        info!("Errors: {}", total_count - success_count);
        return Err(eyre::eyre!(
            "{} of {} actions failed",
            total_count - success_count,
            total_count
        ));
    } else {
        info!("All actions completed successfully.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map of three UOP entries: two whole ones and a partial last one (the trailing block appended to it).
    const SIZE: MapSizeCells = MapSizeCells { width: 8 * 144, height: 8 * 64 };
    const MAP_INDEX: u32 = 2;
    const EDITED: (u32, u32, u16, i8) = (1000, 300, 0x77, -9);

    #[test]
    fn maps_pack_and_extract_back_and_keep_a_template_layout() {
        let dir = std::env::temp_dir().join(format!("uocf_cli_mul_convert_maps_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut map = ClassicMapBlocks::new(SIZE).unwrap();
        let (x, y, land, z) = EDITED;
        map.set_cell(x, y, land, z).unwrap();
        let (mul, uop, again, repacked) =
            (dir.join("map2.mul"), dir.join("map2LegacyMUL.uop"), dir.join("again.mul"), dir.join("repacked.uop"));
        map.write_mul(&mul).unwrap();
        pack_map(&mul, &uop, MAP_INDEX, Some(SIZE), None).unwrap();
        extract_map(&uop, &again, MAP_INDEX, Some(SIZE)).unwrap();
        assert_eq!(std::fs::read(&again).unwrap(), std::fs::read(&mul).unwrap());
        pack_map(&again, &repacked, MAP_INDEX, Some(SIZE), Some(&uop)).unwrap();
        assert_eq!(std::fs::read(&repacked).unwrap(), std::fs::read(&uop).unwrap());
        assert!(pack_map(&again, &uop, MAP_INDEX, Some(SIZE), Some(&uop)).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
