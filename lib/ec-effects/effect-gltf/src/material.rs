//! Gamebryo render state → glTF material.
//!
//! Effects draw without lights (`UOParticleEffectNif` scenes have no light affecting them, except
//! 00005540_shadowstrike's own), so the client's fixed-function combine reduces to
//!
//! - `Kd.rgb` = the material emissive, or the vertex colour with vertex colour source `EMISSIVE`;
//! - `Kd.a` = the vertex alpha with (lighting `E`, source `EMISSIVE`) or (lighting `E_A_D`, source `AMB_DIFF`),
//!   otherwise the material alpha;
//! - `out.rgb = Kd.rgb * base * dark * 2 detail (decals lerped in) + env + glow`, `out.a = Kd.a * base.a`;
//! - apply mode `REPLACE`: `out = base (+ glow)`.
//!
//! An unlit glTF material (`KHR_materials_unlit`) draws `baseColorFactor * COLOR_0 * baseColorTexture`, so the
//! factor and `COLOR_0` split `Kd` by mode:
//!
//! | vertex colours                  | baseColorFactor         | COLOR_0               |
//! |---------------------------------|-------------------------|-----------------------|
//! | source EMISSIVE, lighting E     | (1, 1, 1, 1)            | vertex RGBA           |
//! | source EMISSIVE, lighting E_A_D | (1, 1, 1, alpha)        | (vertex RGB, 1)       |
//! | source AMB_DIFF, lighting E_A_D | (emissive, 1)           | (1, 1, 1, vertex A)   |
//! | otherwise / no vertex colours   | (emissive, alpha)       | none                  |
//! | apply mode REPLACE with a base  | (1, 1, 1, 1)            | none                  |
//!
//! Colours are clamped to [0, 1] and converted to linear (see below; the raw values stay in `extras`, the raw
//! vertex colours in the `_NIF_VERTEX_COLOR` attribute). The glow map (which the client adds to the colour, and
//! an unlit material cannot), the dark, detail, decal, gloss and bump maps and the environment map
//! (`NiTextureEffect`) are embedded and listed in `extras` only; apply mode `DECAL` is exported as `MODULATE`.
//!
//! The PBR fallback follows the `KHR_materials_unlit` implementation note: metallic 0, roughness 1, no
//! `emissiveTexture` and no `emissiveFactor` (zero). An emissive texture on an unlit material is valid glTF,
//! but Blender 5.0's importer builds no node for it (unlit import reads the base colour only), so an animated
//! `KHR_texture_transform` of it fails the whole import (`KeyError: 'blender_nodetree'`);
//! [`crate::validate`] rejects both.
//!
//! - `alphaMode`: `BLEND` when blending is on (the blend factors are in `extras`: additive blending shows as
//!   ordinary alpha blending), `MASK` with `alphaCutoff = reference / 255` when only the alpha test is on (its
//!   compare function is in `extras`), `OPAQUE` otherwise.
//! - `doubleSided`: stencil draw mode `BOTH`. Mode `CW` (front faces culled) flips the triangle winding.
//! - Static texture transforms: `KHR_texture_transform` (offset, rotation, scale of the `NiTextureTransform`
//!   matrix; the decomposition is exact for the Maya methods and for uniform-scale Max transforms).
//! - Animated materials: alpha and colour controllers drive `baseColorFactor`, texture transform controllers
//!   of the base map the `KHR_texture_transform` members, through `KHR_animation_pointer`.
//!
//! # Colour space
//!
//! The client's colours (material colours, vertex colours, light colours, particle colours) are display-space
//! values: the fixed-function combine and the blending run on the sRGB-encoded framebuffer values without any
//! conversion. glTF's colour factors, `COLOR_0` and `KHR_lights_punctual` colours are linear (only
//! `baseColorTexture` is sRGB-encoded, so the textures are written as they are). Every colour written to those
//! slots is therefore clamped to [0, 1] and converted once with the exact sRGB EOTF ([`srgb_to_linear`]); alpha is
//! not converted. Where:
//!
//! - `baseColorFactor`, static and every sample of its `KHR_animation_pointer` channel;
//! - `COLOR_0` of meshes ([`ColorMode::apply`]) and of baked particle frames ([`crate::bake`]);
//! - light colours ([`crate::scene`]): the colour is converted, the intensity stays the dimmer (a plain
//!   multiplier, now of the linear colour; the client scales the display-space colour by it, so a dimmer below 1
//!   is lighter here than in the client);
//! - morph targets carry positions only, so no morphed colour exists.
//!
//! Animated colours are never written as key control values: every channel is sampled (LINEAR keys at the sample
//! rate plus the key times, see [`crate::anim`]) and each sample is converted, so Bezier and TCB colour keys are
//! exact at every sample; between samples glTF interpolates in linear space where the client interpolates the
//! display-space value, a difference within the sampling error. The original client values are kept in `extras`
//! (`render_state`, `material_at_start`, `controllers`, `light`, the particle system description), which say so
//! in `color_space` ([`DISPLAY_SPACE_NOTE`]); the baked particle colours are not stored raw, but the conversion is
//! invertible on [0, 1].
//!
//! The viewer still blends in linear space where the client blends display-space values, so translucent and
//! additive draws keep their hue but not their exact brightness (no colour value can make up for that).

