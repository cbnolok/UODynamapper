#![allow(unused)]

use crate::configs::settings::Settings;
use crate::core::maps::MapPlane;
use crate::core::multis::MultiDefinitionsRes;
use crate::core::statics::{LazyStaticsStore, StaticsStoreRes};
use crate::core::system_sets::StartupSysSet;
use crate::prelude::*;
use bevy::prelude::*;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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

const GUMPS_CC_FILE_NAME: &str = "gumps_cc.uddp";
const GUMPS_EC_FILE_NAME: &str = "gumps_ec.uddp";
const CLASSIC_CLIENT_GUMPS_LABEL: &str = "the Classic client gump files";
/// `UniformState.effects.shading_mode` of the Classic preset. Enhanced is 1 and KR-like 2.
const CLASSIC_SHADING_MODE: u32 = 0;

/// Which client's gumps a rendering preset draws (the user, 2026-11-06): the Classic preset the Classic client's, and
/// the Enhanced and KR-like presets, which render from Enhanced data, the Enhanced client's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GumpClient {
    Classic,
    Enhanced,
}

impl GumpClient {
    /// The preset is `UniformState.effects.shading_mode`, which the shader dialog changes at runtime.
    pub const fn from_shading_mode(shading_mode: u32) -> Self {
        if shading_mode == CLASSIC_SHADING_MODE {
            Self::Classic
        } else {
            Self::Enhanced
        }
    }
}

/// Which loaded source `pick_gump_source` read a gump from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GumpSourceKind {
    Enhanced,
    Classic,
}

/// What `pick_gump_source` gives for one gump.
#[derive(Debug)]
enum GumpPick<T> {
    Found(GumpSourceKind, T),
    /// The source the rule picked has the gump but cannot decode it. No other source is tried.
    Failed(GumpSourceKind, color_eyre::eyre::Report),
    /// The Classic preset with no Classic source loaded: it shows no gumps.
    NoClassicSource,
    /// The Enhanced or KR-like preset, gumps_ec.uddp lacks the gump and no Classic source is loaded.
    Missing,
}

/// The source of one gump for the preset drawing `client` (the user, 2026-11-06). The Classic preset reads the Classic
/// source only, never gumps_ec.uddp. Enhanced and KR-like read gumps_ec.uddp when it has the gump, else the Classic
/// source; a gumps_ec.uddp entry that fails to decode is an error, not a reason to fall back.
///
/// `enhanced` is set when gumps_ec.uddp is loaded and looks the gump up there: `None` when the package lacks it, else
/// its decode. `classic` is set when a Classic source is loaded and decodes the gump from it.
fn pick_gump_source<T>(
    client: GumpClient,
    enhanced: Option<impl FnOnce() -> Option<color_eyre::eyre::Result<T>>>,
    classic: Option<impl FnOnce() -> color_eyre::eyre::Result<T>>,
) -> GumpPick<T> {
    if client == GumpClient::Enhanced {
        if let Some(decoded) = enhanced.and_then(|lookup| lookup()) {
            return match decoded {
                Ok(gump) => GumpPick::Found(GumpSourceKind::Enhanced, gump),
                Err(error) => GumpPick::Failed(GumpSourceKind::Enhanced, error),
            };
        }
    }

    match (classic, client) {
        (Some(decode), _) => match decode() {
            Ok(gump) => GumpPick::Found(GumpSourceKind::Classic, gump),
            Err(error) => GumpPick::Failed(GumpSourceKind::Classic, error),
        },
        (None, GumpClient::Classic) => GumpPick::NoClassicSource,
        (None, GumpClient::Enhanced) => GumpPick::Missing,
    }
}

