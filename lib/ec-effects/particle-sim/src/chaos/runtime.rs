//! The client's `Emitter` (0x008a13c0 and neighbours): emission, particle update, events and the hand-off to the
//! renderer.

use std::sync::Arc;

use super::desc::{ChaosEventKind, ChaosParticleType, ChaosSequenceDesc, ChaosSequenceFlags};
use super::mesh::{MeshClones, MeshPlacement, MeshSource, euler_xyz};
use super::{
    ChaosSystem, MIN_EMIT_INTERVAL_S, PreparedEvent, SUBSTEP_INTERVAL_S, sample_components, sample_int_range,
    sample_range,
};
use crate::math::{Mat3, Vec3};
use crate::render::{
    BlendState, DrawBatch, MeshInstance, PARTICLE_DEPTH_STATE, RenderFrame, SortKey, SpriteInstance, SpriteKind,
    UvRect,
};
use crate::rng::Rng;
use crate::system::NodeTransform;

/// Most EVENTTIMER rewinds per particle and update. The client has no limit: an EVENTTIMER event that sets the
/// timer to its own time hangs it. Here the particle stops running events for the update instead.
pub const MAX_EVENT_TIMER_REWINDS: usize = 64;
/// The gravity axis `G` (`DAT_00de2810`, written by `FUN_004216b0` from column 2 of the effect rotation at
/// `DAT_00e37224+0x30`). Gravity keeps only its component along it. That rotation is `C · P`, the rotation every
/// effect node takes (`uosa_effects.md`, "Effect camera and lighting"), so `G` is camera up in the world and local
/// +Z in the effect's frame, the frame this runtime simulates in (the client's view is
/// [`crate::CameraFrame::client_view`]). `uosa_chaos.md` ("SequenceUpdate", open questions) left it as a probable
/// world (0,0,1).
pub const GRAVITY_AXIS: Vec3 = Vec3::Z;
/// Size of a new particle before its events (pool template).
const TEMPLATE_SIZE: f32 = 1.0;
/// Lifetime of a new particle before sampling (pool template).
const TEMPLATE_LIFETIME: f32 = 1.0;
/// Colour of a new particle before its events (pool template): opaque white.
const TEMPLATE_COLOR: [f32; 4] = [1.0; 4];
/// Length of a new particle before its events (pool template).
const TEMPLATE_LENGTH: f32 = 1.0;
/// Span used by a fade whose target has the same time (division by zero guard, `Apply`).
const ZERO_SPAN_REPLACEMENT: f32 = 1.0;
/// Colours handed to the renderer are clamped to this range.
const COLOR_RANGE: (f32, f32) = (0.0, 1.0);
/// Node scale used while the emitter has no owner (after Stop).
const DETACHED_NODE_SCALE: f32 = 1.0;
/// `ParticleSet::Update` (0x008b2de0) recomputes the bounds of the quad sequences when this many seconds have
/// passed since the last time (`DAT_00d05214`).
pub const BOUND_INTERVAL_S: f32 = 0.033;
/// Added to the bound radius (`DAT_00d05120`, same function).
pub const BOUND_RADIUS_MARGIN: f32 = 0.01;

/// One particle (`Particle`, 0xa8 bytes in the client; field offsets in the comments).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChaosParticle {
    /// +0x8
    pub size: f32,
    /// +0xc
    pub size_step: f32,
    /// +0x10, seconds
    pub lifetime: f32,
    /// +0x14, seconds since birth at the last update
    pub age: f32,
    /// +0x18, the clock events are compared with
    pub event_timer: f32,
    /// +0x1c
    pub birth: f32,
    /// +0x20
    pub last_update: f32,
    /// +0x28, RGBA, not clamped
    pub color: [f32; 4],
    /// +0x38
    pub color_step: [f32; 4],
    /// World position, or node-relative for LOCAL sequences.
    pub position: Vec3,
    pub previous_position: Vec3,
    /// +0x48
    pub velocity: [f32; 3],
    /// +0x54
    pub velocity_step: [f32; 3],
    /// +0x60
    pub thrust: f32,
    /// +0x64
    pub thrust_step: f32,
    /// +0x80, radians
    pub rotation: [f32; 3],
    /// +0x8c
    pub rotation_step: [f32; 3],
    /// +0x98
    pub length: f32,
    /// +0x9c
    pub length_step: f32,
    /// Seconds of the last update step (MESH clones sub-step over it).
    pub step: f32,
    /// Index of the next event to run.
    pub next_event: usize,
}

