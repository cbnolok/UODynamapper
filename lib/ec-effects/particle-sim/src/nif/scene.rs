//! The scene graph runtime: node transforms, controllers, billboards and morphing.
//!
//! An update at time `t` walks the scene depth first as `NiNode::UpdateDownwardPass` does: a node's own
//! controllers, then its properties' controllers, then its world transform, then its children. A particle system
//! advances inside its `NiPSysUpdateCtlr`, before its own world transform is recomputed, so it reads the world
//! transforms that the walk has left so far (this frame's for nodes before it, the previous frame's for the
//! others), as in the client. Billboard nodes turn to the camera afterwards (`NiBillboardNode::OnVisible`).

use std::sync::Arc;

use super::desc::{
    BillboardMode, ControllerTarget, CycleType, MaterialColorField, ModifierParam, NifControllerKind, NifEffectDesc,
    NifInterpolator, NifLight, NifMaterial, NifModifierKind, NifNodeKind, NifTextureTransform, NifTransform,
    TextureTransformMember, AV_FLAG_APP_CULLED,
};
use super::keys::{ControllerClock, Quat};
use super::particles::{
    emitter_controller_emissions, spawn_particles, DeferredSpawn, Emission, EmitterClock, SystemContext,
    SystemState,
};
use crate::math::{Mat3, Vec3};
use crate::render::CameraFrame;
use crate::rng::CrtRng;

/// `NiBillboardNode::RotateToCamera`: below this the camera is along the billboard's Y axis.
const BILLBOARD_ROOT_EPSILON: f32 = 1e-6;
/// `ROTATE_ABOUT_UP`: below this the camera is above the billboard.
const BILLBOARD_UP_EPSILON: f32 = 1e-12;
/// `RotateToCenter`: below this squared distance the camera is at the billboard.
const BILLBOARD_CENTER_EPSILON: f32 = 0.001;
/// `RotateToCenter`: above this cosine the view already points at the centre.
const BILLBOARD_CENTER_DOT: f32 = 0.999_999;
/// `NiMorphingKernel`: targets with a smaller weight are skipped.
const MORPH_WEIGHT_EPSILON: f32 = 0.001;

/// A Gamebryo transform (`NiTransform`): `p' = translation + scale * rotation * p`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xf {
    pub rotation: Mat3,
    pub translation: Vec3,
    pub scale: f32,
}

impl Default for Xf {
    fn default() -> Self {
        Self::IDENTITY
    }
}

pub(crate) fn transpose(m: &Mat3) -> Mat3 {
    Mat3::from_cols(Vec3::from_array(m.rows[0]), Vec3::from_array(m.rows[1]), Vec3::from_array(m.rows[2]))
}

impl Xf {
    pub const IDENTITY: Self = Self { rotation: Mat3::IDENTITY, translation: Vec3::ZERO, scale: 1.0 };

    pub fn from_desc(transform: &NifTransform) -> Self {
        Self {
            rotation: Mat3::from_rows(transform.rotation),
            translation: Vec3::from_array(transform.translation),
            scale: transform.scale,
        }
    }

    /// `self * child` (`NiTransform::operator*`).
    pub fn mul(&self, child: &Self) -> Self {
        Self {
            rotation: self.rotation.mul_mat3(&child.rotation),
            translation: self.translation + self.rotation.mul_vec3(child.translation) * self.scale,
            scale: self.scale * child.scale,
        }
    }

    /// `NiTransform::Invert`.
    pub fn inverse(&self) -> Self {
        let rotation = transpose(&self.rotation);
        let scale = 1.0 / self.scale;
        Self { rotation, translation: rotation.mul_vec3(self.translation) * -scale, scale }
    }

    pub fn point(&self, p: Vec3) -> Vec3 {
        self.translation + self.rotation.mul_vec3(p) * self.scale
    }
}

/// A scene node's runtime state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NodeState {
    pub local: Xf,
    pub world: Xf,
    pub flags: u16,
    /// Not culled, nor any ancestor (`APP_CULLED`).
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ControllerState {
    pub clock: ControllerClock,
    pub emitter: EmitterClock,
    /// `NiPSysResetOnLoopCtlr` +0x34.
    pub reset_last: Option<f32>,
}

