//! A compiled NIF scene ([`NifEffectDesc`]) as glTF nodes, meshes, lights and animation channels.
//!
//! - Nodes keep their names and local transforms (`p' = T + s R p`, written as glTF TRS; a reflection is a
//!   negated scale). The scene root's stored transform is replaced by the effect placement in the client, so the
//!   root node is written with the identity (its stored transform is in `extras`).
//! - Every node carries `extras.gamebryo`: its desc index, block kind, flags, extra strings, sorting, the
//!   controllers it owns, and per kind the particle system, light, texture effect or billboard description.
//! - Meshes (`NiTriShape`; the shipped files have no strips and no skin, which gamebryo-nif does not parse): one
//!   primitive with POSITION, NORMAL (normalised), COLOR_0 by material mode, TEXCOORD_n per UV set, the raw
//!   vertex colours as `_NIF_VERTEX_COLOR`, and morph targets (`NiGeomMorpherController`: relative targets
//!   become the base target 0 plus deltas, absolute targets a zero base plus every target).
//! - Lights: `KHR_lights_punctual` on a child node turned so that glTF's -Z is Gamebryo's +X (the light
//!   direction); colour = diffuse converted to linear ([`crate::material`], colour space), intensity = dimmer
//!   (unconverted: a multiplier of the linear colour). Ambient lights are in `extras` only.
//! - `APP_CULLED` nodes have scale 0 (and `app_culled` in `extras`).
//! - Mesh-particle prototypes (objects outside the scene graph) are written under a
//!   `mesh_particle_prototypes` node, visible, animated on the clip time (the client animates each clone with
//!   its particle's age).

use particle_sim::math::{Mat3, Vec3};
use particle_sim::nif::desc::{
    AV_FLAG_APP_CULLED, ControllerTarget, CullMode, NifLightKind, NifNodeKind, NifTransform,
};
use particle_sim::NifEffectDesc;
use serde_json::{Value, json};

use crate::anim::{self, AnimationBuilder, Interpolation, LocalTransform, Timeline, flatten, is_constant};
use crate::glb::{Builder, MODE_TRIANGLES, Node, TARGET_ARRAY_BUFFER};
use crate::material::{ColorMode, DISPLAY_SPACE_NOTE, linear_rgb};
use crate::texture::{TextureCache, TextureLoader};
use crate::{ExportOptions, describe};

/// The lights extension.
pub const EXT_LIGHTS_PUNCTUAL: &str = "KHR_lights_punctual";
/// The light node's rotation, x y z w: -90 degrees about Y, so the glTF light direction (-Z) is Gamebryo's +X.
pub const LIGHT_DIRECTION_ROTATION: [f32; 4] = [0.0, -std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2];
/// A normal shorter than this is replaced by [`FALLBACK_NORMAL`].
const NORMAL_EPSILON: f32 = 1e-12;
/// The normal written for a zero normal.
const FALLBACK_NORMAL: [f32; 3] = [0.0, 0.0, 1.0];
/// glTF's largest spot cone half angle, radians.
const MAX_SPOT_ANGLE: f32 = std::f32::consts::FRAC_PI_2;
/// The name of the custom attribute holding the raw vertex colours.
pub const RAW_COLOR_ATTRIBUTE: &str = "_NIF_VERTEX_COLOR";
/// The name of the node holding the mesh-particle prototypes.
pub const PROTOTYPES_NODE: &str = "mesh_particle_prototypes";

/// The state of one export.
pub struct Exporter<'a> {
    pub builder: Builder,
    pub cache: TextureCache,
    pub loader: TextureLoader<'a>,
    pub warnings: Vec<String>,
    pub animation: AnimationBuilder,
    pub options: &'a ExportOptions,
}

/// One NIF scene being exported.
pub struct Part<'d> {
    pub desc: &'d NifEffectDesc,
    /// The name used in warnings and material names.
    pub label: String,
    pub timeline: Timeline,
}

impl Part<'_> {
    /// The part's effect time at the start of the clip.
    pub fn local_start(&self) -> f32 {
        self.timeline.local.first().copied().unwrap_or(0.0)
    }
}

