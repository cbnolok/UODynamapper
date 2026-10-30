//! Effect preview from `effects_ec.uddp` alone: which particles of a root list play (resolving random, LOD,
//! round-robin, wait, property flag and user preference nodes the way one client play does), every system and
//! texture taken from the package, a fixed-step simulation that can be scrubbed (re-simulated from 0 with the same
//! seed), and software rendering with particle-sim's rasterizer. No egui here.
//!
//! The scene is the one the uocf inspector builds from client files (`effect_preview.rs` there), with the package
//! as its only input: Chaos parts get their MESH `.nif`s from the package systems, NIF parts their textures by
//! the names in their descs (`<nif>#<i>` for embedded pixels), and every part is placed with
//! [`EffectPlacement::seen_by`] the client camera. NIF and Chaos parts are drawn in one sorted draw group
//! ([`Canvas::draw_group`]). Hued NIF parts play as `UOHuedParticleEffectNif` (half speed, drawn through the hue
//! atlas: [`particle_sim::hued`]).

use std::collections::HashMap;
use std::sync::Arc;

use particle_sim::chaos::{ChaosParticleType, MeshSource};
use particle_sim::hued::{
    HUED_EFFECT_TIME_SCALE, HUED_QUAD_HALF_SIZE, HueMode, HuedRenderSettings, NO_HUE, draw_hued_quad, hue_mode,
    hue_quad_sort_key, offscreen_camera, render_hued_target,
};
use particle_sim::nif::default_camera;
use particle_sim::nif::desc::NifTextureSource;
use particle_sim::raster::{Canvas, GroupItem, OVERLAY_BLEND, OrthoView, RasterTexture, SceneDrawOptions, frame_bounds};
use particle_sim::render::{CameraFrame, RenderFrame};
use particle_sim::{
    ChaosEffectInstance, ChaosSystem, ChaosSystemDesc, EffectInstance, EffectPlacement, Mt19937, NifEffect,
    NifEffectInstance, NodeTransform, ParticleEffectParams, ParticleSystemInstance, Rng, Vec3,
};
use udd_assets::EffectsEcPackage;
use udd_assets::effects_ec::{
    EffectNode, ParticleNode, SYSTEM_FLAG_COMPILE_FAILED, SYSTEM_FLAG_MISSING, SYSTEM_KIND_CHAOS, normalize_resource_name,
};
use uocf::enhanced::effect_definition::{
    MAX_EFFECT_NESTING_DEPTH, PARTICLE_ATTACH_PROJECTILE_ONLY, PARTICLE_LOD_DEFAULT, PARTICLE_LOD_MAX,
    RANDOM_WEIGHT_TOTAL,
};

use super::model::{PROPERTY_FLAG_OBJECT_HUE, RootKey, node_type_name};
use super::texture::raster_texture;

/// Simulation step: the preview always advances in whole steps of this many milliseconds.
pub const PREVIEW_STEP_MS: u32 = 16;
pub const DEFAULT_PREVIEW_SEED: u32 = 0x5EED;
pub const DEFAULT_TIMELINE_S: f32 = 4.0;
pub const MIN_TIMELINE_S: f32 = 0.5;
pub const MAX_TIMELINE_S: f32 = 30.0;
/// Side of the square preview canvas, pixels.
pub const PREVIEW_RESOLUTION_PX: usize = 384;
/// Default UI particle LOD setting (High), as the client.
pub const DEFAULT_LOD_LEVEL: u32 = PARTICLE_LOD_DEFAULT;
pub const MAX_LOD_LEVEL: u32 = PARTICLE_LOD_MAX;
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 20.0;
pub const DEFAULT_BACKGROUND: [f32; 4] = [0.10, 0.10, 0.12, 1.0];
const MS_PER_SECOND: f32 = 1000.0;
/// Distance of the client camera from the view centre (only axial sprites see it).
const CAMERA_DISTANCE: f32 = 1000.0;
/// Fraction of the canvas left free around the fitted bounds.
const FIT_MARGIN: f32 = 0.08;
/// Smallest world extent fitted into the canvas (an effect with a single point).
const MIN_FIT_EXTENT: f32 = 4.0;
/// The fit samples the frame every this many steps.
const FIT_SAMPLE_EVERY_STEPS: u32 = 4;
/// Outline colour of meshes that are not drawn (a MESH `.nif` that does not resolve).
const MESH_MARKER_COLOR: [f32; 4] = [1.0, 0.0, 1.0, 1.0];
const EMITTER_COLOR: [f32; 4] = [0.9, 0.9, 0.2, 1.0];
const EMITTER_MARKER_HALF_PX: f32 = 2.5;

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

/// What the scene and its simulation depend on: a change rebuilds them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimulationSettings {
    pub seed: u32,
    pub loop_mode: LoopMode,
    /// Hue of the object the effect plays on ([`NO_HUE`]: none). NIF particles whose definition sets `hued` take
    /// it; hue-keyed PropertyFlag nodes pick their case by it.
    pub hue: u32,
    /// UI particle LOD setting (1..3) for LOD nodes.
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
            hue: NO_HUE,
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

