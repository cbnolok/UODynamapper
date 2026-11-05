//! Whole-facet conversion behind `uocf-cli facet convert` and the inspector's Maps tab: any of Classic MUL, Classic
//! UOP, Enhanced Client facet and Kingdom Reborn facet to any other, with a resize to another client's facet size. It
//! keeps the world map image (`facetNN.mul`) of a client that draws it in step with the map written: the 7.0.x clients
//! read it without bounds checks, tile (x, y) at column x of row y, so an image of another size draws the wrong tiles.
//!
//! The callers keep their arguments, dialogs and output: they hand over a [`FacetConversion`], name their own flags or
//! controls in the messages ([`ConvertHints`]) and receive what is done line by line ([`Report`]).
//!
//! With [`FacetConversion::keep_unchanged`] a save without changes is the source as it is: a Classic file to its own
//! container, facet, edition and size is copied, and so is a facet package to its own kind. Rewriting a Classic file
//! through the encoders would not always give its bytes: in the old clients' files a map block's header is not its
//! index, and a static index entry's third field varies. A Classic UOP written over the same facet with a change keeps
//! the source's layout and its unchanged entries.

use std::path::{Path, PathBuf};

use crate::classic::facet_image::{facet_image_file_name, FacetImageFile};
use crate::classic::facet_image_render::{
    plan_facet_image, FacetImageMode, FacetImagePlan, FacetImageRenderer, ImageUpdate, SHIPPED_STYLE_MATCH_PERCENT,
};
use crate::classic::facet_sources::{
    ClassicDifOptions, ClassicFacetSources, ClassicMapFile, ClassicMapFormat, MapDifFiles, StaticDifFiles, StaticsFiles,
};
use crate::classic::map::MapSizeCells;
use crate::classic::target_client::{tiledata_counts, ClassicContainer, TargetClient};
use crate::classic::world::{ClassicWorld, LandFill};
use crate::client_exe::facet_dimensions::ClientFamily;
use crate::enhanced::facet_encoder::build_facet_from_world;
use crate::enhanced::facet_package::{detect_facet_dialect, EcFacetPackage};
use crate::enhanced::facet_radar::{edition_radar_dds_file_name, RadarPalette};
use crate::enhanced::facet_sector::{data_map_entry_path, facet_package_file_name, FacetGeometry};
use crate::enhanced::facet_to_classic::facet_to_classic;
use crate::kr::facet::facet_encoder::build_kr_facet_from_world;
use crate::kr::facet::kr_land_map::KrLandMap;
use crate::map_edition::MapEdition;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;
use crate::utils::output_file::write_through_temp;
use crate::utils::path::find_file_ignoring_case;

crate::eyre_imports!();

/// The map containers a Classic source folder is searched for, in order.
pub const SOURCE_MAP_FORMATS: [ClassicMapFormat; 2] = [ClassicMapFormat::Uop, ClassicMapFormat::Mul];
pub const RADARCOL_NAME: &str = "radarcol.mul";
const UOP_EXTENSION: &str = "uop";
/// The tool that changes a client's facet size table.
const PATCHER_TOOL: &str = "uo-client-patcher-gui";

/// Receives what a conversion does, line by line, as it goes.
pub type Report<'a> = &'a mut dyn FnMut(String);

/// What to do with the dif files of a Classic source.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SourceDifs {
    /// Bake the kinds the source client applies, told by its executable; nothing when that is unknown.
    #[default]
    AsTheClientApplies,
    /// Bake every dif file found: each id in file order, a later one winning, as the client applies them.
    Bake,
    Ignore,
}

#[derive(Clone, Debug)]
pub enum ConversionSource {
    /// A Classic client folder.
    ClassicFolder { dir: PathBuf, edition: MapEdition },
    /// A Classic `mapN.mul` or `mapNLegacyMUL.uop` with its statics.
    ClassicFiles { map: PathBuf, index: PathBuf, statics: PathBuf },
    /// An Enhanced Client or Kingdom Reborn `facetN.uop`, told apart by its contents.
    Package(PathBuf),
}

/// The Classic map container(s) written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClassicContainers {
    /// The one the target client ships, else the source's (Classic source).
    Auto,
    Mul,
    Uop,
    Both,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConversionTarget {
    Classic(ClassicContainers),
    Enhanced,
    KingdomReborn,
}

impl ConversionTarget {
    fn family(self) -> ClientFamily {
        match self {
            Self::Classic(_) => ClientFamily::Classic,
            Self::Enhanced => ClientFamily::Enhanced,
            Self::KingdomReborn => ClientFamily::KingdomReborn,
        }
    }
}

