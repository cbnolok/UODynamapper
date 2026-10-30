//! Controller evaluation and sampling into glTF animation channels.
//!
//! Every controller is evaluated as the client runs it: the effect time `t` (seconds from `Start`) becomes the
//! scaled time `phase + frequency * t`, wrapped or clamped into `[start_time, stop_time]` by the cycle type and
//! mirrored by the direction flag (particle-sim's `ControllerClock::scaled_time`, `ComputeScaledTime`), then the
//! interpolator gives the value (particle-sim's key evaluation: linear, Bezier, TCB, step and Euler keys, and
//! `NiPathInterpolator`). Controllers run in update order and each valid channel replaces the previous value,
//! as in `NiTransformController::Update`.
//!
//! Sampling: the values are written as LINEAR keys at [`SAMPLE_RATE_HZ`] (by default) over the clip, plus the times at which
//! the scaled time reaches a key (when the time map is monotonic), so linear keys are reproduced exactly and
//! curved keys within the sampling error. Visibility becomes the node scale: 0 while `APP_CULLED`, written as
//! STEP keys when only visibility moves the scale, otherwise as LINEAR keys with an extra sample
//! [`STEP_EDGE_SECONDS`] before each change. Channels whose samples never change are dropped (the node's static
//! value already holds the value at time 0, the state `UOParticleEffectNif::Init`'s `Update(0)` leaves).

use particle_sim::math::{Mat3, Vec3};
use particle_sim::nif::desc::{
    AV_FLAG_APP_CULLED, ControllerTarget, ControllerTiming, CycleType, MaterialColorField, NifControllerKind,
    NifInterpolator, NifMaterial, NifTextureTransform, TextureTransformMember,
};
use particle_sim::nif::keys::{ControllerClock, Quat};
use particle_sim::NifEffectDesc;
use serde_json::{Value, json};

use crate::glb::Builder;

/// Sampling rate of the animation channels, samples per second.
pub const SAMPLE_RATE_HZ: f32 = 60.0;
/// A visibility change is made sharp in a LINEAR channel by a sample this long before it, seconds.
pub const STEP_EDGE_SECONDS: f32 = 1e-4;
/// Two sample times closer than this are one.
pub const TIME_EPSILON: f32 = 1e-5;
/// Samples that differ by less than this make a constant channel.
pub const VALUE_EPSILON: f32 = 1e-6;
/// `NiPathInterpolator` flag: the node turns along the path (so the path also drives the rotation).
const PATH_FLAG_FOLLOW: u16 = 0x20;
/// Most key repetitions added for a looping controller (guards against tiny spans).
const MAX_KEY_REPEATS: usize = 4096;

/// A node's local transform: `p' = translation + scale * rotation * p`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalTransform {
    pub translation: Vec3,
    pub rotation: Mat3,
    pub scale: f32,
}

/// The scaled time of a controller at effect time `t`.
pub fn scaled_time(timing: &ControllerTiming, t: f32) -> f32 {
    ControllerClock::default().scaled_time(timing, timing.cycle, timing.range(), t)
}

/// Whether a controller runs at all: active and owned by a scene node.
fn runs(desc: &NifEffectDesc, index: usize) -> bool {
    let controller = &desc.controllers[index];
    controller.timing.active && controller.owner.is_some()
}

/// The controllers that run and target `target`, in update order.
pub fn controllers_on(desc: &NifEffectDesc, target: ControllerTarget) -> impl Iterator<Item = usize> + '_ {
    (0..desc.controllers.len()).filter(move |&index| runs(desc, index) && desc.controllers[index].target == target)
}

/// What the controllers of a node move.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodeChannels {
    pub translation: bool,
    pub rotation: bool,
    pub scale: bool,
    pub visibility: bool,
}

