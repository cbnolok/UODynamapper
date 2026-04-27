mod extract;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use color_eyre::eyre;
use std::collections::BTreeMap;
use std::fs;
use uocf::udd::{Codec, LookupMode, UddpReader, UDDP_MAGIC, UDPI_MAGIC};
use uddconv::{
    cc_art::{
        CcArtAtlasOptions, CcArtPackage, DEFAULT_ATLAS_GUTTER as CC_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as CC_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as CC_DEFAULT_ATLAS_PAGE_WIDTH,
        convert_art_mul_to_cc_art_uddp_from_sources,
    },
    ec_art::{
        EcArtAtlasOptions, EcArtPackage, DEFAULT_ATLAS_GUTTER as EC_ART_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as EC_ART_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as EC_ART_DEFAULT_ATLAS_PAGE_WIDTH,
        convert_ec_art_uop_to_ec_art_uddp_from_sources,
    },
    ec_land::{
        EcLandAtlasOptions, EcLandPackage, DEFAULT_ATLAS_GUTTER as EC_LAND_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as EC_LAND_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as EC_LAND_DEFAULT_ATLAS_PAGE_WIDTH,
        convert_ec_land_uop_to_ec_land_uddp_from_sources,
    },
    source_paths::{gather_source_dirs, resolve_output_path},
    unified_tiledata::{UnifiedTileDataPackage, build_unified_tiledata_uddp_from_sources},
};

/// UODynamapper Asset Converter - A tool to convert classic Ultima Online assets
/// into UODynamapper-specific formats.
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Args, Clone)]
struct SourceDirArgs {
    #[arg(long)]
    ccdir: Option<PathBuf>,
    #[arg(long)]
    ecdir: Option<PathBuf>,
}

fn collect_source_dirs(args: &SourceDirArgs) -> eyre::Result<Vec<PathBuf>> {
    let dirs = gather_source_dirs(args.ccdir.as_ref(), args.ecdir.as_ref());
    if dirs.is_empty() {
        eyre::bail!("at least one source root must be provided via --ccdir or --ecdir");
    }
    Ok(dirs)
}

#[derive(Subcommand)]
enum Commands {
    /// Packs classic art.mul/artidx.mul into cc_art.uddp atlas pages.
    PackArt {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "cc_art.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = CC_DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = CC_DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = CC_DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
        /// Encode atlas pages as BC7 block-compressed (GPU-ready, ~8x smaller VRAM).
        /// Requires atlas dimensions divisible by 4.
        #[arg(long, default_value_t = false)]
        bc7: bool,
    },
    /// Packs Enhanced Client Texture.uop worldart statics into ec_art.uddp atlas pages.
    PackEcArt {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "ec_art.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
        /// Encode atlas pages as BC7 block-compressed (GPU-ready, ~8x smaller VRAM).
        #[arg(long, default_value_t = false)]
        bc7: bool,
    },
    /// Packs Enhanced Client Texture.uop land textures into ec_land.uddp atlas pages.
    PackEcLand {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "ec_land.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = EC_LAND_DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = EC_LAND_DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = EC_LAND_DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
        /// Encode atlas pages as BC7 block-compressed (GPU-ready, ~8x smaller VRAM).
        #[arg(long, default_value_t = false)]
        bc7: bool,
    },
    /// Packs CC tiledata and EC tileart into a unified tiledata.uddp.
    PackUnifiedTiledata {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "unified_tiledata.uddp")]
        output: PathBuf,
    },
    /// Show structural information about a .uddp or .uddpi package.
    Info {
        file: PathBuf,
    },
    /// Extract package contents into a folder for inspection.
    Extract {
        file: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

fn lookup_mode_name(mode: LookupMode) -> &'static str {
    match mode {
        LookupMode::VirtualPathHash => "VirtualPathHash",
        LookupMode::DenseId => "DenseId",
        LookupMode::SparseId => "SparseId",
    }
}

fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::None => "None",
        Codec::ZstdNoDict => "ZstdNoDict",
        Codec::ZstdTypeDict => "ZstdTypeDict",
        Codec::Reserved => "Reserved",
    }
}

