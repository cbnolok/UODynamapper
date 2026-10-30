//! `UOParticleEffectNif`: a NIF effect as the client runs it, and its render output.

use std::sync::Arc;

use super::desc::{
    AV_FLAG_APP_CULLED, CullMode, NifEffectDesc, NifMaterial, NifModifierKind, NifNodeKind, NifRenderState, NifTextureTransform, TextureApplyMode,
    TextureSlot, TextureTransformMethod,
};
use super::particles::NifParticle;
use super::scene::{SceneOrder, SceneState, Xf, make_rotation};
use crate::math::{Mat3, Vec3};
use crate::render::{
    BatchState, BlendFactor, BlendState, CameraFrame, DepthState, DrawBatch, MaterialColors, MeshDraw,
    MeshGeometry, RenderFrame, SceneLighting, SpriteInstance, SpriteKind, TextureStage, UvRect, UvTransform,
    light_vertex,
};
use crate::rng::{CrtRng, MsvcRand};
use crate::system::{NodeTransform, ParticleSystemInstance, SECONDS_PER_MS};

/// A particle's quad side relative to its radius (`NiPSFacingQuadGeneratorKernel`: half extent `radius * size`).
const SPRITE_SIDE_PER_RADIUS: f32 = 2.0;
/// Blending off: the source replaces the destination.
const OPAQUE_BLEND: BlendState = BlendState { src: BlendFactor::One, dst: BlendFactor::Zero };
/// Camera pitch of [`default_camera`]: 45 degrees down, looking north (+Y) with Z up.
const DEFAULT_CAMERA_PITCH: f32 = std::f32::consts::FRAC_PI_4;
/// Distance of [`default_camera`] from the origin.
const DEFAULT_CAMERA_DISTANCE: f32 = 1000.0;

/// A camera looking north (+Y) and 45 degrees down (Z up), right along +X: the billboard camera of an instance
/// until [`NifEffectInstance::set_camera`].
pub fn default_camera() -> CameraFrame {
    let (sin, cos) = DEFAULT_CAMERA_PITCH.sin_cos();
    let direction = Vec3::new(0.0, cos, -sin);
    CameraFrame { position: direction * -DEFAULT_CAMERA_DISTANCE, direction, up: Vec3::new(0.0, sin, cos), right: Vec3::X }
}

/// A [`NifEffectDesc`] prepared for running: shared geometry and names, and the scene update order.
#[derive(Debug, Clone)]
pub struct NifEffect {
    desc: NifEffectDesc,
    geometries: Vec<Arc<MeshGeometry>>,
    mesh_names: Vec<Arc<str>>,
    texture_names: Vec<Arc<str>>,
    order: SceneOrder,
    /// Per node: the depth-first draw position.
    draw_order: Vec<u32>,
}

impl NifEffect {
    pub fn prepare(desc: NifEffectDesc) -> Self {
        let order = SceneOrder::new(&desc);
        let mut draw_order = vec![u32::MAX; desc.nodes.len()];
        for (position, &node) in order.nodes.iter().enumerate() {
            draw_order[node] = position as u32;
        }
        Self {
            geometries: desc.meshes.iter().map(|mesh| Arc::new(mesh.geometry.clone())).collect(),
            mesh_names: desc.meshes.iter().map(|mesh| Arc::from(desc.nodes[mesh.node].name.as_str())).collect(),
            texture_names: desc.textures.iter().map(|texture| Arc::from(texture.name())).collect(),
            order,
            draw_order,
            desc,
        }
    }

    pub fn desc(&self) -> &NifEffectDesc {
        &self.desc
    }
}

/// The mesh prototype nodes of particle system `system` (its `NiPSysMeshUpdateModifier`), by generation.
fn mesh_prototypes(desc: &NifEffectDesc, system: usize) -> &[usize] {
    desc.particle_systems
        .get(system)
        .and_then(|system| {
            system.modifiers.iter().find_map(|modifier| match &modifier.kind {
                NifModifierKind::MeshUpdate { meshes } => Some(meshes.as_slice()),
                _ => None,
            })
        })
        .unwrap_or(&[])
}