pub fn node_channels(desc: &NifEffectDesc, node: usize) -> NodeChannels {
    let mut channels = NodeChannels::default();
    for index in controllers_on(desc, ControllerTarget::Node(node)) {
        match &desc.controllers[index].kind {
            NifControllerKind::Transform(NifInterpolator::Transform(interp)) => {
                channels.translation |= interp.translation.is_some() || interp.translation_pose.is_some();
                channels.rotation |= interp.rotation.is_some() || interp.rotation_pose.is_some();
                channels.scale |= interp.scale.is_some() || interp.scale_pose.is_some();
            }
            NifControllerKind::Transform(NifInterpolator::Path(path)) => {
                channels.translation = true;
                channels.rotation |= path.flags & PATH_FLAG_FOLLOW != 0;
            }
            NifControllerKind::Transform(NifInterpolator::Point3(_)) => channels.translation = true,
            NifControllerKind::Transform(_) => {}
            NifControllerKind::Visibility(_) => channels.visibility = true,
            _ => {}
        }
    }
    channels
}

/// The local transform and visibility of node `node` at effect time `t`, starting from `base` (the stored
/// transform; identity for the scene root, whose transform the client replaces by the effect placement).
pub fn node_state(desc: &NifEffectDesc, node: usize, base: LocalTransform, t: f32) -> (LocalTransform, bool) {
    let mut local = base;
    let mut flags = desc.nodes[node].flags;
    for index in controllers_on(desc, ControllerTarget::Node(node)) {
        let controller = &desc.controllers[index];
        let time = scaled_time(&controller.timing, t);
        match &controller.kind {
            NifControllerKind::Transform(interp) => apply_transform(interp, time, &mut local),
            NifControllerKind::Visibility(interp) => {
                if let Some(visible) = interp.eval(time) {
                    if visible {
                        flags &= !AV_FLAG_APP_CULLED;
                    } else {
                        flags |= AV_FLAG_APP_CULLED;
                    }
                }
            }
            _ => {}
        }
    }
    (local, flags & AV_FLAG_APP_CULLED == 0)
}