/// A node's stored transform as an animation base.
fn base_transform(transform: &NifTransform) -> LocalTransform {
    LocalTransform {
        translation: Vec3::from_array(transform.translation),
        rotation: Mat3::from_rows(transform.rotation),
        scale: transform.scale,
    }
}

/// The identity: the scene root's base (the client replaces it by the placement).
const IDENTITY_TRANSFORM: LocalTransform =
    LocalTransform { translation: Vec3::ZERO, rotation: Mat3::IDENTITY, scale: 1.0 };

/// glTF translation, rotation and uniform scale of a local transform, the scale 0 when hidden.
fn trs(local: &LocalTransform, visible: bool) -> ([f32; 3], [f32; 4], f32) {
    let (rotation, sign) = anim::matrix_to_quaternion(&local.rotation);
    let scale = if visible { local.scale * sign } else { 0.0 };
    (local.translation.to_array(), rotation, scale)
}

fn transform_json(transform: &NifTransform) -> Value {
    json!({ "translation": transform.translation, "rotation_rows": transform.rotation, "scale": describe::float(transform.scale) })
}

fn normalized(normal: [f32; 3]) -> [f32; 3] {
    let length_squared: f32 = normal.iter().map(|value| value * value).sum();
    if !(length_squared > NORMAL_EPSILON) || !length_squared.is_finite() {
        return FALLBACK_NORMAL;
    }
    let length = length_squared.sqrt();
    normal.map(|value| value / length)
}