/// The startup line naming the source each preset draws its gumps from.
fn gump_sources_summary(classic_label: Option<&str>, enhanced_loaded: bool) -> String {
    let classic_preset = classic_label.unwrap_or("none, so it shows no gumps");
    let enhanced_presets = match (enhanced_loaded, classic_label) {
        (true, Some(classic_label)) => format!("{GUMPS_EC_FILE_NAME}, then {classic_label} for ids it lacks"),
        (true, None) => GUMPS_EC_FILE_NAME.to_string(),
        (false, Some(classic_label)) => format!("{classic_label} ({GUMPS_EC_FILE_NAME} is not loaded)"),
        (false, None) => "none".to_string(),
    };
    format!("gumps: Classic preset {classic_preset}; Enhanced and KR-like {enhanced_presets}")
}

/// The startup warning for a gumps_ec.uddp that leaves `shared_count` gumps for gumps_cc.uddp to draw, when no Classic
/// source is loaded to draw them.
fn shared_gumps_without_classic_warning(shared_count: usize, classic_loaded: bool) -> Option<String> {
    (shared_count > 0 && !classic_loaded).then(|| {
        format!(
            "{GUMPS_EC_FILE_NAME} shares {shared_count} gumps with {GUMPS_CC_FILE_NAME}, and neither that package nor \
             {CLASSIC_CLIENT_GUMPS_LABEL} are loaded: those gumps won't show under the Enhanced and KR-like presets."
        )
    })
}

/// What [`SharedGumpCheck::check`] found for one gump the Classic source served under the Enhanced presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SharedGumpVerdict {
    /// gumps_ec.uddp doesn't share the gump: it lacks it.
    NotShared,
    /// The gump was checked before.
    AlreadyChecked,
    Matches,
    /// The first shared gump the Classic source draws differently.
    FirstMismatch,
    /// A later one, `count` with the first.
    Mismatch { count: u32 },
}

/// Checks each gump gumps_ec.uddp shares with gumps_cc.uddp the first time the Classic source serves it under the
/// Enhanced presets: the image hash gumps_ec.uddp lists against the served image's. Each gump is hashed once.
#[derive(Default)]
struct SharedGumpCheck {
    checked: Mutex<HashSet<u32>>,
    mismatch_count: AtomicU32,
}

impl SharedGumpCheck {
    /// `listed_hash` is gumps_ec.uddp's hash of the gump, `None` when it doesn't share it.
    fn check(
        &self,
        gump_id: u32,
        listed_hash: Option<u64>,
        served: &udd_assets::gumps::GumpImage,
    ) -> SharedGumpVerdict {
        let Some(listed_hash) = listed_hash else {
            return SharedGumpVerdict::NotShared;
        };
        if !self.checked.lock().insert(gump_id) {
            return SharedGumpVerdict::AlreadyChecked;
        }
        if udd_assets::gumps::gump_image_hash(served) == listed_hash {
            return SharedGumpVerdict::Matches;
        }
        match self.mismatch_count.fetch_add(1, Ordering::Relaxed) + 1 {
            1 => SharedGumpVerdict::FirstMismatch,
            count => SharedGumpVerdict::Mismatch { count },
        }
    }
}

/// The Classic gump source: gumps_cc.uddp, or the Classic client's gump files when that package is absent or fails to
/// load.
pub enum ClassicGumpSource {
    Package(udd_assets::GumpsPackage),
    ClientFiles(uocf::classic::gump::GumpMap),
}

