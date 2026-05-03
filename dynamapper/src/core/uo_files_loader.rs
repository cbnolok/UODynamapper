#![allow(unused)]

use crate::configs::settings::Settings;
use crate::core::system_sets::StartupSysSet;
use crate::prelude::*;
use bevy::prelude::*;
//use parking_lot::RwLock;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uddconv::tilemeta::TileMetaPackage;
use uocf::classic::tiledata;
use uocf::classic::{land_texture, map};
use std::collections::HashMap;

const MAX_MAP_INDEX: u32 = 5; // inclusive max, so map0..=map5

/// Arc is not yet needed (no cross-thread sharing), but kept for consistency
/// with other UO data resources and future background-thread access.
#[derive(Resource)]
pub struct UoFilesSettingsRes(pub Arc<UoFilesSettings>);

/// Not wrapped in Arc: owned exclusively by the Bevy main world.
/// Accessed only via Res<MapPlanesRes> on the main thread.
#[derive(Resource)]
pub struct MapPlanesRes(pub Vec<Option<map::MapPlane>>);

/// Arc: will be cloned and sent to background threads for tiledata lookups
/// (e.g. future item/static rendering, pathfinding).
#[derive(Resource)]
pub struct TileDataRes(pub Arc<tiledata::TileData>);

/// Preferred runtime metadata package for static/item rendering.
#[derive(Resource)]
pub struct TileMetaPackageRes(pub Arc<TileMetaPackage>);

/// Arc: cloned into the chunk-loader OS thread (via LoadRequest.texmap_2d)
/// and passed to texture cache systems that warm pixel data off-thread.
#[derive(Resource)]
pub struct TexMap2DRes(pub Arc<land_texture::TexMap>);

/// Optional prepacked atlas package for static and land art.
#[derive(Resource)]
pub struct CcArtPackageRes(pub Arc<uddconv::cc_art::CcArtPackage>);

/// Optional prepacked atlas package for EC static art.
#[derive(Resource)]
pub struct EcArtPackageRes(pub Arc<uddconv::ec_art::EcArtPackage>);

/// Optional prepacked atlas package for EC land art.
#[derive(Resource)]
pub struct EcLandPackageRes(pub Arc<uddconv::ec_land::EcLandPackage>);

/// Transcode table for Classic to Enhanced terrain IDs.
#[derive(Resource)]
pub struct TerrainTranscodeRes(pub Arc<HashMap<u32, u32>>);

/// Enhanced terrain definitions.
#[derive(Resource)]
pub struct TerrainDefinitionRes(pub Arc<HashMap<u32, uddconv::cc_ec_conv::TerrainDefEntry>>);

/// Stored statics data for each map plane.
#[derive(Resource)]
pub struct StaticsStoreRes(pub Vec<Option<Arc<uocf::classic::statics::StaticsStore>>>);

/// Radarcol palette for dot mode rendering.
#[derive(Resource)]
pub struct RadarColRes(pub Arc<Vec<uocf::utils::color::Rgb555>>);

pub struct UoFilesSettings {
    pub base_folder: PathBuf,
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
    raw_root: &Path,
    file_name: &str,
) -> Option<PathBuf> {
    let preferred = uddp_root.join(file_name);
    if preferred.is_file() {
        return Some(preferred);
    }

    if uddp_root != raw_root {
        let fallback = raw_root.join(file_name);
        if fallback.is_file() {
            return Some(fallback);
        }
    }

    None
}