/// `NiTextureTransform::UpdateMatrix` as a [`UvTransform`] (`u' = col0.x u + col1.x v + col2.x`).
pub fn texture_transform_matrix(transform: &NifTextureTransform) -> UvTransform {
    let (sin, cos) = transform.rotate.sin_cos();
    let [tx, ty] = transform.translate;
    let [sx, sy] = transform.scale;
    let [cx, cy] = transform.center;
    let (col0, col1, col2) = match transform.method {
        TextureTransformMethod::Max => (
            [sx * cos, sy * -sin],
            [sx * sin, sy * cos],
            [
                cx + sx * (cos * (-cx - tx) + sin * (-cy + ty)),
                cy + sy * (-sin * (-cx - tx) + cos * (-cy + ty)),
            ],
        ),
        TextureTransformMethod::Maya => (
            [cos * sx, -sin * sx],
            [-sin * sy, -cos * sy],
            [
                (tx - cx) * cos + (-ty - cy + 1.0) * sin + cx,
                (-tx + cx) * sin + (-ty - cy + 1.0) * cos + cy,
            ],
        ),
        TextureTransformMethod::MayaDeprecated => (
            [cos * sx, sin * sx],
            [-sin * sy, cos * sy],
            [(tx - cx) * cos + (ty - cy) * -sin + cx, (tx - cx) * sin + (ty - cy) * cos + cy],
        ),
    };
    UvTransform { rows: [[col0[0], col1[0], col2[0]], [col0[1], col1[1], col2[1]]] }
}

/// A running NIF effect (`UOParticleEffectNif`, UOSA.exe 0x0041a8e0..0x0041ac40).
///
/// - Creation is `Init`: the scene is updated once at time 0.
/// - [`ParticleSystemInstance::start`] stores the definition's duration and loop flag and starts the clock at 0.
/// - Each update adds `dt` to the clock, counts the lifetime ticks down, and stops a non-looping effect once
///   `duration <= elapsed` (the effect is then finished and not updated again, as in the client: a non-looping
///   effect with duration 0 stops at its first update); a looping effect with a duration wraps the clock
///   (`fmod`). The scene is then updated at the clock time.
/// - The node transform replaces the root's local transform (the client sets the root rotation, `fabs(scale)`
///   and position each frame).
///
/// Simulation is deterministic for a seed: the client draws from the C runtime `rand()` shared by the whole
/// process; [`MsvcRand`] is that generator.
pub struct NifEffectInstance<R: CrtRng = MsvcRand> {
    effect: Arc<NifEffect>,
    rng: R,
    scene: SceneState,
    camera: CameraFrame,
    lighting: SceneLighting,
    running: bool,
    started: bool,
    looping: bool,
    duration: f32,
    elapsed: f32,
    lifetime_ticks: Option<u32>,
    /// Per particle system, per live particle of a mesh particle system: the state of its prototype clone.
    prototype_poses: Vec<Vec<SceneState>>,
}

impl NifEffectInstance<MsvcRand> {
    pub fn with_seed(effect: Arc<NifEffect>, seed: u32) -> Self {
        Self::new(effect, MsvcRand::new(seed))
    }
}

impl<R: CrtRng> NifEffectInstance<R> {
    pub fn new(effect: Arc<NifEffect>, rng: R) -> Self {
        let scene = SceneState::new(&effect.desc);
        let mut instance = Self {
            effect,
            rng,
            scene,
            camera: default_camera(),
            lighting: SceneLighting::default(),
            running: false,
            started: false,
            looping: false,
            duration: 0.0,
            elapsed: 0.0,
            lifetime_ticks: None,
            prototype_poses: Vec::new(),
        };
        instance.update_scene();
        instance
    }

    pub fn effect(&self) -> &Arc<NifEffect> {
        &self.effect
    }

    /// The camera billboards turn to. Set it before updating.
    pub fn set_camera(&mut self, camera: CameraFrame) {
        self.camera = camera;
    }

    pub fn camera(&self) -> &CameraFrame {
        &self.camera
    }

    /// The lights of [`MeshDraw`] and particle colours.
    pub fn set_lighting(&mut self, lighting: SceneLighting) {
        self.lighting = lighting;
    }