impl ChaosParticle {
    /// The pool template.
    fn template(time: f32) -> Self {
        Self {
            size: TEMPLATE_SIZE,
            size_step: 0.0,
            lifetime: TEMPLATE_LIFETIME,
            age: 0.0,
            event_timer: 0.0,
            birth: time,
            last_update: time,
            color: TEMPLATE_COLOR,
            color_step: [0.0; 4],
            position: Vec3::ZERO,
            previous_position: Vec3::ZERO,
            velocity: [0.0; 3],
            velocity_step: [0.0; 3],
            thrust: 0.0,
            thrust_step: 0.0,
            rotation: [0.0; 3],
            rotation_step: [0.0; 3],
            length: TEMPLATE_LENGTH,
            length_step: 0.0,
            step: 0.0,
            next_event: 0,
        }
    }

    /// The value and step of the field an event writes (`None` for EVENTTIMER, which has no step).
    fn channel(&mut self, kind: ChaosEventKind) -> Option<(&mut [f32], &mut [f32])> {
        const RED: usize = 0;
        const GREEN: usize = 1;
        const BLUE: usize = 2;
        const ALPHA: usize = 3;
        const X: usize = 0;
        const Y: usize = 1;
        const Z: usize = 2;
        let one = |index: usize| index..index + 1;
        Some(match kind {
            ChaosEventKind::Scale => (std::slice::from_mut(&mut self.size), std::slice::from_mut(&mut self.size_step)),
            ChaosEventKind::Color => (&mut self.color[..], &mut self.color_step[..]),
            ChaosEventKind::RedColor => (&mut self.color[one(RED)], &mut self.color_step[one(RED)]),
            ChaosEventKind::GreenColor => (&mut self.color[one(GREEN)], &mut self.color_step[one(GREEN)]),
            ChaosEventKind::BlueColor => (&mut self.color[one(BLUE)], &mut self.color_step[one(BLUE)]),
            ChaosEventKind::Alpha => (&mut self.color[one(ALPHA)], &mut self.color_step[one(ALPHA)]),
            ChaosEventKind::Velocity => (&mut self.velocity[..], &mut self.velocity_step[..]),
            ChaosEventKind::VelocityX => (&mut self.velocity[one(X)], &mut self.velocity_step[one(X)]),
            ChaosEventKind::VelocityY => (&mut self.velocity[one(Y)], &mut self.velocity_step[one(Y)]),
            ChaosEventKind::VelocityZ => (&mut self.velocity[one(Z)], &mut self.velocity_step[one(Z)]),
            ChaosEventKind::Thrust => {
                (std::slice::from_mut(&mut self.thrust), std::slice::from_mut(&mut self.thrust_step))
            }
            ChaosEventKind::Rotation => (&mut self.rotation[..], &mut self.rotation_step[..]),
            ChaosEventKind::Length => {
                (std::slice::from_mut(&mut self.length), std::slice::from_mut(&mut self.length_step))
            }
            ChaosEventKind::EventTimer => return None,
        })
    }

    /// `Particle::Update` (0x008b6fb0). Returns false when the particle died.
    fn update(&mut self, time: f32, gravity: Vec3, flags: ChaosSequenceFlags, node_position: Vec3) -> bool {
        self.age = time - self.birth;
        if self.age >= self.lifetime {
            return false;
        }
        let dt = time - self.last_update;
        self.step = dt;
        self.event_timer += dt;
        self.previous_position = self.position;
        if flags.contains(ChaosSequenceFlags::STAY_AT_EMITTER) {
            self.position = node_position;
        } else {
            let mut velocity = Vec3::from_array(self.velocity);
            let mut thrust = Vec3::ZERO;
            if flags.contains(ChaosSequenceFlags::THRUST) && !velocity.is_zero() {
                self.thrust += self.thrust_step * dt;
                thrust = velocity.normalize_or_zero() * self.thrust;
            }
            velocity += (gravity + Vec3::from_array(self.velocity_step) + thrust) * dt;
            self.position += velocity * dt;
            self.velocity = velocity.to_array();
        }
        if flags.contains(ChaosSequenceFlags::ROTATION) {
            for (value, step) in self.rotation.iter_mut().zip(self.rotation_step) {
                *value += step * dt;
            }
        }
        for (value, step) in self.color.iter_mut().zip(self.color_step) {
            *value += step * dt;
        }
        self.size += self.size_step * dt;
        self.length += self.length_step * dt;
        self.last_update = time;
        true
    }
}