fn data_type_name(data_type: u8) -> &'static str {
    match data_type {
        0 => "Unknown",
        1 => "Art",
        2 => "Anim",
        3 => "Map",
        4 => "Gump",
        5 => "GumpArt",
        6 => "Sound",
        7 => "Music",
        8 => "Multi",
        9 => "Texture",
        10 => "Light",
        11 => "Metadata",
        12 => "Sector",
        13 => "Tile",
        14 => "Static",
        _ => "Custom",
    }
}

fn unpack_type_local(meta32: u32) -> u8 {
    (meta32 & 0x3F) as u8
}

fn unpack_codec_local(meta32: u32) -> Codec {
    match ((meta32 >> 6) & 0x03) as u8 {
        0 => Codec::None,
        1 => Codec::ZstdNoDict,
        2 => Codec::ZstdTypeDict,
        _ => Codec::Reserved,
    }
}

fn unpack_delta32_local(meta32: u32, pos64: u64) -> u32 {
    let delta_hi = (meta32 >> 8) & 0xFF;
    let delta_lo = (pos64 >> 40) as u32 & 0x00FF_FFFF;
    (delta_hi << 24) | delta_lo
}

fn reconstruct_stored_size_local(raw_size: u32, meta32: u32, pos64: u64) -> u32 {
    match unpack_codec_local(meta32) {
        Codec::None => raw_size,
        Codec::ZstdNoDict | Codec::ZstdTypeDict | Codec::Reserved => {
            raw_size.saturating_sub(unpack_delta32_local(meta32, pos64))
        }
    }
}

fn page_pixel_format_name(byte_len: usize, width: u32, height: u32) -> &'static str {
    let rgba_len = width as usize * height as usize * 4;
    if byte_len == rgba_len {
        "Rgba8888"
    } else {
        "Encoded"
    }
}

fn print_known_package_summary(bytes: &[u8]) -> eyre::Result<bool> {
    if let Ok(package) = CcArtPackage::from_uddp_package(UddpReader::open(bytes.to_vec())?) {
        let populated_slots = package.slots().iter().filter(|slot| slot.is_present()).count();
        let first_page_format = package
            .pages()
            .first()
            .and_then(|page| package.read_page_bytes(page.page_index).ok().map(|data| (page, data.len())));

        println!("Recognized package: cc_art");
        println!("Known logical files: metadata=2, textures={}", package.pages().len());
        println!(
            "Atlas: {}x{}, gutter={}, pages={}, slots={}, populated={}",
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
            package.pages().len(),
            package.slots().len(),
            populated_slots
        );
        if let Some((page, byte_len)) = first_page_format {
            println!(
                "Page payload format: {} (first stored page bytes={})",
                page_pixel_format_name(byte_len, page.used_width, page.used_height),
                byte_len
            );
        }
        return Ok(true);
    }

    if let Ok(package) = EcArtPackage::from_uddp_package(UddpReader::open(bytes.to_vec())?) {
        let populated_slots = package.slots().iter().filter(|slot| slot.is_present()).count();
        println!("Recognized package: ec_art");
        println!("Known logical files: metadata=2, textures={}", package.pages().len());
        println!(
            "Atlas: {}x{}, gutter={}, pages={}, slots={}, populated={}",
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
            package.pages().len(),
            package.slots().len(),
            populated_slots
        );
        return Ok(true);
    }

    if let Ok(package) = EcLandPackage::from_uddp_package(UddpReader::open(bytes.to_vec())?) {
        let populated_slots = package.slots().iter().filter(|slot| slot.is_present()).count();
        println!("Recognized package: ec_land");
        println!("Known logical files: metadata=2, textures={}", package.pages().len());
        println!(
            "Atlas: {}x{}, gutter={}, pages={}, slots={}, populated={}",
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
            package.pages().len(),
            package.slots().len(),
            populated_slots
        );
        return Ok(true);
    }

    if let Ok(package) = UnifiedTileDataPackage::from_uddp_package(UddpReader::open(bytes.to_vec())?) {
        println!("Recognized package: unified_tiledata");
        println!("Known logical files: metadata=2");
        println!(
            "Unified tiledata: land_tiles={}, item_tiles={}",
            package.land_tiles().len(),
            package.item_tiles().len()
        );
        return Ok(true);
    }

    Ok(false)
}