/// The animated state of one effect.
#[derive(Debug, Clone)]
pub(crate) struct SceneState {
    pub nodes: Vec<NodeState>,
    pub materials: Vec<NifMaterial>,
    /// The lights, with their animated colours and dimmer.
    pub lights: Vec<NifLight>,
    /// Per texturing property, per map: the current texture transform.
    pub texture_transforms: Vec<Vec<Option<NifTextureTransform>>>,
    pub morph_weights: Vec<Vec<f32>>,
    /// Per mesh: the morphed vertex positions.
    pub morphed: Vec<Option<Arc<[[f32; 3]]>>>,
    pub controllers: Vec<ControllerState>,
    pub systems: Vec<SystemState>,
}

/// The update order of an effect: scene nodes depth first, and per node the controllers it runs.
#[derive(Debug, Clone, Default)]
pub(crate) struct SceneOrder {
    pub nodes: Vec<usize>,
    pub controllers: Vec<Vec<usize>>,
    pub has_billboards: bool,
    /// Per node: when it is a mesh particle prototype (`NiPSysMeshUpdateModifier`), the controllers of its
    /// subtree (targeting its nodes, their meshes' morphs and render-state properties), in list order.
    pub prototype_controllers: Vec<Vec<usize>>,
}

impl SceneOrder {
    pub fn new(desc: &NifEffectDesc) -> Self {
        let mut nodes = Vec::new();
        if let Some(root) = desc.root {
            let mut stack = vec![root];
            while let Some(node) = stack.pop() {
                if node >= desc.nodes.len() || nodes.contains(&node) {
                    continue;
                }
                nodes.push(node);
                stack.extend(desc.nodes[node].children.iter().rev());
            }
        }
        let mut controllers = vec![Vec::new(); desc.nodes.len()];
        for (index, controller) in desc.controllers.iter().enumerate() {
            if let Some(owner) = controller.owner.filter(|&owner| owner < desc.nodes.len()) {
                controllers[owner].push(index);
            }
        }
        let has_billboards =
            nodes.iter().any(|&node| matches!(desc.nodes[node].kind, NifNodeKind::Billboard(_)));
        let mut prototype_controllers = vec![Vec::new(); desc.nodes.len()];
        for system in &desc.particle_systems {
            for modifier in &system.modifiers {
                let NifModifierKind::MeshUpdate { meshes } = &modifier.kind else { continue };
                for &prototype in meshes.iter().filter(|&&prototype| prototype < desc.nodes.len()) {
                    prototype_controllers[prototype] = subtree_controllers(desc, prototype);
                }
            }
        }
        Self { nodes, controllers, has_billboards, prototype_controllers }
    }
}

/// The controllers animating the subtree of `root`: those targeting its nodes, the morphs of its meshes, and
/// the materials and texturing properties of its meshes.
fn subtree_controllers(desc: &NifEffectDesc, root: usize) -> Vec<usize> {
    let mut subtree = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node >= desc.nodes.len() || subtree.contains(&node) {
            continue;
        }
        subtree.push(node);
        stack.extend(desc.nodes[node].children.iter().copied());
    }
    let meshes: Vec<usize> = subtree
        .iter()
        .filter_map(|&node| match desc.nodes[node].kind {
            NifNodeKind::Mesh { mesh } => Some(mesh),
            _ => None,
        })
        .collect();
    let renders = || meshes.iter().filter_map(|&mesh| desc.meshes.get(mesh)).map(|mesh| &mesh.render_state);
    desc.controllers
        .iter()
        .enumerate()
        .filter(|(_, controller)| match (&controller.kind, controller.target) {
            (NifControllerKind::Morph { mesh, .. }, _) => meshes.contains(mesh),
            (_, ControllerTarget::Node(node)) => subtree.contains(&node),
            (_, ControllerTarget::Material(material)) => renders().any(|render| render.material == Some(material)),
            (_, ControllerTarget::Texturing(texturing)) => {
                renders().any(|render| render.texturing == Some(texturing))
            }
            (_, ControllerTarget::ParticleSystem(_) | ControllerTarget::Light(_)) => false,
        })
        .map(|(index, _)| index)
        .collect()
}

