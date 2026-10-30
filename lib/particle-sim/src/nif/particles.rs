//! Gamebryo particle systems as UOSA.exe runs them: `NiParticleSystem::UpdateSystem`, the `NiPSysData` pool, the
//! emitters, the modifiers and the emitter controller.
//!
//! Every function names the client routine it follows. Random numbers come from the C runtime `rand()`
//! ([`CrtRng`]) as in the client: a unit random is `rand() * (1 / RAND_MAX)`, a symmetric one
//! `rand() * (2 / RAND_MAX) - 1`, drawn in the client's order.

use std::sync::Arc;

use super::desc::{
    BombDecay, BombSymmetry, BoolInterp, ControllerTiming, EmitterShape, FloatInterp, MeshEmission,
    MeshEmitterVelocity, NifEmitter, NifMesh, NifModifierKind, NifParticleSystem, NifSpawn,
};
use super::keys::eval_bool_keys;
use super::scene::{NodeState, Xf};
use crate::math::{Mat3, Vec3};
use crate::rng::CrtRng;

/// `1 / RAND_MAX`, the unit random scale (UOSA.exe `DAT_00d04f08`).
const UNIT_RANDOM_SCALE: f32 = 3.051_850_9e-5;
/// `2 / RAND_MAX`, the symmetric random scale (`DAT_00d04fc8`).
const SYMMETRIC_RANDOM_SCALE: f32 = 6.103_701_9e-5;
/// Centre of a unit random (`DAT_00c95604`): `(u - 0.5) * var + value`.
const RANDOM_CENTER: f32 = 0.5;
/// 2π as the client stores it (`DAT_00ddfd44`).
const TWO_PI: f32 = 6.283_185_5;
/// π (`DAT_00c9b0d0`).
const PI: f32 = 3.141_592_7;
/// `π / RAND_MAX` (`DAT_00d051a0`): the random polar angle of `NiPSysRotationModifier`'s random axis.
const RANDOM_POLAR_SCALE: f32 = 9.587_672_7e-5;
/// Smallest scale and size (`DAT_00c981d0`): `NiPSysGrowFadeModifier`'s floor, the floor of the particle system
/// scale in relative transforms (0x00811f80) and the spawn alignment threshold.
const MIN_SCALE: f32 = 9.999_999_7e-5;
/// Rotation angles above this reset to 0 (`NiPSysRotationModifier::Update` 0x0080a460, `DAT_00d05078`, 10π).
const ROTATION_RESET_ANGLE: f32 = 31.415_928;
/// `NiPSysGravityModifier` strength scale (0x0081de40, `DAT_00d05228`).
const GRAVITY_FORCE_SCALE: f32 = 1.6;
/// `NiPSysGravityModifier` turbulence scale (`DAT_00d05224`).
const TURBULENCE_SCALE: f32 = 500.0;
/// `NiPSysDragModifier` time scale (0x00819540, `DAT_00d0522c`, about 30 per second).
const DRAG_TIME_SCALE: f32 = 30.000_029;
/// Centroid weight of a mesh emitter face (0x008216f0, `DAT_00cdb4f0`; assumed 1/3).
const FACE_CENTER_WEIGHT: f32 = 1.0 / 3.0;
/// Midpoint weight of a mesh emitter edge (0x00821c50, `DAT_00c95604`).
const EDGE_CENTER_WEIGHT: f32 = 0.5;
/// Triangles a mesh emitter tries before using a degenerate one (0x008216f0: 6 attempts).
const MESH_FACE_ATTEMPTS: u32 = 6;
/// Edges of a triangle (`rand() % 3` picks one, 0x00821c50).
const TRIANGLE_EDGES: u16 = 3;
/// Fast inverse square root magic of the face surface sampling (0x008216f0).
const RSQRT_MAGIC: u32 = 0x5f37_59df;
/// Its Newton step constant (`DAT_00c803d0`; assumed 1.5).
const RSQRT_NEWTON: f32 = 1.5;
/// Size of an emitted particle (0x00828e20, `DAT_00c981c8`).
const EMITTED_SIZE: f32 = 1.0;
/// Radius of particles of a system without radii (not used by the client's files; a neutral default).
const DEFAULT_RADIUS: f32 = 1.0;
/// White, the colour of particles of a system without colours.
const WHITE: [f32; 4] = [1.0; 4];
/// Midpoint of a birth rate interval (`NiPSEmitParticlesCtlr::UpdateParticleSystem`).
const BIRTH_RATE_MIDPOINT: f32 = 0.5;
/// Guard of the emitter key walk: the SDK loop can revisit a final active key forever; the client's files
/// need at most a few passes.
const MAX_EMITTER_KEY_PASSES: u32 = 64;
/// Spawn count rounding: a fractional part above this adds one (0x0080aed0).
const SPAWN_ROUND_UP: f32 = 0.5;
/// The fractional part's divisor of the spawn rounding (`fmod(x, 1)`, 0x00bd6eaa).
const SPAWN_ROUND_MODULUS: f32 = 1.0;
/// A speed sign coin: unit randoms at or below this negate the rotation speed (0x0080a540).
const RANDOM_SIGN_THRESHOLD: f32 = 0.5;

fn unit_random(rng: &mut impl CrtRng) -> f32 {
    f32::from(rng.rand()) * UNIT_RANDOM_SCALE
}

fn symmetric_random(rng: &mut impl CrtRng) -> f32 {
    f32::from(rng.rand()) * SYMMETRIC_RANDOM_SCALE - 1.0
}

/// `(u - 0.5) * var + value`.
fn centered_random(rng: &mut impl CrtRng, value: f32, var: f32) -> f32 {
    (unit_random(rng) - RANDOM_CENTER) * var + value
}

/// The direction of declination `dec` from +Z and planar angle `planar` about it.
fn direction(declination: f32, planar: f32) -> Vec3 {
    let (sin_dec, cos_dec) = declination.sin_cos();
    let (sin_planar, cos_planar) = planar.sin_cos();
    Vec3::new(cos_planar * sin_dec, sin_planar * sin_dec, cos_dec)
}

/// The fast square root of the mesh emitter face sampling: `x * rsqrt(x)` with one Newton step.
fn fast_sqrt(x: f32) -> f32 {
    let y = f32::from_bits(RSQRT_MAGIC.wrapping_sub(x.to_bits() >> 1));
    let y = (RSQRT_NEWTON - x * RANDOM_CENTER * y * y) * y;
    y * x
}

/// One particle of `NiPSysData`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NifParticle {
    /// In the particle system's space (world space for world-space systems).
    pub position: Vec3,
    pub velocity: Vec3,
    pub age: f32,
    pub life_span: f32,
    pub last_update: f32,
    pub generation: u16,
    pub color: [f32; 4],
    pub radius: f32,
    pub size: f32,
    pub rotation_angle: f32,
    pub rotation_speed: f32,
    pub rotation_axis: Vec3,
}

impl Default for NifParticle {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            age: 0.0,
            life_span: 0.0,
            last_update: 0.0,
            generation: 0,
            color: WHITE,
            radius: DEFAULT_RADIUS,
            size: EMITTED_SIZE,
            rotation_angle: 0.0,
            rotation_speed: 0.0,
            rotation_axis: Vec3::Z,
        }
    }
}

