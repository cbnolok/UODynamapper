//! Preview of effect definitions with particle-sim: which particles of a definition list play (resolving
//! random, LOD, round-robin, wait, property flag and user preference definitions the way one client play would),
//! a fixed-step simulation that can be scrubbed (re-simulated from 0 with the same seed, so a time always shows
//! the same frame), an orthographic camera fitted to the whole timeline, software rendering with particle-sim's
//! rasterizer, and PNG exports. No egui here.
//!
//! Chaos (`.ems`) particles play now. NIF particles are listed with [`PartStatus::Nif`]; a NIF particle system
//! plugs in as another [`PreparedSystem`] variant producing a `ParticleSystemInstance`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use image::RgbaImage;
use particle_sim::chaos::compile::compile_chaos_source;
use particle_sim::raster::{camera_axes, frame_extent, Canvas, OrthoView, RasterTexture, OVERLAY_BLEND};
use particle_sim::render::RenderFrame;
use particle_sim::{ChaosEffectInstance, ChaosSystem, EffectInstance, EffectPlacement, Mt19937, ParticleEffectParams, Rng, Vec3};
use uocf::enhanced::effect_definition::{EffectDefinition, ParticleEffect, PropertyFlagMode, RANDOM_WEIGHT_TOTAL};
use uocf::enhanced::string_dictionary::UoStringDictionary;

use super::effect_files::{decode_texture, is_nif_name};
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
/// Camera elevation above the horizon, looking north.
pub const CAMERA_ELEVATION_DEGREES: f32 = 30.0;
/// Distance of the camera from the view centre (for the axial view vector).
const CAMERA_DISTANCE: f32 = 1000.0;
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
    /// World units per second along +x; 0 keeps the emitter still.
    pub move_speed: f32,
    /// UI particle LOD setting (1..3) for LOD definitions.
    pub lod_level: u32,
    pub timeline_s: f32,
}

impl Default for SimulationSettings {
    fn default() -> Self {
        Self {
            seed: DEFAULT_PREVIEW_SEED,
            loop_mode: LoopMode::Definition,
            duration_override: None,
            move_speed: 0.0,
            lod_level: DEFAULT_LOD_LEVEL,
            timeline_s: DEFAULT_TIMELINE_S,
        }
    }
}

impl SimulationSettings {
    pub fn timeline_ms(&self) -> u32 {
        (self.timeline_s.clamp(MIN_TIMELINE_S, MAX_TIMELINE_S) * MS_PER_SECOND) as u32
    }
}

/// How the frame is drawn: no effect on the simulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSettings {
    pub background: [f32; 4],
    pub show_bounds: bool,
    pub show_emitter: bool,
    /// Multiplies the fitted scale.
    pub zoom: f32,
    /// Canvas pixels.
    pub pan: [f32; 2],
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self { background: DEFAULT_BACKGROUND, show_bounds: false, show_emitter: true, zoom: 1.0, pan: [0.0; 2] }
    }
}

/// Why a part does or does not play.
#[derive(Debug, Clone, PartialEq)]
pub enum PartStatus {
    /// A Chaos particle that plays.
    Chaos { name: String, sequences: usize },
    /// A NIF particle: not previewable yet.
    Nif { name: String },
    /// The particle name is not in the string dictionary (or there is none).
    NameUnresolved { name_index: u32 },
    /// The file is not in `EffectTexture.uop`.
    MissingFile { name: String },
    /// The client rejects the script: it plays nothing.
    ScriptRejected { name: String, errors: Vec<String> },
    /// A sound, earthquake or position offset: nothing to draw.
    NotVisual,
}

impl PartStatus {
    pub fn plays(&self) -> bool {
        matches!(self, Self::Chaos { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Chaos { name, sequences } => format!("{name}: Chaos, {sequences} sequence(s)"),
            Self::Nif { name } => format!("{name}: NIF preview not available yet"),
            Self::NameUnresolved { name_index } => format!("name index {name_index} is not in the string dictionary"),
            Self::MissingFile { name } => format!("{name}: not in EffectTexture.uop"),
            Self::ScriptRejected { name, errors } => format!("{name}: rejected by the client ({})", errors.join("; ")),
            Self::NotVisual => "not drawn".to_string(),
        }
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
}

struct PreparedPart {
    params: ParticleEffectParams,
    seed: u32,
    system: PreparedSystem,
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
    textures: HashMap<String, RasterTexture>,
}

/// File access of the scene builder.
pub struct PreviewInput<'a> {
    pub strings: Option<&'a UoStringDictionary>,
    /// The payload of an `EffectTexture.uop` file by name, with pending changes applied.
    pub payload: &'a dyn Fn(&str) -> Option<Arc<[u8]>>,
}

impl PreviewScene {
    /// Resolves a definition list the way one play does and prepares its particles.
    pub fn build(definitions: &[EffectDefinition], settings: &SimulationSettings, input: &PreviewInput<'_>) -> Self {
        let mut builder = SceneBuilder::new(settings, input);
        builder.list(definitions, &[], ChildList::Top);
        builder.scene
    }