use particle_sim::nif::desc::{
    ControllerTarget, CullMode, LightingMode, NifMaterial, NifRenderState, NifTextureTransform, TextureApplyMode,
    TextureEffectKind, TextureSlot, VertexColorSource,
};
use particle_sim::nif::texture_transform_matrix;
use particle_sim::NifEffectDesc;
use serde_json::{Map, Value, json};

use crate::anim::{self, Interpolation, flatten, is_constant};
use crate::describe;
use crate::scene::{Exporter, Part};

/// The unlit material extension.
pub const EXT_MATERIALS_UNLIT: &str = "KHR_materials_unlit";
/// The texture transform extension.
pub const EXT_TEXTURE_TRANSFORM: &str = "KHR_texture_transform";
/// Alpha test references are bytes: the cutoff is `reference / ALPHA_REFERENCE_MAX`.
const ALPHA_REFERENCE_MAX: f32 = 255.0;
/// Columns of a texture transform matrix that are farther from orthogonal than this cannot be written exactly.
const ORTHOGONALITY_EPSILON: f32 = 1e-4;
/// Unlit materials are written with this PBR fallback: not metallic.
const FALLBACK_METALLIC: f32 = 0.0;
/// Unlit materials are written with this PBR fallback: fully rough.
const FALLBACK_ROUGHNESS: f32 = 1.0;
/// White, opaque.
const WHITE: [f32; 4] = [1.0; 4];
/// sRGB EOTF (IEC 61966-2-1): encoded values up to this lie on the linear segment.
const SRGB_LINEAR_SEGMENT_END: f32 = 0.04045;
/// sRGB EOTF: the linear segment divides by this.
const SRGB_LINEAR_SEGMENT_SLOPE: f32 = 12.92;
/// sRGB EOTF: the power segment is `((c + offset) / (1 + offset))^exponent`, this the offset.
const SRGB_POWER_OFFSET: f32 = 0.055;
/// sRGB EOTF: the power segment's exponent.
const SRGB_POWER_EXPONENT: f32 = 2.4;
/// The `extras` note on the colour space of the Gamebryo colours.
pub const DISPLAY_SPACE_NOTE: &str = "Gamebryo colours in extras are the client's display-space (sRGB-encoded) values; \
    the glTF colours (baseColorFactor, COLOR_0, light colours and their animation) are them clamped to [0, 1] and \
    converted to linear with the sRGB EOTF, alpha unconverted";

/// The sRGB EOTF: a display-space (sRGB-encoded) channel in [0, 1] as a linear one.
pub fn srgb_to_linear(value: f32) -> f32 {
    if value <= SRGB_LINEAR_SEGMENT_END {
        value / SRGB_LINEAR_SEGMENT_SLOPE
    } else {
        ((value + SRGB_POWER_OFFSET) / (1.0 + SRGB_POWER_OFFSET)).powf(SRGB_POWER_EXPONENT)
    }
}

/// A display-space colour clamped to [0, 1] and converted to linear.
pub fn linear_rgb(color: [f32; 3]) -> [f32; 3] {
    color.map(|value| srgb_to_linear(value.clamp(0.0, 1.0)))
}

/// A display-space colour clamped to [0, 1], its colour converted to linear, its alpha kept.
pub fn linear_rgba(color: [f32; 4]) -> [f32; 4] {
    let [r, g, b] = linear_rgb([color[0], color[1], color[2]]);
    [r, g, b, color[3].clamp(0.0, 1.0)]
}

/// How the vertex colours go into `COLOR_0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// Vertex RGBA.
    Rgba,
    /// Vertex RGB, alpha 1.
    RgbOpaque,
    /// White with the vertex alpha.
    AlphaOnly,
    /// No `COLOR_0`.
    None,
}

