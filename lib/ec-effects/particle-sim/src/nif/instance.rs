//! `UOParticleEffectNif`: a NIF effect as the client runs it, and its render output.

use std::sync::Arc;

use super::desc::{
    AV_FLAG_APP_CULLED, CullMode, NifEffectDesc, NifLight, NifLightKind, NifMaterial, NifModifierKind, NifNode,
    NifNodeKind, NifRenderState, NifTextureEffect, NifTextureTransform, SortingMode, TextureApplyMode,
    TextureCoordGen, TextureEffectKind, TextureFilter, TextureSlot, TextureTransformMethod,
};
use super::particles::NifParticle;
use super::scene::{SceneOrder, SceneState, Xf, make_rotation, transpose};
use crate::math::{Mat3, Vec3};
use crate::render::{
    BatchState, BlendFactor, BlendState, CameraFrame, DepthState, DrawBatch, EffectLight, EffectLightKind,
    EnvironmentMap, MaterialColors, MeshDraw, MeshGeometry, RenderFrame, SceneLighting, SortKey, SpriteInstance,
    SpriteKind, TextureStage, UvRect, UvTransform, accumulate_lights, light_vertex,
};
use crate::rng::{CrtRng, MsvcRand};
use crate::system::{NodeTransform, ParticleSystemInstance, SECONDS_PER_MS};

/// A particle's quad side relative to its radius (`NiPSFacingQuadGeneratorKernel`: half extent `radius * size`).
const SPRITE_SIDE_PER_RADIUS: f32 = 2.0;
/// Blending off: the source replaces the destination.
const OPAQUE_BLEND: BlendState = BlendState { src: BlendFactor::One, dst: BlendFactor::Zero };
/// Time scale of a plain `UOParticleEffectNif`.
const NORMAL_TIME_SCALE: f32 = 1.0;
/// Distance of [`default_camera`] from the origin.
const DEFAULT_CAMERA_DISTANCE: f32 = 1000.0;
/// Sphere map projection (`NiTextureEffect::UpdateProjection`, `NiTextureEffect.cpp:94-117`): scale and offset of
/// the effect-space reflection vector's z (u) and -y (v).
const SPHERE_MAP_SCALE: f32 = 0.5;
const SPHERE_MAP_OFFSET: f32 = 0.5;

/// The client's view ([`CameraFrame::client_view`]: looking along local +Y with +Z up, right along +X): the
/// billboard camera of an instance until [`NifEffectInstance::set_camera`].
pub fn default_camera() -> CameraFrame {
    CameraFrame::client_view(DEFAULT_CAMERA_DISTANCE)
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
    /// Per node: the [`NifEffectDesc::lights`] that light it ([`affected_effects`]).
    node_lights: Vec<Vec<usize>>,
    /// Per node: the [`NifEffectDesc::texture_effects`] that reach it, in push order ([`affected_effects`]).
    node_texture_effects: Vec<Vec<usize>>,
    /// Per node: whether its blended drawables are sorted ([`node_sorting`]).
    node_sorted: Vec<bool>,
}