/// An event's `Apply` (vtable +4; 0x008af6f0 and siblings). Returns true when it changed the event timer.
fn apply_event(particle: &mut ChaosParticle, events: &[PreparedEvent], index: usize, rng: &mut (impl Rng + ?Sized)) -> bool {
    let event = &events[index];
    let components = event.kind.components();
    let value = &event.value;
    let Some((field, step)) = particle.channel(event.kind) else {
        if event.fade {
            return false;
        }
        particle.event_timer = sample_range(rng, value.min[0], value.max[0]);
        return true;
    };
    if !event.fade {
        for (component, field) in field.iter_mut().enumerate().take(components) {
            *field = sample_range(rng, value.min[component], value.max[component]);
        }
    }
    match event.next_fade {
        Some(next) => {
            let target = &events[next];
            let mut span = target.time - event.time;
            if span == 0.0 {
                span = ZERO_SPAN_REPLACEMENT;
            }
            let targets: Vec<f32> = (0..components)
                .map(|component| sample_range(rng, target.value.min[component], target.value.max[component]))
                .collect();
            for ((step, field), target) in step.iter_mut().zip(field.iter()).zip(targets) {
                *step = (target - *field) / span;
            }
        }
        None if !event.kind.keeps_step() => step.iter_mut().for_each(|step| *step = 0.0),
        None => {}
    }
    false
}

/// `ParticleEventSequence::RunEvents` (0x008b7f40).
fn run_events(particle: &mut ChaosParticle, events: &[PreparedEvent], rng: &mut (impl Rng + ?Sized)) {
    let mut index = particle.next_event;
    let mut rewinds = 0;
    while index < events.len() && particle.event_timer >= events[index].time {
        if apply_event(particle, events, index, rng) {
            let timer = particle.event_timer;
            index = events.partition_point(|event| event.time < timer);
            rewinds += 1;
            if rewinds >= MAX_EVENT_TIMER_REWINDS {
                break;
            }
        } else {
            index += 1;
        }
    }
    particle.next_event = index;
}

/// The node values the emitter read at its last update.
#[derive(Debug, Clone, Copy, PartialEq)]
struct NodeSample {
    position: Vec3,
    velocity: Vec3,
    rotation: Mat3,
    scale: f32,
}

impl Default for NodeSample {
    fn default() -> Self {
        Self { position: Vec3::ZERO, velocity: Vec3::ZERO, rotation: Mat3::IDENTITY, scale: 1.0 }
    }
}

#[derive(Debug, Clone, Default)]
struct SequenceState {
    /// Oldest first (the client's deque order).
    particles: Vec<ChaosParticle>,
    /// `last_emit` time of the sequence.
    last_emit: f32,
    /// Bound of the quad shape (centre and radius, in render space), from the last refresh; `None` before
    /// the first.
    bound: Option<(Vec3, f32)>,
    /// The clones of a MESH sequence whose `.nif` the [`MeshSource`] resolved.
    meshes: Option<MeshClones>,
}

/// The client's `Emitter`: one running instance of a compiled script.
#[derive(Debug, Clone)]
pub struct ChaosEmitter {
    system: Arc<ChaosSystem>,
    sequences: Vec<SequenceState>,
    started: bool,
    /// The node is attached (cleared by Stop: the emitter then reads stale node values with scale 1).
    attached: bool,
    one_shot: bool,
    infinite: bool,
    first_burst: bool,
    skip_first_update: bool,
    remaining: f32,
    node: NodeSample,
    /// Node position at the previous update (+0x4c).
    previous_node_position: Option<Vec3>,
    /// Time of the last bound refresh (`ParticleSet` +0x24).
    bound_time: f32,
}

