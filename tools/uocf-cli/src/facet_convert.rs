//! `facet convert` (any of Classic MUL, Classic UOP, Enhanced Client facet, Kingdom Reborn facet to any other, with
//! a resize to another client's facet size) and `facet patch-classic` (edit one Classic map edition). Both keep
//! the world map image (`facetNN.mul`) of a client that draws it in step with the map written: the 7.0.x clients
//! read it without bounds checks, tile (x, y) at column x of row y, so an image of another size draws the wrong
//! tiles.

use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use uocf::classic::facet_sources::{
    ClassicDifOptions, ClassicFacetSources, ClassicMapFile, ClassicMapFormat, MapDifFiles, StaticDifFiles,
    StaticsFiles,
};
use uocf::classic::facet_image::{facet_image_file_name, FacetImageFile, FACET_IMAGE_COUNT};
use uocf::classic::facet_image_render::{
    plan_facet_image, ChangedTile, FacetImageMode, FacetImagePlan, FacetImageRenderer, ImageUpdate,
    SHIPPED_STYLE_MATCH_PERCENT,
};
use uocf::classic::map::MapSizeCells;
#[cfg(test)]
use uocf::classic::map_write::ClassicMapBlocks;
#[cfg(test)]
use uocf::classic::statics_write::ClassicStaticsBlocks;
use uocf::classic::target_client::{tiledata_counts, ClassicContainer, TargetClient};
use uocf::classic::world::{ClassicWorld, LandFill, VOID_LAND_ID, VOID_LAND_Z};
use uocf::client_exe::facet_dimensions::ClientFamily;
use uocf::enhanced::facet_encoder::build_facet_from_world;
use uocf::enhanced::facet_package::EcFacetPackage;
use uocf::enhanced::facet_radar::{edition_radar_dds_file_name, RadarPalette};
use uocf::enhanced::facet_sector::facet_package_file_name;
use uocf::enhanced::facet_to_classic::facet_to_classic;
use uocf::kr::facet::facet_encoder::build_kr_facet_from_world;
use uocf::kr::facet::kr_land_map::KrLandMap;
use uocf::map_edition::MapEdition;
use uocf::uop_container::package::UopPackage;
use uocf::utils::path::find_file_ignoring_case;

use crate::facet::{load_package, open_facet, parse_list, parse_number, to_z, CliMapEdition};

/// The map containers a Classic source folder is searched for, in order.
const SOURCE_MAP_FORMATS: [ClassicMapFormat; 2] = [ClassicMapFormat::Uop, ClassicMapFormat::Mul];
const RADARCOL_NAME: &str = "radarcol.mul";
const UOP_EXTENSION: &str = "uop";
/// The separator of `--size WxH`.
const SIZE_SEPARATOR: char = 'x';
/// Values of `--add-static` and `--set-static` in `patch-classic`.
const STATIC_FIELDS: usize = 3;
const SET_STATIC_FIELDS: usize = 4;
/// The tool that changes a client's facet size table.
const PATCHER_TOOL: &str = "uo-client-patcher-gui";

/// `--to`: what `facet convert` writes.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertTarget {
    /// Classic `mapN.mul` and/or `mapNLegacyMUL.uop`, with `staidxN.mul` and `staticsN.mul`.
    Classic,
    /// Enhanced Client `facetN.uop` (and `facetNN.dds` with a radarcol.mul).
    Enhanced,
    /// Kingdom Reborn `facetN.uop` (and `facetNN.dds` with a radarcol.mul).
    Kr,
}

/// `--container`: the Classic map container(s) written.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContainerChoice {
    /// The one the target client ships (--target-client), else the source's (Classic source).
    #[default]
    Auto,
    Mul,
    Uop,
    Both,
}

/// `--difs`: what to do with the source's mapdif/stadif files (Classic source).
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DifMode {
    /// Bake the kinds the source client applies (told by its client.exe); not baked when that is unknown.
    #[default]
    Auto,
    /// Bake every dif file found: each id in file order, a later one winning, as the client applies them.
    Bake,
    /// Do not apply them.
    Ignore,
}

/// `--facet-image`: the world map image (`facetNN.mul`) written with a Classic map, for a target client that draws
/// it (7.0.x).
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FacetImageArg {
    /// The target's image as it is; refused when its size is not the map's.
    Keep,
    /// The target's image, with the tiles that differ from the target's map re-rendered by the slope rule; the
    /// whole image rendered when the sizes differ or the target has none (the default).
    Update,
    /// The whole image rendered from the map by the slope rule.
    Regenerate,
    /// No image; refused when the target's image is not the map's size.
    None,
}

impl From<FacetImageArg> for FacetImageMode {
    fn from(arg: FacetImageArg) -> Self {
        match arg {
            FacetImageArg::Keep => Self::Keep,
            FacetImageArg::Update => Self::Update,
            FacetImageArg::Regenerate => Self::Regenerate,
            FacetImageArg::None => Self::None,
        }
    }
}