impl ColorMode {
    /// The `COLOR_0` value of a raw vertex colour (clamped, converted to linear).
    pub fn apply(self, color: [f32; 4]) -> Option<[f32; 4]> {
        let color = linear_rgba(color);
        match self {
            Self::Rgba => Some(color),
            Self::RgbOpaque => Some([color[0], color[1], color[2], 1.0]),
            Self::AlphaOnly => Some([1.0, 1.0, 1.0, color[3]]),
            Self::None => None,
        }
    }
}

/// The colour mode of a render state; `replace`: the base map replaces the colour.
fn color_mode(state: &NifRenderState, has_colors: bool, replace: bool) -> ColorMode {
    if !has_colors || replace {
        return ColorMode::None;
    }
    match (state.vertex_color.source, state.vertex_color.lighting) {
        (VertexColorSource::Emissive, LightingMode::Emissive) => ColorMode::Rgba,
        (VertexColorSource::Emissive, LightingMode::EmissiveAmbientDiffuse) => ColorMode::RgbOpaque,
        (VertexColorSource::AmbientDiffuse, LightingMode::EmissiveAmbientDiffuse) => ColorMode::AlphaOnly,
        _ => ColorMode::None,
    }
}

/// `baseColorFactor` of a material state (its colour converted to linear).
fn base_factor(mode: ColorMode, replace: bool, material: &NifMaterial) -> [f32; 4] {
    if replace {
        return WHITE;
    }
    let [r, g, b] = linear_rgb(material.emissive);
    let alpha = material.alpha.clamp(0.0, 1.0);
    match mode {
        ColorMode::Rgba => WHITE,
        ColorMode::RgbOpaque => [1.0, 1.0, 1.0, alpha],
        ColorMode::AlphaOnly => [r, g, b, 1.0],
        ColorMode::None => [r, g, b, alpha],
    }
}

/// `KHR_texture_transform` members (offset, rotation, scale) of a Gamebryo texture transform. glTF applies
/// `T * R(r) * S` with `R(r) = [[cos r, sin r], [-sin r, cos r]]` (rows), so the matrix columns give
/// `scale.x = |col0|`, `r = atan2(-col0.y, col0.x)`, `scale.y = cos r col1.y + sin r col1.x`.
pub fn texture_transform_members(transform: &NifTextureTransform) -> ([f32; 2], f32, [f32; 2], bool) {
    let matrix = texture_transform_matrix(transform).rows;
    let col0 = [matrix[0][0], matrix[1][0]];
    let col1 = [matrix[0][1], matrix[1][1]];
    let offset = [matrix[0][2], matrix[1][2]];
    let scale_x = (col0[0] * col0[0] + col0[1] * col0[1]).sqrt();
    let rotation = (-col0[1]).atan2(col0[0]);
    let (sin, cos) = rotation.sin_cos();
    let scale_y = cos * col1[1] + sin * col1[0];
    let exact = (col0[0] * col1[0] + col0[1] * col1[1]).abs() < ORTHOGONALITY_EPSILON
        && (cos * col1[0] - sin * col1[1]).abs() < ORTHOGONALITY_EPSILON;
    (offset, rotation, [scale_x, scale_y], exact)
}

fn transform_json(transform: &NifTextureTransform) -> (Value, bool) {
    let (offset, rotation, scale, exact) = texture_transform_members(transform);
    (json!({ "offset": offset, "rotation": rotation, "scale": scale }), exact)
}

/// The environment map that reaches node `node`: the texture effects attached from the root down, on, of the
/// environment map kind, not excluding the node or an ancestor; the last one applies.
pub fn environment_map(desc: &NifEffectDesc, node: usize) -> Option<usize> {
    let mut chain = vec![node];
    let mut current = node;
    while let Some(parent) = desc.nodes[current].parent {
        chain.push(parent);
        current = parent;
    }
    chain.reverse();
    let mut found = None;
    for &ancestor in &chain {
        for &effect_index in &desc.nodes[ancestor].texture_effects {
            let Some(effect) = desc.texture_effects.get(effect_index) else { continue };
            let excluded = chain.iter().any(|node| effect.unaffected_nodes.contains(node));
            if effect.on && effect.kind == TextureEffectKind::EnvironmentMap && !excluded {
                found = Some(effect_index);
            }
        }
    }
    found
}

/// The material of a mesh and how its vertex colours are written.
pub struct MeshMaterial {
    pub index: usize,
    pub colors: ColorMode,
}