/// How a frame is drawn: a change only redraws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSettings {
    pub background: [f32; 4],
    pub show_emitter: bool,
    /// Draw the particles (sprite batches).
    pub show_particles: bool,
    /// Draw the meshes (NIF geometry and mesh particles).
    pub show_meshes: bool,
    /// Multiplies the fitted scale.
    pub zoom: f32,
    pub resolution_px: usize,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            background: DEFAULT_BACKGROUND,
            show_emitter: true,
            show_particles: true,
            show_meshes: true,
            zoom: 1.0,
            resolution_px: PREVIEW_RESOLUTION_PX,
        }
    }
}

/// What is previewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewTarget {
    /// A collection entry, tileart record or sequence frame (tileart particles play on the world object).
    Root(RootKey),
    /// One system played as a particle with default fields.
    System(u32),
}

impl PreviewTarget {
    pub fn label(&self, package: &EffectsEcPackage) -> String {
        match self {
            Self::Root(key) => key.label(),
            Self::System(system) => format!("system {}", package.system_name(*system)),
        }
    }
}

/// Where a resource a part uses stands.
#[derive(Debug, Clone, PartialEq)]
pub enum ResourceState {
    Loaded,
    /// The package record says it has no payload (`MISSING`, `UNDECODED`, ... flags): a client data gap, drawn
    /// white (textures) or as outlines (meshes).
    NoPayload { flags: u8 },
    /// The payload does not read or decode (a corrupt package).
    Failed(String),
    /// The package has no record of this name.
    NotInPackage,
    /// Embedded pixels the packer did not rename to `<nif>#<i>`.
    NotRenamed,
}

/// A texture or MESH `.nif` a part uses, by the name its desc gives.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceUse {
    pub name: String,
    /// The package record (texture or system index).
    pub index: Option<u32>,
    pub state: ResourceState,
}