impl SceneState {
    pub fn new(desc: &NifEffectDesc) -> Self {
        let nodes = desc
            .nodes
            .iter()
            .map(|node| {
                let local = Xf::from_desc(&node.transform);
                NodeState { local, world: local, flags: node.flags, visible: node.flags & AV_FLAG_APP_CULLED == 0 }
            })
            .collect();
        let texture_transforms = desc
            .texturings
            .iter()
            .map(|texturing| texturing.maps.iter().map(|map| map.transform).collect())
            .collect();
        let morph_weights = desc
            .meshes
            .iter()
            .map(|mesh| {
                mesh.morph.as_ref().map_or_else(Vec::new, |morph| {
                    morph.targets.iter().map(|target| target.initial_weight).collect()
                })
            })
            .collect();
        Self {
            nodes,
            materials: desc.materials.clone(),
            lights: desc.lights.clone(),
            texture_transforms,
            morph_weights,
            morphed: vec![None; desc.meshes.len()],
            controllers: vec![ControllerState::default(); desc.controllers.len()],
            systems: desc.particle_systems.iter().map(SystemState::new).collect(),
        }
    }

    /// The world transform of `node` from its parent's and its local transform; world-space particle systems
    /// keep the identity (UOSA.exe 0x008116f0).
    fn update_world(&mut self, desc: &NifEffectDesc, node: usize) {
        let parent = desc.nodes[node].parent.map(|parent| self.nodes[parent]);
        let world_space = match desc.nodes[node].kind {
            NifNodeKind::ParticleSystem { system } => desc.particle_systems.get(system).is_some_and(|s| s.world_space),
            _ => false,
        };
        let state = &mut self.nodes[node];
        state.world = if world_space {
            Xf::IDENTITY
        } else {
            match parent {
                Some(parent) => parent.world.mul(&state.local),
                None => state.local,
            }
        };
        let culled = state.flags & AV_FLAG_APP_CULLED != 0;
        state.visible = !culled && parent.is_none_or(|parent| parent.visible);
    }

    /// `NiAVObject::Update(time)` of the root.
    pub fn update(
        &mut self,
        desc: &NifEffectDesc,
        order: &SceneOrder,
        time: f32,
        camera: &CameraFrame,
        rng: &mut impl CrtRng,
    ) {
        for &node in &order.nodes {
            for &controller in &order.controllers[node] {
                self.run_controller(desc, controller, time, rng);
            }
            self.update_world(desc, node);
        }
        if order.has_billboards {
            self.apply_billboards(desc, order, camera);
        }
    }

    /// The state of one mesh particle's prototype clone. The client updates each particle's clone with the
    /// particle's age as the time, controllers included (`NiPSysMeshUpdateModifier::Update`, UOSA.exe
    /// 0x008256d0, calls the clone's update with `age` after placing it); here the prototype's `controllers`
    /// run from fresh clocks at `age` on a copy of this state (without particle systems).
    pub fn prototype_pose(&self, desc: &NifEffectDesc, controllers: &[usize], age: f32, rng: &mut impl CrtRng) -> Self {
        let mut pose = Self {
            nodes: self.nodes.clone(),
            materials: self.materials.clone(),
            lights: self.lights.clone(),
            texture_transforms: self.texture_transforms.clone(),
            morph_weights: self.morph_weights.clone(),
            morphed: self.morphed.clone(),
            controllers: self.controllers.clone(),
            systems: Vec::new(),
        };
        for &controller in controllers {
            pose.controllers[controller] = ControllerState::default();
            pose.run_controller(desc, controller, age, rng);
        }
        pose
    }

    /// Billboards turn to the camera and push their transform to their descendants.
    fn apply_billboards(&mut self, desc: &NifEffectDesc, order: &SceneOrder, camera: &CameraFrame) {
        for &node in &order.nodes {
            self.update_world(desc, node);
            if let NifNodeKind::Billboard(mode) = desc.nodes[node].kind {
                let world = &mut self.nodes[node].world;
                let face = billboard_face(mode, world, camera);
                world.rotation = world.rotation.mul_mat3(&face);
            }
        }
    }

