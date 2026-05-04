use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use color_eyre::eyre;
use uddconv::{
    cc_art::{
        CcArtAtlasOptions, DEFAULT_ATLAS_GUTTER as CC_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as CC_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as CC_DEFAULT_ATLAS_PAGE_WIDTH,
        convert_art_mul_to_cc_art_uddp_from_sources,
    },
    ec_art::{
        DEFAULT_ATLAS_GUTTER as EC_ART_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as EC_ART_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as EC_ART_DEFAULT_ATLAS_PAGE_WIDTH, EcArtAtlasOptions,
        convert_ec_art_uop_to_ec_art_uddp_from_sources,
    },
    ec_land::{
        DEFAULT_ATLAS_GUTTER as EC_LAND_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as EC_LAND_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as EC_LAND_DEFAULT_ATLAS_PAGE_WIDTH, EcLandAtlasOptions,
        convert_ec_land_uop_to_ec_land_uddp_from_sources,
    },
    source_paths::{gather_source_dirs, resolve_output_path},
    tilemeta::{
        build_tilemeta_item_payload_from_sources, build_tilemeta_uddp_from_sources,
        TileMetaBuildOptions, TILEMETA_ITEM_ENTRY_PATH,
    },
    cc_map::convert_map_mul_to_uddp_from_sources,
    cc_statics::convert_statics_mul_to_uddp_from_sources,
};

use crate::package_edit;

/// Pack UODynamapper runtime packages from Classic and Enhanced Client assets.
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

fn find_raw_tilemeta_package(uddp_dir: &Path) -> eyre::Result<PathBuf> {
    ["tilemeta.uddp"]
        .into_iter()
        .map(|file_name| uddp_dir.join(file_name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            eyre::eyre!(
                "missing raw tilemeta package in '{}': expected tilemeta.uddp",
                uddp_dir.display()
            )
        })
}

fn cropped_tilemeta_output_path(raw_tilemeta: &Path) -> eyre::Result<PathBuf> {
    let stem = raw_tilemeta
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| eyre::eyre!("invalid raw tilemeta package name: {}", raw_tilemeta.display()))?;
    let extension = raw_tilemeta
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("uddp");

    Ok(raw_tilemeta.with_file_name(format!("{stem}_ec_art_cropped.{extension}")))
}

fn update_tilemeta_for_cropped_ec_art(
    source_dirs: &[PathBuf],
    uddp_dir: &Path,
) -> eyre::Result<(PathBuf, PathBuf, u32)> {
    let raw_tilemeta = find_raw_tilemeta_package(uddp_dir)?;
    let cropped_tilemeta = cropped_tilemeta_output_path(&raw_tilemeta)?;
    let (item_payload, summary) = build_tilemeta_item_payload_from_sources(
        source_dirs,
        &TileMetaBuildOptions {
            adjust_cropped_ec_art: true,
            use_ec_radarcol: false,
        },
    )?;

    package_edit::replace_virtual_path_file(
        &raw_tilemeta,
        &cropped_tilemeta,
        TILEMETA_ITEM_ENTRY_PATH,
        &item_payload,
    )?;

    Ok((
        raw_tilemeta,
        cropped_tilemeta,
        summary.adjusted_ec_item_count,
    ))
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
        #[arg(long, default_value_t = false)]
        bc7: bool,
    },
    /// Packs cropped Enhanced Client Texture.uop worldart statics into ec_art_cropped.uddp atlas pages.
    PackEcArtCropped {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "ec_art_cropped.uddp")]
        output: PathBuf,
        #[arg(long)]
        uddp_dir: Option<PathBuf>,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
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
        #[arg(long, default_value_t = false)]
        bc7: bool,
    },
    /// Packs CC tiledata and EC tileart into tilemeta.uddp.
    #[command(name = "pack-tilemeta")]
    PackTilemeta {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "tilemeta.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = false)]
        ec_art_cropped: bool,
        #[arg(long, default_value_t = false)]
        use_ec_radarcol: bool,
    },
    /// Generates a radar map (facet0X.dds) from Classic map and statics.
    PackRadar {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long)]
        map_id: u32,
        #[arg(long = "uddpdir")]
        uddp_dir: Option<PathBuf>,
        #[arg(long = "outdir")]
        outdir: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        /// Output format: rgba8, bc7, bc7ktx2
        #[arg(long, default_value = "bc7")]
        format: String,
        /// Zstd compression level for KTX2 (1-22)
        #[arg(long, default_value_t = 3)]
        zstd_level: i32,
    },
    /// Packs Classic mapX.mul into mapX.uddp blocks.
    PackMap {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long)]
        map_id: u32,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Packs Classic staticsX.mul into staticsX.uddp blocks.
    PackStatics {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long)]
        map_id: u32,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

