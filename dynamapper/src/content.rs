pub mod gumps;
pub mod land_routes;
pub mod land_texture_variation;

use crate::settings::{ClientTextureSource, Settings};
use crate::world::maps::MapPlane;
use crate::world::multis::MultiDefinitionsRes;
use crate::world::statics::{LazyStaticsStore, StaticsStoreRes};
use crate::app::system_sets::StartupSysSet;
use gumps::{ClassicGumpSource, GumpMapRes, GUMPS_CC_FILE_NAME, GUMPS_EC_FILE_NAME};
use crate::prelude::*;
use bevy::prelude::*;
use parking_lot::Mutex;
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

/// Preferred runtime metadata package for static/item rendering: the Enhanced tables (`0`), which the tilemeta
/// overrides (`TilemetaOverrides.kdl`) correct, and the tables of the items drawn with Classic art (`for_art`).
#[derive(Resource)]
pub struct TileMetaPackageRes(pub Arc<TileMetaPackage>, Arc<TileMetaPackage>);

impl TileMetaPackageRes {
    /// `package`, with its tables as the clients' files define them (`TileMetaPackage::without_overrides`) for the
    /// items drawn with Classic art; the same package when it was built without the overrides.
    pub fn new(package: Arc<TileMetaPackage>) -> color_eyre::eyre::Result<Self> {
        let cc_art = package.without_overrides()?.map_or_else(|| package.clone(), Arc::new);
        Ok(Self(package, cc_art))
    }

    /// The tables an item drawn from `art` is drawn by: the overrides were measured against the Enhanced and Kingdom
    /// Reborn art, so an item drawn with Classic art keeps the clients' flags and lights. The Enhanced tables without
    /// an art source (the dot mode).
    pub fn for_art(&self, art: Option<ClientTextureSource>) -> &Arc<TileMetaPackage> {
        match art {
            Some(ClientTextureSource::Cc) => &self.1,
            Some(ClientTextureSource::Ec) | None => &self.0,
        }
    }

    /// The tables the world analyses read, the z-view (`z_view.rs` `sys_update_z_view_limit`) and the caves
    /// (`caves.rs`): the overridden Enhanced tables, whatever the art source. An open item (STYLE_RULES §7): with
    /// Classic art they still see, say, a swamp piece's restored Surface, which the items drawn keep without; no
    /// visible effect has been found. Test `the_world_analyses_read_the_overridden_tables_whatever_the_art`.
    pub fn for_world_analyses(&self) -> &Arc<TileMetaPackage> {
        &self.0
    }
}

/// Arc: passed to the texture cache systems that warm pixel data off-thread.
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

/// The Kingdom Reborn land tex_land_ec.uddp holds (its Kingdom Reborn section), in the EC land layout. Only the
/// terrain draws it, when the Kingdom Reborn land source is selected; EC art keeps reading [`TexLandEcPackageRes`].
#[derive(Resource)]
pub struct TexLandKrPackageRes(pub Arc<udd_assets::tex_land_ec::TexLandEcPackage>);

/// Optional auxiliary texture package: the non-primary tileart textures that item liquids and item surfaces sample.
#[derive(Resource)]
pub struct TexAuxEcPackageRes(pub Arc<udd_assets::tex_aux_ec::TexAuxEcPackage>);

/// The Kingdom Reborn items the Enhanced packages hold (their Kingdom Reborn sections), in the Enhanced layouts: KR's
/// tileart (in `tilemeta.uddp`), and the item art (in `tex_art_ec.uddp`) and material textures (in `tex_aux_ec.uddp`)
/// KR draws differently from the Enhanced Client, overlays over the packages holding them, which the others are read
/// from. Under the KR land the statics that continue the land are drawn from them
/// (`static_source::StaticLandStyle::KingdomReborn`).
#[derive(Resource, Clone)]
pub struct KrItemPackagesRes {
    pub tilemeta: Arc<TileMetaPackage>,
    pub art: Arc<udd_assets::tex_art_ec::TexArtEcPackage>,
    pub aux: Arc<udd_assets::tex_aux_ec::TexAuxEcPackage>,
}

