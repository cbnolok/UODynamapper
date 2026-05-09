#![allow(unused)]

use crate::configs::settings::Settings;
use crate::core::maps::MapPlane;
use crate::core::statics::{LazyStaticsStore, StaticsStoreRes};
use crate::core::system_sets::StartupSysSet;
use crate::prelude::*;
use bevy::prelude::*;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uddconv::tilemeta::TileMetaPackage;
use uocf::classic::{land_texture, map::MapSizeCells};

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
pub struct TerrainDefinitionRes(
    pub Arc<HashMap<u32, uddconv::cc_ec_land_transcode::TerrainDefEntry>>,
);

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
        "Resolved source roots. Raw client root: '{}'. UDDP root: '{}'.",
        uo_path.display(),
        udd_path.display()
    ));

    let mut map_planes: Vec<Option<MapPlane>> = std::iter::repeat_with(|| None)
        .take((MAX_MAP_INDEX + 1) as usize)
        .collect::<Vec<_>>();
    let mut statics_stores: Vec<Option<Mutex<LazyStaticsStore>>> = std::iter::repeat_with(|| None)
        .take((MAX_MAP_INDEX + 1) as usize)
        .collect::<Vec<_>>();

    for map_plane_index in 0..=MAX_MAP_INDEX {
        if map_plane_index != settings.core.world.start_p.m as u32 {
            continue;
        }

        let map_file_name = format!("map{map_plane_index}.uddp");
        let map_path = resolve_optional_uddp_path(&udd_path, &uo_path, &map_file_name);
        if let Some(map_path) = map_path {
            log_source_choice(
                &lg,
                &format!("map plane {map_plane_index}"),
                SourceContainerKind::Uddp,
                std::slice::from_ref(&map_path),
            );
            let map_size_override =
                settings
                    .maps
                    .map_size(map_plane_index)
                    .map(|map_size| MapSizeCells {
                        width: map_size.width,
                        height: map_size.height,
                    });
            let map_plane = MapPlane::load(map_path.clone(), map_plane_index, map_size_override)
                .unwrap_or_else(|_| panic!("Error initializing map plane {map_plane_index}"));

            let statics_file_name = format!("statics{map_plane_index}.uddp");
            let statics_path = resolve_optional_uddp_path(&udd_path, &uo_path, &statics_file_name);
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
                lg(&format!(
                    "No statics source selected for plane {map_plane_index}: {statics_file_name} not found in udd_path or raw client folder."
                ));
            }

            map_planes[map_plane_index as usize] = Some(map_plane);
        } else {
            lg(&format!(
                "No map source selected for plane {map_plane_index}: {map_file_name} not found in udd_path or raw client folder."
            ));
        }
    }

    let tilemeta_path = resolve_optional_uddp_paths(&udd_path, &uo_path, &["tilemeta.uddp"])
        .unwrap_or_else(|| panic!("tilemeta.uddp is required in udd_path or raw client folder"));
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
    let mut ec_land_package = if let Some(ec_land_path) = ec_land_path {
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

    lg("Done loading UO Data.");

    // Load CC-EC conversion tables from KDL
    let asset_root = crate::core::constants::valid_asset_dir();
    let transcode_path = asset_root.join("cc_ec_convtables/TerrainTranscode.kdl");

    if transcode_path.exists() {
        lg(&format!(
            "Loading TerrainTranscode.kdl from: {}",
            transcode_path.display()
        ));
        match uddconv::cc_ec_land_transcode::TerrainTranscode::load(&transcode_path) {
            Ok(transcode) => {
                lg("Loaded TerrainTranscode.kdl (loose file)");
                let transcode_map = transcode.to_map();

                // Apply override to EC land package if present
                if let Some(ec_land) = ec_land_package.as_mut() {
                    lg("Applying loose TerrainTranscode.kdl as override to EC land package.");
                    ec_land.set_transcode(transcode_map.clone());
                }

                commands.insert_resource(TerrainTranscodeRes(Arc::new(transcode_map)));
            }
            Err(e) => {
                bevy::log::error!("Failed to load TerrainTranscode.kdl: {e}");
            }
        }
    } else {
        lg(&format!(
            "TerrainTranscode.kdl not found at {}",
            transcode_path.display()
        ));
    }

    let definition_path = asset_root.join("cc_ec_convtables/TerrainDefinition.kdl");
    if definition_path.exists() {
        match uddconv::cc_ec_land_transcode::TerrainDefinitionKdl::load(&definition_path) {
            Ok(definition) => {
                lg("Loaded TerrainDefinition.kdl");
                commands.insert_resource(TerrainDefinitionRes(Arc::new(definition.to_map())));
            }
            Err(e) => {
                bevy::log::error!("Failed to load TerrainDefinition.kdl: {e}");
            }
        }
    } else {
        lg(&format!(
            "TerrainDefinition.kdl not found at {}",
            definition_path.display()
        ));
    }

    commands.insert_resource(UoFilesSettingsRes(Arc::new(UoFilesSettings {
        base_folder: uo_path,
        udd_folder: udd_path,
    })));
    commands.insert_resource(MapPlanesRes(map_planes));
    commands.insert_resource(TileMetaPackageRes(tilemeta_package.clone()));
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
}