/// A facet conversion, without the values a caller parses from text ([`ParsedValues`]).
#[derive(Clone, Debug)]
pub struct FacetConversion {
    pub source: ConversionSource,
    pub facet: u8,
    /// A Classic source's dif files.
    pub difs: SourceDifs,
    pub target: ConversionTarget,
    pub target_facet: u8,
    /// `None` for the source's.
    pub target_edition: Option<MapEdition>,
    /// A client folder or executable written for: its size table gives the size, its folder the Classic container,
    /// the `tiledata.mul` of the id check and the world map image of a client that draws one.
    pub target_client: Option<PathBuf>,
    /// Lets a smaller size drop the content outside it.
    pub crop: bool,
    /// A `mapNLegacyMUL.uop` whose layout the written UOP keeps.
    pub uop_template: Option<PathBuf>,
    /// The `tiledata.mul` of the id check, instead of the target client's.
    pub target_tiledata: Option<PathBuf>,
    /// Accepts what the Classic files cannot hold of a facet package.
    pub allow_lossy: bool,
    /// The `radarcol.mul` of a facet's radar image, instead of one in the source's or the target client's folder.
    pub radarcol: Option<PathBuf>,
    /// Kingdom Reborn's land materials, and what stands for the ids they lack.
    pub kr_land: KrLandMap,
    /// `None` is [`FacetImageMode::Update`] without a word about a target that draws no image.
    pub facet_image: Option<FacetImageMode>,
    /// A save without changes is the source as it is (see the module documentation).
    pub keep_unchanged: bool,
    pub output_dir: PathBuf,
}

/// What a caller parses from text. A value it could not read is an error only when the conversion gets to it, after
/// what the conversion has reported until then, and not at all when it is not needed.
pub struct ParsedValues {
    /// The size of a Classic source no client describes; `None` for the source client's size table, else the size the
    /// map file holds for the facet's height.
    pub source_size: eyre::Result<Option<MapSizeCells>>,
    /// The size written; `None` for the target client's size table, else the source's size.
    pub size: eyre::Result<Option<MapSizeCells>>,
    /// What a larger size adds.
    pub fill: eyre::Result<LandFill>,
}

/// How a caller's messages name what its user can change: its flags or its controls.
pub struct ConvertHints {
    /// How to ask for the standard edition.
    pub standard_edition: &'static str,
    /// What told the target's family, before `{family} files`.
    pub target_is: &'static str,
    /// What told the size, before `{width}x{height}`.
    pub size_is: &'static str,
    /// How to allow a crop.
    pub crop: &'static str,
    /// How to accept losses.
    pub allow_lossy: &'static str,
    /// The heading of the losses accepted.
    pub accepted_losses: &'static str,
    /// How to bake difs whose use is unknown.
    pub bake_difs: &'static str,
    /// How to tell a Classic container.
    pub choose_container: &'static str,
    /// The world map image choice, when it does not apply: `{image_choice} ignored: ...`.
    pub image_choice: &'static str,
    /// Why the choice was not applied without a target client.
    pub image_needs_client: &'static str,
    /// Said of a package target: the choice applies to a Classic one only.
    pub image_classic_only: &'static str,
    /// Why an unchanged image was written although the map differs.
    pub image_kept: &'static str,
    /// Why no image was written.
    pub image_refused: &'static str,
    /// How to render an image from a variant edition.
    pub image_regenerate: &'static str,
    /// Where a `radarcol.mul` was looked for, after `no radarcol.mul`.
    pub no_radarcol: &'static str,
}

/// The files of a Classic source when the world is what they hold.
struct ClassicFiles {
    map: PathBuf,
    map_format: ClassicMapFormat,
    index: PathBuf,
    data: PathBuf,
}

/// A facet read for the conversion.
struct Source {
    world: ClassicWorld,
    edition: MapEdition,
    /// The Classic container, for a Classic source.
    container: Option<ClassicContainer>,
    /// With `keep_unchanged`, the package of a Classic UOP source, whose layout a UOP written over the same facet keeps.
    template: Option<UopPackage>,
    /// With `keep_unchanged`, the files of a Classic source when the world is what they hold, without baked difs.
    unchanged: Option<ClassicFiles>,
    /// Folders and files the outputs must not overwrite.
    inputs: Vec<PathBuf>,
    /// Folders a radarcol.mul may be found in.
    dirs: Vec<PathBuf>,
}

/// A facet package opened, not yet converted: one that is saved as it is never needs that.
struct PackageSource {
    path: PathBuf,
    dir: PathBuf,
    package: EcFacetPackage,
    edition: MapEdition,
}

enum OpenedSource {
    Package(PackageSource),
    Classic(Source),
}