    /// The effect clock in seconds.
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    /// The live particles of particle system `system` (in its space).
    pub fn particles(&self, system: usize) -> &[NifParticle] {
        self.scene.systems.get(system).map_or(&[], |state| state.pool.active.as_slice())
    }

    /// `IsFinished` (0x0041ab70): started, and out of lifetime ticks or stopped.
    pub fn finished(&self) -> bool {
        self.started && (self.lifetime_ticks == Some(0) || !self.running)
    }

    fn update_scene(&mut self) {
        let effect = &self.effect;
        self.scene.update(&effect.desc, &effect.order, self.elapsed, &self.camera, &mut self.rng);
        self.prototype_poses.resize_with(effect.desc.particle_systems.len(), Vec::new);
        for (system, desc) in effect.desc.particle_systems.iter().enumerate() {
            let poses = &mut self.prototype_poses[system];
            poses.clear();
            let prototypes = mesh_prototypes(&effect.desc, system);
            let Some(last) = prototypes.len().checked_sub(1) else { continue };
            if !desc.mesh_particles {
                continue;
            }
            for particle in self.scene.systems.get(system).map_or(&[][..], |state| state.pool.active.as_slice()) {
                let prototype = prototypes[usize::from(particle.generation).min(last)];
                let controllers = &effect.order.prototype_controllers[prototype];
                poses.push(self.scene.prototype_pose(&effect.desc, controllers, particle.age, &mut self.rng));
            }
        }
    }

    /// Advances by `dt` seconds (`UOParticleEffectNif::Update`).
    pub fn update_seconds(&mut self, dt: f32, node: &NodeTransform) {
        if !self.running {
            return;
        }
        self.elapsed += dt;
        if let Some(ticks) = &mut self.lifetime_ticks {
            *ticks = ticks.saturating_sub(1);
        }
        if !self.looping && self.duration <= self.elapsed {
            self.running = false;
            return;
        }
        if self.finished() {
            return;
        }
        if self.duration != 0.0 && self.looping {
            self.elapsed %= self.duration;
        }
        if let Some(root) = self.effect.desc.root {
            let state = &mut self.scene.nodes[root];
            state.local = Xf { rotation: node.rotation, translation: node.position, scale: node.scale.abs() };
        }
        self.update_scene();
    }

    fn material_colors(scene: &SceneState, render: &NifRenderState) -> MaterialColors {
        let material = render.material.and_then(|index| scene.materials.get(index));
        let NifMaterial { ambient, diffuse, specular, emissive, alpha, .. } =
            material.cloned().unwrap_or_default();
        MaterialColors { ambient, diffuse, specular, emissive, alpha }
    }

    fn texture_stages(&self, scene: &SceneState, render: &NifRenderState) -> (TextureApplyMode, Vec<TextureStage>) {
        let Some((index, texturing)) =
            render.texturing.and_then(|index| Some((index, self.effect.desc.texturings.get(index)?)))
        else {
            return (TextureApplyMode::Modulate, Vec::new());
        };
        let stages = texturing
            .maps
            .iter()
            .enumerate()
            .filter_map(|(map_index, map)| {
                let texture = self.effect.texture_names.get(map.texture?)?.clone();
                let transform = scene.texture_transforms[index][map_index];
                Some(TextureStage {
                    slot: map.slot,
                    texture,
                    uv_set: map.uv_set,
                    uv_transform: transform.as_ref().map_or(UvTransform::IDENTITY, texture_transform_matrix),
                    addressing: map.addressing,
                })
            })
            .collect();
        (texturing.apply_mode, stages)
    }

    fn mesh_draw(&self, scene: &SceneState, mesh: usize, world: &Xf, order: u32) -> MeshDraw {
        let desc = &self.effect.desc.meshes[mesh];
        let render = &desc.render_state;
        let (apply_mode, textures) = self.texture_stages(scene, render);
        MeshDraw {
            order,
            name: self.effect.mesh_names[mesh].clone(),
            geometry: self.effect.geometries[mesh].clone(),
            positions: scene.morphed[mesh].clone(),
            rotation: world.rotation,
            translation: world.translation,
            scale: world.scale,
            material: Self::material_colors(scene, render),
            vertex_color: render.vertex_color,
            apply_mode,
            textures,
            blend: render.alpha.blend_state(),
            alpha_test: render.alpha.alpha_test(),
            depth: DepthState { test: render.z_buffer.test, write: render.z_buffer.write },
            depth_function: render.z_buffer.function,
            cull: render.cull,
        }
    }

