//! Build-time support for `tex_land_kr.uddp`, the Kingdom Reborn land.
//!
//! The package has the `tex_land_ec.uddp` layout and is built by the same packer
//! ([`crate::textures::tex_land_ec`]); only its inputs differ. Each input comes from the Kingdom Reborn client where
//! it has one and from the Enhanced Client where it does not:
//! - definitions: KR's `TerrainDefinition.uop` (alias flags moved to the EC layout), plus the EC materials KR lacks;
//! - textures: an id a KR definition uses from KR's `Texture.uop`, an id only EC definitions use from EC's; an id
//!   missing there comes from the other client;
//! - routing: KR has no `legacyterrainmap.csv`; its Classic land routes are the ones measured on its facets
//!   (`lib/uocf/assets/kr/mappings.bin`, [`KrLandMap::shipped_verified`]). They replace the EC routes of those ids in
//!   `metadata/legacy_terrain_map.csv` and leave `metadata/transcode.bin` (TerrainTranscode.kdl, which the runtime
//!   reads over the csv), which keeps only the ids KR does not route;
//! - KR's grunge texture, `build/worldart/01000045.dds`, as the optional entry
//!   [`UDDP_GRUNGE_ENTRY_VPATH`], which Dynamapper's Client Grunge multiplies onto the statics.
//!
//! Every definition, texture and route is logged with the client it came from, and written to the report when one
//! is asked for.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use log::{info, warn};
use udd_assets::eckr_terrain_kdl::EckrTerrainRouting;
use udd_assets::tex_land_ec::{encode_grunge_entry, UDDP_GRUNGE_ENTRY_VPATH};
use udd_container::DataType;
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::legacy_terrain_map::{LegacyTerrainMap, LegacyTerrainRoute};
use uocf::enhanced::terrain_definition::{
    detect_terrain_definition_dialect, TerrainDefinitionEntry, TerrainDefinitionPackage,
};
use uocf::enhanced::textures::Textures;
use uocf::kr::facet::kr_land_map::KrLandMap;
use uocf::uop_container::package::UopPackage;

use crate::package_progress::AssetTaskProgress;
use crate::source_paths::find_first_existing_file;
use crate::textures::tex_land_ec::{
    build_land_package, find_layer_texture_file, read_terrain_overrides_kdl, read_transcode_kdl,
    terrain_override_texture_ids, validate_options, ExtraLandEntry, LandPackageInputs, LandTexturePool,
    TexLandEcAtlasOptions, TexLandEcBuildSummary,
};

/// Default file name of the package.
pub const TEX_LAND_KR_FILE_NAME: &str = "tex_land_kr.uddp";
/// Texture id of the grunge texture, `build/worldart/01000045.dds` (256x256 grey noise; KR's differs from EC's).
pub const KR_GRUNGE_TEXTURE_ID: u32 = udd_assets::tilemeta::CLIENT_GRUNGE_TEXTURE_ID;

const TERRAIN_DEFINITION_FILE_NAME: &str = "TerrainDefinition.uop";
const TEXTURE_FILE_NAME: &str = "Texture.uop";
const GAME_DATA_FILE_NAME: &str = "GameData.uop";

pub struct TexLandKrPackOptions {
    /// Page layout, encoding, upscaling and the routing and overrides tables, as for the Enhanced land.
    pub atlas: TexLandEcAtlasOptions,
    /// Write every source decision of the packer to this file, one line each.
    pub report: Option<PathBuf>,
}

/// The client one input of the Kingdom Reborn land came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandSourceClient {
    KingdomReborn,
    Enhanced,
}