/// Converts and saves the facet. `report` gets what is done as it goes; what was reported stays true of a conversion
/// that fails later.
pub fn convert_facet(request: &FacetConversion, values: ParsedValues, hints: &ConvertHints, report: Report<'_>) -> eyre::Result<()> {
    let ParsedValues { source_size, size, fill } = values;
    let (source, client, size) = match open_source(request, source_size, hints, report)? {
        OpenedSource::Package(mut package) if request.keep_unchanged => {
            let (client, size) = open_target(request, package.edition, size, hints)?;
            if is_same_package(request, &package, size) {
                prepare_output_dir(&request.output_dir, &[package.path.clone(), package.dir.clone()])?;
                let output = request.output_dir.join(facet_package_file_name(request.target_facet, package.edition.is_variant()));
                package.package.save(&output)?;
                report(format!("Wrote {}: the package as it is.", output.display()));
                return Ok(());
            }
            (package_world(package, request, hints, report)?, client, size)
        }
        OpenedSource::Package(package) => {
            let source = package_world(package, request, hints, report)?;
            let (client, size) = open_target(request, source.edition, size, hints)?;
            (source, client, size)
        }
        OpenedSource::Classic(source) => {
            let (client, size) = open_target(request, source.edition, size, hints)?;
            (source, client, size)
        }
    };

    let mut world = source.world;
    let from = world.size_cells();
    let size = size.unwrap_or(from);
    if size != from {
        let (resized, resize) = world.resized(size, fill?, request.crop).wrap_err_with(|| {
            format!("the target is smaller; {} to drop that content, or give the target client a bigger map with {PATCHER_TOOL}", hints.crop)
        })?;
        report(format!("Resized {}x{} to {}x{}.", from.width, from.height, size.width, size.height));
        for line in resize.describe() {
            report(format!("  {line}"));
        }
        world = resized;
    }

    let target_facet = u32::from(request.target_facet);
    let target_edition = request.target_edition.unwrap_or(source.edition);
    if let Some(client) = &client {
        check_target_client(client, request.target_facet, report);
    }
    let tiledata = match (&request.target_tiledata, &client) {
        (Some(path), _) => Some(tiledata_counts(path)?),
        (None, Some(client)) => client.tiledata_counts()?,
        (None, None) => None,
    };
    if let Some(counts) = tiledata {
        let beyond = world.tile_ids_beyond(counts.land, counts.items);
        if !beyond.is_empty() {
            report(format!(
                "warning: the target tiledata.mul has {} land and {} item tiles: {} land cells (up to {:?}) and {} \
                 statics (up to {:?}) use ids it does not have.",
                counts.land, counts.items, beyond.land_cells, beyond.max_land, beyond.statics, beyond.max_graphic
            ));
        }
    }

    prepare_output_dir(&request.output_dir, &source.inputs)?;
    match request.target {
        ConversionTarget::Classic(containers) => {
            let containers = match containers {
                ClassicContainers::Mul => vec![ClassicContainer::Mul],
                ClassicContainers::Uop => vec![ClassicContainer::Uop],
                ClassicContainers::Both => vec![ClassicContainer::Mul, ClassicContainer::Uop],
                ClassicContainers::Auto => {
                    let container = client
                        .as_ref()
                        .and_then(|client| client.classic_container(target_facet))
                        .or(source.container)
                        .ok_or_else(|| eyre!("no Classic container to default to: {}", hints.choose_container))?;
                    vec![container]
                }
            };
            let same_names = request.target_facet == request.facet && target_edition == source.edition;
            let template = match &request.uop_template {
                Some(path) => Some(load_package(path)?),
                None => source.template.filter(|_| same_names),
            };
            let unchanged = source.unchanged.as_ref().filter(|_| same_names && size == from);
            write_classic_files(&world, target_facet, target_edition, &containers, template, unchanged, &request.output_dir, report)?;
            write_facet_image(request, client.as_ref(), &world, target_edition, hints, report)?;
        }
        ConversionTarget::Enhanced | ConversionTarget::KingdomReborn => {
            if request.facet_image.is_some() {
                report(format!("note: {}", hints.image_classic_only));
            }
            let mut dirs = source.dirs;
            dirs.extend(client.as_ref().map(|client| client.dir.clone()));
            write_package(&world, request, target_edition, &dirs, hints, report)?;
        }
    }
    Ok(())
}

fn load_package(path: &Path) -> eyre::Result<UopPackage> {
    UopPackage::load(path).wrap_err_with(|| format!("load {}", path.display()))
}

fn open_source(request: &FacetConversion, source_size: eyre::Result<Option<MapSizeCells>>, hints: &ConvertHints, report: Report<'_>) -> eyre::Result<OpenedSource> {
    let facet = u32::from(request.facet);
    let (sources, dir) = match &request.source {
        ConversionSource::Package(path) => return Ok(OpenedSource::Package(open_package(path, request.facet, report)?)),
        ConversionSource::ClassicFolder { dir, edition } => {
            let difs = dif_options(request.difs, dir, facet, hints, report);
            (ClassicFacetSources::resolve(std::slice::from_ref(dir), facet, &SOURCE_MAP_FORMATS, difs, *edition)?, dir.clone())
        }
        ConversionSource::ClassicFiles { map, index, statics } => {
            let dir = map.parent().map(Path::to_path_buf).unwrap_or_default();
            let difs = dif_options(request.difs, &dir, facet, hints, report);
            (classic_files(facet, map, index, statics, difs), dir)
        }
    };
    let size = match source_size? {
        Some(size) => Some(size),
        None => client_source_size(&dir, facet),
    };
    Ok(OpenedSource::Classic(load_classic(request, &sources, dir, size, report)?))
}