#[derive(Args, Debug)]
pub struct ConvertArgs {
    /// Classic client folder to read the source facet from (mapN.mul or mapNLegacyMUL.uop, staidxN.mul,
    /// staticsN.mul, the dif files; client.exe for the size and the dif usage).
    #[arg(long, conflicts_with_all = ["map", "uop"])]
    pub client_dir: Option<PathBuf>,
    /// Classic source map (mapN.mul or mapNLegacyMUL.uop), with --staidx and --statics.
    #[arg(long, requires_all = ["staidx", "statics"], conflicts_with = "uop")]
    pub map: Option<PathBuf>,
    /// Classic `staidxN.mul` read with --map.
    #[arg(long)]
    pub staidx: Option<PathBuf>,
    /// Classic `staticsN.mul` read with --map.
    #[arg(long)]
    pub statics: Option<PathBuf>,
    /// Source facet package (facetN.uop or facetNx.uop; Enhanced Client or Kingdom Reborn, detected).
    #[arg(long)]
    pub uop: Option<PathBuf>,
    /// Source facet index (the base index for an x edition).
    #[arg(long)]
    pub facet: u8,
    /// Classic source edition read from --client-dir.
    #[arg(long = "map-edition", value_enum, default_value_t = CliMapEdition::Standard)]
    pub map_edition: CliMapEdition,
    /// Classic source difs.
    #[arg(long, value_enum, default_value_t = DifMode::Auto)]
    pub difs: DifMode,
    /// Classic source size WxH, for a map no client.exe size table describes (default: the source client's size
    /// table, else the size the map file holds for the facet's height).
    #[arg(long, conflicts_with = "uop")]
    pub source_size: Option<String>,
    /// What to write.
    #[arg(long, value_enum)]
    pub to: ConvertTarget,
    /// Facet index written (default: --facet).
    #[arg(long)]
    pub target_facet: Option<u8>,
    /// Edition written (default: the source's; Kingdom Reborn has only the standard one).
    #[arg(long, value_enum)]
    pub target_edition: Option<CliMapEdition>,
    /// Client folder or executable (client.exe, UOSA.exe, UOKR.exe) written for: its size table gives the target
    /// size, its folder the Classic container and tiledata.mul.
    #[arg(long)]
    pub target_client: Option<PathBuf>,
    /// Target size WxH in cells (multiples of 8); checked against --target-client when both are given.
    #[arg(long)]
    pub size: Option<String>,
    /// Allow a smaller target size to drop the content outside it (refused by default; the drop is reported).
    #[arg(long)]
    pub crop: bool,
    /// Land id of the cells a larger target size adds (default 0x244, the shipped clients' void).
    #[arg(long)]
    pub fill_land: Option<String>,
    /// Land z of the added cells (default 0).
    #[arg(long, allow_hyphen_values = true)]
    pub fill_z: Option<i8>,
    /// Classic map container(s) written.
    #[arg(long, value_enum, default_value_t = ContainerChoice::Auto)]
    pub container: ContainerChoice,
    /// A mapNLegacyMUL.uop whose layout the written UOP keeps (unchanged entries stay byte for byte).
    #[arg(long)]
    pub uop_template: Option<PathBuf>,
    /// The target's tiledata.mul, for the tile id check (default: the one in --target-client's folder).
    #[arg(long)]
    pub target_tiledata: Option<PathBuf>,
    /// Convert a facet package even when the Classic files cannot hold all of it (the losses are listed; refused by
    /// default). Stale delimiters are not a loss: they are reported and rebuilt by the rule.
    #[arg(long)]
    pub allow_lossy: bool,
    /// radarcol.mul, to write the radar image of an Enhanced Client or Kingdom Reborn facet.
    #[arg(long)]
    pub radarcol: Option<PathBuf>,
    /// Kingdom Reborn: the Classic land id whose material and subtype to store for Classic ids the land map lacks.
    #[arg(long)]
    pub kr_fallback_land: Option<String>,
    /// Classic, with a --target-client that draws facetNN.mul (7.0.x): the world map image written (default
    /// update). A rendered tile is an approximation of the shipped style.
    #[arg(long, value_enum)]
    pub facet_image: Option<FacetImageArg>,
    /// Output folder (created; must not be a source folder).
    #[arg(short, long)]
    pub output_dir: PathBuf,
}

#[derive(Args, Debug)]
pub struct PatchClassicArgs {
    /// Classic client folder holding the facet's files.
    #[arg(long, conflicts_with = "map", required_unless_present = "map")]
    pub client_dir: Option<PathBuf>,
    /// mapN.mul or mapNLegacyMUL.uop (or its x edition), with --staidx and --statics.
    #[arg(long, requires_all = ["staidx", "statics"])]
    pub map: Option<PathBuf>,
    /// Classic `staidxN.mul` read with --map.
    #[arg(long)]
    pub staidx: Option<PathBuf>,
    /// Classic `staticsN.mul` read with --map.
    #[arg(long)]
    pub statics: Option<PathBuf>,
    #[arg(long)]
    pub facet: u8,
    /// Map size WxH, for a map no client.exe size table describes (default: the folder's client.exe size table,
    /// else the size the map file holds for the facet's height).
    #[arg(long)]
    pub size: Option<String>,
    /// The edition edited (the other is left as it is).
    #[arg(long = "map-edition", value_enum, default_value_t = CliMapEdition::Standard, conflicts_with = "map")]
    pub map_edition: CliMapEdition,
    #[arg(long)]
    pub x: u32,
    #[arg(long)]
    pub y: u32,
    /// New land id.
    #[arg(long)]
    pub land: Option<String>,
    /// New land z.
    #[arg(long, allow_hyphen_values = true)]
    pub z: Option<i8>,
    /// Remove static INDEX of the cell (before the other static edits).
    #[arg(long, value_name = "INDEX")]
    pub remove_static: Vec<usize>,
    /// Change a static of the cell: INDEX,GRAPHIC,Z,HUE.
    #[arg(long, value_name = "INDEX,GRAPHIC,Z,HUE", allow_hyphen_values = true)]
    pub set_static: Vec<String>,
    /// Add a static: GRAPHIC,Z,HUE; it goes after the cell's statics.
    #[arg(long, value_name = "GRAPHIC,Z,HUE", allow_hyphen_values = true)]
    pub add_static: Vec<String>,
    /// Output folder (created; must not be the source folder). The map is written in the source's container (a
    /// UOP keeps the source's layout), with staidxN.mul and staticsN.mul.
    #[arg(short, long)]
    pub output_dir: PathBuf,
}

/// Where a facet came from: the files and what they tell about the target defaults.
struct Source {
    world: ClassicWorld,
    edition: MapEdition,
    /// The Classic container, for a Classic source.
    container: Option<ClassicContainer>,
    /// Folders and files the outputs must not overwrite.
    inputs: Vec<PathBuf>,
    /// Folders a radarcol.mul may be found in.
    dirs: Vec<PathBuf>,
}