/// Why a part does or does not play.
#[derive(Debug, Clone, PartialEq)]
pub enum PartStatus {
    /// A Chaos particle that plays.
    Chaos { name: String, sequences: usize, hue: HueMode },
    /// A NIF effect that plays.
    Nif { name: String, particle_systems: usize, meshes: usize, hue: HueMode },
    /// `SYSTEM_FLAG_MISSING`: the file is not in the client; the client plays nothing.
    NotInClient { name: String },
    /// `SYSTEM_FLAG_COMPILE_FAILED`: the client rejects the file; it plays nothing.
    CompileFailed { name: String },
    /// The blob does not read or decode (a corrupt package).
    Unreadable { name: String, error: String },
    /// The particle names no system (the string dictionary lacks the name).
    NoSystem,
    /// A tileart particle with attach mode 2: not played on the world object (only in flight).
    ProjectileOnly { name: String },
    /// A sound, earthquake or position offset: nothing to draw.
    NotVisual { kind: &'static str },
}

impl PartStatus {
    pub fn plays(&self) -> bool {
        matches!(self, Self::Chaos { .. } | Self::Nif { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Chaos { name, sequences, hue } => format!("{name}: Chaos, {sequences} sequence(s){}", hue_note(*hue)),
            Self::Nif { name, particle_systems, meshes, hue } => {
                format!("{name}: NIF, {particle_systems} particle system(s), {meshes} mesh(es){}", hue_note(*hue))
            }
            Self::NotInClient { name } => format!("{name}: not in client (plays nothing)"),
            Self::CompileFailed { name } => format!("{name}: rejected by the client (compile failed, plays nothing)"),
            Self::Unreadable { name, error } => format!("{name}: unreadable blob ({error})"),
            Self::NoSystem => "particle without system (name not in the string dictionary)".to_owned(),
            Self::ProjectileOnly { name } => format!("{name}: attach mode 2, not played on the tileart object"),
            Self::NotVisual { kind } => format!("{kind}: not drawn"),
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

/// One node of the previewed list that the play reaches.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewPart {
    /// Path from the root list: `0`, `1/case 5/0`.
    pub path: String,
    /// The definition node (`None` for a single-system preview).
    pub node: Option<u32>,
    pub system: Option<u32>,
    pub status: PartStatus,
    pub textures: Vec<ResourceUse>,
    /// The MESH `.nif`s of a Chaos part.
    pub meshes: Vec<ResourceUse>,
}

/// A prepared system ready to instantiate.
#[derive(Clone)]
enum PreparedSystem {
    Chaos { system: Arc<ChaosSystem>, meshes: MeshSource },
    Nif(Arc<NifEffect>),
}

#[derive(Clone)]
struct PreparedPart {
    params: ParticleEffectParams,
    /// Plays as `UOHuedParticleEffectNif` ([`HueMode::Hued`]).
    hued: bool,
    seed: u32,
    system: PreparedSystem,
}

/// Decoded payloads shared by every scene of a package: textures, Chaos descs and prepared NIF effects by index.
#[derive(Default)]
pub struct ResourceCache {
    textures: HashMap<u32, Result<Arc<RasterTexture>, String>>,
    chaos: HashMap<u32, Result<Arc<ChaosSystemDesc>, String>>,
    nif: HashMap<u32, Result<Arc<NifEffect>, String>>,
}

impl ResourceCache {
    fn texture(&mut self, package: &EffectsEcPackage, index: u32) -> Result<Arc<RasterTexture>, String> {
        self.textures
            .entry(index)
            .or_insert_with(|| {
                let data = package.read_texture(index).map_err(|error| format!("{error:#}"))?;
                raster_texture(&data).map(Arc::new)
            })
            .clone()
    }

    fn chaos(&mut self, package: &EffectsEcPackage, index: u32) -> Result<Arc<ChaosSystemDesc>, String> {
        self.chaos
            .entry(index)
            .or_insert_with(|| package.read_chaos_desc(index).map(Arc::new).map_err(|error| format!("{error:#}")))
            .clone()
    }

    fn nif(&mut self, package: &EffectsEcPackage, index: u32) -> Result<Arc<NifEffect>, String> {
        self.nif
            .entry(index)
            .or_insert_with(|| {
                package.read_nif_desc(index).map(|desc| Arc::new(NifEffect::prepare(desc))).map_err(|error| format!("{error:#}"))
            })
            .clone()
    }
}

/// What a play of a root list draws: the reached parts, the choices made, the prepared systems and their textures.
pub struct PreviewScene {
    pub target: PreviewTarget,
    pub settings: SimulationSettings,
    pub parts: Vec<PreviewPart>,
    /// Choices made while resolving (random draws, LOD levels, cases, ...).
    pub notes: Vec<String>,
    prepared: Vec<PreparedPart>,
    /// Loaded textures by normalized name.
    textures: HashMap<String, Arc<RasterTexture>>,
}

impl PreviewScene {
    /// Resolves `target` the way one play does and prepares its particles from the package.
    pub fn build(
        package: &EffectsEcPackage,
        cache: &mut ResourceCache,
        target: PreviewTarget,
        settings: &SimulationSettings,
    ) -> Self {
        let mut builder = SceneBuilder {
            package,
            cache,
            settings,
            draw: Mt19937::new(settings.seed),
            tileart: matches!(target, PreviewTarget::Root(RootKey::Tile(_))),
            standalone: matches!(target, PreviewTarget::System(_)),
            scene: PreviewScene {
                target,
                settings: *settings,
                parts: Vec::new(),
                notes: Vec::new(),
                prepared: Vec::new(),
                textures: HashMap::new(),
            },
        };
        match target {
            PreviewTarget::Root(key) => {
                let roots = key.roots(package);
                if roots.is_empty() {
                    builder.scene.notes.push(format!("{} has no root list", key.label()));
                }
                builder.list(roots, "", 0);
            }
            PreviewTarget::System(system) => {
                if (system as usize) < package.systems().len() {
                    let particle = ParticleNode {
                        system: Some(system),
                        offset_x: 0.0,
                        offset_y: 0.0,
                        camera_offset: 0.0,
                        scale: 1.0,
                        duration: 0.0,
                        attach_mode: 0,
                        field_3c: 0,
                        looping: false,
                        hued: true,
                    };
                    let part = builder.particle(&particle, "0".to_owned(), None);
                    builder.scene.parts.push(part);
                } else {
                    builder.scene.notes.push(format!("system {system} is not in the package"));
                }
            }
        }
        builder.scene
    }

    /// Whether anything plays.
    pub fn has_playing_parts(&self) -> bool {
        !self.prepared.is_empty()
    }

    /// A loaded texture by any spelling of its name.
    pub fn texture(&self, name: &str) -> Option<&RasterTexture> {
        self.textures.get(&normalize_resource_name(name)).map(Arc::as_ref)
    }

    /// The names of the parts that play, in instance order (the order of [`PreviewSimulation`] parts).
    pub fn playing_part_count(&self) -> usize {
        self.prepared.len()
    }
}

struct SceneBuilder<'s> {
    package: &'s EffectsEcPackage,
    cache: &'s mut ResourceCache,
    settings: &'s SimulationSettings,
    /// Draws of random nodes, seeded by the preview seed.
    draw: Mt19937,
    /// Tileart effects played on the world object.
    tileart: bool,
    /// One system previewed on its own.
    standalone: bool,
    scene: PreviewScene,
}

impl SceneBuilder<'_> {
    fn list(&mut self, list: &[u32], parent: &str, depth: usize) {
        if depth >= MAX_EFFECT_NESTING_DEPTH {
            if !list.is_empty() {
                self.scene.notes.push(format!("{parent}: nested deeper than {MAX_EFFECT_NESTING_DEPTH} (cyclic?), not played"));
            }
            return;
        }
        for (index, &node) in list.iter().enumerate() {
            let path = if parent.is_empty() { index.to_string() } else { format!("{parent}/{index}") };
            self.node(node, path, depth);
        }
    }

    fn node(&mut self, index: u32, path: String, depth: usize) {
        let node = self.package.node(index);
        match node {
            EffectNode::Particle(particle) if self.tileart && particle.attach_mode == PARTICLE_ATTACH_PROJECTILE_ONLY => {
                if self.settings.play_projectile_only {
                    self.scene.notes.push(format!("{path} particle: attach mode 2, played although the client does not"));
                    let part = self.particle(&particle, path, Some(index));
                    self.scene.parts.push(part);
                } else {
                    let name = particle.system.map_or_else(String::new, |system| self.package.system_name(system).to_owned());
                    self.push_unplayed(path, index, particle.system, PartStatus::ProjectileOnly { name });
                }
            }
            EffectNode::Particle(particle) => {
                let part = self.particle(&particle, path, Some(index));
                self.scene.parts.push(part);
            }
            EffectNode::Sound { .. } | EffectNode::Earthquake { .. } | EffectNode::PositionOffset { .. } => {
                self.push_unplayed(path, index, None, PartStatus::NotVisual { kind: node_type_name(&node) });
            }
            EffectNode::RoundRobin { children } => {
                if !children.is_empty() {
                    self.scene.notes.push(format!("{path} round robin: child 0 (the first play)"));
                    self.list(&children[..1], &path, depth + 1);
                }
            }
            EffectNode::Wait { children, .. } => {
                self.scene.notes.push(format!("{path} wait: the first play passes (later plays are throttled)"));
                self.list(children, &path, depth + 1);
            }
            EffectNode::UserPreference { children, .. } => {
                self.scene.notes.push(format!("{path} user preference: played (no user settings here)"));
                self.list(children, &path, depth + 1);
            }
            EffectNode::Lod { levels } => {
                let setting = self.settings.lod_level;
                // Levels ascend by key: the last one not above the setting.
                match levels.iter().rev().find(|level| level.key <= setting) {
                    Some(level) => {
                        self.scene.notes.push(format!("{path} LOD: level {} for setting {setting}", level.key));
                        let children = self.package.list(level.list_first, level.list_count);
                        self.list(children, &format!("{path}/level {}", level.key), depth + 1);
                    }
                    None => self.scene.notes.push(format!("{path} LOD: no level <= {setting}, plays nothing")),
                }
            }
            EffectNode::PropertyFlag { property_mode, property, default_children, cases, .. } => {
                if !property_mode {
                    self.scene.notes.push(format!("{path} property flag (flag mask): no object flags, plays nothing"));
                } else if property != PROPERTY_FLAG_OBJECT_HUE {
                    self.scene.notes.push(format!("{path} property flag (property {property}): not modelled, default children"));
                    self.list(default_children, &format!("{path}/default"), depth + 1);
                } else {
                    let hue = self.settings.hue;
                    match cases.iter().find(|case| case.key == hue) {
                        Some(case) => {
                            self.scene.notes.push(format!("{path} property flag (hue): case {hue} for object hue {hue}"));
                            let children = self.package.list(case.list_first, case.list_count);
                            self.list(children, &format!("{path}/case {hue}"), depth + 1);
                        }
                        None => {
                            self.scene.notes.push(format!(
                                "{path} property flag (hue): no case for object hue {hue}, {} default child(ren)",
                                default_children.len()
                            ));
                            self.list(default_children, &format!("{path}/default"), depth + 1);
                        }
                    }
                }
            }
            EffectNode::Random { entries } => {
                let mut draw = self.draw.rand01() * RANDOM_WEIGHT_TOTAL;
                let rolled = draw;
                let chosen = entries.iter().position(|entry| {
                    let hit = draw < entry.weight();
                    draw -= entry.weight();
                    hit
                });
                match chosen {
                    Some(entry_index) => {
                        self.scene.notes.push(format!("{path} random: drew {rolled:.1}, entry {entry_index}"));
                        let entry = entries[entry_index];
                        let children = self.package.list(entry.list_first, entry.list_count);
                        self.list(children, &format!("{path}/entry {entry_index}"), depth + 1);
                    }
                    None => self.scene.notes.push(format!("{path} random: drew {rolled:.1}, no entry")),
                }
            }
        }
    }

    fn push_unplayed(&mut self, path: String, node: u32, system: Option<u32>, status: PartStatus) {
        self.scene.parts.push(PreviewPart { path, node: Some(node), system, status, textures: Vec::new(), meshes: Vec::new() });
    }

    fn particle(&mut self, particle: &ParticleNode, path: String, node: Option<u32>) -> PreviewPart {
        let mut part = PreviewPart { path, node, system: particle.system, status: PartStatus::NoSystem, textures: Vec::new(), meshes: Vec::new() };
        let Some(system) = particle.system else {
            return part;
        };
        let record = *self.package.system(system);
        let name = self.package.system_name(system).to_owned();
        part.status = if record.flags & SYSTEM_FLAG_MISSING != 0 {
            PartStatus::NotInClient { name }
        } else if record.flags & SYSTEM_FLAG_COMPILE_FAILED != 0 {
            PartStatus::CompileFailed { name }
        } else if record.kind == SYSTEM_KIND_CHAOS {
            self.chaos_particle(particle, system, name, &mut part)
        } else {
            self.nif_particle(particle, system, name, &mut part)
        };
        part
    }

    fn chaos_particle(&mut self, particle: &ParticleNode, system: u32, name: String, part: &mut PreviewPart) -> PartStatus {
        let desc = match self.cache.chaos(self.package, system) {
            Ok(desc) => desc,
            Err(error) => return PartStatus::Unreadable { name, error },
        };
        // MESH sequences name `.nif` scenes, which the package holds as NIF systems with their textures; the
        // other resources are textures.
        let mut meshes = HashMap::new();
        for resource in desc.resources() {
            let is_mesh = desc
                .sequences
                .iter()
                .any(|sequence| sequence.resource == resource && sequence.particle_type == ChaosParticleType::Mesh);
            if !is_mesh {
                let texture = self.texture(resource);
                part.textures.push(texture);
                continue;
            }
            let (index, state) = match self.package.system_by_name(resource) {
                None => (None, ResourceState::NotInPackage),
                Some(index) if !self.package.system(index).has_payload() => {
                    (Some(index), ResourceState::NoPayload { flags: self.package.system(index).flags })
                }
                Some(index) if self.package.system(index).kind == SYSTEM_KIND_CHAOS => {
                    (Some(index), ResourceState::Failed("a Chaos script, not a NIF".to_owned()))
                }
                Some(index) => match self.cache.nif(self.package, index) {
                    Ok(effect) => {
                        for texture in &effect.desc().textures {
                            let texture = self.nif_texture(&texture.source, &texture.name());
                            part.textures.push(texture);
                        }
                        meshes.insert(normalize_resource_name(resource), effect);
                        (Some(index), ResourceState::Loaded)
                    }
                    Err(error) => (Some(index), ResourceState::Failed(error)),
                },
            };
            if state != ResourceState::Loaded {
                self.scene.notes.push(format!("{name}: MESH {resource} does not resolve, drawn as outlines"));
            }
            part.meshes.push(ResourceUse { name: resource.to_owned(), index, state });
        }
        let meshes = MeshSource::new(move |mesh| meshes.get(&normalize_resource_name(mesh)).cloned());
        let sequences = desc.sequences.len();
        let seed = self.part_seed();
        let prepared = Arc::new(ChaosSystem::prepare((*desc).clone(), &mut Mt19937::new(seed)));
        let params = self.params(particle, None);
        self.scene.prepared.push(PreparedPart { params, hued: false, seed, system: PreparedSystem::Chaos { system: prepared, meshes } });
        PartStatus::Chaos { name, sequences, hue: hue_mode(true, particle.hued, self.settings.hue) }
    }

    fn nif_particle(&mut self, particle: &ParticleNode, system: u32, name: String, part: &mut PreviewPart) -> PartStatus {
        let effect = match self.cache.nif(self.package, system) {
            Ok(effect) => effect,
            Err(error) => return PartStatus::Unreadable { name, error },
        };
        let desc = effect.desc();
        for texture in &desc.textures {
            let texture = self.nif_texture(&texture.source, &texture.name());
            part.textures.push(texture);
        }
        let length = desc.animation_length();
        let fallback = if length > 0.0 { length } else { self.settings.timeline_s };
        let params = self.params(particle, self.standalone.then_some(fallback));
        let (particle_systems, meshes) = (desc.particle_systems.len(), desc.meshes.len());
        let hue = hue_mode(false, particle.hued, self.settings.hue);
        let seed = self.part_seed();
        self.scene.prepared.push(PreparedPart { params, hued: hue == HueMode::Hued, seed, system: PreparedSystem::Nif(effect) });
        PartStatus::Nif { name, particle_systems, meshes, hue }
    }

    /// A NIF texture: by name in the package (embedded pixels are stored as `<nif>#<i>` and renamed in the desc).
    fn nif_texture(&mut self, source: &NifTextureSource, name: &str) -> ResourceUse {
        match source {
            NifTextureSource::External { file_name } => self.texture(file_name),
            NifTextureSource::Embedded { .. } | NifTextureSource::Undecoded { .. } => {
                ResourceUse { name: name.to_owned(), index: None, state: ResourceState::NotRenamed }
            }
        }
    }

    /// A package texture by name, loaded into the scene when it has a payload that decodes.
    fn texture(&mut self, name: &str) -> ResourceUse {
        let Some(index) = self.package.texture_by_name(name) else {
            return ResourceUse { name: name.to_owned(), index: None, state: ResourceState::NotInPackage };
        };
        let record = *self.package.texture(index);
        let state = if !record.has_payload() {
            ResourceState::NoPayload { flags: record.flags }
        } else {
            match self.cache.texture(self.package, index) {
                Ok(texture) => {
                    self.scene.textures.insert(normalize_resource_name(name), texture);
                    ResourceState::Loaded
                }
                Err(error) => ResourceState::Failed(error),
            }
        };
        ResourceUse { name: name.to_owned(), index: Some(index), state }
    }

    /// The seed of the next prepared part.
    fn part_seed(&self) -> u32 {
        self.settings.seed.wrapping_add(self.scene.prepared.len() as u32)
    }

    /// The node's fields with the preview's loop override; `fallback_duration` replaces a duration that is not
    /// positive.
    fn params(&self, particle: &ParticleNode, fallback_duration: Option<f32>) -> ParticleEffectParams {
        let mut params = ParticleEffectParams {
            offset_x: particle.offset_x,
            offset_y: particle.offset_y,
            camera_offset: particle.camera_offset,
            scale: particle.scale,
            duration_s: particle.duration,
            looping: particle.looping,
        };
        if let Some(duration) = fallback_duration.filter(|_| params.duration_s <= 0.0) {
            params.duration_s = duration;
        }
        match self.settings.loop_mode {
            LoopMode::Definition => {}
            LoopMode::Loop => params.looping = true,
            LoopMode::Once => params.looping = false,
        }
        params
    }
}

/// A running part: a Chaos effect, or a NIF effect kept concrete for its camera.
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