    fn run_controller(&mut self, desc: &NifEffectDesc, index: usize, time: f32, rng: &mut impl CrtRng) {
        let controller = &desc.controllers[index];
        let timing = &controller.timing;
        let clock = &mut self.controllers[index].clock;
        match &controller.kind {
            NifControllerKind::ParticleUpdate => {
                // NiPSysUpdateCtlr::Update (0x0080ddb0): unbounded range, CLAMP.
                let ControllerTarget::ParticleSystem(system) = controller.target else { return };
                if let Some(scaled) = clock.tick(timing, CycleType::Clamp, None, time) {
                    self.update_system(desc, system, scaled, rng);
                }
            }
            NifControllerKind::ResetOnLoop => {
                // NiPSysResetOnLoopCtlr::Update (0x0080a0d0): LOOP.
                let ControllerTarget::ParticleSystem(system) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, CycleType::Loop, timing.range(), time) else { return };
                let state = &mut self.controllers[index];
                let last = *state.reset_last.get_or_insert(scaled);
                if scaled < last {
                    if let Some(system) = self.systems.get_mut(system) {
                        system.reset_pending = true;
                    }
                }
                state.reset_last = Some(scaled);
            }
            NifControllerKind::Emitter { modifier, birth_rate, active } => {
                let ControllerTarget::ParticleSystem(system) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                let (Some(modifier), Some(state)) = (*modifier, self.systems.get(system)) else { return };
                let mut emissions = Vec::new();
                emitter_controller_emissions(
                    &mut self.controllers[index].emitter,
                    birth_rate.as_ref(),
                    active.as_ref(),
                    timing,
                    scaled,
                    state.last_time,
                    &mut emissions,
                );
                self.emit(desc, system, modifier, &emissions, rng);
            }
            NifControllerKind::ModifierFloat { modifier, param, interp } => {
                let ControllerTarget::ParticleSystem(system) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                let Some(value) = interp.as_ref().and_then(|interp| interp.eval(scaled)) else { return };
                let Some(state) = modifier.and_then(|m| self.systems.get_mut(system)?.modifiers.get_mut(m)) else {
                    return;
                };
                set_modifier_param(&mut state.kind, *param, value);
            }
            NifControllerKind::ModifierActive { modifier, interp } => {
                let ControllerTarget::ParticleSystem(system) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                let Some(value) = interp.as_ref().and_then(|interp| interp.eval(scaled)) else { return };
                if let Some(state) = modifier.and_then(|m| self.systems.get_mut(system)?.modifiers.get_mut(m)) {
                    state.active = value;
                }
            }
            NifControllerKind::Transform(interp) => {
                let ControllerTarget::Node(node) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                if let Some(state) = self.nodes.get_mut(node) {
                    apply_transform(interp, scaled, &mut state.local);
                }
            }
            NifControllerKind::Visibility(interp) => {
                let ControllerTarget::Node(node) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                if let (Some(visible), Some(state)) = (interp.eval(scaled), self.nodes.get_mut(node)) {
                    if visible {
                        state.flags &= !AV_FLAG_APP_CULLED;
                    } else {
                        state.flags |= AV_FLAG_APP_CULLED;
                    }
                }
            }
            NifControllerKind::Alpha(interp) => {
                let ControllerTarget::Material(material) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                if let (Some(alpha), Some(material)) = (interp.eval(scaled), self.materials.get_mut(material)) {
                    material.alpha = alpha;
                }
            }
            NifControllerKind::MaterialColor { field, interp } => {
                let ControllerTarget::Material(material) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                let (Some(color), Some(material)) = (interp.eval(scaled), self.materials.get_mut(material)) else {
                    return;
                };
                match field {
                    MaterialColorField::Ambient => material.ambient = color,
                    MaterialColorField::Diffuse => material.diffuse = color,
                    MaterialColorField::Specular => material.specular = color,
                    MaterialColorField::Emissive => material.emissive = color,
                }
            }
            NifControllerKind::LightColor { ambient, interp } => {
                let ControllerTarget::Light(light) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                let (Some(color), Some(light)) = (interp.eval(scaled), self.lights.get_mut(light)) else { return };
                let color = color.map(|channel| channel.clamp(0.0, 1.0));
                let black = [0.0; 3];
                if *ambient {
                    (light.ambient, light.diffuse, light.specular) = (color, black, black);
                } else {
                    (light.ambient, light.diffuse, light.specular) = (black, color, color);
                }
            }
            NifControllerKind::LightDimmer(interp) => {
                let ControllerTarget::Light(light) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                if let (Some(dimmer), Some(light)) = (interp.eval(scaled), self.lights.get_mut(light)) {
                    light.dimmer = dimmer;
                }
            }
            NifControllerKind::TextureTransform { slot, member, interp } => {
                let ControllerTarget::Texturing(texturing) = controller.target else { return };
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                let Some(value) = interp.eval(scaled) else { return };
                let Some(map) = desc.texturings.get(texturing).and_then(|t| t.maps.iter().position(|m| m.slot == *slot))
                else {
                    return;
                };
                // NiTextureTransformController::Update creates a default transform when the map has none.
                let transform = self.texture_transforms[texturing][map].get_or_insert_with(NifTextureTransform::default);
                match member {
                    TextureTransformMember::TranslateU => transform.translate[0] = value,
                    TextureTransformMember::TranslateV => transform.translate[1] = value,
                    TextureTransformMember::Rotate => transform.rotate = value,
                    TextureTransformMember::ScaleU => transform.scale[0] = value,
                    TextureTransformMember::ScaleV => transform.scale[1] = value,
                }
            }
            NifControllerKind::Morph { mesh, weights } => {
                let Some(scaled) = clock.tick(timing, timing.cycle, timing.range(), time) else { return };
                let mesh = *mesh;
                let Some(current) = self.morph_weights.get_mut(mesh) else { return };
                for (weight, interp) in current.iter_mut().zip(weights) {
                    if let Some(value) = interp.as_ref().and_then(|interp| interp.eval(scaled)) {
                        *weight = value;
                    }
                }
                self.morph(desc, mesh);
            }
            NifControllerKind::Unsupported { .. } => {}
        }
    }

    /// `NiMorphingKernel`: the sum of the targets by weight; relative targets weigh the base (target 0) 1.
    fn morph(&mut self, desc: &NifEffectDesc, mesh: usize) {
        let Some(morph) = desc.meshes[mesh].morph.as_ref() else { return };
        let weights = &self.morph_weights[mesh];
        let count = desc.meshes[mesh].geometry.positions.len();
        let mut positions = vec![[0.0f32; 3]; count];
        for (index, target) in morph.targets.iter().enumerate() {
            let weight = if morph.relative_targets && index == 0 { 1.0 } else { weights[index] };
            if weight.abs() < MORPH_WEIGHT_EPSILON {
                continue;
            }
            for (out, vertex) in positions.iter_mut().zip(&target.vertices) {
                for k in 0..3 {
                    out[k] += vertex[k] * weight;
                }
            }
        }
        self.morphed[mesh] = Some(positions.into());
    }

    /// `NiParticleSystem::UpdateSystem` of system `index`, then the spawns it sent to other systems.
    fn update_system(&mut self, desc: &NifEffectDesc, index: usize, time: f32, rng: &mut impl CrtRng) {
        if index >= self.systems.len() {
            return;
        }
        let mut system = std::mem::take(&mut self.systems[index]);
        let mut deferred = Vec::new();
        system.update(&self.context(desc, index), time, rng, &mut deferred);
        self.systems[index] = system;
        for spawn in deferred {
            self.apply_deferred_spawn(desc, &spawn, rng);
        }
    }

    /// A spawn into another particle system: the particle moves into the target's space.
    fn apply_deferred_spawn(&mut self, desc: &NifEffectDesc, spawn: &DeferredSpawn, rng: &mut impl CrtRng) {
        let (Some(source), Some(target)) =
            (desc.particle_systems.get(spawn.source_system), desc.particle_systems.get(spawn.target_system))
        else {
            return;
        };
        let to_target = self.nodes[target.node].world.inverse().mul(&self.nodes[source.node].world);
        let mut particle = spawn.particle;
        particle.position = to_target.point(particle.position);
        particle.velocity = to_target.rotation.mul_vec3(particle.velocity);
        let system = &mut self.systems[spawn.target_system];
        let Some(NifModifierKind::Spawn(params)) = system.modifiers.get(spawn.modifier).map(|m| m.kind.clone()) else {
            return;
        };
        spawn_particles(&params, &particle, spawn.time, spawn.spawn_time, &mut system.pool, &system.modifiers, target, rng);
    }

    fn emit(&mut self, desc: &NifEffectDesc, index: usize, emitter: usize, emissions: &[Emission], rng: &mut impl CrtRng) {
        if emissions.is_empty() {
            return;
        }
        let mut system = std::mem::take(&mut self.systems[index]);
        {
            let ctx = self.context(desc, index);
            let mut ages = Vec::new();
            for emission in emissions {
                emission.ages(&mut ages);
                if !ages.is_empty() {
                    system.emit(&ctx, emitter, emission.time, &ages, rng);
                }
            }
        }
        self.systems[index] = system;
    }

    fn context<'a>(&'a self, desc: &'a NifEffectDesc, index: usize) -> SystemContext<'a> {
        SystemContext {
            index,
            desc: &desc.particle_systems[index],
            nodes: &self.nodes,
            meshes: &desc.meshes,
            mesh_positions: &self.morphed,
        }
    }
}