impl ClassicGumpSource {
    fn label(&self) -> &'static str {
        match self {
            Self::Package(_) => GUMPS_CC_FILE_NAME,
            Self::ClientFiles(_) => CLASSIC_CLIENT_GUMPS_LABEL,
        }
    }

    fn decode(
        &self,
        gump_id: u32,
        scratch: &mut Vec<u8>,
    ) -> color_eyre::eyre::Result<udd_assets::gumps::GumpImage> {
        let classic = match self {
            Self::Package(package) => return package.read_gump_image(gump_id),
            Self::ClientFiles(classic) => classic,
        };
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

/// gumps_ec.uddp with its gump ids, sorted, so a lookup knows when the Enhanced presets fall back.
struct EnhancedGumps {
    package: udd_assets::GumpsPackage,
    gump_ids: Vec<u32>,
}

impl EnhancedGumps {
    fn new(package: udd_assets::GumpsPackage) -> Self {
        let gump_ids = package.available_gump_ids();
        Self { package, gump_ids }
    }

    fn lookup(&self, gump_id: u32) -> Option<color_eyre::eyre::Result<udd_assets::gumps::GumpImage>> {
        self.gump_ids
            .binary_search(&gump_id)
            .is_ok()
            .then(|| self.package.read_gump_image(gump_id))
    }
}

/// Optional gump art source. UDDP packages are preferred, legacy Classic sources are fallback.
/// The active preset picks which source serves a gump (`GumpClient`, `pick_gump_source`).
#[derive(Resource)]
pub struct GumpMapRes {
    classic: Option<Arc<ClassicGumpSource>>,
    enhanced: Option<Arc<EnhancedGumps>>,
    /// Set once the Classic preset asked for a gump with no Classic source loaded, which is logged once.
    classic_missing_logged: AtomicBool,
    shared_check: SharedGumpCheck,
}

impl GumpMapRes {
    /// `None` when neither source is loaded.
    pub fn new(classic: Option<ClassicGumpSource>, enhanced: Option<udd_assets::GumpsPackage>) -> Option<Self> {
        if classic.is_none() && enhanced.is_none() {
            return None;
        }
        Some(Self {
            classic: classic.map(Arc::new),
            enhanced: enhanced.map(|package| Arc::new(EnhancedGumps::new(package))),
            classic_missing_logged: AtomicBool::new(false),
            shared_check: SharedGumpCheck::default(),
        })
    }

    fn sources_summary(&self) -> String {
        gump_sources_summary(self.classic.as_deref().map(ClassicGumpSource::label), self.enhanced.is_some())
    }

    fn is_shared_gump(&self, gump_id: u32) -> bool {
        self.enhanced.as_ref().is_some_and(|enhanced| enhanced.package.shared_classic_gump_hash(gump_id).is_some())
    }

    fn shared_gumps_warning(&self) -> Option<String> {
        let shared_count = self.enhanced.as_ref().map_or(0, |enhanced| enhanced.package.shared_classic_gump_count());
        shared_gumps_without_classic_warning(shared_count, self.classic.is_some())
    }

    /// Checks a gump the Classic source served under the Enhanced presets against the hash gumps_ec.uddp lists when it
    /// shares the gump. A mismatch still draws the gump.
    fn check_shared_gump(&self, gump_id: u32, served: &udd_assets::gumps::GumpImage) {
        let (Some(enhanced), Some(classic)) = (self.enhanced.as_deref(), self.classic.as_deref()) else {
            return;
        };
        let listed_hash = enhanced.package.shared_classic_gump_hash(gump_id);
        match self.shared_check.check(gump_id, listed_hash, served) {
            SharedGumpVerdict::NotShared | SharedGumpVerdict::AlreadyChecked | SharedGumpVerdict::Matches => {}
            SharedGumpVerdict::FirstMismatch => console_logger::one(
                console_logger::LogSev::Warn,
                console_logger::LogAbout::UoFiles,
                &format!(
                    "Gump {gump_id}, which {GUMPS_EC_FILE_NAME} shares, differs in {}: it is not the \
                     {GUMPS_CC_FILE_NAME} that {GUMPS_EC_FILE_NAME} was packed against. The gump is drawn anyway; \
                     repack {GUMPS_EC_FILE_NAME} against this {GUMPS_CC_FILE_NAME}, or standalone.",
                    classic.label()
                ),
            ),
            SharedGumpVerdict::Mismatch { count } => debug!(
                "Gump {gump_id}, which {GUMPS_EC_FILE_NAME} shares, differs in {}: {count} shared gumps differ so far.",
                classic.label()
            ),
        }
    }

    pub fn decode_gump(
        &self,
        client: GumpClient,
        gump_id: u32,
        scratch: &mut Vec<u8>,
    ) -> color_eyre::eyre::Result<udd_assets::gumps::GumpImage> {
        let enhanced = self.enhanced.as_deref().map(|enhanced| move || enhanced.lookup(gump_id));
        let classic = self.classic.as_deref().map(|classic| move || classic.decode(gump_id, scratch));
        match pick_gump_source(client, enhanced, classic) {
            GumpPick::Found(GumpSourceKind::Classic, gump) if client == GumpClient::Enhanced => {
                self.check_shared_gump(gump_id, &gump);
                Ok(gump)
            }
            GumpPick::Found(_, gump) => Ok(gump),
            GumpPick::Failed(GumpSourceKind::Enhanced, error) => {
                let error = error.wrap_err(format!("{GUMPS_EC_FILE_NAME} gump {gump_id} does not decode"));
                console_logger::one(
                    console_logger::LogSev::Error,
                    console_logger::LogAbout::UoFiles,
                    &format!("{error:#}"),
                );
                Err(error)
            }
            GumpPick::Failed(GumpSourceKind::Classic, error) => Err(error),
            GumpPick::NoClassicSource => {
                if !self.classic_missing_logged.swap(true, Ordering::Relaxed) {
                    console_logger::one(
                        console_logger::LogSev::Warn,
                        console_logger::LogAbout::UoFiles,
                        &format!(
                            "The Classic preset shows no gumps: neither {GUMPS_CC_FILE_NAME} nor {CLASSIC_CLIENT_GUMPS_LABEL} are loaded."
                        ),
                    );
                }
                Err(color_eyre::eyre::eyre!("the Classic preset has no gump source loaded"))
            }
            GumpPick::Missing if self.is_shared_gump(gump_id) => Err(color_eyre::eyre::eyre!(
                "gump {gump_id} is one {GUMPS_EC_FILE_NAME} shares with {GUMPS_CC_FILE_NAME}, and no Classic gump \
                 source is loaded"
            )),
            GumpPick::Missing => Err(color_eyre::eyre::eyre!(
                "gump {gump_id} is not in {GUMPS_EC_FILE_NAME} and no Classic gump source is loaded"
            )),
        }
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
    use super::{
        gump_sources_summary, pick_gump_source, shared_gumps_without_classic_warning, GumpClient, GumpPick,
        GumpSourceKind, SharedGumpCheck, SharedGumpVerdict, GUMPS_CC_FILE_NAME,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const CLASSIC_PRESET: u32 = 0;
    const ENHANCED_PRESET: u32 = 1;
    const KR_LIKE_PRESET: u32 = 2;
    const ENHANCED_IMAGE: &str = "gumps_ec image";
    const CLASSIC_IMAGE: &str = "Classic image";

    /// gumps_ec.uddp's state for the looked-up gump.
    #[derive(Clone, Copy, Debug)]
    enum Enhanced {
        NotLoaded,
        LacksGump,
        HasGump,
        DecodeError,
    }

    /// What `pick_gump_source` should give, without the error text.
    #[derive(Debug, PartialEq, Eq)]
    enum Expected {
        Found(GumpSourceKind, &'static str),
        Failed(GumpSourceKind),
        NoClassicSource,
        Missing,
    }

    /// What `pick` saw: the result, and whether each loaded source was asked.
    struct Picked {
        result: Expected,
        enhanced_asked: bool,
        classic_asked: bool,
    }

    /// Runs the rule with stand-in sources.
    fn pick(shading_mode: u32, enhanced: Enhanced, classic_loaded: bool, classic_decodes: bool) -> Picked {
        let mut enhanced_asked = false;
        let mut classic_asked = false;
        let enhanced_flag = &mut enhanced_asked;
        let enhanced_lookup = match enhanced {
            Enhanced::NotLoaded => None,
            loaded => Some(move || {
                *enhanced_flag = true;
                match loaded {
                    Enhanced::HasGump => Some(Ok(ENHANCED_IMAGE)),
                    Enhanced::DecodeError => Some(Err(color_eyre::eyre::eyre!("bad gumps_ec entry"))),
                    _ => None,
                }
            }),
        };
        let classic = classic_loaded.then_some(|| {
            classic_asked = true;
            if classic_decodes {
                Ok(CLASSIC_IMAGE)
            } else {
                Err(color_eyre::eyre::eyre!("bad Classic entry"))
            }
        });
        let result = match pick_gump_source(GumpClient::from_shading_mode(shading_mode), enhanced_lookup, classic) {
            GumpPick::Found(kind, image) => Expected::Found(kind, image),
            GumpPick::Failed(kind, _) => Expected::Failed(kind),
            GumpPick::NoClassicSource => Expected::NoClassicSource,
            GumpPick::Missing => Expected::Missing,
        };
        Picked { result, enhanced_asked, classic_asked }
    }

    #[test]
    fn presets_map_to_their_gump_client() {
        assert_eq!(GumpClient::from_shading_mode(CLASSIC_PRESET), GumpClient::Classic);
        assert_eq!(GumpClient::from_shading_mode(ENHANCED_PRESET), GumpClient::Enhanced);
        assert_eq!(GumpClient::from_shading_mode(KR_LIKE_PRESET), GumpClient::Enhanced);
    }

    /// Every preset against which packages are loaded, whether gumps_ec.uddp has the gump, and its decode error.
    #[test]
    fn gump_source_follows_the_preset() {
        use Enhanced::{DecodeError, HasGump, LacksGump, NotLoaded};
        use Expected::{Failed, Found, Missing, NoClassicSource};
        use GumpSourceKind::{Classic, Enhanced as Ec};
        let classic_cases = [
            (NotLoaded, true, Found(Classic, CLASSIC_IMAGE)),
            (LacksGump, true, Found(Classic, CLASSIC_IMAGE)),
            (HasGump, true, Found(Classic, CLASSIC_IMAGE)),
            (DecodeError, true, Found(Classic, CLASSIC_IMAGE)),
            (NotLoaded, false, NoClassicSource),
            (LacksGump, false, NoClassicSource),
            (HasGump, false, NoClassicSource),
            (DecodeError, false, NoClassicSource),
        ];
        for (enhanced, classic_loaded, expected) in classic_cases {
            let picked = pick(CLASSIC_PRESET, enhanced, classic_loaded, true);
            let case = format!("Classic preset, gumps_ec {enhanced:?}, Classic source {classic_loaded}");
            assert_eq!(picked.result, expected, "{case}");
            assert!(!picked.enhanced_asked, "{case}: the Classic preset never reads gumps_ec");
        }

        let enhanced_cases = [
            (NotLoaded, true, Found(Classic, CLASSIC_IMAGE)),
            (LacksGump, true, Found(Classic, CLASSIC_IMAGE)),
            (HasGump, true, Found(Ec, ENHANCED_IMAGE)),
            (DecodeError, true, Failed(Ec)),
            (NotLoaded, false, Missing),
            (LacksGump, false, Missing),
            (HasGump, false, Found(Ec, ENHANCED_IMAGE)),
            (DecodeError, false, Failed(Ec)),
        ];
        for shading_mode in [ENHANCED_PRESET, KR_LIKE_PRESET] {
            for (enhanced, classic_loaded, expected) in &enhanced_cases {
                let picked = pick(shading_mode, *enhanced, *classic_loaded, true);
                let case = format!("preset {shading_mode}, gumps_ec {enhanced:?}, Classic source {classic_loaded}");
                assert_eq!(&picked.result, expected, "{case}");
                // The Classic source is asked only when gumps_ec lacks the gump, never after a decode error.
                let falls_back = matches!(picked.result, Found(Classic, _));
                assert_eq!(picked.classic_asked, falls_back, "{case}");
            }
        }
    }

    #[test]
    fn classic_gump_decode_error_is_reported_from_the_classic_source() {
        for shading_mode in [CLASSIC_PRESET, ENHANCED_PRESET, KR_LIKE_PRESET] {
            let picked = pick(shading_mode, Enhanced::LacksGump, true, false);
            assert_eq!(picked.result, Expected::Failed(GumpSourceKind::Classic), "preset {shading_mode}");
        }
    }

    #[test]
    fn gump_startup_summary_names_each_preset_source() {
        assert_eq!(
            gump_sources_summary(Some(GUMPS_CC_FILE_NAME), true),
            "gumps: Classic preset gumps_cc.uddp; Enhanced and KR-like gumps_ec.uddp, then gumps_cc.uddp for ids it lacks"
        );
        assert_eq!(
            gump_sources_summary(None, true),
            "gumps: Classic preset none, so it shows no gumps; Enhanced and KR-like gumps_ec.uddp"
        );
        assert_eq!(
            gump_sources_summary(Some(GUMPS_CC_FILE_NAME), false),
            "gumps: Classic preset gumps_cc.uddp; Enhanced and KR-like gumps_cc.uddp (gumps_ec.uddp is not loaded)"
        );
    }

    /// The startup warning comes only for a shared gumps_ec.uddp without a Classic source to draw what it shares.
    #[test]
    fn shared_gumps_without_a_classic_source_are_warned_of_once_at_startup() {
        const SHARED_COUNT: usize = 2216;
        assert_eq!(shared_gumps_without_classic_warning(0, false), None);
        assert_eq!(shared_gumps_without_classic_warning(SHARED_COUNT, true), None);
        let warning = shared_gumps_without_classic_warning(SHARED_COUNT, false).expect("a warning");
        assert!(warning.contains("2216 gumps with gumps_cc.uddp"), "{warning}");
        assert!(warning.contains("won't show under the Enhanced and KR-like presets"), "{warning}");
    }

    /// A shared gump is hashed the first time it is served and never again; the first mismatch is told apart from the
    /// later ones, which carry the count; a gump gumps_ec.uddp doesn't share is never checked.
    #[test]
    fn shared_gumps_are_hashed_once_and_mismatches_counted() {
        const MATCHING: u32 = 100;
        const FIRST_DIFFERING: u32 = 101;
        const SECOND_DIFFERING: u32 = 102;
        const NOT_SHARED: u32 = 103;
        const SIDE: u32 = 2;
        const RGBA_CHANNELS: usize = 4;
        const OPAQUE_GREY: u8 = 128;
        let served = udd_assets::gumps::GumpImage {
            physical_width: SIDE,
            physical_height: SIDE,
            logical_width: SIDE,
            logical_height: SIDE,
            upscale_factor: 1,
            rgba: vec![OPAQUE_GREY; (SIDE * SIDE) as usize * RGBA_CHANNELS],
        };
        let served_hash = udd_assets::gumps::gump_image_hash(&served);
        let other_hash = served_hash.wrapping_add(1);
        let check = SharedGumpCheck::default();

        assert_eq!(check.check(NOT_SHARED, None, &served), SharedGumpVerdict::NotShared);
        assert_eq!(check.check(MATCHING, Some(served_hash), &served), SharedGumpVerdict::Matches);
        assert_eq!(check.check(MATCHING, Some(served_hash), &served), SharedGumpVerdict::AlreadyChecked);
        assert_eq!(check.check(FIRST_DIFFERING, Some(other_hash), &served), SharedGumpVerdict::FirstMismatch);
        assert_eq!(check.check(FIRST_DIFFERING, Some(other_hash), &served), SharedGumpVerdict::AlreadyChecked);
        assert_eq!(
            check.check(SECOND_DIFFERING, Some(other_hash), &served),
            SharedGumpVerdict::Mismatch { count: 2 }
        );
        assert_eq!(check.check(NOT_SHARED, None, &served), SharedGumpVerdict::NotShared);
    }

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