impl ChaosEmitter {
    pub fn new(system: Arc<ChaosSystem>) -> Self {
        let sequences = vec![SequenceState::default(); system.desc().sequences.len()];
        Self {
            system,
            sequences,
            started: false,
            attached: false,
            one_shot: false,
            infinite: false,
            first_burst: false,
            skip_first_update: false,
            remaining: 0.0,
            node: NodeSample::default(),
            previous_node_position: None,
            bound_time: 0.0,
        }
    }

    /// Gives the MESH sequences whose `.nif` `source` resolves a clone per particle; the others keep drawing
    /// [`MeshInstance`] placements.
    pub fn set_mesh_source(&mut self, source: &MeshSource) {
        for (state, sequence) in self.sequences.iter_mut().zip(0..) {
            if self.system.desc().sequences[sequence].particle_type == ChaosParticleType::Mesh {
                let name = &self.system.prepared(sequence).resource;
                state.meshes = source.resolve(name).map(MeshClones::new);
            }
        }
    }

    /// The bound of sequence `sequence`'s quads (centre, radius) at the last refresh.
    pub fn bound(&self, sequence: usize) -> Option<(Vec3, f32)> {
        self.sequences[sequence].bound
    }

    pub fn system(&self) -> &Arc<ChaosSystem> {
        &self.system
    }

    /// `Emitter::Start(0, duration, not_looping)`:
    /// - looping: emit continuously for `duration` seconds, forever when `duration <= 0`;
    /// - not looping with `minEmitTime > 0`: emit continuously for `sample(minEmitTime)` seconds;
    /// - not looping otherwise: one burst.
    ///
    /// Every case starts with a burst, at the second update (the first update after Start emits nothing). A
    /// started emitter ignores Start.
    pub fn start(&mut self, rng: &mut (impl Rng + ?Sized), duration_s: f32, not_looping: bool) {
        if self.started {
            return;
        }
        let min_emit_time = self.system.desc().min_emit_time;
        let min_emit = sample_range(rng, min_emit_time.min, min_emit_time.max);
        let mut duration = duration_s;
        if not_looping && min_emit > 0.0 {
            self.one_shot = false;
            duration = min_emit;
        } else {
            self.one_shot = not_looping;
        }
        self.remaining = duration;
        self.infinite = duration <= 0.0;
        self.first_burst = true;
        self.started = true;
        self.attached = true;
        self.skip_first_update = true;
    }

    /// `Emitter::Stop(pool, true, false)` (0x008a1ce0): stops emitting, detaches the node and kills the
    /// particles of StayAtEmitter sequences. The others live out their lifetime.
    pub fn stop(&mut self) {
        self.started = false;
        self.attached = false;
        for (state, sequence) in self.sequences.iter_mut().zip(&self.system.desc().sequences) {
            if sequence.flags.contains(ChaosSequenceFlags::STAY_AT_EMITTER) {
                state.particles.clear();
            }
        }
    }

    /// Whether the emitter still emits (or will).
    pub fn is_active(&self) -> bool {
        self.started
    }

    pub fn particle_count(&self) -> usize {
        self.sequences.iter().map(|state| state.particles.len()).sum()
    }

    /// The live particles of a sequence, oldest first.
    pub fn particles(&self, sequence: usize) -> &[ChaosParticle] {
        &self.sequences[sequence].particles
    }

    /// `Emitter::Update` (0x008a13c0) at manager time `time` (seconds) after a step of `dt` seconds.
    pub fn update(&mut self, rng: &mut (impl Rng + ?Sized), time: f32, dt: f32, node: &NodeTransform) {
        self.read_node(dt, node);
        if self.skip_first_update {
            self.skip_first_update = false;
        } else if self.started {
            self.remaining -= dt;
            if self.first_burst || self.one_shot {
                self.burst(rng, time);
            } else if !self.infinite && self.remaining <= 0.0 {
                self.stop();
            } else {
                self.emit_continuous(rng, time);
            }
        }
        for sequence in 0..self.sequences.len() {
            self.update_sequence(rng, sequence, time);
        }
        self.update_set(time);
        self.previous_node_position = Some(self.node.position);
    }

    /// Where sequence `sequence` draws a particle: its position, plus the node position for LOCAL sequences.
    fn render_position(&self, sequence: usize, particle: &ChaosParticle) -> Vec3 {
        if self.system.desc().sequences[sequence].flags.contains(ChaosSequenceFlags::LOCAL) {
            particle.position + self.node.position
        } else {
            particle.position
        }
    }