    /// Mesh draws of a subtree in `scene` with `parent` as the world transform above `node`; `local` replaces
    /// the node's own local transform.
    fn draw_subtree(
        &self,
        scene: &SceneState,
        node: usize,
        parent: &Xf,
        local: Option<Xf>,
        order: u32,
        out: &mut RenderFrame,
    ) {
        let desc = &self.effect.desc;
        let state = &scene.nodes[node];
        if state.flags & AV_FLAG_APP_CULLED != 0 {
            return;
        }
        let world = parent.mul(&local.unwrap_or(state.local));
        if let NifNodeKind::Mesh { mesh } = desc.nodes[node].kind {
            out.mesh_draws.push(self.mesh_draw(scene, mesh, &world, order));
        }
        for &child in &desc.nodes[node].children {
            self.draw_subtree(scene, child, &world, None, order, out);
        }
    }

    fn render_particles(&self, system: usize, order: u32, out: &mut RenderFrame) {
        let desc = &self.effect.desc.particle_systems[system];
        let world = self.scene.nodes[desc.node].world;
        let particles = self.particles(system);
        if particles.is_empty() {
            return;
        }
        if desc.mesh_particles {
            self.render_mesh_particles(system, &world, order, out);
            return;
        }
        let render = &desc.render_state;
        let material = Self::material_colors(&self.scene, render);
        let (apply_mode, stages) = self.texture_stages(&self.scene, render);
        let base = stages.iter().find(|stage| stage.slot == TextureSlot::Base);
        let replaces = apply_mode == TextureApplyMode::Replace && base.is_some();
        let kind = if desc.has_rotation_angles { SpriteKind::RotatedBillboard } else { SpriteKind::Billboard };
        let start = out.sprites.len();
        for particle in particles {
            let color = if replaces {
                [1.0; 4]
            } else {
                let vertex = desc.has_colors.then_some(particle.color);
                light_vertex(&material, vertex, render.vertex_color, &self.lighting)
            };
            out.sprites.push(SpriteInstance {
                position: world.point(particle.position),
                size: SPRITE_SIDE_PER_RADIUS * particle.radius * particle.size * world.scale,
                rotation: if desc.has_rotation_angles { particle.rotation_angle } else { 0.0 },
                color,
                uv: UvRect::FULL,
                direction: Vec3::ZERO,
                length: 0.0,
            });
        }
        let batch = out.batches.len();
        out.batches.push(DrawBatch {
            texture: base.map_or_else(|| Arc::from(""), |stage| stage.texture.clone()),
            blend: render.alpha.blend_state().unwrap_or(OPAQUE_BLEND),
            depth: DepthState { test: render.z_buffer.test, write: render.z_buffer.write },
            two_sided: render.cull == CullMode::None,
            kind,
            sprites: start..out.sprites.len(),
        });
        out.batch_states.push(BatchState {
            batch,
            order,
            alpha_test: render.alpha.alpha_test(),
            depth_function: render.z_buffer.function,
            uv_transform: base.map_or(UvTransform::IDENTITY, |stage| stage.uv_transform),
            apply_mode,
            textures: stages,
        });
    }

