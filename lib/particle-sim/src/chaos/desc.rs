//! The compiled, engine-neutral description of a Chaos particle system.
//!
//! [`ChaosSystemDesc`] is what the client's `Emitter::CompileScript` (0x008a1160) keeps from a `.ems` script,
//! as plain data: fixed-size fields and vectors, no references, no serde. A converter stores it in a package
//! and a renderer loads it without reading the script. Everything the client decides at parse time is already
//! applied:
//! - multipliers: `scaleMultiplier` / `velocityMultiplier` scaled the SCALE / VELOCITY events that follow them;
//! - ROTATION values are in radians;
//! - a `final` event has the time range (L, L), L the sequence lifetime max at that line;
//! - the last `texture` / `meshName` / `emitterName` is the resource, reduced to its file name;
//! - keywords were matched the client's way (substrings, first match wins).
//!
//! What the client draws at random when it compiles stays a range here: the event times (`at random(a, b)`),
//! sampled once per compiled script by [`super::ChaosSystem::prepare`].
//!
//! Layout changes bump [`CHAOS_DESC_VERSION`].

use crate::render::BlendFactor;

/// Version of the [`ChaosSystemDesc`] layout.
pub const CHAOS_DESC_VERSION: u32 = 1;

/// Default `numParticles` (ParticleEventSequence ctor 0x008b7330).
pub const DEFAULT_NUM_PARTICLES: u16 = 100;
/// Default `burst`: one particle per emission step.
pub const DEFAULT_BURST: IntRange = IntRange { min: 1, max: 1 };
/// Default `loops` (never read at run time).
pub const DEFAULT_LOOPS: i32 = -1;
/// Default `scaleMultiplier` and `velocityMultiplier`.
pub const DEFAULT_MULTIPLIER: f32 = 1.0;

/// `min..max` as written; see [`super::sample_range`] for how the client draws from it (`min > max` included).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Range {
    pub min: f32,
    pub max: f32,
}

impl Range {
    pub const ZERO: Self = Self { min: 0.0, max: 0.0 };

    pub const fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    pub const fn constant(value: f32) -> Self {
        Self { min: value, max: value }
    }
}

/// A vector range: each component sampled on its own (x, y, z in that order).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VecRange {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl VecRange {
    pub const ZERO: Self = Self { min: [0.0; 3], max: [0.0; 3] };
}

/// An integer range (`burst`): `max` is exclusive when it differs from `min` (see [`super::sample_int_range`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IntRange {
    pub min: u32,
    pub max: u32,
}

/// Sequence flags, with the client's bit numbers (`ParticleEventSequence` +0x8).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ChaosSequenceFlags(pub u32);

impl ChaosSequenceFlags {
    /// `centered` (no effect found).
    pub const CENTERED: u32 = 1 << 1;
    /// `sort` (no effect found).
    pub const SORT: u32 = 1 << 2;
    /// A ROTATION event exists: rotation is integrated and billboards spin.
    pub const ROTATION: u32 = 1 << 3;
    /// `stayAtEmitter`: particles sit on the node and die when the emitter stops.
    pub const STAY_AT_EMITTER: u32 = 1 << 4;
    /// A THRUST event exists: thrust is integrated.
    pub const THRUST: u32 = 1 << 5;
    /// `scaleMultiplier` was set.
    pub const SCALE_MULTIPLIER: u32 = 1 << 6;
    /// `velocityMultiplier` was set.
    pub const VELOCITY_MULTIPLIER: u32 = 1 << 7;
    /// `local`: particles are stored relative to the node and drawn at its current position.
    pub const LOCAL: u32 = 1 << 8;
    /// `celAnimate`: flipbook texture.
    pub const CEL_ANIMATE: u32 = 1 << 9;
    /// The flipbook loops.
    pub const CEL_LOOP: u32 = 1 << 10;
    /// `emissionBoost`: extra emission per unit of node movement.
    pub const EMISSION_BOOST: u32 = 1 << 11;
    /// `collisionCheck` (hangs the client; the compiler rejects it).
    pub const COLLISION_CHECK: u32 = 1 << 12;
    pub const COLLISION_STOP: u32 = 1 << 13;
    pub const COLLISION_REFLECT: u32 = 1 << 14;

