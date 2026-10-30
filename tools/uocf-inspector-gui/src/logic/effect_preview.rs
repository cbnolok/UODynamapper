//! Preview of effect definitions with particle-sim: which particles of a definition list play (resolving
//! random, LOD, round-robin, wait, property flag and user preference definitions the way one client play would),
//! a fixed-step simulation that can be scrubbed (re-simulated from 0 with the same seed, so a time always shows
//! the same frame), an orthographic orbit camera fitted to the whole timeline, software rendering with
//! particle-sim's rasterizer, and PNG exports. No egui here.
//!
//! Chaos (`.ems`) scripts and NIF (`.nif`) effects play side by side; the drawables of every part are drawn in one
//! list, unsorted ones first, then blended ones far to near (particle-sim's [`Canvas::draw_group`]). Chaos MESH
//! particles draw clones of their `.nif`.
//! With an object hue, the NIF particles whose definition sets `hued` play as `UOHuedParticleEffectNif` (half speed,
//! drawn through the `Hues.uop` atlas: particle-sim's [`particle_sim::hued`]).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use gamebryo_nif::NifFile;
use image::RgbaImage;
use particle_sim::chaos::compile::compile_chaos_source;
use particle_sim::chaos::{ChaosParticleType, MeshSource};
use particle_sim::mips::client_mip_levels;
use particle_sim::nif::default_camera;
use particle_sim::nif::desc::NifTextureSource;
use particle_sim::hued::{
    draw_hued_quad, hue_mode, hue_quad_sort_key, offscreen_camera, render_hued_target, HueMode, HuedRenderSettings,
    HUED_EFFECT_TIME_SCALE, HUED_QUAD_HALF_SIZE, NO_HUE,
};
use particle_sim::raster::{
    frame_bounds, frame_extent, orbit_axes, Canvas, GroupItem, OrthoView, RasterTexture, SceneDrawOptions,
    CLIENT_VIEW_ELEVATION, CLIENT_VIEW_YAW, OVERLAY_BLEND,
};
use particle_sim::render::{CameraFrame, RenderFrame, SceneLighting};
use particle_sim::{
    ChaosEffectInstance, ChaosSystem, EffectInstance, EffectPlacement, Mt19937, NifEffect, NifEffectInstance,
    NodeTransform, ParticleEffectParams, ParticleSystemInstance, Rng, Vec3,
};
use uocf::enhanced::effect_definition::{
    EffectDefinition, ParticleEffect, PropertyFlagEffect, PropertyFlagMode, PARTICLE_ATTACH_PROJECTILE_ONLY,
    RANDOM_WEIGHT_TOTAL,
};
use uocf::enhanced::hues::{hues_atlas_hash, HUES_UOP_NAME};
use uocf::uop_container::package::{LoadMode, UopPackage};
use uocf::enhanced::string_dictionary::UoStringDictionary;

use super::effect_files::{decode_texture, decode_texture_levels};
use super::effects_editor::{is_chaos_name, particle_name, ChildList, DefinitionKind, PathStep};

/// Simulation step: the preview always advances in whole steps of this many milliseconds.
pub const PREVIEW_STEP_MS: u32 = 16;
pub const DEFAULT_PREVIEW_SEED: u32 = 0x5EED;
pub const DEFAULT_TIMELINE_S: f32 = 4.0;
pub const MIN_TIMELINE_S: f32 = 0.5;
pub const MAX_TIMELINE_S: f32 = 30.0;
pub const DEFAULT_MOVE_SPEED: f32 = 2.0;
pub const MAX_MOVE_SPEED: f32 = 20.0;
pub const MIN_PLAYBACK_SPEED: f32 = 0.1;
pub const MAX_PLAYBACK_SPEED: f32 = 4.0;
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 20.0;
/// Side of the square preview canvas, pixels.
pub const PREVIEW_RESOLUTION_PX: usize = 384;
/// Default UI particle LOD setting (High), as the client.
pub const DEFAULT_LOD_LEVEL: u32 = uocf::enhanced::effect_definition::PARTICLE_LOD_DEFAULT;
pub const MAX_LOD_LEVEL: u32 = uocf::enhanced::effect_definition::PARTICLE_LOD_MAX;
pub const DEFAULT_BACKGROUND: [f32; 4] = [0.10, 0.10, 0.12, 1.0];
/// Default camera pitch, degrees: the client's view. Every effect is a camera-aligned billboard (local x = camera
/// right, y = view direction, z = camera up; `uosa_effects.md`, "Effect camera and lighting"), so the client sees
/// the effect's frame orthographically along +y with +z up: particle-sim's `CameraFrame::client_view`, the camera
/// its NIF instances default to.
pub const DEFAULT_CAMERA_PITCH_DEGREES: f32 = CLIENT_VIEW_ELEVATION;
/// Default camera yaw, degrees from +y toward +x: the client's view.
pub const DEFAULT_CAMERA_YAW_DEGREES: f32 = CLIENT_VIEW_YAW;
/// Pitch limits, short of straight down and up (the up axis would be undefined).
pub const MIN_CAMERA_PITCH_DEGREES: f32 = -89.0;
pub const MAX_CAMERA_PITCH_DEGREES: f32 = 89.0;
/// Yaw range, degrees.
pub const MIN_CAMERA_YAW_DEGREES: f32 = -180.0;
pub const MAX_CAMERA_YAW_DEGREES: f32 = 180.0;
const FULL_TURN_DEGREES: f32 = 360.0;
/// Distance of the camera from the view centre (for the axial view vector).
const CAMERA_DISTANCE: f32 = 1000.0;
/// How the meshes are lit (shown with the lighting controls).
pub const LIGHTING_NOTE: &str = "Meshes and NIF particle colours use these lights without direction (diffuse \
    as if facing every surface). The default is the client's: no light reaches effects, so lit (E_A_D) materials \
    show only their emissive colour. Lights inside an effect NIF (one shipped file has two) are applied on top, \
    as the client does: per vertex on the meshes of the nodes whose effect list holds them; particles, which have \
    no normals, only get ambient lights.";
/// Fraction of the canvas left free around the fitted bounds.
const FIT_MARGIN: f32 = 0.08;
/// Smallest world extent fitted into the canvas (an effect with a single point).
const MIN_FIT_EXTENT: f32 = 4.0;
/// The fit samples the frame every this many steps.
const FIT_SAMPLE_EVERY_STEPS: u32 = 4;
const MESH_COLOR: [f32; 4] = [1.0, 0.0, 1.0, 1.0];
const EMITTER_COLOR: [f32; 4] = [0.9, 0.9, 0.2, 1.0];
const BOUNDS_COLOR: [f32; 4] = [0.3, 0.8, 1.0, 0.8];
const EMITTER_MARKER_HALF_PX: f32 = 2.5;
const MS_PER_SECOND: f32 = 1000.0;
/// Contact sheet layout.
pub const CONTACT_SHEET_COLUMNS: usize = 4;
const CONTACT_SHEET_GAP_PX: u32 = 2;
const CONTACT_SHEET_GAP_COLOR: image::Rgba<u8> = image::Rgba([40, 40, 48, 255]);
/// Number of frames of an exported sequence or contact sheet by default.
pub const DEFAULT_EXPORT_FRAMES: usize = 12;
pub const MAX_EXPORT_FRAMES: usize = 240;

/// `PropertyFlagEffect::property` that keys the cases by the object hue (`UOObject::GetHue`, vtable +0x58;
/// `uosa_effects.md`, "12 PropertyFlag").
const PROPERTY_FLAG_OBJECT_HUE: u32 = 7;

/// The loop flag the preview gives the particles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    /// The definition's `looping`.
    #[default]
    Definition,
    Loop,
    Once,
}

impl LoopMode {
    pub const ALL: [Self; 3] = [Self::Definition, Self::Loop, Self::Once];

    pub fn label(self) -> &'static str {
        match self {
            Self::Definition => "as defined",
            Self::Loop => "loop",
            Self::Once => "once",
        }
    }
}

/// What the simulation depends on: a change rebuilds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimulationSettings {
    pub seed: u32,
    pub loop_mode: LoopMode,
    /// Seconds, replacing every particle's `duration`.
    pub duration_override: Option<f32>,
    /// Hue of the object the effect plays on ([`NO_HUE`]: none). NIF particles whose definition sets `hued` take it.
    pub hue: u32,
    /// World units per second along +x; 0 keeps the emitter still.
    pub move_speed: f32,
    /// UI particle LOD setting (1..3) for LOD definitions.
    pub lod_level: u32,
    pub timeline_s: f32,
    /// Play the attach mode 2 particles of tileart effects, which the client does not play on the world object.
    pub play_projectile_only: bool,
}

impl Default for SimulationSettings {
    fn default() -> Self {
        Self {
            seed: DEFAULT_PREVIEW_SEED,
            loop_mode: LoopMode::Definition,
            duration_override: None,
            hue: NO_HUE,
            move_speed: 0.0,
            lod_level: DEFAULT_LOD_LEVEL,
            timeline_s: DEFAULT_TIMELINE_S,
            play_projectile_only: false,
        }
    }
}

impl SimulationSettings {
    pub fn timeline_ms(&self) -> u32 {
        (self.timeline_s.clamp(MIN_TIMELINE_S, MAX_TIMELINE_S) * MS_PER_SECOND) as u32
    }
}

/// The orbit of the preview camera about the view centre, from the client's view (the default).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraOrbit {
    /// Degrees from +y toward +x about +z.
    pub yaw_degrees: f32,
    /// Degrees above the x-y plane (the camera looks down from positive pitch).
    pub pitch_degrees: f32,
}

impl Default for CameraOrbit {
    fn default() -> Self {
        Self { yaw_degrees: DEFAULT_CAMERA_YAW_DEGREES, pitch_degrees: DEFAULT_CAMERA_PITCH_DEGREES }
    }
}

impl CameraOrbit {
    /// The orbit turned by a drag: yaw wraps around, pitch stays in its limits.
    pub fn turned(self, yaw_degrees: f32, pitch_degrees: f32) -> Self {
        let yaw = (self.yaw_degrees + yaw_degrees - MIN_CAMERA_YAW_DEGREES).rem_euclid(FULL_TURN_DEGREES);
        Self {
            yaw_degrees: yaw + MIN_CAMERA_YAW_DEGREES,
            pitch_degrees: (self.pitch_degrees + pitch_degrees).clamp(MIN_CAMERA_PITCH_DEGREES, MAX_CAMERA_PITCH_DEGREES),
        }
    }

    /// (direction, up, right).
    pub fn axes(&self) -> (Vec3, Vec3, Vec3) {
        orbit_axes(self.yaw_degrees.to_radians(), self.pitch_degrees.to_radians())
    }

    /// The camera looking at the origin from [`CAMERA_DISTANCE`] (the billboard camera of NIF effects).
    pub fn camera(&self) -> CameraFrame {
        let (direction, up, right) = self.axes();
        CameraFrame { position: direction * -CAMERA_DISTANCE, direction, up, right }
    }
}