impl Exporter<'_> {
    /// Exports `part`'s scene; returns the glTF nodes to place under the part's parent (the scene root, then the
    /// prototypes node when there are prototypes).
    pub fn export_nif(&mut self, part: &Part<'_>) -> Vec<usize> {
        let desc = part.desc;
        let mut roots = Vec::new();
        if let Some(root) = desc.root {
            roots.push(self.export_node(part, root, true));
        } else {
            self.warnings.push(format!("{}: no scene root", part.label));
        }
        let prototypes: Vec<usize> = (0..desc.nodes.len())
            .filter(|&index| {
                let node = &desc.nodes[index];
                !node.in_scene && node.parent.is_none_or(|parent| desc.nodes[parent].in_scene) && Some(index) != desc.root
            })
            .collect();
        if !prototypes.is_empty() {
            let children: Vec<usize> = prototypes.iter().map(|&index| self.export_node(part, index, false)).collect();
            let mut holder = Node::named(PROTOTYPES_NODE);
            holder.children = children;
            holder.extras = Some(json!({ "gamebryo": {
                "note": "Mesh-particle prototypes: objects outside the scene graph that particles clone",
            }}));
            roots.push(self.builder.add_node(holder));
        }
        roots
    }

    fn export_node(&mut self, part: &Part<'_>, index: usize, is_root: bool) -> usize {
        let desc = part.desc;
        let node = &desc.nodes[index];
        let base = if is_root { IDENTITY_TRANSFORM } else { base_transform(&node.transform) };
        let (initial, visible) = anim::node_state(desc, index, base, part.local_start());
        let (translation, rotation, scale) = trs(&initial, visible);
        let name = if node.name.is_empty() { format!("{} #{index}", kind_name(&node.kind)) } else { node.name.clone() };
        let gltf_index = self.builder.add_node(Node {
            translation,
            rotation,
            scale: [scale; 3],
            ..Node::named(name)
        });

        let mut extras = json!({
            "index": index,
            "kind": kind_name(&node.kind),
            "flags": node.flags,
            "app_culled": node.flags & AV_FLAG_APP_CULLED != 0,
            "stored_transform": transform_json(&node.transform),
            "extra_strings": node.extra_strings,
            "sort_adjust": node.sort_adjust.map(describe::name),
            "in_scene": node.in_scene,
            "affecting_lights": node.lights,
            "texture_effects": node.texture_effects,
            "controllers": describe::controllers_of(desc, |target, _| target == ControllerTarget::Node(index)),
        });
        if is_root {
            extras["root_transform_replaced_by_placement"] = json!(true);
        }
        let mut children = Vec::new();
        match &node.kind {
            NifNodeKind::Mesh { mesh } => {
                let (mesh_index, weights) = self.export_mesh(part, *mesh);
                let gltf_node = &mut self.builder.nodes[gltf_index];
                gltf_node.mesh = Some(mesh_index);
                gltf_node.weights = weights;
                extras["mesh"] = json!({ "index": mesh, "bound_center": desc.meshes[*mesh].bound_center });
                if anim::morph_animated(desc, *mesh) {
                    self.animate_morph(part, *mesh, gltf_index);
                }
            }
            NifNodeKind::ParticleSystem { system } => {
                let system_desc = &desc.particle_systems[*system];
                let textures = self.texturing_textures(desc, system_desc.render_state.texturing);
                extras["particle_system"] = describe::particle_system(desc, *system, system_desc, &textures);
            }
            NifNodeKind::Light { light } => {
                let light_desc = &desc.lights[*light];
                extras["light"] = describe::light(light_desc);
                extras["light"]["color_space"] = json!(DISPLAY_SPACE_NOTE);
                extras["light"]["controllers"] =
                    json!(describe::controllers_of(desc, |target, _| target == ControllerTarget::Light(*light)));
                if let Some(child) = self.export_light(part, *light) {
                    children.push(child);
                }
            }
            NifNodeKind::TextureEffect { effect } => {
                let effect_desc = &desc.texture_effects[*effect];
                let texture = self.cache.nif_texture(
                    &mut self.builder,
                    self.loader,
                    desc,
                    effect_desc.texture,
                    effect_desc.filter,
                    effect_desc.addressing,
                    &mut self.warnings,
                );
                extras["texture_effect"] = describe::texture_effect(desc, effect_desc, texture);
            }
            NifNodeKind::Billboard(mode) => {
                extras["billboard_mode"] = describe::name(mode);
                extras["billboard_note"] = json!("exported with its stored orientation; the client turns it to the camera");
            }
            NifNodeKind::Node | NifNodeKind::Other { .. } => {}
        }
        self.builder.nodes[gltf_index].extras = Some(json!({ "gamebryo": extras }));
        self.animate_node(part, index, base, gltf_index);

        for &child in &node.children {
            if child < desc.nodes.len() && desc.nodes[child].parent == Some(index) {
                children.push(self.export_node(part, child, false));
            }
        }
        self.builder.nodes[gltf_index].children = children;
        gltf_index
    }

    /// Translation, rotation and scale channels of node `index` (scale 0 while hidden).
    fn animate_node(&mut self, part: &Part<'_>, index: usize, base: LocalTransform, gltf_index: usize) {
        let channels = anim::node_channels(part.desc, index);
        if channels == anim::NodeChannels::default() || part.timeline.is_empty() {
            return;
        }
        let mut translations = Vec::new();
        let mut rotations = Vec::new();
        let mut scales = Vec::new();
        for &t in &part.timeline.local {
            let (local, visible) = anim::node_state(part.desc, index, base, t);
            let (translation, rotation, scale) = trs(&local, visible);
            translations.push(translation);
            rotations.push(rotation);
            scales.push([scale; 3]);
        }
        let times = &part.timeline.times;
        if !is_constant(&translations) {
            let values = flatten(&translations);
            self.animation.node_channel(&mut self.builder, gltf_index, "translation", times, &values, 3, Interpolation::Linear);
        }
        if !is_constant(&rotations) {
            anim::align_quaternions(&mut rotations);
            let values = flatten(&rotations);
            self.animation.node_channel(&mut self.builder, gltf_index, "rotation", times, &values, 4, Interpolation::Linear);
        }
        if !is_constant(&scales) {
            // Only visibility moves the scale: on/off steps.
            let interpolation = if channels.scale || channels.rotation { Interpolation::Linear } else { Interpolation::Step };
            let values = flatten(&scales);
            self.animation.node_channel(&mut self.builder, gltf_index, "scale", times, &values, 3, interpolation);
        }
    }

    /// The glTF mesh of mesh `mesh` and its node's default morph weights.
    fn export_mesh(&mut self, part: &Part<'_>, mesh: usize) -> (usize, Option<Vec<f32>>) {
        let desc = part.desc;
        let mesh_desc = &desc.meshes[mesh];
        let geometry = &mesh_desc.geometry;
        let vertex_count = geometry.positions.len();
        let material = self.mesh_material(part, mesh);

        let morph = mesh_desc.morph.as_ref().filter(|morph| {
            !morph.targets.is_empty() && morph.targets.iter().all(|target| target.vertices.len() == vertex_count)
        });
        if mesh_desc.morph.is_some() && morph.is_none() {
            self.warnings.push(format!("{}: mesh {mesh} morph targets do not match its vertices; not exported", part.label));
        }
        let positions: Vec<[f32; 3]> = match morph {
            Some(morph) if morph.relative_targets => morph.targets[0].vertices.clone(),
            Some(_) => vec![[0.0; 3]; vertex_count],
            None => geometry.positions.clone(),
        };
        let mut attributes = serde_json::Map::new();
        attributes.insert("POSITION".into(), json!(self.builder.vec3(&positions, Some(TARGET_ARRAY_BUFFER), true)));
        if geometry.normals.len() == vertex_count && vertex_count > 0 {
            let normals: Vec<[f32; 3]> = geometry.normals.iter().map(|&normal| normalized(normal)).collect();
            attributes.insert("NORMAL".into(), json!(self.builder.vec3(&normals, Some(TARGET_ARRAY_BUFFER), false)));
        }
        if geometry.colors.len() == vertex_count && vertex_count > 0 {
            if material.colors != ColorMode::None {
                let colors: Vec<[f32; 4]> =
                    geometry.colors.iter().filter_map(|&color| material.colors.apply(color)).collect();
                attributes.insert("COLOR_0".into(), json!(self.builder.vec4(&colors, Some(TARGET_ARRAY_BUFFER))));
            }
            attributes.insert(
                RAW_COLOR_ATTRIBUTE.into(),
                json!(self.builder.vec4(&geometry.colors, Some(TARGET_ARRAY_BUFFER))),
            );
        }
        for (set, uvs) in geometry.uv_sets.iter().enumerate() {
            if uvs.len() == vertex_count {
                attributes.insert(format!("TEXCOORD_{set}"), json!(self.builder.vec2(uvs, Some(TARGET_ARRAY_BUFFER))));
            } else {
                self.warnings.push(format!("{}: mesh {mesh} UV set {set} has {} of {vertex_count} UVs", part.label, uvs.len()));
            }
        }
        let front_culled = mesh_desc.render_state.cull == CullMode::Front;
        let mut indices = Vec::with_capacity(geometry.triangles.len() * 3);
        for triangle in &geometry.triangles {
            if triangle.iter().any(|&index| usize::from(index) >= vertex_count) {
                continue;
            }
            let [a, b, c] = triangle.map(u32::from);
            // Front faces culled: the back faces are the visible ones.
            indices.extend(if front_culled { [a, c, b] } else { [a, b, c] });
        }
        if indices.len() != geometry.triangles.len() * 3 {
            self.warnings.push(format!("{}: mesh {mesh} has triangles past its vertices; skipped", part.label));
        }
        let mut primitive = json!({ "attributes": attributes, "material": material.index, "mode": MODE_TRIANGLES });
        if !indices.is_empty() {
            primitive["indices"] = json!(self.builder.indices(&indices));
        }

        let mut weights = None;
        let mut target_names = Vec::new();
        if let Some(morph) = morph {
            let skip = usize::from(morph.relative_targets);
            let mut targets = Vec::new();
            for target in &morph.targets[skip..] {
                let accessor = self.builder.vec3(&target.vertices, Some(TARGET_ARRAY_BUFFER), true);
                targets.push(json!({ "POSITION": accessor }));
                target_names.push(target.name.clone());
            }
            if !targets.is_empty() {
                primitive["targets"] = json!(targets);
                let initial = anim::morph_weights(desc, mesh, part.local_start());
                weights = Some(initial[skip..].to_vec());
            }
        }
        let mut mesh_json = json!({ "name": format!("{} mesh {mesh}", part.label), "primitives": [primitive] });
        if let Some(weights) = &weights {
            mesh_json["weights"] = json!(weights);
            mesh_json["extras"] = json!({ "targetNames": target_names, "gamebryo": {
                "relative_targets": morph.map(|morph| morph.relative_targets),
                "targets": morph.map(|morph| morph.targets.iter().map(|t| json!({ "name": t.name, "initial_weight": t.initial_weight })).collect::<Vec<_>>()),
            }});
        }
        self.builder.meshes.push(mesh_json);
        (self.builder.meshes.len() - 1, weights)
    }

    /// The `weights` channel of a morphed mesh's node.
    fn animate_morph(&mut self, part: &Part<'_>, mesh: usize, gltf_index: usize) {
        let Some(defaults) = self.builder.nodes[gltf_index].weights.clone() else { return };
        if part.timeline.is_empty() || defaults.is_empty() {
            return;
        }
        let skip = usize::from(part.desc.meshes[mesh].morph.as_ref().is_some_and(|morph| morph.relative_targets));
        let mut values = Vec::new();
        let mut constant = true;
        for &t in &part.timeline.local {
            let weights = anim::morph_weights(part.desc, mesh, t);
            constant &= weights[skip..].iter().zip(&defaults).all(|(a, b)| (a - b).abs() < anim::VALUE_EPSILON);
            values.extend_from_slice(&weights[skip..]);
        }
        if !constant {
            let times = &part.timeline.times;
            self.animation.node_channel(&mut self.builder, gltf_index, "weights", times, &values, 1, Interpolation::Linear);
        }
    }

    /// The `KHR_lights_punctual` node of light `light`, or `None` for an ambient light.
    fn export_light(&mut self, part: &Part<'_>, light: usize) -> Option<usize> {
        let light_desc = &part.desc.lights[light];
        let color = linear_rgb(light_desc.diffuse);
        // The dimmer scales the colour as it is (not converted).
        // A light that is off shines with intensity 0 (node scale does not dim a glTF light).
        let intensity = if light_desc.on { light_desc.dimmer.max(0.0) } else { 0.0 };
        let name = format!("{} light {light}", part.label);
        let mut gltf_light = match &light_desc.kind {
            NifLightKind::Ambient => return None,
            NifLightKind::Directional => json!({ "type": "directional" }),
            NifLightKind::Point { .. } => json!({ "type": "point" }),
            NifLightKind::Spot { outer_angle, .. } => json!({
                "type": "spot",
                "spot": { "innerConeAngle": 0.0, "outerConeAngle": outer_angle.to_radians().clamp(f32::EPSILON, MAX_SPOT_ANGLE) },
            }),
        };
        gltf_light["name"] = json!(name);
        gltf_light["color"] = json!(color);
        gltf_light["intensity"] = json!(intensity);
        self.builder.lights.push(gltf_light);
        self.builder.extensions_used.insert(EXT_LIGHTS_PUNCTUAL);
        let mut node = Node::named(name);
        node.rotation = LIGHT_DIRECTION_ROTATION;
        node.extensions = Some(json!({ EXT_LIGHTS_PUNCTUAL: { "light": self.builder.lights.len() - 1 } }));
        Some(self.builder.add_node(node))
    }
}

/// The block kind of a node, for names and extras.
fn kind_name(kind: &NifNodeKind) -> String {
    match kind {
        NifNodeKind::Node => "NiNode".to_owned(),
        NifNodeKind::Billboard(_) => "NiBillboardNode".to_owned(),
        NifNodeKind::Mesh { .. } => "NiTriShape".to_owned(),
        NifNodeKind::ParticleSystem { .. } => "NiParticleSystem".to_owned(),
        NifNodeKind::Light { .. } => "NiLight".to_owned(),
        NifNodeKind::TextureEffect { .. } => "NiTextureEffect".to_owned(),
        NifNodeKind::Other { type_name } => type_name.clone(),
    }
}