fn open_package(path: &Path, facet: u8, report: Report<'_>) -> eyre::Result<PackageSource> {
    let loaded = load_package(path)?;
    let dialect = detect_facet_dialect(&loaded, facet).wrap_err_with(|| format!("read {}", path.display()))?;
    let package = EcFacetPackage::from_package_dialect(loaded, facet, None, dialect)?;
    let has_data_map = package.package().unpack_file_by_hash(hash_file_name_single(&data_map_entry_path(facet)))?.is_some();
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let package = if has_data_map {
        package
    } else {
        // No data.map: the size comes from the client's size table, when its executable is next to it.
        match TargetClient::open(&dir).and_then(|client| client.facet_size(u32::from(facet))) {
            Ok(size) => {
                report(format!("{} has no data.map: {}x{} from the size table.", path.display(), size.width, size.height));
                EcFacetPackage::from_package_dialect(load_package(path)?, facet, Some(FacetGeometry::new(size.width, size.height)), dialect)?
            }
            Err(error) => {
                report(format!(
                    "warning: {} has no data.map and no readable client executable is next to it ({error:#}): using the \
                     stock size of facet {facet}.",
                    path.display()
                ));
                package
            }
        }
    };
    let kind = if package.dialect().is_kingdom_reborn() { "Kingdom Reborn" } else { "Enhanced Client" };
    let geometry = package.geometry();
    report(format!("Reading the {kind} facet {} ({}x{}).", path.display(), geometry.width, geometry.height));
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let edition = if stem.ends_with(MapEdition::Variant.file_suffix()) { MapEdition::Variant } else { MapEdition::Standard };
    Ok(PackageSource { path: path.to_path_buf(), dir, package, edition })
}

/// Whether the target is the package's own kind, facet, edition and size.
fn is_same_package(request: &FacetConversion, package: &PackageSource, size: Option<MapSizeCells>) -> bool {
    let family = if package.package.dialect().is_kingdom_reborn() { ClientFamily::KingdomReborn } else { ClientFamily::Enhanced };
    let geometry = package.package.geometry();
    request.target.family() == family
        && request.target_facet == request.facet
        && request.target_edition.unwrap_or(package.edition) == package.edition
        && size.is_none_or(|size| size == MapSizeCells { width: geometry.width, height: geometry.height })
}

fn package_world(source: PackageSource, request: &FacetConversion, hints: &ConvertHints, report: Report<'_>) -> eyre::Result<Source> {
    let PackageSource { path, dir, package, edition } = source;
    let converted = facet_to_classic(&package, Some(&request.kr_land))?;
    if let Some(note) = converted.losses.rebuilt_delimiters() {
        report(format!("warning: {}: {note}", path.display()));
    }
    let losses = converted.losses.real_losses().describe();
    if !losses.is_empty() {
        let list = losses.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
        if !request.allow_lossy {
            eyre::bail!("{} does not convert without losses ({} to accept them):\n{list}", path.display(), hints.allow_lossy);
        }
        report(format!("{}:\n{list}", hints.accepted_losses));
    }
    Ok(Source {
        world: converted.world,
        edition,
        container: None,
        template: None,
        unchanged: None,
        inputs: vec![path, dir.clone()],
        dirs: vec![dir],
    })
}

/// The Classic files of `facet` named one by one; dif files are looked for next to the map.
pub fn classic_files(facet: u32, map: &Path, index: &Path, statics: &Path, difs: ClassicDifOptions) -> ClassicFacetSources {
    let is_uop = map.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(UOP_EXTENSION));
    let format = if is_uop { ClassicMapFormat::Uop } else { ClassicMapFormat::Mul };
    let dif_dirs: Vec<PathBuf> = map.parent().map(Path::to_path_buf).into_iter().collect();
    let map = ClassicMapFile { format, path: map.to_path_buf(), map_index: facet };
    let statics = StaticsFiles { index: index.to_path_buf(), statics: statics.to_path_buf() };
    ClassicFacetSources::from_files(facet, map, Some(statics), &dif_dirs, difs)
}

/// The size table of the Classic client in `dir`, when its executable is there; `None` leaves the size to the map file.
pub fn client_source_size(dir: &Path, facet: u32) -> Option<MapSizeCells> {
    TargetClient::open(dir).ok().filter(|client| client.family() == ClientFamily::Classic).and_then(|client| client.facet_size(facet).ok())
}