/// How the frame is drawn. Changing the orbit re-simulates (NIF billboards face the camera while they update);
/// the rest only redraws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSettings {
    pub background: [f32; 4],
    pub show_bounds: bool,
    pub show_emitter: bool,
    /// Draw the particles (sprite batches).
    pub show_particles: bool,
    /// Draw the meshes (NIF geometry and mesh particles).
    pub show_meshes: bool,
    pub orbit: CameraOrbit,
    /// Lights of the meshes and NIF particle colours ([`LIGHTING_NOTE`]).
    pub lighting: SceneLighting,
    /// Multiplies the fitted scale.
    pub zoom: f32,
    /// Canvas pixels.
    pub pan: [f32; 2],
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            background: DEFAULT_BACKGROUND,
            show_bounds: false,
            show_emitter: true,
            show_particles: true,
            show_meshes: true,
            orbit: CameraOrbit::default(),
            lighting: SceneLighting::default(),
            zoom: 1.0,
            pan: [0.0; 2],
        }
    }
}

/// Why a part does or does not play.
#[derive(Debug, Clone, PartialEq)]
pub enum PartStatus {
    /// A Chaos particle that plays (never hued: `hue` tells why when there is a hue).
    Chaos { name: String, sequences: usize, hue: HueMode },
    /// A NIF effect that plays; `warnings` are the objects the compiler skipped or could not read.
    Nif { name: String, particle_systems: usize, meshes: usize, warnings: Vec<String>, hue: HueMode },
    /// The NIF file does not parse: it plays nothing.
    NifRejected { name: String, error: String },
    /// The particle name is not in the string dictionary (or there is none).
    NameUnresolved { name_index: u32 },
    /// The file is not in `EffectTexture.uop`.
    MissingFile { name: String },
    /// The client rejects the script: it plays nothing.
    ScriptRejected { name: String, errors: Vec<String> },
    /// A sound, earthquake or position offset: nothing to draw.
    NotVisual,
    /// A tileart particle with attach mode 2: `UOObject::CreateComponents` does not play it on the world object
    /// (`uosa_effects.md`, "Particle attach mode"); it plays only in flight (projectiles).
    ProjectileOnly { name: String },
}

impl PartStatus {
    pub fn plays(&self) -> bool {
        matches!(self, Self::Chaos { .. } | Self::Nif { .. })
    }

    /// Compile warnings of a NIF part.
    pub fn warnings(&self) -> &[String] {
        match self {
            Self::Nif { warnings, .. } => warnings,
            _ => &[],
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Chaos { name, sequences, hue } => format!("{name}: Chaos, {sequences} sequence(s){}", hue_note(*hue)),
            Self::Nif { name, particle_systems, meshes, warnings, hue } => format!(
                "{name}: NIF, {particle_systems} particle system(s), {meshes} mesh(es), {} compile warning(s){}",
                warnings.len(),
                hue_note(*hue)
            ),
            Self::NifRejected { name, error } => format!("{name}: NIF does not parse ({error})"),
            Self::NameUnresolved { name_index } => format!("name index {name_index} is not in the string dictionary"),
            Self::MissingFile { name } => format!("{name}: not in EffectTexture.uop"),
            Self::ScriptRejected { name, errors } => format!("{name}: rejected by the client ({})", errors.join("; ")),
            Self::NotVisual => "not drawn".to_string(),
            Self::ProjectileOnly { name } => {
                format!("{name}: attach mode 2, not played by the client on the tileart object (only in flight)")
            }
        }
    }
}

/// What a part list says about the object hue: nothing without one.
fn hue_note(hue: HueMode) -> &'static str {
    match hue {
        HueMode::Hued => ", hued (half speed)",
        HueMode::Chaos => ", not hued (Chaos effects never are)",
        HueMode::FlagNotSet => ", not hued (the definition's hued is not set)",
        HueMode::NoHue => "",
    }
}

/// One definition of the previewed list that the play reaches.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewPart {
    /// Path from the previewed list.
    pub path: Vec<PathStep>,
    pub kind: DefinitionKind,
    pub status: PartStatus,
}

/// A compiled particle system ready to instantiate.
#[derive(Clone)]
pub enum PreparedSystem {
    Chaos(Arc<ChaosSystem>),
    Nif(Arc<NifEffect>),
}

struct PreparedPart {
    params: ParticleEffectParams,
    /// Plays as `UOHuedParticleEffectNif` ([`HueMode::Hued`]).
    hued: bool,
    seed: u32,
    system: PreparedSystem,
    /// Textures embedded in a NIF file, by lowercase render name (the names repeat across files).
    embedded: HashMap<String, RasterTexture>,
    /// The package textures the part's NIF file mipmaps (`NiSourceTexture` prefs `YES`), lowercase.
    mipmapped: HashSet<String>,
    /// The `.nif` of a Chaos part's MESH sequences.
    meshes: Option<MeshSource>,
}

/// What a play of a definition list draws: the reached parts, the choices made and the prepared systems with
/// their textures.
pub struct PreviewScene {
    pub parts: Vec<PreviewPart>,
    /// Choices made while resolving (random draws, LOD levels, ...).
    pub notes: Vec<String>,
    /// Textures that are missing or do not decode.
    pub texture_problems: Vec<String>,
    prepared: Vec<PreparedPart>,
    /// Package textures with the client's mip levels, by lowercase name and whether they are mipmapped.
    textures: HashMap<(String, bool), RasterTexture>,
}

/// File access of the scene builder.
pub struct PreviewInput<'a> {
    pub strings: Option<&'a UoStringDictionary>,
    /// The payload of an `EffectTexture.uop` file by name, with pending changes applied.
    pub payload: &'a dyn Fn(&str) -> Option<Arc<[u8]>>,
    /// Edited NIF files that do not write yet, by name: played instead of their payload.
    pub nif_drafts: Option<&'a dyn Fn(&str) -> Option<Arc<NifFile>>>,
}

impl PreviewScene {
    /// Resolves a definition list the way one play does and prepares its particles.
    pub fn build(definitions: &[EffectDefinition], settings: &SimulationSettings, input: &PreviewInput<'_>) -> Self {
        let mut builder = SceneBuilder::new(settings, input);
        builder.list(definitions, &[], ChildList::Top);
        builder.scene
    }

    /// The effects of a tileart record played on its world object (`UOObject::CreateComponents`): as
    /// [`Self::build`], but attach mode 2 particles do not play unless `play_projectile_only` is set.
    pub fn build_tileart(
        definitions: &[EffectDefinition],
        settings: &SimulationSettings,
        input: &PreviewInput<'_>,
    ) -> Self {
        let mut builder = SceneBuilder::new(settings, input);
        builder.tileart = true;
        builder.list(definitions, &[], ChildList::Top);
        builder.scene
    }

    /// One Chaos script or NIF file (an `EffectTexture.uop` file) played as a particle with default fields; a
    /// NIF without a duration plays its animation length (or the timeline).
    pub fn build_script(name: &str, settings: &SimulationSettings, input: &PreviewInput<'_>) -> Self {
        let EffectDefinition::Particle(particle) = DefinitionKind::Particle.new_definition() else {
            unreachable!("a particle kind makes a particle")
        };
        let mut builder = SceneBuilder::new(settings, input);
        builder.standalone = true;
        let status = builder.particle_named(&particle, name.to_owned());
        builder.scene.parts.push(PreviewPart { path: vec![PathStep::top(0)], kind: DefinitionKind::Particle, status });
        builder.scene
    }

    /// Whether anything plays.
    pub fn has_playing_parts(&self) -> bool {
        !self.prepared.is_empty()
    }

    /// A texture of part `part`: its embedded textures first, then the package's.
    fn texture(&self, part: usize, name: &str) -> Option<&RasterTexture> {
        let key = name.to_ascii_lowercase();
        let part = self.prepared.get(part)?;
        let mipmapped = part.mipmapped.contains(&key);
        part.embedded.get(&key).or_else(|| self.textures.get(&(key, mipmapped)))
    }
}

struct SceneBuilder<'s, 'a> {
    settings: &'s SimulationSettings,
    input: &'s PreviewInput<'a>,
    /// Draws of random definitions, seeded by the preview seed.
    draw: Mt19937,
    /// One file previewed on its own ([`PreviewScene::build_script`]).
    standalone: bool,
    /// Tileart effects played on the world object ([`PreviewScene::build_tileart`]).
    tileart: bool,
    scene: PreviewScene,
}

impl<'s, 'a> SceneBuilder<'s, 'a> {
    fn new(settings: &'s SimulationSettings, input: &'s PreviewInput<'a>) -> Self {
        Self {
            settings,
            input,
            draw: Mt19937::new(settings.seed),
            standalone: false,
            tileart: false,
            scene: PreviewScene {
                parts: Vec::new(),
                notes: Vec::new(),
                texture_problems: Vec::new(),
                prepared: Vec::new(),
                textures: HashMap::new(),
            },
        }
    }

    fn list(&mut self, definitions: &[EffectDefinition], parent: &[PathStep], list: ChildList) {
        for (index, definition) in definitions.iter().enumerate() {
            let mut path = parent.to_vec();
            path.push(PathStep { list, index });
            self.definition(definition, path);
        }
    }

    fn definition(&mut self, definition: &EffectDefinition, path: Vec<PathStep>) {
        let kind = DefinitionKind::of(definition);
        let label = path_label(&path);
        match definition {
            EffectDefinition::Particle(particle)
                if self.tileart && particle.attach_mode == PARTICLE_ATTACH_PROJECTILE_ONLY =>
            {
                if self.settings.play_projectile_only {
                    self.scene.notes.push(format!("{label} particle: attach mode 2, played although the client does not"));
                    let status = self.particle(particle);
                    self.scene.parts.push(PreviewPart { path, kind, status });
                } else {
                    let name = particle_name(particle, self.input.strings)
                        .unwrap_or_else(|| format!("name index {}", particle.name_index));
                    self.scene.parts.push(PreviewPart { path, kind, status: PartStatus::ProjectileOnly { name } });
                }
            }
            EffectDefinition::Particle(particle) => {
                let status = self.particle(particle);
                self.scene.parts.push(PreviewPart { path, kind, status });
            }
            EffectDefinition::Sound(_) | EffectDefinition::Earthquake(_) | EffectDefinition::PositionOffset(_) => {
                self.scene.parts.push(PreviewPart { path, kind, status: PartStatus::NotVisual });
            }
            EffectDefinition::RoundRobin(effect) => {
                if !effect.children.is_empty() {
                    self.scene.notes.push(format!("{label} round robin: child 0 (the first play)"));
                    self.list(&effect.children[..1], &path, ChildList::Children);
                }
            }
            EffectDefinition::Wait(effect) => {
                self.scene.notes.push(format!("{label} wait: the first play passes (later plays are throttled)"));
                self.list(&effect.children, &path, ChildList::Children);
            }
            EffectDefinition::UserPreference(effect) => {
                self.scene.notes.push(format!("{label} user preference: played (no user settings here)"));
                self.list(&effect.children, &path, ChildList::Children);
            }
            EffectDefinition::Lod(effect) => {
                let setting = self.settings.lod_level;
                let chosen = effect
                    .levels
                    .iter()
                    .enumerate()
                    .filter(|(_, level)| level.key <= setting)
                    .max_by_key(|(_, level)| level.key);
                match chosen {
                    Some((index, level)) => {
                        self.scene.notes.push(format!("{label} LOD: level {} for setting {setting}", level.key));
                        self.list(&level.children, &path, ChildList::Level(index));
                    }
                    None => self.scene.notes.push(format!("{label} LOD: no level <= {setting}, plays nothing")),
                }
            }
            EffectDefinition::PropertyFlag(effect) => self.property_flag(effect, &label, &path),
            EffectDefinition::Random(effect) => {
                let mut draw = self.draw.rand01() * RANDOM_WEIGHT_TOTAL;
                let rolled = draw;
                let chosen = effect.entries.iter().position(|entry| {
                    let hit = draw < entry.weight;
                    draw -= entry.weight;
                    hit
                });
                match chosen {
                    Some(index) => {
                        self.scene.notes.push(format!("{label} random: drew {rolled:.1}, entry {index}"));
                        self.list(&effect.entries[index].children, &path, ChildList::RandomEntry(index));
                    }
                    None => self.scene.notes.push(format!("{label} random: drew {rolled:.1}, no entry")),
                }
            }
        }
    }