/// The earlier standalone Kingdom Reborn packages, whose data the Enhanced packages hold now.
const OBSOLETE_KR_PACKAGE_FILE_NAMES: [&str; 4] =
    ["tex_land_kr.uddp", "tilemeta_kr.uddp", "tex_art_kr.uddp", "tex_aux_kr.uddp"];
/// How to get the Kingdom Reborn sections into the Enhanced packages.
const KR_SECTION_REPACK: &str = "Repack tex_art_ec.uddp, tex_land_ec.uddp, tilemeta.uddp and tex_aux_ec.uddp with the \
    current udd-cli (pack-ec-textures, pack-ec-tex-aux)";

/// Loads [`KrItemPackagesRes`] from the Kingdom Reborn sections of the loaded `tilemeta`, `art` and `aux`, read in
/// place. A section that is missing, or a package that is not loaded, is an error naming the repack when the KR land
/// is the selected source, whose land-continuing statics then keep the Enhanced item data; a section that does not
/// read is always one.
fn load_kr_item_packages(lg: &impl Fn(&str), lg_err: &impl Fn(&str), kr_land_selected: bool, tilemeta: &TileMetaPackage,
    art: Option<&udd_assets::tex_art_ec::TexArtEcPackage>, aux: Option<&udd_assets::tex_aux_ec::TexAuxEcPackage>)
    -> Option<KrItemPackagesRes> {
    let loaded = (|| -> color_eyre::eyre::Result<Option<KrItemPackagesRes>> {
        let (Some(art), Some(aux)) = (art, aux) else { return Ok(None) };
        let (Some(tilemeta), Some(art), Some(aux)) =
            (tilemeta.kr_tilemeta()?, art.kr_overlay(AtlasCacheOptions::disabled())?, aux.kr_overlay()?)
        else {
            return Ok(None);
        };
        Ok(Some(KrItemPackagesRes { tilemeta: Arc::new(tilemeta), art: Arc::new(art), aux: Arc::new(aux) }))
    })();
    match loaded {
        Ok(Some(packages)) => {
            lg("Kingdom Reborn items: the Kingdom Reborn sections of tilemeta.uddp, tex_art_ec.uddp and tex_aux_ec.uddp.");
            Some(packages)
        }
        Ok(None) => {
            let message = format!("No Kingdom Reborn items: tilemeta.uddp, tex_art_ec.uddp and tex_aux_ec.uddp do not \
                all hold a Kingdom Reborn section. Under the Kingdom Reborn land, the statics that continue the land \
                keep the Enhanced item data. {KR_SECTION_REPACK}.");
            if kr_land_selected { lg_err(&message) } else { lg(&message) }
            None
        }
        Err(error) => {
            lg_err(&format!("Kingdom Reborn items not loaded: {error:#}. {KR_SECTION_REPACK}."));
            None
        }
    }
}

/// The Kingdom Reborn land `tex_land_ec` holds, read in place with `reader_options`; `None`, logged, when it holds
/// none or it does not read.
fn load_kr_land(lg: &impl Fn(&str), lg_err: &impl Fn(&str), tex_land_ec: &udd_assets::tex_land_ec::TexLandEcPackage,
    reader_options: UddpReaderOptions) -> Option<udd_assets::tex_land_ec::TexLandEcPackage> {
    match tex_land_ec.kr_land(reader_options, AtlasCacheOptions::enabled()) {
        Ok(Some(package)) => {
            log_tex_land_ec_coverage(lg, &package, "Kingdom Reborn", "tex_land_ec.uddp (Kingdom Reborn section)");
            Some(package)
        }
        Ok(None) => {
            lg(&format!("tex_land_ec.uddp holds no Kingdom Reborn land. {KR_SECTION_REPACK}."));
            None
        }
        Err(error) => {
            lg_err(&format!("The Kingdom Reborn land of tex_land_ec.uddp not loaded: {error:#}. {KR_SECTION_REPACK}."));
            None
        }
    }
}

