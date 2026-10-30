//! The Gamebryo data glTF cannot express, as JSON for `extras`: render state, materials and texture maps,
//! texture effects, lights, particle systems with their emitters and modifiers, and controllers with their
//! timing and keys as stored (key type, times, values, tangents, TCB parameters).
//!
//! Enum values are written by their particle-sim variant names; indices refer to the [`NifEffectDesc`] tables
//! (nodes by their desc index, which [`crate`] also stores in each node's extras).

use std::fmt::Debug;

use particle_sim::nif::desc::{
    BoolInterp, ControllerTarget, ControllerTiming, Curve, EmitterShape, FloatInterp, Key, KeyKind, NifAlpha,
    NifController, NifControllerKind, NifEmitter, NifInterpolator, NifLight, NifLightKind, NifMaterial, NifModifier,
    NifModifierKind, NifParticleSystem, NifRenderState, NifTextureEffect, NifTextureMap, NifTextureSource,
    NifTextureTransform, NifTexturing, NifZBuffer, Point3Interp, RotationCurve, TextureFilter, TransformInterp,
};
use particle_sim::render::BlendFactor;
use particle_sim::NifEffectDesc;
use serde_json::{Value, json};

/// The variant name of a field-less enum.
pub fn name(value: impl Debug) -> Value {
    json!(format!("{value:?}"))
}

/// Floats for JSON: non-finite values (which JSON cannot hold) become their text.
pub fn float(value: f32) -> Value {
    if value.is_finite() { json!(value) } else { json!(value.to_string()) }
}

/// A blend factor with its Gamebryo value.
fn blend_factor(factor: BlendFactor) -> Value {
    json!({ "name": name(factor), "value": factor as u8 })
}

pub fn alpha(alpha: &NifAlpha) -> Value {
    json!({
        "blend": alpha.blend,
        "src_blend": blend_factor(alpha.src),
        "dst_blend": blend_factor(alpha.dst),
        "alpha_test": alpha.test,
        "alpha_test_function": name(alpha.test_function),
        "alpha_test_reference": alpha.test_reference,
        "no_sorter": alpha.no_sorter,
    })
}

pub fn z_buffer(z: &NifZBuffer) -> Value {
    json!({ "test": z.test, "write": z.write, "function": name(z.function) })
}

pub fn material(material: &NifMaterial) -> Value {
    json!({
        "name": material.name,
        "ambient": material.ambient,
        "diffuse": material.diffuse,
        "specular": material.specular,
        "emissive": material.emissive,
        "shine": float(material.shine),
        "alpha": float(material.alpha),
    })
}

pub fn texture_transform(transform: &NifTextureTransform) -> Value {
    json!({
        "translate": transform.translate,
        "scale": transform.scale,
        "rotate": float(transform.rotate),
        "method": name(transform.method),
        "center": transform.center,
    })
}

fn filter(raw: u32) -> Value {
    let TextureFilter { mag, min, mip } = TextureFilter::from_raw(raw);
    json!({ "mode": raw, "mag": name(mag), "min": name(min), "mip": name(mip) })
}

/// The name and source of texture `index` of `desc`.
pub fn texture_reference(desc: &NifEffectDesc, index: Option<usize>) -> Value {
    let Some(texture) = index.and_then(|index| desc.textures.get(index)) else {
        return Value::Null;
    };
    let source = match &texture.source {
        NifTextureSource::External { file_name } => json!({ "external": file_name }),
        NifTextureSource::Embedded { width, height, levels } => {
            json!({ "embedded": { "width": width, "height": height, "levels": levels.len() } })
        }
        NifTextureSource::Undecoded { format } => json!({ "undecoded_pixel_format": format }),
    };
    json!({ "name": texture.name(), "source": source, "mipmapped": texture.mipmapped })
}

/// A texture map; `gltf_texture` is the glTF texture index of its image, when it has one.
pub fn texture_map(desc: &NifEffectDesc, map: &NifTextureMap, gltf_texture: Option<usize>) -> Value {
    json!({
        "slot": name(map.slot),
        "texture": texture_reference(desc, map.texture),
        "gltf_texture": gltf_texture,
        "uv_set": map.uv_set,
        "filter": filter(map.filter),
        "wrap_s": map.addressing.wrap_s,
        "wrap_t": map.addressing.wrap_t,
        "transform": map.transform.as_ref().map(texture_transform),
    })
}