impl LandSourceClient {
    pub fn label(self) -> &'static str {
        match self {
            Self::KingdomReborn => ClientDialect::KingdomReborn.label(),
            Self::Enhanced => ClientDialect::Enhanced.label(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TexLandKrBuildSummary {
    pub package: TexLandEcBuildSummary,
    /// Definitions from Kingdom Reborn, and how many of them differ from the Enhanced Client's of the same id.
    pub kr_definition_count: u32,
    pub kr_definitions_differing_from_ec: u32,
    /// Definitions from the Enhanced Client: the materials Kingdom Reborn lacks.
    pub ec_definition_count: u32,
    pub kr_texture_count: u32,
    pub ec_texture_count: u32,
    /// Textures KR definitions use that KR's `Texture.uop` lacks, taken from the Enhanced Client.
    pub kr_textures_from_ec: Vec<u32>,
    /// Textures both clients' definitions use whose images differ; the Kingdom Reborn image is packed.
    pub conflicting_texture_ids: Vec<u32>,
    /// Classic land routes from Kingdom Reborn's facets, and how many of them change the Enhanced Client's route.
    pub kr_route_count: u32,
    pub kr_routes_changing_ec: u32,
    /// Classic land routes kept from the Enhanced Client's `legacyterrainmap.csv`.
    pub ec_route_count: u32,
    /// TerrainTranscode.kdl routes kept, and those dropped for ids Kingdom Reborn routes.
    pub transcode_route_count: u32,
    pub transcode_routes_dropped: u32,
    /// Width and height of the grunge texture, when Kingdom Reborn has one.
    pub grunge_size: Option<(u32, u32)>,
}

impl fmt::Display for TexLandKrBuildSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "definitions: {} Kingdom Reborn ({} differ from the Enhanced Client's), {} Enhanced Client; \
             textures: {} Kingdom Reborn, {} Enhanced Client ({} for KR definitions), {} conflicting; \
             routes: {} Kingdom Reborn ({} change the Enhanced Client's), {} Enhanced Client csv, {} TerrainTranscode.kdl \
             ({} dropped); grunge: {}; {} pages, {} of {} slots populated",
            self.kr_definition_count,
            self.kr_definitions_differing_from_ec,
            self.ec_definition_count,
            self.kr_texture_count,
            self.ec_texture_count,
            self.kr_textures_from_ec.len(),
            self.conflicting_texture_ids.len(),
            self.kr_route_count,
            self.kr_routes_changing_ec,
            self.ec_route_count,
            self.transcode_route_count,
            self.transcode_routes_dropped,
            match self.grunge_size {
                Some((width, height)) => format!("{width}x{height}"),
                None => "none".to_owned(),
            },
            self.package.page_count,
            self.package.populated_slot_count,
            self.package.slot_count,
        )
    }
}

pub fn convert_kr_land_to_tex_land_kr_uddp(
    kr_dir: &Path,
    ec_dir: &Path,
    out_file: &Path,
    options: &TexLandKrPackOptions,
) -> eyre::Result<TexLandKrBuildSummary> {
    convert_kr_land_to_tex_land_kr_uddp_with_progress(kr_dir, ec_dir, out_file, options, |_| {}, |_| {})
}