impl Exporter<'_> {
    /// The glTF textures of the maps of a texturing property, in map order (embedding their images).
    pub fn texturing_textures(&mut self, desc: &NifEffectDesc, texturing: Option<usize>) -> Vec<Option<usize>> {
        let Some(texturing) = texturing.and_then(|index| desc.texturings.get(index)) else { return Vec::new() };
        texturing
            .maps
            .iter()
            .map(|map| {
                self.cache.nif_texture(
                    &mut self.builder,
                    self.loader,
                    desc,
                    map.texture,
                    map.filter,
                    map.addressing,
                    &mut self.warnings,
                )
            })
            .collect()
    }

    /// The glTF material of mesh `mesh` of the part.
    pub fn mesh_material(&mut self, part: &Part<'_>, mesh: usize) -> MeshMaterial {
        let desc = part.desc;
        let mesh_desc = &desc.meshes[mesh];
        let state = &mesh_desc.render_state;
        let uv_sets = mesh_desc.geometry.uv_sets.len();
        let gltf_textures = self.texturing_textures(desc, state.texturing);
        let texturing = state.texturing.and_then(|index| desc.texturings.get(index).map(|t| (index, t)));
        let map_of = |slot: TextureSlot| {
            texturing.and_then(|(_, t)| t.maps.iter().position(|map| map.slot == slot && map.texture.is_some()))
        };
        let base_map = map_of(TextureSlot::Base).filter(|&map| gltf_textures[map].is_some());
        let glow_map = map_of(TextureSlot::Glow).filter(|&map| gltf_textures[map].is_some());
        let replace = base_map.is_some() && texturing.is_some_and(|(_, t)| t.apply_mode == TextureApplyMode::Replace);
        let colors = color_mode(state, !mesh_desc.geometry.colors.is_empty(), replace);
        let material_at = |t: f32| match state.material {
            Some(material) => anim::material_state(desc, material, t),
            None => NifMaterial { emissive: [0.0; 3], ..NifMaterial::default() },
        };
        let initial = material_at(part.local_start());
        let material_index = self.builder.materials.len();

        let texture_info = |exporter: &mut Self, map: usize, slot_pointer: &str, approximations: &mut Vec<String>| {
            let (texturing_index, texturing) = texturing.expect("a map has a texturing property");
            let map_desc = &texturing.maps[map];
            let mut uv_set = map_desc.uv_set as usize;
            if uv_set >= uv_sets {
                exporter.warnings.push(format!(
                    "{}: mesh {mesh} map {:?} uses UV set {uv_set} of {uv_sets}; set 0 is used",
                    part.label, map_desc.slot
                ));
                uv_set = 0;
            }
            let mut info = json!({ "index": gltf_textures[map], "texCoord": uv_set });
            let animated = anim::texture_transform_animated(desc, texturing_index, map);
            let transform = anim::texture_transform_state(desc, texturing_index, map, part.local_start());
            if let Some(transform) = transform.or(animated.then(NifTextureTransform::default)) {
                let (value, exact) = transform_json(&transform);
                if !exact {
                    approximations.push(format!("{:?} map transform is not a rotation and scale", map_desc.slot));
                }
                info["extensions"] = json!({ EXT_TEXTURE_TRANSFORM: value });
                exporter.builder.extensions_used.insert(EXT_TEXTURE_TRANSFORM);
            }
            if animated && !part.timeline.is_empty() {
                exporter.animate_texture_transform(part, texturing_index, map, material_index, slot_pointer);
            }
            info
        };

        let mut approximations = Vec::new();
        let mut pbr = Map::new();
        pbr.insert("baseColorFactor".into(), json!(base_factor(colors, replace, &initial)));
        pbr.insert("metallicFactor".into(), json!(FALLBACK_METALLIC));
        pbr.insert("roughnessFactor".into(), json!(FALLBACK_ROUGHNESS));
        if let Some(map) = base_map {
            let info = texture_info(self, map, "pbrMetallicRoughness/baseColorTexture", &mut approximations);
            pbr.insert("baseColorTexture".into(), info);
        }
        let mut material = Map::new();
        material.insert("name".into(), json!(format!("{} mesh {mesh}", part.label)));
        material.insert("pbrMetallicRoughness".into(), Value::Object(pbr));
        if glow_map.is_some() {
            approximations.push("glow map in extras only".to_owned());
        }
        if state.alpha.blend {
            material.insert("alphaMode".into(), json!("BLEND"));
        } else if state.alpha.test {
            material.insert("alphaMode".into(), json!("MASK"));
            material.insert("alphaCutoff".into(), json!(f32::from(state.alpha.test_reference) / ALPHA_REFERENCE_MAX));
        }
        if state.cull == CullMode::None {
            material.insert("doubleSided".into(), json!(true));
        }
        material.insert("extensions".into(), json!({ EXT_MATERIALS_UNLIT: {} }));
        self.builder.extensions_used.insert(EXT_MATERIALS_UNLIT);

        if state.alpha.blend {
            approximations.push(format!("blend {:?} / {:?} drawn as alpha blending", state.alpha.src, state.alpha.dst));
        }
        if texturing.is_some_and(|(_, t)| t.apply_mode == TextureApplyMode::Decal) {
            approximations.push("apply mode Decal exported as Modulate".to_owned());
        }
        let environment = environment_map(desc, mesh_desc.node).map(|effect_index| {
            let effect = &desc.texture_effects[effect_index];
            let texture = self.cache.nif_texture(
                &mut self.builder,
                self.loader,
                desc,
                effect.texture,
                effect.filter,
                effect.addressing,
                &mut self.warnings,
            );
            approximations.push("environment map in extras only".to_owned());
            json!({ "texture_effect": effect_index, "description": describe::texture_effect(desc, effect, texture) })
        });
        let animated = state.material.is_some_and(|material| {
            anim::controllers_on(desc, ControllerTarget::Material(material)).next().is_some()
        });
        if animated && !part.timeline.is_empty() {
            let samples: Vec<NifMaterial> = part.timeline.local.iter().map(|&t| material_at(t)).collect();
            let bases: Vec<[f32; 4]> = samples.iter().map(|m| base_factor(colors, replace, m)).collect();
            if !is_constant(&bases) {
                self.animation.pointer_channel(
                    &mut self.builder,
                    format!("/materials/{material_index}/pbrMetallicRoughness/baseColorFactor"),
                    &part.timeline.times,
                    &flatten(&bases),
                    4,
                    Interpolation::Linear,
                );
            }
            let rgb: Vec<[f32; 3]> = bases.iter().map(|base| [base[0], base[1], base[2]]).collect();
            if !is_constant(&rgb) {
                approximations.push("animated colour interpolated in linear space between samples".to_owned());
            }
        }
        let controllers = describe::controllers_of(desc, |target, _| {
            Some(target) == state.material.map(ControllerTarget::Material)
                || Some(target) == state.texturing.map(ControllerTarget::Texturing)
        });
        material.insert(
            "extras".into(),
            json!({ "gamebryo": {
                "render_state": describe::render_state(desc, state, &gltf_textures),
                "material_at_start": state.material.map(|_| describe::material(&initial)),
                "environment_map": environment,
                "controllers": controllers,
                "approximations": approximations,
                "color_space": DISPLAY_SPACE_NOTE,
            }}),
        );
        self.builder.materials.push(Value::Object(material));

        MeshMaterial { index: material_index, colors }
    }

    /// `KHR_animation_pointer` channels of the texture transform of map `map` (the texture info at
    /// `/materials/<material>/<slot_pointer>`).
    fn animate_texture_transform(
        &mut self,
        part: &Part<'_>,
        texturing: usize,
        map: usize,
        material: usize,
        slot_pointer: &str,
    ) {
        let desc = part.desc;
        let mut offsets = Vec::new();
        let mut rotations = Vec::new();
        let mut scales = Vec::new();
        for &t in &part.timeline.local {
            let transform = anim::texture_transform_state(desc, texturing, map, t).unwrap_or_default();
            let (offset, rotation, scale, _) = texture_transform_members(&transform);
            offsets.push(offset);
            rotations.push([rotation]);
            scales.push(scale);
        }
        // atan2 wraps at ±pi: keep a spinning map's angle continuous.
        for index in 1..rotations.len() {
            let previous = rotations[index - 1][0];
            let turns = ((rotations[index][0] - previous) / std::f32::consts::TAU).round();
            rotations[index][0] -= turns * std::f32::consts::TAU;
        }
        let base = format!("/materials/{material}/{slot_pointer}/extensions/{EXT_TEXTURE_TRANSFORM}");
        let times = &part.timeline.times;
        if !is_constant(&offsets) {
            let values = flatten(&offsets);
            self.animation.pointer_channel(&mut self.builder, format!("{base}/offset"), times, &values, 2, Interpolation::Linear);
        }
        if !is_constant(&rotations) {
            let values = flatten(&rotations);
            self.animation.pointer_channel(&mut self.builder, format!("{base}/rotation"), times, &values, 1, Interpolation::Linear);
        }
        if !is_constant(&scales) {
            let values = flatten(&scales);
            self.animation.pointer_channel(&mut self.builder, format!("{base}/scale"), times, &values, 2, Interpolation::Linear);
        }
    }
}
