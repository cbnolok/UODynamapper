#![allow(unused)]

use crate::configs::settings::Settings;
use crate::core::maps::MapPlane;
use crate::core::multis::MultiDefinitionsRes;
use crate::core::statics::{LazyStaticsStore, StaticsStoreRes};
use crate::core::system_sets::StartupSysSet;
use crate::prelude::*;
use bevy::prelude::*;
use parking_lot::Mutex;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use udd_assets::AtlasCacheOptions;
use udd_assets::tilemeta::TileMetaPackage;
use udd_container::{UddpReader, UddpReaderOptions};

const MAX_MAP_INDEX: u32 = 5; // inclusive max, so map0..=map5

/// Arc is not yet needed (no cross-thread sharing), but kept for consistency
/// with other UO data resources and future background-thread access.
#[derive(Resource)]
pub struct UoFilesSettingsRes(pub Arc<UoFilesSettings>);

/// Not wrapped in Arc: owned exclusively by the Bevy main world.
/// Accessed only via Res<MapPlanesRes> on the main thread.
#[derive(Resource)]
pub struct MapPlanesRes(pub Vec<Option<MapPlane>>);

/// Preferred runtime metadata package for static/item rendering.
#[derive(Resource)]
pub struct TileMetaPackageRes(pub Arc<TileMetaPackage>);

/// Arc: cloned into the chunk-loader OS thread (via LoadRequest.texmap_2d)
/// and passed to texture cache systems that warm pixel data off-thread.
#[derive(Resource)]
pub struct TexMap2DRes(pub Arc<udd_assets::tex_land_cc::TexLandCcPackage>);

/// Optional prepacked atlas package for static and land art.
#[derive(Resource)]
pub struct TexArtCcPackageRes(pub Arc<udd_assets::tex_art_cc::TexArtCcPackage>);

/// Optional prepacked atlas package for EC static art.
#[derive(Resource)]
pub struct TexArtEcPackageRes(pub Arc<udd_assets::tex_art_ec::TexArtEcPackage>);

/// Optional prepacked atlas package for EC land art.
#[derive(Resource)]
pub struct TexLandEcPackageRes(pub Arc<udd_assets::tex_land_ec::TexLandEcPackage>);

/// Optional auxiliary texture package for EC non-base terrain slots (secondary base, alpha mask, noise).
#[derive(Resource)]
pub struct TexAuxEcPackageRes(pub Arc<udd_assets::tex_aux_ec::TexAuxEcPackage>);

/// Optional terrain-layer sidecar for identifying secondary-base and alpha-mask layers.
#[derive(Resource)]
pub struct TerrainMetaPackageRes(pub Arc<udd_assets::terrain_meta::TerrainMetaPackage>);

/// Optional prepacked world light mask package from light.mul/lightidx.mul.
#[derive(Resource)]
pub struct WorldLightsPackageRes(pub Arc<udd_assets::world_lights::WorldLightsPackage>);

/// Optional gump art source. UDDP packages are preferred, legacy Classic sources are fallback.
#[derive(Resource)]
pub struct GumpMapRes {
    package: Option<Arc<udd_assets::GumpsPackage>>,
    classic: Option<Arc<uocf::classic::gump::GumpMap>>,
}

impl GumpMapRes {
    pub fn from_package(package: udd_assets::GumpsPackage) -> Self {
        Self {
            package: Some(Arc::new(package)),
            classic: None,
        }
    }

    pub fn from_classic(classic: uocf::classic::gump::GumpMap) -> Self {
        Self {
            package: None,
            classic: Some(Arc::new(classic)),
        }
    }

    pub fn decode_gump(
        &self,
        gump_id: u32,
        scratch: &mut Vec<u8>,
    ) -> color_eyre::eyre::Result<udd_assets::gumps::GumpImage> {
        if let Some(package) = &self.package {
            return package.read_gump_image(gump_id);
        }

        let classic = self
            .classic
            .as_ref()
            .ok_or_else(|| color_eyre::eyre::eyre!("gump source is not loaded"))?;
        let (width, height, pixels) = classic.decode_gump(gump_id, scratch)?;
        Ok(udd_assets::gumps::GumpImage {
            physical_width: u32::from(width),
            physical_height: u32::from(height),
            logical_width: u32::from(width),
            logical_height: u32::from(height),
            upscale_factor: 1,
            rgba: pixels,
        })
    }
}