/// `NiPSysModifierFloatCtlr` targets.
fn set_modifier_param(kind: &mut NifModifierKind, param: ModifierParam, value: f32) {
    match (kind, param) {
        (NifModifierKind::Emitter(emitter), ModifierParam::EmitterSpeed) => emitter.speed = value,
        (NifModifierKind::Emitter(emitter), ModifierParam::EmitterLifeSpan) => emitter.life_span = value,
        (NifModifierKind::Emitter(emitter), ModifierParam::EmitterInitialRadius) => emitter.initial_radius = value,
        (NifModifierKind::Gravity { strength, .. }, ModifierParam::GravityStrength) => *strength = value,
        (NifModifierKind::Rotation { initial_speed, .. }, ModifierParam::InitialRotationSpeed) => {
            *initial_speed = value
        }
        (NifModifierKind::Rotation { initial_speed_var, .. }, ModifierParam::InitialRotationSpeedVar) => {
            *initial_speed_var = value
        }
        _ => {}
    }
}

/// `NiTransformController::Update`: each valid channel replaces the node's local one. A path interpolator
/// sets the translation, and the rotation with its follow flag (`PathInterp::eval`).
fn apply_transform(interp: &NifInterpolator, time: f32, local: &mut Xf) {
    match interp {
        NifInterpolator::Transform(transform) => {
            let translation = transform.translation.as_ref().and_then(|c| c.eval(time)).or(transform.translation_pose);
            if let Some(translation) = translation {
                local.translation = Vec3::from_array(translation);
            }
            let rotation = transform
                .rotation
                .as_ref()
                .and_then(|curve| curve.eval(time))
                .or(transform.rotation_pose.map(Quat::from_array));
            if let Some(rotation) = rotation {
                local.rotation = rotation.to_mat3();
            }
            if let Some(scale) = transform.scale.as_ref().and_then(|c| c.eval(time)).or(transform.scale_pose) {
                local.scale = scale;
            }
        }
        NifInterpolator::Path(path) => {
            let Some(pose) = path.eval(time) else { return };
            local.translation = pose.translation;
            if let Some(rotation) = pose.rotation {
                local.rotation = rotation;
            }
        }
        NifInterpolator::Point3(point) => {
            if let Some(position) = point.eval(time) {
                local.translation = Vec3::from_array(position);
            }
        }
        NifInterpolator::Float(_) | NifInterpolator::Bool(_) => {}
    }
}