impl NifEffect {
    pub fn prepare(desc: NifEffectDesc) -> Self {
        let order = SceneOrder::new(&desc);
        let mut draw_order = vec![u32::MAX; desc.nodes.len()];
        for (position, &node) in order.nodes.iter().enumerate() {
            draw_order[node] = position as u32;
        }
        Self {
            node_lights: affected_effects(&desc, |node| &node.lights, |light| {
                desc.lights.get(light).map_or(&[][..], |light| &light.unaffected_nodes)
            }),
            node_texture_effects: affected_effects(&desc, |node| &node.texture_effects, |effect| {
                desc.texture_effects.get(effect).map_or(&[][..], |effect| &effect.unaffected_nodes)
            }),
            node_sorted: node_sorting(&desc),
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

/// A value of each node inherited down the scene: `own(node, inherited)` gives the node's from its parent's.
/// The scene is walked from the root; mesh particle prototypes start from their particle system's value (the
/// clones hang under it); other nodes outside the scene start from `T::default()`.
fn inherit_down<T: Clone + Default>(desc: &NifEffectDesc, own: impl Fn(usize, T) -> T) -> Vec<T> {
    let count = desc.nodes.len();
    let mut values = vec![T::default(); count];
    let mut visited = vec![false; count];
    let mut walk = |top: usize, inherited: T, values: &mut Vec<T>| {
        let mut stack = vec![(top, inherited)];
        while let Some((node, inherited)) = stack.pop() {
            if node >= count || visited[node] {
                continue;
            }
            visited[node] = true;
            let value = own(node, inherited);
            stack.extend(desc.nodes[node].children.iter().map(|&child| (child, value.clone())));
            values[node] = value;
        }
    };
    if let Some(root) = desc.root {
        walk(root, T::default(), &mut values);
    }
    for (index, system) in desc.particle_systems.iter().enumerate() {
        let inherited = values.get(system.node).cloned().unwrap_or_default();
        for &prototype in mesh_prototypes(desc, index) {
            walk(prototype, inherited.clone(), &mut values);
        }
    }
    for node in 0..count {
        walk(node, T::default(), &mut values);
    }
    values
}

/// The dynamic effects (lights or texture effects) of each node: its effect state, as `NiAVObject::UpdateEffects`
/// (UOSA.exe 0x006641f0) builds it on the scene root once, at `UOParticleEffectNif::Init` (the call at
/// 0x0041aa53, after the time-0 update). Walking down (`NiNode::UpdateEffectsDownward`, `PushLocalEffects`), a
/// node drops the inherited effects that list it as unaffected and adds those of its own effect list (`own`);
/// children inherit the result.
fn affected_effects<'d>(
    desc: &'d NifEffectDesc,
    own: impl Fn(&'d NifNode) -> &'d [usize],
    unaffected: impl Fn(usize) -> &'d [usize],
) -> Vec<Vec<usize>> {
    inherit_down(desc, |node, inherited: Vec<usize>| {
        let mut state: Vec<usize> = inherited.into_iter().filter(|&effect| !unaffected(effect).contains(&node)).collect();
        for &effect in own(&desc.nodes[node]) {
            if !state.contains(&effect) {
                state.push(effect);
            }
        }
        state
    })
}

/// Whether each node's blended drawables are sorted: the outermost `NiSortAdjustNode` above (or at) the node
/// decides (`NiSortAdjustNode::OnVisible`, UOSA.exe 0x006e3130, sets the flag of its subtree after culling it,
/// so an outer node overrides an inner one); without one they are.
fn node_sorting(desc: &NifEffectDesc) -> Vec<bool> {
    let modes = inherit_down(desc, |node, inherited: Option<SortingMode>| inherited.or(desc.nodes[node].sort_adjust));
    modes.into_iter().map(|mode| mode != Some(SortingMode::Off)).collect()
}

/// The world projection of a texture effect at its node's world transform `world`
/// (`NiTextureEffect::UpdateProjection`, `NiTextureEffect.cpp:92-144`): (matrix, translation).
fn world_projection(effect: &NifTextureEffect, world: &Xf) -> (Mat3, Vec3) {
    let rotation = &world.rotation;
    let model = Mat3::from_rows(effect.model_projection);
    match effect.coord_gen {
        // Orthographic, at infinity: u = 0.5 + 0.5 s.z, v = 0.5 - 0.5 s.y with s in the effect's space.
        TextureCoordGen::SphereMap => {
            let column = |k: usize| {
                Vec3::new(SPHERE_MAP_SCALE * rotation.rows[k][2], -SPHERE_MAP_SCALE * rotation.rows[k][1], 0.0)
            };
            (Mat3::from_cols(column(0), column(1), column(2)), Vec3::new(SPHERE_MAP_OFFSET, SPHERE_MAP_OFFSET, 0.0))
        }
        TextureCoordGen::SpecularCubeMap | TextureCoordGen::DiffuseCubeMap => {
            (model.mul_mat3(&transpose(rotation)), Vec3::ZERO)
        }
        TextureCoordGen::WorldParallel | TextureCoordGen::WorldPerspective => {
            let inverse_scale = 1.0 / world.scale;
            let projection = model.mul_mat3(&transpose(rotation));
            let projection = Mat3::from_rows(projection.rows.map(|row| row.map(|value| value * inverse_scale)));
            let translation = Vec3::from_array(effect.model_translation) - projection.mul_vec3(world.translation);
            (projection, translation)
        }
    }
}

/// A light as the renderer applies it at the light node's world transform `world`; `None` when it is switched
/// off (`NiDX9LightManager::UpdateLights` skips it; the ambient sum `SCM_DEF_AMBIENTLIGHT` too).
fn effect_light(light: &NifLight, world: &Xf) -> Option<EffectLight> {
    if !light.on {
        return None;
    }
    let direction = world.rotation.col(0);
    let position = world.translation;
    let kind = match light.kind {
        NifLightKind::Ambient => EffectLightKind::Ambient,
        NifLightKind::Directional => EffectLightKind::Directional { direction },
        NifLightKind::Point { attenuation } => EffectLightKind::Point { position, attenuation },
        NifLightKind::Spot { attenuation, outer_angle, exponent } => EffectLightKind::Spot {
            position,
            direction,
            attenuation,
            // NiSpotLight::SetSpotAngle: cos(angle * NI_PI / 180).
            cos_outer: outer_angle.to_radians().cos(),
            exponent,
        },
    };
    let dim = |color: [f32; 3]| color.map(|channel| channel * light.dimmer);
    Some(EffectLight { kind, ambient: dim(light.ambient), diffuse: dim(light.diffuse) })
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
/// - Each update adds `dt` (times the time scale: 0.5 for hued effects) to the clock, counts the lifetime ticks
///   down, and stops a non-looping effect once `duration <= elapsed` (the effect is then finished and not updated
///   again, as in the client: a non-looping effect with duration 0 stops at its first update); a looping effect
///   with a duration wraps the clock (`fmod`). The scene is then updated at the clock time.
/// - The node transform replaces the root's local transform (the client sets the root rotation, `fabs(scale)`
///   and position each frame).
///
/// Simulation is deterministic for a seed: the client draws from the C runtime `rand()` shared by the whole
/// process; [`MsvcRand`] is that generator.
#[derive(Debug, Clone)]
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
    /// Multiplies every update's time step ([`Self::set_time_scale`]).
    time_scale: f32,
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
            time_scale: NORMAL_TIME_SCALE,
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

    /// Multiplies the time step of every update: [`crate::hued::HUED_EFFECT_TIME_SCALE`] runs the effect as
    /// `UOHuedParticleEffectNif` (half speed); the lifetime tick countdown still counts one tick per update.
    pub fn set_time_scale(&mut self, scale: f32) {
        self.time_scale = scale;
    }

    pub fn time_scale(&self) -> f32 {
        self.time_scale
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

    /// Advances by `dt` seconds times the time scale (`UOParticleEffectNif::Update`,
    /// `UOHuedParticleEffectNif::Update` 0x0041edc0).
    pub fn update_seconds(&mut self, dt: f32, node: &NodeTransform) {
        if !self.running {
            return;
        }
        self.elapsed += dt * self.time_scale;
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
        self.place_root(node);
        self.update_scene();
    }

    /// Updates the scene at effect time `time` with the root at `node`, outside the effect clock rules (no
    /// start, duration or ticks): a Chaos MESH particle's clone, updated with the particle age
    /// ([`crate::chaos::MeshSource`]).
    pub fn pose_at(&mut self, time: f32, node: &NodeTransform) {
        self.elapsed = time;
        self.place_root(node);
        self.update_scene();
    }

    fn place_root(&mut self, node: &NodeTransform) {
        if let Some(root) = self.effect.desc.root {
            let state = &mut self.scene.nodes[root];
            state.local = Xf { rotation: node.rotation, translation: node.position, scale: node.scale.abs() };
        }
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
                    filter: TextureFilter::from_raw(map.filter),
                })
            })
            .collect();
        (texturing.apply_mode, stages)
    }