    fn is_shown(&self) -> bool {
        !(self.system().is_finished() && self.system().is_drained())
    }
}

/// The running preview: every playing part of a scene, advanced in fixed steps, seen by the client camera.
pub struct PreviewSimulation {
    instances: Vec<PartInstance>,
    hue: u32,
    time_ms: u32,
}

impl PreviewSimulation {
    /// Starts every playing part at time 0.
    pub fn new(scene: &PreviewScene) -> Self {
        let camera = default_camera();
        let instances = scene
            .prepared
            .iter()
            .map(|part| match &part.system {
                PreparedSystem::Chaos { system, meshes } => {
                    let mut system = ChaosEffectInstance::with_seed(Arc::clone(system), part.seed);
                    system.set_mesh_source(meshes);
                    let mut instance = EffectInstance::new(part.params, Box::new(system));
                    instance.start();
                    PartInstance::Chaos(instance)
                }
                PreparedSystem::Nif(effect) => {
                    let mut system = NifEffectInstance::with_seed(Arc::clone(effect), part.seed);
                    system.set_camera(camera);
                    if part.hued {
                        system.set_time_scale(HUED_EFFECT_TIME_SCALE);
                    }
                    system.start(part.params.duration_s, part.params.looping);
                    PartInstance::Nif { params: part.params, system, hued: part.hued, position: Vec3::ZERO }
                }
            })
            .collect();
        Self { instances, hue: scene.settings.hue, time_ms: 0 }
    }