/// `NiTransformController::Update`: each valid channel replaces the local one.
fn apply_transform(interp: &NifInterpolator, time: f32, local: &mut LocalTransform) {
    match interp {
        NifInterpolator::Transform(transform) => {
            if let Some(translation) =
                transform.translation.as_ref().and_then(|curve| curve.eval(time)).or(transform.translation_pose)
            {
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
            if let Some(scale) = transform.scale.as_ref().and_then(|curve| curve.eval(time)).or(transform.scale_pose) {
                local.scale = scale;
            }
        }
        NifInterpolator::Path(path) => {
            if let Some(pose) = path.eval(time) {
                local.translation = pose.translation;
                if let Some(rotation) = pose.rotation {
                    local.rotation = rotation;
                }
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

/// Material `material` at effect time `t` (alpha and colour controllers applied in update order).
pub fn material_state(desc: &NifEffectDesc, material: usize, t: f32) -> NifMaterial {
    let mut state = desc.materials[material].clone();
    for index in controllers_on(desc, ControllerTarget::Material(material)) {
        let controller = &desc.controllers[index];
        let time = scaled_time(&controller.timing, t);
        match &controller.kind {
            NifControllerKind::Alpha(interp) => {
                if let Some(alpha) = interp.eval(time) {
                    state.alpha = alpha;
                }
            }
            NifControllerKind::MaterialColor { field, interp } => {
                if let Some(color) = interp.eval(time) {
                    match field {
                        MaterialColorField::Ambient => state.ambient = color,
                        MaterialColorField::Diffuse => state.diffuse = color,
                        MaterialColorField::Specular => state.specular = color,
                        MaterialColorField::Emissive => state.emissive = color,
                    }
                }
            }
            _ => {}
        }
    }
    state
}

/// The transform of map `map` of texturing property `texturing` at effect time `t`;
/// `NiTextureTransformController::Update` creates a default transform for a map without one.
pub fn texture_transform_state(desc: &NifEffectDesc, texturing: usize, map: usize, t: f32) -> Option<NifTextureTransform> {
    let map_desc = &desc.texturings[texturing].maps[map];
    let mut transform = map_desc.transform;
    for index in controllers_on(desc, ControllerTarget::Texturing(texturing)) {
        let controller = &desc.controllers[index];
        let NifControllerKind::TextureTransform { slot, member, interp } = &controller.kind else { continue };
        if *slot != map_desc.slot {
            continue;
        }
        let Some(value) = interp.eval(scaled_time(&controller.timing, t)) else { continue };
        let transform = transform.get_or_insert_with(NifTextureTransform::default);
        match member {
            TextureTransformMember::TranslateU => transform.translate[0] = value,
            TextureTransformMember::TranslateV => transform.translate[1] = value,
            TextureTransformMember::Rotate => transform.rotate = value,
            TextureTransformMember::ScaleU => transform.scale[0] = value,
            TextureTransformMember::ScaleV => transform.scale[1] = value,
        }
    }
    transform
}

/// Whether any running controller animates texturing property `texturing`'s map in `slot`.
pub fn texture_transform_animated(desc: &NifEffectDesc, texturing: usize, map: usize) -> bool {
    let slot = desc.texturings[texturing].maps[map].slot;
    controllers_on(desc, ControllerTarget::Texturing(texturing))
        .any(|index| matches!(&desc.controllers[index].kind, NifControllerKind::TextureTransform { slot: s, .. } if *s == slot))
}

/// The morph weights of mesh `mesh` at effect time `t` (initial weights, then its morph controllers).
pub fn morph_weights(desc: &NifEffectDesc, mesh: usize, t: f32) -> Vec<f32> {
    let Some(morph) = desc.meshes[mesh].morph.as_ref() else { return Vec::new() };
    let mut weights: Vec<f32> = morph.targets.iter().map(|target| target.initial_weight).collect();
    for (index, controller) in desc.controllers.iter().enumerate() {
        let NifControllerKind::Morph { mesh: target, weights: interps } = &controller.kind else { continue };
        if *target != mesh || !runs(desc, index) {
            continue;
        }
        let time = scaled_time(&controller.timing, t);
        for (weight, interp) in weights.iter_mut().zip(interps) {
            if let Some(value) = interp.as_ref().and_then(|interp| interp.eval(time)) {
                *weight = value;
            }
        }
    }
    weights
}

/// Whether a running morph controller targets mesh `mesh`.
pub fn morph_animated(desc: &NifEffectDesc, mesh: usize) -> bool {
    desc.controllers.iter().enumerate().any(|(index, controller)| {
        matches!(&controller.kind, NifControllerKind::Morph { mesh: target, .. } if *target == mesh) && runs(desc, index)
    })
}

/// The key times of a controller's interpolator, in scaled time.
fn key_times(kind: &NifControllerKind) -> Vec<f32> {
    let mut times = Vec::new();
    let mut add = |keys: &mut dyn Iterator<Item = f32>| times.extend(keys);
    match kind {
        NifControllerKind::Transform(NifInterpolator::Transform(interp)) => {
            if let Some(curve) = &interp.translation {
                add(&mut curve.keys.iter().map(|key| key.time));
            }
            if let Some(curve) = &interp.scale {
                add(&mut curve.keys.iter().map(|key| key.time));
            }
            match &interp.rotation {
                Some(particle_sim::nif::desc::RotationCurve::Quaternion { keys, .. }) => {
                    add(&mut keys.iter().map(|key| key.time))
                }
                Some(particle_sim::nif::desc::RotationCurve::Euler { axes }) => {
                    for curve in axes.iter().flatten() {
                        add(&mut curve.keys.iter().map(|key| key.time));
                    }
                }
                None => {}
            }
        }
        NifControllerKind::Transform(NifInterpolator::Point3(interp)) => {
            if let Some(curve) = &interp.keys {
                add(&mut curve.keys.iter().map(|key| key.time));
            }
        }
        NifControllerKind::Transform(NifInterpolator::Path(path)) => {
            if let Some(curve) = &path.percent {
                add(&mut curve.keys.iter().map(|key| key.time));
            }
        }
        NifControllerKind::Visibility(interp) => add(&mut interp.keys.iter().map(|key| key.time)),
        NifControllerKind::Alpha(interp) | NifControllerKind::TextureTransform { interp, .. } => {
            if let Some(curve) = &interp.keys {
                add(&mut curve.keys.iter().map(|key| key.time));
            }
        }
        NifControllerKind::MaterialColor { interp, .. } => {
            if let Some(curve) = &interp.keys {
                add(&mut curve.keys.iter().map(|key| key.time));
            }
        }
        NifControllerKind::Morph { weights, .. } => {
            for curve in weights.iter().flatten().filter_map(|interp| interp.keys.as_ref()) {
                add(&mut curve.keys.iter().map(|key| key.time));
            }
        }
        _ => {}
    }
    times
}

/// The effect times in `[0, clip]` at which controller `index` reaches its keys, when its time map is
/// monotonic (positive frequency, no reverse cycle or direction): `t = (key - phase) / frequency`, repeated every
/// `span / frequency` for a looping controller.
pub fn controller_key_times(desc: &NifEffectDesc, index: usize, clip: f32) -> Vec<f32> {
    let controller = &desc.controllers[index];
    let timing = &controller.timing;
    if timing.frequency <= 0.0 || timing.reverse_direction || timing.cycle == CycleType::Reverse {
        return Vec::new();
    }
    let span = timing.stop_time - timing.start_time;
    let period = span / timing.frequency;
    let repeats = if timing.cycle == CycleType::Loop && period > 0.0 {
        ((clip / period).ceil() as usize + 1).min(MAX_KEY_REPEATS)
    } else {
        1
    };
    let mut times = Vec::new();
    for key in key_times(&controller.kind) {
        if key < timing.start_time || key > timing.stop_time {
            continue;
        }
        let first = (key - timing.phase) / timing.frequency;
        for repeat in 0..repeats {
            let t = first + repeat as f32 * period;
            if (0.0..=clip).contains(&t) {
                times.push(t);
            }
        }
    }
    times
}

/// The sample times of a clip: the uniform grid at `rate` samples per second, plus `extra` times within the
/// clip; sorted, without near-duplicates.
pub fn sample_times(clip: f32, rate: f32, extra: impl IntoIterator<Item = f32>) -> Vec<f32> {
    let steps = (clip * rate).ceil().max(1.0) as usize;
    let mut times: Vec<f32> = (0..=steps).map(|step| (step as f32 / rate).min(clip)).collect();
    times.extend(extra.into_iter().filter(|t| (0.0..=clip).contains(t)));
    times.sort_by(f32::total_cmp);
    times.dedup_by(|later, earlier| *later - *earlier < TIME_EPSILON);
    times
}

/// Whether a list of sample values is constant.
pub fn is_constant<const N: usize>(values: &[[f32; N]]) -> bool {
    values.windows(2).all(|pair| pair[0].iter().zip(&pair[1]).all(|(a, b)| (a - b).abs() < VALUE_EPSILON))
}

/// A rotation matrix as a glTF quaternion (x, y, z, w) and the sign to give the scale: a reflection (negative
/// determinant) is written as the rotation `-R` with a negated scale (`s R = (-s)(-R)`).
pub fn matrix_to_quaternion(m: &Mat3) -> ([f32; 4], f32) {
    let r = m.rows;
    let det = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1]) - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
        + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
    let sign = if det < 0.0 { -1.0 } else { 1.0 };
    let a = r.map(|row| row.map(|value| value * sign));
    let trace = a[0][0] + a[1][1] + a[2][2];
    let (x, y, z, w);
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        w = 0.25 * s;
        x = (a[2][1] - a[1][2]) / s;
        y = (a[0][2] - a[2][0]) / s;
        z = (a[1][0] - a[0][1]) / s;
    } else if a[0][0] > a[1][1] && a[0][0] > a[2][2] {
        let s = (1.0 + a[0][0] - a[1][1] - a[2][2]).sqrt() * 2.0;
        w = (a[2][1] - a[1][2]) / s;
        x = 0.25 * s;
        y = (a[0][1] + a[1][0]) / s;
        z = (a[0][2] + a[2][0]) / s;
    } else if a[1][1] > a[2][2] {
        let s = (1.0 + a[1][1] - a[0][0] - a[2][2]).sqrt() * 2.0;
        w = (a[0][2] - a[2][0]) / s;
        x = (a[0][1] + a[1][0]) / s;
        y = 0.25 * s;
        z = (a[1][2] + a[2][1]) / s;
    } else {
        let s = (1.0 + a[2][2] - a[0][0] - a[1][1]).sqrt() * 2.0;
        w = (a[1][0] - a[0][1]) / s;
        x = (a[0][2] + a[2][0]) / s;
        y = (a[1][2] + a[2][1]) / s;
        z = 0.25 * s;
    }
    let length = (x * x + y * y + z * z + w * w).sqrt();
    if !(length.is_finite() && length > 0.0) {
        return ([0.0, 0.0, 0.0, 1.0], sign);
    }
    ([x / length, y / length, z / length, w / length], sign)
}

/// Quaternions on one hemisphere, so that LINEAR (slerp) keys take the short way.
pub fn align_quaternions(quaternions: &mut [[f32; 4]]) {
    for index in 1..quaternions.len() {
        let previous = quaternions[index - 1];
        let current = &mut quaternions[index];
        let dot: f32 = previous.iter().zip(current.iter()).map(|(a, b)| a * b).sum();
        if dot < 0.0 {
            *current = current.map(|value| -value);
        }
    }
}

/// The sample times of one part of an export: clip times and the part's own effect time at each.
#[derive(Debug, Clone, Default)]
pub struct Timeline {
    /// Seconds from the start of the clip, strictly increasing.
    pub times: Vec<f32>,
    /// The part's effect time at each sample.
    pub local: Vec<f32>,
}

impl Timeline {
    /// The samples of a part of `desc` over `clip` seconds; `period`: the part's clock wraps every `period`
    /// seconds (a looping effect with a duration). Empty when the clip is empty.
    pub fn new(desc: &NifEffectDesc, clip: f32, period: Option<f32>, rate: f32) -> Self {
        if !(clip > 0.0) {
            return Self::default();
        }
        let period = period.filter(|&period| period > 0.0 && period < clip);
        let horizon = period.unwrap_or(clip);
        let mut local_keys = Vec::new();
        for index in 0..desc.controllers.len() {
            if !runs(desc, index) {
                continue;
            }
            for key in controller_key_times(desc, index, horizon) {
                local_keys.push(key);
                // Steps (visibility, step keys) stay sharp under LINEAR interpolation.
                local_keys.push(key - STEP_EDGE_SECONDS);
            }
        }
        let mut extra = Vec::new();
        match period {
            None => extra.extend(local_keys),
            Some(period) => {
                let cycles = ((clip / period).ceil() as usize).min(MAX_KEY_REPEATS);
                for cycle in 0..=cycles {
                    let start = cycle as f32 * period;
                    extra.extend(local_keys.iter().filter(|&&key| key < period).map(|key| start + key));
                    // The wrap: the last state of a cycle just before the first state of the next one.
                    extra.push(start - STEP_EDGE_SECONDS);
                    extra.push(start);
                }
            }
        }
        let times = sample_times(clip, rate, extra);
        let local = times
            .iter()
            .map(|&t| match period {
                Some(period) => {
                    // A sample within TIME_EPSILON of a wrap belongs to the new cycle (rounding of `n * period`);
                    // the edge sample STEP_EDGE_SECONDS before it to the ending one.
                    let cycle = ((t + TIME_EPSILON) / period).floor();
                    (t - cycle * period).max(0.0)
                }
                None => t,
            })
            .collect();
        Self { times, local }
    }

    pub fn is_empty(&self) -> bool {
        self.times.len() < 2
    }
}

/// glTF interpolation of a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
}

impl Interpolation {
    fn name(self) -> &'static str {
        match self {
            Self::Linear => "LINEAR",
            Self::Step => "STEP",
        }
    }
}