/// The dif kinds to bake from `dir`, and a note when the choice could not be told.
fn dif_options(difs: SourceDifs, dir: &Path, facet: u32, hints: &ConvertHints, report: Report<'_>) -> ClassicDifOptions {
    match difs {
        SourceDifs::Bake => ClassicDifOptions::ALL,
        SourceDifs::Ignore => ClassicDifOptions::NONE,
        SourceDifs::AsTheClientApplies => {
            let present = MapDifFiles::find(&[dir.to_path_buf()], facet).is_ok() || StaticDifFiles::find(&[dir.to_path_buf()], facet).is_ok();
            if !present {
                return ClassicDifOptions::NONE;
            }
            match TargetClient::open(dir).ok().and_then(|client| client.difs().cloned()) {
                Some(usage) => {
                    let options = ClassicDifOptions {
                        map_difs: usage.land.applies() == Some(true),
                        static_difs: usage.statics.applies() == Some(true),
                    };
                    report(format!(
                        "Source difs: land {}, statics {} (from the client.exe of {}).",
                        usage.land.describe(),
                        usage.statics.describe(),
                        dir.display()
                    ));
                    options
                }
                None => {
                    report(format!(
                        "warning: {} has dif files for facet {facet}, but whether its client applies them is unknown (no \
                         readable client.exe): not baked; {} to bake them.",
                        dir.display(),
                        hints.bake_difs
                    ));
                    ClassicDifOptions::NONE
                }
            }
        }
    }
}

fn load_classic(request: &FacetConversion, sources: &ClassicFacetSources, dir: PathBuf, size: Option<MapSizeCells>, report: Report<'_>) -> eyre::Result<Source> {
    let (world, map_bake, static_bake) = ClassicWorld::load(sources, size)?;
    let baked = map_bake.is_some() || static_bake.is_some();
    let world_size = world.size_cells();
    report(format!(
        "Reading Classic facet {} ({} edition, {}x{}) from {}.",
        request.facet,
        sources.edition,
        world_size.width,
        world_size.height,
        sources.map.path.display()
    ));
    if let Some(note) = sources.base_facet_warning() {
        report(note);
    }
    for (kind, bake) in [("map", map_bake), ("static", static_bake)] {
        if let Some(bake) = bake {
            report(format!(
                "Baked the {kind} difs: {} listed, {} blocks patched (a later id wins), {} outside the map.",
                bake.listed, bake.distinct, bake.outside
            ));
        }
    }
    let is_classic = matches!(request.target, ConversionTarget::Classic(_));
    let unsorted = world.unsorted_static_blocks();
    if unsorted > 0 && !is_classic {
        report(format!(
            "note: {unsorted} static blocks are not in (y, x) order; a facet keeps each cell's statics but not their \
             order across cells, so converting back to Classic sorts them."
        ));
    }
    let container = match sources.map.format {
        ClassicMapFormat::Mul => ClassicContainer::Mul,
        ClassicMapFormat::Uop => ClassicContainer::Uop,
    };
    let template = match sources.map.format {
        ClassicMapFormat::Uop if request.keep_unchanged && is_classic => Some(load_package(&sources.map.path)?),
        _ => None,
    };
    let mut inputs = vec![sources.map.path.clone(), dir.clone()];
    let mut unchanged = None;
    if let Ok(files) = sources.statics_files() {
        inputs.extend([files.index.clone(), files.statics.clone()]);
        if request.keep_unchanged && !baked {
            unchanged = Some(ClassicFiles { map: sources.map.path.clone(), map_format: sources.map.format, index: files.index.clone(), data: files.statics.clone() });
        }
    }
    Ok(Source { world, edition: sources.edition, container: Some(container), template, unchanged, inputs, dirs: vec![dir] })
}

/// The target client, checked against the target, and the size asked for, when one is: the client's size table refuses
/// another one.
fn open_target(request: &FacetConversion, source_edition: MapEdition, size: eyre::Result<Option<MapSizeCells>>, hints: &ConvertHints) -> eyre::Result<(Option<TargetClient>, Option<MapSizeCells>)> {
    if request.target == ConversionTarget::KingdomReborn && request.target_edition.unwrap_or(source_edition).is_variant() {
        eyre::bail!("Kingdom Reborn has no variant facet packages: {}", hints.standard_edition);
    }
    let client = request.target_client.as_deref().map(TargetClient::open).transpose()?;
    if let Some(client) = &client {
        let expected = request.target.family();
        if client.family() != expected {
            eyre::bail!("{} is a {} client, but {} {} files", client.exe.display(), client.family().label(), hints.target_is, expected.label());
        }
    }
    let table = client.as_ref().map(|client| client.facet_size(u32::from(request.target_facet))).transpose()?;
    let size = match (size?, table) {
        (Some(size), Some(table)) if size != table => eyre::bail!(
            "{}{}x{} differs from the {}x{} the size table of {} gives facet {}; to give that client a bigger map, patch \
             its size table with {PATCHER_TOOL}",
            hints.size_is,
            size.width,
            size.height,
            table.width,
            table.height,
            client.as_ref().expect("a table size has a client").exe.display(),
            request.target_facet
        ),
        (Some(size), _) | (None, Some(size)) => Some(size),
        (None, None) => None,
    };
    Ok((client, size))
}