/// Optional Classic Client bitmap fonts.
#[derive(Resource)]
pub struct ClassicFontsRes(pub Arc<uocf::classic::fonts::ClassicFonts>);

/// Optional packed hue lookup texture and metadata.
#[derive(Resource)]
pub struct HuesPackageRes(pub Arc<udd_assets::HuesPackage>);

pub struct UoFilesSettings {
    pub udd_folder: PathBuf,
}

#[derive(Clone, Copy)]
enum SourceContainerKind {
    Uddp,
    Mul,
    Uop,
}

impl SourceContainerKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Uddp => "uddp",
            Self::Mul => "mul",
            Self::Uop => "uop",
        }
    }
}

fn log_source_choice(
    lg: &impl Fn(&str),
    logical_name: &str,
    kind: SourceContainerKind,
    paths: &[PathBuf],
) {
    let joined = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    lg(&format!(
        "Using {logical_name} source [{}]: {joined}",
        kind.label()
    ));
}

fn resolve_optional_uddp_path(
    uddp_root: &Path,
    file_name: &str,
) -> Option<PathBuf> {
    let preferred = uddp_root.join(file_name);
    if preferred.is_file() {
        return Some(preferred);
    }

    None
}

fn resolve_optional_uddp_paths(
    uddp_root: &Path,
    file_names: &[&str],
) -> Option<PathBuf> {
    file_names
        .iter()
        .find_map(|file_name| resolve_optional_uddp_path(uddp_root, file_name))
}

pub struct UOFilesPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(UOFilesPlugin);
impl Plugin for UOFilesPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(
            Startup,
            sys_setup_uo_data.in_set(StartupSysSet::LoadStartupUOFiles),
        );
    }
}

/// How many unresolved terrain override texture refs the load log names.
const UNRESOLVED_OVERRIDE_REF_LOG_SAMPLE: usize = 8;