fn print_package_info(path: &PathBuf) -> eyre::Result<()> {
    let bytes = fs::read(path)?;
    let package = UddpReader::open(bytes.clone())?;
    let header = package.header();
    let kind = match header.magic {
        UDDP_MAGIC => "UDDP",
        UDPI_MAGIC => "UDDPI",
        _ => "Unknown",
    };

    let mut codec_counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut type_counts: BTreeMap<u8, (u32, u64, u64)> = BTreeMap::new();
    let mut raw_total = 0u64;
    let mut stored_total = 0u64;

    for record in package.records() {
        let codec = unpack_codec_local(record.locator.meta32);
        let data_type = unpack_type_local(record.locator.meta32);
        let stored_size = reconstruct_stored_size_local(
            record.locator.raw_size,
            record.locator.meta32,
            record.locator.pos64,
        ) as u64;
        let raw_size = record.locator.raw_size as u64;

        *codec_counts.entry(codec_name(codec)).or_default() += 1;
        let entry = type_counts.entry(data_type).or_insert((0, 0, 0));
        entry.0 += 1;
        entry.1 += raw_size;
        entry.2 += stored_size;
        raw_total += raw_size;
        stored_total += stored_size;
    }

    println!("File: {}", path.display());
    println!("Kind: {}", kind);
    println!("Lookup mode: {}", lookup_mode_name(package.lookup_mode()));
    println!("Package bytes: {}", package.package_size_bytes());
    println!("Header file count: {}", header.file_count);
    println!("Stored package hash64: {:#018x}", package.stored_package_hash64());
    println!("Computed package hash64: {:#018x}", package.computed_package_hash64());
    match package.lookup_mode() {
        LookupMode::VirtualPathHash => {
            println!("Logical key counts: path_hashes={}", header.file_count);
        }
        LookupMode::DenseId | LookupMode::SparseId => {
            println!("Logical key counts: ids={}", header.file_count);
        }
    }

    let dictionaries = package.dictionary_records();
    println!("Dictionaries: {}", dictionaries.len());
    for (data_type, codec, size) in dictionaries {
        println!(
            "  {} ({}) : codec={}, bytes={}",
            data_type_name(data_type),
            data_type,
            codec_name(codec),
            size
        );
    }

    let recognized = print_known_package_summary(&bytes)?;
    if !recognized {
        println!(
            "Payload totals: raw={} stored={} saved={}",
            raw_total,
            stored_total,
            raw_total.saturating_sub(stored_total)
        );

        println!("Compression:");
        for (codec, count) in codec_counts {
            println!("  {}: {} files", codec, count);
        }

        println!("Data types:");
        for (data_type, (count, raw_size, stored_size)) in type_counts {
            println!(
                "  {} ({}) : {} files, raw={}, stored={}",
                data_type_name(data_type),
                data_type,
                count,
                raw_size,
                stored_size
            );
        }
    }

    Ok(())
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    match Cli::parse().command {
        Commands::PackArt {
            source_dirs: source_dir_args,
            output,
            atlas_width,
            atlas_height,
            gutter,
            bc7,
        } => {
            let paths = self::collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let summary = convert_art_mul_to_cc_art_uddp_from_sources(
                &paths,
                &out_file,
                &CcArtAtlasOptions { atlas_width, atlas_height, gutter, use_bc7: bc7 },
            )?;
            println!(
                "Wrote {} pages ({}) for {} populated slots out of {} total slots to '{}'.",
                summary.page_count,
                if bc7 { "BC7" } else { "RGBA8888" },
                summary.populated_slot_count,
                summary.slot_count,
                out_file.display()
            );
        }
        Commands::PackEcArt {
            source_dirs: source_dir_args,
            output,
            atlas_width,
            atlas_height,
            gutter,
            bc7,
        } => {
            let paths = self::collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let summary = convert_ec_art_uop_to_ec_art_uddp_from_sources(
                &paths,
                &out_file,
                &EcArtAtlasOptions { atlas_width, atlas_height, gutter, use_bc7: bc7 },
            )?;
            println!(
                "Wrote {} pages ({}) for {} populated slots out of {} total slots to '{}'.",
                summary.page_count,
                if bc7 { "BC7" } else { "RGBA8888" },
                summary.populated_slot_count,
                summary.slot_count,
                out_file.display()
            );
        }
        Commands::PackEcLand {
            source_dirs: source_dir_args,
            output,
            atlas_width,
            atlas_height,
            gutter,
            bc7,
        } => {
            let paths = self::collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let summary = convert_ec_land_uop_to_ec_land_uddp_from_sources(
                &paths,
                &out_file,
                &EcLandAtlasOptions { atlas_width, atlas_height, gutter, use_bc7: bc7 },
            )?;
            println!(
                "Wrote {} pages ({}) for {} populated slots out of {} total slots to '{}'.",
                summary.page_count,
                if bc7 { "BC7" } else { "RGBA8888" },
                summary.populated_slot_count,
                summary.slot_count,
                out_file.display()
            );
        }
        Commands::PackUnifiedTiledata {
            source_dirs: source_dir_args,
            output,
        } => {
            let paths = self::collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            build_unified_tiledata_uddp_from_sources(&paths, &out_file)?;
            println!("Wrote unified_tiledata.uddp to '{}'.", out_file.display());
        }
        Commands::Info { file } => {
            print_package_info(&file)?;
        }
        Commands::Extract { file, output } => {
            extract::extract_package(&file, output.as_deref())?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::udd::{AddFileRequest, CompressionFlag, DataType, UddpBuilder};

    #[test]
    fn collect_source_dirs_rejects_missing_roots() {
        let error = collect_source_dirs(&SourceDirArgs {
            ccdir: None,
            ecdir: None,
        })
        .expect_err("missing source dirs must fail");

        assert!(error
            .to_string()
            .contains("at least one source root must be provided"));
    }

    #[test]
    fn collect_source_dirs_deduplicates_same_root() {
        let shared = PathBuf::from("/tmp/uo-client");
        let dirs = collect_source_dirs(&SourceDirArgs {
            ccdir: Some(shared.clone()),
            ecdir: Some(shared),
        })
        .expect("collect source dirs");

        assert_eq!(dirs, vec![PathBuf::from("/tmp/uo-client")]);
    }

    #[test]
    fn cli_parses_pack_unified_tiledata_with_both_roots() {
        let cli = Cli::try_parse_from([
            "uddconv",
            "pack-unified-tiledata",
            "--ccdir",
            "/cc",
            "--ecdir",
            "/ec",
            "--output",
            "tiledata.uddp",
        ])
        .expect("parse cli args");

        match cli.command {
            Commands::PackUnifiedTiledata { source_dirs, output } => {
                assert_eq!(source_dirs.ccdir, Some(PathBuf::from("/cc")));
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(output, PathBuf::from("tiledata.uddp"));
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn print_known_package_summary_recognizes_unified_tiledata_packages() {
        let land_tiles = vec![0u8; std::mem::size_of::<uddconv::unified_tiledata::UnifiedLandTile>()];
        let item_tiles = vec![0u8; std::mem::size_of::<uddconv::unified_tiledata::UnifiedItemTile>()];
        let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
        package
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::ZstdNoDict,
                virtual_path: Some(uddconv::unified_tiledata::UNIFIED_LAND_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: &land_tiles,
            })
            .expect("add land metadata");
        package
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::ZstdNoDict,
                virtual_path: Some(uddconv::unified_tiledata::UNIFIED_ITEM_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: &item_tiles,
            })
            .expect("add item metadata");

        let bytes = package.build().expect("build package");

        assert!(print_known_package_summary(&bytes).expect("inspect package"));
    }
}