pub fn run_convert(args: ConvertArgs) -> eyre::Result<()> {
    let kr_land = kr_land_map(args.kr_fallback_land.as_deref())?;
    let source = load_source(&args, &kr_land)?;
    let target_facet = args.target_facet.unwrap_or(args.facet);
    let target_edition: MapEdition = args.target_edition.map_or(source.edition, Into::into);
    if args.to == ConvertTarget::Kr && target_edition.is_variant() {
        eyre::bail!("Kingdom Reborn has no variant facet packages: pass --target-edition standard");
    }

    // The target client and size.
    let client = args.target_client.as_deref().map(TargetClient::open).transpose()?;
    if let Some(client) = &client {
        let expected = match args.to {
            ConvertTarget::Classic => ClientFamily::Classic,
            ConvertTarget::Enhanced => ClientFamily::Enhanced,
            ConvertTarget::Kr => ClientFamily::KingdomReborn,
        };
        if client.family() != expected {
            eyre::bail!(
                "{} is a {} client, but --to writes {} files",
                client.exe.display(),
                client.family().label(),
                expected.label()
            );
        }
    }
    let client_size = client.as_ref().map(|client| client.facet_size(u32::from(target_facet))).transpose()?;
    let explicit_size = args.size.as_deref().map(parse_size).transpose()?;
    let size = match (explicit_size, client_size) {
        (Some(size), Some(table)) if size != table => eyre::bail!(
            "--size {}x{} differs from the {}x{} the size table of {} gives facet {target_facet}; to give that client \
             a bigger map, patch its size table with {PATCHER_TOOL}",
            size.width,
            size.height,
            table.width,
            table.height,
            client.as_ref().expect("a table size has a client").exe.display()
        ),
        (Some(size), _) | (None, Some(size)) => size,
        (None, None) => source.world.size_cells(),
    };

    let mut world = source.world;
    let from = world.size_cells();
    if size != from {
        let fill = LandFill {
            land: args.fill_land.as_deref().map(parse_number::<u16>).transpose()?.unwrap_or(VOID_LAND_ID),
            z: args.fill_z.unwrap_or(VOID_LAND_Z),
        };
        let (resized, report) = world.resized(size, fill, args.crop).wrap_err_with(|| {
            format!(
                "the target is smaller; pass --crop to drop that content, or give the target client a bigger map \
                 with {PATCHER_TOOL}"
            )
        })?;
        println!("Resized {}x{} to {}x{}.", from.width, from.height, size.width, size.height);
        for line in report.describe() {
            println!("  {line}");
        }
        world = resized;
    }

    // Era checks: Trammel read from Felucca's files, applied difs, tile ids past the target's tiledata.
    if let Some(client) = &client {
        if target_facet == 1 && client.trammel_reads_map0() {
            println!(
                "warning: {} builds Trammel from map0 and the facet-1 difs; it does not read map1 files.",
                client.exe.display()
            );
        }
        if let Some(difs) = client.difs() {
            let applied = [("map", difs.land.applies()), ("static", difs.statics.applies())];
            for (kind, applies) in applied {
                if applies == Some(true) {
                    println!(
                        "note: {} applies its {kind} difs for facet {target_facet} on top of the files written when \
                         the server sends their counts.",
                        client.exe.display()
                    );
                }
            }
        }
    }
    let tiledata = match (&args.target_tiledata, &client) {
        (Some(path), _) => Some(tiledata_counts(path)?),
        (None, Some(client)) => client.tiledata_counts()?,
        (None, None) => None,
    };
    if let Some(counts) = tiledata {
        let report = world.tile_ids_beyond(counts.land, counts.items);
        if !report.is_empty() {
            println!(
                "warning: the target tiledata.mul has {} land and {} item tiles: {} land cells (up to {:?}) and {} \
                 statics (up to {:?}) use ids it does not have.",
                counts.land, counts.items, report.land_cells, report.max_land, report.statics, report.max_graphic
            );
        }
    }

    std::fs::create_dir_all(&args.output_dir).wrap_err_with(|| format!("create {}", args.output_dir.display()))?;
    check_output_dir(&args.output_dir, &source.inputs)?;
    match args.to {
        ConvertTarget::Classic => {
            let containers = match args.container {
                ContainerChoice::Mul => vec![ClassicContainer::Mul],
                ContainerChoice::Uop => vec![ClassicContainer::Uop],
                ContainerChoice::Both => vec![ClassicContainer::Mul, ClassicContainer::Uop],
                ContainerChoice::Auto => {
                    let container = client
                        .as_ref()
                        .and_then(|client| client.classic_container(u32::from(target_facet)))
                        .or(source.container)
                        .ok_or_else(|| {
                            eyre::eyre!(
                                "no Classic container to default to: pass --container mul, uop or both, or \
                                 --target-client"
                            )
                        })?;
                    vec![container]
                }
            };
            let template = args.uop_template.as_deref().map(load_package).transpose()?;
            write_classic(&world, u32::from(target_facet), target_edition, &containers, template, &args.output_dir)?;
            write_converted_facet_image(
                args.facet_image,
                client.as_ref(),
                &world,
                u32::from(target_facet),
                target_edition,
                &args.output_dir,
            )?;
        }
        ConvertTarget::Enhanced | ConvertTarget::Kr => {
            if args.facet_image.is_some() {
                println!("note: --facet-image applies to --to classic only.");
            }
            let radarcol = args.radarcol.clone().or_else(|| {
                let mut dirs = source.dirs.clone();
                dirs.extend(client.as_ref().map(|client| client.dir.clone()));
                dirs.iter().find_map(|dir| find_file_ignoring_case(dir, RADARCOL_NAME))
            });
            let palette = radarcol.as_deref().map(RadarPalette::load_radarcol).transpose()?;
            let (mut package, radar) = if args.to == ConvertTarget::Kr {
                build_kr_facet_from_world(&world, target_facet, palette.as_ref(), &kr_land)?
            } else {
                build_facet_from_world(&world, target_facet, palette.as_ref())?
            };
            let path = args.output_dir.join(facet_package_file_name(target_facet, target_edition.is_variant()));
            package.save(&path)?;
            println!("Wrote {}.", path.display());
            match radar {
                Some(radar) => {
                    let path = args.output_dir.join(edition_radar_dds_file_name(target_facet, target_edition));
                    radar.save(&path)?;
                    println!("Wrote {} (colours from {}).", path.display(), radarcol.expect("a palette").display());
                }
                None => println!("No radar image written: no {RADARCOL_NAME} found; pass --radarcol."),
            }
        }
    }
    Ok(())
}