/// The glTF accessor type of an element of `components` floats.
fn element_type(components: usize) -> &'static str {
    match components {
        1 => "SCALAR",
        2 => "VEC2",
        3 => "VEC3",
        _ => "VEC4",
    }
}

/// The samplers and channels of one glTF animation, shared by every part of an export.
#[derive(Debug, Default)]
pub struct AnimationBuilder {
    samplers: Vec<Value>,
    channels: Vec<Value>,
    /// Input accessors by their time list, so that channels of one timeline share their input.
    inputs: Vec<(Vec<f32>, usize)>,
}

impl AnimationBuilder {
    fn input(&mut self, builder: &mut Builder, times: &[f32]) -> usize {
        if let Some((_, accessor)) = self.inputs.iter().find(|(known, _)| known == times) {
            return *accessor;
        }
        let accessor = builder.scalars(times);
        self.inputs.push((times.to_vec(), accessor));
        accessor
    }

    fn sampler(
        &mut self,
        builder: &mut Builder,
        times: &[f32],
        values: &[f32],
        components: usize,
        interpolation: Interpolation,
    ) -> usize {
        let input = self.input(builder, times);
        let output = builder.floats(values, element_type(components), components);
        self.samplers.push(json!({ "input": input, "output": output, "interpolation": interpolation.name() }));
        self.samplers.len() - 1
    }