/// A texturing property; `gltf_textures` gives the glTF texture of each map, in map order.
pub fn texturing(desc: &NifEffectDesc, texturing: &NifTexturing, gltf_textures: &[Option<usize>]) -> Value {
    let maps: Vec<Value> = texturing
        .maps
        .iter()
        .enumerate()
        .map(|(index, map)| texture_map(desc, map, gltf_textures.get(index).copied().flatten()))
        .collect();
    json!({ "apply_mode": name(texturing.apply_mode), "maps": maps })
}

pub fn texture_effect(desc: &NifEffectDesc, effect: &NifTextureEffect, gltf_texture: Option<usize>) -> Value {
    json!({
        "node": effect.node,
        "kind": name(effect.kind),
        "coord_gen": name(effect.coord_gen),
        "on": effect.on,
        "texture": texture_reference(desc, effect.texture),
        "gltf_texture": gltf_texture,
        "model_projection": effect.model_projection,
        "model_translation": effect.model_translation,
        "filter": filter(effect.filter),
        "wrap_s": effect.addressing.wrap_s,
        "wrap_t": effect.addressing.wrap_t,
        "unaffected_nodes": effect.unaffected_nodes,
    })
}

/// The render state of a drawable, with its material and texturing property (by table index and in full).
pub fn render_state(desc: &NifEffectDesc, state: &NifRenderState, gltf_textures: &[Option<usize>]) -> Value {
    json!({
        "alpha": alpha(&state.alpha),
        "z_buffer": z_buffer(&state.z_buffer),
        "vertex_color": { "source": name(state.vertex_color.source), "lighting": name(state.vertex_color.lighting) },
        "cull": name(state.cull),
        "specular": state.specular,
        "material_index": state.material,
        "material": state.material.and_then(|index| desc.materials.get(index)).map(material),
        "texturing_index": state.texturing,
        "texturing": state.texturing.and_then(|index| desc.texturings.get(index)).map(|t| texturing(desc, t, gltf_textures)),
    })
}

pub fn light(light: &NifLight) -> Value {
    let kind = match &light.kind {
        NifLightKind::Ambient => json!({ "type": "Ambient" }),
        NifLightKind::Directional => json!({ "type": "Directional" }),
        NifLightKind::Point { attenuation } => json!({
            "type": "Point",
            "attenuation": [attenuation.constant, attenuation.linear, attenuation.quadratic],
        }),
        NifLightKind::Spot { attenuation, outer_angle, exponent } => json!({
            "type": "Spot",
            "attenuation": [attenuation.constant, attenuation.linear, attenuation.quadratic],
            "outer_angle_degrees": float(*outer_angle),
            "exponent": float(*exponent),
        }),
    };
    json!({
        "node": light.node,
        "kind": kind,
        "on": light.on,
        "dimmer": float(light.dimmer),
        "ambient": light.ambient,
        "diffuse": light.diffuse,
        "specular": light.specular,
        "unaffected_nodes": light.unaffected_nodes,
    })
}

/// A value of a key as JSON.
pub trait KeyJson: Copy {
    fn to_json(self) -> Value;
}

impl KeyJson for f32 {
    fn to_json(self) -> Value {
        float(self)
    }
}

impl<const N: usize> KeyJson for [f32; N] {
    fn to_json(self) -> Value {
        Value::Array(self.iter().map(|&value| float(value)).collect())
    }
}

fn key<T: KeyJson>(kind: KeyKind, key: &Key<T>) -> Value {
    let mut value = json!({ "time": float(key.time), "value": key.value.to_json() });
    match kind {
        KeyKind::Bezier => {
            value["in_tangent"] = key.in_tangent.to_json();
            value["out_tangent"] = key.out_tangent.to_json();
        }
        KeyKind::Tcb => {
            value["tcb"] = json!([float(key.tcb.tension), float(key.tcb.continuity), float(key.tcb.bias)]);
        }
        KeyKind::Linear | KeyKind::Step => {}
    }
    value
}

pub fn curve<T: KeyJson>(curve: &Curve<T>) -> Value {
    json!({ "key_type": name(curve.kind), "keys": curve.keys.iter().map(|k| key(curve.kind, k)).collect::<Vec<_>>() })
}