pub fn run_patch_classic(args: PatchClassicArgs) -> eyre::Result<()> {
    let facet = u32::from(args.facet);
    let sources = match (&args.client_dir, &args.map) {
        (Some(dir), _) => ClassicFacetSources::resolve(
            std::slice::from_ref(dir),
            facet,
            &SOURCE_MAP_FORMATS,
            ClassicDifOptions::NONE,
            args.map_edition.into(),
        )?,
        (None, Some(map)) => classic_files(
            facet,
            map,
            args.staidx.as_ref().expect("clap requires --staidx with --map"),
            args.statics.as_ref().expect("clap requires --statics with --map"),
            ClassicDifOptions::NONE,
        ),
        (None, None) => unreachable!("clap requires --client-dir or --map"),
    };
    if sources.base_facet.is_some() {
        eyre::bail!(
            "facet {facet} has no files of its own here: it is built from map{} and its difs; edit facet {}",
            sources.map.map_index,
            sources.map.map_index
        );
    }
    let dir = sources.map.path.parent().map(Path::to_path_buf).unwrap_or_default();
    let (mut world, _, _) = ClassicWorld::load(&sources, source_size(args.size.as_deref(), &dir, facet)?)?;
    let (x, y) = (args.x, args.y);
    let before = (world.map.cell(x, y)?, world.statics.cell_statics(x, y)?);
    if args.land.is_some() || args.z.is_some() {
        let (land, z) = world.map.cell(x, y)?;
        let land = args.land.as_deref().map(parse_number::<u16>).transpose()?.unwrap_or(land);
        world.map.set_cell(x, y, land, args.z.unwrap_or(z))?;
    }
    for index in args.remove_static {
        world.statics.remove_static(x, y, index)?;
    }
    for item in &args.set_static {
        let [index, graphic, z, hue] = parse_list::<SET_STATIC_FIELDS>(item)?;
        world.statics.set_static(x, y, to_index(index)?, to_u16(graphic, "graphic")?, to_z(z)?, to_u16(hue, "hue")?)?;
    }
    for item in &args.add_static {
        let [graphic, z, hue] = parse_list::<STATIC_FIELDS>(item)?;
        world.statics.add_static(x, y, to_u16(graphic, "graphic")?, to_z(z)?, to_u16(hue, "hue")?)?;
    }

    std::fs::create_dir_all(&args.output_dir).wrap_err_with(|| format!("create {}", args.output_dir.display()))?;
    let mut inputs = vec![sources.map.path.clone()];
    inputs.extend(sources.map.path.parent().map(Path::to_path_buf));
    check_output_dir(&args.output_dir, &inputs)?;
    let (container, template) = match sources.map.format {
        ClassicMapFormat::Mul => (ClassicContainer::Mul, None),
        ClassicMapFormat::Uop => (ClassicContainer::Uop, Some(load_package(&sources.map.path)?)),
    };
    write_classic(&world, sources.map.map_index, sources.edition, &[container], template, &args.output_dir)?;
    let after = (world.map.cell(x, y)?, world.statics.cell_statics(x, y)?);
    let changed = (after != before).then_some(ChangedTile { x, y, land_z_changed: after.0.1 != before.0.1 });
    patch_facet_image(&dir, sources.map.map_index, sources.edition, &world, changed, &args.output_dir)?;
    let (land, z) = world.map.cell(x, y)?;
    println!("({x}, {y}) in the {} edition: land 0x{land:04x} z {z}", sources.edition);
    for (index, record) in world.statics.cell_statics(x, y)?.iter().enumerate() {
        println!("  static {index}: 0x{:04x} z {} hue {}", record.graphic, record.z, record.hue);
    }
    Ok(())
}

/// `Wrote ...` for an image updated tile by tile.
fn describe_update(path: &Path, update: &ImageUpdate) -> String {
    format!(
        "Wrote {}: {} tiles re-rendered by the slope rule, {} pixels changed in {} rows ({} bit-15 pixels of unchanged \
         tiles kept).",
        path.display(),
        update.tiles_rendered,
        update.pixels_changed,
        update.rows_changed,
        update.undrawn_kept
    )
}

/// The target's facet as it is, to compare the converted one with.
fn load_target_world(dir: &Path, facet: u32, edition: MapEdition, size: MapSizeCells) -> eyre::Result<ClassicWorld> {
    let sources =
        ClassicFacetSources::resolve(&[dir.to_path_buf()], facet, &SOURCE_MAP_FORMATS, ClassicDifOptions::NONE, edition)?;
    Ok(ClassicWorld::load(&sources, Some(size))?.0)
}

/// `facet convert --to classic`: the `facetNN.mul` of a target client that draws it (`--facet-image`).
fn write_converted_facet_image(
    mode: Option<FacetImageArg>,
    client: Option<&TargetClient>,
    world: &ClassicWorld,
    facet: u32,
    edition: MapEdition,
    output_dir: &Path,
) -> eyre::Result<()> {
    let name = facet_image_file_name(facet);
    let Some(client) = client.filter(|client| client.reads_facet_image(facet)) else {
        if mode.is_some() {
            let why = match client {
                Some(client) => format!("{} does not draw {name}", client.exe.display()),
                None => "it needs a --target-client that draws facetNN.mul".to_owned(),
            };
            println!("note: --facet-image ignored: {why}.");
        }
        return Ok(());
    };
    let mode: FacetImageMode = mode.map_or(FacetImageMode::Update, Into::into);
    let current_image = find_file_ignoring_case(&client.dir, &name).and_then(|path| match FacetImageFile::load(&path) {
        Ok(image) => Some(image),
        Err(error) => {
            println!("warning: {error:#}: the target's {name} is treated as missing.");
            None
        }
    });
    let size = world.size_cells();
    let fits = current_image.as_ref().is_some_and(|image| image.check_size(size.width, size.height).is_ok());
    if edition.is_variant() && mode == FacetImageMode::Update && fits {
        println!(
            "note: {name} not written: the shipped images follow the standard edition, not the {edition} one; pass \
             --facet-image regenerate to render it from this one."
        );
        return Ok(());
    }
    let current_world = if mode == FacetImageMode::Update && fits {
        match load_target_world(&client.dir, facet, edition, size) {
            Ok(current) => Some(current),
            Err(error) => {
                println!("warning: the target's map could not be read to compare with: {error:#}");
                None
            }
        }
    } else {
        None
    };
    let renderer = FacetImageRenderer::load(&client.dir);
    let plan = match plan_facet_image(mode, world, current_image.as_ref(), current_world.as_ref(), renderer.as_ref().ok()) {
        Ok(plan) => plan,
        Err(error) => match &renderer {
            Err(load) => eyre::bail!("{name}: {error:#} ({load:#})"),
            Ok(_) => return Err(error.wrap_err(name)),
        },
    };
    let path = output_dir.join(&name);
    match plan {
        FacetImagePlan::Unchanged(image) => {
            image.save(&path)?;
            let why = if mode == FacetImageMode::Keep { "--facet-image keep" } else { "the map is the target's" };
            println!("Wrote {}: the target's image, unchanged ({why}).", path.display());
        }
        FacetImagePlan::Updated(image, update) => {
            image.save(&path)?;
            println!("{}", describe_update(&path, &update));
        }
        FacetImagePlan::Rendered(image, reason) => {
            image.save(&path)?;
            println!(
                "Wrote {} ({}x{}), rendered whole from the map by the slope rule ({}): an approximation, about \
                 {SHIPPED_STYLE_MATCH_PERCENT} % of pixels match the shipped style.",
                path.display(),
                image.width(),
                image.height(),
                reason.describe()
            );
        }
        FacetImagePlan::NotWritten => {
            let why = if mode == FacetImageMode::None { "--facet-image none" } else { "the target has none to keep" };
            println!("No {name} written ({why}).");
        }
    }
    Ok(())
}