/// The particle arrays: active particles, and particles added since the last `ResolveAddedParticles`.
#[derive(Debug, Clone, Default)]
pub(crate) struct ParticlePool {
    pub active: Vec<NifParticle>,
    pub added: Vec<NifParticle>,
    pub capacity: usize,
}

impl ParticlePool {
    /// `NiPSysData::AddParticle` returns `0xffff` when the arrays are full.
    fn is_full(&self) -> bool {
        self.active.len() + self.added.len() >= self.capacity
    }

    fn resolve_added(&mut self) {
        self.active.append(&mut self.added);
    }

    fn clear(&mut self) {
        self.active.clear();
        self.added.clear();
    }

    pub fn len(&self) -> usize {
        self.active.len() + self.added.len()
    }
}

/// A modifier with its animatable state.
#[derive(Debug, Clone)]
pub(crate) struct ModifierState {
    pub active: bool,
    /// The description's parameters, changed by modifier controllers.
    pub kind: NifModifierKind,
}

/// The runtime of one particle system.
#[derive(Debug, Clone, Default)]
pub(crate) struct SystemState {
    pub pool: ParticlePool,
    pub modifiers: Vec<ModifierState>,
    /// `NiParticleSystem` +0xf8, the last update time (`None`: the `-FLT_MAX` sentinel).
    pub last_time: Option<f32>,
    /// +0xfc: set by `NiPSysResetOnLoopCtlr`.
    pub reset_pending: bool,
}

/// A particle spawned into another particle system, applied after the source system's update.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DeferredSpawn {
    pub source_system: usize,
    pub target_system: usize,
    pub modifier: usize,
    pub time: f32,
    pub spawn_time: f32,
    pub particle: NifParticle,
}

/// What a particle system update reads from the scene.
pub(crate) struct SystemContext<'a> {
    pub index: usize,
    pub desc: &'a NifParticleSystem,
    pub nodes: &'a [NodeState],
    pub meshes: &'a [NifMesh],
    /// Current (morphed) vertex positions per mesh, `None` for the stored ones.
    pub mesh_positions: &'a [Option<Arc<[[f32; 3]]>>],
}

impl SystemContext<'_> {
    /// The particle system's world transform (identity for world-space systems).
    fn world(&self) -> Xf {
        self.nodes[self.desc.node].world
    }

    /// The transform from `object`'s space to the particle system's (0x00811f80): identity without an object or
    /// for the system itself; the system's scale is floored at [`MIN_SCALE`] before inverting.
    fn relative(&self, object: Option<usize>) -> Xf {
        match object {
            Some(object) if object != self.desc.node && object < self.nodes.len() => {
                let mut world = self.world();
                if world.scale <= MIN_SCALE {
                    world.scale = MIN_SCALE;
                }
                world.inverse().mul(&self.nodes[object].world)
            }
            _ => Xf::IDENTITY,
        }
    }
}

impl SystemState {
    pub fn new(desc: &NifParticleSystem) -> Self {
        Self {
            pool: ParticlePool { capacity: usize::from(desc.capacity), ..Default::default() },
            modifiers: desc
                .modifiers
                .iter()
                .map(|modifier| ModifierState { active: modifier.active, kind: modifier.kind.clone() })
                .collect(),
            last_time: None,
            reset_pending: false,
        }
    }

    /// `NiParticleSystem::UpdateSystem` (0x00811390).
    pub fn update(
        &mut self,
        ctx: &SystemContext,
        time: f32,
        rng: &mut impl CrtRng,
        deferred: &mut Vec<DeferredSpawn>,
    ) {
        self.pool.resolve_added();
        if self.reset_pending || self.last_time.is_some_and(|last| time < last) {
            self.pool.clear();
            self.reset_pending = false;
        }
        let Self { pool, modifiers, .. } = self;
        for modifier in modifiers.iter() {
            if modifier.active {
                update_modifier(&modifier.kind, modifiers, pool, ctx, time, rng, deferred);
            }
        }
        self.last_time = Some(time);
    }

    /// `NiPSysEmitter::EmitParticles` (0x00828e20) of emitter modifier `emitter`: one particle per age.
    pub fn emit(&mut self, ctx: &SystemContext, emitter: usize, time: f32, ages: &[f32], rng: &mut impl CrtRng) {
        let Self { pool, modifiers, .. } = self;
        let Some(NifModifierKind::Emitter(params)) = modifiers.get(emitter).map(|modifier| &modifier.kind) else {
            return;
        };
        for &age in ages {
            let life_span = centered_random(rng, params.life_span, params.life_span_var);
            if !(age <= life_span) {
                continue;
            }
            if pool.is_full() {
                return;
            }
            let speed = centered_random(rng, params.speed, params.speed_var);
            let declination = params.declination_var * symmetric_random(rng) + params.declination;
            let planar = params.planar_angle_var * symmetric_random(rng) + params.planar_angle;
            let mut particle = NifParticle {
                velocity: direction(declination, planar) * speed,
                age,
                life_span,
                ..Default::default()
            };
            initial_position_and_velocity(params, ctx, &mut particle.position, &mut particle.velocity, rng);
            if ctx.desc.has_colors {
                particle.color = params.initial_color;
            }
            if ctx.desc.has_radii {
                particle.radius = params.radius_var * symmetric_random(rng) + params.initial_radius;
            }
            particle.last_update = time - age;
            initialize_particle(modifiers, ctx.desc, &mut particle, rng);
            pool.added.push(particle);
        }
    }
}

/// `ComputeInitialPositionAndVelocity` of the emitters (vtable +0x60): volume emitters (0x00828a10) place the
/// particle in the emitter object's space and rotate the velocity into the system's; mesh emitters (0x00821420).
fn initial_position_and_velocity(
    params: &NifEmitter,
    ctx: &SystemContext,
    position: &mut Vec3,
    velocity: &mut Vec3,
    rng: &mut impl CrtRng,
) {
    let (object, local) = match &params.shape {
        // 0x008164f0: depth, height, width randoms (z, y, x).
        EmitterShape::Box { object, width, height, depth } => {
            let z = (unit_random(rng) - RANDOM_CENTER) * depth;
            let y = (unit_random(rng) - RANDOM_CENTER) * height;
            let x = (unit_random(rng) - RANDOM_CENTER) * width;
            (*object, Vec3::new(x, y, z))
        }
        // 0x0080c470.
        EmitterShape::Sphere { object, radius } => {
            let distance = radius * unit_random(rng);
            let azimuth = TWO_PI * unit_random(rng);
            let polar = TWO_PI * unit_random(rng);
            let (sin_polar, cos_polar) = polar.sin_cos();
            let (sin_azimuth, cos_azimuth) = azimuth.sin_cos();
            (*object, Vec3::new(sin_polar * cos_azimuth, sin_polar * sin_azimuth, cos_polar) * distance)
        }
        // 0x008177c0.
        EmitterShape::Cylinder { object, radius, height } => {
            let distance = radius * unit_random(rng);
            let angle = TWO_PI * unit_random(rng);
            let (sin, cos) = angle.sin_cos();
            let z = (unit_random(rng) - RANDOM_CENTER) * height;
            (*object, Vec3::new(cos * distance, sin * distance, z))
        }
        EmitterShape::Mesh { meshes, velocity: velocity_type, emission, emit_axis } => {
            mesh_position_and_velocity(params, ctx, meshes, *velocity_type, *emission, *emit_axis, position, velocity, rng);
            return;
        }
    };
    let transform = ctx.relative(object);
    *position = transform.point(local);
    *velocity = transform.rotation.mul_vec3(*velocity);
}