pub fn run() -> eyre::Result<()> {
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
            let paths = collect_source_dirs(&source_dir_args)?;
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
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let summary = convert_ec_art_uop_to_ec_art_uddp_from_sources(
                &paths,
                &out_file,
                &EcArtAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    crop_transparent_bounds: false,
                    use_bc7: bc7,
                },
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
        Commands::PackEcArtCropped {
            source_dirs: source_dir_args,
            output,
            uddp_dir,
            atlas_width,
            atlas_height,
            gutter,
            bc7,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let summary = convert_ec_art_uop_to_ec_art_uddp_from_sources(
                &paths,
                &out_file,
                &EcArtAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    crop_transparent_bounds: true,
                    use_bc7: bc7,
                },
            )?;
            println!(
                "Cropped {} populated EC static slots before packing.",
                summary.cropped_slot_count,
            );
            println!(
                "Wrote {} pages ({}) for {} populated slots out of {} total slots to '{}'.",
                summary.page_count,
                if bc7 { "BC7" } else { "RGBA8888" },
                summary.populated_slot_count,
                summary.slot_count,
                out_file.display()
            );
            if let Some(uddp_dir) = uddp_dir {
                let resolved_uddp_dir = resolve_output_path(&paths, &uddp_dir);
                let (raw_tilemeta, cropped_tilemeta, adjusted_item_count) =
                    update_tilemeta_for_cropped_ec_art(&paths, &resolved_uddp_dir)?;
                println!(
                    "Updated EC sampling offsets for {} tilemeta items in '{}' using raw '{}'.",
                    adjusted_item_count,
                    cropped_tilemeta.display(),
                    raw_tilemeta.display(),
                );
                println!(
                    "The raw tilemeta package was left untouched so repeated cropped-art test conversions do not stack offset edits."
                );
            }
        }
        Commands::PackEcLand {
            source_dirs: source_dir_args,
            output,
            atlas_width,
            atlas_height,
            gutter,
            bc7,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let summary = convert_ec_land_uop_to_ec_land_uddp_from_sources(
                &paths,
                &out_file,
                &EcLandAtlasOptions { atlas_width, atlas_height, gutter, use_bc7: bc7 },
            )?;
            println!(
                "TerrainDefinition raw texture packing: {} entries, {} alias refs, {} unique alias slots, {} source textures, {} selected textures, {} unique packed textures.",
                summary.terrain_entry_count,
                summary.terrain_alias_ref_count,
                summary.unique_alias_slot_count,
                summary.unique_source_texture_count,
                summary.unique_texture_selection_count,
                summary.unique_packed_texture_count,
            );
            if summary.ignored_source_texture_ids.is_empty() {
                println!("Ignored terrain source textures: none");
            } else {
                let ignored = summary
                    .ignored_source_texture_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "Ignored terrain source textures ({}): {}",
                    summary.ignored_source_texture_ids.len(),
                    ignored
                );
            }
            println!(
                "Wrote {} pages ({}) for {} populated slots out of {} total slots to '{}'.",
                summary.page_count,
                if bc7 { "BC7" } else { "RGBA8888" },
                summary.populated_slot_count,
                summary.slot_count,
                out_file.display()
            );
        }
        Commands::PackTilemeta {
            source_dirs: source_dir_args,
            output,
            ec_art_cropped,
            use_ec_radarcol,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            build_tilemeta_uddp_from_sources(
                &paths,
                &out_file,
                &TileMetaBuildOptions {
                    adjust_cropped_ec_art: ec_art_cropped,
                    use_ec_radarcol,
                },
            )?;
            println!("Wrote tilemeta.uddp to '{}'.", out_file.display());
        }
        Commands::PackMap {
            source_dirs: source_dir_args,
            map_id,
            output,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let default_output = PathBuf::from(format!("map{}.uddp", map_id));
            let out_file = resolve_output_path(&paths, output.as_ref().unwrap_or(&default_output));
            let summary = convert_map_mul_to_uddp_from_sources(
                &paths,
                &out_file,
                map_id,
            )?;
            println!(
                "Wrote {} blocks for map {} to '{}' ({}x{} blocks).",
                summary.block_count,
                summary.map_id,
                out_file.display(),
                summary.width_blocks,
                summary.height_blocks,
            );
        }
        Commands::PackStatics {
            source_dirs: source_dir_args,
            map_id,
            output,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let default_output = PathBuf::from(format!("statics{}.uddp", map_id));
            let out_file = resolve_output_path(&paths, output.as_ref().unwrap_or(&default_output));
            let summary = convert_statics_mul_to_uddp_from_sources(
                &paths,
                &out_file,
                map_id,
            )?;
            println!(
                "Wrote {} blocks with {} total statics for map {} to '{}'.",
                summary.block_count,
                summary.total_statics,
                summary.map_id,
                out_file.display(),
            );
        }
        Commands::PackRadar {
            source_dirs: source_dir_args,
            map_id,
            uddp_dir,
            outdir,
            output,
            format,
            zstd_level,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;

            let radar_format = match format.to_lowercase().as_str() {
                "rgba8" => uddconv::cc_radar::RadarFormat::Rgba8,
                "bc7" => uddconv::cc_radar::RadarFormat::Bc7,
                "bc7ktx2" => uddconv::cc_radar::RadarFormat::Bc7Ktx2,
                _ => eyre::bail!("Invalid format: {}. Valid: rgba8, bc7, bc7ktx2", format),
            };

            let default_output = PathBuf::from(format!("facet0{}.{}", map_id, radar_format.extension()));

            let output_path_to_use = output.as_ref().unwrap_or(&default_output);
            let out_file = if let Some(dir) = &outdir {
                dir.join(output_path_to_use)
            } else {
                resolve_output_path(&paths, output_path_to_use)
            };

            let uddp_dir = uddp_dir.unwrap_or_else(|| PathBuf::from("."));
            let tilemeta_path = find_raw_tilemeta_package(&uddp_dir)?;

            if radar_format == uddconv::cc_radar::RadarFormat::Bc7Ktx2 {
                uddconv_ktx2::build_facet_radar_ktx2(
                    &paths,
                    &tilemeta_path,
                    &out_file,
                    map_id,
                    zstd_level,
                )?;
            } else {
                uddconv::cc_radar::build_facet_radar_dds(
                    &paths,
                    &tilemeta_path,
                    &out_file,
                    map_id,
                    &uddconv::cc_radar::RadarBuildOptions {
                        format: radar_format,
                        zstd_level,
                    }
                )?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn cli_parses_pack_tilemeta_alias_with_both_roots() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-tilemeta",
            "--ccdir",
            "/cc",
            "--ecdir",
            "/ec",
            "--output",
            "tiledata.uddp",
        ])
        .expect("parse cli args");

        match cli.command {
            Commands::PackTilemeta {
                source_dirs,
                output,
                ec_art_cropped,
                use_ec_radarcol,
            } => {
                assert_eq!(source_dirs.ccdir, Some(PathBuf::from("/cc")));
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(output, PathBuf::from("tiledata.uddp"));
                assert!(!ec_art_cropped);
                assert!(!use_ec_radarcol);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_ec_art_cropped() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-ec-art-cropped",
            "--ecdir",
            "/ec",
            "--output",
            "ec_art_cropped.uddp",
        ])
        .expect("parse cropped ec art args");

        match cli.command {
            Commands::PackEcArtCropped {
                source_dirs,
                output,
                uddp_dir,
                ..
            } => {
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(output, PathBuf::from("ec_art_cropped.uddp"));
                assert_eq!(uddp_dir, None);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_ec_art_cropped_with_uddp_dir() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-ec-art-cropped",
            "--ecdir",
            "/ec",
            "--uddp-dir",
            "/packages",
        ])
        .expect("parse cropped ec art args with uddp dir");

        match cli.command {
            Commands::PackEcArtCropped {
                source_dirs,
                uddp_dir,
                ..
            } => {
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(uddp_dir, Some(PathBuf::from("/packages")));
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cropped_tilemeta_output_path_appends_cropped_suffix() {
        let output = cropped_tilemeta_output_path(Path::new("/tmp/tilemeta.uddp"))
            .expect("derive cropped tilemeta output path");

        assert_eq!(output, PathBuf::from("/tmp/tilemeta_ec_art_cropped.uddp"));
    }
}