/// `facet patch-classic`: the `facetNN.mul` next to the edited map, with the edited tile re-rendered (and, when
/// its land z changed, the tiles left of and above it).
fn patch_facet_image(
    dir: &Path,
    facet: u32,
    edition: MapEdition,
    world: &ClassicWorld,
    changed: Option<ChangedTile>,
    output_dir: &Path,
) -> eyre::Result<()> {
    if facet >= FACET_IMAGE_COUNT {
        return Ok(());
    }
    let name = facet_image_file_name(facet);
    let Some(path) = find_file_ignoring_case(dir, &name) else {
        return Ok(());
    };
    let Some(changed) = changed else {
        println!("{name} not written: the cell is unchanged.");
        return Ok(());
    };
    if edition.is_variant() {
        println!(
            "note: {name} not updated: the shipped images follow the standard edition, not the {edition} one."
        );
        return Ok(());
    }
    let mut image = FacetImageFile::load(&path)?;
    let size = world.size_cells();
    if let Err(error) = image.check_size(size.width, size.height) {
        println!("warning: {}: {error:#}; not updated.", path.display());
        return Ok(());
    }
    let renderer = match FacetImageRenderer::load(dir) {
        Ok(renderer) => renderer,
        Err(error) => {
            println!("warning: {name} not updated: {error:#}.");
            return Ok(());
        }
    };
    let update = renderer.update(world, &mut image, &[changed])?;
    let out = output_dir.join(&name);
    image.save(&out)?;
    println!("{}", describe_update(&out, &update));
    Ok(())
}

/// The size given, else the size table of the Classic client in `dir` when its executable is there; `None` leaves
/// it to the map file.
fn source_size(explicit: Option<&str>, dir: &Path, facet: u32) -> eyre::Result<Option<MapSizeCells>> {
    Ok(match explicit {
        Some(size) => Some(parse_size(size)?),
        None => TargetClient::open(dir)
            .ok()
            .filter(|client| client.family() == ClientFamily::Classic)
            .and_then(|client| client.facet_size(facet).ok()),
    })
}

fn kr_land_map(fallback: Option<&str>) -> eyre::Result<KrLandMap> {
    let mut land_map = KrLandMap::shipped();
    if let Some(fallback) = fallback.map(parse_number::<u16>).transpose()? {
        let land =
            land_map.get(fallback).ok_or_else(|| eyre::eyre!("--kr-fallback-land {fallback} is not in the land map"))?;
        land_map = land_map.with_fallback(land);
    }
    Ok(land_map)
}

fn classic_files(facet: u32, map: &Path, index: &Path, statics: &Path, difs: ClassicDifOptions) -> ClassicFacetSources {
    let is_uop = map.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(UOP_EXTENSION));
    let format = if is_uop { ClassicMapFormat::Uop } else { ClassicMapFormat::Mul };
    let dif_dirs: Vec<PathBuf> = map.parent().map(Path::to_path_buf).into_iter().collect();
    let map = ClassicMapFile { format, path: map.to_path_buf(), map_index: facet };
    let statics = StaticsFiles { index: index.to_path_buf(), statics: statics.to_path_buf() };
    ClassicFacetSources::from_files(facet, map, Some(statics), &dif_dirs, difs)
}

/// The dif kinds to bake for `--difs` from `dir`, and a note when the choice could not be told.
fn dif_options(mode: DifMode, dir: &Path, facet: u32) -> ClassicDifOptions {
    match mode {
        DifMode::Bake => ClassicDifOptions::ALL,
        DifMode::Ignore => ClassicDifOptions::NONE,
        DifMode::Auto => {
            let present = MapDifFiles::find(&[dir.to_path_buf()], facet).is_ok()
                || StaticDifFiles::find(&[dir.to_path_buf()], facet).is_ok();
            if !present {
                return ClassicDifOptions::NONE;
            }
            let usage = TargetClient::open(dir).ok().and_then(|client| client.difs().cloned());
            match usage {
                Some(usage) => {
                    let options = ClassicDifOptions {
                        map_difs: usage.land.applies() == Some(true),
                        static_difs: usage.statics.applies() == Some(true),
                    };
                    println!(
                        "Source difs: land {}, statics {} (from the client.exe of {}).",
                        usage.land.describe(),
                        usage.statics.describe(),
                        dir.display()
                    );
                    options
                }
                None => {
                    println!(
                        "warning: {} has dif files for facet {facet}, but whether its client applies them is unknown \
                         (no readable client.exe): not baked; pass --difs bake to bake them.",
                        dir.display()
                    );
                    ClassicDifOptions::NONE
                }
            }
        }
    }
}