/// Era checks: Trammel read from Felucca's files, applied difs.
fn check_target_client(client: &TargetClient, facet: u8, report: Report<'_>) {
    if facet == 1 && client.trammel_reads_map0() {
        report(format!("warning: {} builds Trammel from map0 and the facet-1 difs; it does not read map1 files.", client.exe.display()));
    }
    if let Some(difs) = client.difs() {
        for (kind, applies) in [("map", difs.land.applies()), ("static", difs.statics.applies())] {
            if applies == Some(true) {
                report(format!(
                    "note: {} applies its {kind} difs for facet {facet} on top of the files written when the server \
                     sends their counts.",
                    client.exe.display()
                ));
            }
        }
    }
}

fn prepare_output_dir(output_dir: &Path, inputs: &[PathBuf]) -> eyre::Result<()> {
    std::fs::create_dir_all(output_dir).wrap_err_with(|| format!("create {}", output_dir.display()))?;
    check_output_dir(output_dir, inputs)
}

/// Fails when `output_dir` is one of `inputs` or holds one of them.
pub fn check_output_dir(output_dir: &Path, inputs: &[PathBuf]) -> eyre::Result<()> {
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
pub fn write_classic(world: &ClassicWorld, facet: u32, edition: MapEdition, containers: &[ClassicContainer], template: Option<UopPackage>, output_dir: &Path, report: Report<'_>) -> eyre::Result<()> {
    write_classic_files(world, facet, edition, containers, template, None, output_dir, report)
}

/// [`write_classic`], with the files of a source whose world is what they hold copied instead of written.
fn write_classic_files(world: &ClassicWorld, facet: u32, edition: MapEdition, containers: &[ClassicContainer], template: Option<UopPackage>, unchanged: Option<&ClassicFiles>, output_dir: &Path, report: Report<'_>) -> eyre::Result<()> {
    let x = edition.file_suffix();
    let mut template = template;
    let unchanged_map = |format: ClassicMapFormat| unchanged.filter(|files| files.map_format == format);
    for container in containers {
        match container {
            ClassicContainer::Mul => {
                let path = output_dir.join(format!("map{facet}{x}.mul"));
                match unchanged_map(ClassicMapFormat::Mul) {
                    Some(files) => copy_as_it_is(&files.map, &path, report)?,
                    None => {
                        world.map.write_mul(&path)?;
                        report(format!("Wrote {}.", path.display()));
                    }
                }
            }
            ClassicContainer::Uop => {
                let path = output_dir.join(format!("map{facet}{x}LegacyMUL.uop"));
                match unchanged_map(ClassicMapFormat::Uop) {
                    Some(files) => copy_as_it_is(&files.map, &path, report)?,
                    None => {
                        let written = world.map.write_uop(&path, facet, template.take())?;
                        report(format!(
                            "Wrote {} ({} entries written, {} unchanged, {} removed).",
                            path.display(),
                            written.written,
                            written.unchanged,
                            written.removed
                        ));
                    }
                }
            }
        }
    }
    let (index, data) = (output_dir.join(format!("staidx{facet}{x}.mul")), output_dir.join(format!("statics{facet}{x}.mul")));
    if let Some(files) = unchanged {
        copy_as_it_is(&files.index, &index, report)?;
        return copy_as_it_is(&files.data, &data, report);
    }
    world.statics.write_mul(&index, &data)?;
    let size = world.size_cells();
    report(format!(
        "Wrote {} and {} ({}x{}, {} statics).",
        index.display(),
        data.display(),
        size.width,
        size.height,
        world.statics.static_count()
    ));
    Ok(())
}

fn copy_as_it_is(source: &Path, output: &Path, report: Report<'_>) -> eyre::Result<()> {
    write_through_temp(output, |temp| {
        std::fs::copy(source, temp).map(|_| ()).wrap_err_with(|| format!("copy {} to {}", source.display(), output.display()))
    })?;
    report(format!("Wrote {}: the source file as it is.", output.display()));
    Ok(())
}

/// `Wrote ...` for an image updated tile by tile.
pub fn describe_image_update(path: &Path, update: &ImageUpdate) -> String {
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
    let sources = ClassicFacetSources::resolve(&[dir.to_path_buf()], facet, &SOURCE_MAP_FORMATS, ClassicDifOptions::NONE, edition)?;
    Ok(ClassicWorld::load(&sources, Some(size))?.0)
}

/// The `facetNN.mul` of a target client that draws it.
fn write_facet_image(request: &FacetConversion, client: Option<&TargetClient>, world: &ClassicWorld, edition: MapEdition, hints: &ConvertHints, report: Report<'_>) -> eyre::Result<()> {
    let facet = u32::from(request.target_facet);
    let name = facet_image_file_name(facet);
    let Some(client) = client.filter(|client| client.reads_facet_image(facet)) else {
        if request.facet_image.is_some() {
            let why = match client {
                Some(client) => format!("{} does not draw {name}", client.exe.display()),
                None => hints.image_needs_client.to_owned(),
            };
            report(format!("note: {} ignored: {why}.", hints.image_choice));
        }
        return Ok(());
    };
    let mode = request.facet_image.unwrap_or(FacetImageMode::Update);
    let current_image = find_file_ignoring_case(&client.dir, &name).and_then(|path| match FacetImageFile::load(&path) {
        Ok(image) => Some(image),
        Err(error) => {
            report(format!("warning: {error:#}: the target's {name} is treated as missing."));
            None
        }
    });
    let size = world.size_cells();
    let fits = current_image.as_ref().is_some_and(|image| image.check_size(size.width, size.height).is_ok());
    if edition.is_variant() && mode == FacetImageMode::Update && fits {
        report(format!(
            "note: {name} not written: the shipped images follow the standard edition, not the {edition} one; {} to \
             render it from this one.",
            hints.image_regenerate
        ));
        return Ok(());
    }
    let current_world = if mode == FacetImageMode::Update && fits {
        match load_target_world(&client.dir, facet, edition, size) {
            Ok(current) => Some(current),
            Err(error) => {
                report(format!("warning: the target's map could not be read to compare with: {error:#}"));
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
    let path = request.output_dir.join(&name);
    match plan {
        FacetImagePlan::Unchanged(image) => {
            image.save(&path)?;
            let why = if mode == FacetImageMode::Keep { hints.image_kept } else { "the map is the target's" };
            report(format!("Wrote {}: the target's image, unchanged ({why}).", path.display()));
        }
        FacetImagePlan::Updated(image, update) => {
            image.save(&path)?;
            report(describe_image_update(&path, &update));
        }
        FacetImagePlan::Rendered(image, reason) => {
            image.save(&path)?;
            report(format!(
                "Wrote {} ({}x{}), rendered whole from the map by the slope rule ({}): an approximation, about \
                 {SHIPPED_STYLE_MATCH_PERCENT} % of pixels match the shipped style.",
                path.display(),
                image.width(),
                image.height(),
                reason.describe()
            ));
        }
        FacetImagePlan::NotWritten => {
            let why = if mode == FacetImageMode::None { hints.image_refused } else { "the target has none to keep" };
            report(format!("No {name} written ({why})."));
        }
    }
    Ok(())
}

/// Writes the Enhanced Client or Kingdom Reborn package, and its radar image with a radarcol.mul given or in `dirs`.
fn write_package(world: &ClassicWorld, request: &FacetConversion, edition: MapEdition, dirs: &[PathBuf], hints: &ConvertHints, report: Report<'_>) -> eyre::Result<()> {
    let facet = request.target_facet;
    let radarcol = request.radarcol.clone().or_else(|| dirs.iter().find_map(|dir| find_file_ignoring_case(dir, RADARCOL_NAME)));
    let palette = radarcol.as_deref().map(RadarPalette::load_radarcol).transpose()?;
    let (mut package, radar) = if request.target == ConversionTarget::KingdomReborn {
        build_kr_facet_from_world(world, facet, palette.as_ref(), &request.kr_land)?
    } else {
        build_facet_from_world(world, facet, palette.as_ref())?
    };
    let path = request.output_dir.join(facet_package_file_name(facet, edition.is_variant()));
    package.save(&path)?;
    report(format!("Wrote {}.", path.display()));
    match radar {
        Some(radar) => {
            let path = request.output_dir.join(edition_radar_dds_file_name(facet, edition));
            radar.save(&path)?;
            report(format!("Wrote {} (colours from {}).", path.display(), radarcol.expect("a palette").display()));
        }
        None => report(format!("No radar image written: no {RADARCOL_NAME} {}.", hints.no_radarcol)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::facet_sources::statics_file_names;
    use crate::utils::output_file::block_temp_file;

    const FACET: u8 = 4;
    const HEIGHT: u32 = 1448;
    const WIDTH: u32 = 8;
    const HINTS: ConvertHints = ConvertHints {
        standard_edition: "",
        target_is: "",
        size_is: "",
        crop: "",
        allow_lossy: "",
        accepted_losses: "",
        bake_difs: "",
        choose_container: "",
        image_choice: "",
        image_needs_client: "",
        image_classic_only: "",
        image_kept: "",
        image_refused: "",
        image_regenerate: "",
        no_radarcol: "",
    };

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uocf_facet_convert_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A folder holding a narrow Classic facet.
    fn classic_source(dir: &Path) {
        let world = ClassicWorld::filled(MapSizeCells { width: WIDTH, height: HEIGHT }, LandFill::VOID).unwrap();
        world.map.write_mul(&dir.join(format!("map{FACET}.mul"))).unwrap();
        let [index, data] = statics_file_names(u32::from(FACET), MapEdition::Standard);
        world.statics.write_mul(&dir.join(index), &dir.join(data)).unwrap();
    }

    fn request(source: &Path, target: ConversionTarget, output_dir: &Path) -> FacetConversion {
        FacetConversion {
            source: ConversionSource::ClassicFolder { dir: source.to_path_buf(), edition: MapEdition::Standard },
            facet: FACET,
            difs: SourceDifs::Ignore,
            target,
            target_facet: FACET,
            target_edition: None,
            target_client: None,
            crop: false,
            uop_template: None,
            target_tiledata: None,
            allow_lossy: false,
            radarcol: None,
            kr_land: KrLandMap::shipped(),
            facet_image: None,
            keep_unchanged: false,
            output_dir: output_dir.to_path_buf(),
        }
    }

    fn values() -> ParsedValues {
        ParsedValues { source_size: Ok(None), size: Ok(None), fill: Ok(LandFill::VOID) }
    }

    fn convert(request: &FacetConversion) -> eyre::Result<Vec<String>> {
        let mut lines = Vec::new();
        convert_facet(request, values(), &HINTS, &mut |line| lines.push(line))?;
        Ok(lines)
    }

    #[test]
    fn a_save_without_changes_copies_only_when_asked_to_and_a_failed_copy_leaves_the_file() {
        const EXISTING: &[u8] = b"existing";
        /// The third field of a static index entry, and a value the writer does not give it.
        const THIRD_FIELD: std::ops::Range<usize> = 8..12;
        const OTHER_THIRD_FIELD: u32 = 0x1234_5678;
        let root = temp("keep_unchanged");
        let (source, rewritten, copied) = (root.join("source"), root.join("rewritten"), root.join("copied"));
        std::fs::create_dir_all(&source).unwrap();
        classic_source(&source);
        let [index, _] = statics_file_names(u32::from(FACET), MapEdition::Standard);
        // An index entry with a third field the writer does not produce: rewriting changes the file, copying does not.
        let mut bytes = std::fs::read(source.join(&index)).unwrap();
        bytes[THIRD_FIELD].copy_from_slice(&OTHER_THIRD_FIELD.to_le_bytes());
        std::fs::write(source.join(&index), &bytes).unwrap();

        let target = ConversionTarget::Classic(ClassicContainers::Mul);
        convert(&request(&source, target, &rewritten)).unwrap();
        assert!(std::fs::read(rewritten.join(&index)).unwrap() != bytes);
        let keeping = FacetConversion { keep_unchanged: true, ..request(&source, target, &copied) };
        let lines = convert(&keeping).unwrap();
        assert!(lines.iter().any(|line| line.contains("the source file as it is")), "{lines:?}");
        assert!(std::fs::read(copied.join(&index)).unwrap() == bytes);

        // A failed copy leaves what the folder held.
        std::fs::write(copied.join(&index), EXISTING).unwrap();
        block_temp_file(&copied.join(&index));
        assert!(convert(&keeping).is_err());
        assert_eq!(std::fs::read(copied.join(&index)).unwrap(), EXISTING);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_value_the_caller_could_not_read_fails_only_when_it_is_needed() {
        let root = temp("late_values");
        let (source, output) = (root.join("source"), root.join("output"));
        std::fs::create_dir_all(&source).unwrap();
        classic_source(&source);
        let target = ConversionTarget::Classic(ClassicContainers::Mul);
        let conversion = request(&source, target, &output);
        let run = |values| convert_facet(&conversion, values, &HINTS, &mut |_| {});
        // No resize: the fill is not needed.
        run(ParsedValues { fill: Err(eyre!("no fill")), ..values() }).unwrap();
        let grown = Ok(Some(MapSizeCells { width: WIDTH * 2, height: HEIGHT }));
        let error = run(ParsedValues { size: grown, fill: Err(eyre!("no fill")), ..values() }).unwrap_err();
        assert_eq!(format!("{error:#}"), "no fill");
        let error = run(ParsedValues { size: Err(eyre!("no size")), ..values() }).unwrap_err();
        assert_eq!(format!("{error:#}"), "no size");
        let error = run(ParsedValues { source_size: Err(eyre!("no source size")), ..values() }).unwrap_err();
        assert_eq!(format!("{error:#}"), "no source size");
        std::fs::remove_dir_all(root).unwrap();
    }
}