/// `NiPSysMeshEmitter::ComputeInitialPositionAndVelocity` (0x00821420): a random mesh, then a vertex (0x008215b0),
/// a face (0x008216f0) or an edge (0x00821c50) of it, then the mesh transform and the velocity type (0x00822c90).
/// Skinned meshes are not handled (the client's files have none).
fn mesh_position_and_velocity(
    params: &NifEmitter,
    ctx: &SystemContext,
    meshes: &[usize],
    velocity_type: MeshEmitterVelocity,
    emission: MeshEmission,
    emit_axis: [f32; 3],
    position: &mut Vec3,
    velocity: &mut Vec3,
    rng: &mut impl CrtRng,
) {
    let count = meshes.len();
    if count == 0 {
        return;
    }
    let mut pick = (count as f32 * unit_random(rng)).round_ties_even() as usize;
    if pick == count {
        pick = count - 1;
    }
    let Some(mesh) = ctx.meshes.get(meshes[pick]) else {
        return;
    };
    let positions: &[[f32; 3]] =
        ctx.mesh_positions.get(meshes[pick]).and_then(|morphed| morphed.as_deref()).unwrap_or(&mesh.geometry.positions);
    let normals = &mesh.geometry.normals;
    let vertex = |index: u16| positions.get(usize::from(index)).map_or(Vec3::ZERO, |&p| Vec3::from_array(p));
    let normal = |index: u16| normals.get(usize::from(index)).map_or(Vec3::ZERO, |&n| Vec3::from_array(n));
    let use_normals = velocity_type == MeshEmitterVelocity::Normals && !normals.is_empty();
    let speed = velocity.length();
    match emission {
        MeshEmission::Vertices => {
            if positions.is_empty() {
                return;
            }
            let last = (positions.len() - 1) as f32;
            let index = (unit_random(rng) * last).round_ties_even() as u16;
            *position = vertex(index);
            if use_normals {
                *velocity = normal(index) * speed;
            }
        }
        MeshEmission::FaceCenter | MeshEmission::FaceSurface | MeshEmission::EdgeCenter | MeshEmission::EdgeSurface => {
            let triangles = &mesh.geometry.triangles;
            if triangles.is_empty() {
                return;
            }
            let count = triangles.len();
            let mut index = (count as f32 * unit_random(rng)).round_ties_even() as usize;
            if index == count {
                index = count - 1;
            }
            let mut triangle = triangles[index];
            for _ in 0..MESH_FACE_ATTEMPTS {
                triangle = triangles[index];
                let [a, b, c] = triangle;
                if a != b && a != c && b != c {
                    break;
                }
                index = (index + 1) % count;
            }
            let [a, b, c] = triangle;
            if matches!(emission, MeshEmission::FaceCenter | MeshEmission::FaceSurface) {
                *position = (vertex(a) + vertex(b) + vertex(c)) * FACE_CENTER_WEIGHT;
                if use_normals {
                    *velocity = ((normal(a) + normal(b) + normal(c)) * FACE_CENTER_WEIGHT).normalize_or_zero() * speed;
                }
                if emission == MeshEmission::FaceSurface {
                    let base = vertex(a);
                    let edge1 = vertex(b) - base;
                    let edge2 = vertex(c) - base;
                    let s = fast_sqrt(unit_random(rng));
                    let t = unit_random(rng);
                    *position = (edge2 * t - edge1) * s + base + edge1;
                }
            } else {
                let (from, to) = match rng.rand() % TRIANGLE_EDGES {
                    1 => (b, c),
                    2 => (a, c),
                    _ => (a, b),
                };
                let (p0, p1) = (vertex(from), vertex(to));
                *position = (p1 + p0) * EDGE_CENTER_WEIGHT;
                if use_normals {
                    *velocity = ((normal(to) + normal(from)) * EDGE_CENTER_WEIGHT).normalize_or_zero() * speed;
                }
                if emission == MeshEmission::EdgeSurface {
                    let t = unit_random(rng);
                    *position = (p1 - p0) * t + p0;
                }
            }
        }
    }
    let transform = ctx.relative(Some(mesh.node));
    *position = transform.point(*position);
    match velocity_type {
        MeshEmitterVelocity::Normals => *velocity = transform.rotation.mul_vec3(*velocity),
        MeshEmitterVelocity::Random => {
            let x = symmetric_random(rng);
            let y = symmetric_random(rng);
            let z = symmetric_random(rng);
            let direction = Vec3::new(x, y, z).normalize_or_zero();
            *velocity = direction * centered_random(rng, params.speed, params.speed_var);
        }
        MeshEmitterVelocity::Direction => {
            let speed = centered_random(rng, params.speed, params.speed_var);
            let axis = Vec3::from_array(emit_axis).normalize_or_zero();
            *velocity = transform.rotation.mul_vec3(axis) * speed;
        }
    }
}

/// The `Initialize` of every modifier for a new particle (0x008115b0), active or not: only
/// `NiPSysRotationModifier` (0x0080a540) draws anything. `NiPSysMeshUpdateModifier`'s mesh cloning is a render
/// matter here.
fn initialize_particle(
    modifiers: &[ModifierState],
    desc: &NifParticleSystem,
    particle: &mut NifParticle,
    rng: &mut impl CrtRng,
) {
    for modifier in modifiers {
        if let NifModifierKind::Rotation {
            initial_speed,
            initial_speed_var,
            initial_angle,
            initial_angle_var,
            random_speed_sign,
            random_initial_axis,
            initial_axis,
        } = &modifier.kind
        {
            if desc.has_rotation_axes {
                particle.rotation_axis = if *random_initial_axis {
                    let cos_polar = (f32::from(rng.rand()) * RANDOM_POLAR_SCALE).cos();
                    let sin_polar = (1.0 - cos_polar * cos_polar).sqrt();
                    let azimuth = TWO_PI * unit_random(rng);
                    Vec3::new(azimuth.cos() * sin_polar, azimuth.sin() * sin_polar, cos_polar)
                } else {
                    Vec3::from_array(*initial_axis)
                };
            }
            if desc.has_rotation_angles {
                particle.rotation_angle = initial_angle_var * symmetric_random(rng) + initial_angle;
                let speed = initial_speed_var * symmetric_random(rng) + initial_speed;
                particle.rotation_speed =
                    if *random_speed_sign && unit_random(rng) <= RANDOM_SIGN_THRESHOLD { -0.0 - speed } else { speed };
            }
        }
    }
}