    /// `ParticleSet::Update` (0x008b2de0): every [`BOUND_INTERVAL_S`], the bound of each quad sequence
    /// (POINT, AXIAL) with live particles: centre = the centre of the box of the particle positions, radius =
    /// the largest distance from it plus [`BOUND_RADIUS_MARGIN`]; every update, the MESH clones.
    fn update_set(&mut self, time: f32) {
        let refresh = time - self.bound_time >= BOUND_INTERVAL_S;
        if refresh {
            self.bound_time = time;
        }
        for sequence in 0..self.sequences.len() {
            let desc = &self.system.desc().sequences[sequence];
            let positions: Vec<Vec3> =
                self.sequences[sequence].particles.iter().map(|particle| self.render_position(sequence, particle)).collect();
            match desc.particle_type {
                ChaosParticleType::Point | ChaosParticleType::Axial if refresh && !positions.is_empty() => {
                    self.sequences[sequence].bound = Some(bound_of(&positions));
                }
                ChaosParticleType::Mesh => {
                    let rotates = desc.flags.contains(ChaosSequenceFlags::ROTATION);
                    let node_rotation = self.node.rotation;
                    let state = &mut self.sequences[sequence];
                    let Some(meshes) = &mut state.meshes else { continue };
                    meshes.update(state.particles.iter().zip(positions).map(|(particle, position)| MeshPlacement {
                        position,
                        // `ParticleSet::Update` case 2 multiplies Euler · node rotation (row-major, M · v).
                        rotation: if rotates {
                            euler_xyz(particle.rotation).mul_mat3(&node_rotation)
                        } else {
                            node_rotation
                        },
                        size: particle.size,
                        age: particle.age,
                        step: particle.step,
                    }));
                }
                _ => {}
            }
        }
    }

    /// Reads the node like the client: position, velocity, scale and rotation from the owner; stale values and
    /// scale 1 without one.
    fn read_node(&mut self, dt: f32, node: &NodeTransform) {
        if !self.attached {
            self.node.scale = DETACHED_NODE_SCALE;
            return;
        }
        let velocity = node.velocity.unwrap_or_else(|| match self.previous_node_position {
            Some(previous) if dt > 0.0 => (node.position - previous) * (1.0 / dt),
            Some(_) => self.node.velocity,
            None => Vec3::ZERO,
        });
        self.node = NodeSample { position: node.position, velocity, rotation: node.rotation, scale: node.scale };
    }

    fn burst(&mut self, rng: &mut (impl Rng + ?Sized), time: f32) {
        for sequence in 0..self.sequences.len() {
            let count = sample_int_range(rng, self.system.desc().sequences[sequence].burst);
            for _ in 0..count {
                if !self.create_particle(rng, sequence, time) {
                    break;
                }
            }
            self.sequences[sequence].last_emit = time;
        }
        self.first_burst = false;
        if self.one_shot {
            self.stop();
            self.one_shot = false;
        }
    }