    /// The current world lights ([`effect_light`]), by [`NifEffectDesc::lights`] index.
    fn world_lights(&self) -> Vec<Option<EffectLight>> {
        self.scene
            .lights
            .iter()
            .map(|light| self.scene.nodes.get(light.node).and_then(|node| effect_light(light, &node.world)))
            .collect()
    }

    /// The current environment maps, by [`NifEffectDesc::texture_effects`] index: `None` for other texture
    /// effects, switched off ones and ones without a texture.
    fn world_env_maps(&self) -> Vec<Option<EnvironmentMap>> {
        let desc = &self.effect.desc;
        desc.texture_effects
            .iter()
            .map(|effect| {
                let texture = self.effect.texture_names.get(effect.texture?)?.clone();
                if effect.kind != TextureEffectKind::EnvironmentMap || !effect.on {
                    return None;
                }
                let (projection, translation) = world_projection(effect, &self.scene.nodes.get(effect.node)?.world);
                Some(EnvironmentMap {
                    texture,
                    coord_gen: effect.coord_gen,
                    projection,
                    translation,
                    addressing: effect.addressing,
                    filter: TextureFilter::from_raw(effect.filter),
                })
            })
            .collect()
    }

    /// The environment map of a mesh at `node`: the last environment map pushed on its effect state
    /// (`NiDynamicEffectState::AddEffect` keeps one), when it is on (`SetEnvMapDescriptor`) and the geometry has
    /// the normals its coordinates need (`NiStandardMaterial.cpp:523-538`).
    fn node_env_map(&self, env_maps: &[Option<EnvironmentMap>], node: usize, geometry: &MeshGeometry) -> Option<EnvironmentMap> {
        let desc = &self.effect.desc;
        let last = self.effect.node_texture_effects.get(node)?.iter().rev().find(|&&effect| {
            desc.texture_effects.get(effect).is_some_and(|effect| effect.kind == TextureEffectKind::EnvironmentMap)
        })?;
        let env = env_maps.get(*last)?.clone()?;
        (!env.coord_gen.needs_normals() || !geometry.normals.is_empty()).then_some(env)
    }