/// Warns about the earlier standalone Kingdom Reborn packages in `udd_path`, which are not read.
fn warn_obsolete_kr_packages(lg_warn: &impl Fn(&str), udd_path: &Path) {
    let found: Vec<&str> = OBSOLETE_KR_PACKAGE_FILE_NAMES.into_iter()
        .filter(|name| resolve_optional_uddp_path(udd_path, name).is_some())
        .collect();
    if !found.is_empty() {
        lg_warn(&format!("{} in udd_path: an earlier layout, not read; the Enhanced packages hold the Kingdom Reborn \
            data now. {KR_SECTION_REPACK} if they lack it, and remove these.", found.join(", ")));
    }
}

/// Optional prepacked world light mask package from light.mul/lightidx.mul./// Optional prepacked world light mask package from light.mul/lightidx.mul.
#[derive(Resource)]
pub struct WorldLightsPackageRes(pub Arc<udd_assets::world_lights::WorldLightsPackage>);

/// Optional frames of the Enhanced client's 3D paperdoll, which the Enhanced "Gumps and paperdoll" preference draws.
/// Without it, that preference draws the gump paperdoll.
#[derive(Resource)]
pub struct PaperdollEcPackageRes(pub Arc<udd_assets::PaperdollEcPackage>);

const PAPERDOLL_EC_FILE_NAME: &str = "paperdoll_ec.uddp";

/// Optional Classic Client bitmap fonts.
#[derive(Resource)]
#[allow(dead_code)] // Loaded ahead of the Classic text rendering; nothing draws with it yet.
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
}

impl SourceContainerKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Uddp => "uddp",
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

/// Loads an optional land package in the EC land layout (tex_land_ec.uddp) and logs its coverage;
/// `None` when the file is not in `udd_path`.
fn load_optional_enhanced_land_package(
    lg: &impl Fn(&str),
    udd_path: &Path,
    file_name: &str,
    label: &str,
    reader_options: UddpReaderOptions,
) -> Option<udd_assets::tex_land_ec::TexLandEcPackage> {
    let path = resolve_optional_uddp_path(udd_path, file_name)?;
    log_source_choice(
        lg,
        &format!("{} land package", label.to_lowercase()),
        SourceContainerKind::Uddp,
        std::slice::from_ref(&path),
    );
    let package = udd_assets::tex_land_ec::TexLandEcPackage::from_uddp_package_with_options(
        UddpReader::load_with_options(&path, reader_options)
            .unwrap_or_else(|_| panic!("Error loading {}", path.display())),
        AtlasCacheOptions::enabled(),
    )
    .unwrap_or_else(|error| panic!("Error loading {}: {error:#}", path.display()));
    log_tex_land_ec_coverage(lg, &package, label, file_name);
    Some(package)
}