/// Builds `out_file` from the Kingdom Reborn client in `kr_dir`, taking what it lacks from the Enhanced Client in
/// `ec_dir` (see the module documentation).
pub fn convert_kr_land_to_tex_land_kr_uddp_with_progress(
    kr_dir: &Path,
    ec_dir: &Path,
    out_file: &Path,
    options: &TexLandKrPackOptions,
    payload_progress: impl Fn(AssetTaskProgress) + Sync,
    package_progress: impl FnMut(udd_container::BuildProgress),
) -> eyre::Result<TexLandKrBuildSummary> {
    validate_options(&options.atlas)?;
    // Before the decode and the packing, so a missing or invalid table fails at once.
    let (transcode_path, transcode_bytes) = read_transcode_kdl(options.atlas.transcode_kdl_path.as_deref())?;
    let terrain_overrides = read_terrain_overrides_kdl(options.atlas.terrain_overrides_kdl_path.as_deref())?;

    let kr_terrain_path = required_client_file(kr_dir, TERRAIN_DEFINITION_FILE_NAME, "--krdir")?;
    let kr_texture_path = required_client_file(kr_dir, TEXTURE_FILE_NAME, "--krdir")?;
    let ec_terrain_path = required_client_file(ec_dir, TERRAIN_DEFINITION_FILE_NAME, "--ecdir")?;
    let ec_texture_path = required_client_file(ec_dir, TEXTURE_FILE_NAME, "--ecdir")?;
    info!("Converting Kingdom Reborn Land to {}", out_file.display());
    for path in [&kr_terrain_path, &kr_texture_path, &ec_terrain_path, &ec_texture_path] {
        println!("Using {}", path.display());
    }

    let kr_definitions = load_definitions(&kr_terrain_path, ClientDialect::KingdomReborn)?;
    let ec_definitions = load_definitions(&ec_terrain_path, ClientDialect::Enhanced)?;
    let kr_textures =
        Textures::new(&kr_texture_path, None, None).wrap_err_with(|| format!("load {}", kr_texture_path.display()))?;
    let ec_textures =
        Textures::new(&ec_texture_path, None, None).wrap_err_with(|| format!("load {}", ec_texture_path.display()))?;

    let mut report = Vec::new();
    let definitions = merge_definitions(&kr_definitions, &ec_definitions, &mut report);
    let override_texture_ids = terrain_override_texture_ids(&terrain_overrides);
    let textures = choose_textures(
        &definitions,
        &override_texture_ids,
        &kr_textures,
        &ec_textures,
        &mut report,
    )?;

    let ec_routes = match find_first_existing_file(&[ec_dir.to_path_buf()], &[GAME_DATA_FILE_NAME]) {
        Some(game_data_path) => Some(
            LegacyTerrainMap::load(&game_data_path)
                .wrap_err_with(|| format!("load Enhanced Client terrain routing from {}", game_data_path.display()))?,
        ),
        None => {
            warn!("KR land: no {GAME_DATA_FILE_NAME} in --ecdir; only the Kingdom Reborn routes go to the csv.");
            None
        }
    };
    let routing = merge_routes(
        ec_routes,
        &KrLandMap::shipped_verified(),
        &definitions,
        &transcode_path,
        &transcode_bytes,
        &mut report,
    )?;

    let grunge = read_grunge(&kr_textures, &options.atlas, &mut report)?;
    let grunge_size = grunge.as_ref().map(|entry| (entry.width, entry.height));

    let terrain_definition = TerrainDefinitionPackage {
        entries: definitions.iter().map(|(entry, _)| entry.clone()).collect(),
    };
    let package = build_land_package(
        LandPackageInputs {
            terrain_definition: &terrain_definition,
            textures: LandTexturePool::PerTexture(textures.pool),
            transcode: (routing.transcode_label, routing.transcode_kdl.into_bytes()),
            legacy_terrain_map_csv: Some(routing.legacy_terrain_map_csv),
            terrain_overrides,
            extra_entries: grunge.into_iter().collect(),
        },
        out_file,
        &options.atlas,
        payload_progress,
        package_progress,
    )?;
    for texture_id in &package.ignored_source_texture_ids {
        record(&mut report, format!("texture {texture_id:08}: in neither client, not packed"));
    }

    let count = |client| definitions.iter().filter(|(_, source)| *source == client).count() as u32;
    let summary = TexLandKrBuildSummary {
        kr_definition_count: count(LandSourceClient::KingdomReborn),
        kr_definitions_differing_from_ec: kr_definitions
            .entries
            .iter()
            .filter(|kr| {
                ec_definitions.entries.iter().any(|ec| ec.id == kr.id && !same_definition(kr, ec))
            })
            .count() as u32,
        ec_definition_count: count(LandSourceClient::Enhanced),
        kr_texture_count: textures.kr_count,
        ec_texture_count: textures.ec_count,
        kr_textures_from_ec: textures.kr_textures_from_ec,
        conflicting_texture_ids: textures.conflicting_texture_ids,
        kr_route_count: routing.kr_route_count,
        kr_routes_changing_ec: routing.kr_routes_changing_ec,
        ec_route_count: routing.ec_route_count,
        transcode_route_count: routing.transcode_route_count,
        transcode_routes_dropped: routing.transcode_routes_dropped,
        grunge_size,
        package,
    };
    info!("KR land: {summary}");
    if let Some(report_path) = options.report.as_ref() {
        report.push(format!("summary: {summary}"));
        let mut text = report.join("\n");
        text.push('\n');
        std::fs::write(report_path, text).wrap_err_with(|| format!("write {}", report_path.display()))?;
        info!("KR land: findings written to '{}'.", report_path.display());
    }
    Ok(summary)
}