    pub fn time_ms(&self) -> u32 {
        self.time_ms
    }

    /// One step.
    pub fn step(&mut self) {
        self.time_ms += PREVIEW_STEP_MS;
        let camera = default_camera();
        let placement = EffectPlacement::seen_by(Vec3::ZERO, &camera);
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

    /// Every part stopped emitting and has no particles left.
    pub fn is_drained(&self) -> bool {
        self.instances.iter().all(|instance| !instance.is_shown())
    }

    /// Where the hue quad of instance `part` is drawn, if it plays hued.
    pub fn hued_position(&self, part: usize) -> Option<Vec3> {
        match self.instances.get(part) {
            Some(PartInstance::Nif { hued: true, position, .. }) => Some(*position),
            _ => None,
        }
    }

    pub fn has_hued_parts(&self) -> bool {
        (0..self.instances.len()).any(|part| self.hued_position(part).is_some())
    }

    /// The frame of each part still shown, with the instance index.
    pub fn render_parts(&self) -> Vec<(usize, RenderFrame)> {
        self.instances
            .iter()
            .enumerate()
            .filter(|(_, instance)| instance.is_shown())
            .map(|(part, instance)| {
                let mut frame = RenderFrame::default();
                instance.system().render(&mut frame);
                (part, frame)
            })
            .collect()
    }

    /// Every part in one frame (bounds and checks; draw [`Self::render_parts`]).
    pub fn render(&self) -> RenderFrame {
        let mut frame = RenderFrame::default();
        for instance in self.instances.iter().filter(|instance| instance.is_shown()) {
            instance.system().render(&mut frame);
        }
        frame
    }
}

/// A world-space box: (min, max).
pub type WorldBounds = [Vec3; 2];

/// The world box around everything a scene draws over the timeline (sampled), the anchor and the hue quads.
pub fn fit_bounds(scene: &PreviewScene) -> WorldBounds {
    let mut simulation = PreviewSimulation::new(scene);
    let camera = default_camera();
    let quad_diagonals = [camera.right + camera.up, camera.right - camera.up].map(|axis| axis * HUED_QUAD_HALF_SIZE);
    let mut bounds: WorldBounds = [Vec3::ZERO; 2];
    let mut include = |point: Vec3| {
        let [low, high] = &mut bounds;
        *low = Vec3::new(low.x.min(point.x), low.y.min(point.y), low.z.min(point.z));
        *high = Vec3::new(high.x.max(point.x), high.y.max(point.y), high.z.max(point.z));
    };
    let end = scene.settings.timeline_ms();
    let mut steps = 0;
    loop {
        if steps % FIT_SAMPLE_EVERY_STEPS == 0 {
            if let Some((min, max)) = frame_bounds(&simulation.render()) {
                include(min);
                include(max);
            }
            for part in 0..simulation.instances.len() {
                if let Some(position) = simulation.hued_position(part).filter(|_| simulation.instances[part].is_shown()) {
                    for diagonal in quad_diagonals {
                        include(position - diagonal);
                        include(position + diagonal);
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
    bounds
}

/// The client view of a `side` x `side` canvas showing `bounds`, zoomed.
pub fn preview_view(bounds: WorldBounds, zoom: f32, side: usize) -> OrthoView {
    let camera = default_camera();
    let [min, max] = bounds;
    let (mut extent_x, mut extent_y) = ([f32::MAX, f32::MIN], [f32::MAX, f32::MIN]);
    for corner in 0..8u32 {
        let pick = |bit: u32, low: f32, high: f32| if corner & (1 << bit) == 0 { low } else { high };
        let point = Vec3::new(pick(0, min.x, max.x), pick(1, min.y, max.y), pick(2, min.z, max.z));
        let (x, y) = (point.dot(camera.right), point.dot(camera.up));
        extent_x = [extent_x[0].min(x), extent_x[1].max(x)];
        extent_y = [extent_y[0].min(y), extent_y[1].max(y)];
    }
    let size = (extent_x[1] - extent_x[0]).max(extent_y[1] - extent_y[0]).max(MIN_FIT_EXTENT);
    let pixels_per_unit = side as f32 * (1.0 - 2.0 * FIT_MARGIN) / size * zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    OrthoView::client_view((min + max) * 0.5, CAMERA_DISTANCE, pixels_per_unit, side, side)
}

/// Draws the current frame of a simulation: every part in one sorted draw group, hued parts through `hue_atlas`
/// (drawn plain without it).
pub fn render_canvas(
    scene: &PreviewScene,
    simulation: &PreviewSimulation,
    view: &OrthoView,
    settings: &ViewSettings,
    hue_atlas: Option<&RasterTexture>,
) -> Canvas {
    let mut canvas = Canvas::new(view.width, view.height, settings.background);
    let options = SceneDrawOptions {
        sprites: settings.show_particles,
        meshes: settings.show_meshes,
        mesh_markers: Some(MESH_MARKER_COLOR),
        depth_clip: None,
        // The client's lighting (none), depth buffer and texture filtering.
        ..SceneDrawOptions::default()
    };
    let parts = simulation.render_parts();
    let textures = |name: &str| scene.texture(name);
    let hued = HuedRenderSettings::default();
    let hued_draws: Vec<Option<(Vec3, Box<dyn Fn(&mut Canvas) + '_>)>> = parts
        .iter()
        .map(|(part, frame)| {
            let position = simulation.hued_position(*part)?;
            let atlas = hue_atlas?;
            let (options, hued, textures) = (&options, &hued, &textures);
            let draw = move |canvas: &mut Canvas| {
                let target = render_hued_target(frame, textures, options, &view.camera, position, hued.aspect);
                draw_hued_quad(canvas, view, &target, position, simulation.hue, atlas, hued.sampling);
            };
            Some((position, Box::new(draw) as Box<dyn Fn(&mut Canvas)>))
        })
        .collect();
    let items: Vec<GroupItem<'_, '_>> = parts
        .iter()
        .zip(&hued_draws)
        .map(|((_, frame), hued_draw)| match hued_draw {
            Some((position, draw)) => GroupItem::Custom { sort: hue_quad_sort_key(*position), draw: draw.as_ref() },
            None => GroupItem::Frame { frame, textures: &textures },
        })
        .collect();
    canvas.draw_group(view, &items, &options);
    if settings.show_emitter {
        let (x, y) = view.to_screen(Vec3::ZERO);
        canvas.fill_square(x, y, EMITTER_MARKER_HALF_PX, EMITTER_COLOR, OVERLAY_BLEND);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::effects::test_package::{self, CASE_HUE, TestPackage, ids, systems, textures};

    /// Time the tests simulate to.
    const SCRUB_TIME_MS: u32 = 1000;

    fn build(package: &EffectsEcPackage, target: PreviewTarget, settings: &SimulationSettings) -> PreviewScene {
        PreviewScene::build(package, &mut ResourceCache::default(), target, settings)
    }

    fn collection(id: u32) -> PreviewTarget {
        PreviewTarget::Root(RootKey::Collection(id))
    }

    #[test]
    fn parts_play_or_say_why_not() {
        let (package, _) = test_package::open(TestPackage::standard());
        let settings = SimulationSettings::default();
        let scene = build(&package, collection(ids::PLAIN), &settings);
        let statuses: Vec<&PartStatus> = scene.parts.iter().map(|part| &part.status).collect();
        assert!(matches!(statuses[0], PartStatus::Chaos { name, sequences: 1, .. } if name == "spark.ems"), "{statuses:?}");
        assert_eq!(statuses[1], &PartStatus::NotInClient { name: "lost.nif".to_owned() });
        assert!(statuses[1].describe().contains("not in client"));
        assert_eq!(statuses[2], &PartStatus::NotVisual { kind: "Sound" });
        assert_eq!(statuses[3], &PartStatus::NoSystem);
        assert_eq!(scene.parts[0].textures, [ResourceUse { name: "spark.tga".to_owned(), index: Some(textures::SPARK), state: ResourceState::Loaded }]);
        assert!(scene.texture("Data\\Effects\\SPARK.TGA").is_some_and(|texture| texture.level_count() == 2));

        let mut simulation = PreviewSimulation::new(&scene);
        simulation.advance_to(SCRUB_TIME_MS);
        assert!(simulation.particle_count() > 0);
        let view_settings = ViewSettings { show_emitter: false, ..ViewSettings::default() };
        let view = preview_view(fit_bounds(&scene), view_settings.zoom, view_settings.resolution_px);
        let canvas = render_canvas(&scene, &simulation, &view, &view_settings, None);
        assert!(canvas.pixels.iter().any(|&pixel| pixel != view_settings.background), "the sparks are drawn");
    }

    #[test]
    fn the_hue_picks_the_property_flag_case_and_hues_nif_parts() {
        let (package, _) = test_package::open(TestPackage::standard());
        let played = |hue: u32| {
            let scene = build(&package, collection(ids::HUE_SWITCH), &SimulationSettings { hue, ..Default::default() });
            let part = scene.parts[0].clone();
            (part.path, part.status)
        };
        let (path, status) = played(NO_HUE);
        assert_eq!(path, "0/case 0/0");
        assert!(matches!(status, PartStatus::Chaos { hue: HueMode::NoHue, .. }));
        let (path, status) = played(CASE_HUE);
        assert_eq!(path, format!("0/case {CASE_HUE}/0"));
        assert!(matches!(status, PartStatus::Nif { hue: HueMode::Hued, .. }), "{status:?}");
        let (path, status) = played(CASE_HUE + 1);
        assert_eq!((path.as_str(), status), ("0/default/0", PartStatus::NotVisual { kind: "Sound" }));

        let settings = SimulationSettings { hue: CASE_HUE, ..Default::default() };
        let scene = build(&package, collection(ids::HUE_SWITCH), &settings);
        let simulation = PreviewSimulation::new(&scene);
        assert!(simulation.has_hued_parts());
        assert_eq!(scene.parts[0].textures[0].index, Some(textures::SMOKE_EMBEDDED), "the embedded texture by its package name");
    }

    #[test]
    fn choices_follow_the_settings() {
        let (package, _) = test_package::open(TestPackage::standard());
        let systems_of = |settings: &SimulationSettings| -> Vec<Option<u32>> {
            build(&package, collection(ids::CHOICES), settings).parts.iter().map(|part| part.system).collect()
        };
        // Random -> spark; Lod 3 -> spark; RoundRobin -> its first child, smoke.
        let high = systems_of(&SimulationSettings::default());
        assert_eq!(high, [Some(systems::SPARK), Some(systems::SPARK), Some(systems::SMOKE)]);
        let low = systems_of(&SimulationSettings { lod_level: 1, ..Default::default() });
        assert_eq!(low[1], Some(systems::LOST));
        let none = systems_of(&SimulationSettings { lod_level: 0, ..Default::default() });
        assert_eq!(none.len(), 2, "no LOD level fits");

        let cyclic = build(&package, collection(ids::CYCLE), &SimulationSettings::default());
        assert!(cyclic.parts.is_empty());
        assert!(cyclic.notes.iter().any(|note| note.contains("cyclic")), "{:?}", cyclic.notes);
    }

    #[test]
    fn tileart_skips_projectile_particles_and_meshes_come_from_the_package() {
        let (package, _) = test_package::open(TestPackage::standard());
        let tile = PreviewTarget::Root(RootKey::Tile(ids::TILE));
        let scene = build(&package, tile, &SimulationSettings::default());
        assert_eq!(scene.parts[0].status, PartStatus::ProjectileOnly { name: "mesh.ems".to_owned() });
        assert!(scene.parts[1].status.plays());

        let scene = build(&package, tile, &SimulationSettings { play_projectile_only: true, ..Default::default() });
        let part = &scene.parts[0];
        assert!(matches!(&part.status, PartStatus::Chaos { sequences: 2, .. }), "{:?}", part.status);
        assert_eq!(part.meshes, [ResourceUse { name: "smoke.nif".to_owned(), index: Some(systems::SMOKE), state: ResourceState::Loaded }]);
        let gone = part.textures.iter().find(|texture| texture.name == "gone.tga").unwrap();
        assert!(matches!(gone.state, ResourceState::NoPayload { .. }), "a client data gap, drawn white");

        let frame = PreviewTarget::Root(RootKey::Frame { sequence_id: ids::SEQUENCE, action_id: ids::ACTION, frame: ids::FRAME });
        assert!(build(&package, frame, &SimulationSettings::default()).parts[0].status.plays());

        let single = build(&package, PreviewTarget::System(systems::MESH), &SimulationSettings::default());
        assert!(single.has_playing_parts());
        let missing = build(&package, PreviewTarget::System(systems::LOST), &SimulationSettings::default());
        assert!(matches!(missing.parts[0].status, PartStatus::NotInClient { .. }));
        let out_of_range = build(&package, PreviewTarget::System(u32::MAX), &SimulationSettings::default());
        assert!(out_of_range.parts.is_empty() && !out_of_range.notes.is_empty());
    }

    #[test]
    fn a_corrupt_blob_is_reported_not_played() {
        let (package, _) = test_package::open(TestPackage { corrupt_spark: true, ..TestPackage::standard() });
        let scene = build(&package, collection(ids::PLAIN), &SimulationSettings::default());
        assert!(matches!(&scene.parts[0].status, PartStatus::Unreadable { name, .. } if name == "spark.ems"));
        assert!(!scene.has_playing_parts());
        let mut simulation = PreviewSimulation::new(&scene);
        simulation.advance_to(SCRUB_TIME_MS);
        assert_eq!(simulation.particle_count(), 0);
    }
}