    fn emit_continuous(&mut self, rng: &mut (impl Rng + ?Sized), time: f32) {
        let movement = self.previous_node_position.map_or(0.0, |previous| (self.node.position - previous).length());
        for sequence in 0..self.sequences.len() {
            let desc = &self.system.desc().sequences[sequence];
            let boost = if desc.flags.contains(ChaosSequenceFlags::EMISSION_BOOST) {
                movement * sample_range(rng, desc.emission_boost.min, desc.emission_boost.max)
            } else {
                0.0
            };
            let mut interval = 1.0 / (desc.emit_rate.max + boost);
            // `!(interval >= min)` also catches a NaN or negative rate, as the client's compare does.
            if !(interval >= MIN_EMIT_INTERVAL_S) {
                interval = MIN_EMIT_INTERVAL_S;
            }
            let burst = desc.burst;
            // The client compares with a global substep time far in the past, so the first test always passes.
            let mut last_substep: Option<f32> = None;
            let mut t = interval + self.sequences[sequence].last_emit;
            'emit: while t <= time {
                let count = sample_int_range(rng, burst);
                for _ in 0..count {
                    if !self.create_particle(rng, sequence, t) {
                        self.sequences[sequence].last_emit = time;
                        break 'emit;
                    }
                }
                self.sequences[sequence].last_emit = t;
                let due = last_substep.is_none_or(|last| t - last >= SUBSTEP_INTERVAL_S);
                if due && !self.sequences[sequence].particles.is_empty() {
                    self.update_sequence(rng, sequence, t);
                    last_substep = Some(t);
                }
                t += interval;
            }
        }
    }

    /// `ParticleEventSequence::CreateParticle` (0x008b7ae0).
    fn create_particle(&mut self, rng: &mut (impl Rng + ?Sized), sequence: usize, time: f32) -> bool {
        let desc = &self.system.desc().sequences[sequence];
        let state = &self.sequences[sequence];
        if state.particles.len() >= usize::from(desc.num_particles) {
            return false;
        }
        let node = self.node;
        let events = &self.system.prepared(sequence).events;
        let mut particle = ChaosParticle::template(time);
        particle.lifetime = sample_range(rng, desc.lifetime.min, desc.lifetime.max);
        let offset = Vec3::from_array(sample_components(rng, &desc.chaos_location.min, &desc.chaos_location.max));
        let mut location = node.rotation.mul_vec3(offset * node.scale);
        if !desc.flags.contains(ChaosSequenceFlags::LOCAL) {
            location += node.position;
        }
        particle.position = location;
        particle.previous_position = location;
        let mut index = 0;
        while index < events.len() && events[index].time == 0.0 {
            apply_event(&mut particle, events, index, rng);
            index += 1;
        }
        let chaos_velocity = Vec3::from_array(sample_components(rng, &desc.chaos_velocity.min, &desc.chaos_velocity.max));
        let velocity = Vec3::from_array(particle.velocity) + chaos_velocity;
        let inherited = Vec3::from_array(sample_components(rng, &desc.inherited.min, &desc.inherited.max));
        let velocity = node.rotation.mul_vec3(velocity) + node.velocity.mul_elements(inherited);
        particle.size *= node.scale;
        particle.size_step *= node.scale;
        particle.velocity = (velocity * node.scale).to_array();
        particle.velocity_step = (Vec3::from_array(particle.velocity_step) * node.scale).to_array();
        particle.next_event = index;
        self.sequences[sequence].particles.push(particle);
        true
    }

    /// `ParticleEventSequence::Update` (0x008b7fb0): gravity sampled once, then every particle updated, the
    /// dead removed and the events of the living run.
    fn update_sequence(&mut self, rng: &mut (impl Rng + ?Sized), sequence: usize, time: f32) {
        let desc: &ChaosSequenceDesc = &self.system.desc().sequences[sequence];
        let events = &self.system.prepared(sequence).events;
        let state = &mut self.sequences[sequence];
        if state.particles.is_empty() {
            return;
        }
        let gravity =
            Vec3::from_array(sample_components(rng, &desc.gravity.min, &desc.gravity.max)).mul_elements(GRAVITY_AXIS);
        let node_position = self.node.position;
        state.particles.retain_mut(|particle| {
            if !particle.update(time, gravity, desc.flags, node_position) {
                return false;
            }
            run_events(particle, events, rng);
            true
        });
    }

    /// The hand-off to the renderer (end of `Emitter::Update`, `ParticleTriShape::BuildVertices` 0x008c4c30,
    /// `SetCelTexCoords` 0x008c5970, `MeshParticle`): one batch per drawn sequence, in sequence order.
    pub fn render(&self, out: &mut RenderFrame) {
        let node = self.node;
        // Mesh clone draws are numbered after the quad batches (which have no `BatchState`: order 0).
        let mut order = 1;
        for (sequence, state) in self.sequences.iter().enumerate() {
            if state.particles.is_empty() {
                continue;
            }
            let desc = &self.system.desc().sequences[sequence];
            let resource = &self.system.prepared(sequence).resource;
            let local = desc.flags.contains(ChaosSequenceFlags::LOCAL);
            let rotates = desc.flags.contains(ChaosSequenceFlags::ROTATION);
            let render_position = |particle: &ChaosParticle| self.render_position(sequence, particle);
            let kind = match desc.particle_type {
                ChaosParticleType::Point if rotates => SpriteKind::RotatedBillboard,
                ChaosParticleType::Point => SpriteKind::Billboard,
                ChaosParticleType::Axial => SpriteKind::Axial,
                ChaosParticleType::Mesh => {
                    if let Some(meshes) = &state.meshes {
                        order = meshes.render(out, order);
                        continue;
                    }
                    for particle in &state.particles {
                        out.meshes.push(MeshInstance {
                            mesh: Arc::clone(resource),
                            translation: render_position(particle),
                            scale: particle.size,
                            node_rotation: node.rotation,
                            euler: rotates.then(|| Vec3::from_array(particle.rotation)),
                            animation_time: particle.age,
                            color: clamp_color(particle.color),
                        });
                    }
                    continue;
                }
                ChaosParticleType::None
                | ChaosParticleType::Billboard
                | ChaosParticleType::Emitter
                | ChaosParticleType::Light
                | ChaosParticleType::Test => continue,
            };
            let start = out.sprites.len();
            for particle in &state.particles {
                let mut sprite = SpriteInstance {
                    position: render_position(particle),
                    size: particle.size,
                    rotation: 0.0,
                    color: clamp_color(particle.color),
                    uv: cel_uv(desc, particle.age),
                    direction: Vec3::ZERO,
                    length: 0.0,
                };
                match kind {
                    SpriteKind::Axial => {
                        let motion = Vec3::from_array(particle.velocity) + node.velocity;
                        if motion.is_zero() {
                            continue;
                        }
                        sprite.direction = motion.normalize_or_zero();
                        sprite.length = particle.length * node.scale;
                    }
                    // Only the y component spins the quad.
                    SpriteKind::RotatedBillboard => sprite.rotation = particle.rotation[1],
                    SpriteKind::Billboard => {}
                }
                out.sprites.push(sprite);
            }
            if out.sprites.len() > start {
                out.batches.push(DrawBatch {
                    texture: Arc::clone(resource),
                    blend: BlendState { src: desc.src_blend, dst: desc.dst_blend },
                    depth: PARTICLE_DEPTH_STATE,
                    two_sided: kind == SpriteKind::Axial,
                    kind,
                    sprites: start..out.sprites.len(),
                    // Before the first refresh the client's bound is the template quads' (at the shape origin).
                    sort: SortKey::sorted(state.bound.map_or(if local { node.position } else { Vec3::ZERO }, |(center, _)| center)),
                });
            }
        }
    }
}