    /// A channel of node `node`'s `path` (translation, rotation, scale, weights); `values` holds `components`
    /// floats per time (per time and morph target for weights).
    pub fn node_channel(
        &mut self,
        builder: &mut Builder,
        node: usize,
        path: &str,
        times: &[f32],
        values: &[f32],
        components: usize,
        interpolation: Interpolation,
    ) {
        let sampler = self.sampler(builder, times, values, components, interpolation);
        self.channels.push(json!({ "sampler": sampler, "target": { "node": node, "path": path } }));
    }

    /// A `KHR_animation_pointer` channel of the JSON pointer `pointer`.
    pub fn pointer_channel(
        &mut self,
        builder: &mut Builder,
        pointer: String,
        times: &[f32],
        values: &[f32],
        components: usize,
        interpolation: Interpolation,
    ) {
        builder.extensions_used.insert(EXT_ANIMATION_POINTER);
        let sampler = self.sampler(builder, times, values, components, interpolation);
        self.channels.push(json!({
            "sampler": sampler,
            "target": { "path": "pointer", "extensions": { EXT_ANIMATION_POINTER: { "pointer": pointer } } },
        }));
    }

    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// The animation, when it has channels.
    pub fn finish(self, name: &str, extras: Value) -> Option<Value> {
        (!self.channels.is_empty())
            .then(|| json!({ "name": name, "samplers": self.samplers, "channels": self.channels, "extras": extras }))
    }
}

/// The animation pointer extension (material and texture transform animation).
pub const EXT_ANIMATION_POINTER: &str = "KHR_animation_pointer";

/// Flattens `[f32; N]` samples.
pub fn flatten<const N: usize>(values: &[[f32; N]]) -> Vec<f32> {
    values.iter().flatten().copied().collect()
}