fn log_tex_land_ec_coverage(lg: &impl Fn(&str), package: &udd_assets::tex_land_ec::TexLandEcPackage) {
    let populated_slot_count = package
        .slots()
        .iter()
        .filter(|slot| slot.is_present())
        .count();
    let alias_ref_count = package.terrain_provenance().len();
    let slot_table_len = package.slots().len();

    lg(&format!(
        "Enhanced land package coverage: {populated_slot_count} populated slots from {alias_ref_count} TerrainDefinition alias refs (slot table span: 0..{}). Unmapped EC land ids remain explicitly black.",
        slot_table_len.saturating_sub(1),
    ));
    if let Some(source_map) = package.source_terrain_map() {
        lg(&format!("Loaded {} original GameData.uop terrain routes with preserved subtypes.", source_map.routes.len()));
    } else {
        lg("No original GameData.uop terrain routes are embedded; rebuild EC land with the current converter to recover source terrain routes.");
    }
    let summary = package.terrain_override_load_summary();
    lg(&format!(
        "Embedded EcTerrainOverrides.kdl: entries={} actions={} texture_refs={} resolved={}.",
        summary.entry_count,
        summary.action_count,
        summary.texture_ref_count,
        summary.resolved_texture_ref_count,
    ));
    if !summary.unresolved_texture_refs.is_empty() {
        let sample = summary
            .unresolved_texture_refs
            .iter()
            .take(UNRESOLVED_OVERRIDE_REF_LOG_SAMPLE)
            .map(|texture_ref| {
                format!(
                    "{}:{}:{}",
                    texture_ref.material_id,
                    texture_ref.role,
                    texture_ref.texture_id
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        bevy::log::warn!(
            "Embedded EcTerrainOverrides.kdl references {} texture(s) that are not packed in tex_land_ec.uddp. Falling back where needed. Sample: {}",
            summary.unresolved_texture_refs.len(),
            sample
        );
    }
}

pub fn sys_setup_uo_data(mut commands: Commands, settings: Res<Settings>) {
    log_system_add_startup::<UOFilesPlugin>(StartupSysSet::LoadStartupUOFiles, fname!());
    let lg = |text: &str| {
        console_logger::one(
            console_logger::LogSev::Info,
            console_logger::LogAbout::UoFiles,
            text,
        )
    };
    let lg_err = |text: &str| {
        console_logger::one(
            console_logger::LogSev::Error,
            console_logger::LogAbout::UoFiles,
            text,
        )
    };
    let udd_path = PathBuf::from(&settings.runtime_assets.udd_path);

    lg("Start loading UO Data.");
    lg(&format!(
        "Resolved source root. UDDP root: '{}'.",
        udd_path.display()
    ));

    let mut map_planes: Vec<Option<MapPlane>> = std::iter::repeat_with(|| None)
        .take((MAX_MAP_INDEX + 1) as usize)
        .collect::<Vec<_>>();
    let mut statics_stores: Vec<Option<Mutex<LazyStaticsStore>>> = std::iter::repeat_with(|| None)
        .take((MAX_MAP_INDEX + 1) as usize)
        .collect::<Vec<_>>();

    for map_plane_index in 0..=MAX_MAP_INDEX {
        if map_plane_index != settings.session_state.world.last_p.m as u32 {
            continue;
        }

        let map_file_name = format!("map{map_plane_index}.uddp");
        let map_path = resolve_optional_uddp_path(&udd_path, &map_file_name);
        if let Some(map_path) = map_path {
            log_source_choice(
                &lg,
                &format!("map plane {map_plane_index}"),
                SourceContainerKind::Uddp,
                std::slice::from_ref(&map_path),
            );
            let map_plane = MapPlane::load(map_path.clone(), map_plane_index)
                .unwrap_or_else(|error| panic!("Error initializing map plane {map_plane_index}: {error:#}"));

            let statics_file_name = format!("statics{map_plane_index}.uddp");
            let statics_path = resolve_optional_uddp_path(&udd_path, &statics_file_name);
            if let Some(statics_path) = statics_path {
                log_source_choice(
                    &lg,
                    &format!("statics plane {map_plane_index}"),
                    SourceContainerKind::Uddp,
                    std::slice::from_ref(&statics_path),
                );
                let store = LazyStaticsStore::new(
                    &statics_path,
                    map_plane.size_blocks.width * 8,
                    map_plane.size_blocks.height * 8,
                )
                .unwrap_or_else(|_| {
                    panic!("Error initializing statics reader for plane {map_plane_index}")
                });
                statics_stores[map_plane_index as usize] = Some(Mutex::new(store));
            } else {
                lg_err(&format!(
                    "No statics source selected for plane {map_plane_index}: {statics_file_name} not found in udd_path."
                ));
            }

            map_planes[map_plane_index as usize] = Some(map_plane);
        } else {
            lg_err(&format!(
                "No map source selected for plane {map_plane_index}: {map_file_name} not found in udd_path."
            ));
        }
    }

    let tilemeta_path = resolve_optional_uddp_paths(&udd_path, &["tilemeta.uddp"])
        .unwrap_or_else(|| panic!("tilemeta.uddp is required in udd_path"));
    log_source_choice(
        &lg,
        "tile metadata",
        SourceContainerKind::Uddp,
        std::slice::from_ref(&tilemeta_path),
    );
    let tilemeta_package = Arc::new(
        TileMetaPackage::load(&tilemeta_path)
            .unwrap_or_else(|_| panic!("Error loading {}", tilemeta_path.display())),
    );
    let land_atlas_reader_options = UddpReaderOptions::enable_decoded_entry_cache();

    let texmaps_package_path = resolve_optional_uddp_path(&udd_path, "tex_land_cc.uddp")
        .unwrap_or_else(|| panic!("tex_land_cc.uddp is required in udd_path"));
    log_source_choice(
        &lg,
        "terrain texmaps package",
        SourceContainerKind::Uddp,
        std::slice::from_ref(&texmaps_package_path),
    );
    let texmap_2d = udd_assets::tex_land_cc::TexLandCcPackage::from_uddp_package_with_options(
        UddpReader::load_with_options(&texmaps_package_path, land_atlas_reader_options)
            .unwrap_or_else(|_| panic!("Error loading {}", texmaps_package_path.display())),
        AtlasCacheOptions::enabled(),
    )
    .unwrap_or_else(|_| panic!("Error loading {}", texmaps_package_path.display()));

    let tex_art_cc_path = resolve_optional_uddp_path(&udd_path, "tex_art_cc.uddp");
    let tex_art_cc_package = if let Some(tex_art_cc_path) = tex_art_cc_path {
        log_source_choice(
            &lg,
            "classic art package",
            SourceContainerKind::Uddp,
            std::slice::from_ref(&tex_art_cc_path),
        );
        Some(
            udd_assets::tex_art_cc::TexArtCcPackage::from_uddp_package_with_options(
                UddpReader::load_with_options(&tex_art_cc_path, UddpReaderOptions::disabled())
                    .unwrap_or_else(|_| panic!("Error loading {}", tex_art_cc_path.display())),
                AtlasCacheOptions::disabled(),
            )
                .unwrap_or_else(|_| panic!("Error loading {}", tex_art_cc_path.display())),
        )
    } else {
        lg_err("No classic art package source selected: tex_art_cc.uddp not found in udd_path.");
        None
    };

    let tex_art_ec_path = resolve_optional_uddp_path(&udd_path, "tex_art_ec.uddp");
    let tex_art_ec_package = if let Some(tex_art_ec_path) = tex_art_ec_path {
        log_source_choice(
            &lg,
            "enhanced art package",
            SourceContainerKind::Uddp,
            std::slice::from_ref(&tex_art_ec_path),
        );
        Some(
            udd_assets::tex_art_ec::TexArtEcPackage::from_uddp_package_with_options(
                UddpReader::load_with_options(&tex_art_ec_path, UddpReaderOptions::disabled())
                    .unwrap_or_else(|_| panic!("Error loading {}", tex_art_ec_path.display())),
                AtlasCacheOptions::disabled(),
            )
                .unwrap_or_else(|_| panic!("Error loading {}", tex_art_ec_path.display())),
        )
    } else {
        lg_err("No enhanced art package source selected: tex_art_ec.uddp not found in udd_path.");
        None
    };

    let tex_land_ec_path = resolve_optional_uddp_path(&udd_path, "tex_land_ec.uddp");
    let tex_land_ec_package = if let Some(tex_land_ec_path) = tex_land_ec_path {
        log_source_choice(
            &lg,
            "enhanced land package",
            SourceContainerKind::Uddp,
            std::slice::from_ref(&tex_land_ec_path),
        );
        let package = udd_assets::tex_land_ec::TexLandEcPackage::from_uddp_package_with_options(
            UddpReader::load_with_options(&tex_land_ec_path, land_atlas_reader_options)
                .unwrap_or_else(|_| panic!("Error loading {}", tex_land_ec_path.display())),
            AtlasCacheOptions::enabled(),
        )
        .unwrap_or_else(|error| panic!("Error loading {}: {error:#}", tex_land_ec_path.display()));
        log_tex_land_ec_coverage(&lg, &package);
        Some(package)
    } else {
        lg_err("No enhanced land package source selected: tex_land_ec.uddp not found in udd_path.");
        None
    };

    let tex_aux_ec_path = resolve_optional_uddp_path(&udd_path, "tex_aux_ec.uddp");
    let tex_aux_ec_package = if let Some(tex_aux_ec_path) = tex_aux_ec_path {
        log_source_choice(
            &lg,
            "EC auxiliary texture package",
            SourceContainerKind::Uddp,
            std::slice::from_ref(&tex_aux_ec_path),
        );
        match udd_assets::tex_aux_ec::TexAuxEcPackage::load(&tex_aux_ec_path) {
            Ok(package) => Some(package),
            Err(e) => {
                lg_err(&format!("Failed to load tex_aux_ec.uddp: {e}"));
                None
            }
        }
    } else {
        lg("No EC auxiliary texture package found: tex_aux_ec.uddp not found in udd_path.");
        None
    };

    let terrain_meta_path = resolve_optional_uddp_path(&udd_path, "terrain_meta.bin");
    let terrain_meta_package = if let Some(terrain_meta_path) = terrain_meta_path {
        log_source_choice(
            &lg,
            "terrain meta sidecar",
            SourceContainerKind::Mul,
            std::slice::from_ref(&terrain_meta_path),
        );
        match udd_assets::terrain_meta::TerrainMetaPackage::load(&terrain_meta_path) {
            Ok(package) => Some(package),
            Err(e) => {
                lg_err(&format!("Failed to load terrain_meta.bin: {e}"));
                None
            }
        }
    } else {
        lg("No terrain meta sidecar found: terrain_meta.bin not found in udd_path.");
        None
    };

    let world_lights_path = resolve_optional_uddp_path(&udd_path, "world_lights.uddp");
    let world_lights_package = if let Some(world_lights_path) = world_lights_path {
        log_source_choice(
            &lg,
            "world light masks",
            SourceContainerKind::Uddp,
            std::slice::from_ref(&world_lights_path),
        );
        Some(
            udd_assets::world_lights::WorldLightsPackage::load(&world_lights_path)
                .unwrap_or_else(|_| panic!("Error loading {}", world_lights_path.display())),
        )
    } else {
        lg("No world light mask package selected: world_lights.uddp not found in udd_path.");
        None
    };

    let load_classic_gumps = || match uocf::classic::gump::GumpMap::load(&udd_path) {
        Ok(gump_map) => {
            lg("Loaded Classic Client gump art source.");
            Some(GumpMapRes::from_classic(gump_map))
        }
        Err(error) => {
            lg_err(&format!(
                "No Classic gump source selected from {}: {error}",
                udd_path.display()
            ));
            None
        }
    };
    let gump_map = if !settings.runtime_assets.load_gumps {
        lg("Gump art loading is disabled by runtime_assets.load_gumps.");
        None
    } else {
        let gump_package_path =
            resolve_optional_uddp_paths(&udd_path, &["gumps_cc.uddp", "gumps_ec.uddp"]);
        if let Some(gump_package_path) = gump_package_path {
            log_source_choice(
                &lg,
                "gumps",
                SourceContainerKind::Uddp,
                std::slice::from_ref(&gump_package_path),
            );
            match udd_assets::GumpsPackage::load(&gump_package_path) {
                Ok(package) => Some(GumpMapRes::from_package(package)),
                Err(error) => {
                    lg_err(&format!(
                        "Failed to load gump package {}: {error}",
                        gump_package_path.display()
                    ));
                    load_classic_gumps()
                }
            }
        } else {
            load_classic_gumps()
        }
    };

    let classic_fonts = match uocf::classic::fonts::ClassicFonts::load(&udd_path) {
        Ok(fonts) => {
            lg(&format!(
                "Loaded Classic Client fonts: {} ASCII font face(s).",
                fonts.ascii_font_count()
            ));
            Some(fonts)
        }
        Err(error) => {
            lg("No Classic Client font source selected: fonts.mul/unifont*.mul not found in udd_path.");
            bevy::log::debug!("Classic Client font load detail: {error}");
            None
        }
    };

    let hues_package_path = resolve_optional_uddp_path(&udd_path, "hues.uddp");
    let hues_package = if let Some(hues_package_path) = hues_package_path {
        log_source_choice(
            &lg,
            "hue lookup package",
            SourceContainerKind::Uddp,
            std::slice::from_ref(&hues_package_path),
        );
        match udd_assets::HuesPackage::load(&hues_package_path) {
            Ok(package) => Some(package),
            Err(error) => {
                lg_err(&format!("Failed to load hues.uddp: {error}"));
                None
            }
        }
    } else {
        lg("No hue lookup package selected: hues.uddp not found in udd_path.");
        None
    };

    lg("Done loading UO Data.");

    // The CC-EC land routing table (TerrainTranscode.kdl) and the terrain overrides (EcTerrainOverrides.kdl) are
    // embedded in tex_land_ec.uddp, which applies them.

    let multi_definitions = load_multi_definitions(&udd_path, &lg, &lg_err);

    commands.insert_resource(UoFilesSettingsRes(Arc::new(UoFilesSettings {
        udd_folder: udd_path,
    })));
    commands.insert_resource(MapPlanesRes(map_planes));
    commands.insert_resource(TileMetaPackageRes(tilemeta_package.clone()));
    commands.insert_resource(TexMap2DRes(Arc::new(texmap_2d)));
    if let Some(tex_art_cc_package) = tex_art_cc_package {
        commands.insert_resource(TexArtCcPackageRes(Arc::new(tex_art_cc_package)));
    }
    if let Some(tex_art_ec_package) = tex_art_ec_package {
        commands.insert_resource(TexArtEcPackageRes(Arc::new(tex_art_ec_package)));
    }
    if let Some(tex_land_ec_package) = tex_land_ec_package {
        commands.insert_resource(TexLandEcPackageRes(Arc::new(tex_land_ec_package)));
    }
    if let Some(world_lights_package) = world_lights_package {
        commands.insert_resource(WorldLightsPackageRes(Arc::new(world_lights_package)));
    }
    if let Some(gump_map) = gump_map {
        commands.insert_resource(gump_map);
    }
    if let Some(classic_fonts) = classic_fonts {
        commands.insert_resource(ClassicFontsRes(Arc::new(classic_fonts)));
    }
    if let Some(hues_package) = hues_package {
        commands.insert_resource(HuesPackageRes(Arc::new(hues_package)));
    }
    if let Some(multis) = multi_definitions {
        commands.insert_resource(multis);
    }
    if let Some(tex_aux_ec_package) = tex_aux_ec_package {
        commands.insert_resource(TexAuxEcPackageRes(Arc::new(tex_aux_ec_package)));
    }
    if let Some(terrain_meta_package) = terrain_meta_package {
        commands.insert_resource(TerrainMetaPackageRes(Arc::new(terrain_meta_package)));
    }
    commands.insert_resource(StaticsStoreRes(statics_stores));
}

fn load_multi_definitions(
    source_root: &Path,
    lg: &impl Fn(&str),
    lg_err: &impl Fn(&str),
) -> Option<MultiDefinitionsRes> {
    if source_root.join("multi.mul").is_file()
        && (source_root.join("multi.idx").is_file() || source_root.join("multiidx.mul").is_file())
    {
        match uocf::classic::multi::MultiMap::load(source_root)
            .and_then(|multis| multis.load_all_parts())
        {
            Ok(parts) => {
                let definitions = MultiDefinitionsRes::from_classic_parts(parts);
                lg(&format!(
                    "Loaded classic multi definitions: {} visible definitions.",
                    definitions.definition_count()
                ));
                return Some(definitions);
            }
            Err(error) => {
                lg_err(&format!(
                    "Failed to load classic multi definitions from {}: {error}",
                    source_root.display()
                ));
            }
        }
    }

    for file_name in ["MultiCollection.uop", "multicollection.uop"] {
        let path = source_root.join(file_name);
        if !path.is_file() {
            continue;
        }

        match uocf::enhanced::multis::MultiCollection::load(&path) {
            Ok(collection) => {
                let definitions = MultiDefinitionsRes::from_ec_collection(&collection);
                lg(&format!(
                    "Loaded EC multi definitions: {} visible definitions from {}.",
                    definitions.definition_count(),
                    path.display()
                ));
                return Some(definitions);
            }
            Err(error) => {
                lg_err(&format!(
                    "Failed to load EC multi definitions from {}: {error}",
                    path.display()
                ));
            }
        }
    }

    lg("No multi definition source selected: multi.mul/multi.idx or MultiCollection.uop not found in udd_path.");
    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    #[ignore = "requires current canonical packages in DYNAMAPPER_TEST_PACKAGES"]
    fn classic_land_routes_follow_the_client_terrain_map() {
        let root = PathBuf::from(std::env::var_os("DYNAMAPPER_TEST_PACKAGES")
            .expect("set DYNAMAPPER_TEST_PACKAGES to the matching current package directory"));
        let package = udd_assets::tex_land_ec::TexLandEcPackage::load(root.join("tex_land_ec.uddp")).unwrap();
        let client = package.source_terrain_map().expect("the package embeds legacyterrainmap.csv").routes.clone();
        assert!(!client.is_empty());
        // Same as the loader: the client's map, then the package's embedded KDL routes over it.
        let differing: BTreeMap<u16, (u32, Option<u32>)> = client
            .values()
            .filter_map(|route| {
                let effective = package.resolve_material_decision(u32::from(route.legacy_id)).material_id;
                (effective != Some(route.material_id)).then_some((route.legacy_id, (route.material_id, effective)))
            })
            .collect();
        assert!(differing.is_empty(), "classic land id -> (client material, effective material): {differing:?}");
    }
}