    fn particle(&mut self, particle: &ParticleEffect) -> PartStatus {
        let Some(name) = particle_name(particle, self.input.strings) else {
            return PartStatus::NameUnresolved { name_index: particle.name_index };
        };
        self.particle_named(particle, name)
    }

    fn particle_named(&mut self, particle: &ParticleEffect, name: String) -> PartStatus {
        if !is_chaos_name(&name) {
            return self.nif_particle(particle, name);
        }
        let Some(payload) = (self.input.payload)(&name) else {
            return PartStatus::MissingFile { name };
        };
        let report = compile_chaos_source(&payload);
        let Some(desc) = report.desc else {
            let errors = report.errors().map(ToString::to_string).collect();
            return PartStatus::ScriptRejected { name, errors };
        };
        // MESH sequences name `.nif` scenes, compiled once per name (`ParticleSystemManager::LoadMesh`) with their
        // textures; the other resources are textures, not mipmapped (`MIP_DEFAULT` prefs,
        // `NiTexture::ms_bMipmapByDefault` left false).
        let (mut embedded, mut mipmapped) = (HashMap::new(), HashSet::new());
        let mut meshes = HashMap::new();
        for resource in desc.resources() {
            let is_mesh = desc
                .sequences
                .iter()
                .any(|sequence| sequence.resource == resource && sequence.particle_type == ChaosParticleType::Mesh);
            if !is_mesh {
                self.load_texture(resource, false);
                continue;
            }
            match self.read_nif(resource) {
                Ok(nif) => {
                    let desc = particle_sim::nif::compile(&nif, resource).desc;
                    self.nif_textures(resource, &desc, &mut embedded, &mut mipmapped);
                    meshes.insert(resource.to_ascii_lowercase(), Arc::new(NifEffect::prepare(desc)));
                }
                Err(status) => self.scene.notes.push(format!("{name}: MESH {}, drawn as outlines", status.describe())),
            }
        }
        let meshes = Some(MeshSource::new(move |name| meshes.get(&name.to_ascii_lowercase()).cloned()));
        let sequences = desc.sequences.len();
        let seed = self.part_seed();
        let system = Arc::new(ChaosSystem::prepare(desc, &mut Mt19937::new(seed)));
        let params = self.params(particle, None);
        let system = PreparedSystem::Chaos(system);
        self.scene.prepared.push(PreparedPart { params, hued: false, seed, system, embedded, mipmapped, meshes });
        PartStatus::Chaos { name, sequences, hue: hue_mode(true, particle.hued, self.settings.hue) }
    }

    /// A property flag definition (`uosa_effects.md`, "12 PropertyFlag"): with property 7 the key is the object
    /// hue (the preview hue), and the matching case plays, else the default children. Other properties and the
    /// flag mask mode read object values the preview has no counterpart for.
    fn property_flag(&mut self, effect: &PropertyFlagEffect, label: &str, path: &[PathStep]) {
        if effect.mode == PropertyFlagMode::FlagMask {
            self.scene.notes.push(format!("{label} property flag (flag mask): no object flags, plays nothing"));
            return;
        }
        if effect.property != PROPERTY_FLAG_OBJECT_HUE {
            let property = effect.property;
            self.scene.notes.push(format!("{label} property flag (property {property}): not modelled, default children"));
            self.list(&effect.default_children, path, ChildList::Default);
            return;
        }
        let hue = self.settings.hue;
        match effect.cases.iter().position(|case| case.key == hue) {
            Some(index) => {
                self.scene.notes.push(format!("{label} property flag (hue): case {hue} for object hue {hue}"));
                self.list(&effect.cases[index].children, path, ChildList::Case(index));
            }
            None => {
                let children = effect.default_children.len();
                self.scene.notes.push(format!(
                    "{label} property flag (hue): no case for object hue {hue}, {children} default child(ren)"
                ));
                self.list(&effect.default_children, path, ChildList::Default);
            }
        }
    }

    /// A NIF file: its draft, else its payload parsed; the part status when it does not load.
    fn read_nif(&self, name: &str) -> Result<Arc<NifFile>, PartStatus> {
        if let Some(nif) = self.input.nif_drafts.and_then(|drafts| drafts(name)) {
            return Ok(nif);
        }
        let name = name.to_owned();
        let payload = (self.input.payload)(&name).ok_or_else(|| PartStatus::MissingFile { name: name.clone() })?;
        NifFile::read(&payload).map(Arc::new).map_err(|error| PartStatus::NifRejected { name, error: error.to_string() })
    }

    /// Loads the textures of NIF `name`: package files into the scene (recording the mipmapped ones), embedded
    /// pixels into `embedded` with their stored levels (a mipmapped single level gets the generated chain).
    fn nif_textures(
        &mut self,
        name: &str,
        desc: &particle_sim::NifEffectDesc,
        embedded: &mut HashMap<String, RasterTexture>,
        mipmapped: &mut HashSet<String>,
    ) {
        for texture in &desc.textures {
            match &texture.source {
                NifTextureSource::External { file_name } => {
                    self.load_texture(file_name, texture.mipmapped);
                    if texture.mipmapped {
                        mipmapped.insert(file_name.to_ascii_lowercase());
                    }
                }
                NifTextureSource::Embedded { width, height, levels } => {
                    let (levels, _) = client_mip_levels(*width, *height, levels.clone(), texture.mipmapped, true);
                    match RasterTexture::from_rgba8_levels(*width as usize, *height as usize, &levels) {
                        Some(raster) => {
                            embedded.insert(texture.name().to_ascii_lowercase(), raster);
                        }
                        None => self.scene.texture_problems.push(format!("{name}: {}: bad size", texture.name())),
                    }
                }
                NifTextureSource::Undecoded { format } => self
                    .scene
                    .texture_problems
                    .push(format!("{name}: {}: pixel format {format} not decoded (drawn white)", texture.name())),
            }
        }
    }

    /// A NIF effect: the draft of the file, else its payload parsed.
    fn nif_particle(&mut self, particle: &ParticleEffect, name: String) -> PartStatus {
        let nif = match self.read_nif(&name) {
            Ok(nif) => nif,
            Err(status) => return status,
        };
        let output = particle_sim::nif::compile(&nif, &name);
        let desc = output.desc;
        let (mut embedded, mut mipmapped) = (HashMap::new(), HashSet::new());
        self.nif_textures(&name, &desc, &mut embedded, &mut mipmapped);
        let fallback_duration = if desc.animation_length() > 0.0 { desc.animation_length() } else { self.settings.timeline_s };
        let params = self.params(particle, self.standalone.then_some(fallback_duration));
        let (particle_systems, meshes) = (desc.particle_systems.len(), desc.meshes.len());
        let seed = self.part_seed();
        let system = PreparedSystem::Nif(Arc::new(NifEffect::prepare(desc)));
        let hue = hue_mode(false, particle.hued, self.settings.hue);
        let hued = hue == HueMode::Hued;
        self.scene.prepared.push(PreparedPart { params, hued, seed, system, embedded, mipmapped, meshes: None });
        PartStatus::Nif { name, particle_systems, meshes, warnings: output.warnings, hue }
    }

    /// The seed of the next prepared part.
    fn part_seed(&self) -> u32 {
        self.settings.seed.wrapping_add(self.scene.prepared.len() as u32)
    }

    /// The definition's fields with the preview's duration and loop overrides; `fallback_duration` replaces a
    /// duration that is not positive.
    fn params(&self, particle: &ParticleEffect, fallback_duration: Option<f32>) -> ParticleEffectParams {
        let mut params = ParticleEffectParams::from(particle);
        if let Some(duration) = fallback_duration.filter(|_| params.duration_s <= 0.0) {
            params.duration_s = duration;
        }
        if let Some(duration) = self.settings.duration_override {
            params.duration_s = duration;
        }
        match self.settings.loop_mode {
            LoopMode::Definition => {}
            LoopMode::Loop => params.looping = true,
            LoopMode::Once => params.looping = false,
        }
        params
    }

    /// Loads a package texture with the client's mip levels: its stored levels, or a chain generated from one
    /// level when `mipmapped` (the packer's rules, [`particle_sim::mips`]).
    fn load_texture(&mut self, name: &str, mipmapped: bool) {
        let key = (name.to_ascii_lowercase(), mipmapped);
        if self.scene.textures.contains_key(&key) {
            return;
        }
        // A `.nif` texture is a NIF image (one `NiPixelData`), which the client's image converter reads too.
        match (self.input.payload)(name).map(|payload| decode_texture_levels(name, &payload)) {
            Some(Ok(decoded)) => {
                let (width, height) = (decoded.width, decoded.height);
                let (levels, _) = client_mip_levels(width, height, decoded.levels, mipmapped, decoded.generatable);
                if let Some(texture) = RasterTexture::from_rgba8_levels(width as usize, height as usize, &levels) {
                    self.scene.textures.insert(key, texture);
                }
            }
            Some(Err(error)) => self.scene.texture_problems.push(format!("{name}: {error}")),
            None => self.scene.texture_problems.push(format!("{name}: not in EffectTexture.uop (drawn white)")),
        }
    }
}