/// Logs one source decision and keeps it for the report.
fn record(report: &mut Vec<String>, line: String) {
    info!("KR land: {line}");
    report.push(line);
}

fn required_client_file(dir: &Path, file_name: &str, arg_name: &str) -> eyre::Result<PathBuf> {
    find_first_existing_file(&[dir.to_path_buf()], &[file_name])
        .ok_or_else(|| eyre::eyre!("missing required file in {arg_name} {}: {file_name}", dir.display()))
}

/// The definitions of `path`, which must be in `dialect`, with their alias flags in the Enhanced Client layout.
fn load_definitions(path: &Path, dialect: ClientDialect) -> eyre::Result<TerrainDefinitionPackage> {
    let package = UopPackage::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
    let found = detect_terrain_definition_dialect(&package)?;
    if found != dialect {
        eyre::bail!("{} is a {} TerrainDefinition.uop, expected {}", path.display(), found.label(), dialect.label());
    }
    let mut definitions = TerrainDefinitionPackage::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
    if dialect.is_kingdom_reborn() {
        for entry in &mut definitions.entries {
            *entry = entry.kr_to_enhanced();
        }
    }
    Ok(definitions)
}

/// Kingdom Reborn's definitions, then the Enhanced Client's of the ids KR lacks, sorted by id, each with its client.
fn merge_definitions(
    kr: &TerrainDefinitionPackage,
    ec: &TerrainDefinitionPackage,
    report: &mut Vec<String>,
) -> Vec<(TerrainDefinitionEntry, LandSourceClient)> {
    let ec_by_id = ec.entries.iter().map(|entry| (entry.id, entry)).collect::<BTreeMap<_, _>>();
    let kr_ids = kr.entries.iter().map(|entry| entry.id).collect::<BTreeSet<_>>();
    let mut merged = Vec::with_capacity(kr.entries.len() + ec.entries.len());
    for entry in &kr.entries {
        let comparison = match ec_by_id.get(&entry.id) {
            None => "the Enhanced Client lacks it",
            Some(ec_entry) if same_definition(entry, ec_entry) => "the same in the Enhanced Client",
            Some(_) => "differs from the Enhanced Client's",
        };
        record(report, format!("definition {} {:?}: {} ({comparison})", entry.id, entry.name, LandSourceClient::KingdomReborn.label()));
        merged.push((entry.clone(), LandSourceClient::KingdomReborn));
    }
    for entry in ec.entries.iter().filter(|entry| !kr_ids.contains(&entry.id)) {
        record(report, format!("definition {} {:?}: {} (Kingdom Reborn lacks it)", entry.id, entry.name, LandSourceClient::Enhanced.label()));
        merged.push((entry.clone(), LandSourceClient::Enhanced));
    }
    merged.sort_by_key(|(entry, _)| entry.id);
    merged
}