    /// `NiMeshParticleSystem` (`NiPSysMeshUpdateModifier::Update`, UOSA.exe 0x008256d0): each particle draws
    /// a clone of the mesh prototype of its generation, its local transform replaced by the particle's position,
    /// rotation (`MakeRotation(angle, axis)`) and `|radius × size|`, in the state of its clone (the prototype's
    /// controllers at the particle's age).
    fn render_mesh_particles(&self, system: usize, world: &Xf, order: u32, out: &mut RenderFrame) {
        let desc = &self.effect.desc.particle_systems[system];
        let prototypes = mesh_prototypes(&self.effect.desc, system);
        let Some(last) = prototypes.len().checked_sub(1) else { return };
        let poses = self.prototype_poses.get(system).map_or(&[][..], Vec::as_slice);
        for (particle, pose) in self.particles(system).iter().zip(poses) {
            let prototype = prototypes[usize::from(particle.generation).min(last)];
            let rotation = if desc.has_rotation_angles {
                make_rotation(particle.rotation_angle, particle.rotation_axis.normalize_or_zero())
            } else {
                Mat3::IDENTITY
            };
            let placement =
                Xf { rotation, translation: particle.position, scale: (particle.radius * particle.size).abs() };
            self.draw_subtree(pose, prototype, world, Some(placement), order, out);
        }
    }

    /// Appends the effect's draws: mesh draws and particle batches of the visible scene nodes, in scene order.
    pub fn render_frame(&self, out: &mut RenderFrame) {
        let effect = &self.effect;
        for &node in &effect.order.nodes {
            let state = &self.scene.nodes[node];
            if !state.visible {
                continue;
            }
            let order = effect.draw_order[node];
            match effect.desc.nodes[node].kind {
                NifNodeKind::Mesh { mesh } => out.mesh_draws.push(self.mesh_draw(&self.scene, mesh, &state.world, order)),
                NifNodeKind::ParticleSystem { system } => self.render_particles(system, order, out),
                _ => {}
            }
        }
    }
}

impl<R: CrtRng + Send + Sync> ParticleSystemInstance for NifEffectInstance<R> {
    /// `Start` (0x0041ab40) with the definition's duration and loop flag (stored by `Init`).
    fn start(&mut self, duration_s: f32, looping: bool) {
        self.duration = duration_s;
        self.looping = looping;
        self.elapsed = 0.0;
        self.running = true;
        self.started = true;
    }

    /// `Stop` (0x0041ab60).
    fn stop(&mut self) {
        self.running = false;
    }

    fn update(&mut self, dt_ms: u32, node: &NodeTransform) {
        self.update_seconds(dt_ms as f32 * SECONDS_PER_MS, node);
    }

    fn is_finished(&self) -> bool {
        self.finished()
    }

    fn particle_count(&self) -> usize {
        self.scene.systems.iter().map(|system| system.pool.len()).sum()
    }

    /// The client releases a finished NIF effect with whatever it shows.
    fn is_drained(&self) -> bool {
        self.is_finished()
    }

    fn set_lifetime_ticks(&mut self, ticks: Option<u32>) {
        self.lifetime_ticks = ticks;
    }