fn load_source(args: &ConvertArgs, kr_land: &KrLandMap) -> eyre::Result<Source> {
    let facet = u32::from(args.facet);
    if let Some(uop) = &args.uop {
        let package = open_facet(uop, args.facet)?;
        let has_data_map = package
            .package()
            .unpack_file_by_hash(uocf::uop_container::hash::hash_file_name_single(
                &uocf::enhanced::facet_sector::data_map_entry_path(args.facet),
            ))?
            .is_some();
        let dir = uop.parent().map(Path::to_path_buf).unwrap_or_default();
        let package = if has_data_map {
            package
        } else {
            // No data.map: the size comes from the client's size table, when its executable is next to it.
            match TargetClient::open(&dir).and_then(|client| client.facet_size(facet)) {
                Ok(size) => {
                    println!("{} has no data.map: {}x{} from the size table.", uop.display(), size.width, size.height);
                    let geometry = uocf::enhanced::facet_sector::FacetGeometry::new(size.width, size.height);
                    let dialect = package.dialect();
                    EcFacetPackage::from_package_dialect(load_package(uop)?, args.facet, Some(geometry), dialect)?
                }
                Err(error) => {
                    println!(
                        "warning: {} has no data.map and no readable client executable is next to it ({error:#}): \
                         using the stock size of facet {facet}.",
                        uop.display()
                    );
                    package
                }
            }
        };
        let kind = if package.dialect().is_kingdom_reborn() { "Kingdom Reborn" } else { "Enhanced Client" };
        let geometry = package.geometry();
        println!("Reading the {kind} facet {} ({}x{}).", uop.display(), geometry.width, geometry.height);
        let converted = facet_to_classic(&package, Some(kr_land))?;
        if let Some(note) = converted.losses.rebuilt_delimiters() {
            println!("warning: {}: {note}", uop.display());
        }
        let losses = converted.losses.real_losses().describe();
        if !losses.is_empty() {
            let list = losses.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
            if !args.allow_lossy {
                eyre::bail!("{} does not convert without losses (pass --allow-lossy to accept them):\n{list}", uop.display());
            }
            println!("Accepted losses (--allow-lossy):\n{list}");
        }
        let stem = uop.file_stem().map(|stem| stem.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let edition = if stem.ends_with(MapEdition::Variant.file_suffix()) { MapEdition::Variant } else { MapEdition::Standard };
        return Ok(Source { world: converted.world, edition, container: None, inputs: vec![uop.clone(), dir.clone()], dirs: vec![dir] });
    }

    let (sources, dir) = match (&args.client_dir, &args.map) {
        (Some(dir), _) => {
            let difs = dif_options(args.difs, dir, facet);
            let sources = ClassicFacetSources::resolve(
                std::slice::from_ref(dir),
                facet,
                &SOURCE_MAP_FORMATS,
                difs,
                args.map_edition.into(),
            )?;
            (sources, dir.clone())
        }
        (None, Some(map)) => {
            let dir = map.parent().map(Path::to_path_buf).unwrap_or_default();
            let difs = dif_options(args.difs, &dir, facet);
            let (index, statics) = (
                args.staidx.as_ref().expect("clap requires --staidx with --map"),
                args.statics.as_ref().expect("clap requires --statics with --map"),
            );
            (classic_files(facet, map, index, statics, difs), dir)
        }
        (None, None) => eyre::bail!("give the source: --client-dir, --map (with --staidx and --statics) or --uop"),
    };
    let size = source_size(args.source_size.as_deref(), &dir, facet)?;
    let (world, map_bake, static_bake) = ClassicWorld::load(&sources, size)?;
    let world_size = world.size_cells();
    println!(
        "Reading Classic facet {facet} ({} edition, {}x{}) from {}.",
        sources.edition,
        world_size.width,
        world_size.height,
        sources.map.path.display()
    );
    if let Some(note) = sources.base_facet_warning() {
        println!("{note}");
    }
    for (kind, bake) in [("map", map_bake), ("static", static_bake)] {
        if let Some(bake) = bake {
            println!(
                "Baked the {kind} difs: {} listed, {} blocks patched (a later id wins), {} outside the map.",
                bake.listed, bake.distinct, bake.outside
            );
        }
    }
    let unsorted = world.unsorted_static_blocks();
    if unsorted > 0 && args.to != ConvertTarget::Classic {
        println!(
            "note: {unsorted} static blocks are not in (y, x) order; a facet keeps each cell's statics but not their \
             order across cells, so converting back to Classic sorts them."
        );
    }
    let container = match sources.map.format {
        ClassicMapFormat::Mul => ClassicContainer::Mul,
        ClassicMapFormat::Uop => ClassicContainer::Uop,
    };
    let mut inputs = vec![sources.map.path.clone(), dir.clone()];
    if let Ok(files) = sources.statics_files() {
        inputs.extend([files.index.clone(), files.statics.clone()]);
    }
    Ok(Source { world, edition: sources.edition, container: Some(container), inputs, dirs: vec![dir] })
}

/// Fails when `output_dir` is one of `inputs` or holds one of them.
fn check_output_dir(output_dir: &Path, inputs: &[PathBuf]) -> eyre::Result<()> {
    let output = output_dir.canonicalize().wrap_err_with(|| format!("resolve {}", output_dir.display()))?;
    for input in inputs {
        let Ok(input) = input.canonicalize() else { continue };
        if input == output || input.parent() == Some(output.as_path()) {
            eyre::bail!("the output folder {} holds the source files; choose another one", output_dir.display());
        }
    }
    Ok(())
}

/// Writes the map in `containers` and the statics, under the file names of `facet` in `edition`.
fn write_classic(
    world: &ClassicWorld,
    facet: u32,
    edition: MapEdition,
    containers: &[ClassicContainer],
    template: Option<UopPackage>,
    output_dir: &Path,
) -> eyre::Result<()> {
    let x = edition.file_suffix();
    let mut template = template;
    for container in containers {
        match container {
            ClassicContainer::Mul => {
                let path = output_dir.join(format!("map{facet}{x}.mul"));
                world.map.write_mul(&path)?;
                println!("Wrote {}.", path.display());
            }
            ClassicContainer::Uop => {
                let path = output_dir.join(format!("map{facet}{x}LegacyMUL.uop"));
                let report = world.map.write_uop(&path, facet, template.take())?;
                println!(
                    "Wrote {} ({} entries written, {} unchanged, {} removed).",
                    path.display(),
                    report.written,
                    report.unchanged,
                    report.removed
                );
            }
        }
    }
    let (index, data) = (output_dir.join(format!("staidx{facet}{x}.mul")), output_dir.join(format!("statics{facet}{x}.mul")));
    world.statics.write_mul(&index, &data)?;
    let size = world.size_cells();
    println!(
        "Wrote {} and {} ({}x{}, {} statics).",
        index.display(),
        data.display(),
        size.width,
        size.height,
        world.statics.static_count()
    );
    Ok(())
}

fn to_u16(value: i64, what: &str) -> eyre::Result<u16> {
    u16::try_from(value).map_err(|_| eyre::eyre!("{what} {value} is outside 0..=0xFFFF (the Classic statics field)"))
}

fn to_index(value: i64) -> eyre::Result<usize> {
    usize::try_from(value).map_err(|_| eyre::eyre!("static index {value} is negative"))
}

/// Parses `WxH`.
fn parse_size(text: &str) -> eyre::Result<MapSizeCells> {
    let (width, height) =
        text.split_once(SIZE_SEPARATOR).ok_or_else(|| eyre::eyre!("--size {text:?}: expected WIDTHxHEIGHT"))?;
    Ok(MapSizeCells { width: parse_number(width)?, height: parse_number(height)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facet::FacetCmd;
    use clap::Parser;

    /// A 64x64 facet (one sector) with land, z and statics in a few cells.
    const SIDE: u32 = 64;
    const FACET: &str = "4";
    const SIZE: &str = "64x64";
    const EDITED: [(u32, u32, u16, i8); 3] = [(0, 0, 3, 5), (40, 9, 0x10, -4), (63, 63, 0x50, 0)];
    const STATICS: [(u32, u32, u16, i8, u16); 3] = [(1, 1, 0x0E75, 2, 0), (1, 1, 0x0E76, 9, 0x21), (60, 30, 0x1BA4, 0, 0)];

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: FacetCmd,
    }

    fn run(args: &[&str]) -> eyre::Result<()> {
        let mut argv = vec!["facet"];
        argv.extend_from_slice(args);
        crate::facet::run(Cli::try_parse_from(argv)?.cmd)
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uocf_cli_facet_convert_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn world() -> ClassicWorld {
        let size = MapSizeCells { width: SIDE, height: SIDE };
        let mut world = ClassicWorld::filled(size, LandFill::VOID).unwrap();
        for (x, y, land, z) in EDITED {
            world.map.set_cell(x, y, land, z).unwrap();
        }
        for (x, y, graphic, z, hue) in STATICS {
            world.statics.add_static(x, y, graphic, z, hue).unwrap();
        }
        world
    }

    fn write_source(dir: &Path) -> ClassicWorld {
        let world = world();
        world.map.write_mul(&dir.join(format!("map{FACET}.mul"))).unwrap();
        world.statics.write_mul(&dir.join(format!("staidx{FACET}.mul")), &dir.join(format!("statics{FACET}.mul"))).unwrap();
        world
    }

    fn read_bytes(dir: &Path, names: &[String]) -> Vec<Vec<u8>> {
        names.iter().map(|name| std::fs::read(dir.join(name)).unwrap()).collect()
    }

    fn classic_names() -> Vec<String> {
        vec![format!("map{FACET}.mul"), format!("staidx{FACET}.mul"), format!("statics{FACET}.mul")]
    }

    #[test]
    fn classic_converts_to_enhanced_and_kr_and_back_byte_for_byte() {
        let root = temp("round_trip");
        let (source, ec, kr, back_ec, back_kr) =
            (root.join("source"), root.join("ec"), root.join("kr"), root.join("back_ec"), root.join("back_kr"));
        std::fs::create_dir_all(&source).unwrap();
        write_source(&source);
        let source_arg = source.to_str().unwrap();
        run(&["convert", "--client-dir", source_arg, "--facet", FACET, "--source-size", SIZE, "--to", "enhanced", "-o", ec.to_str().unwrap()]).unwrap();
        run(&["convert", "--client-dir", source_arg, "--facet", FACET, "--source-size", SIZE, "--to", "kr", "-o", kr.to_str().unwrap()]).unwrap();
        let uop = |dir: &Path| dir.join(facet_package_file_name(4, false)).to_str().unwrap().to_owned();
        for (from, to) in [(&ec, &back_ec), (&kr, &back_kr)] {
            run(&["convert", "--uop", &uop(from), "--facet", FACET, "--to", "classic", "--container", "mul", "-o", to.to_str().unwrap()]).unwrap();
            assert_eq!(read_bytes(to, &classic_names()), read_bytes(&source, &classic_names()));
        }
        // The UOP container, and the UOP back to MUL.
        let (uop_dir, mul_dir) = (root.join("uop"), root.join("mul"));
        run(&["convert", "--client-dir", source_arg, "--facet", FACET, "--source-size", SIZE, "--to", "classic", "--container", "uop", "-o", uop_dir.to_str().unwrap()]).unwrap();
        run(&["convert", "--client-dir", uop_dir.to_str().unwrap(), "--facet", FACET, "--source-size", SIZE, "--to", "classic", "--container", "mul", "-o", mul_dir.to_str().unwrap()]).unwrap();
        assert_eq!(read_bytes(&mul_dir, &classic_names()), read_bytes(&source, &classic_names()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shrinking_refuses_content_outside_unless_cropping_and_growing_fills() {
        let root = temp("resize");
        let (source, small, big) = (root.join("source"), root.join("small"), root.join("big"));
        std::fs::create_dir_all(&source).unwrap();
        write_source(&source);
        let base = ["convert", "--client-dir", source.to_str().unwrap(), "--facet", FACET, "--source-size", SIZE, "--to", "classic", "--container", "mul"];
        let shrink = |extra: &[&str]| {
            let mut args = base.to_vec();
            args.extend_from_slice(&["--size", "56x64", "-o", small.to_str().unwrap()]);
            args.extend_from_slice(extra);
            run(&args)
        };
        let error = format!("{:#}", shrink(&[]).unwrap_err());
        assert!(error.contains("--crop") && error.contains("(63, 63)"), "{error}");
        shrink(&["--crop"]).unwrap();
        let cropped = ClassicMapBlocks::read_mul(&small.join(format!("map{FACET}.mul")), 4, Some(MapSizeCells { width: 56, height: 64 })).unwrap();
        assert_eq!(cropped.cell(40, 9).unwrap(), (0x10, -4));

        let mut args = base.to_vec();
        args.extend_from_slice(&["--size", "72x64", "--fill-land", "0x3", "--fill-z", "-1", "-o", big.to_str().unwrap()]);
        run(&args).unwrap();
        let grown = ClassicMapBlocks::read_mul(&big.join(format!("map{FACET}.mul")), 4, Some(MapSizeCells { width: 72, height: 64 })).unwrap();
        assert_eq!((grown.cell(70, 3).unwrap(), grown.cell(0, 0).unwrap()), ((3, -1), (3, 5)));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_delimiters_only_warn_and_a_real_loss_needs_allow_lossy() {
        /// A hue the Classic static record (u16) cannot hold.
        const WIDE_HUE: u32 = 0x1_0000;
        let root = temp("lossy");
        let (source, ec, back, wide, wide_back) =
            (root.join("source"), root.join("ec"), root.join("back"), root.join("wide"), root.join("wide_back"));
        for dir in [&source, &ec, &wide] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let (mut package, _) = build_facet_from_world(&write_source(&source), 4, None).unwrap();
        let path = ec.join(facet_package_file_name(4, false));
        // A delimiter only the client's files hold: a stale copy of a neighbour, rebuilt by the rule.
        let square = &mut package.sector_mut(0).unwrap().tiles[uocf::enhanced::facet_sector::square_index(0, 63)];
        square.delimiters.push(uocf::enhanced::facet_decoder::EcFacetDelimiter { direction: 0, z: 0, graphic_id: 1 });
        package.save(&path).unwrap();
        let to_classic = |from: &Path, to: &Path| {
            vec![
                "convert".to_owned(), "--uop".to_owned(), from.to_str().unwrap().to_owned(), "--facet".to_owned(),
                FACET.to_owned(), "--to".to_owned(), "classic".to_owned(), "--container".to_owned(), "mul".to_owned(),
                "-o".to_owned(), to.to_str().unwrap().to_owned(),
            ]
        };
        let args = to_classic(&path, &back);
        run(&args.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
        assert_eq!(read_bytes(&back, &classic_names()), read_bytes(&source, &classic_names()));

        // A hue above 0xFFFF is a real loss: refused unless --allow-lossy.
        let (x, y, ..) = STATICS[0];
        package.sector_mut(0).unwrap().tiles[uocf::enhanced::facet_sector::square_index(x, y)].statics[0].hue = WIDE_HUE;
        let wide_path = wide.join(facet_package_file_name(4, false));
        package.save(&wide_path).unwrap();
        let mut args = to_classic(&wide_path, &wide_back);
        let error = format!("{:#}", run(&args.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_err());
        assert!(error.contains("--allow-lossy") && error.contains("hue") && !error.contains("delimiters"), "{error}");
        args.push("--allow-lossy".to_owned());
        run(&args.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn patch_classic_edits_one_cell_of_a_uop_map() {
        let root = temp("patch");
        let (source, uop_dir, out) = (root.join("source"), root.join("uop"), root.join("out"));
        std::fs::create_dir_all(&source).unwrap();
        write_source(&source);
        run(&["convert", "--client-dir", source.to_str().unwrap(), "--facet", FACET, "--source-size", SIZE, "--to", "classic", "--container", "uop", "-o", uop_dir.to_str().unwrap()]).unwrap();
        let map = uop_dir.join(format!("map{FACET}LegacyMUL.uop"));
        let (index, data) = (uop_dir.join(format!("staidx{FACET}.mul")), uop_dir.join(format!("statics{FACET}.mul")));
        run(&[
            "patch-classic", "--map", map.to_str().unwrap(), "--staidx", index.to_str().unwrap(), "--statics",
            data.to_str().unwrap(), "--facet", FACET, "--size", SIZE, "--x", "1", "--y", "1", "--land", "0x99", "--z", "-3",
            "--remove-static", "0", "--add-static", "0x1234,7,0x22", "-o", out.to_str().unwrap(),
        ])
        .unwrap();
        let size = MapSizeCells { width: SIDE, height: SIDE };
        let patched = ClassicMapBlocks::read_uop(&out.join(format!("map{FACET}LegacyMUL.uop")), 4, Some(size)).unwrap();
        assert_eq!(patched.cell(1, 1).unwrap(), (0x99, -3));
        let statics = ClassicStaticsBlocks::read_mul(&out.join(format!("staidx{FACET}.mul")), &out.join(format!("statics{FACET}.mul")), size).unwrap();
        let cell: Vec<_> = statics.cell_statics(1, 1).unwrap().iter().map(|record| (record.graphic, record.z, record.hue)).collect();
        assert_eq!(cell, [(0x0E76, 9, 0x21), (0x1234, 7, 0x22)]);
        assert!(run(&["patch-classic", "--map", map.to_str().unwrap(), "--staidx", index.to_str().unwrap(), "--statics", data.to_str().unwrap(), "--facet", FACET, "--size", SIZE, "--x", "1", "--y", "1", "-o", uop_dir.to_str().unwrap()]).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn patch_classic_re_renders_the_facet_image_rows_of_the_edit_only() {
        /// The size of a UO Gold `tiledata.mul` (16,384 items), read as all heights 0.
        const GOLD_TILEDATA_LEN: usize = 1_036_288;
        /// Radar colours for the land ids used here (the void 0x244 too): greys.
        const RADAR_COLOURS: u16 = 0x300;
        const GREY_STEP: u16 = 0x0421;
        const HILL_Z: &str = "20";
        let root = temp("patch_image");
        let (source, out, same) = (root.join("source"), root.join("out"), root.join("same"));
        std::fs::create_dir_all(&source).unwrap();
        let world = write_source(&source);
        let radarcol: Vec<_> = (0..RADAR_COLOURS).map(|id| uocf::utils::color::Rgb555::new_from_val(id % 0x20 * GREY_STEP)).collect();
        std::fs::write(source.join(RADARCOL_NAME), uocf::classic::radarcol::encode_radarcol(&radarcol)).unwrap();
        std::fs::write(source.join("tiledata.mul"), vec![0u8; GOLD_TILEDATA_LEN]).unwrap();
        let image_name = facet_image_file_name(4);
        let image = FacetImageRenderer::load(&source).unwrap().render(&world).unwrap();
        image.save(&source.join(&image_name)).unwrap();
        let patch = |extra: &[&str], out: &Path| {
            let mut args = vec!["patch-classic", "--client-dir", source.to_str().unwrap(), "--facet", FACET, "--size", SIZE, "--x", "9", "--y", "6"];
            args.extend_from_slice(extra);
            args.extend_from_slice(&["-o", out.to_str().unwrap()]);
            run(&args)
        };
        patch(&["--z", HILL_Z], &out).unwrap();
        let patched = FacetImageFile::load(&out.join(&image_name)).unwrap();
        let changed_rows: Vec<usize> = (0..image.rows().len()).filter(|&row| image.rows()[row] != patched.rows()[row]).collect();
        // Tile (9, 6) keeps its slope; (8, 6) now rises and (9, 5) falls.
        assert_eq!(changed_rows, [5, 6]);
        let (before, after) = (image.pixels(), patched.pixels());
        let differing: Vec<usize> = (0..before.len()).filter(|&at| before[at] != after[at]).collect();
        assert_eq!(differing, [5 * SIDE as usize + 9, 6 * SIDE as usize + 8]);
        // An edit that changes nothing writes no image.
        patch(&["--z", "0"], &same).unwrap();
        assert!(!same.join(&image_name).exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sizes_parse_as_width_x_height() {
        assert_eq!(parse_size("7168x4096").unwrap(), MapSizeCells { width: 7168, height: 4096 });
        assert!(parse_size("7168").is_err());
    }
}