fn resolve_optional_uddp_paths(
    uddp_root: &Path,
    raw_root: &Path,
    file_names: &[&str],
) -> Option<PathBuf> {
    file_names
        .iter()
        .find_map(|file_name| resolve_optional_uddp_path(uddp_root, raw_root, file_name))
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

fn log_ec_land_coverage(lg: &impl Fn(&str), package: &uddconv::ec_land::EcLandPackage) {
    let populated_slot_count = package
        .slots()
        .iter()
        .filter(|slot| slot.is_present())
        .count();
    let alias_ref_count = package.terrain_provenance().len();
    let slot_table_len = package.slots().len();

    lg(&format!(
        "Enhanced land package coverage: {populated_slot_count} populated terrain ids from {alias_ref_count} TerrainDefinition alias refs (slot table span: 0..{}). Unmapped terrain ids fall back to classic texmaps.mul.",
        slot_table_len.saturating_sub(1),
    ));
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
    let uo_path: PathBuf = settings.uo_files.folder.clone().into();
    let udd_path: PathBuf = settings
        .uo_files
        .udd_path
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| uo_path.clone());
    let lossy = settings.graphics.lossy_texture_compression;

    lg("Start loading UO Data.");
    lg(&format!(
        "Resolved source roots with priority uddp > mul > uop. Raw client root: '{}'. UDDP root: '{}'.",
        uo_path.display(),
        udd_path.display()
    ));

    let mut map_planes: Vec<Option<map::MapPlane>> = std::iter::repeat_with(|| None)
        .take((MAX_MAP_INDEX + 1) as usize)
        .collect::<Vec<_>>();
    let mut statics_stores: Vec<Option<Arc<uocf::classic::statics::StaticsStore>>> = std::iter::repeat_with(|| None)
        .take((MAX_MAP_INDEX + 1) as usize)
        .collect::<Vec<_>>();
        
    for map_plane_index in 0..=MAX_MAP_INDEX {
        if map_plane_index != settings.core.world.start_p.m as u32 {
            continue;
        }
        
        let map_file = uo_path.join(format!("map{map_plane_index}.mul"));
        if map_file.exists() {
            log_source_choice(
                &lg,
                &format!("map plane {map_plane_index}"),
                SourceContainerKind::Mul,
                std::slice::from_ref(&map_file),
            );
            let map_size_override =
                settings
                    .maps
                    .map_size(map_plane_index)
                    .map(|map_size| map::MapSizeCells {
                        width: map_size.width,
                        height: map_size.height,
                    });
            let map_plane =
                map::MapPlane::init_with_size(map_file, map_plane_index, map_size_override)
                    .unwrap_or_else(|_| panic!("Error initializing map plane {map_plane_index}"));
                    
            let statics_idx_file = uo_path.join(format!("staidx{map_plane_index}.mul"));
            let statics_file = uo_path.join(format!("statics{map_plane_index}.mul"));
            if statics_idx_file.exists() && statics_file.exists() {
                log_source_choice(
                    &lg,
                    &format!("statics plane {map_plane_index}"),
                    SourceContainerKind::Mul,
                    &[statics_idx_file.clone(), statics_file.clone()],
                );
                let mut reader = uocf::classic::statics::StaticsReader::new(
                    &statics_idx_file,
                    &statics_file,
                    map_plane.size_blocks.width * 8,
                    map_plane.size_blocks.height * 8,
                ).unwrap_or_else(|_| panic!("Error initializing statics reader for plane {map_plane_index}"));
                
                let store = reader.load_all()
                    .unwrap_or_else(|_| panic!("Error loading statics for plane {map_plane_index}"));
                statics_stores[map_plane_index as usize] = Some(Arc::new(store));
            }
                    
            map_planes[map_plane_index as usize] = Some(map_plane);
        }
    }

    let tilemeta_path = resolve_optional_uddp_paths(&udd_path, &uo_path, &["tilemeta.uddp"]);
    let tilemeta_package = if let Some(tilemeta_path) = tilemeta_path {
        log_source_choice(
            &lg,
            "tile metadata",
            SourceContainerKind::Uddp,
            std::slice::from_ref(&tilemeta_path),
        );
        Some(Arc::new(
            TileMetaPackage::load(&tilemeta_path)
                .unwrap_or_else(|_| panic!("Error loading {}", tilemeta_path.display())),
        ))
    } else {
        lg("No tilemeta package source selected: tilemeta.uddp not found in udd_path or raw client folder.");
        None
    };

    let tiledata = if tilemeta_package.is_none() {
        let tiledata_path = uo_path.join("tiledata.mul");
        log_source_choice(
            &lg,
            "tiledata fallback",
            SourceContainerKind::Mul,
            std::slice::from_ref(&tiledata_path),
        );
        Some(Arc::new(
            tiledata::TileData::load(tiledata_path).expect("Load tiledata"),
        ))
    } else {
        None
    };

    let texmaps_path = uo_path.join("texmaps.mul");
    let texidx_path = uo_path.join("texidx.mul");
    log_source_choice(
        &lg,
        "terrain texmaps",
        SourceContainerKind::Mul,
        &[texmaps_path.clone(), texidx_path.clone()],
    );
    let texmap_2d = land_texture::TexMap::load(texmaps_path, texidx_path).expect("Load texmap");

    let cc_art_path = resolve_optional_uddp_path(&udd_path, &uo_path, "cc_art.uddp");
    let cc_art_package = if let Some(cc_art_path) = cc_art_path {
        log_source_choice(
            &lg,
            "classic art package",
            SourceContainerKind::Uddp,
            std::slice::from_ref(&cc_art_path),
        );
        Some(
            uddconv::cc_art::CcArtPackage::load(&cc_art_path)
                .unwrap_or_else(|_| panic!("Error loading {}", cc_art_path.display())),
        )
    } else {
        lg("No classic art package source selected: cc_art.uddp not found in udd_path or raw client folder.");
        None
    };

    let ec_art_path = resolve_optional_uddp_path(&udd_path, &uo_path, "ec_art.uddp");
    let ec_art_package = if let Some(ec_art_path) = ec_art_path {
        log_source_choice(
            &lg,
            "enhanced art package",
            SourceContainerKind::Uddp,
            std::slice::from_ref(&ec_art_path),
        );
        Some(
            uddconv::ec_art::EcArtPackage::load(&ec_art_path)
                .unwrap_or_else(|_| panic!("Error loading {}", ec_art_path.display())),
        )
    } else {
        lg("No enhanced art package source selected: ec_art.uddp not found in udd_path or raw client folder.");
        None
    };

    let ec_land_path = resolve_optional_uddp_path(&udd_path, &uo_path, "ec_land.uddp");
    let ec_land_package = if let Some(ec_land_path) = ec_land_path {
        log_source_choice(
            &lg,
            "enhanced land package",
            SourceContainerKind::Uddp,
            std::slice::from_ref(&ec_land_path),
        );
        let package = uddconv::ec_land::EcLandPackage::load(&ec_land_path)
            .unwrap_or_else(|_| panic!("Error loading {}", ec_land_path.display()));
        log_ec_land_coverage(&lg, &package);
        Some(package)
    } else {
        lg("No enhanced land package source selected: ec_land.uddp not found in udd_path or raw client folder.");
        None
    };

    let radarcol_path = uo_path.join("radarcol.mul");
    let radarcol = if radarcol_path.exists() {
        log_source_choice(
            &lg,
            "radar colors",
            SourceContainerKind::Mul,
            std::slice::from_ref(&radarcol_path),
        );
        Some(uocf::classic::radarcol::load_radarcol(&radarcol_path).expect("Load radarcol"))
    } else {
        lg("radarcol.mul not found");
        None
    };

    lg("Done loading UO Data.");

    // Load CC-EC conversion tables from KDL
    let transcode_path = Path::new("assets/cc_ec_convtables/TerrainTranscode.kdl");
    if transcode_path.exists() {
        match uddconv::cc_ec_conv::TerrainTranscode::load(transcode_path) {
            Ok(transcode) => {
                lg("Loaded TerrainTranscode.kdl");
                commands.insert_resource(TerrainTranscodeRes(Arc::new(transcode.to_map())));
            }
            Err(e) => {
                bevy::log::error!("Failed to load TerrainTranscode.kdl: {e}");
            }
        }
    }

    let definition_path = Path::new("assets/cc_ec_convtables/TerrainDefinition.kdl");
    if definition_path.exists() {
        match uddconv::cc_ec_conv::TerrainDefinitionKdl::load(definition_path) {
            Ok(definition) => {
                lg("Loaded TerrainDefinition.kdl");
                commands.insert_resource(TerrainDefinitionRes(Arc::new(definition.to_map())));
            }
            Err(e) => {
                bevy::log::error!("Failed to load TerrainDefinition.kdl: {e}");
            }
        }
    }

    commands.insert_resource(UoFilesSettingsRes(Arc::new(UoFilesSettings {
        base_folder: uo_path,
        udd_folder: udd_path,
    })));
    commands.insert_resource(MapPlanesRes(map_planes));
    if let Some(tilemeta_package) = tilemeta_package.clone() {
        commands.insert_resource(TileMetaPackageRes(tilemeta_package));
    }
    if let Some(tiledata) = tiledata {
        commands.insert_resource(TileDataRes(tiledata));
    }
    commands.insert_resource(TexMap2DRes(Arc::new(texmap_2d)));
    if let Some(cc_art_package) = cc_art_package {
        commands.insert_resource(CcArtPackageRes(Arc::new(cc_art_package)));
    }
    if let Some(ec_art_package) = ec_art_package {
        commands.insert_resource(EcArtPackageRes(Arc::new(ec_art_package)));
    }
    if let Some(ec_land_package) = ec_land_package {
        commands.insert_resource(EcLandPackageRes(Arc::new(ec_land_package)));
    }
    commands.insert_resource(StaticsStoreRes(statics_stores));
    if let Some(radarcol) = radarcol {
        commands.insert_resource(RadarColRes(Arc::new(radarcol)));
    }

    if settings.graphics.art_texture_source == crate::configs::settings::ClientTextureSource::Ec
        && tilemeta_package.is_none()
    {
        console_logger::one(
            console_logger::LogSev::Warn,
            console_logger::LogAbout::UoFiles,
            "EC art graphics requested, but tilemeta.uddp is unavailable. EC art retrieval will be disabled until tilemeta is loaded.",
        );
    }
}