    /// Whether a drawable at `node` with `render` state is sorted ([`SortKey::sorted`]).
    fn sorted(&self, node: usize, render: &NifRenderState) -> bool {
        render.alpha.blend && !render.alpha.no_sorter && self.effect.node_sorted.get(node).copied().unwrap_or(true)
    }

    /// The bound centre of particle system `system` in world space (`world`: the system's transform): with an
    /// `NiPSysBoundUpdateModifier`, the centre of the box of the particle positions sampled every `skip`
    /// particles (`Update`, UOSA.exe 0x00815930); otherwise the bound stored in the file.
    fn system_bound_center(&self, system: usize, world: &Xf) -> Vec3 {
        let desc = &self.effect.desc.particle_systems[system];
        let skip = desc.modifiers.iter().find_map(|modifier| match modifier.kind {
            NifModifierKind::BoundUpdate { skip } if modifier.active => Some(skip),
            _ => None,
        });
        let particles = self.particles(system);
        let local = match skip {
            Some(skip) if !particles.is_empty() => {
                let step = usize::try_from(skip).unwrap_or(0).max(1);
                let mut sampled = particles.iter().step_by(step).map(|particle| particle.position);
                let first = sampled.next().unwrap_or(Vec3::ZERO);
                let (min, max) = sampled.fold((first, first), |(min, max), p| {
                    (Vec3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z)), Vec3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z)))
                });
                (min + max) * 0.5
            }
            _ => Vec3::from_array(desc.bound_center),
        };
        world.point(local)
    }

    /// The lights reaching `node`, from the frame's [`Self::world_lights`].
    fn node_lights(&self, lights: &[Option<EffectLight>], node: usize) -> Vec<EffectLight> {
        self.effect
            .node_lights
            .get(node)
            .map_or_else(Vec::new, |indices| indices.iter().filter_map(|&light| lights.get(light).copied().flatten()).collect())
    }

    fn mesh_draw(&self, scene: &SceneState, mesh: usize, world: &Xf, order: u32, effects: &FrameEffects) -> MeshDraw {
        let desc = &self.effect.desc.meshes[mesh];
        let render = &desc.render_state;
        let (apply_mode, textures) = self.texture_stages(scene, render);
        let geometry = &self.effect.geometries[mesh];
        let center = world.point(Vec3::from_array(desc.bound_center));
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
            lights: self.node_lights(&effects.lights, desc.node),
            env_map: self.node_env_map(&effects.env_maps, desc.node, geometry),
            sort: SortKey { sorted: self.sorted(desc.node, render), center },
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
        effects: &FrameEffects,
        out: &mut RenderFrame,
    ) {
        let desc = &self.effect.desc;
        let state = &scene.nodes[node];
        if state.flags & AV_FLAG_APP_CULLED != 0 {
            return;
        }
        let world = parent.mul(&local.unwrap_or(state.local));
        if let NifNodeKind::Mesh { mesh } = desc.nodes[node].kind {
            out.mesh_draws.push(self.mesh_draw(scene, mesh, &world, order, effects));
        }
        for &child in &desc.nodes[node].children {
            self.draw_subtree(scene, child, &world, None, order, effects, out);
        }
    }

    fn render_particles(&self, system: usize, order: u32, effects: &FrameEffects, out: &mut RenderFrame) {
        let desc = &self.effect.desc.particle_systems[system];
        let world = self.scene.nodes[desc.node].world;
        let particles = self.particles(system);
        if particles.is_empty() {
            return;
        }
        if desc.mesh_particles {
            self.render_mesh_particles(system, &world, order, effects, out);
            return;
        }
        let render = &desc.render_state;
        let material = Self::material_colors(&self.scene, render);
        let (apply_mode, stages) = self.texture_stages(&self.scene, render);
        let base = stages.iter().find(|stage| stage.slot == TextureSlot::Base);
        let replaces = apply_mode == TextureApplyMode::Replace && base.is_some();
        let kind = if desc.has_rotation_angles { SpriteKind::RotatedBillboard } else { SpriteKind::Billboard };
        // Particles have no normals: only the effect's ambient lights reach them, the same at every particle.
        let lighting = accumulate_lights(&self.lighting, &self.node_lights(&effects.lights, desc.node), Vec3::ZERO, None);
        let start = out.sprites.len();
        for particle in particles {
            let color = if replaces {
                [1.0; 4]
            } else {
                let vertex = desc.has_colors.then_some(particle.color);
                light_vertex(&material, vertex, render.vertex_color, &lighting)
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
            sort: SortKey { sorted: self.sorted(desc.node, render), center: self.system_bound_center(system, &world) },
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
    fn render_mesh_particles(
        &self,
        system: usize,
        world: &Xf,
        order: u32,
        effects: &FrameEffects,
        out: &mut RenderFrame,
    ) {
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
            self.draw_subtree(pose, prototype, world, Some(placement), order, effects, out);
        }
    }

    /// Appends the effect's draws: mesh draws and particle batches of the visible scene nodes, in scene order.
    pub fn render_frame(&self, out: &mut RenderFrame) {
        let effect = &self.effect;
        let effects = FrameEffects { lights: self.world_lights(), env_maps: self.world_env_maps() };
        for &node in &effect.order.nodes {
            let state = &self.scene.nodes[node];
            if !state.visible {
                continue;
            }
            let order = effect.draw_order[node];
            match effect.desc.nodes[node].kind {
                NifNodeKind::Mesh { mesh } => {
                    out.mesh_draws.push(self.mesh_draw(&self.scene, mesh, &state.world, order, &effects));
                }
                NifNodeKind::ParticleSystem { system } => self.render_particles(system, order, &effects, out),
                _ => {}
            }
        }
    }
}

/// The dynamic effects of a frame at their current world transforms.
struct FrameEffects {
    /// By [`NifEffectDesc::lights`] index ([`NifEffectInstance::world_lights`]).
    lights: Vec<Option<EffectLight>>,
    /// By [`NifEffectDesc::texture_effects`] index ([`NifEffectInstance::world_env_maps`]).
    env_maps: Vec<Option<EnvironmentMap>>,
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
        NifParticleSystem, NifTransform, Point3Interp, Tcb,
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
            lights: Vec::new(),
            texture_effects: Vec::new(),
            sort_adjust: None,
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
            bound_center: [0.0; 3],
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
            bound_center: [0.0; 3],
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

    /// Root 0 lists directional lights 0 (node 4) and 1 (node 5); node 1 is unaffected by light 1. Mesh 0 (node 2)
    /// hangs under node 1, mesh 1 (node 3) under the root.
    fn lit_desc() -> NifEffectDesc {
        let mut root = node("root", None, vec![1, 3, 4, 5], NifNodeKind::Node);
        root.lights = vec![0, 1];
        let mut light_node = node("light", Some(0), Vec::new(), NifNodeKind::Light { light: 0 });
        // Model +X turned to world -Z: the light shines down.
        light_node.transform.rotation = [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]];
        let nodes = vec![
            root,
            node("shaded", Some(0), vec![2], NifNodeKind::Node),
            node("mesh", Some(1), Vec::new(), NifNodeKind::Mesh { mesh: 0 }),
            node("mesh", Some(0), Vec::new(), NifNodeKind::Mesh { mesh: 1 }),
            light_node,
            node("light", Some(0), Vec::new(), NifNodeKind::Light { light: 1 }),
        ];
        let mesh = |node| NifMesh {
            node,
            geometry: MeshGeometry {
                positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                triangles: vec![[0, 1, 2]],
                ..MeshGeometry::default()
            },
            bound_center: [0.0; 3],
            morph: None,
            render_state: NifRenderState::default(),
        };
        let light = |node, unaffected_nodes| NifLight {
            node,
            kind: NifLightKind::Directional,
            on: true,
            dimmer: 1.0,
            ambient: [0.0; 3],
            diffuse: [1.0; 3],
            specular: [1.0; 3],
            unaffected_nodes,
        };
        NifEffectDesc {
            nodes,
            root: Some(0),
            meshes: vec![mesh(2), mesh(3)],
            lights: vec![light(4, Vec::new()), light(5, vec![1])],
            ..NifEffectDesc::default()
        }
    }

    fn rendered_lights(desc: NifEffectDesc) -> Vec<Vec<EffectLight>> {
        let instance = NifEffectInstance::with_seed(Arc::new(NifEffect::prepare(desc)), SEED);
        let mut frame = RenderFrame::default();
        instance.render(&mut frame);
        frame.mesh_draws.into_iter().map(|draw| draw.lights).collect()
    }

    #[test]
    fn effect_lists_light_their_subtrees_except_unaffected_nodes() {
        let desc = lit_desc();
        let lights = affected_effects(&desc, |node| &node.lights, |light| &desc.lights[light].unaffected_nodes);
        assert_eq!(lights, vec![vec![0, 1], vec![0], vec![0], vec![0, 1], vec![0, 1], vec![0, 1]]);
        let draws = rendered_lights(desc.clone());
        assert_eq!(draws.iter().map(Vec::len).collect::<Vec<_>>(), [1, 2]);
        // The direction is the light node's world rotation column 0.
        assert_eq!(draws[0][0].kind, EffectLightKind::Directional { direction: -Vec3::Z });
        assert_eq!(draws[1][1].kind, EffectLightKind::Directional { direction: Vec3::X });
        // A switched-off light reaches nothing.
        let mut off = desc;
        off.lights[0].on = false;
        assert_eq!(rendered_lights(off).iter().map(Vec::len).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn light_controllers_drive_colours_and_dimmer() {
        const DIMMER: f32 = 0.5;
        const COLOR: [f32; 3] = [0.25, 2.0, -1.0];
        let mut desc = lit_desc();
        let controller = |target, kind| NifController { owner: Some(0), target, timing: timing(), kind };
        desc.controllers = vec![
            controller(ControllerTarget::Light(0), NifControllerKind::LightDimmer(FloatInterp { pose: Some(DIMMER), keys: None })),
            controller(
                ControllerTarget::Light(0),
                NifControllerKind::LightColor { ambient: false, interp: Point3Interp { pose: Some(COLOR), keys: None } },
            ),
            controller(
                ControllerTarget::Light(1),
                NifControllerKind::LightColor { ambient: true, interp: Point3Interp { pose: Some(COLOR), keys: None } },
            ),
        ];
        let draws = rendered_lights(desc);
        // Clamped to [0, 1], times the dimmer; the diffuse colour also sets the specular one and clears the
        // ambient one, and the reverse.
        let light = draws[1][0];
        assert_eq!((light.ambient, light.diffuse), ([0.0; 3], [0.125, 0.5, 0.0]));
        let light = draws[1][1];
        assert_eq!((light.ambient, light.diffuse), ([0.25, 1.0, 0.0], [0.0; 3]));
    }

    /// A one-triangle mesh on `node` with normals, blended or not, `no_sorter` as given.
    fn triangle_mesh(node: usize, blend: bool, no_sorter: bool) -> NifMesh {
        use crate::nif::desc::NifAlpha;
        NifMesh {
            node,
            geometry: MeshGeometry {
                positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                triangles: vec![[0, 1, 2]],
                ..MeshGeometry::default()
            },
            bound_center: [0.25, 0.25, 0.0],
            morph: None,
            render_state: NifRenderState {
                alpha: NifAlpha { blend, no_sorter, ..NifAlpha::default() },
                ..NifRenderState::default()
            },
        }
    }

    fn rendered(desc: NifEffectDesc) -> RenderFrame {
        let instance = NifEffectInstance::with_seed(Arc::new(NifEffect::prepare(desc)), SEED);
        let mut frame = RenderFrame::default();
        instance.render(&mut frame);
        frame
    }

    #[test]
    fn blended_drawables_sort_unless_no_sorter_or_under_a_sorting_off_node() {
        // 0 root: 1 (sorting off) > 2 (sort-adjust inherit) > 3 mesh; root > 4 blended, 5 no-sorter, 6 opaque.
        let mut off = node("off", Some(0), vec![2], NifNodeKind::Node);
        off.sort_adjust = Some(SortingMode::Off);
        let mut inherit = node("inherit", Some(1), vec![3], NifNodeKind::Node);
        inherit.sort_adjust = Some(SortingMode::Inherit);
        let mut blended = node("blended", Some(0), Vec::new(), NifNodeKind::Mesh { mesh: 1 });
        blended.transform.translation = [10.0, 0.0, 0.0];
        let desc = NifEffectDesc {
            nodes: vec![
                node("root", None, vec![1, 4, 5, 6], NifNodeKind::Node),
                off,
                inherit,
                node("under off", Some(2), Vec::new(), NifNodeKind::Mesh { mesh: 0 }),
                blended,
                node("no sorter", Some(0), Vec::new(), NifNodeKind::Mesh { mesh: 2 }),
                node("opaque", Some(0), Vec::new(), NifNodeKind::Mesh { mesh: 3 }),
            ],
            root: Some(0),
            meshes: vec![triangle_mesh(3, true, false), triangle_mesh(4, true, false), triangle_mesh(5, true, true), triangle_mesh(6, false, false)],
            ..NifEffectDesc::default()
        };
        let frame = rendered(desc);
        let sorted: Vec<(&str, bool)> = frame.mesh_draws.iter().map(|draw| (&*draw.name, draw.sort.sorted)).collect();
        assert_eq!(sorted, [("under off", false), ("blended", true), ("no sorter", false), ("opaque", false)]);
        // The bound centre is the model bound's in world space.
        assert_eq!(frame.mesh_draws[1].sort.center, Vec3::new(10.25, 0.25, 0.0));
    }

    #[test]
    fn environment_maps_reach_meshes_with_normals_in_their_effect_lists() {
        // Root lists env maps 0 and 1 (the last pushed wins); 1 skips node 2. Mesh 1 (node 3) has no normals.
        let env = |node, texture, unaffected_nodes| NifTextureEffect {
            node,
            kind: TextureEffectKind::EnvironmentMap,
            coord_gen: TextureCoordGen::SphereMap,
            on: true,
            texture: Some(texture),
            model_projection: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            model_translation: [0.0; 3],
            filter: crate::nif::desc::FILTER_TRILERP,
            addressing: crate::nif::desc::TextureAddressing { wrap_s: true, wrap_t: true },
            unaffected_nodes,
        };
        let mut root = node("root", None, vec![1, 2, 3, 4, 5], NifNodeKind::Node);
        root.texture_effects = vec![0, 1];
        let texture = |index, name: &str| crate::nif::desc::NifTexture {
            index,
            source: crate::nif::desc::NifTextureSource::External { file_name: name.to_owned() },
            mipmapped: false,
        };
        let mut bare = triangle_mesh(3, false, false);
        bare.geometry.normals.clear();
        let mut desc = NifEffectDesc {
            nodes: vec![
                root,
                node("mirror", Some(0), Vec::new(), NifNodeKind::Mesh { mesh: 0 }),
                node("skipped", Some(0), Vec::new(), NifNodeKind::Mesh { mesh: 2 }),
                node("bare", Some(0), Vec::new(), NifNodeKind::Mesh { mesh: 1 }),
                node("env 0", Some(0), Vec::new(), NifNodeKind::TextureEffect { effect: 0 }),
                node("env 1", Some(0), Vec::new(), NifNodeKind::TextureEffect { effect: 1 }),
            ],
            root: Some(0),
            meshes: vec![triangle_mesh(1, false, false), bare, triangle_mesh(2, false, false)],
            textures: vec![texture(0, "first.dds"), texture(1, "second.tga")],
            texture_effects: vec![env(4, 0, Vec::new()), env(5, 1, vec![2])],
            ..NifEffectDesc::default()
        };
        let maps = |desc: NifEffectDesc| -> Vec<Option<String>> {
            rendered(desc).mesh_draws.iter().map(|draw| draw.env_map.as_ref().map(|env| env.texture.to_string())).collect()
        };
        // Scene order: mirror, skipped, bare.
        assert_eq!(maps(desc.clone()), [Some("second.tga".to_owned()), Some("first.dds".to_owned()), None]);
        // The sphere map projection of an effect at the identity.
        let frame = rendered(desc.clone());
        let env_map = frame.mesh_draws[0].env_map.as_ref().unwrap();
        assert_eq!(env_map.projection, Mat3::from_rows([[0.0, 0.0, 0.5], [0.0, -0.5, 0.0], [0.0, 0.0, 0.0]]));
        assert_eq!(env_map.translation, Vec3::new(0.5, 0.5, 0.0));
        assert_eq!(env_map.filter, TextureFilter::TRILERP);
        // A switched-off effect still takes the slot (`NiDynamicEffectState::AddEffect`), so nothing is mapped.
        desc.texture_effects[1].on = false;
        assert_eq!(maps(desc), [None, Some("first.dds".to_owned()), None]);
    }

    #[test]
    fn particle_batches_sort_by_their_bound_centre() {
        // Without a bound update modifier: the file's bound.
        let mut desc = fountain_desc();
        desc.particle_systems[0].bound_center = [1.0, 2.0, 3.0];
        desc.particle_systems[0].render_state.alpha.blend = true;
        let mut instance = NifEffectInstance::with_seed(Arc::new(NifEffect::prepare(desc.clone())), SEED);
        instance.start(0.0, true);
        for _ in 0..FRAMES {
            instance.update(FRAME_MS, &NodeTransform::default());
        }
        let mut frame = RenderFrame::default();
        instance.render(&mut frame);
        assert_eq!(frame.batches[0].sort, SortKey::sorted(Vec3::new(1.0, 2.0, 3.0)));
        // With one: the centre of the box of every particle (skip 0 samples them all).
        desc.particle_systems[0].modifiers.push(modifier(NifModifierKind::BoundUpdate { skip: 0 }, 8000));
        let mut instance = NifEffectInstance::with_seed(Arc::new(NifEffect::prepare(desc)), SEED);
        instance.start(0.0, true);
        for _ in 0..FRAMES {
            instance.update(FRAME_MS, &NodeTransform::default());
        }
        let mut frame = RenderFrame::default();
        instance.render(&mut frame);
        let particles = instance.particles(0);
        let axis = |f: fn(&NifParticle) -> f32| {
            let values = particles.iter().map(f);
            (values.clone().fold(f32::INFINITY, f32::min) + values.fold(f32::NEG_INFINITY, f32::max)) * 0.5
        };
        let expected = Vec3::new(axis(|p| p.position.x), axis(|p| p.position.y), axis(|p| p.position.z));
        assert!((frame.batches[0].sort.center - expected).length() < 1e-5, "{:?} {expected:?}", frame.batches[0].sort);
    }
}