    fn render(&self, out: &mut RenderFrame) {
        self.render_frame(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nif::desc::{
        BoolInterp, ControllerTarget, ControllerTiming, Curve, CycleType, EmitterShape, FloatInterp, Key, KeyKind,
        NifController, NifControllerKind, NifEmitter, NifMesh, NifModifier, NifModifierKind, NifNode,
        NifParticleSystem, NifTransform, Tcb,
    };

    const SEED: u32 = 3;
    const FRAME_MS: u32 = 33;
    const FRAMES: usize = 60;
    const BIRTH_RATE: f32 = 20.0;

    fn node(name: &str, parent: Option<usize>, children: Vec<usize>, kind: NifNodeKind) -> NifNode {
        NifNode {
            name: name.to_owned(),
            parent,
            children,
            transform: NifTransform::IDENTITY,
            flags: 0,
            kind,
            extra_strings: Vec::new(),
            in_scene: true,
        }
    }

    fn modifier(kind: NifModifierKind, order: u32) -> NifModifier {
        NifModifier { name: format!("m{order}"), order, active: true, kind }
    }

    fn timing() -> ControllerTiming {
        ControllerTiming {
            cycle: CycleType::Loop,
            active: true,
            app_init: false,
            reverse_direction: false,
            frequency: 1.0,
            phase: 0.0,
            start_time: 0.0,
            stop_time: 2.0,
        }
    }

    /// A fountain: a box emitter at 20 particles per second, aging and moving.
    fn fountain() -> Arc<NifEffect> {
        Arc::new(NifEffect::prepare(fountain_desc()))
    }

    fn fountain_desc() -> NifEffectDesc {
        let emitter = NifEmitter {
            speed: 5.0,
            speed_var: 1.0,
            declination: 0.2,
            declination_var: 0.2,
            planar_angle: 0.0,
            planar_angle_var: std::f32::consts::PI,
            initial_color: [1.0; 4],
            initial_radius: 1.0,
            radius_var: 0.0,
            life_span: 1.0,
            life_span_var: 0.5,
            shape: EmitterShape::Box { object: None, width: 1.0, height: 1.0, depth: 1.0 },
        };
        let system = NifParticleSystem {
            node: 1,
            world_space: true,
            capacity: 100,
            has_colors: true,
            has_radii: true,
            has_sizes: true,
            has_rotation_angles: false,
            has_rotation_axes: false,
            modifiers: vec![
                modifier(NifModifierKind::AgeDeath { spawn_on_death: false, spawn_modifier: None }, 0),
                modifier(NifModifierKind::Emitter(emitter), 1000),
                modifier(NifModifierKind::Position, 6000),
            ],
            render_state: NifRenderState::default(),
            mesh_particles: false,
        };
        let target = ControllerTarget::ParticleSystem(0);
        let desc = NifEffectDesc {
            name: "fountain".to_owned(),
            nodes: vec![
                node("root", None, vec![1], NifNodeKind::Node),
                node("psys", Some(0), Vec::new(), NifNodeKind::ParticleSystem { system: 0 }),
            ],
            root: Some(0),
            particle_systems: vec![system],
            controllers: vec![
                NifController {
                    owner: Some(1),
                    target,
                    timing: timing(),
                    kind: NifControllerKind::Emitter {
                        modifier: Some(1),
                        birth_rate: Some(FloatInterp { pose: Some(BIRTH_RATE), keys: None }),
                        active: Some(BoolInterp { pose: Some(true), keys: Vec::new() }),
                    },
                },
                NifController { owner: Some(1), target, timing: timing(), kind: NifControllerKind::ParticleUpdate },
            ],
            ..Default::default()
        };
        desc
    }

    fn run(seed: u32, frames: usize) -> NifEffectInstance {
        let mut instance = NifEffectInstance::with_seed(fountain(), seed);
        instance.start(0.0, true);
        for _ in 0..frames {
            instance.update(FRAME_MS, &NodeTransform::default());
        }
        instance
    }

    #[test]
    fn simulation_is_deterministic_for_a_seed() {
        let a = run(SEED, FRAMES);
        let b = run(SEED, FRAMES);
        assert!(!a.particles(0).is_empty());
        assert_eq!(a.particles(0), b.particles(0));
        assert_ne!(a.particles(0), run(SEED + 1, FRAMES).particles(0));
        // About birth rate × mean life span alive, never more than the capacity.
        let count = a.particle_count();
        assert!(count <= 100 && count as f32 > BIRTH_RATE * 0.5, "{count}");
    }

    #[test]
    fn effects_stop_at_their_duration_or_lifetime() {
        let mut instance = NifEffectInstance::with_seed(fountain(), SEED);
        assert!(!instance.is_finished(), "not started");
        instance.start(1.0, false);
        let frames_per_second = (crate::system::MS_PER_SECOND / FRAME_MS as f32).ceil() as usize;
        for _ in 0..frames_per_second - 1 {
            instance.update(FRAME_MS, &NodeTransform::default());
        }
        assert!(!instance.is_finished());
        instance.update(FRAME_MS, &NodeTransform::default());
        assert!(instance.is_finished());

        let mut looping = NifEffectInstance::with_seed(fountain(), SEED);
        looping.start(1.0, true);
        for _ in 0..FRAMES {
            looping.update(FRAME_MS, &NodeTransform::default());
            assert!(looping.elapsed() < 1.0);
        }
        assert!(!looping.is_finished());
        looping.set_lifetime_ticks(Some(2));
        looping.update(FRAME_MS, &NodeTransform::default());
        assert!(!looping.is_finished());
        looping.update(FRAME_MS, &NodeTransform::default());
        assert!(looping.is_finished());
    }

    #[test]
    fn particles_render_as_one_batch() {
        let instance = run(SEED, FRAMES);
        let mut frame = RenderFrame::default();
        instance.render(&mut frame);
        assert_eq!(frame.batches.len(), 1);
        assert_eq!(frame.batch_states.len(), 1);
        assert_eq!(frame.sprites.len(), instance.particle_count());
        assert_eq!(frame.batches[0].blend, OPAQUE_BLEND);
        let particle = instance.particles(0)[0];
        assert_eq!(frame.sprites[0].position, particle.position);
        assert!((frame.sprites[0].size - SPRITE_SIDE_PER_RADIUS * particle.radius * particle.size).abs() < 1e-6);
    }

    #[test]
    fn texture_transforms_match_the_sdk_matrices() {
        let maya = texture_transform_matrix(&NifTextureTransform::default());
        let [u, v] = maya.apply([0.25, 0.25]);
        assert!((u - 0.25).abs() < 1e-6 && (v - 0.75).abs() < 1e-6, "MAYA flips v: {u} {v}");
        let max = NifTextureTransform { translate: [0.1, 0.0], method: TextureTransformMethod::Max, ..Default::default() };
        let [u, v] = texture_transform_matrix(&max).apply([0.25, 0.25]);
        assert!((u - 0.15).abs() < 1e-6 && (v - 0.25).abs() < 1e-6, "{u} {v}");
    }

    #[test]
    fn mesh_particles_draw_their_prototype_animated_at_their_age() {
        const PROTOTYPE: usize = 2;
        const ALPHA_KEY_TIME: f32 = 2.0;
        let mut desc = fountain_desc();
        let system = &mut desc.particle_systems[0];
        system.mesh_particles = true;
        system.modifiers.push(modifier(NifModifierKind::MeshUpdate { meshes: vec![PROTOTYPE] }, 7000));
        let mut prototype = node("prototype", None, Vec::new(), NifNodeKind::Mesh { mesh: 0 });
        prototype.in_scene = false;
        desc.nodes.push(prototype);
        desc.materials.push(NifMaterial::default());
        desc.meshes.push(NifMesh {
            node: PROTOTYPE,
            geometry: MeshGeometry {
                positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                triangles: vec![[0, 1, 2]],
                ..MeshGeometry::default()
            },
            morph: None,
            render_state: NifRenderState { material: Some(0), ..NifRenderState::default() },
        });
        // The prototype's alpha is its time over [0, ALPHA_KEY_TIME].
        let key = |time: f32| Key { time, value: time / ALPHA_KEY_TIME, in_tangent: 0.0, out_tangent: 0.0, tcb: Tcb::default() };
        desc.controllers.push(NifController {
            owner: None,
            target: ControllerTarget::Material(0),
            timing: ControllerTiming { cycle: CycleType::Clamp, stop_time: ALPHA_KEY_TIME, ..timing() },
            kind: NifControllerKind::Alpha(FloatInterp {
                pose: None,
                keys: Some(Curve { kind: KeyKind::Linear, keys: vec![key(0.0), key(ALPHA_KEY_TIME)] }),
            }),
        });
        let mut instance = NifEffectInstance::with_seed(Arc::new(NifEffect::prepare(desc)), SEED);
        instance.start(0.0, true);
        for _ in 0..FRAMES {
            instance.update(FRAME_MS, &NodeTransform::default());
        }
        let mut frame = RenderFrame::default();
        instance.render(&mut frame);
        let particles = instance.particles(0);
        assert!(!particles.is_empty());
        assert!(frame.sprites.is_empty());
        assert_eq!(frame.mesh_draws.len(), particles.len());
        for (draw, particle) in frame.mesh_draws.iter().zip(particles) {
            assert!((draw.material.alpha - particle.age / ALPHA_KEY_TIME).abs() < 1e-4, "{draw:?} {particle:?}");
            assert!((draw.translation - particle.position).length() < 1e-4);
            assert!((draw.scale - (particle.radius * particle.size).abs()).abs() < 1e-6);
        }
    }
}
