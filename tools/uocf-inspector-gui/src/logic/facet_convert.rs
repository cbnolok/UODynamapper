//! Whole-facet conversion behind the Maps tab: any of Classic MUL, Classic UOP, Enhanced Client facet and Kingdom
//! Reborn facet to any other, with a resize to another client's facet size, on the model of `uocf-cli facet convert`.
//!
//! A save without changes is the source as it is: a Classic file to its own container, facet, edition and size is copied,
//! and so is a facet package to its own kind. Rewriting a Classic file through the encoders would not always give its
//! bytes: in the old clients' files a map block's header is not its index, and a static index entry's third field varies.
//! A Classic UOP written over the same facet with a change keeps the source's layout and its unchanged entries.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use uocf::classic::facet_image::{facet_image_file_name, FacetImageFile};
use uocf::classic::facet_image_render::{
    plan_facet_image, FacetImageMode, FacetImagePlan, FacetImageRenderer, SHIPPED_STYLE_MATCH_PERCENT,
};
use uocf::classic::facet_sources::{ClassicDifOptions, ClassicFacetSources, ClassicMapFormat, MapDifFiles, StaticDifFiles};
use uocf::classic::map::MapSizeCells;
use uocf::classic::target_client::TargetClient;
use uocf::classic::world::{ClassicWorld, LandFill};
use uocf::client_exe::facet_dimensions::ClientFamily;
use uocf::enhanced::facet_encoder::build_facet_from_world;
use uocf::enhanced::facet_package::{detect_facet_dialect, EcFacetPackage};
use uocf::enhanced::facet_radar::{edition_radar_dds_file_name, RadarPalette};
use uocf::enhanced::facet_sector::{data_map_entry_path, facet_package_file_name, FacetGeometry};
use uocf::enhanced::facet_to_classic::facet_to_classic;
use uocf::kr::facet::facet_encoder::build_kr_facet_from_world;
use uocf::kr::facet::kr_land_map::KrLandMap;
use uocf::map_edition::MapEdition;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;
use uocf::utils::output_file::write_through_temp;
use uocf::utils::path::find_file_ignoring_case;

/// The map containers a Classic source folder is searched for, in order.
const SOURCE_MAP_FORMATS: [ClassicMapFormat; 2] = [ClassicMapFormat::Uop, ClassicMapFormat::Mul];
const RADARCOL_NAME: &str = "radarcol.mul";
/// The tool that changes a client's facet size table.
const PATCHER_TOOL: &str = "uo-client-patcher-gui";
const PACKAGE_NAME_PREFIX: &str = "facet";

/// What to do with the dif files of a Classic source.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DifChoice {
    /// Bake the kinds the source client applies, told by its executable; nothing when that is unknown.
    #[default]
    AsTheClientApplies,
    /// Bake every dif file found.
    Bake,
    Ignore,
}

impl DifChoice {
    pub const ALL: [Self; 3] = [Self::AsTheClientApplies, Self::Bake, Self::Ignore];

    pub fn label(self) -> &'static str {
        match self {
            Self::AsTheClientApplies => "As its client applies them",
            Self::Bake => "Bake them all",
            Self::Ignore => "Ignore them",
        }
    }
}