fn optional_curve<T: KeyJson>(value: &Option<Curve<T>>) -> Value {
    value.as_ref().map_or(Value::Null, curve)
}

pub fn float_interp(interp: &FloatInterp) -> Value {
    json!({ "pose": interp.pose.map(float), "keys": optional_curve(&interp.keys) })
}

pub fn point3_interp(interp: &Point3Interp) -> Value {
    json!({ "pose": interp.pose, "keys": optional_curve(&interp.keys) })
}

pub fn bool_interp(interp: &BoolInterp) -> Value {
    let keys: Vec<Value> = interp.keys.iter().map(|key| json!({ "time": float(key.time), "value": key.value })).collect();
    json!({ "pose": interp.pose, "keys": keys })
}

fn rotation_curve(rotation: &RotationCurve) -> Value {
    match rotation {
        RotationCurve::Quaternion { kind, keys } => {
            let keys: Vec<Value> = keys
                .iter()
                .map(|key| {
                    let mut value = json!({ "time": float(key.time), "wxyz": key.value });
                    if *kind == KeyKind::Tcb {
                        value["tcb"] = json!([key.tcb.tension, key.tcb.continuity, key.tcb.bias]);
                    }
                    value
                })
                .collect();
            json!({ "type": "Quaternion", "key_type": name(kind), "keys": keys })
        }
        RotationCurve::Euler { axes } => {
            json!({ "type": "EulerXYZ", "axes": axes.iter().map(optional_curve).collect::<Vec<_>>() })
        }
    }
}

fn transform_interp(interp: &TransformInterp) -> Value {
    json!({
        "translation_pose": interp.translation_pose,
        "rotation_pose_wxyz": interp.rotation_pose,
        "scale_pose": interp.scale_pose.map(float),
        "translation": optional_curve(&interp.translation),
        "rotation": interp.rotation.as_ref().map_or(Value::Null, rotation_curve),
        "scale": optional_curve(&interp.scale),
    })
}

pub fn interpolator(interp: &NifInterpolator) -> Value {
    match interp {
        NifInterpolator::Float(interp) => json!({ "type": "Float", "data": float_interp(interp) }),
        NifInterpolator::Point3(interp) => json!({ "type": "Point3", "data": point3_interp(interp) }),
        NifInterpolator::Bool(interp) => json!({ "type": "Bool", "data": bool_interp(interp) }),
        NifInterpolator::Transform(interp) => json!({ "type": "Transform", "data": transform_interp(interp) }),
        NifInterpolator::Path(path) => json!({
            "type": "Path",
            "path": optional_curve(&path.path),
            "percent": optional_curve(&path.percent),
            "flags": path.flags,
            "follow_axis": path.follow_axis,
            "bank_dir": path.bank_dir,
            "max_bank_angle": float(path.max_bank_angle),
            "smoothing": float(path.smoothing),
        }),
    }
}

pub fn timing(timing: &ControllerTiming) -> Value {
    json!({
        "cycle": name(timing.cycle),
        "active": timing.active,
        "app_init": timing.app_init,
        "reverse_direction": timing.reverse_direction,
        "frequency": float(timing.frequency),
        "phase": float(timing.phase),
        "start_time": float(timing.start_time),
        "stop_time": float(timing.stop_time),
    })
}

fn target(target: ControllerTarget) -> Value {
    match target {
        ControllerTarget::Node(index) => json!({ "node": index }),
        ControllerTarget::Light(index) => json!({ "light": index }),
        ControllerTarget::Material(index) => json!({ "material": index }),
        ControllerTarget::Texturing(index) => json!({ "texturing": index }),
        ControllerTarget::ParticleSystem(index) => json!({ "particle_system": index }),
    }
}