    /// One Chaos script (an `EffectTexture.uop` file) played as a particle with default fields.
    pub fn build_script(name: &str, settings: &SimulationSettings, input: &PreviewInput<'_>) -> Self {
        let EffectDefinition::Particle(particle) = DefinitionKind::Particle.new_definition() else {
            unreachable!("a particle kind makes a particle")
        };
        let mut builder = SceneBuilder::new(settings, input);
        let status = builder.particle_named(&particle, name.to_owned());
        builder.scene.parts.push(PreviewPart { path: vec![PathStep::top(0)], kind: DefinitionKind::Particle, status });
        builder.scene
    }

    /// Whether anything plays.
    pub fn has_playing_parts(&self) -> bool {
        !self.prepared.is_empty()
    }

    fn texture(&self, name: &str) -> Option<&RasterTexture> {
        self.textures.get(&name.to_ascii_lowercase())
    }
}

struct SceneBuilder<'s, 'a> {
    settings: &'s SimulationSettings,
    input: &'s PreviewInput<'a>,
    /// Draws of random definitions, seeded by the preview seed.
    draw: Mt19937,
    scene: PreviewScene,
}

impl<'s, 'a> SceneBuilder<'s, 'a> {
    fn new(settings: &'s SimulationSettings, input: &'s PreviewInput<'a>) -> Self {
        Self {
            settings,
            input,
            draw: Mt19937::new(settings.seed),
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
            EffectDefinition::PropertyFlag(effect) => {
                if effect.mode == PropertyFlagMode::Property {
                    self.scene.notes.push(format!("{label} property flag: default children (no object here)"));
                    self.list(&effect.default_children, &path, ChildList::Default);
                } else {
                    self.scene.notes.push(format!("{label} property flag (flag mask): no object flags, plays nothing"));
                }
            }
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
            return if is_nif_name(&name) || (self.input.payload)(&name).is_some() {
                PartStatus::Nif { name }
            } else {
                PartStatus::MissingFile { name }
            };
        }
        let Some(payload) = (self.input.payload)(&name) else {
            return PartStatus::MissingFile { name };
        };
        let report = compile_chaos_source(&payload);
        let Some(desc) = report.desc else {
            let errors = report.errors().map(ToString::to_string).collect();
            return PartStatus::ScriptRejected { name, errors };
        };
        for resource in desc.resources() {
            self.load_texture(resource);
        }
        let sequences = desc.sequences.len();
        let seed = self.settings.seed.wrapping_add(self.scene.prepared.len() as u32);
        let system = Arc::new(ChaosSystem::prepare(desc, &mut Mt19937::new(seed)));
        let mut params = ParticleEffectParams::from(particle);
        if let Some(duration) = self.settings.duration_override {
            params.duration_s = duration;
        }
        match self.settings.loop_mode {
            LoopMode::Definition => {}
            LoopMode::Loop => params.looping = true,
            LoopMode::Once => params.looping = false,
        }
        self.scene.prepared.push(PreparedPart { params, seed, system: PreparedSystem::Chaos(system) });
        PartStatus::Chaos { name, sequences }
    }