/// A short label of a path for notes: `0`, `1/entry 2/0`.
pub fn path_label(path: &[PathStep]) -> String {
    path.iter()
        .map(|step| match step.list {
            ChildList::Top | ChildList::Children => step.index.to_string(),
            ChildList::Level(level) => format!("level#{level}/{}", step.index),
            ChildList::Default => format!("default/{}", step.index),
            ChildList::Case(case) => format!("case#{case}/{}", step.index),
            ChildList::RandomEntry(entry) => format!("entry {entry}/{}", step.index),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A running part: a Chaos effect, or a NIF effect kept concrete for its lights.
enum PartInstance {
    Chaos(EffectInstance),
    Nif {
        params: ParticleEffectParams,
        system: NifEffectInstance,
        /// Plays as `UOHuedParticleEffectNif`: half speed, billboards facing the offscreen camera, drawn through
        /// the hue atlas.
        hued: bool,
        /// The node position of the last update: where the hue quad is drawn.
        position: Vec3,
    },
}

impl PartInstance {
    fn update(&mut self, dt_ms: u32, placement: &EffectPlacement, camera: &CameraFrame) {
        match self {
            Self::Chaos(instance) => instance.update(dt_ms, placement),
            Self::Nif { params, system, hued, position } => {
                // `EffectInstance::node`.
                let node = NodeTransform {
                    position: placement.anchor + params.node_offset(placement),
                    rotation: placement.node_rotation,
                    scale: params.node_scale(),
                    velocity: placement.velocity,
                };
                if *hued {
                    // `UOHuedParticleEffectNif::Update` places the offscreen camera at the effect first.
                    system.set_camera(offscreen_camera(camera, node.position));
                }
                *position = node.position;
                system.update(dt_ms, &node);
            }
        }
    }

    fn system(&self) -> &dyn ParticleSystemInstance {
        match self {
            Self::Chaos(instance) => instance.system(),
            Self::Nif { system, .. } => system,
        }
    }
}

/// The running preview: every playing part of a scene, advanced in fixed steps.
pub struct PreviewSimulation {
    instances: Vec<PartInstance>,
    settings: SimulationSettings,
    orbit: CameraOrbit,
    time_ms: u32,
}

impl PreviewSimulation {
    /// Starts every playing part at time 0, seen from `orbit` (NIF billboards face it).
    pub fn new(scene: &PreviewScene, settings: &SimulationSettings, orbit: CameraOrbit) -> Self {
        let instances = scene
            .prepared
            .iter()
            .map(|part| match &part.system {
                PreparedSystem::Chaos(system) => {
                    let mut system = ChaosEffectInstance::with_seed(Arc::clone(system), part.seed);
                    if let Some(meshes) = &part.meshes {
                        system.set_mesh_source(meshes);
                    }
                    let mut instance = EffectInstance::new(part.params, Box::new(system));
                    instance.start();
                    PartInstance::Chaos(instance)
                }
                PreparedSystem::Nif(effect) => {
                    let mut system = NifEffectInstance::with_seed(Arc::clone(effect), part.seed);
                    system.set_camera(orbit.camera());
                    if part.hued {
                        system.set_time_scale(HUED_EFFECT_TIME_SCALE);
                    }
                    system.start(part.params.duration_s, part.params.looping);
                    PartInstance::Nif { params: part.params, system, hued: part.hued, position: Vec3::ZERO }
                }
            })
            .collect();
        Self { instances, settings: *settings, orbit, time_ms: 0 }
    }

    /// A simulation advanced to `time_ms` (rounded down to whole steps): the same frame every time.
    pub fn at(scene: &PreviewScene, settings: &SimulationSettings, orbit: CameraOrbit, time_ms: u32) -> Self {
        let mut simulation = Self::new(scene, settings, orbit);
        simulation.advance_to(time_ms);
        simulation
    }

    pub fn time_ms(&self) -> u32 {
        self.time_ms
    }

    /// The orbit the simulation was built for.
    pub fn orbit(&self) -> CameraOrbit {
        self.orbit
    }

    /// The object hue of the simulation ([`NO_HUE`]: none).
    pub fn hue(&self) -> u32 {
        self.settings.hue
    }

    /// Where the hue quad of part `part` is drawn, if the part plays hued.
    pub fn hued_position(&self, part: usize) -> Option<Vec3> {
        match self.instances.get(part) {
            Some(PartInstance::Nif { hued: true, position, .. }) => Some(*position),
            _ => None,
        }
    }

    /// Whether any part plays hued.
    pub fn has_hued_parts(&self) -> bool {
        (0..self.instances.len()).any(|part| self.hued_position(part).is_some())
    }

    /// The lights of the NIF parts (render time only).
    pub fn set_lighting(&mut self, lighting: SceneLighting) {
        for instance in &mut self.instances {
            if let PartInstance::Nif { system, .. } = instance {
                system.set_lighting(lighting);
            }
        }
    }

    /// Where the emitter is now.
    pub fn emitter_position(&self) -> Vec3 {
        emitter_at(self.settings.move_speed, self.time_ms)
    }

    /// One step.
    pub fn step(&mut self) {
        self.time_ms += PREVIEW_STEP_MS;
        let camera = self.orbit.camera();
        // `camera_offset` follows the client's camera, not the orbit.
        let placement = EffectPlacement::seen_by(self.emitter_position(), &default_camera());
        for instance in &mut self.instances {
            instance.update(PREVIEW_STEP_MS, &placement, &camera);
        }
    }

    /// Steps while a whole step fits before `time_ms`.
    pub fn advance_to(&mut self, time_ms: u32) {
        while self.time_ms + PREVIEW_STEP_MS <= time_ms {
            self.step();
        }
    }

    pub fn particle_count(&self) -> usize {
        self.instances.iter().map(|instance| instance.system().particle_count()).sum()
    }

    /// Every part stopped emitting and has no particles left (a finished NIF effect is released whatever it
    /// shows, as the client does).
    pub fn is_drained(&self) -> bool {
        self.instances.iter().all(|instance| instance.system().is_finished() && instance.system().is_drained())
    }

    /// The frame of each part still shown, with the part index (scene orders are local to a part).
    pub fn render_parts(&self) -> Vec<(usize, RenderFrame)> {
        self.instances
            .iter()
            .enumerate()
            .filter(|(_, instance)| !(instance.system().is_finished() && instance.system().is_drained()))
            .map(|(part, instance)| {
                let mut frame = RenderFrame::default();
                instance.system().render(&mut frame);
                (part, frame)
            })
            .collect()
    }

    /// Every part in one frame (for bounds; draw [`Self::render_parts`]).
    pub fn render(&self) -> RenderFrame {
        let mut frame = RenderFrame::default();
        for instance in &self.instances {
            if !(instance.system().is_finished() && instance.system().is_drained()) {
                instance.system().render(&mut frame);
            }
        }
        frame
    }
}

fn emitter_at(move_speed: f32, time_ms: u32) -> Vec3 {
    Vec3::new(move_speed * time_ms as f32 / MS_PER_SECOND, 0.0, 0.0)
}

/// A world-space box: (min, max).
pub type WorldBounds = [Vec3; 2];

/// The world box around everything a scene draws over the timeline (sampled) and the emitter path, seen from
/// the default orbit (NIF billboards turn with the camera; the box barely does). A hued part draws its hue quad
/// (`HUED_QUAD_HALF_SIZE` around its position along the camera right and up), which the box includes.
pub fn fit_bounds(scene: &PreviewScene, settings: &SimulationSettings) -> WorldBounds {
    let mut simulation = PreviewSimulation::new(scene, settings, CameraOrbit::default());
    let camera = simulation.orbit().camera();
    let quad_diagonals = [camera.right + camera.up, camera.right - camera.up].map(|axis| axis * HUED_QUAD_HALF_SIZE);
    let mut bounds: Option<WorldBounds> = None;
    let mut include = |min: Vec3, max: Vec3| {
        let [low, high] = bounds.get_or_insert([min, max]);
        *low = Vec3::new(low.x.min(min.x), low.y.min(min.y), low.z.min(min.z));
        *high = Vec3::new(high.x.max(max.x), high.y.max(max.y), high.z.max(max.z));
    };
    let end = settings.timeline_ms();
    let mut steps = 0;
    loop {
        let emitter = simulation.emitter_position();
        include(emitter, emitter);
        if steps % FIT_SAMPLE_EVERY_STEPS == 0 {
            if let Some((min, max)) = frame_bounds(&simulation.render()) {
                include(min, max);
            }
            for (part, _) in simulation.render_parts() {
                if let Some(position) = simulation.hued_position(part) {
                    for diagonal in quad_diagonals {
                        include(position - diagonal, position - diagonal);
                        include(position + diagonal, position + diagonal);
                    }
                }
            }
        }
        if simulation.time_ms() + PREVIEW_STEP_MS > end {
            break;
        }
        simulation.step();
        steps += 1;
    }
    bounds.unwrap_or([Vec3::ZERO; 2])
}

/// The view of a `width` x `height` canvas showing `bounds` (from [`fit_bounds`]) from the view's orbit,
/// zoomed and panned.
pub fn preview_view(bounds: WorldBounds, view: &ViewSettings, width: usize, height: usize) -> OrthoView {
    let (_, up, right) = view.orbit.axes();
    let [min, max] = bounds;
    let corners = (0..8).map(|corner: u32| {
        let pick = |bit: u32, low: f32, high: f32| if corner & (1 << bit) == 0 { low } else { high };
        Vec3::new(pick(0, min.x, max.x), pick(1, min.y, max.y), pick(2, min.z, max.z))
    });
    let (mut extent_x, mut extent_y) = ([f32::MAX, f32::MIN], [f32::MAX, f32::MIN]);
    for corner in corners {
        let (x, y) = (corner.dot(right), corner.dot(up));
        extent_x = [extent_x[0].min(x), extent_x[1].max(x)];
        extent_y = [extent_y[0].min(y), extent_y[1].max(y)];
    }
    let size = (extent_x[1] - extent_x[0]).max(extent_y[1] - extent_y[0]).max(MIN_FIT_EXTENT);
    let pixels = width.min(height) as f32;
    let pixels_per_unit = pixels * (1.0 - 2.0 * FIT_MARGIN) / size * view.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    let center = (min + max) * 0.5 - right * (view.pan[0] / pixels_per_unit) + up * (view.pan[1] / pixels_per_unit);
    let CameraOrbit { yaw_degrees, pitch_degrees } = view.orbit;
    OrthoView::orbit(yaw_degrees.to_radians(), pitch_degrees.to_radians(), center, CAMERA_DISTANCE, pixels_per_unit, width, height)
}

/// Draws the current frame of a simulation with the overlays. Hued parts go through `hue_atlas` (the `Hues.uop`
/// atlas, [`load_hue_atlas`]); without it they are drawn plain.
pub fn render_canvas(
    scene: &PreviewScene,
    simulation: &PreviewSimulation,
    view: &OrthoView,
    settings: &ViewSettings,
    hue_atlas: Option<&RasterTexture>,
) -> Canvas {
    let mut canvas = Canvas::new(view.width, view.height, settings.background);
    let options = SceneDrawOptions {
        lighting: settings.lighting,
        sprites: settings.show_particles,
        meshes: settings.show_meshes,
        mesh_markers: Some(MESH_COLOR),
        depth_clip: None,
        // The client's depth buffer and texture filtering.
        ..SceneDrawOptions::default()
    };
    // One draw list over every part (the client's accumulator gathers the whole scene): a hued part is its hue
    // quad, rendered from its offscreen target when drawn.
    let parts = simulation.render_parts();
    let textures: Vec<_> = parts.iter().map(|&(part, _)| part_textures(scene, part)).collect();
    let hued = HuedRenderSettings::default();
    let hued_draws: Vec<Option<(Vec3, Box<dyn Fn(&mut Canvas) + '_>)>> = parts
        .iter()
        .zip(&textures)
        .map(|((part, frame), texture)| {
            let position = simulation.hued_position(*part)?;
            let atlas = hue_atlas?;
            let (options, hued) = (&options, &hued);
            let draw = move |canvas: &mut Canvas| {
                let target = render_hued_target(frame, texture.as_ref(), options, &view.camera, position, hued.aspect);
                draw_hued_quad(canvas, view, &target, position, simulation.hue(), atlas, hued.sampling);
            };
            Some((position, Box::new(draw) as Box<dyn Fn(&mut Canvas)>))
        })
        .collect();
    let items: Vec<GroupItem<'_, '_>> = parts
        .iter()
        .zip(&textures)
        .zip(&hued_draws)
        .map(|(((_, frame), texture), hued_draw)| match hued_draw {
            Some((position, draw)) => GroupItem::Custom { sort: hue_quad_sort_key(*position), draw: draw.as_ref() },
            None => GroupItem::Frame { frame, textures: texture.as_ref() },
        })
        .collect();
    canvas.draw_group(view, &items, &options);
    if settings.show_bounds {
        let frame = simulation.render();
        if let Some([min_x, max_x, min_y, max_y]) = frame_extent(&frame, &view.camera) {
            let origin = view.project(view.center);
            let to_screen = |x: f32, y: f32| {
                (
                    view.width as f32 * 0.5 + (x - origin.0) * view.pixels_per_unit,
                    view.height as f32 * 0.5 - (y - origin.1) * view.pixels_per_unit,
                )
            };
            canvas.outline_rect(to_screen(min_x, max_y), to_screen(max_x, min_y), BOUNDS_COLOR, OVERLAY_BLEND);
        }
    }
    if settings.show_emitter {
        let (x, y) = view.to_screen(simulation.emitter_position());
        canvas.fill_square(x, y, EMITTER_MARKER_HALF_PX, EMITTER_COLOR, OVERLAY_BLEND);
    }
    canvas
}

/// The texture lookup of part `part` of a scene.
fn part_textures<'a>(scene: &'a PreviewScene, part: usize) -> Box<dyn Fn(&str) -> Option<&'a RasterTexture> + 'a> {
    Box::new(move |name| scene.texture(part, name))
}

/// A canvas as an image.
pub fn canvas_image(canvas: &Canvas) -> RgbaImage {
    RgbaImage::from_raw(canvas.width as u32, canvas.height as u32, canvas.to_rgba8()).expect("canvas size matches")
}

/// `count` frame times spread evenly over the timeline, the last at its end.
pub fn export_times(settings: &SimulationSettings, count: usize) -> Vec<u32> {
    let count = count.clamp(1, MAX_EXPORT_FRAMES) as u32;
    let end = settings.timeline_ms();
    (1..=count).map(|index| end * index / count).collect()
}

/// Renders the frames at `times` (ascending) in one simulation run.
pub fn render_frames(
    scene: &PreviewScene,
    settings: &SimulationSettings,
    view: &OrthoView,
    view_settings: &ViewSettings,
    hue_atlas: Option<&RasterTexture>,
    times: &[u32],
) -> Vec<RgbaImage> {
    let mut simulation = PreviewSimulation::new(scene, settings, view_settings.orbit);
    simulation.set_lighting(view_settings.lighting);
    times
        .iter()
        .map(|&time| {
            simulation.advance_to(time);
            canvas_image(&render_canvas(scene, &simulation, view, view_settings, hue_atlas))
        })
        .collect()
}

/// The hue atlas of hued NIF particles: `build/hues/hues.dds` of the client's `Hues.uop` (1024 x 1024 A8R8G8B8).
pub fn load_hue_atlas(client_dir: &Path) -> Result<RasterTexture, String> {
    let path = crate::app::find_client_file_case_insensitive(client_dir, HUES_UOP_NAME)
        .ok_or_else(|| format!("{HUES_UOP_NAME} is not in {}", client_dir.display()))?;
    let package = UopPackage::load_with_mode(&path, LoadMode::Lazy)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let payload = package
        .unpack_file_by_hash(hues_atlas_hash())
        .map_err(|error| format!("failed to unpack the hue atlas: {error}"))?
        .ok_or_else(|| format!("{} has no hue atlas", path.display()))?;
    let image = decode_texture(&payload)?;
    let (width, height) = image.dimensions();
    RasterTexture::from_rgba8(width as usize, height as usize, image.as_raw()).ok_or_else(|| "bad hue atlas size".to_string())
}

/// Frames side by side, [`CONTACT_SHEET_COLUMNS`] per row.
pub fn contact_sheet(frames: &[RgbaImage]) -> RgbaImage {
    let (cell_width, cell_height) = frames.first().map_or((1, 1), RgbaImage::dimensions);
    let columns = CONTACT_SHEET_COLUMNS.min(frames.len()).max(1);
    let rows = frames.len().div_ceil(columns).max(1);
    let width = columns as u32 * (cell_width + CONTACT_SHEET_GAP_PX) + CONTACT_SHEET_GAP_PX;
    let height = rows as u32 * (cell_height + CONTACT_SHEET_GAP_PX) + CONTACT_SHEET_GAP_PX;
    let mut sheet = RgbaImage::from_pixel(width, height, CONTACT_SHEET_GAP_COLOR);
    for (index, frame) in frames.iter().enumerate() {
        let x = (index % columns) as u32 * (cell_width + CONTACT_SHEET_GAP_PX) + CONTACT_SHEET_GAP_PX;
        let y = (index / columns) as u32 * (cell_height + CONTACT_SHEET_GAP_PX) + CONTACT_SHEET_GAP_PX;
        image::imageops::replace(&mut sheet, frame, i64::from(x), i64::from(y));
    }
    sheet
}

/// Writes `frames` as `<stem>_0000.png`, ... into `dir`; returns the number written.
pub fn write_png_sequence(frames: &[RgbaImage], dir: &Path, stem: &str) -> Result<usize, String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    for (index, frame) in frames.iter().enumerate() {
        let path = dir.join(format!("{stem}_{index:04}.png"));
        frame.save(&path).map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    Ok(frames.len())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::logic::effect_files::is_nif_name;
    use crate::logic::nif_editor::tests::{test_nif_bytes, TEST_MESH_HALF_SIZE};
    use uocf::enhanced::effect_definition::{
        EffectMapEntry, LodEffect, RandomEffect, RandomEffectEntry, RoundRobinEffect, SoundEffect,
    };

    /// A script with one additive sequence emitting from the start.
    pub(crate) const SPARK_SCRIPT: &str = "particlesystem \"spark\" 1\n{\neventsequence \"a\"\n{\ntype = point\n\
        texture = \"spark.tga\"\nsourceBlendMode = ALPHA_SRCALPHA\ndestBlendMode = ALPHA_ONE\nlifetime = 1\n\
        emitRate = 40\nnumParticles = 64\ninitial scale = 1\ninitial velocity = XYZ(random(-1, 1), 0, 2)\n}\n}\n";
    const SCRIPT_NAME: &str = "spark.ems";
    const NIF_NAME: &str = "smoke.nif";
    const SCRIPT_INDEX: u32 = 1;
    const NIF_INDEX: u32 = 2;
    const SCRUB_TIME_MS: u32 = 1000;
    const TEST_HUE: u32 = 5;
    /// Every texel of the test hue atlas.
    const TEST_HUE_COLOR: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

    pub(crate) fn dictionary() -> UoStringDictionary {
        let strings = [SCRIPT_NAME, NIF_NAME];
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for value in strings {
            payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
            payload.extend_from_slice(value.as_bytes());
        }
        UoStringDictionary::from_payload_bytes(&payload, "test").unwrap()
    }

    fn particle(name_index: u32) -> EffectDefinition {
        let EffectDefinition::Particle(mut particle) = DefinitionKind::Particle.new_definition() else { unreachable!() };
        particle.name_index = name_index;
        particle.looping = true;
        EffectDefinition::Particle(particle)
    }

    fn files(name: &str) -> Option<Arc<[u8]>> {
        match name.to_ascii_lowercase().as_str() {
            SCRIPT_NAME => Some(Arc::from(SPARK_SCRIPT.as_bytes())),
            NIF_NAME => Some(Arc::from(test_nif_bytes())),
            _ => None,
        }
    }

    fn scene(definitions: &[EffectDefinition], settings: &SimulationSettings) -> PreviewScene {
        let strings = dictionary();
        PreviewScene::build(definitions, settings, &PreviewInput { strings: Some(&strings), payload: &files, nif_drafts: None })
    }

    #[test]
    fn the_script_compiles_and_plays() {
        let settings = SimulationSettings::default();
        let scene = scene(&[particle(SCRIPT_INDEX)], &settings);
        assert!(matches!(scene.parts[0].status, PartStatus::Chaos { .. }), "{:?}", scene.parts[0].status);
        assert!(PreviewSimulation::at(&scene, &settings, CameraOrbit::default(), SCRUB_TIME_MS).particle_count() > 0);
        assert_eq!(scene.texture_problems.len(), 1, "spark.tga is missing: {:?}", scene.texture_problems);
    }

    #[test]
    fn chaos_mesh_particles_draw_their_nif() {
        const MESH_SCRIPT_NAME: &str = "mesh.ems";
        let script = format!(
            "particlesystem \"mesh\" 1\n{{\neventsequence \"a\"\n{{\ntype = mesh\nmeshName = \"{NIF_NAME}\"\n\
             lifetime = 10\nburst = 1\n}}\n}}\n"
        );
        let payload = |name: &str| match name {
            MESH_SCRIPT_NAME => Some(Arc::from(script.as_bytes())),
            _ => files(name),
        };
        let settings = SimulationSettings::default();
        let input = PreviewInput { strings: None, payload: &payload, nif_drafts: None };
        let scene = PreviewScene::build_script(MESH_SCRIPT_NAME, &settings, &input);
        assert!(matches!(scene.parts[0].status, PartStatus::Chaos { .. }), "{:?}", scene.parts[0].status);
        let frame = PreviewSimulation::at(&scene, &settings, CameraOrbit::default(), SCRUB_TIME_MS).render();
        assert!(frame.meshes.is_empty(), "the NIF resolves: no outlines");
        assert!(!frame.mesh_draws.is_empty(), "the particle draws its NIF");
    }

    #[test]
    fn parts_report_nif_missing_and_unresolved_names() {
        let settings = SimulationSettings::default();
        let sound = EffectDefinition::Sound(SoundEffect { sound_id: 1, value_08: 1.0 });
        let scene = scene(&[particle(NIF_INDEX), particle(99), sound], &settings);
        assert!(
            matches!(&scene.parts[0].status, PartStatus::Nif { name, meshes: 1, .. } if name == NIF_NAME),
            "{:?}",
            scene.parts[0].status
        );
        assert!(matches!(scene.parts[1].status, PartStatus::NameUnresolved { name_index: 99 }));
        assert_eq!(scene.parts[2].status, PartStatus::NotVisual);
        assert!(scene.has_playing_parts());
        let broken = |name: &str| (name == NIF_NAME).then(|| Arc::from(&b"Gamebryo File Format, Version 20.0.0.4\n"[..]));
        let strings = dictionary();
        let input = PreviewInput { strings: Some(&strings), payload: &broken, nif_drafts: None };
        let rejected = PreviewScene::build(&[particle(NIF_INDEX)], &settings, &input);
        assert!(matches!(rejected.parts[0].status, PartStatus::NifRejected { .. }));
        assert!(!rejected.has_playing_parts());
        // A draft replaces the payload.
        let mut draft = crate::logic::nif_editor::tests::test_nif();
        draft.blocks.truncate(crate::logic::nif_editor::tests::SHAPE_BLOCK);
        let draft = Arc::new(draft);
        let drafts = |name: &str| (name == NIF_NAME).then(|| Arc::clone(&draft));
        let input = PreviewInput { strings: Some(&strings), payload: &broken, nif_drafts: Some(&drafts) };
        let drafted = PreviewScene::build(&[particle(NIF_INDEX)], &settings, &input);
        assert!(matches!(drafted.parts[0].status, PartStatus::Nif { meshes: 0, .. }), "{:?}", drafted.parts[0].status);
    }

    /// With an object hue, only NIF particles whose definition sets `hued` play hued, drawn through the atlas.
    #[test]
    fn hued_nif_parts_take_the_hue_and_chaos_parts_never() {
        let hued = |name_index| {
            let EffectDefinition::Particle(mut particle) = particle(name_index) else { unreachable!() };
            particle.hued = true;
            EffectDefinition::Particle(particle)
        };
        let definitions = [hued(SCRIPT_INDEX), hued(NIF_INDEX), particle(NIF_INDEX)];
        let hue_of = |scene: &PreviewScene| -> Vec<Option<HueMode>> {
            let mode = |status: &PartStatus| match status {
                PartStatus::Chaos { hue, .. } | PartStatus::Nif { hue, .. } => Some(*hue),
                _ => None,
            };
            scene.parts.iter().map(|part| mode(&part.status)).collect()
        };
        let plain_settings = SimulationSettings::default();
        assert_eq!(hue_of(&scene(&definitions, &plain_settings)), [Some(HueMode::NoHue); 3]);
        let settings = SimulationSettings { hue: TEST_HUE, ..plain_settings };
        let hued_scene = scene(&definitions, &settings);
        assert_eq!(hue_of(&hued_scene), [Some(HueMode::Chaos), Some(HueMode::Hued), Some(HueMode::FlagNotSet)]);
        assert!(hued_scene.parts[1].status.describe().ends_with("hued (half speed)"));
        let simulation = PreviewSimulation::at(&hued_scene, &settings, CameraOrbit::default(), SCRUB_TIME_MS);
        let hued_parts: Vec<bool> = (0..definitions.len()).map(|part| simulation.hued_position(part).is_some()).collect();
        assert_eq!(hued_parts, [false, true, false]);

        // The hued mesh (the triangle covers the effect position) is drawn in the atlas colour.
        let only_hued = scene(&definitions[1..2], &settings);
        let simulation = PreviewSimulation::at(&only_hued, &settings, CameraOrbit::default(), SCRUB_TIME_MS);
        let view_settings = ViewSettings { show_emitter: false, ..Default::default() };
        let bounds = fit_bounds(&only_hued, &settings);
        let position = simulation.hued_position(0).expect("the part plays hued");
        let camera = CameraOrbit::default().camera();
        for corner in [camera.right + camera.up, camera.right - camera.up].map(|axis| axis * HUED_QUAD_HALF_SIZE) {
            for point in [position + corner, position - corner] {
                let ([low, high], point) = (bounds.map(Vec3::to_array), point.to_array());
                let inside = (0..3).all(|axis| low[axis] <= point[axis] && point[axis] <= high[axis]);
                assert!(inside, "the hue quad corner {point:?} is in the bounds {bounds:?}");
            }
        }
        let view = preview_view(bounds, &view_settings, PREVIEW_RESOLUTION_PX, PREVIEW_RESOLUTION_PX);
        let side = uocf::enhanced::hues::HUES_ATLAS_WIDTH as usize;
        let atlas = RasterTexture::from_texels(side, side, vec![TEST_HUE_COLOR; side * side]).expect("atlas size");
        let hued_canvas = render_canvas(&only_hued, &simulation, &view, &view_settings, Some(&atlas));
        assert!(hued_canvas.pixels.contains(&TEST_HUE_COLOR));
        let unhued_canvas = render_canvas(&only_hued, &simulation, &view, &view_settings, None);
        assert!(!unhued_canvas.pixels.contains(&TEST_HUE_COLOR), "without the atlas the part is drawn plain");
    }

    /// A property flag keyed by the hue plays the case of the preview hue, else its default children.
    #[test]
    fn property_flags_pick_the_case_of_the_object_hue() {
        let sound = EffectDefinition::Sound(SoundEffect { sound_id: 1, value_08: 1.0 });
        let flag = EffectDefinition::PropertyFlag(PropertyFlagEffect {
            mode: PropertyFlagMode::Property,
            mask: 0,
            property: PROPERTY_FLAG_OBJECT_HUE,
            default_children: vec![sound],
            cases: vec![
                EffectMapEntry { key: NO_HUE, children: vec![particle(SCRIPT_INDEX)] },
                EffectMapEntry { key: TEST_HUE, children: vec![particle(NIF_INDEX)] },
            ],
        });
        let played = |hue: u32| {
            let scene = scene(std::slice::from_ref(&flag), &SimulationSettings { hue, ..Default::default() });
            let part = &scene.parts[0];
            (part.path.last().unwrap().list, part.status.plays())
        };
        assert_eq!(played(NO_HUE), (ChildList::Case(0), true));
        assert_eq!(played(TEST_HUE), (ChildList::Case(1), true));
        assert_eq!(played(TEST_HUE + 1), (ChildList::Default, false));
    }

    /// Tileart particles with attach mode 2 do not play on the world object unless the preview asks for them.
    #[test]
    fn tileart_projectile_particles_play_only_on_request() {
        let mut thrown = particle(SCRIPT_INDEX);
        if let EffectDefinition::Particle(particle) = &mut thrown {
            particle.attach_mode = PARTICLE_ATTACH_PROJECTILE_ONLY;
        }
        let strings = dictionary();
        let input = PreviewInput { strings: Some(&strings), payload: &files, nif_drafts: None };
        let tileart = |settings: &SimulationSettings| {
            PreviewScene::build_tileart(std::slice::from_ref(&thrown), settings, &input)
        };
        let skipped = tileart(&SimulationSettings::default());
        assert!(matches!(&skipped.parts[0].status, PartStatus::ProjectileOnly { name } if name == SCRIPT_NAME));
        assert!(!skipped.has_playing_parts());
        let forced = tileart(&SimulationSettings { play_projectile_only: true, ..Default::default() });
        assert!(forced.parts[0].status.plays());
        assert!(scene(std::slice::from_ref(&thrown), &SimulationSettings::default()).parts[0].status.plays());
    }

    /// A Chaos script and a NIF mesh play together; the toggles hide particles or meshes, the orbit turns the
    /// view.
    #[test]
    fn mixed_entries_play_every_part() {
        let settings = SimulationSettings::default();
        let scene = scene(&[particle(SCRIPT_INDEX), particle(NIF_INDEX)], &settings);
        assert!(scene.parts.iter().all(|part| part.status.plays()));
        let bounds = fit_bounds(&scene, &settings);
        let half = TEST_MESH_HALF_SIZE;
        assert!(bounds[0].x <= -half && bounds[1].x >= half && bounds[0].z <= -half, "{bounds:?}");
        let draw = |view_settings: ViewSettings| {
            let view = preview_view(bounds, &view_settings, PREVIEW_RESOLUTION_PX, PREVIEW_RESOLUTION_PX);
            let mut simulation = PreviewSimulation::at(&scene, &settings, view_settings.orbit, SCRUB_TIME_MS);
            simulation.set_lighting(view_settings.lighting);
            assert_eq!(simulation.render_parts().len(), 2);
            canvas_image(&render_canvas(&scene, &simulation, &view, &view_settings, None))
        };
        let plain = ViewSettings { show_emitter: false, ..Default::default() };
        let background = DEFAULT_BACKGROUND.map(|channel| (channel * 255.0).round() as u8);
        let drawn = |image: &RgbaImage| image.pixels().filter(|pixel| pixel.0 != background).count();
        let both = draw(plain);
        let meshes = draw(ViewSettings { show_particles: false, ..plain });
        let particles = draw(ViewSettings { show_meshes: false, ..plain });
        let nothing = draw(ViewSettings { show_meshes: false, show_particles: false, ..plain });
        assert_eq!(drawn(&nothing), 0);
        assert!(drawn(&meshes) > 0 && drawn(&particles) > 0);
        assert!(drawn(&both) > drawn(&meshes).max(drawn(&particles)));
        // Seen edge-on from the east, the triangle (in the x-z plane) disappears.
        let east = CameraOrbit { yaw_degrees: -90.0, pitch_degrees: 0.0 };
        let edge_on = draw(ViewSettings { show_particles: false, orbit: east, ..plain });
        assert!(drawn(&edge_on) < drawn(&meshes) / 4, "{} vs {}", drawn(&edge_on), drawn(&meshes));
        assert_eq!(CameraOrbit::default().turned(270.0, 90.0), CameraOrbit { yaw_degrees: -90.0, pitch_degrees: MAX_CAMERA_PITCH_DEGREES });
    }

    #[test]
    fn selectors_resolve_like_one_play() {
        let settings = SimulationSettings { lod_level: 2, ..Default::default() };
        let lod = EffectDefinition::Lod(LodEffect {
            levels: vec![
                EffectMapEntry { key: 3, children: vec![particle(NIF_INDEX)] },
                EffectMapEntry { key: 1, children: vec![particle(SCRIPT_INDEX)] },
            ],
        });
        let round_robin = EffectDefinition::RoundRobin(RoundRobinEffect { children: vec![particle(SCRIPT_INDEX), particle(NIF_INDEX)] });
        let random = EffectDefinition::Random(RandomEffect {
            entries: vec![
                RandomEffectEntry { weight: RANDOM_WEIGHT_TOTAL, children: vec![particle(SCRIPT_INDEX)] },
                RandomEffectEntry { weight: 0.0, children: vec![particle(NIF_INDEX)] },
            ],
        });
        let scene = scene(&[lod, round_robin, random], &settings);
        let paths: Vec<_> = scene.parts.iter().map(|part| path_label(&part.path)).collect();
        assert_eq!(paths, ["0/level#1/0", "1/0", "2/entry 0/0"]);
        assert!(scene.parts.iter().all(|part| part.status.plays()));
    }

    #[test]
    fn scrubbing_resimulates_the_same_frame() {
        let settings = SimulationSettings { move_speed: DEFAULT_MOVE_SPEED, ..Default::default() };
        let definitions = [particle(SCRIPT_INDEX), particle(SCRIPT_INDEX)];
        let scene = scene(&definitions, &settings);
        let mut played = PreviewSimulation::new(&scene, &settings, CameraOrbit::default());
        for target in (0..=SCRUB_TIME_MS).step_by(37) {
            played.advance_to(target);
        }
        let scrubbed = PreviewSimulation::at(&scene, &settings, CameraOrbit::default(), SCRUB_TIME_MS);
        assert_eq!(played.time_ms(), scrubbed.time_ms());
        assert!(!scrubbed.render().sprites.is_empty());
        assert_eq!(frame_content(&played.render()), frame_content(&scrubbed.render()));
        let rebuilt = scene_frame(&definitions, &settings);
        assert_eq!(rebuilt, frame_content(&scrubbed.render()), "a rebuilt scene with the same seed draws the same");
        let other_seed = SimulationSettings { seed: settings.seed + 1, ..settings };
        assert_ne!(scene_frame(&definitions, &other_seed), rebuilt);
    }

    type FrameContent = (Vec<particle_sim::DrawBatch>, Vec<particle_sim::SpriteInstance>, Vec<particle_sim::MeshInstance>);

    fn frame_content(frame: &RenderFrame) -> FrameContent {
        (frame.batches.clone(), frame.sprites.clone(), frame.meshes.clone())
    }

    fn scene_frame(definitions: &[EffectDefinition], settings: &SimulationSettings) -> FrameContent {
        frame_content(&PreviewSimulation::at(&scene(definitions, settings), settings, CameraOrbit::default(), SCRUB_TIME_MS).render())
    }

    #[test]
    fn view_fits_and_exports_frames() {
        let settings = SimulationSettings::default();
        let scene = scene(&[particle(SCRIPT_INDEX)], &settings);
        let bounds = fit_bounds(&scene, &settings);
        assert!(bounds[1].z > bounds[0].z, "the particles rise: {bounds:?}");
        let view_settings = ViewSettings { show_bounds: true, ..Default::default() };
        let view = preview_view(bounds, &view_settings, PREVIEW_RESOLUTION_PX, PREVIEW_RESOLUTION_PX);
        let times = export_times(&settings, DEFAULT_EXPORT_FRAMES);
        assert_eq!(times.len(), DEFAULT_EXPORT_FRAMES);
        assert_eq!(*times.last().unwrap(), settings.timeline_ms());
        let frames = render_frames(&scene, &settings, &view, &view_settings, None, &times);
        let background = DEFAULT_BACKGROUND.map(|channel| (channel * 255.0).round() as u8);
        assert!(frames[DEFAULT_EXPORT_FRAMES / 2].pixels().any(|pixel| pixel.0 != background), "something is drawn");
        let sheet = contact_sheet(&frames);
        let rows = DEFAULT_EXPORT_FRAMES.div_ceil(CONTACT_SHEET_COLUMNS) as u32;
        assert_eq!(sheet.height(), rows * (PREVIEW_RESOLUTION_PX as u32 + CONTACT_SHEET_GAP_PX) + CONTACT_SHEET_GAP_PX);
    }

    /// Particle name part preferred for the preview rendered from the real client.
    const REAL_PREVIEW_NAME_PART: &str = "flamestrike";
    const REAL_PREVIEW_FRAME_FILE: &str = "effects_preview_frame.png";
    const REAL_PREVIEW_SHEET_FILE: &str = "effects_preview_contact_sheet.png";
    const ADDED_SCRIPT_NAME: &str = "uocf_inspector_test.ems";
    const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";

    /// Loads both client effect packages, edits them, saves both into a scratch directory and reads the copies
    /// back. When `UOCF_INSPECTOR_TEST_SCRATCH_DIR` is set, it also leaves a rendered preview frame and contact
    /// sheet of a real entry there.
    #[test]
    #[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
    fn client_effects_edit_save_and_preview() {
        use crate::logic::effect_files::{canonical_script, latin1_decode, EffectFiles, FileKind, FileStatus};
        use crate::logic::effects_editor::tests::{scratch_dir, EC_CLIENT_DIR_ENV, SCRATCH_DIR_ENV};
        use crate::logic::effects_editor::{
            add_entry, duplicate_entry, next_free_entry_id, open_collection, save_collection,
            CollectionSource, EffectsDocument, EffectsSelection, FileOverride,
        };
        use uocf::enhanced::effect_definition::{EFFECT_DEFINITION_COLLECTION_UOP_NAME, EffectDefinitionCollection};
        use std::collections::BTreeMap;
        use std::path::PathBuf;
        use uocf::enhanced::effect_texture::{effect_texture_hash, EFFECT_TEXTURE_UOP_NAME};

        let client_dir = PathBuf::from(std::env::var_os(EC_CLIENT_DIR_ENV).expect("set UO_EC_CLIENT_DIR"));
        let collection_source = client_dir.join(EFFECT_DEFINITION_COLLECTION_UOP_NAME);
        let files_source = client_dir.join(EFFECT_TEXTURE_UOP_NAME);
        let strings = UoStringDictionary::load(&client_dir.join(STRING_DICTIONARY_UOP_NAME)).unwrap();
        let original = open_collection(&collection_source).unwrap();
        let files = EffectFiles::open(&files_source, Some(&strings)).unwrap();
        let mut document =
            EffectsDocument::new(original.clone(), CollectionSource::ClientPackage(collection_source.clone()));

        // Collection edits: a new entry with a definition and a copy of the first entry.
        let EffectsSelection::Entry { entry: added } =
            document.edit_collection(None, |collection| add_entry(collection, None)).unwrap()
        else {
            panic!("the new entry is selected")
        };
        document.edit_collection(None, |collection| {
            collection.entries[added].definitions.push(DefinitionKind::Sound.new_definition())
        });
        document
            .edit_collection(None, |collection| {
                let id = next_free_entry_id(collection);
                duplicate_entry(collection, 0, id)
            })
            .unwrap();
        // File edits: the first Chaos script in canonical form and a new script.
        let rows = files.rows(document.files());
        let script = rows.iter().find(|row| row.kind == FileKind::Chaos && row.name.is_some()).expect("a named script");
        let script_name = script.name.clone().unwrap();
        let canonical = canonical_script(&latin1_decode(&files.payload(document.files(), script.hash).unwrap())).unwrap();
        let canonical: Arc<[u8]> = Arc::from(canonical.into_bytes());
        let added_payload: Arc<[u8]> = Arc::from(SPARK_SCRIPT.as_bytes());
        document.edit(None, |data| {
            data.files.insert(script.hash, FileOverride { name: script_name.clone(), payload: Some(Arc::clone(&canonical)) });
            let hash = effect_texture_hash(ADDED_SCRIPT_NAME);
            data.files.insert(hash, FileOverride { name: ADDED_SCRIPT_NAME.to_string(), payload: Some(Arc::clone(&added_payload)) });
        });
        assert!(document.collection_dirty() && document.files_dirty());
        let edited: EffectDefinitionCollection = document.collection().clone();

        let dir = scratch_dir("client");
        let protected = [collection_source.as_path(), files_source.as_path()];
        let collection_output = dir.join(EFFECT_DEFINITION_COLLECTION_UOP_NAME);
        let changes = save_collection(&edited, &collection_source, &collection_output, &protected).unwrap();
        assert_eq!(changes.added, 2);
        // The package keeps its own entry order: compare by entry.
        let by_key = |collection: EffectDefinitionCollection| -> BTreeMap<u64, Vec<EffectDefinition>> {
            collection.entries.into_iter().map(|entry| (entry.key.hash(), entry.definitions)).collect()
        };
        assert_eq!(by_key(open_collection(&collection_output).unwrap()), by_key(edited.clone()));
        assert!(save_collection(&edited, &collection_source, &collection_source, &protected).is_err());

        let files_output = dir.join(EFFECT_TEXTURE_UOP_NAME);
        let report = files.save_as(document.files(), &files_output, &protected).unwrap();
        assert_eq!((report.replaced, report.added), (1, 1));
        assert!(files.save_as(document.files(), &files_source, &protected).is_err());
        let saved = EffectFiles::open(&files_output, Some(&strings)).unwrap();
        let no_overrides = BTreeMap::new();
        assert_eq!(saved.payload(&no_overrides, script.hash).as_deref(), Some(canonical.as_ref()));
        let added_hash = effect_texture_hash(ADDED_SCRIPT_NAME);
        assert_eq!(saved.payload(&no_overrides, added_hash).as_deref(), Some(added_payload.as_ref()));
        assert!(saved.rows(&no_overrides).iter().all(|row| row.status == FileStatus::Original));
        assert_eq!(open_collection(&collection_source).unwrap(), original, "the client file is unchanged");
        std::fs::remove_dir_all(dir).unwrap();

        // Preview of a real entry, preferring the flame strike.
        let settings = SimulationSettings::default();
        let payload = |name: &str| files.payload(&no_overrides, effect_texture_hash(name));
        let input = PreviewInput { strings: Some(&strings), payload: &payload, nif_drafts: None };
        let names = |definitions: &[EffectDefinition]| -> Vec<String> {
            let mut names = Vec::new();
            collect_particle_names(definitions, &strings, &mut names);
            names
        };
        let entry = original
            .entries
            .iter()
            .filter(|entry| names(&entry.definitions).iter().any(|name| name.contains(REAL_PREVIEW_NAME_PART)))
            .chain(original.entries.iter())
            .find(|entry| PreviewScene::build(&entry.definitions, &settings, &input).has_playing_parts())
            .expect("an entry plays a Chaos particle");
        let scene = PreviewScene::build(&entry.definitions, &settings, &input);
        assert!(scene.texture_problems.is_empty(), "{:?}", scene.texture_problems);
        let view_settings = ViewSettings::default();
        let view = preview_view(fit_bounds(&scene, &settings), &view_settings, PREVIEW_RESOLUTION_PX, PREVIEW_RESOLUTION_PX);
        let frames = render_frames(&scene, &settings, &view, &view_settings, None, &export_times(&settings, DEFAULT_EXPORT_FRAMES));
        let background = DEFAULT_BACKGROUND.map(|channel| (channel * 255.0).round() as u8);
        assert!(frames.iter().any(|frame| frame.pixels().any(|pixel| pixel.0 != background)), "something is drawn");
        if let Some(base) = std::env::var_os(SCRATCH_DIR_ENV) {
            let base = PathBuf::from(base);
            frames[DEFAULT_EXPORT_FRAMES / 2].save(base.join(REAL_PREVIEW_FRAME_FILE)).unwrap();
            contact_sheet(&frames).save(base.join(REAL_PREVIEW_SHEET_FILE)).unwrap();
            let shown: Vec<String> = names(&entry.definitions);
            eprintln!("preview of collection entry {:?} ({shown:?}) written to {}", entry.key, base.display());
        }
    }

    /// NIF effects of the real client previewed headlessly: a projectile, an explosion and a mesh-only one.
    const REAL_NIF_NAMES: [&str; 2] = ["00014052_smallfireball.nif", "00014026_explosion3.nif"];
    const REAL_NIF_SHEET_PREFIX: &str = "nif_preview_";
    const TILEART_UOP_NAME: &str = "tileart.uop";
    /// The object hue of the hued NIF preview.
    const REAL_PREVIEW_HUE: u32 = 33;
    /// Zoom of the hued NIF preview (the effect is small).
    const REAL_HUED_PREVIEW_ZOOM: f32 = 4.0;
    /// The emitter speed edit: new speed = old * factor + offset (different from any old speed).
    const SPEED_EDIT_FACTOR: f32 = 2.0;
    const SPEED_EDIT_OFFSET: f32 = 1.0;
    const EMITTER_TYPE: &str = "NiPSysEmitter";
    const SPEED_FIELD: &str = "speed";

    /// Previews collection entries playing NIF effects (contact sheets into `UOCF_INSPECTOR_TEST_SCRATCH_DIR`),
    /// then edits the emitter speed of a NIF, saves `EffectTexture.uop` into a scratch directory and reads it
    /// back: the edit is there, every other file is byte-identical.
    #[test]
    #[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
    fn client_nif_effects_preview_edit_and_save() {
        use crate::logic::effect_files::{EffectFiles, FileKind};
        use crate::logic::effects_editor::tests::{scratch_dir, EC_CLIENT_DIR_ENV, SCRATCH_DIR_ENV};
        use crate::logic::effects_editor::{open_collection, CollectionSource, EffectsDocument};
        use crate::logic::nif_editor::{current_nif, edit_nif_file, FieldStep, NifEdit, ScalarValue};
        use std::collections::BTreeMap;
        use std::path::PathBuf;
        use uocf::enhanced::effect_definition::EFFECT_DEFINITION_COLLECTION_UOP_NAME;
        use uocf::enhanced::effect_texture::{effect_texture_hash, EFFECT_TEXTURE_UOP_NAME};
        use uocf::enhanced::hues::{HUES_ATLAS_HEIGHT, HUES_ATLAS_WIDTH};
        use uocf::enhanced::tileart::TileArtEntry;

        let client_dir = PathBuf::from(std::env::var_os(EC_CLIENT_DIR_ENV).expect("set UO_EC_CLIENT_DIR"));
        let strings = UoStringDictionary::load(&client_dir.join(STRING_DICTIONARY_UOP_NAME)).unwrap();
        let collection = open_collection(&client_dir.join(EFFECT_DEFINITION_COLLECTION_UOP_NAME)).unwrap();
        let files_source = client_dir.join(EFFECT_TEXTURE_UOP_NAME);
        let files = EffectFiles::open(&files_source, Some(&strings)).unwrap();
        let no_overrides = BTreeMap::new();
        let payload = |name: &str| files.payload(&no_overrides, effect_texture_hash(name));
        let input = PreviewInput { strings: Some(&strings), payload: &payload, nif_drafts: None };
        let settings = SimulationSettings::default();
        let names = |definitions: &[EffectDefinition]| -> Vec<String> {
            let mut names = Vec::new();
            collect_particle_names(definitions, &strings, &mut names);
            names
        };

        // A mesh-only NIF some entry plays.
        let mut nif_kinds: BTreeMap<String, bool> = BTreeMap::new();
        let mesh_only = collection.entries.iter().find_map(|entry| {
            names(&entry.definitions).into_iter().filter(|name| is_nif_name(name)).find(|name| {
                *nif_kinds.entry(name.clone()).or_insert_with(|| {
                    let Some(Ok(nif)) = payload(name).map(|bytes| NifFile::read(&bytes)) else { return false };
                    let desc = particle_sim::nif::compile(&nif, name).desc;
                    desc.particle_systems.is_empty() && !desc.meshes.is_empty()
                })
            })
        });
        let mesh_only = mesh_only.expect("an entry plays a mesh-only NIF");
        let base = std::env::var_os(SCRATCH_DIR_ENV).map(PathBuf::from);
        let view_settings = ViewSettings::default();
        let background = DEFAULT_BACKGROUND.map(|channel| (channel * 255.0).round() as u8);
        for target in REAL_NIF_NAMES.iter().copied().chain([mesh_only.as_str()]) {
            let entry = collection
                .entries
                .iter()
                .find(|entry| names(&entry.definitions).iter().any(|name| name == target))
                .unwrap_or_else(|| panic!("an entry plays {target}"));
            let scene = PreviewScene::build(&entry.definitions, &settings, &input);
            let nif_part = scene
                .parts
                .iter()
                .find(|part| matches!(&part.status, PartStatus::Nif { name, .. } if name.eq_ignore_ascii_case(target)))
                .unwrap_or_else(|| panic!("{target} plays: {:?}", scene.parts.iter().map(|part| part.status.describe()).collect::<Vec<_>>()));
            let view = preview_view(fit_bounds(&scene, &settings), &view_settings, PREVIEW_RESOLUTION_PX, PREVIEW_RESOLUTION_PX);
            let frames = render_frames(&scene, &settings, &view, &view_settings, None, &export_times(&settings, DEFAULT_EXPORT_FRAMES));
            assert!(frames.iter().any(|frame| frame.pixels().any(|pixel| pixel.0 != background)), "{target} draws");
            eprintln!(
                "entry {:?} with {target}: {}; texture problems {:?}",
                entry.key,
                nif_part.status.describe(),
                scene.texture_problems
            );
            if let Some(base) = &base {
                let stem = target.trim_end_matches(".nif");
                contact_sheet(&frames).save(base.join(format!("{REAL_NIF_SHEET_PREFIX}{stem}.png"))).unwrap();
            }
        }

        // A NIF particle whose definition sets `hued` (only tileart effects do), previewed without and with an
        // object hue.
        let atlas = load_hue_atlas(&client_dir).unwrap();
        assert_eq!((atlas.width, atlas.height), (HUES_ATLAS_WIDTH as usize, HUES_ATLAS_HEIGHT as usize));
        let hued_settings = SimulationSettings { hue: REAL_PREVIEW_HUE, ..settings };
        let hued_nif = |definitions: &[EffectDefinition]| {
            let mut found = None;
            for definition in definitions {
                definition.visit(&mut |definition| {
                    if let EffectDefinition::Particle(particle) = definition {
                        let name = particle.name(&strings).map(str::to_ascii_lowercase);
                        if particle.hued && found.is_none() {
                            found = name.filter(|name| is_nif_name(name));
                        }
                    }
                });
            }
            found
        };
        let tileart = UopPackage::load(client_dir.join(TILEART_UOP_NAME)).unwrap();
        let tiles: Vec<TileArtEntry> = tileart.iter_files().filter_map(|file| TileArtEntry::parse_raw(file).ok()).collect();
        let (tile, scene, hued_target) = tiles
            .iter()
            .filter_map(|tile| hued_nif(&tile.effects).map(|name| (tile, name)))
            .find_map(|(tile, name)| {
                let scene = PreviewScene::build(&tile.effects, &hued_settings, &input);
                let hued = scene.parts.iter().any(|part| matches!(&part.status, PartStatus::Nif { hue: HueMode::Hued, .. }));
                hued.then_some((tile, scene, name))
            })
            .expect("a tileart record plays a hued NIF particle");
        let plain_scene = PreviewScene::build(&tile.effects, &settings, &input);
        let hued_view_settings = ViewSettings { zoom: REAL_HUED_PREVIEW_ZOOM, ..view_settings };
        let view = preview_view(fit_bounds(&plain_scene, &settings), &hued_view_settings, PREVIEW_RESOLUTION_PX, PREVIEW_RESOLUTION_PX);
        let times = export_times(&settings, DEFAULT_EXPORT_FRAMES);
        let plain = render_frames(&plain_scene, &settings, &view, &hued_view_settings, None, &times);
        let hued = render_frames(&scene, &hued_settings, &view, &hued_view_settings, Some(&atlas), &times);
        assert!(hued.iter().any(|frame| frame.pixels().any(|pixel| pixel.0 != background)), "{hued_target} draws hued");
        assert_ne!(plain, hued, "the hue changes the frames");
        eprintln!(
            "hued tile 0x{:04X} with {hued_target}: {:?}",
            tile.tile_id,
            scene.parts.iter().map(|part| part.status.describe()).collect::<Vec<_>>()
        );
        if let Some(base) = &base {
            let stem = hued_target.trim_end_matches(".nif");
            contact_sheet(&plain).save(base.join(format!("{REAL_NIF_SHEET_PREFIX}{stem}.png"))).unwrap();
            contact_sheet(&hued).save(base.join(format!("{REAL_NIF_SHEET_PREFIX}{stem}_hue{REAL_PREVIEW_HUE}.png"))).unwrap();
        }

        // Edit the first emitter's speed of the small fireball, save into scratch and read back.
        let edited_name = REAL_NIF_NAMES[0];
        let hash = effect_texture_hash(edited_name);
        let mut document = EffectsDocument::new(collection.clone(), CollectionSource::None);
        let nif = current_nif(&document, &files, hash).unwrap();
        let (block, old_speed, field) = nif
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.block_type().is_a(EMITTER_TYPE))
            .find_map(|(index, block)| {
                block.fields().iter().enumerate().find_map(|(field, value)| match value.value {
                    gamebryo_nif::Value::F32(&speed) if value.name() == SPEED_FIELD => Some((index, speed, field)),
                    _ => None,
                })
            })
            .expect("the fireball has an emitter");
        let new_speed = old_speed * SPEED_EDIT_FACTOR + SPEED_EDIT_OFFSET;
        let edit = (vec![FieldStep::Field(field)], NifEdit::Set(ScalarValue::Float(new_speed)));
        edit_nif_file(&mut document, &files, hash, block, &[edit]).unwrap();
        assert!(document.nif_drafts().is_empty(), "a speed edit writes");
        let dir = scratch_dir("client_nif");
        let output = dir.join(EFFECT_TEXTURE_UOP_NAME);
        let report = files.save_as(document.files(), &output, &[files_source.as_path()]).unwrap();
        assert_eq!((report.replaced, report.added, report.removed), (1, 0, 0));
        let saved = EffectFiles::open(&output, Some(&strings)).unwrap();
        let back = NifFile::read(&saved.payload(&no_overrides, hash).unwrap()).unwrap();
        let speed = back.blocks[block].fields().iter().find_map(|value| match value.value {
            gamebryo_nif::Value::F32(&speed) if value.name() == SPEED_FIELD => Some(speed),
            _ => None,
        });
        assert_eq!(speed, Some(new_speed));
        let mut expected = Arc::unwrap_or_clone(nif);
        crate::logic::nif_editor::apply_edit(
            &mut expected,
            block,
            &[FieldStep::Field(field)],
            &NifEdit::Set(ScalarValue::Float(new_speed)),
        )
        .unwrap();
        assert_eq!(back, expected, "nothing else changed in the NIF");
        let rows = files.rows(&no_overrides);
        let mut compared = 0;
        for row in rows.iter().filter(|row| row.hash != hash) {
            assert_eq!(
                saved.payload(&no_overrides, row.hash),
                files.payload(&no_overrides, row.hash),
                "{} is byte-identical",
                row.label()
            );
            compared += 1;
        }
        assert_eq!(saved.rows(&no_overrides).len(), rows.len());
        assert!(rows.iter().any(|row| row.kind == FileKind::Nif));
        eprintln!("{edited_name}: block {block} speed {old_speed} -> {new_speed}; {compared} other files identical");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn collect_particle_names(definitions: &[EffectDefinition], strings: &UoStringDictionary, names: &mut Vec<String>) {
        for definition in definitions {
            if let EffectDefinition::Particle(particle) = definition {
                names.extend(particle.name(strings).map(str::to_ascii_lowercase));
            }
            for (_, children) in crate::logic::effects_editor::child_lists(definition) {
                collect_particle_names(children, strings, names);
            }
        }
    }
}