    pub fn contains(self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    pub fn insert(&mut self, flag: u32) {
        self.0 |= flag;
    }
}

/// Particle type (`type`), with the client's values (`ParseParticleType` 0x008b1320).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChaosParticleType {
    /// No `type` line: nothing is rendered.
    None = 0,
    /// Camera-facing quads.
    Point = 1,
    /// A `.nif` clone per particle.
    Mesh = 2,
    /// No renderer in the client.
    Billboard = 3,
    /// A sub-effect per particle (no data uses it; not simulated here).
    Emitter = 4,
    /// Quads stretched along the motion.
    Axial = 5,
    /// No renderer in the client.
    Light = 6,
    /// No renderer in the client.
    Test = 8,
}

impl ChaosParticleType {
    /// Whether the client draws something for this type.
    pub fn is_rendered(self) -> bool {
        matches!(self, Self::Point | Self::Mesh | Self::Emitter | Self::Axial)
    }
}

/// `CelSetup(frames, uStep, vStep, columns, rows, fps, loop)` (`ParseCelSetup` 0x008b0b20).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChaosCelSetup {
    pub frames: u16,
    /// Cell width in UV units.
    pub u_step: f32,
    /// Cell height in UV units.
    pub v_step: f32,
    pub columns: u8,
    /// Stored but never read by the client.
    pub rows: u8,
    pub fps: f32,
    pub looping: bool,
}

impl Default for ChaosCelSetup {
    fn default() -> Self {
        Self { frames: 1, u_step: 0.0, v_step: 0.0, columns: 1, rows: 1, fps: 0.0, looping: false }
    }
}

/// Event kinds, with the client's type ids (`ParticleEvent` +0x18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChaosEventKind {
    Scale = 0,
    Color = 1,
    RedColor = 2,
    GreenColor = 3,
    BlueColor = 4,
    /// Reachable only through a keyword that also contains COLOR (`alphaColor`).
    Alpha = 5,
    Velocity = 6,
    VelocityX = 7,
    VelocityY = 8,
    VelocityZ = 9,
    Thrust = 10,
    EventTimer = 11,
    Rotation = 12,
    Length = 13,
}

impl ChaosEventKind {
    /// How many value components the event has (1, 3 or 4).
    pub fn components(self) -> usize {
        match self {
            Self::Color => 4,
            Self::Velocity | Self::Rotation => 3,
            _ => 1,
        }
    }

    /// Whether a fade with no later fade keeps its step (Scale, colours, Velocity) rather than stopping
    /// (Thrust, Rotation, Length).
    pub fn keeps_step(self) -> bool {
        !matches!(self, Self::Thrust | Self::Rotation | Self::Length | Self::EventTimer)
    }

    /// Whether a StayAtEmitter sequence rejects the event.
    pub fn moves_particle(self) -> bool {
        matches!(self, Self::Velocity | Self::VelocityX | Self::VelocityY | Self::VelocityZ | Self::Thrust)
    }
}

/// A value range of an event. Components: scalar events use index 0; Velocity and Rotation x, y, z; Color r, g,
/// b, a. Unused components are 0.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ChaosEventValue {
    pub min: [f32; 4],
    pub max: [f32; 4],
}

/// One `[fade so] initial | at T | final <property> = value` line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChaosEventDesc {
    pub kind: ChaosEventKind,
    /// `fade so`: the event does not set its value; it ends the previous ramp of its kind.
    pub fade: bool,
    /// Seconds of particle (event-timer) time; `initial` is (0, 0).
    pub time: Range,
    pub value: ChaosEventValue,
}