    fn load_texture(&mut self, name: &str) {
        let key = name.to_ascii_lowercase();
        if is_nif_name(&key) || self.scene.textures.contains_key(&key) {
            return;
        }
        match (self.input.payload)(name).map(|payload| decode_texture(&payload)) {
            Some(Ok(image)) => {
                let (width, height) = image.dimensions();
                if let Some(texture) = RasterTexture::from_rgba8(width as usize, height as usize, image.as_raw()) {
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

/// The running preview: every playing part of a scene, advanced in fixed steps.
pub struct PreviewSimulation {
    instances: Vec<EffectInstance>,
    settings: SimulationSettings,
    time_ms: u32,
    camera_vector: Vec3,
}

impl PreviewSimulation {
    /// Starts every playing part at time 0.
    pub fn new(scene: &PreviewScene, settings: &SimulationSettings) -> Self {
        let instances = scene
            .prepared
            .iter()
            .map(|part| {
                let system = match &part.system {
                    PreparedSystem::Chaos(system) => ChaosEffectInstance::with_seed(Arc::clone(system), part.seed),
                };
                let mut instance = EffectInstance::new(part.params, Box::new(system));
                instance.start();
                instance
            })
            .collect();
        let (direction, _, _) = camera_axes(CAMERA_ELEVATION_DEGREES.to_radians());
        Self { instances, settings: *settings, time_ms: 0, camera_vector: direction }
    }

    /// A simulation advanced to `time_ms` (rounded down to whole steps): the same frame every time.
    pub fn at(scene: &PreviewScene, settings: &SimulationSettings, time_ms: u32) -> Self {
        let mut simulation = Self::new(scene, settings);
        simulation.advance_to(time_ms);
        simulation
    }

    pub fn time_ms(&self) -> u32 {
        self.time_ms
    }

    /// Where the emitter is now.
    pub fn emitter_position(&self) -> Vec3 {
        emitter_at(self.settings.move_speed, self.time_ms)
    }

    /// One step.
    pub fn step(&mut self) {
        self.time_ms += PREVIEW_STEP_MS;
        let placement = EffectPlacement { camera_vector: self.camera_vector, ..EffectPlacement::at(self.emitter_position()) };
        for instance in &mut self.instances {
            instance.update(PREVIEW_STEP_MS, &placement);
        }
    }

    /// Steps while a whole step fits before `time_ms`.
    pub fn advance_to(&mut self, time_ms: u32) {
        while self.time_ms + PREVIEW_STEP_MS <= time_ms {
            self.step();
        }
    }

    pub fn particle_count(&self) -> usize {
        self.instances.iter().map(EffectInstance::particle_count).sum()
    }

    /// Every part stopped emitting and has no particles left.
    pub fn is_drained(&self) -> bool {
        self.instances.iter().all(|instance| instance.is_finished() && instance.is_drained())
    }

    pub fn render(&self) -> RenderFrame {
        let mut frame = RenderFrame::default();
        for instance in &self.instances {
            instance.render(&mut frame);
        }
        frame
    }
}

fn emitter_at(move_speed: f32, time_ms: u32) -> Vec3 {
    Vec3::new(move_speed * time_ms as f32 / MS_PER_SECOND, 0.0, 0.0)
}

/// The view plane extent of everything a scene draws over the timeline (sampled), with the emitter path:
/// (min right, max right, min up, max up).
pub fn fit_extent(scene: &PreviewScene, settings: &SimulationSettings) -> [f32; 4] {
    let probe = probe_view();
    let mut simulation = PreviewSimulation::new(scene, settings);
    let mut extent: Option<[f32; 4]> = None;
    let mut include = |bounds: [f32; 4]| {
        let current = extent.get_or_insert(bounds);
        *current = [current[0].min(bounds[0]), current[1].max(bounds[1]), current[2].min(bounds[2]), current[3].max(bounds[3])];
    };
    let end = settings.timeline_ms();
    let mut steps = 0;
    loop {
        let (x, y) = probe.project(simulation.emitter_position());
        include([x, x, y, y]);
        if steps % FIT_SAMPLE_EVERY_STEPS == 0 {
            if let Some(bounds) = frame_extent(&simulation.render(), &probe.camera) {
                include(bounds);
            }
        }
        if simulation.time_ms() + PREVIEW_STEP_MS > end {
            break;
        }
        simulation.step();
        steps += 1;
    }
    extent.unwrap_or_default()
}

fn probe_view() -> OrthoView {
    OrthoView::new(CAMERA_ELEVATION_DEGREES.to_radians(), Vec3::ZERO, CAMERA_DISTANCE, 1.0, 1, 1)
}

/// The view of a `width` x `height` canvas showing `extent` (from [`fit_extent`]), zoomed and panned.
pub fn preview_view(extent: [f32; 4], view: &ViewSettings, width: usize, height: usize) -> OrthoView {
    let probe = probe_view();
    let [min_x, max_x, min_y, max_y] = extent;
    let size = (max_x - min_x).max(max_y - min_y).max(MIN_FIT_EXTENT);
    let pixels = width.min(height) as f32;
    let pixels_per_unit = pixels * (1.0 - 2.0 * FIT_MARGIN) / size * view.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    let center = probe.camera.right * ((min_x + max_x) * 0.5 - view.pan[0] / pixels_per_unit)
        + probe.camera.up * ((min_y + max_y) * 0.5 + view.pan[1] / pixels_per_unit);
    OrthoView::new(CAMERA_ELEVATION_DEGREES.to_radians(), center, CAMERA_DISTANCE, pixels_per_unit, width, height)
}

/// Draws the current frame of a simulation with the overlays.
pub fn render_canvas(scene: &PreviewScene, simulation: &PreviewSimulation, view: &OrthoView, settings: &ViewSettings) -> Canvas {
    let mut canvas = Canvas::new(view.width, view.height, settings.background);
    let frame = simulation.render();
    canvas.draw_frame(view, &frame, &|name| scene.texture(name), MESH_COLOR);
    if settings.show_bounds {
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
    times: &[u32],
) -> Vec<RgbaImage> {
    let mut simulation = PreviewSimulation::new(scene, settings);
    times
        .iter()
        .map(|&time| {
            simulation.advance_to(time);
            canvas_image(&render_canvas(scene, &simulation, view, view_settings))
        })
        .collect()
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
            _ => None,
        }
    }

    fn scene(definitions: &[EffectDefinition], settings: &SimulationSettings) -> PreviewScene {
        let strings = dictionary();
        PreviewScene::build(definitions, settings, &PreviewInput { strings: Some(&strings), payload: &files })
    }

    #[test]
    fn the_script_compiles_and_plays() {
        let settings = SimulationSettings::default();
        let scene = scene(&[particle(SCRIPT_INDEX)], &settings);
        assert!(matches!(scene.parts[0].status, PartStatus::Chaos { .. }), "{:?}", scene.parts[0].status);
        assert!(PreviewSimulation::at(&scene, &settings, SCRUB_TIME_MS).particle_count() > 0);
        assert_eq!(scene.texture_problems.len(), 1, "spark.tga is missing: {:?}", scene.texture_problems);
    }

    #[test]
    fn parts_report_nif_missing_and_unresolved_names() {
        let settings = SimulationSettings::default();
        let sound = EffectDefinition::Sound(SoundEffect { sound_id: 1, value_08: 1.0 });
        let scene = scene(&[particle(NIF_INDEX), particle(99), sound], &settings);
        assert!(matches!(&scene.parts[0].status, PartStatus::Nif { name } if name == NIF_NAME));
        assert!(matches!(scene.parts[1].status, PartStatus::NameUnresolved { name_index: 99 }));
        assert_eq!(scene.parts[2].status, PartStatus::NotVisual);
        assert!(!scene.has_playing_parts());
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
        let mut played = PreviewSimulation::new(&scene, &settings);
        for target in (0..=SCRUB_TIME_MS).step_by(37) {
            played.advance_to(target);
        }
        let scrubbed = PreviewSimulation::at(&scene, &settings, SCRUB_TIME_MS);
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
        frame_content(&PreviewSimulation::at(&scene(definitions, settings), settings, SCRUB_TIME_MS).render())
    }

    #[test]
    fn view_fits_and_exports_frames() {
        let settings = SimulationSettings::default();
        let scene = scene(&[particle(SCRIPT_INDEX)], &settings);
        let extent = fit_extent(&scene, &settings);
        assert!(extent[3] > extent[2], "the particles rise: {extent:?}");
        let view_settings = ViewSettings { show_bounds: true, ..Default::default() };
        let view = preview_view(extent, &view_settings, PREVIEW_RESOLUTION_PX, PREVIEW_RESOLUTION_PX);
        let times = export_times(&settings, DEFAULT_EXPORT_FRAMES);
        assert_eq!(times.len(), DEFAULT_EXPORT_FRAMES);
        assert_eq!(*times.last().unwrap(), settings.timeline_ms());
        let frames = render_frames(&scene, &settings, &view, &view_settings, &times);
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
        let input = PreviewInput { strings: Some(&strings), payload: &payload };
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
        let view = preview_view(fit_extent(&scene, &settings), &view_settings, PREVIEW_RESOLUTION_PX, PREVIEW_RESOLUTION_PX);
        let frames = render_frames(&scene, &settings, &view, &view_settings, &export_times(&settings, DEFAULT_EXPORT_FRAMES));
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