/// Whether a converted Kingdom Reborn definition reads as the Enhanced Client's: equal but for the indexes into each
/// client's own string table (the name, shader and layer paths compare as their strings).
fn same_definition(kr: &TerrainDefinitionEntry, ec: &TerrainDefinitionEntry) -> bool {
    let mut kr = kr.clone();
    kr.name_index = ec.name_index;
    if let (Some(kr_texture), Some(ec_texture)) = (kr.texture.as_mut(), ec.texture.as_ref()) {
        kr_texture.shader_name_index = ec_texture.shader_name_index;
        for (kr_layer, ec_layer) in kr_texture.layers.iter_mut().zip(&ec_texture.layers) {
            kr_layer.path_index = ec_layer.path_index;
        }
    }
    kr == *ec
}

fn entry_texture_ids(entry: &TerrainDefinitionEntry) -> impl Iterator<Item = u32> + '_ {
    entry.texture.iter().flat_map(|texture| texture.layers.iter().filter_map(|layer| layer.texture_id))
}

struct ChosenTextures<'a> {
    pool: HashMap<u32, &'a Textures>,
    kr_count: u32,
    ec_count: u32,
    kr_textures_from_ec: Vec<u32>,
    conflicting_texture_ids: Vec<u32>,
}

/// The client of each texture id: an id a Kingdom Reborn definition (or the overrides table) uses from KR, one only
/// Enhanced Client definitions use from EC; an id the chosen client lacks from the other one. An id both clients'
/// definitions use with different images is a conflict, and KR's image is packed for both.
fn choose_textures<'a>(
    definitions: &[(TerrainDefinitionEntry, LandSourceClient)],
    override_texture_ids: &BTreeSet<u32>,
    kr_textures: &'a Textures,
    ec_textures: &'a Textures,
    report: &mut Vec<String>,
) -> eyre::Result<ChosenTextures<'a>> {
    let ids_of = |client| {
        definitions
            .iter()
            .filter(|(_, source)| *source == client)
            .flat_map(|(entry, _)| entry_texture_ids(entry))
            .collect::<BTreeSet<_>>()
    };
    let kr_definition_ids = ids_of(LandSourceClient::KingdomReborn);
    let ec_definition_ids = ids_of(LandSourceClient::Enhanced);
    let preferring_kr = kr_definition_ids.union(override_texture_ids).copied().collect::<BTreeSet<_>>();

    let mut chosen = ChosenTextures {
        pool: HashMap::new(),
        kr_count: 0,
        ec_count: 0,
        kr_textures_from_ec: Vec::new(),
        conflicting_texture_ids: Vec::new(),
    };
    for texture_id in preferring_kr.union(&ec_definition_ids).copied() {
        let kr_file = find_layer_texture_file(texture_id, kr_textures)?;
        let ec_file = find_layer_texture_file(texture_id, ec_textures)?;
        let used_by = match (preferring_kr.contains(&texture_id), ec_definition_ids.contains(&texture_id)) {
            (true, true) => "both clients' definitions",
            (true, false) => "Kingdom Reborn definitions",
            _ => "Enhanced Client definitions",
        };
        let (client, note) = match (preferring_kr.contains(&texture_id), &kr_file, &ec_file) {
            (true, Some(kr_file), ec_file) => {
                let conflict = ec_definition_ids.contains(&texture_id)
                    && ec_file.as_ref().is_some_and(|ec_file| ec_file.raw_bytes() != kr_file.raw_bytes());
                if conflict {
                    chosen.conflicting_texture_ids.push(texture_id);
                }
                (Some(LandSourceClient::KingdomReborn), if conflict { "; conflict: the Enhanced Client's image differs" } else { "" })
            }
            (true, None, Some(_)) => {
                chosen.kr_textures_from_ec.push(texture_id);
                (Some(LandSourceClient::Enhanced), "; Kingdom Reborn lacks it")
            }
            (false, _, Some(_)) => (Some(LandSourceClient::Enhanced), ""),
            (false, Some(_), None) => (Some(LandSourceClient::KingdomReborn), "; the Enhanced Client lacks it"),
            (_, None, None) => (None, ""),
        };
        match client {
            Some(LandSourceClient::KingdomReborn) => {
                chosen.kr_count += 1;
                chosen.pool.insert(texture_id, kr_textures);
            }
            Some(LandSourceClient::Enhanced) => {
                chosen.ec_count += 1;
                chosen.pool.insert(texture_id, ec_textures);
            }
            None => {}
        }
        if let Some(client) = client {
            record(report, format!("texture {texture_id:08} (used by {used_by}): {}{note}", client.label()));
        }
    }
    Ok(chosen)
}