/// Centre of the box of `positions` and the largest distance from it plus [`BOUND_RADIUS_MARGIN`].
fn bound_of(positions: &[Vec3]) -> (Vec3, f32) {
    let first = positions[0];
    let (min, max) = positions.iter().fold((first, first), |(min, max), p| {
        (Vec3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z)), Vec3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z)))
    });
    let center = (min + max) * 0.5;
    let squared = positions.iter().map(|&p| (p - center).dot(p - center)).fold(0.0, f32::max);
    (center, squared.sqrt() + BOUND_RADIUS_MARGIN)
}

fn clamp_color(color: [f32; 4]) -> [f32; 4] {
    color.map(|channel| channel.clamp(COLOR_RANGE.0, COLOR_RANGE.1))
}

/// The flipbook cell of a particle (`SetCelTexCoords` 0x008c5970). Frame and column counts of 0, which would
/// fault the client, are read as 1. Negative steps (a tiling variant no data uses) are not implemented: the
/// rectangle is used as is.
pub(crate) fn cel_uv(desc: &ChaosSequenceDesc, age: f32) -> UvRect {
    if !desc.flags.contains(ChaosSequenceFlags::CEL_ANIMATE) {
        return UvRect::FULL;
    }
    let cel = &desc.cel;
    let frames = cel.frames.max(1);
    let columns = u16::from(cel.columns.max(1));
    let mut frame = (age * cel.fps) as i32 as u16;
    if desc.flags.contains(ChaosSequenceFlags::CEL_LOOP) {
        frame %= frames;
    }
    frame = frame.min(frames - 1);
    let column = f32::from(frame % columns);
    let row = f32::from(frame / columns);
    let u0 = column * cel.u_step;
    let v0 = row * cel.v_step;
    UvRect { u0, v0, u1: u0 + cel.u_step, v1: v0 + cel.v_step }
}