/// One `eventsequence` block: a particle emitter of the system.
#[derive(Debug, Clone, PartialEq)]
pub struct ChaosSequenceDesc {
    pub name: String,
    pub particle_type: ChaosParticleType,
    /// Texture (POINT, AXIAL), mesh (MESH) or sub-script (EMITTER) file name; empty when none.
    pub resource: String,
    pub src_blend: BlendFactor,
    pub dst_blend: BlendFactor,
    pub flags: ChaosSequenceFlags,
    /// Particle lifetime in seconds, sampled per particle.
    pub lifetime: Range,
    /// Particles per emission step per second; only `max` is used.
    pub emit_rate: Range,
    /// Cap on the live particles of the sequence.
    pub num_particles: u16,
    /// Never read at run time.
    pub loops: i32,
    /// Already applied to the SCALE events that followed it.
    pub scale_multiplier: f32,
    /// Already applied to the VELOCITY events that followed it.
    pub velocity_multiplier: f32,
    /// Acceleration, sampled every update and projected on the world up axis.
    pub gravity: VecRange,
    /// Per-component factor on the node velocity at spawn.
    pub inherited: VecRange,
    /// Added to the initial velocity at spawn.
    pub chaos_velocity: VecRange,
    /// Spawn offset from the node (rotated and scaled by it).
    pub chaos_location: VecRange,
    /// Particles per emission step.
    pub burst: IntRange,
    /// Extra emit rate per unit of node movement (with [`ChaosSequenceFlags::EMISSION_BOOST`]).
    pub emission_boost: Range,
    /// Flipbook (with [`ChaosSequenceFlags::CEL_ANIMATE`]).
    pub cel: ChaosCelSetup,
    /// In script order (not yet sorted by time).
    pub events: Vec<ChaosEventDesc>,
}

impl ChaosSequenceDesc {
    /// A sequence with the client's defaults.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            particle_type: ChaosParticleType::None,
            resource: String::new(),
            src_blend: BlendFactor::One,
            dst_blend: BlendFactor::One,
            flags: ChaosSequenceFlags::default(),
            lifetime: Range::ZERO,
            emit_rate: Range::ZERO,
            num_particles: DEFAULT_NUM_PARTICLES,
            loops: DEFAULT_LOOPS,
            scale_multiplier: DEFAULT_MULTIPLIER,
            velocity_multiplier: DEFAULT_MULTIPLIER,
            gravity: VecRange::ZERO,
            inherited: VecRange::ZERO,
            chaos_velocity: VecRange::ZERO,
            chaos_location: VecRange::ZERO,
            burst: DEFAULT_BURST,
            emission_boost: Range::ZERO,
            cel: ChaosCelSetup::default(),
            events: Vec::new(),
        }
    }
}

/// A compiled `particlesystem` script.
#[derive(Debug, Clone, PartialEq)]
pub struct ChaosSystemDesc {
    /// [`CHAOS_DESC_VERSION`] of the producer.
    pub version: u32,
    /// The system name (the client caches compiled scripts by uppercase file path, not by this name).
    pub name: String,
    /// The number after the name (ignored by the client).
    pub script_version: f32,
    /// `minEmitTime`: emission time of non-looping effects, sampled at start; (0, 0) when absent.
    pub min_emit_time: Range,
    pub sequences: Vec<ChaosSequenceDesc>,
}

impl ChaosSystemDesc {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: CHAOS_DESC_VERSION,
            name: name.into(),
            script_version: 0.0,
            min_emit_time: Range::ZERO,
            sequences: Vec::new(),
        }
    }

    /// The texture and mesh files the system draws, in sequence order, without repeats.
    pub fn resources(&self) -> Vec<&str> {
        let mut resources: Vec<&str> = Vec::new();
        for sequence in &self.sequences {
            if !sequence.resource.is_empty() && !resources.contains(&sequence.resource.as_str()) {
                resources.push(&sequence.resource);
            }
        }
        resources
    }
}