struct MergedRouting {
    legacy_terrain_map_csv: Vec<u8>,
    transcode_label: PathBuf,
    transcode_kdl: String,
    kr_route_count: u32,
    kr_routes_changing_ec: u32,
    ec_route_count: u32,
    transcode_route_count: u32,
    transcode_routes_dropped: u32,
}

/// The Enhanced Client's csv with Kingdom Reborn's routes over it, and the routing table without the ids KR routes,
/// so that the runtime, which reads the table over the csv, takes KR's route for every id KR has one for.
fn merge_routes(
    ec_routes: Option<LegacyTerrainMap>,
    kr_routes: &KrLandMap,
    definitions: &[(TerrainDefinitionEntry, LandSourceClient)],
    transcode_path: &Path,
    transcode_bytes: &[u8],
    report: &mut Vec<String>,
) -> eyre::Result<MergedRouting> {
    let materials = definitions.iter().map(|(entry, _)| entry.id).collect::<BTreeSet<_>>();
    let mut csv = ec_routes.unwrap_or_else(|| LegacyTerrainMap {
        routes: BTreeMap::new(),
        row_order: Vec::new(),
        source_csv: Vec::new(),
    });
    let ec_route_ids = csv.routes.keys().copied().collect::<BTreeSet<_>>();
    let mut kr_routes_changing_ec = 0;
    let kr_rows = kr_routes.rows();
    for &(legacy_id, material, subtype) in &kr_rows {
        let route = LegacyTerrainRoute { legacy_id, material_id: u32::from(material), subtype: u32::from(subtype) };
        if !materials.contains(&route.material_id) {
            warn!("KR land: Classic land {legacy_id} routes to material {material}, which no definition has");
        }
        let change = match csv.set_route(route) {
            None => "the Enhanced Client has no route".to_owned(),
            Some(previous) if previous == route => "the same in the Enhanced Client".to_owned(),
            Some(previous) => {
                kr_routes_changing_ec += 1;
                format!("the Enhanced Client routes it to {}/{}", previous.material_id, previous.subtype)
            }
        };
        record(report, format!("route {legacy_id} -> {material}/{subtype}: Kingdom Reborn ({change})"));
    }
    let kr_ids = kr_rows.iter().map(|&(legacy_id, _, _)| u32::from(legacy_id)).collect::<BTreeSet<_>>();
    let ec_route_count = ec_route_ids.iter().filter(|id| !kr_ids.contains(&u32::from(**id))).count() as u32;

    let source_name = transcode_path.display().to_string();
    let transcode_text = std::str::from_utf8(transcode_bytes).wrap_err_with(|| format!("{source_name}: not UTF-8 text"))?;
    let transcode = EckrTerrainRouting::from_str(&source_name, transcode_text)?.to_map();
    let mut kept = BTreeMap::<u32, Vec<u32>>::new();
    let mut transcode_routes_dropped = 0;
    for (legacy_id, material) in transcode.into_iter().collect::<BTreeMap<_, _>>() {
        if kr_ids.contains(&legacy_id) {
            transcode_routes_dropped += 1;
            continue;
        }
        kept.entry(material).or_default().push(legacy_id);
    }
    let transcode_route_count = kept.values().map(|ids| ids.len() as u32).sum();
    let file_name = transcode_path.file_name().map_or(source_name.clone(), |name| name.to_string_lossy().into_owned());
    let mut transcode_kdl = format!(
        "// {file_name} for the Kingdom Reborn land, written by the KR land packer (udd-cli pack-kr-land): its routes\n\
         // without the {} Classic land ids Kingdom Reborn's facets route, which metadata/legacy_terrain_map.csv carries.\n",
        kr_ids.len()
    );
    for (material, legacy_ids) in &kept {
        write!(transcode_kdl, "t {material}").expect("write to a String");
        for legacy_id in legacy_ids {
            write!(transcode_kdl, " {legacy_id}").expect("write to a String");
        }
        transcode_kdl.push('\n');
    }
    // The runtime reads the embedded copy strictly.
    let transcode_label = PathBuf::from(format!("{file_name} without the Kingdom Reborn routes"));
    EckrTerrainRouting::from_str(&transcode_label.display().to_string(), &transcode_kdl)?;
    record(
        report,
        format!(
            "routing: {} Kingdom Reborn routes ({kr_routes_changing_ec} change the Enhanced Client's), {ec_route_count} \
             Enhanced Client csv routes, {transcode_route_count} {file_name} routes kept, {transcode_routes_dropped} dropped",
            kr_rows.len()
        ),
    );
    Ok(MergedRouting {
        legacy_terrain_map_csv: csv.to_csv_bytes(),
        transcode_label,
        transcode_kdl,
        kr_route_count: kr_rows.len() as u32,
        kr_routes_changing_ec,
        ec_route_count,
        transcode_route_count,
        transcode_routes_dropped,
    })
}