/// The rotation that takes +Z to the direction of `velocity` (the spawn modifier's frame, 0x0080afc0): identity
/// for a zero velocity, a half turn about X for -Z, otherwise the shortest rotation.
fn align_z_to(velocity: Vec3) -> Mat3 {
    let length = velocity.length();
    if length <= MIN_SCALE {
        return Mat3::IDENTITY;
    }
    let v = velocity * (1.0 / length);
    let one_plus_z = 1.0 + v.z;
    if one_plus_z <= MIN_SCALE {
        return Mat3::from_rows([[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]]);
    }
    // Rodrigues with axis Z × v = (-y, x, 0): R = I + K + K² / (1 + z).
    let (x, y) = (v.x, v.y);
    let k = 1.0 / one_plus_z;
    Mat3::from_rows([[1.0 - x * x * k, -x * y * k, x], [-x * y * k, 1.0 - y * y * k, y], [-x, -y, v.z]])
}

/// `NiPSysSpawnModifier::SpawnParticles` (0x0080aed0) from `source` (already in the target's space).
pub(crate) fn spawn_particles(
    spawn: &NifSpawn,
    source: &NifParticle,
    time: f32,
    spawn_time: f32,
    pool: &mut ParticlePool,
    modifiers: &[ModifierState],
    desc: &NifParticleSystem,
    rng: &mut impl CrtRng,
) {
    if source.generation >= spawn.num_spawn_generations || !(unit_random(rng) <= spawn.percentage_spawned) {
        return;
    }
    let range = (i32::from(spawn.max_num_to_spawn) - i32::from(spawn.min_num_to_spawn)) as f32 * unit_random(rng);
    let mut variation = range as i32 as i16;
    if range % SPAWN_ROUND_MODULUS > SPAWN_ROUND_UP {
        variation = variation.wrapping_add(1);
    }
    let count = match spawn.min_num_to_spawn.wrapping_add(variation as u16) {
        0 => 1,
        count => count,
    };
    for _ in 0..count {
        spawn_particle(spawn, source, time, spawn_time, pool, modifiers, desc, rng);
    }
}

/// `NiPSysSpawnModifier::SpawnParticle` (0x0080afc0).
fn spawn_particle(
    spawn: &NifSpawn,
    source: &NifParticle,
    time: f32,
    spawn_time: f32,
    pool: &mut ParticlePool,
    modifiers: &[ModifierState],
    desc: &NifParticleSystem,
    rng: &mut impl CrtRng,
) {
    if pool.is_full() {
        return;
    }
    let speed = (spawn.spawn_speed_chaos * unit_random(rng) + 1.0) * source.velocity.length();
    let declination = spawn.spawn_dir_chaos * unit_random(rng) * PI;
    let planar = unit_random(rng) * TWO_PI;
    let velocity = align_z_to(source.velocity).mul_vec3(direction(declination, planar)) * speed;
    let age = time - spawn_time;
    let life_span = centered_random(rng, spawn.life_span, spawn.life_span_var);
    let mut particle = NifParticle {
        velocity,
        age,
        life_span,
        generation: source.generation.wrapping_add(1),
        last_update: time - age,
        ..*source
    };
    initialize_particle(modifiers, desc, &mut particle, rng);
    pool.added.push(particle);
}

/// One modifier's `Update` (vtable +0x48).
fn update_modifier(
    kind: &NifModifierKind,
    modifiers: &[ModifierState],
    pool: &mut ParticlePool,
    ctx: &SystemContext,
    time: f32,
    rng: &mut impl CrtRng,
    deferred: &mut Vec<DeferredSpawn>,
) {
    match kind {
        NifModifierKind::AgeDeath { spawn_on_death, spawn_modifier } => {
            let spawn = spawn_modifier.filter(|_| *spawn_on_death);
            age_death(spawn, modifiers, pool, ctx, time, rng, deferred);
        }
        NifModifierKind::Position => {
            // 0x00827a10.
            pool.resolve_added();
            for particle in &mut pool.active {
                particle.position += particle.velocity * (time - particle.last_update);
                particle.last_update = time;
            }
        }
        NifModifierKind::GrowFade { grow_time, grow_generation, fade_time, fade_generation } => {
            // 0x0081fe80.
            if !ctx.desc.has_sizes {
                return;
            }
            for particle in &mut pool.active {
                let mut grow = 1.0;
                if particle.generation == *grow_generation && particle.age < *grow_time && *grow_time != 0.0 {
                    grow = particle.age / grow_time;
                }
                let remaining = particle.life_span - particle.age;
                let mut fade = 1.0;
                if particle.generation == *fade_generation && remaining < *fade_time && *fade_time != 0.0 {
                    fade = remaining / fade_time;
                }
                particle.size = grow.min(fade).max(MIN_SCALE);
            }
        }
        NifModifierKind::Color { colors } => {
            // 0x00817270: the colour keys at age / life span.
            let (Some(curve), true) = (colors, ctx.desc.has_colors) else {
                return;
            };
            for particle in &mut pool.active {
                if let Some(color) = curve.eval(particle.age / particle.life_span) {
                    particle.color = color;
                }
            }
        }
        NifModifierKind::Rotation { .. } => {
            // 0x0080a460.
            if !ctx.desc.has_rotation_angles {
                return;
            }
            for particle in &mut pool.active {
                let mut angle = (time - particle.last_update) * particle.rotation_speed + particle.rotation_angle;
                if angle <= ROTATION_RESET_ANGLE {
                    while TWO_PI < angle {
                        angle -= TWO_PI;
                    }
                } else {
                    angle = 0.0;
                }
                particle.rotation_angle = angle;
            }
        }
        NifModifierKind::Gravity { object, axis, decay, strength, spherical, turbulence, turbulence_scale } => {
            let Some(object) = object else {
                return;
            };
            gravity(pool, ctx, *object, Vec3::from_array(*axis), *decay, *strength, *spherical, *turbulence, *turbulence_scale, time, rng);
        }
        NifModifierKind::Drag { object, axis, percentage, range, falloff } => {
            // 0x00819540.
            let Some(object) = object else {
                return;
            };
            if !(*percentage > 0.0) {
                return;
            }
            let transform = ctx.relative(Some(*object));
            let axis = transform.rotation.mul_vec3(Vec3::from_array(*axis)).normalize_or_zero();
            let axis_length_squared = axis.dot(axis);
            if axis_length_squared == 0.0 {
                return;
            }
            for particle in &mut pool.active {
                let distance = (particle.position - transform.translation).length();
                let scaled_dt = (time - particle.last_update) * DRAG_TIME_SCALE;
                let percent = if distance <= *range {
                    *percentage
                } else if distance < *falloff {
                    (1.0 - (distance - range) / (falloff - range)) * percentage
                } else {
                    continue;
                };
                let mut amount = particle.velocity.dot(axis) / axis_length_squared;
                if scaled_dt * percent <= 1.0 {
                    amount *= scaled_dt * percent;
                }
                particle.velocity -= axis * amount;
            }
        }
        NifModifierKind::Bomb { object, axis, decay, delta_v, decay_type, symmetry } => {
            bomb(pool, ctx, *object, Vec3::from_array(*axis), *decay, *delta_v, *decay_type, *symmetry, time);
        }
        NifModifierKind::Emitter(_)
        | NifModifierKind::Spawn(_)
        | NifModifierKind::BoundUpdate
        | NifModifierKind::MeshUpdate { .. }
        | NifModifierKind::Unsupported { .. } => {}
    }
}