/// `NiMatrix3::MakeRotation(angle, axis)`.
pub(crate) fn make_rotation(angle: f32, axis: Vec3) -> Mat3 {
    let (s, c) = angle.sin_cos();
    let omc = 1.0 - c;
    let Vec3 { x, y, z } = axis;
    Mat3::from_rows([
        [x * x * omc + c, x * y * omc + z * s, x * z * omc - y * s],
        [x * y * omc - z * s, y * y * omc + c, y * z * omc + x * s],
        [x * z * omc + y * s, y * z * omc - x * s, z * z * omc + c],
    ])
}

/// The camera axes (toward-camera D, up U, right R) for the `*_FACE_CENTER` modes (`RotateToCenter`): turned so
/// that D points from the billboard to the camera. `None` when the camera is at the billboard.
fn center_axes(camera: &CameraFrame, center: Vec3) -> Option<(Vec3, Vec3, Vec3)> {
    let at = camera.position - center;
    if at.dot(at) < BILLBOARD_CENTER_EPSILON {
        return None;
    }
    let at = at.normalize_or_zero();
    let (d, u, r) = (-camera.direction, camera.up, camera.right);
    let dot = at.dot(d);
    if dot < BILLBOARD_CENTER_DOT {
        let rotation = make_rotation(dot.clamp(-1.0, 1.0).acos(), at.cross(d).normalize_or_zero());
        return Some((at, rotation.mul_vec3(u), rotation.mul_vec3(r)));
    }
    Some((d, u, r))
}