/// Kingdom Reborn's grunge texture as an RGBA8888 entry behind its version header ([`encode_grunge_entry`]),
/// compressed as the pages are.
fn read_grunge(
    kr_textures: &Textures,
    atlas: &TexLandEcAtlasOptions,
    report: &mut Vec<String>,
) -> eyre::Result<Option<ExtraLandEntry>> {
    let Some(file) = kr_textures.get_from_id(KR_GRUNGE_TEXTURE_ID)? else {
        warn!("KR land: Kingdom Reborn has no grunge texture {KR_GRUNGE_TEXTURE_ID:08}; the package goes without one.");
        return Ok(None);
    };
    let image = file.decode_to_rgba8().wrap_err("decode the Kingdom Reborn grunge texture")?;
    let (width, height) = image.dimensions();
    record(
        report,
        format!("grunge: texture {KR_GRUNGE_TEXTURE_ID:08} {width}x{height}, {} -> {UDDP_GRUNGE_ENTRY_VPATH}", LandSourceClient::KingdomReborn.label()),
    );
    Ok(Some(ExtraLandEntry {
        virtual_path: UDDP_GRUNGE_ENTRY_VPATH,
        data_type: DataType::Texture,
        compression: atlas.compression,
        width,
        height,
        data: encode_grunge_entry(width, height, image.as_raw())?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::terrain_definition::{TerrainDefinitionTexture, TerrainDefinitionTextureLayer};
    use uocf::kr::facet::kr_land_map::KrLand;

    const GRASS: u32 = 1;
    const DIRT: u32 = 2;
    const EC_ONLY_MATERIAL: u32 = 200;
    const GRASS_TEXTURE: u32 = 2_000_010;

    fn definition(id: u32, name_index: u32, path_index: u32, coverage_exponent: f32) -> TerrainDefinitionEntry {
        TerrainDefinitionEntry {
            id,
            name_index,
            coverage_exponent,
            name: Some(format!("material {id}")),
            texture: Some(TerrainDefinitionTexture {
                shader_name_index: name_index,
                shader_name: Some("TerrainDefault".to_owned()),
                layers: vec![TerrainDefinitionTextureLayer {
                    path_index,
                    texture_id: Some(GRASS_TEXTURE),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn kingdom_reborn_definitions_come_first_and_the_enhanced_client_fills_the_rest() {
        // The string indexes differ between the clients' tables; only the coverage of DIRT differs.
        let kr = TerrainDefinitionPackage { entries: vec![definition(GRASS, 10, 11, 1.0), definition(DIRT, 12, 13, 2.0)] };
        let ec = TerrainDefinitionPackage {
            entries: vec![
                definition(GRASS, 20, 21, 1.0),
                definition(DIRT, 22, 23, 3.0),
                definition(EC_ONLY_MATERIAL, 24, 25, 1.0),
            ],
        };
        let mut report = Vec::new();

        let merged = merge_definitions(&kr, &ec, &mut report);

        let sources: Vec<_> = merged.iter().map(|(entry, source)| (entry.id, *source)).collect();
        assert_eq!(
            sources,
            [
                (GRASS, LandSourceClient::KingdomReborn),
                (DIRT, LandSourceClient::KingdomReborn),
                (EC_ONLY_MATERIAL, LandSourceClient::Enhanced),
            ]
        );
        assert_eq!(merged[1].0.coverage_exponent, 2.0, "KR's definition is packed");
        assert!(same_definition(&kr.entries[0], &ec.entries[0]));
        assert!(!same_definition(&kr.entries[1], &ec.entries[1]));
        assert!(report[0].ends_with("Kingdom Reborn (the same in the Enhanced Client)"), "{}", report[0]);
        assert!(report[1].ends_with("Kingdom Reborn (differs from the Enhanced Client's)"), "{}", report[1]);
        assert!(report[2].ends_with("Enhanced Client (Kingdom Reborn lacks it)"), "{}", report[2]);
    }

    #[test]
    fn kingdom_reborn_routes_replace_the_csv_ones_and_leave_the_routing_table() {
        const KR_ROUTED_LAND: u16 = 3;
        const EC_ROUTED_LAND: u16 = 4;
        const TABLE_ONLY_LAND: u32 = 5;
        const KR_SUBTYPE: u8 = 2;
        let mut ec_routes = LegacyTerrainMap { routes: BTreeMap::new(), row_order: Vec::new(), source_csv: Vec::new() };
        ec_routes.set_route(LegacyTerrainRoute { legacy_id: KR_ROUTED_LAND, material_id: DIRT, subtype: 0 });
        ec_routes.set_route(LegacyTerrainRoute { legacy_id: EC_ROUTED_LAND, material_id: DIRT, subtype: 1 });
        let mut kr_routes = KrLandMap::default();
        kr_routes.insert(KR_ROUTED_LAND, KrLand { material: GRASS as u16, subtype: KR_SUBTYPE });
        let definitions = [
            (definition(GRASS, 0, 0, 1.0), LandSourceClient::KingdomReborn),
            (definition(DIRT, 0, 0, 1.0), LandSourceClient::KingdomReborn),
        ];
        let table = format!("t {DIRT} {KR_ROUTED_LAND} {TABLE_ONLY_LAND}\n");
        let mut report = Vec::new();

        let merged = merge_routes(
            Some(ec_routes),
            &kr_routes,
            &definitions,
            Path::new("/tables/TerrainTranscode.kdl"),
            table.as_bytes(),
            &mut report,
        )
        .expect("merge");

        let csv = String::from_utf8(merged.legacy_terrain_map_csv).expect("csv text");
        assert!(csv.contains(&format!("{KR_ROUTED_LAND},{GRASS},{KR_SUBTYPE}")), "{csv}");
        assert!(csv.contains(&format!("{EC_ROUTED_LAND},{DIRT},1")), "{csv}");
        let kept = EckrTerrainRouting::from_str("kept", &merged.transcode_kdl).expect("strict KDL").to_map();
        assert_eq!(kept, HashMap::from([(TABLE_ONLY_LAND, DIRT)]));
        assert_eq!((merged.kr_route_count, merged.kr_routes_changing_ec, merged.ec_route_count), (1, 1, 1));
        assert_eq!((merged.transcode_route_count, merged.transcode_routes_dropped), (1, 1));
    }
}