/// `NiPSysAgeDeathModifier::Update` (0x008144a0), last particle first: `age += time - last_update` (the age of a
/// new particle is thus counted twice once, as in the client); a particle older than its life span dies,
/// spawning first when the modifier spawns on death.
fn age_death(
    spawn: Option<(usize, usize)>,
    modifiers: &[ModifierState],
    pool: &mut ParticlePool,
    ctx: &SystemContext,
    time: f32,
    rng: &mut impl CrtRng,
    deferred: &mut Vec<DeferredSpawn>,
) {
    let mut index = pool.active.len();
    while index > 0 {
        index -= 1;
        let particle = &mut pool.active[index];
        particle.age += time - particle.last_update;
        let particle = *particle;
        match spawn {
            None => {
                if particle.life_span <= particle.age && particle.age != particle.life_span {
                    pool.active.swap_remove(index);
                }
            }
            Some((target_system, modifier)) => {
                if particle.life_span < particle.age {
                    let spawn_time = particle.life_span + time - particle.age;
                    if target_system == ctx.index {
                        if let Some(NifModifierKind::Spawn(params)) = modifiers.get(modifier).map(|m| &m.kind) {
                            spawn_particles(params, &particle, time, spawn_time, pool, modifiers, ctx.desc, rng);
                        }
                    } else {
                        deferred.push(DeferredSpawn {
                            source_system: ctx.index,
                            target_system,
                            modifier,
                            time,
                            spawn_time,
                            particle,
                        });
                    }
                    pool.active.swap_remove(index);
                }
            }
        }
    }
}

/// `NiPSysGravityModifier::Update` (0x0081de40). Turbulence draws three symmetric randoms per particle.
fn gravity(
    pool: &mut ParticlePool,
    ctx: &SystemContext,
    object: usize,
    axis: Vec3,
    decay: f32,
    strength: f32,
    spherical: bool,
    turbulence: f32,
    turbulence_scale: f32,
    time: f32,
    rng: &mut impl CrtRng,
) {
    if pool.active.is_empty() {
        return;
    }
    let transform = ctx.relative(Some(object));
    let axis = transform.rotation.mul_vec3(axis).normalize_or_zero();
    let center = transform.translation;
    let force = strength * GRAVITY_FORCE_SCALE;
    let turbulence_force = turbulence_scale * turbulence * TURBULENCE_SCALE;
    let has_decay = decay != 0.0;
    let has_turbulence = turbulence != 0.0;
    for particle in &mut pool.active {
        let dt = time - particle.last_update;
        let (direction, distance) = if spherical {
            let offset = center - particle.position;
            (offset.normalize_or_zero(), offset.length())
        } else {
            (axis, (center - particle.position).dot(axis).abs())
        };
        let random = if has_turbulence {
            let x = symmetric_random(rng);
            let y = symmetric_random(rng);
            let z = symmetric_random(rng);
            Vec3::new(x, y, z) * turbulence_force
        } else {
            Vec3::ZERO
        };
        let attenuation = if has_decay { (-decay * distance).exp() } else { 1.0 };
        particle.velocity += (direction * (force * attenuation) + random) * dt;
    }
}

/// `NiPSysBombModifier::Update` (0x00814ad0), for particles updated before `time`. A particle at the bomb's
/// centre is not pushed (the client divides by zero).
fn bomb(
    pool: &mut ParticlePool,
    ctx: &SystemContext,
    object: Option<usize>,
    axis: Vec3,
    decay: f32,
    delta_v: f32,
    decay_type: BombDecay,
    symmetry: BombSymmetry,
    time: f32,
) {
    let (center, axis) = match object {
        Some(object) => {
            let transform = ctx.relative(Some(object));
            (transform.translation, transform.rotation.mul_vec3(axis).normalize_or_zero())
        }
        None => (Vec3::ZERO, axis),
    };
    for particle in &mut pool.active {
        if !(particle.last_update < time) {
            continue;
        }
        let offset = particle.position - center;
        let distance = offset.length();
        if decay_type != BombDecay::None && !(distance <= decay) {
            continue;
        }
        let (direction, distance) = match symmetry {
            BombSymmetry::Spherical => (offset.normalize_or_zero(), distance),
            BombSymmetry::Cylindrical => ((offset - axis * axis.dot(offset)).normalize_or_zero(), distance),
            BombSymmetry::Planar => {
                let along = axis.dot(offset);
                if along < 0.0 { (-axis, -along) } else { (axis, along) }
            }
        };
        let factor = match decay_type {
            BombDecay::None => 1.0,
            BombDecay::Linear => (decay - distance) / decay,
            BombDecay::Exponential => (-distance / decay).exp(),
        };
        particle.velocity += direction * (delta_v * factor * (time - particle.last_update));
    }
}

/// `NiPSysEmitterCtlr` state: the last scaled time (`-FLT_MAX` sentinel as `None`) and emitter activity.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EmitterClock {
    last_scaled: Option<f32>,
    last_active: bool,
}

/// One `EmitParticles` call of the emitter controller: the particle system time and the key interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Emission {
    pub time: f32,
    pub current: f32,
    pub last: f32,
    pub start: f32,
    pub stop: f32,
    pub birth_rate: f32,
}

impl Emission {
    /// The ages of the particles born in the interval (`NiPSysEmitterCtlr` emit helper, 0x0081a630): the counts
    /// are converted through `u16` as in the client.
    pub fn ages(&self, out: &mut Vec<f32>) {
        out.clear();
        let Self { current, last, start, stop, birth_rate, .. } = *self;
        if !(birth_rate > 0.0 && start < stop && current > start && last < stop) {
            return;
        }
        let current_delta = if current <= stop { current - start } else { stop - start };
        let last_delta = if last >= start { last - start } else { 0.0 };
        let current_count = (birth_rate * current_delta) as i32 as u16;
        let mut last_count = (birth_rate * last_delta) as i32 as u16;
        let count = current_count.wrapping_sub(last_count);
        let step = 1.0 / birth_rate;
        for _ in 0..count {
            last_count = last_count.wrapping_add(1);
            out.push(current_delta - f32::from(last_count) * step);
        }
    }
}