/// `NiBillboardNode::RotateToCamera`: the model-space face matrix that `world.rotation` is multiplied by.
fn billboard_face(mode: BillboardMode, world: &Xf, camera: &CameraFrame) -> Mat3 {
    let to_model = transpose(&world.rotation);
    let camera_axes = || Some((-camera.direction, camera.up, camera.right));
    match mode {
        BillboardMode::AlwaysFaceCamera | BillboardMode::AlwaysFaceCenter => {
            let axes = if mode == BillboardMode::AlwaysFaceCenter {
                center_axes(camera, world.translation)
            } else {
                camera_axes()
            };
            let Some((d, u, r)) = axes else { return Mat3::IDENTITY };
            let (d, u, r) = (to_model.mul_vec3(d), to_model.mul_vec3(u), to_model.mul_vec3(r));
            let root = (r.y * r.y + u.y * u.y).sqrt();
            if root > BILLBOARD_ROOT_EPSILON {
                let (cos, sin) = (u.y / root, -r.y / root);
                Mat3::from_cols(r * cos + u * sin, r * -sin + u * cos, d)
            } else {
                Mat3::from_cols(-r, -u, d)
            }
        }
        BillboardMode::RotateAboutUp => {
            let local = to_model.mul_vec3(camera.position - world.translation) * (1.0 / world.scale);
            let length = (local.x * local.x + local.z * local.z).sqrt();
            if length < BILLBOARD_UP_EPSILON {
                return Mat3::IDENTITY;
            }
            let (x, z) = (local.x / length, local.z / length);
            Mat3::from_cols(Vec3::new(z, 0.0, -x), Vec3::Y, Vec3::new(x, 0.0, z))
        }
        BillboardMode::RigidFaceCamera | BillboardMode::RigidFaceCenter => {
            let axes = if mode == BillboardMode::RigidFaceCenter {
                center_axes(camera, world.translation)
            } else {
                camera_axes()
            };
            let Some((d, u, r)) = axes else { return Mat3::IDENTITY };
            Mat3::from_cols(to_model.mul_vec3(r), to_model.mul_vec3(u), to_model.mul_vec3(d))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    #[test]
    fn transforms_compose_and_invert() {
        let a = Xf { rotation: make_rotation(0.7, Vec3::Z), translation: Vec3::new(1.0, 2.0, 3.0), scale: 2.0 };
        let b = Xf { rotation: make_rotation(-0.3, Vec3::X), translation: Vec3::new(-1.0, 0.5, 0.0), scale: 0.5 };
        let p = Vec3::new(0.3, -0.2, 0.9);
        let composed = a.mul(&b).point(p);
        assert!((composed - a.point(b.point(p))).length() < EPSILON);
        let back = a.inverse().point(a.point(p));
        assert!((back - p).length() < EPSILON);
    }

    #[test]
    fn always_face_camera_billboards_face_the_viewer() {
        let camera = CameraFrame { position: Vec3::new(0.0, -10.0, 0.0), direction: Vec3::Y, up: Vec3::Z, right: Vec3::X };
        let world = Xf::IDENTITY;
        let face = billboard_face(BillboardMode::AlwaysFaceCamera, &world, &camera);
        // The billboard's z axis points to the camera.
        assert!((face.col(2) - (-Vec3::Y)).length() < EPSILON);
    }
}