fn log_tex_land_ec_coverage(
    lg: &impl Fn(&str),
    package: &udd_assets::tex_land_ec::TexLandEcPackage,
    label: &str,
    file_name: &str,
) {
    let populated_slot_count = package
        .slots()
        .iter()
        .filter(|slot| slot.is_present())
        .count();
    let alias_ref_count = package.terrain_provenance().len();
    let slot_table_len = package.slots().len();

    lg(&format!(
        "{label} land package coverage: {populated_slot_count} populated slots from {alias_ref_count} TerrainDefinition alias refs (slot table span: 0..{}). Land ids it lacks are drawn from the next land source of the chain, or with the missing-texture pattern.",
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
            "Embedded EcTerrainOverrides.kdl references {} texture(s) that are not packed in {file_name}. Falling back where needed. Sample: {}",
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
    let lg_warn = |text: &str| {
        console_logger::one(
            console_logger::LogSev::Warn,
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
                    map_plane_index,
                    map_plane.size_blocks.width * 8,
                    map_plane.size_blocks.height * 8,
                )
                .unwrap_or_else(|error| {
                    panic!("Error initializing statics reader for plane {map_plane_index}: {error:#}")
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

    let tex_land_ec_package =
        load_optional_enhanced_land_package(&lg, &udd_path, "tex_land_ec.uddp", "Enhanced", land_atlas_reader_options);
    if tex_land_ec_package.is_none() {
        lg_err("No enhanced land package source selected: tex_land_ec.uddp not found in udd_path.");
    }
    warn_obsolete_kr_packages(&lg_warn, &udd_path);
    // The Kingdom Reborn land is optional: its absence is an error only when it is the selected land source, which
    // then draws from the next source of its chain, tex_land_ec.uddp or Classic (texture_cache/land/routing.rs).
    let kr_land_selected = settings.graphics.land_texture_source == crate::settings::LandTextureSource::Kr;
    let tex_land_kr_package = tex_land_ec_package.as_ref().and_then(|tex_land_ec| {
        load_kr_land(&lg, &|text: &str| if kr_land_selected { lg_err(text) } else { lg(text) }, tex_land_ec,
            land_atlas_reader_options)
    });
    if tex_land_kr_package.is_none() && kr_land_selected {
        lg_err("No Kingdom Reborn land: the selected land source draws from the next one of its chain.");
    }

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

    let kr_item_packages = load_kr_item_packages(&lg, &lg_err, kr_land_selected, &tilemeta_package,
        tex_art_ec_package.as_ref(), tex_aux_ec_package.as_ref());

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
                .unwrap_or_else(|error| panic!("Error loading {}: {error:#}", world_lights_path.display())),
        )
    } else {
        lg("No world light mask package selected: world_lights.uddp not found in udd_path.");
        None
    };

    let load_classic_gumps = || match uocf::classic::gump::GumpMap::load(&udd_path) {
        Ok(gump_map) => {
            lg("Loaded Classic Client gump art source.");
            Some(ClassicGumpSource::ClientFiles(gump_map))
        }
        Err(error) => {
            lg_err(&format!(
                "No Classic gump source selected from {}: {error}",
                udd_path.display()
            ));
            None
        }
    };
    let load_gump_package = |file_name: &str, logical_name: &str| {
        let path = resolve_optional_uddp_path(&udd_path, file_name)?;
        log_source_choice(&lg, logical_name, SourceContainerKind::Uddp, std::slice::from_ref(&path));
        udd_assets::GumpsPackage::load(&path)
            .map_err(|error| lg_err(&format!("Failed to load gump package {}: {error}", path.display())))
            .ok()
    };
    let gump_map = if !settings.runtime_assets.load_gumps {
        lg("Gump art loading is disabled by runtime_assets.load_gumps.");
        None
    } else {
        // The Classic client's gump files stand in only for a missing or unreadable gumps_cc.uddp.
        let classic = load_gump_package(GUMPS_CC_FILE_NAME, "Classic gumps")
            .map(ClassicGumpSource::Package)
            .or_else(load_classic_gumps);
        let enhanced = load_gump_package(GUMPS_EC_FILE_NAME, "Enhanced gumps");
        let gump_map = GumpMapRes::new(classic, enhanced);
        if let Some(gump_map) = &gump_map {
            lg(&gump_map.sources_summary());
            if let Some(warning) = gump_map.shared_gumps_warning() {
                console_logger::one(console_logger::LogSev::Warn, console_logger::LogAbout::UoFiles, &warning);
            }
        }
        gump_map
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

    let paperdoll_ec_package = match resolve_optional_uddp_path(&udd_path, PAPERDOLL_EC_FILE_NAME) {
        Some(path) => {
            log_source_choice(&lg, "Enhanced paperdoll", SourceContainerKind::Uddp, std::slice::from_ref(&path));
            udd_assets::PaperdollEcPackage::load(&path)
                .map_err(|error| {
                    lg_err(&format!(
                        "Failed to load {}: {error:#}. Enhanced gumps and paperdoll draw the gump paperdoll.",
                        path.display()
                    ))
                })
                .ok()
        }
        None => {
            lg(&format!(
                "No Enhanced paperdoll package selected: {PAPERDOLL_EC_FILE_NAME} not found in udd_path. Enhanced \
                 gumps and paperdoll draw the gump paperdoll."
            ));
            None
        }
    };

    lg("Done loading UO Data.");

    // The CC-EC land routing table (TerrainTranscode.kdl) and the terrain overrides (EcTerrainOverrides.kdl) are
    // embedded in tex_land_ec.uddp, which applies them.

    let multi_definitions = load_multi_definitions(&udd_path, &lg, &lg_err);

    commands.insert_resource(UoFilesSettingsRes(Arc::new(UoFilesSettings {
        udd_folder: udd_path,
    })));
    commands.insert_resource(MapPlanesRes(map_planes));
    commands.insert_resource(TileMetaPackageRes::new(tilemeta_package.clone())
        .unwrap_or_else(|error| panic!("Error loading the tilemeta overrides of {}: {error:#}", tilemeta_path.display())));
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
    if let Some(tex_land_kr_package) = tex_land_kr_package {
        commands.insert_resource(TexLandKrPackageRes(Arc::new(tex_land_kr_package)));
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
    if let Some(paperdoll_ec_package) = paperdoll_ec_package {
        commands.insert_resource(PaperdollEcPackageRes(Arc::new(paperdoll_ec_package)));
    }
    if let Some(multis) = multi_definitions {
        commands.insert_resource(multis);
    }
    if let Some(tex_aux_ec_package) = tex_aux_ec_package {
        commands.insert_resource(TexAuxEcPackageRes(Arc::new(tex_aux_ec_package)));
    }
    if let Some(kr_item_packages) = kr_item_packages {
        commands.insert_resource(kr_item_packages);
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

    /// The canonical Enhanced packages hold the Kingdom Reborn sections, which load in place.
    #[test]
    #[ignore = "requires current canonical packages in DYNAMAPPER_TEST_PACKAGES"]
    fn canonical_enhanced_packages_hold_the_kingdom_reborn_sections() {
        use super::{load_kr_item_packages, load_kr_land, TileMetaPackage, UddpReaderOptions};
        use std::cell::RefCell;
        let root = PathBuf::from(std::env::var_os("DYNAMAPPER_TEST_PACKAGES")
            .expect("set DYNAMAPPER_TEST_PACKAGES to the matching current package directory"));
        let tilemeta = TileMetaPackage::load(root.join("tilemeta.uddp")).unwrap();
        let art = udd_assets::tex_art_ec::TexArtEcPackage::load(root.join("tex_art_ec.uddp")).unwrap();
        let aux = udd_assets::tex_aux_ec::TexAuxEcPackage::load(root.join("tex_aux_ec.uddp")).unwrap();
        let land = udd_assets::tex_land_ec::TexLandEcPackage::load(root.join("tex_land_ec.uddp")).unwrap();
        let errors = RefCell::new(Vec::new());
        let lg_err = |text: &str| errors.borrow_mut().push(text.to_owned());

        let items = load_kr_item_packages(&|_: &str| {}, &lg_err, true, &tilemeta, Some(&art), Some(&aux))
            .expect("the Kingdom Reborn items");
        assert!(items.art.is_overlay() && items.aux.is_overlay());
        let kr_land = load_kr_land(&|_: &str| {}, &lg_err, &land, UddpReaderOptions::disabled()).expect("the KR land");
        // The textures it leaves to the Enhanced land read from that package's pages, appended after its own.
        assert!(kr_land.is_overlay() && !kr_land.equivalents().is_empty());
        assert!(kr_land.pages().len() as u32 > kr_land.own_page_count());
        let last_page = kr_land.pages().len() as u32 - 1;
        assert!(kr_land.base_page_index(last_page).is_some() && kr_land.read_page_bytes(last_page).is_ok());
        assert!(load_kr_item_packages(&|_: &str| {}, &lg_err, true, &tilemeta, None, Some(&aux)).is_none());
        let errors = errors.into_inner();
        assert!(errors.len() == 1 && errors[0].contains("do not all hold a Kingdom Reborn section"), "{errors:?}");
    }
}