/// One controller in full; `index` is its place in the update order.
pub fn controller(index: usize, controller: &NifController) -> Value {
    let kind = match &controller.kind {
        NifControllerKind::Transform(interp) => json!({ "type": "Transform", "interpolator": interpolator(interp) }),
        NifControllerKind::Visibility(interp) => json!({ "type": "Visibility", "interpolator": bool_interp(interp) }),
        NifControllerKind::Alpha(interp) => json!({ "type": "Alpha", "interpolator": float_interp(interp) }),
        NifControllerKind::MaterialColor { field, interp } => {
            json!({ "type": "MaterialColor", "field": name(field), "interpolator": point3_interp(interp) })
        }
        NifControllerKind::LightColor { ambient, interp } => {
            json!({ "type": "LightColor", "ambient": ambient, "interpolator": point3_interp(interp) })
        }
        NifControllerKind::LightDimmer(interp) => json!({ "type": "LightDimmer", "interpolator": float_interp(interp) }),
        NifControllerKind::TextureTransform { slot, member, interp } => json!({
            "type": "TextureTransform",
            "slot": name(slot),
            "member": name(member),
            "interpolator": float_interp(interp),
        }),
        NifControllerKind::Morph { mesh, weights } => json!({
            "type": "Morph",
            "mesh": mesh,
            "weights": weights.iter().map(|w| w.as_ref().map_or(Value::Null, float_interp)).collect::<Vec<_>>(),
        }),
        NifControllerKind::Emitter { modifier, birth_rate, active } => json!({
            "type": "Emitter",
            "modifier": modifier,
            "birth_rate": birth_rate.as_ref().map(float_interp),
            "active": active.as_ref().map(bool_interp),
        }),
        NifControllerKind::ModifierFloat { modifier, param, interp } => json!({
            "type": "ModifierFloat",
            "modifier": modifier,
            "param": name(param),
            "interpolator": interp.as_ref().map(float_interp),
        }),
        NifControllerKind::ModifierActive { modifier, interp } => {
            json!({ "type": "ModifierActive", "modifier": modifier, "interpolator": interp.as_ref().map(bool_interp) })
        }
        NifControllerKind::ParticleUpdate => json!({ "type": "ParticleUpdate" }),
        NifControllerKind::ResetOnLoop => json!({ "type": "ResetOnLoop" }),
        NifControllerKind::Unsupported { type_name } => json!({ "type": "Unsupported", "type_name": type_name }),
    };
    json!({
        "update_order": index,
        "owner": controller.owner,
        "target": target(controller.target),
        "timing": timing(&controller.timing),
        "kind": kind,
    })
}

/// The controllers of `desc` whose target satisfies `wanted`, in update order.
pub fn controllers_of(desc: &NifEffectDesc, wanted: impl Fn(ControllerTarget, &NifControllerKind) -> bool) -> Vec<Value> {
    desc.controllers
        .iter()
        .enumerate()
        .filter(|(_, c)| wanted(c.target, &c.kind))
        .map(|(index, c)| controller(index, c))
        .collect()
}

fn emitter(emitter: &NifEmitter) -> Value {
    let shape = match &emitter.shape {
        EmitterShape::Box { object, width, height, depth } => {
            json!({ "type": "Box", "object": object, "width": float(*width), "height": float(*height), "depth": float(*depth) })
        }
        EmitterShape::Sphere { object, radius } => json!({ "type": "Sphere", "object": object, "radius": float(*radius) }),
        EmitterShape::Cylinder { object, radius, height } => {
            json!({ "type": "Cylinder", "object": object, "radius": float(*radius), "height": float(*height) })
        }
        EmitterShape::Mesh { meshes, velocity, emission, emit_axis } => json!({
            "type": "Mesh",
            "meshes": meshes,
            "velocity": name(velocity),
            "emission": name(emission),
            "emit_axis": emit_axis,
        }),
    };
    json!({
        "speed": float(emitter.speed),
        "speed_var": float(emitter.speed_var),
        "declination": float(emitter.declination),
        "declination_var": float(emitter.declination_var),
        "planar_angle": float(emitter.planar_angle),
        "planar_angle_var": float(emitter.planar_angle_var),
        "initial_color": emitter.initial_color,
        "initial_radius": float(emitter.initial_radius),
        "radius_var": float(emitter.radius_var),
        "life_span": float(emitter.life_span),
        "life_span_var": float(emitter.life_span_var),
        "shape": shape,
    })
}