#[derive(Clone, Debug)]
pub enum FacetSource {
    Classic { dir: PathBuf, edition: MapEdition, difs: DifChoice },
    /// An Enhanced Client or Kingdom Reborn `facetN.uop`, told apart by its contents.
    Package(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FacetTarget {
    ClassicMul,
    #[default]
    ClassicUop,
    ClassicBoth,
    Enhanced,
    KingdomReborn,
}

impl FacetTarget {
    pub const ALL: [Self; 5] = [Self::ClassicMul, Self::ClassicUop, Self::ClassicBoth, Self::Enhanced, Self::KingdomReborn];

    pub fn label(self) -> &'static str {
        match self {
            Self::ClassicMul => "Classic mapN.mul",
            Self::ClassicUop => "Classic mapNLegacyMUL.uop",
            Self::ClassicBoth => "Classic MUL and UOP",
            Self::Enhanced => "Enhanced Client facetN.uop",
            Self::KingdomReborn => "Kingdom Reborn facetN.uop",
        }
    }

    pub fn is_classic(self) -> bool {
        matches!(self, Self::ClassicMul | Self::ClassicUop | Self::ClassicBoth)
    }

    fn family(self) -> ClientFamily {
        match self {
            Self::ClassicMul | Self::ClassicUop | Self::ClassicBoth => ClientFamily::Classic,
            Self::Enhanced => ClientFamily::Enhanced,
            Self::KingdomReborn => ClientFamily::KingdomReborn,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConvertRequest {
    pub source: FacetSource,
    pub facet: u8,
    pub target: FacetTarget,
    pub target_facet: u8,
    pub target_edition: MapEdition,
    /// The size written; `None` for the target client's size table, else the source's size.
    pub size: Option<MapSizeCells>,
    /// A client folder or executable written for: its size table, its tiledata for the id check, and the world map
    /// image of a client that draws one.
    pub target_client: Option<PathBuf>,
    /// Lets a smaller size drop the content outside it.
    pub crop: bool,
    /// What a larger size adds.
    pub fill: LandFill,
    /// Accepts what the Classic files cannot hold of a facet package.
    pub allow_lossy: bool,
    /// Kingdom Reborn: the Classic land id whose material and subtype to store for the ids its land map lacks, such as
    /// the land tiles added after its release. `None` refuses those ids.
    pub kr_fallback_land: Option<u16>,
    pub facet_image: FacetImageMode,
    pub output_dir: PathBuf,
}

/// The facet and edition a package's file name tells.
pub fn package_name_facet(path: &Path) -> Option<(u8, MapEdition)> {
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    let rest = stem.strip_prefix(PACKAGE_NAME_PREFIX)?;
    let suffix = MapEdition::Variant.file_suffix();
    match rest.strip_suffix(suffix) {
        Some(digits) => Some((digits.parse().ok()?, MapEdition::Variant)),
        None => Some((rest.parse().ok()?, MapEdition::Standard)),
    }
}

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
    /// The package of a Classic UOP source, whose layout a UOP written over the same facet keeps.
    template: Option<UopPackage>,
    /// The files of a Classic source when the world is what they hold, without baked difs.
    files: Option<ClassicFiles>,
    /// Folders and files the outputs must not overwrite.
    inputs: Vec<PathBuf>,
    /// Folders a radarcol.mul may be found in.
    dirs: Vec<PathBuf>,
}

/// Converts and saves the facet; returns what was done, line by line.
pub fn convert_facet(request: &ConvertRequest) -> eyre::Result<Vec<String>> {
    if request.target == FacetTarget::KingdomReborn && request.target_edition.is_variant() {
        eyre::bail!("Kingdom Reborn has no variant facet packages: choose the standard edition");
    }
    let mut notes = Vec::new();
    let client = open_target_client(request)?;
    let size = requested_size(request, client.as_ref())?;
    let kr_land = kr_land_map(request.kr_fallback_land)?;
    let source = match &request.source {
        FacetSource::Package(path) => {
            let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let mut package = open_package(path, request.facet, &mut notes)?;
            let edition = package_name_facet(path).map_or(MapEdition::Standard, |(_, edition)| edition);
            if is_same_package(request, &package, edition, size) {
                std::fs::create_dir_all(&request.output_dir).wrap_err_with(|| format!("create {}", request.output_dir.display()))?;
                check_output_dir(&request.output_dir, &[path.clone(), dir])?;
                let output = request.output_dir.join(facet_package_file_name(request.target_facet, edition.is_variant()));
                package.save(&output)?;
                notes.push(format!("Wrote {}: the package as it is.", output.display()));
                return Ok(notes);
            }
            let world = package_world(path, &package, &kr_land, request.allow_lossy, &mut notes)?;
            Source { world, edition, template: None, files: None, inputs: vec![path.clone(), dir.clone()], dirs: vec![dir] }
        }
        FacetSource::Classic { dir, edition, difs } => load_classic(request, dir, *edition, *difs, &mut notes)?,
    };

    let resized = size.is_some_and(|size| size != source.world.size_cells());
    let world = resize(source.world, size, request, &mut notes)?;
    if let Some(client) = &client {
        check_target_client(client, &world, request.target_facet, &mut notes)?;
    }
    std::fs::create_dir_all(&request.output_dir).wrap_err_with(|| format!("create {}", request.output_dir.display()))?;
    check_output_dir(&request.output_dir, &source.inputs)?;
    if request.target.is_classic() {
        let same_names = request.target_facet == request.facet && request.target_edition == source.edition;
        let template = source.template.filter(|_| same_names);
        let files = source.files.as_ref().filter(|_| same_names && !resized);
        write_classic(&world, request, template, files, &mut notes)?;
        write_facet_image(request, client.as_ref(), &world, &mut notes)?;
    } else {
        let mut dirs = source.dirs;
        dirs.extend(client.as_ref().map(|client| client.dir.clone()));
        write_package(&world, request, &dirs, &kr_land, &mut notes)?;
    }
    Ok(notes)
}

fn kr_land_map(fallback: Option<u16>) -> eyre::Result<KrLandMap> {
    let land_map = KrLandMap::shipped();
    let Some(fallback) = fallback else {
        return Ok(land_map);
    };
    let land = land_map.get(fallback).ok_or_else(|| eyre::eyre!("land {fallback:#06X} is not in the Kingdom Reborn land map"))?;
    Ok(land_map.with_fallback(land))
}

fn open_target_client(request: &ConvertRequest) -> eyre::Result<Option<TargetClient>> {
    let client = request.target_client.as_deref().map(TargetClient::open).transpose()?;
    if let Some(client) = &client {
        let expected = request.target.family();
        if client.family() != expected {
            eyre::bail!("{} is a {} client, but the target is {} files", client.exe.display(), client.family().label(), expected.label());
        }
    }
    Ok(client)
}

/// The size asked for, when one is: the size table of the target client refuses another one.
fn requested_size(request: &ConvertRequest, client: Option<&TargetClient>) -> eyre::Result<Option<MapSizeCells>> {
    let table = client.map(|client| client.facet_size(u32::from(request.target_facet))).transpose()?;
    match (request.size, table) {
        (Some(size), Some(table)) if size != table => eyre::bail!(
            "{}x{} differs from the {}x{} the size table of {} gives facet {}; to give that client a bigger map, patch its size table with {PATCHER_TOOL}",
            size.width,
            size.height,
            table.width,
            table.height,
            client.expect("a table size has a client").exe.display(),
            request.target_facet
        ),
        (Some(size), _) | (None, Some(size)) => Ok(Some(size)),
        (None, None) => Ok(None),
    }
}

fn open_package(path: &Path, facet: u8, notes: &mut Vec<String>) -> eyre::Result<EcFacetPackage> {
    let load = || UopPackage::load(path).wrap_err_with(|| format!("load {}", path.display()));
    let loaded = load()?;
    let dialect = detect_facet_dialect(&loaded, facet).wrap_err_with(|| format!("read {}", path.display()))?;
    let package = EcFacetPackage::from_package_dialect(loaded, facet, None, dialect)?;
    if package.package().unpack_file_by_hash(hash_file_name_single(&data_map_entry_path(facet)))?.is_some() {
        return Ok(package);
    }
    // No data.map: the size comes from the client's size table, when its executable is next to it.
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    match TargetClient::open(&dir).and_then(|client| client.facet_size(u32::from(facet))) {
        Ok(size) => {
            notes.push(format!("{} has no data.map: {}x{} from the size table.", path.display(), size.width, size.height));
            EcFacetPackage::from_package_dialect(load()?, facet, Some(FacetGeometry::new(size.width, size.height)), dialect)
        }
        Err(error) => {
            notes.push(format!("{} has no data.map and no readable client executable is next to it ({error:#}): using the stock size of facet {facet}.", path.display()));
            Ok(package)
        }
    }
}

/// Whether the target is the package's own kind, facet, edition and size.
fn is_same_package(request: &ConvertRequest, package: &EcFacetPackage, edition: MapEdition, size: Option<MapSizeCells>) -> bool {
    let family = if package.dialect().is_kingdom_reborn() { ClientFamily::KingdomReborn } else { ClientFamily::Enhanced };
    let geometry = package.geometry();
    request.target.family() == family
        && request.target_facet == request.facet
        && request.target_edition == edition
        && size.is_none_or(|size| size == MapSizeCells { width: geometry.width, height: geometry.height })
}

fn package_world(path: &Path, package: &EcFacetPackage, kr_land: &KrLandMap, allow_lossy: bool, notes: &mut Vec<String>) -> eyre::Result<ClassicWorld> {
    let converted = facet_to_classic(package, Some(kr_land))?;
    if let Some(note) = converted.losses.rebuilt_delimiters() {
        notes.push(format!("{}: {note}", path.display()));
    }
    let losses = converted.losses.real_losses().describe();
    if !losses.is_empty() {
        let list = losses.join("\n");
        if !allow_lossy {
            eyre::bail!("{} does not convert without losses (tick Accept losses to accept them):\n{list}", path.display());
        }
        notes.push(format!("Accepted losses:\n{list}"));
    }
    Ok(converted.world)
}

/// The dif kinds to bake from `dir`, and a note when the choice could not be told.
fn dif_options(choice: DifChoice, dir: &Path, facet: u32, notes: &mut Vec<String>) -> ClassicDifOptions {
    match choice {
        DifChoice::Bake => ClassicDifOptions::ALL,
        DifChoice::Ignore => ClassicDifOptions::NONE,
        DifChoice::AsTheClientApplies => {
            let dirs = [dir.to_path_buf()];
            if MapDifFiles::find(&dirs, facet).is_err() && StaticDifFiles::find(&dirs, facet).is_err() {
                return ClassicDifOptions::NONE;
            }
            match TargetClient::open(dir).ok().and_then(|client| client.difs().cloned()) {
                Some(usage) => {
                    notes.push(format!("Source difs: land {}, statics {} (from the client of {}).", usage.land.describe(), usage.statics.describe(), dir.display()));
                    ClassicDifOptions { map_difs: usage.land.applies() == Some(true), static_difs: usage.statics.applies() == Some(true) }
                }
                None => {
                    notes.push(format!("{} has dif files for facet {facet}, but whether its client applies them is unknown (no readable executable): not baked.", dir.display()));
                    ClassicDifOptions::NONE
                }
            }
        }
    }
}

fn load_classic(request: &ConvertRequest, dir: &Path, edition: MapEdition, difs: DifChoice, notes: &mut Vec<String>) -> eyre::Result<Source> {
    let facet = u32::from(request.facet);
    let difs = dif_options(difs, dir, facet, notes);
    let sources = ClassicFacetSources::resolve(&[dir.to_path_buf()], facet, &SOURCE_MAP_FORMATS, difs, edition)?;
    let size = TargetClient::open(dir).ok().filter(|client| client.family() == ClientFamily::Classic).and_then(|client| client.facet_size(facet).ok());
    let (world, map_bake, static_bake) = ClassicWorld::load(&sources, size)?;
    let baked = map_bake.is_some() || static_bake.is_some();
    let read = world.size_cells();
    notes.push(format!("Read Classic facet {facet} ({} edition, {}x{}) from {}.", sources.edition, read.width, read.height, sources.map.path.display()));
    notes.extend(sources.base_facet_warning());
    for (kind, bake) in [("map", map_bake), ("static", static_bake)] {
        if let Some(bake) = bake {
            notes.push(format!("Baked the {kind} difs: {} listed, {} blocks patched (a later id wins), {} outside the map.", bake.listed, bake.distinct, bake.outside));
        }
    }
    let unsorted = world.unsorted_static_blocks();
    if unsorted > 0 && !request.target.is_classic() {
        notes.push(format!("{unsorted} static blocks are not in (y, x) order; a facet keeps each cell's statics but not their order across cells, so converting back to Classic sorts them."));
    }
    let template = match sources.map.format {
        ClassicMapFormat::Uop => Some(UopPackage::load(&sources.map.path).wrap_err_with(|| format!("load {}", sources.map.path.display()))?),
        ClassicMapFormat::Mul => None,
    };
    let mut inputs = vec![sources.map.path.clone(), dir.to_path_buf()];
    let mut files = None;
    if let Ok(statics) = sources.statics_files() {
        inputs.extend([statics.index.clone(), statics.statics.clone()]);
        if !baked {
            files = Some(ClassicFiles { map: sources.map.path.clone(), map_format: sources.map.format, index: statics.index.clone(), data: statics.statics.clone() });
        }
    }
    Ok(Source { world, edition: sources.edition, template, files, inputs, dirs: vec![dir.to_path_buf()] })
}

fn resize(world: ClassicWorld, size: Option<MapSizeCells>, request: &ConvertRequest, notes: &mut Vec<String>) -> eyre::Result<ClassicWorld> {
    let from = world.size_cells();
    let Some(size) = size.filter(|size| *size != from) else {
        return Ok(world);
    };
    let (resized, report) = world.resized(size, request.fill, request.crop).wrap_err_with(|| {
        format!("the target is smaller; tick Crop to drop that content, or give the target client a bigger map with {PATCHER_TOOL}")
    })?;
    notes.push(format!("Resized {}x{} to {}x{}.", from.width, from.height, size.width, size.height));
    notes.extend(report.describe());
    Ok(resized)
}

/// Era checks: Trammel read from Felucca's files, applied difs, tile ids past the target's tiledata.
fn check_target_client(client: &TargetClient, world: &ClassicWorld, facet: u8, notes: &mut Vec<String>) -> eyre::Result<()> {
    if facet == 1 && client.trammel_reads_map0() {
        notes.push(format!("{} builds Trammel from map0 and the facet-1 difs; it does not read map1 files.", client.exe.display()));
    }
    if let Some(difs) = client.difs() {
        for (kind, applies) in [("map", difs.land.applies()), ("static", difs.statics.applies())] {
            if applies == Some(true) {
                notes.push(format!("{} applies its {kind} difs for facet {facet} on top of the files written when the server sends their counts.", client.exe.display()));
            }
        }
    }
    if let Some(counts) = client.tiledata_counts()? {
        let report = world.tile_ids_beyond(counts.land, counts.items);
        if !report.is_empty() {
            notes.push(format!(
                "The target tiledata.mul has {} land and {} item tiles: {} land cells (up to {:?}) and {} statics (up to {:?}) use ids it does not have.",
                counts.land, counts.items, report.land_cells, report.max_land, report.statics, report.max_graphic
            ));
        }
    }
    Ok(())
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

/// Writes the map and the statics under the file names of the target facet and edition.
fn write_classic(world: &ClassicWorld, request: &ConvertRequest, template: Option<UopPackage>, files: Option<&ClassicFiles>, notes: &mut Vec<String>) -> eyre::Result<()> {
    let (facet, dir) = (u32::from(request.target_facet), &request.output_dir);
    let x = request.target_edition.file_suffix();
    let unchanged = |format: ClassicMapFormat| files.filter(|files| files.map_format == format);
    if matches!(request.target, FacetTarget::ClassicMul | FacetTarget::ClassicBoth) {
        let path = dir.join(format!("map{facet}{x}.mul"));
        match unchanged(ClassicMapFormat::Mul) {
            Some(files) => copy_as_it_is(&files.map, &path, notes)?,
            None => {
                world.map.write_mul(&path)?;
                notes.push(format!("Wrote {}.", path.display()));
            }
        }
    }
    if matches!(request.target, FacetTarget::ClassicUop | FacetTarget::ClassicBoth) {
        let path = dir.join(format!("map{facet}{x}LegacyMUL.uop"));
        match unchanged(ClassicMapFormat::Uop) {
            Some(files) => copy_as_it_is(&files.map, &path, notes)?,
            None => {
                let report = world.map.write_uop(&path, facet, template)?;
                notes.push(format!("Wrote {} ({} entries written, {} unchanged, {} removed).", path.display(), report.written, report.unchanged, report.removed));
            }
        }
    }
    let (index, data) = (dir.join(format!("staidx{facet}{x}.mul")), dir.join(format!("statics{facet}{x}.mul")));
    if let Some(files) = files {
        copy_as_it_is(&files.index, &index, notes)?;
        copy_as_it_is(&files.data, &data, notes)?;
        return Ok(());
    }
    world.statics.write_mul(&index, &data)?;
    let size = world.size_cells();
    notes.push(format!("Wrote {} and {} ({}x{}, {} statics).", index.display(), data.display(), size.width, size.height, world.statics.static_count()));
    Ok(())
}

fn copy_as_it_is(source: &Path, output: &Path, notes: &mut Vec<String>) -> eyre::Result<()> {
    write_through_temp(output, |temp| {
        std::fs::copy(source, temp).map(|_| ()).wrap_err_with(|| format!("copy {} to {}", source.display(), output.display()))
    })?;
    notes.push(format!("Wrote {}: the source file as it is.", output.display()));
    Ok(())
}

/// The target's facet as it is, to compare the converted one with.
fn load_target_world(dir: &Path, facet: u32, edition: MapEdition, size: MapSizeCells) -> eyre::Result<ClassicWorld> {
    let sources = ClassicFacetSources::resolve(&[dir.to_path_buf()], facet, &SOURCE_MAP_FORMATS, ClassicDifOptions::NONE, edition)?;
    Ok(ClassicWorld::load(&sources, Some(size))?.0)
}

/// The `facetNN.mul` of a target client that draws it: the 7.0.x clients read it without bounds checks, tile (x, y)
/// at column x of row y, so an image of another size draws the wrong tiles.
fn write_facet_image(request: &ConvertRequest, client: Option<&TargetClient>, world: &ClassicWorld, notes: &mut Vec<String>) -> eyre::Result<()> {
    let facet = u32::from(request.target_facet);
    let Some(client) = client.filter(|client| client.reads_facet_image(facet)) else {
        return Ok(());
    };
    let (mode, edition) = (request.facet_image, request.target_edition);
    let name = facet_image_file_name(facet);
    let current_image = find_file_ignoring_case(&client.dir, &name).and_then(|path| match FacetImageFile::load(&path) {
        Ok(image) => Some(image),
        Err(error) => {
            notes.push(format!("{error:#}: the target's {name} is treated as missing."));
            None
        }
    });
    let size = world.size_cells();
    let fits = current_image.as_ref().is_some_and(|image| image.check_size(size.width, size.height).is_ok());
    if edition.is_variant() && mode == FacetImageMode::Update && fits {
        notes.push(format!("{name} not written: the shipped images follow the standard edition, not the {edition} one; choose Regenerate to render it from this one."));
        return Ok(());
    }
    let current_world = if mode == FacetImageMode::Update && fits {
        load_target_world(&client.dir, facet, edition, size)
            .inspect_err(|error| notes.push(format!("The target's map could not be read to compare with: {error:#}")))
            .ok()
    } else {
        None
    };
    let renderer = FacetImageRenderer::load(&client.dir);
    let plan = plan_facet_image(mode, world, current_image.as_ref(), current_world.as_ref(), renderer.as_ref().ok()).map_err(|error| match &renderer {
        Err(load) => eyre::eyre!("{name}: {error:#} ({load:#})"),
        Ok(_) => error.wrap_err(name.clone()),
    })?;
    let path = request.output_dir.join(&name);
    match plan {
        FacetImagePlan::Unchanged(image) => {
            image.save(&path)?;
            notes.push(format!("Wrote {}: the target's image, unchanged.", path.display()));
        }
        FacetImagePlan::Updated(image, update) => {
            image.save(&path)?;
            notes.push(format!(
                "Wrote {}: {} tiles re-rendered by the slope rule, {} pixels changed in {} rows ({} bit-15 pixels of unchanged tiles kept).",
                path.display(),
                update.tiles_rendered,
                update.pixels_changed,
                update.rows_changed,
                update.undrawn_kept
            ));
        }
        FacetImagePlan::Rendered(image, reason) => {
            image.save(&path)?;
            notes.push(format!(
                "Wrote {} ({}x{}), rendered whole from the map by the slope rule ({}): an approximation, about {SHIPPED_STYLE_MATCH_PERCENT} % of pixels match the shipped style.",
                path.display(),
                image.width(),
                image.height(),
                reason.describe()
            ));
        }
        FacetImagePlan::NotWritten => notes.push(format!("No {name} written.")),
    }
    Ok(())
}

/// Writes the Enhanced Client or Kingdom Reborn package, and its radar image with a radarcol.mul in `dirs`.
fn write_package(world: &ClassicWorld, request: &ConvertRequest, dirs: &[PathBuf], kr_land: &KrLandMap, notes: &mut Vec<String>) -> eyre::Result<()> {
    let (facet, edition) = (request.target_facet, request.target_edition);
    let radarcol = dirs.iter().find_map(|dir| find_file_ignoring_case(dir, RADARCOL_NAME));
    let palette = radarcol.as_deref().map(RadarPalette::load_radarcol).transpose()?;
    let (mut package, radar) = if request.target == FacetTarget::KingdomReborn {
        build_kr_facet_from_world(world, facet, palette.as_ref(), kr_land)?
    } else {
        build_facet_from_world(world, facet, palette.as_ref())?
    };
    let path = request.output_dir.join(facet_package_file_name(facet, edition.is_variant()));
    package.save(&path)?;
    notes.push(format!("Wrote {}.", path.display()));
    match (radar, radarcol) {
        (Some(radar), Some(radarcol)) => {
            let path = request.output_dir.join(edition_radar_dds_file_name(facet, edition));
            radar.save(&path)?;
            notes.push(format!("Wrote {} (colours from {}).", path.display(), radarcol.display()));
        }
        _ => notes.push(format!("No radar image written: no {RADARCOL_NAME} in the source or target client's folder.")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::{client_roots, output_path, CLASSIC_CLIENT_ROOTS_ENV, EC_CLIENT_ROOTS_ENV};
    use uocf::classic::facet_sources::statics_file_names;
    use uocf::classic::map_write::ClassicMapBlocks;
    use uocf::classic::statics_write::ClassicStaticsBlocks;

    /// The smallest shipped facet, Tokuno, whose height the map readers know; a narrow copy of it is quick to convert.
    const SMALL_FACET: u8 = 4;
    const SMALL_HEIGHT: u32 = 1448;
    const SMALL_WIDTH: u32 = 8;
    const GROWN_WIDTH: u32 = 16;
    const UNALIGNED_WIDTH: u32 = 12;
    const LAND: u16 = 3;
    const LAND_Z: i8 = 1;
    const FILL: LandFill = LandFill { land: 5, z: 2 };
    const FELUCCA: u8 = 0;
    const OLD_FELUCCA: MapSizeCells = MapSizeCells { width: 6144, height: 4096 };
    /// A cell of the ground a client ships, for the checks after a resize.
    const PROBE: (u32, u32) = (1000, 1000);
    const CURRENT_FELUCCA: MapSizeCells = MapSizeCells { width: 7168, height: 4096 };
    /// A land id of the Kingdom Reborn land map, for the land it has no material for.
    const KR_FALLBACK_LAND: u16 = 3;
    const NOT_KR_LAND: u16 = u16::MAX;

    fn request(source: FacetSource, target: FacetTarget, output_dir: PathBuf) -> ConvertRequest {
        ConvertRequest {
            source,
            facet: SMALL_FACET,
            target,
            target_facet: SMALL_FACET,
            target_edition: MapEdition::Standard,
            size: None,
            target_client: None,
            crop: false,
            fill: LandFill::VOID,
            allow_lossy: false,
            kr_fallback_land: None,
            facet_image: FacetImageMode::default(),
            output_dir,
        }
    }

    fn classic(dir: &Path) -> FacetSource {
        FacetSource::Classic { dir: dir.to_path_buf(), edition: MapEdition::Standard, difs: DifChoice::Ignore }
    }

    /// A folder holding a narrow Classic facet, and the folder to write to.
    fn small_client(name: &str) -> (PathBuf, PathBuf) {
        let dir = output_path(&format!("{name}-source"));
        let output = output_path(&format!("{name}-output"));
        std::fs::create_dir_all(&dir).unwrap();
        let world = ClassicWorld::filled(MapSizeCells { width: SMALL_WIDTH, height: SMALL_HEIGHT }, LandFill { land: LAND, z: LAND_Z }).unwrap();
        world.map.write_mul(&dir.join(format!("map{SMALL_FACET}.mul"))).unwrap();
        let [index, data] = statics_file_names(u32::from(SMALL_FACET), MapEdition::Standard);
        world.statics.write_mul(&dir.join(index), &dir.join(data)).unwrap();
        (dir, output)
    }

    fn clean(dirs: &[&Path]) {
        for dir in dirs {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_package_name_tells_its_facet_and_edition() {
        assert_eq!(package_name_facet(Path::new("/a/facet3.uop")), Some((3, MapEdition::Standard)));
        assert_eq!(package_name_facet(Path::new("/a/FACET1X.uop")), Some((1, MapEdition::Variant)));
        assert_eq!(package_name_facet(Path::new("/a/map1.uop")), None);
    }

    #[test]
    fn a_folder_holding_the_source_is_refused_as_the_output() {
        let (dir, output) = small_client("facet-refused");
        let target = request(classic(&dir), FacetTarget::ClassicMul, dir.clone());
        let before = std::fs::read(dir.join(format!("map{SMALL_FACET}.mul"))).unwrap();
        assert!(convert_facet(&target).is_err());
        assert!(std::fs::read(dir.join(format!("map{SMALL_FACET}.mul"))).unwrap() == before);
        clean(&[&dir, &output]);
    }

    #[test]
    fn a_larger_size_fills_with_the_chosen_land_and_comes_back() {
        let (dir, output) = small_client("facet-grow");
        let size = MapSizeCells { width: GROWN_WIDTH, height: SMALL_HEIGHT };
        let target = ConvertRequest { size: Some(size), fill: FILL, ..request(classic(&dir), FacetTarget::ClassicMul, output.clone()) };
        convert_facet(&target).unwrap();
        let map = ClassicMapBlocks::read_mul(&output.join(format!("map{SMALL_FACET}.mul")), u32::from(SMALL_FACET), None).unwrap();
        assert_eq!(map.size_cells(), size);
        assert_eq!(map.cell(0, 0).unwrap(), (LAND, LAND_Z));
        assert_eq!(map.cell(SMALL_WIDTH, 0).unwrap(), (FILL.land, FILL.z));
        clean(&[&dir, &output]);
    }

    #[test]
    fn a_smaller_size_needs_the_crop_and_a_side_must_be_whole_blocks() {
        let (dir, output) = small_client("facet-grown-source");
        let grown = output_path("facet-grown");
        convert_facet(&ConvertRequest {
            size: Some(MapSizeCells { width: GROWN_WIDTH, height: SMALL_HEIGHT }),
            fill: FILL,
            ..request(classic(&dir), FacetTarget::ClassicMul, grown.clone())
        })
        .unwrap();
        let small = MapSizeCells { width: SMALL_WIDTH, height: SMALL_HEIGHT };
        let shrink = ConvertRequest { size: Some(small), ..request(classic(&grown), FacetTarget::ClassicMul, output.clone()) };
        assert!(convert_facet(&shrink).is_err(), "the crop is refused unless asked for");
        convert_facet(&ConvertRequest { crop: true, ..shrink.clone() }).unwrap();
        let map = ClassicMapBlocks::read_mul(&output.join(format!("map{SMALL_FACET}.mul")), u32::from(SMALL_FACET), None).unwrap();
        assert_eq!(map.size_cells(), small);
        let unaligned = ConvertRequest { size: Some(MapSizeCells { width: UNALIGNED_WIDTH, height: SMALL_HEIGHT }), ..shrink };
        assert!(convert_facet(&unaligned).is_err());
        clean(&[&dir, &output, &grown]);
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn a_shipped_classic_facet_saves_the_same_bytes_and_converts_and_comes_back() {
        for root in client_roots(CLASSIC_CLIENT_ROOTS_ENV) {
            let name = root.file_name().unwrap().to_string_lossy().into_owned();
            let source = classic(&root);
            let uop_source = find_file_ignoring_case(&root, &format!("map{SMALL_FACET}LegacyMUL.uop"));
            let map_source = uop_source.clone().or_else(|| find_file_ignoring_case(&root, &format!("map{SMALL_FACET}.mul"))).unwrap();
            let output = output_path(&format!("facet-classic-{}", name.replace(' ', "-")));
            let target = if uop_source.is_some() { FacetTarget::ClassicUop } else { FacetTarget::ClassicMul };
            for note in convert_facet(&request(source.clone(), target, output.clone())).unwrap() {
                println!("{name}: {note}");
            }
            let [index, data] = statics_file_names(u32::from(SMALL_FACET), MapEdition::Standard);
            let map_name = map_source.file_name().unwrap().to_string_lossy().into_owned();
            for file in [&map_name, &index, &data] {
                let source_file = find_file_ignoring_case(&root, file).unwrap();
                assert!(std::fs::read(output.join(file)).unwrap() == std::fs::read(source_file).unwrap(), "{name}: {file} differs after a save without changes");
            }

            let original = ClassicWorld::load(
                &ClassicFacetSources::resolve(&[root.clone()], u32::from(SMALL_FACET), &SOURCE_MAP_FORMATS, ClassicDifOptions::NONE, MapEdition::Standard).unwrap(),
                None,
            )
            .unwrap()
            .0;
            let package_dir = output_path(&format!("facet-package-{}", name.replace(' ', "-")));
            let back_dir = output_path(&format!("facet-back-{}", name.replace(' ', "-")));
            convert_facet(&request(source.clone(), FacetTarget::Enhanced, package_dir.clone())).unwrap();
            let package = package_dir.join(facet_package_file_name(SMALL_FACET, false));
            convert_facet(&ConvertRequest { allow_lossy: true, ..request(FacetSource::Package(package), FacetTarget::ClassicMul, back_dir.clone()) }).unwrap();
            let back = ClassicMapBlocks::read_mul(&back_dir.join(format!("map{SMALL_FACET}.mul")), u32::from(SMALL_FACET), None).unwrap();
            assert!(back == original.map, "{name}: the land differs after Classic to Enhanced Client to Classic");
            let statics = ClassicStaticsBlocks::read_mul(&back_dir.join(&index), &back_dir.join(&data), back.size_cells()).unwrap();
            assert_eq!(statics.static_count(), original.statics.static_count(), "{name}: the statics after Classic to Enhanced Client to Classic");
            println!("{name}: same bytes; Classic to Enhanced Client to Classic keeps the land and the {} statics", statics.static_count());
            clean(&[&output, &package_dir, &back_dir]);
        }
    }

    #[test]
    #[ignore = "needs real clients: set UO_EC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn a_shipped_facet_package_saves_the_same_bytes_and_converts() {
        for root in client_roots(EC_CLIENT_ROOTS_ENV) {
            let name = root.file_name().unwrap().to_string_lossy().into_owned();
            let path = find_file_ignoring_case(&root, &facet_package_file_name(SMALL_FACET, false)).unwrap();
            let package = open_package(&path, SMALL_FACET, &mut Vec::new()).unwrap();
            let target = if package.dialect().is_kingdom_reborn() { FacetTarget::KingdomReborn } else { FacetTarget::Enhanced };
            let output = output_path(&format!("facet-same-{}", name.replace(' ', "-")));
            let notes = convert_facet(&request(FacetSource::Package(path.clone()), target, output.clone())).unwrap();
            assert!(notes.last().unwrap().contains("as it is"));
            assert!(std::fs::read(output.join(path.file_name().unwrap())).unwrap() == std::fs::read(&path).unwrap(), "{name}: differs after a save without changes");

            let classic_dir = output_path(&format!("facet-classic-out-{}", name.replace(' ', "-")));
            for note in convert_facet(&ConvertRequest { allow_lossy: true, ..request(FacetSource::Package(path), FacetTarget::ClassicMul, classic_dir.clone()) }).unwrap() {
                println!("{name}: {note}");
            }
            let map = ClassicMapBlocks::read_mul(&classic_dir.join(format!("map{SMALL_FACET}.mul")), u32::from(SMALL_FACET), None).unwrap();
            let geometry = package.geometry();
            assert_eq!(map.size_cells(), MapSizeCells { width: geometry.width, height: geometry.height }, "{name}");
            println!("{name}: same bytes; converts to Classic at {}x{}", geometry.width, geometry.height);
            clean(&[&output, &classic_dir]);
        }
    }

    #[test]
    #[ignore = "needs real clients and minutes: set UO_CLASSIC_CLIENT_ROOTS, UO_EC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn felucca_goes_to_the_old_client_sizes_as_classic_and_kingdom_reborn() {
        let classic_roots = client_roots(CLASSIC_CLIENT_ROOTS_ENV);
        let root = classic_roots.first().unwrap();
        let base = ConvertRequest { facet: FELUCCA, target_facet: FELUCCA, size: Some(OLD_FELUCCA), ..request(classic(root), FacetTarget::ClassicMul, PathBuf::new()) };
        let refused = output_path("facet-old-refused");
        assert!(convert_facet(&ConvertRequest { output_dir: refused.clone(), ..base.clone() }).is_err(), "the crop is refused unless asked for");

        let output = output_path("facet-old-classic");
        convert_facet(&ConvertRequest { crop: true, output_dir: output.clone(), ..base.clone() }).unwrap();
        let map = ClassicMapBlocks::read_mul(&output.join(format!("map{FELUCCA}.mul")), u32::from(FELUCCA), Some(OLD_FELUCCA)).unwrap();
        assert_eq!(map.size_cells(), OLD_FELUCCA);
        let original = ClassicMapBlocks::read_uop(&find_file_ignoring_case(root, &format!("map{FELUCCA}LegacyMUL.uop")).unwrap(), u32::from(FELUCCA), None).unwrap();
        assert_eq!(map.cell(PROBE.0, PROBE.1).unwrap(), original.cell(PROBE.0, PROBE.1).unwrap());
        println!("{}: Felucca cropped to {}x{} as Classic MUL", root.display(), OLD_FELUCCA.width, OLD_FELUCCA.height);

        let current = ConvertRequest { crop: true, target: FacetTarget::KingdomReborn, output_dir: output_path("facet-new-kr"), ..base.clone() };
        let refusal = convert_facet(&current).unwrap_err();
        println!("{}: without a fallback land: {refusal:#}", root.display());
        let unknown = ConvertRequest { kr_fallback_land: Some(NOT_KR_LAND), ..current.clone() };
        assert!(convert_facet(&unknown).is_err(), "a fallback the land map lacks");
        convert_facet(&ConvertRequest { kr_fallback_land: Some(KR_FALLBACK_LAND), ..current.clone() }).unwrap();
        let package = open_package(&current.output_dir.join(facet_package_file_name(FELUCCA, false)), FELUCCA, &mut Vec::new()).unwrap();
        assert!(package.dialect().is_kingdom_reborn());
        assert_eq!((package.geometry().width, package.geometry().height), (OLD_FELUCCA.width, OLD_FELUCCA.height));
        println!("{}: Felucca written as a Kingdom Reborn package of {}x{} with a fallback land", root.display(), OLD_FELUCCA.width, OLD_FELUCCA.height);

        let mut folders = vec![refused, output, current.output_dir];
        for root in classic_roots.iter().skip(1) {
            let kr = output_path("facet-gold-kr");
            let request = ConvertRequest { facet: FELUCCA, target_facet: FELUCCA, kr_fallback_land: Some(KR_FALLBACK_LAND), ..request(classic(root), FacetTarget::KingdomReborn, kr.clone()) };
            for note in convert_facet(&request).unwrap() {
                println!("{note}");
            }
            let package = open_package(&kr.join(facet_package_file_name(FELUCCA, false)), FELUCCA, &mut Vec::new()).unwrap();
            assert!(package.dialect().is_kingdom_reborn());
            println!("{}: Felucca written as a Kingdom Reborn package of {}x{}", root.display(), package.geometry().width, package.geometry().height);
            folders.push(kr);
        }

        for root in client_roots(EC_CLIENT_ROOTS_ENV) {
            let path = find_file_ignoring_case(&root, &facet_package_file_name(FELUCCA, false)).unwrap();
            let package = open_package(&path, FELUCCA, &mut Vec::new()).unwrap();
            if !package.dialect().is_kingdom_reborn() {
                continue;
            }
            let size = if package.geometry().width == OLD_FELUCCA.width { CURRENT_FELUCCA } else { OLD_FELUCCA };
            let kr = output_path("facet-kr-resized");
            let source = FacetSource::Package(path);
            let request = ConvertRequest { facet: FELUCCA, target_facet: FELUCCA, size: Some(size), crop: true, allow_lossy: true, ..request(source, FacetTarget::KingdomReborn, kr.clone()) };
            for note in convert_facet(&request).unwrap() {
                println!("{note}");
            }
            let package = open_package(&kr.join(facet_package_file_name(FELUCCA, false)), FELUCCA, &mut Vec::new()).unwrap();
            assert!(package.dialect().is_kingdom_reborn());
            assert_eq!((package.geometry().width, package.geometry().height), (size.width, size.height));
            println!("{}: Felucca written as a Kingdom Reborn package of {}x{}", root.display(), size.width, size.height);
            folders.push(kr);
        }
        for folder in &folders {
            clean(&[folder]);
        }
    }
}