/// `NiPSysEmitterCtlr::UpdateParticleSystem` (SDK `NiPSEmitParticlesCtlr::UpdateParticleSystem`, client
/// 0x00819ef0): walks the emitter-active keys from the last scaled time to `scaled` and emits for every active
/// interval at the birth rate of its midpoint. `system_last_time` is the particle system's last update time.
pub(crate) fn emitter_controller_emissions(
    clock: &mut EmitterClock,
    birth_rate: Option<&FloatInterp>,
    active: Option<&BoolInterp>,
    timing: &ControllerTiming,
    scaled: f32,
    system_last_time: Option<f32>,
    out: &mut Vec<Emission>,
) {
    let (Some(rate), Some(active)) = (birth_rate, active) else {
        return;
    };
    let keys = &active.keys;
    if keys.is_empty() && !active.is_posed() {
        return;
    }
    // The client reads the sentinel as a time; a system that was never updated starts at the scaled time here.
    let system_last = system_last_time.unwrap_or(scaled);
    let rate_at = |time: f32| rate.eval(time).unwrap_or(0.0);
    if active.is_posed() {
        let mut last = *clock.last_scaled.get_or_insert(scaled);
        if !active.pose.unwrap_or(false) {
            return;
        }
        clock.last_scaled = Some(scaled);
        if rate.is_posed() {
            let mut delta = scaled - last;
            if last > scaled {
                last = 0.0;
                delta = scaled;
            }
            out.push(Emission {
                time: system_last + delta,
                current: scaled,
                last,
                start: 0.0,
                stop: scaled,
                birth_rate: rate_at(0.0),
            });
            return;
        }
        let Some((begin, end)) = rate.keys.as_ref().and_then(|curve| curve.time_range()) else {
            return;
        };
        let mut loop_delta = 0.0;
        if last > scaled {
            loop_delta = end - last;
            out.push(Emission {
                time: system_last + loop_delta,
                current: end,
                last,
                start: begin,
                stop: end,
                birth_rate: rate_at(last + (end - last) * BIRTH_RATE_MIDPOINT),
            });
            last = begin;
        }
        out.push(Emission {
            time: system_last + (scaled - last) + loop_delta,
            current: scaled,
            last,
            start: begin,
            stop: scaled,
            birth_rate: rate_at(last + (scaled - last) * BIRTH_RATE_MIDPOINT),
        });
        return;
    }
    let current_active = eval_bool_keys(keys, scaled).unwrap_or(false);
    let Some(mut last) = clock.last_scaled else {
        clock.last_scaled = Some(scaled);
        clock.last_active = current_active;
        return;
    };
    let mut last_active = clock.last_active;
    clock.last_scaled = Some(scaled);
    clock.last_active = current_active;
    let count = keys.len();
    for _ in 0..MAX_EMITTER_KEY_PASSES {
        let looped = last > scaled;
        let start = if last_active {
            (0..count).rev().find(|&k| keys[k].time <= last && !keys[k].value).map_or(0, |k| k + 1)
        } else {
            let after = (0..count)
                .find(|&k| keys[k].time > last && (looped || keys[k].time <= scaled) && keys[k].value);
            match after {
                Some(k) => k,
                None if looped => match (0..count).find(|&k| keys[k].time < scaled && keys[k].value) {
                    Some(k) => k,
                    None => return,
                },
                None => return,
            }
        };
        if start >= count {
            return;
        }
        let stop = (start..count).find(|&k| !keys[k].value).unwrap_or(count - 1);
        let (start_time, stop_time) = (keys[start].time, keys[stop].time);
        let midpoint = start_time + (stop_time - start_time) * BIRTH_RATE_MIDPOINT;
        let (current, interval_last, emit_stop, birth_rate) = if looped {
            if stop_time > last {
                (keys[count - 1].time, last, stop_time, rate_at(midpoint))
            } else {
                let emit_stop = if stop_time < scaled { stop_time } else { scaled };
                let midpoint = start_time + (emit_stop - start_time) * BIRTH_RATE_MIDPOINT;
                (scaled, keys[0].time, emit_stop, rate_at(midpoint))
            }
        } else if stop_time < scaled {
            (stop_time, last, stop_time, rate_at(midpoint))
        } else {
            (scaled, last, stop_time, rate_at(scaled))
        };
        let mut base = system_last;
        if looped {
            base += timing.stop_time - timing.start_time;
        }
        out.push(Emission {
            time: base + scaled - last,
            current,
            last: interval_last,
            start: start_time,
            stop: emit_stop,
            birth_rate,
        });
        if stop_time < scaled {
            last = current;
            last_active = eval_bool_keys(keys, last).unwrap_or(false);
        } else {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nif::desc::{Curve, Key, KeyKind, NifModifier, NifRenderState, Tcb};
    use crate::rng::MsvcRand;

    const EPSILON: f32 = 1e-4;
    const SAMPLES: usize = 2000;
    const SEED: u32 = 7;

    fn node(translation: Vec3) -> NodeState {
        let xf = Xf { translation, ..Xf::IDENTITY };
        NodeState { local: xf, world: xf, flags: 0, visible: true }
    }

    fn system(modifiers: Vec<NifModifierKind>) -> NifParticleSystem {
        NifParticleSystem {
            node: 0,
            world_space: false,
            capacity: u16::MAX,
            has_colors: true,
            has_radii: true,
            has_sizes: true,
            has_rotation_angles: true,
            has_rotation_axes: true,
            modifiers: modifiers
                .into_iter()
                .map(|kind| NifModifier { name: String::new(), order: 0, active: true, kind })
                .collect(),
            render_state: NifRenderState::default(),
            mesh_particles: false,
        }
    }

    fn emitter(shape: EmitterShape) -> NifEmitter {
        NifEmitter {
            speed: 10.0,
            speed_var: 4.0,
            declination: 0.5,
            declination_var: 0.1,
            planar_angle: 1.0,
            planar_angle_var: 0.25,
            initial_color: [0.1, 0.2, 0.3, 0.4],
            initial_radius: 2.0,
            radius_var: 0.5,
            life_span: 3.0,
            life_span_var: 1.0,
            shape,
        }
    }

    struct Fixture {
        desc: NifParticleSystem,
        nodes: Vec<NodeState>,
    }

    impl Fixture {
        fn new(modifiers: Vec<NifModifierKind>) -> Self {
            Self { desc: system(modifiers), nodes: vec![node(Vec3::ZERO), node(Vec3::new(10.0, 0.0, 0.0))] }
        }

        fn ctx(&self) -> SystemContext<'_> {
            SystemContext { index: 0, desc: &self.desc, nodes: &self.nodes, meshes: &[], mesh_positions: &[] }
        }

        /// Emits `SAMPLES` particles of age 0 at time 0 and returns them.
        fn emit(&self, seed: u32) -> Vec<NifParticle> {
            let mut state = SystemState::new(&self.desc);
            let mut rng = MsvcRand::new(seed);
            state.emit(&self.ctx(), 0, 0.0, &[0.0; SAMPLES], &mut rng);
            state.pool.added
        }
    }

    #[test]
    fn emitted_particles_follow_the_emitter_ranges() {
        let fixture = Fixture::new(vec![NifModifierKind::Emitter(emitter(EmitterShape::Box {
            object: None,
            width: 2.0,
            height: 4.0,
            depth: 6.0,
        }))]);
        let particles = fixture.emit(SEED);
        assert_eq!(particles.len(), SAMPLES);
        for p in &particles {
            let speed = p.velocity.length();
            assert!((8.0 - EPSILON..=12.0 + EPSILON).contains(&speed), "speed {speed}");
            let declination = (p.velocity.z / speed).acos();
            assert!((0.4 - EPSILON..=0.6 + EPSILON).contains(&declination), "declination {declination}");
            let planar = p.velocity.y.atan2(p.velocity.x);
            assert!((0.75 - EPSILON..=1.25 + EPSILON).contains(&planar), "planar {planar}");
            assert!((2.0..=4.0).contains(&p.life_span));
            assert!((1.5 - EPSILON..=2.5 + EPSILON).contains(&p.radius));
            assert!(p.position.x.abs() <= 1.0 && p.position.y.abs() <= 2.0 && p.position.z.abs() <= 3.0);
            assert_eq!(p.color, [0.1, 0.2, 0.3, 0.4]);
            assert_eq!(p.size, EMITTED_SIZE);
        }
        // The samples cover the box.
        assert!(particles.iter().any(|p| p.position.z > 2.5) && particles.iter().any(|p| p.position.z < -2.5));
    }

    #[test]
    fn particles_older_than_their_life_span_are_not_emitted() {
        let fixture = Fixture::new(vec![NifModifierKind::Emitter(emitter(EmitterShape::Sphere { object: None, radius: 1.0 }))]);
        let mut state = SystemState::new(&fixture.desc);
        // Life spans are 2..4 s: none survive an age of 5 s.
        state.emit(&fixture.ctx(), 0, 0.0, &[5.0; SAMPLES], &mut MsvcRand::new(SEED));
        assert!(state.pool.added.is_empty());
    }

    #[test]
    fn volume_emitters_stay_inside_their_volume() {
        let sphere = Fixture::new(vec![NifModifierKind::Emitter(emitter(EmitterShape::Sphere { object: Some(1), radius: 3.0 }))]);
        for p in sphere.emit(SEED) {
            assert!((p.position - Vec3::new(10.0, 0.0, 0.0)).length() <= 3.0 + EPSILON);
        }
        let cylinder =
            Fixture::new(vec![NifModifierKind::Emitter(emitter(EmitterShape::Cylinder { object: None, radius: 2.0, height: 5.0 }))]);
        for p in cylinder.emit(SEED) {
            assert!(p.position.x.hypot(p.position.y) <= 2.0 + EPSILON);
            assert!(p.position.z.abs() <= 2.5 + EPSILON);
        }
    }

    #[test]
    fn mesh_emitters_emit_from_vertices_and_faces() {
        let geometry = crate::render::MeshGeometry {
            positions: vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            triangles: vec![[0, 1, 2]],
            ..Default::default()
        };
        let mesh = NifMesh { node: 1, geometry, morph: None, render_state: NifRenderState::default() };
        let meshes = [mesh];
        for emission in [MeshEmission::Vertices, MeshEmission::FaceCenter, MeshEmission::FaceSurface, MeshEmission::EdgeSurface] {
            let fixture = Fixture::new(vec![NifModifierKind::Emitter(emitter(EmitterShape::Mesh {
                meshes: vec![0],
                velocity: MeshEmitterVelocity::Normals,
                emission,
                emit_axis: [0.0, 0.0, 1.0],
            }))]);
            let ctx = SystemContext { meshes: &meshes, ..fixture.ctx() };
            let mut state = SystemState::new(&fixture.desc);
            state.emit(&ctx, 0, 0.0, &[0.0; SAMPLES], &mut MsvcRand::new(SEED));
            for p in &state.pool.added {
                // In the mesh node's space (offset by +10 x): inside the triangle, velocity along the normal.
                let local = p.position - Vec3::new(10.0, 0.0, 0.0);
                assert!(local.x >= -EPSILON && local.y >= -EPSILON && local.x + local.y <= 4.0 + EPSILON, "{emission:?} {local:?}");
                assert!(local.z.abs() < EPSILON);
                assert!(p.velocity.x.abs() < EPSILON && p.velocity.y.abs() < EPSILON && p.velocity.z > 0.0);
                if emission == MeshEmission::FaceCenter {
                    assert!((local - Vec3::new(4.0 / 3.0, 4.0 / 3.0, 0.0)).length() < EPSILON);
                }
            }
        }
    }

    fn pool_with(particle: NifParticle) -> ParticlePool {
        ParticlePool { active: vec![particle], added: Vec::new(), capacity: usize::from(u16::MAX) }
    }

    fn run(kind: NifModifierKind, particle: NifParticle, time: f32) -> ParticlePool {
        let fixture = Fixture::new(vec![kind.clone()]);
        let mut pool = pool_with(particle);
        let modifiers = vec![ModifierState { active: true, kind: kind.clone() }];
        update_modifier(&kind, &modifiers, &mut pool, &fixture.ctx(), time, &mut MsvcRand::new(SEED), &mut Vec::new());
        pool
    }

    #[test]
    fn grow_fade_scales_young_and_dying_particles() {
        let kind = NifModifierKind::GrowFade { grow_time: 1.0, grow_generation: 0, fade_time: 2.0, fade_generation: 0 };
        let young = NifParticle { age: 0.25, life_span: 10.0, ..Default::default() };
        assert!((run(kind.clone(), young, 0.0).active[0].size - 0.25).abs() < EPSILON);
        let dying = NifParticle { age: 9.5, life_span: 10.0, ..Default::default() };
        assert!((run(kind.clone(), dying, 0.0).active[0].size - 0.25).abs() < EPSILON);
        let other_generation = NifParticle { age: 0.25, life_span: 10.0, generation: 1, ..Default::default() };
        assert_eq!(run(kind.clone(), other_generation, 0.0).active[0].size, 1.0);
        let newborn = NifParticle { age: 0.0, life_span: 10.0, ..Default::default() };
        assert_eq!(run(kind, newborn, 0.0).active[0].size, MIN_SCALE);
    }

    #[test]
    fn color_keys_follow_the_age_fraction() {
        let key = |time, value| Key { time, value, in_tangent: value, out_tangent: value, tcb: Tcb::default() };
        let colors = Curve { kind: KeyKind::Linear, keys: vec![key(0.0, [1.0, 0.0, 0.0, 1.0]), key(1.0, [0.0, 0.0, 1.0, 0.0])] };
        let particle = NifParticle { age: 1.0, life_span: 4.0, ..Default::default() };
        let color = run(NifModifierKind::Color { colors: Some(colors) }, particle, 0.0).active[0].color;
        for (actual, expected) in color.iter().zip([0.75, 0.0, 0.25, 0.75]) {
            assert!((actual - expected).abs() < EPSILON, "{color:?}");
        }
    }

    fn rotation() -> NifModifierKind {
        NifModifierKind::Rotation {
            initial_speed: 1.0,
            initial_speed_var: 0.5,
            initial_angle: 0.0,
            initial_angle_var: 0.0,
            random_speed_sign: true,
            random_initial_axis: true,
            initial_axis: [0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn rotation_advances_wraps_and_resets() {
        let spin = NifParticle { rotation_angle: 6.0, rotation_speed: 1.0, ..Default::default() };
        assert!((run(rotation(), spin, 0.5).active[0].rotation_angle - (6.5 - TWO_PI)).abs() < EPSILON);
        let runaway = NifParticle { rotation_angle: 30.0, rotation_speed: 4.0, ..Default::default() };
        assert_eq!(run(rotation(), runaway, 1.0).active[0].rotation_angle, 0.0);
    }

    #[test]
    fn rotation_initializes_speed_sign_and_random_axes() {
        let fixture = Fixture::new(vec![NifModifierKind::Emitter(emitter(EmitterShape::Sphere { object: None, radius: 1.0 })), rotation()]);
        let particles = fixture.emit(SEED);
        assert!(particles.iter().any(|p| p.rotation_speed < 0.0) && particles.iter().any(|p| p.rotation_speed > 0.0));
        for p in &particles {
            assert!((0.5 - EPSILON..=1.5 + EPSILON).contains(&p.rotation_speed.abs()));
            assert!((p.rotation_axis.length() - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn gravity_drag_and_bomb_change_velocities() {
        let still = NifParticle { last_update: 0.0, ..Default::default() };
        let gravity = NifModifierKind::Gravity {
            object: Some(0),
            axis: [0.0, 0.0, -1.0],
            decay: 0.0,
            strength: 10.0,
            spherical: false,
            turbulence: 0.0,
            turbulence_scale: 1.0,
        };
        let velocity = run(gravity, still, 0.5).active[0].velocity;
        assert!((velocity - Vec3::new(0.0, 0.0, -10.0 * GRAVITY_FORCE_SCALE * 0.5)).length() < EPSILON);

        let moving = NifParticle { velocity: Vec3::new(0.0, 0.0, 10.0), ..Default::default() };
        let drag = NifModifierKind::Drag { object: Some(0), axis: [0.0, 0.0, 1.0], percentage: 1.0, range: 100.0, falloff: 200.0 };
        let dt = 0.01;
        let velocity = run(drag, moving, dt).active[0].velocity;
        assert!((velocity.z - 10.0 * (1.0 - dt * DRAG_TIME_SCALE)).abs() < 1e-3, "{velocity:?}");

        let off_centre = NifParticle { position: Vec3::new(2.0, 0.0, 0.0), ..Default::default() };
        let bomb = NifModifierKind::Bomb {
            object: Some(0),
            axis: [0.0, 0.0, 1.0],
            decay: 4.0,
            delta_v: 3.0,
            decay_type: BombDecay::Linear,
            symmetry: BombSymmetry::Spherical,
        };
        let velocity = run(bomb, off_centre, 1.0).active[0].velocity;
        assert!((velocity - Vec3::new(3.0 * 0.5, 0.0, 0.0)).length() < EPSILON, "{velocity:?}");
    }

    fn spawn() -> NifSpawn {
        NifSpawn {
            num_spawn_generations: 1,
            percentage_spawned: 1.0,
            min_num_to_spawn: 3,
            max_num_to_spawn: 3,
            spawn_speed_chaos: 0.0,
            spawn_dir_chaos: 0.0,
            life_span: 2.0,
            life_span_var: 0.0,
        }
    }

    #[test]
    fn age_death_kills_and_spawns() {
        let old = NifParticle { age: 0.9, life_span: 1.0, last_update: 0.0, ..Default::default() };
        let plain = NifModifierKind::AgeDeath { spawn_on_death: false, spawn_modifier: None };
        assert!(run(plain.clone(), old, 0.2).active.is_empty());
        assert_eq!(run(plain, old, 0.05).active[0].age, 0.95);

        let fixture = Fixture::new(vec![]);
        let kinds = [NifModifierKind::AgeDeath { spawn_on_death: true, spawn_modifier: Some((0, 1)) }, NifModifierKind::Spawn(spawn())];
        let modifiers: Vec<ModifierState> = kinds.iter().map(|kind| ModifierState { active: true, kind: kind.clone() }).collect();
        let moving = NifParticle { velocity: Vec3::new(0.0, 5.0, 0.0), ..old };
        let mut pool = pool_with(moving);
        update_modifier(&kinds[0], &modifiers, &mut pool, &fixture.ctx(), 0.2, &mut MsvcRand::new(SEED), &mut Vec::new());
        assert!(pool.active.is_empty());
        assert_eq!(pool.added.len(), 3);
        for child in &pool.added {
            assert_eq!(child.generation, 1);
            assert_eq!(child.life_span, 2.0);
            // Born when the parent died (0.1 s before the update): no chaos keeps the parent's velocity.
            assert!((child.age - 0.1).abs() < EPSILON);
            assert!((child.velocity - moving.velocity).length() < EPSILON, "{:?}", child.velocity);
        }
        // The last generation does not spawn again.
        let mut pool = pool_with(NifParticle { generation: 1, ..moving });
        update_modifier(&kinds[0], &modifiers, &mut pool, &fixture.ctx(), 0.2, &mut MsvcRand::new(SEED), &mut Vec::new());
        assert!(pool.active.is_empty() && pool.added.is_empty());
    }

    #[test]
    fn spawn_alignment_turns_z_onto_the_velocity() {
        for velocity in [Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 0.0, -4.0), Vec3::new(-1.0, 0.0, 0.0)] {
            let turned = align_z_to(velocity).mul_vec3(Vec3::Z);
            assert!((turned - velocity.normalize_or_zero()).length() < EPSILON, "{velocity:?} -> {turned:?}");
        }
    }

    #[test]
    fn emission_ages_space_particles_by_the_birth_rate() {
        let emission = Emission { time: 0.0, current: 1.0, last: 0.5, start: 0.0, stop: 2.0, birth_rate: 10.0 };
        let mut ages = Vec::new();
        emission.ages(&mut ages);
        let expected = [0.4, 0.3, 0.2, 0.1, 0.0];
        assert_eq!(ages.len(), expected.len());
        for (age, expected) in ages.iter().zip(expected) {
            assert!((age - expected).abs() < EPSILON, "{ages:?}");
        }
    }

    #[test]
    fn emitter_controller_walks_the_active_keys() {
        let timing = ControllerTiming {
            cycle: crate::nif::desc::CycleType::Clamp,
            active: true,
            app_init: false,
            reverse_direction: false,
            frequency: 1.0,
            phase: 0.0,
            start_time: 0.0,
            stop_time: 2.0,
        };
        let rate = FloatInterp { pose: Some(10.0), keys: None };
        let active = BoolInterp {
            pose: None,
            keys: vec![crate::nif::desc::BoolKey { time: 0.0, value: true }, crate::nif::desc::BoolKey { time: 1.0, value: false }],
        };
        let mut clock = EmitterClock::default();
        let mut out = Vec::new();
        emitter_controller_emissions(&mut clock, Some(&rate), Some(&active), &timing, 0.0, None, &mut out);
        assert!(out.is_empty(), "the first update only records the time");
        emitter_controller_emissions(&mut clock, Some(&rate), Some(&active), &timing, 0.5, Some(0.0), &mut out);
        assert_eq!(out, [Emission { time: 0.5, current: 0.5, last: 0.0, start: 0.0, stop: 1.0, birth_rate: 10.0 }]);
        out.clear();
        emitter_controller_emissions(&mut clock, Some(&rate), Some(&active), &timing, 1.5, Some(0.5), &mut out);
        assert_eq!(out, [Emission { time: 1.5, current: 1.0, last: 0.5, start: 0.0, stop: 1.0, birth_rate: 10.0 }]);
    }

    #[test]
    fn emission_is_deterministic_for_a_seed() {
        let fixture = Fixture::new(vec![
            NifModifierKind::Emitter(emitter(EmitterShape::Sphere { object: None, radius: 1.0 })),
            rotation(),
        ]);
        assert_eq!(fixture.emit(SEED), fixture.emit(SEED));
        assert_ne!(fixture.emit(SEED), fixture.emit(SEED + 1));
    }
}