fn modifier_kind(kind: &NifModifierKind) -> Value {
    match kind {
        NifModifierKind::Emitter(data) => json!({ "type": "Emitter", "emitter": emitter(data) }),
        NifModifierKind::AgeDeath { spawn_on_death, spawn_modifier } => {
            json!({ "type": "AgeDeath", "spawn_on_death": spawn_on_death, "spawn_modifier": spawn_modifier })
        }
        NifModifierKind::Spawn(spawn) => json!({
            "type": "Spawn",
            "num_spawn_generations": spawn.num_spawn_generations,
            "percentage_spawned": float(spawn.percentage_spawned),
            "min_num_to_spawn": spawn.min_num_to_spawn,
            "max_num_to_spawn": spawn.max_num_to_spawn,
            "spawn_speed_chaos": float(spawn.spawn_speed_chaos),
            "spawn_dir_chaos": float(spawn.spawn_dir_chaos),
            "life_span": float(spawn.life_span),
            "life_span_var": float(spawn.life_span_var),
        }),
        NifModifierKind::Position => json!({ "type": "Position" }),
        NifModifierKind::BoundUpdate { skip } => json!({ "type": "BoundUpdate", "skip": skip }),
        NifModifierKind::GrowFade { grow_time, grow_generation, fade_time, fade_generation } => json!({
            "type": "GrowFade",
            "grow_time": float(*grow_time),
            "grow_generation": grow_generation,
            "fade_time": float(*fade_time),
            "fade_generation": fade_generation,
        }),
        NifModifierKind::Color { colors } => json!({ "type": "Color", "colors": optional_curve(colors) }),
        NifModifierKind::Rotation {
            initial_speed,
            initial_speed_var,
            initial_angle,
            initial_angle_var,
            random_speed_sign,
            random_initial_axis,
            initial_axis,
        } => json!({
            "type": "Rotation",
            "initial_speed": float(*initial_speed),
            "initial_speed_var": float(*initial_speed_var),
            "initial_angle": float(*initial_angle),
            "initial_angle_var": float(*initial_angle_var),
            "random_speed_sign": random_speed_sign,
            "random_initial_axis": random_initial_axis,
            "initial_axis": initial_axis,
        }),
        NifModifierKind::Gravity { object, axis, decay, strength, spherical, turbulence, turbulence_scale } => json!({
            "type": "Gravity",
            "object": object,
            "axis": axis,
            "decay": float(*decay),
            "strength": float(*strength),
            "spherical": spherical,
            "turbulence": float(*turbulence),
            "turbulence_scale": float(*turbulence_scale),
        }),
        NifModifierKind::Drag { object, axis, percentage, range, falloff } => json!({
            "type": "Drag",
            "object": object,
            "axis": axis,
            "percentage": float(*percentage),
            "range": float(*range),
            "falloff": float(*falloff),
        }),
        NifModifierKind::Bomb { object, axis, decay, delta_v, decay_type, symmetry } => json!({
            "type": "Bomb",
            "object": object,
            "axis": axis,
            "decay": float(*decay),
            "delta_v": float(*delta_v),
            "decay_type": name(decay_type),
            "symmetry": name(symmetry),
        }),
        NifModifierKind::MeshUpdate { meshes } => json!({ "type": "MeshUpdate", "meshes": meshes }),
        NifModifierKind::Unsupported { type_name } => json!({ "type": "Unsupported", "type_name": type_name }),
    }
}

fn modifier(modifier: &NifModifier) -> Value {
    json!({ "name": modifier.name, "order": modifier.order, "active": modifier.active, "kind": modifier_kind(&modifier.kind) })
}

/// A particle system: its data arrays, modifiers in update order, render state and controllers.
pub fn particle_system(
    desc: &NifEffectDesc,
    index: usize,
    system: &NifParticleSystem,
    gltf_textures: &[Option<usize>],
) -> Value {
    json!({
        "index": index,
        "mesh_particles": system.mesh_particles,
        "world_space": system.world_space,
        "capacity": system.capacity,
        "arrays": {
            "colors": system.has_colors,
            "radii": system.has_radii,
            "sizes": system.has_sizes,
            "rotation_angles": system.has_rotation_angles,
            "rotation_axes": system.has_rotation_axes,
        },
        "bound_center": system.bound_center,
        "modifiers": system.modifiers.iter().map(modifier).collect::<Vec<_>>(),
        "render_state": render_state(desc, &system.render_state, gltf_textures),
        "controllers": controllers_of(desc, |target, _| target == ControllerTarget::ParticleSystem(index)),
    })
}
