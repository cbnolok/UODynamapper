//! glTF 2.0 binary (`.glb`) export of the Enhanced Client's effects: one NIF scene ([`export_nif`],
//! [`export_nif_file`]) or a whole particle effect definition with its NIF and Chaos parts placed as the
//! client places them ([`export_effect`]). The output is meant for Blender and other glTF viewers: the parts
//! glTF can draw become meshes, materials and animations, everything else is kept in `extras`.
//!
//! # Design
//!
//! - **Source.** The exporter reads particle-sim's compiled [`NifEffectDesc`] (what the previews and the
//!   renderer run), not the raw blocks: node transforms, geometry, render state and controllers are already
//!   resolved as the client resolves them, and the animation is evaluated with particle-sim's key and clock
//!   code, so the export matches the preview.
//! - **Axes.** Gamebryo effects are Z-up with the client camera looking along +Y (X right). The scene is
//!   written in Gamebryo units under one root node rotated -90 degrees about X (Gamebryo `(x, y, z)` → glTF
//!   `(x, z, -y)`, [`AXIS_ROTATION`]): glTF viewers show the effect upright, and Blender's import (which turns
//!   glTF Y-up into Blender Z-up) gives back the Gamebryo coordinates, so Blender's front view is the client
//!   view.
//! - **Scene, meshes, lights:** see [`scene`]. **Materials and textures:** see [`material`] and [`texture`].
//! - **Colour space.** The client's colours are display-space (sRGB-encoded) values; glTF's colour factors,
//!   `COLOR_0` and light colours are linear. Every colour written there (material factors and their animation,
//!   mesh and baked vertex colours, light colours) is converted once with the exact sRGB EOTF
//!   ([`material::srgb_to_linear`]), alpha and light intensity are not; textures stay sRGB-encoded, as glTF reads
//!   `baseColorTexture`. The client values stay in `extras` (`color_space` says so), and [`validate`] checks that
//!   `COLOR_0` is the raw vertex colour converted exactly once. See [`material`], colour space.
//! - **Animation** ([`anim`]): one animation per export, named after the NIF or the effect. Node transforms,
//!   paths (`NiPathInterpolator`) and morph weights are sampled at [`ExportOptions::sample_rate`] plus the
//!   key times (LINEAR), visibility (`NiVisController`) becomes STEP scale-0 keys (an approximation: a hidden
//!   node is collapsed, not removed), material colour, alpha and texture transforms are animated through
//!   `KHR_animation_pointer`. Cycle type, frequency and phase are applied when sampling (the clip shows the
//!   controllers as the client runs them over [`ExportOptions::clip_length`] seconds); the raw timing and keys
//!   of every controller are in the owning node's or material's `extras`. Light and particle system
//!   controllers are in `extras` only. Colour channels are sampled like the others and each sample converted to
//!   linear, so curved colour keys are exact at the samples (glTF interpolates linear values between them).
//! - **Particles.** A particle system is a node without a mesh whose `extras.gamebryo.particle_system` holds the
//!   full system: emitter, modifiers in order, render state, controllers. With [`ExportOptions::bake`] the
//!   simulation is also baked into per-frame quad meshes (see [`bake`] for the limits). Mesh-particle
//!   prototypes are exported as ordinary meshes.
//! - **Whole effects.** Each part of a particle effect definition (`EffectPlacement` offsets, scale, duration and
//!   loop flag in [`EffectPart::params`]) is a node at its node offset (seen by the default client camera) with
//!   the node scale. A looping part with a duration wraps its clock every duration; a non-looping part with a
//!   duration shorter than the clip is hidden at its end. Chaos (`.ems`) parts are a node with the script and a
//!   summary of the compiled system in `extras`; their MESH particle NIFs are exported as meshes under it.
//! - **Not exported:** skinning and strips (no shipped file has them; gamebryo-nif parses neither), the
//!   fixed-function combine beyond the base map (glow, dark, detail, decal, gloss, bump and environment maps
//!   are embedded and described in `extras`), blend factors other than alpha blending, billboard turning,
//!   depth-buffer and sorting state (in `extras`), light animation, and the live particle simulation (only the
//!   optional bake).
//! - **JSON.** Written with serde_json (no glTF crate is in the workspace). [`validate`] checks the structure of
//!   an export: headers, chunks, references, accessor bounds, animation inputs and the hierarchy.

pub mod anim;
pub mod bake;
pub mod describe;
pub mod glb;
pub mod material;
pub mod scene;
pub mod texture;
pub mod validate;

use std::sync::Arc;

use gamebryo_nif::NifFile;
use particle_sim::chaos::{ChaosSequenceDesc, ChaosSystemDesc};
use particle_sim::nif::default_camera;
use particle_sim::{
    ChaosEffectInstance, ChaosSystem, EffectPlacement, Mt19937, NifEffect, NifEffectDesc, NifEffectInstance, NodeTransform,
    ParticleEffectParams, Vec3,
};
use serde_json::{Value, json};

use crate::anim::{AnimationBuilder, Interpolation, Timeline};
use crate::bake::BakeJob;
use crate::glb::{Builder, Node};
use crate::scene::{Exporter, Part};
use crate::texture::TextureCache;
pub use crate::texture::TextureLoader;

/// The root node's rotation, x y z w: -90 degrees about X, Gamebryo Z-up to glTF Y-up.
pub const AXIS_ROTATION: [f32; 4] = [-std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2];
/// Default bake rate, frames per second.
pub const DEFAULT_BAKE_FPS: f32 = 30.0;
/// Default bake seed.
pub const DEFAULT_BAKE_SEED: u32 = 0x5EED;
/// Clip length of a bake when nothing gives one (a part without duration and without controllers), seconds.
pub const DEFAULT_BAKE_SECONDS: f32 = 3.0;

/// Particle bake settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BakeOptions {
    /// Frames per second.
    pub fps: f32,
    /// Random seed of the simulation.
    pub seed: u32,
}

impl Default for BakeOptions {
    fn default() -> Self {
        Self { fps: DEFAULT_BAKE_FPS, seed: DEFAULT_BAKE_SEED }
    }
}

/// Export settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportOptions {
    /// Animation length in seconds; `None`: the NIF's animation length (largest controller stop time), or for
    /// an effect the longest part (its duration, or its animation length).
    pub clip_length: Option<f32>,
    /// Animation samples per second.
    pub sample_rate: f32,
    /// Bake the particle simulation into per-frame meshes.
    pub bake: Option<BakeOptions>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self { clip_length: None, sample_rate: anim::SAMPLE_RATE_HZ, bake: None }
    }
}

/// An exported `.glb` file and what could not be exported faithfully.
#[derive(Debug, Clone, Default)]
pub struct GlbExport {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

/// A Chaos (`.ems`) part of an effect.
#[derive(Debug, Clone)]
pub struct ChaosPart {
    /// The compiled system.
    pub desc: ChaosSystemDesc,
    /// The script text, written to `extras`.
    pub source: Option<String>,
    /// The NIFs its MESH sequences clone (exported as meshes).
    pub meshes: Vec<NifEffectDesc>,
}

/// What a part of an effect draws.
#[derive(Debug, Clone)]
pub enum PartContent {
    Nif(NifEffectDesc),
    Chaos(ChaosPart),
    /// A part that could not be loaded (the reason is written to `extras` and the warnings).
    Unavailable(String),
}

/// One part of a particle effect definition.
#[derive(Debug, Clone)]
pub struct EffectPart {
    /// The node name: the definition's path and file name.
    pub label: String,
    pub params: ParticleEffectParams,
    pub content: PartContent,
}

fn exporter<'a>(loader: TextureLoader<'a>, options: &'a ExportOptions) -> Exporter<'a> {
    Exporter {
        builder: Builder::default(),
        cache: TextureCache::default(),
        loader,
        warnings: Vec::new(),
        animation: AnimationBuilder::default(),
        options,
    }
}

fn axis_root(name: &str, children: Vec<usize>, builder: &mut Builder) -> usize {
    let mut root = Node::named(name);
    root.rotation = AXIS_ROTATION;
    root.children = children;
    root.extras = Some(json!({ "gamebryo": {
        "note": "Z-up to Y-up: the children are in Gamebryo coordinates (X right, Y away from the client camera, Z up)",
    }}));
    builder.add_node(root)
}

fn finish(mut exporter: Exporter<'_>, name: &str, root: usize, clip: f32, animation_extras: Value) -> GlbExport {
    let animation = std::mem::take(&mut exporter.animation);
    let mut extras = animation_extras;
    extras["clip_seconds"] = json!(clip);
    extras["sample_rate_hz"] = json!(exporter.options.sample_rate);
    if let Some(animation) = animation.finish(name, extras) {
        exporter.builder.animations.push(animation);
    }
    let asset_extras = json!({
        "source": name,
        "coordinates": "Gamebryo Z-up under a -90 degree X rotation of the root node",
        "color_space": material::DISPLAY_SPACE_NOTE,
        "warnings": exporter.warnings,
    });
    let bytes = exporter.builder.finish(name, vec![root], asset_extras);
    GlbExport { bytes, warnings: exporter.warnings }
}

/// Exports one compiled NIF scene; `textures` reads the texture files it names.
pub fn export_nif(desc: &NifEffectDesc, textures: TextureLoader<'_>, options: &ExportOptions) -> GlbExport {
    let mut exporter = exporter(textures, options);
    let clip = options.clip_length.unwrap_or_else(|| desc.animation_length()).max(0.0);
    let part = Part { desc, label: desc.name.clone(), timeline: Timeline::new(desc, clip, None, options.sample_rate) };
    let mut children = exporter.export_nif(&part);
    if let Some(bake) = options.bake {
        let bake_clip = if clip > 0.0 { clip } else { DEFAULT_BAKE_SECONDS };
        let job = BakeJob {
            label: desc.name.clone(),
            instance: Box::new(NifEffectInstance::with_seed(Arc::new(NifEffect::prepare(desc.clone())), bake.seed)),
            node: NodeTransform::default(),
            duration_s: bake_clip,
            looping: true,
            desc: Some(desc),
        };
        children.extend(exporter.bake(job, bake_clip, bake.fps));
    }
    let root = axis_root(&desc.name, children, &mut exporter.builder);
    finish(exporter, &desc.name, root, clip, json!({}))
}

/// Compiles and exports a parsed NIF; the compile warnings are part of the result's warnings.
pub fn export_nif_file(nif: &NifFile, name: &str, textures: TextureLoader<'_>, options: &ExportOptions) -> GlbExport {
    let compiled = particle_sim::nif::compile(nif, name);
    let mut export = export_nif(&compiled.desc, textures, options);
    let mut warnings: Vec<String> = compiled.warnings.iter().map(|warning| format!("{name}: {warning}")).collect();
    warnings.append(&mut export.warnings);
    export.warnings = warnings;
    export
}

/// How long a part plays in the clip: its duration, or its animation length.
fn part_length(part: &EffectPart) -> f32 {
    let duration = part.params.duration_s;
    if duration > 0.0 {
        return duration;
    }
    match &part.content {
        PartContent::Nif(desc) => desc.animation_length(),
        PartContent::Chaos(_) | PartContent::Unavailable(_) => 0.0,
    }
}

fn sequence_summary(sequence: &ChaosSequenceDesc) -> Value {
    json!({
        "name": sequence.name,
        "particle_type": describe::name(sequence.particle_type),
        "resource": sequence.resource,
        "src_blend": describe::name(sequence.src_blend),
        "dst_blend": describe::name(sequence.dst_blend),
        "flags": sequence.flags.0,
        "lifetime": describe::name(&sequence.lifetime),
        "emit_rate": describe::name(&sequence.emit_rate),
        "num_particles": sequence.num_particles,
        "loops": sequence.loops,
        "scale_multiplier": describe::float(sequence.scale_multiplier),
        "velocity_multiplier": describe::float(sequence.velocity_multiplier),
        "gravity": describe::name(&sequence.gravity),
        "inherited": describe::name(&sequence.inherited),
        "chaos_velocity": describe::name(&sequence.chaos_velocity),
        "chaos_location": describe::name(&sequence.chaos_location),
        "burst": describe::name(&sequence.burst),
        "emission_boost": describe::name(&sequence.emission_boost),
        "cel": describe::name(&sequence.cel),
        "events": sequence.events.iter().map(describe::name).collect::<Vec<_>>(),
    })
}

/// The `extras` summary of a compiled Chaos system.
pub fn chaos_summary(desc: &ChaosSystemDesc) -> Value {
    json!({
        "name": desc.name,
        "version": desc.version,
        "script_version": describe::float(desc.script_version),
        "min_emit_time": describe::name(&desc.min_emit_time),
        "resources": desc.resources(),
        "sequences": desc.sequences.iter().map(sequence_summary).collect::<Vec<_>>(),
    })
}

/// Exports a particle effect definition: each part at its placement (as the default client camera sees it).
pub fn export_effect(
    name: &str,
    parts: &[EffectPart],
    textures: TextureLoader<'_>,
    options: &ExportOptions,
) -> GlbExport {
    let mut exporter = exporter(textures, options);
    let longest = parts.iter().map(part_length).fold(0.0, f32::max);
    let clip = options.clip_length.unwrap_or(longest).max(0.0);
    let camera = default_camera();
    let placement = EffectPlacement::seen_by(Vec3::ZERO, &camera);
    let mut children = Vec::new();
    let mut part_extras = Vec::new();
    let bake_clip = if clip > 0.0 { clip } else { DEFAULT_BAKE_SECONDS };
    for (index, part) in parts.iter().enumerate() {
        let params = &part.params;
        let offset = params.node_offset(&placement);
        let scale = params.node_scale();
        let label = format!("part {index}: {}", part.label);
        let period = (params.looping && params.duration_s > 0.0).then_some(params.duration_s);
        if !params.looping && params.duration_s <= 0.0 {
            exporter.warnings.push(format!(
                "{label}: not looping and no duration: the client stops it at its first update; exported as if it played"
            ));
        }
        let mut part_node = Node::named(label.clone());
        part_node.translation = offset.to_array();
        part_node.scale = [scale; 3];
        let mut node_children = Vec::new();
        let mut content_extras = json!({});
        let mut bake_job = None;
        match &part.content {
            PartContent::Nif(desc) => {
                let nif = Part { desc, label: label.clone(), timeline: Timeline::new(desc, clip, period, options.sample_rate) };
                node_children.extend(exporter.export_nif(&nif));
                content_extras = json!({ "nif": desc.name });
                if options.bake.is_some() {
                    bake_job = Some((
                        Box::new(NifEffectInstance::with_seed(
                            Arc::new(NifEffect::prepare(desc.clone())),
                            options.bake.map_or(DEFAULT_BAKE_SEED, |bake| bake.seed),
                        )) as Box<dyn particle_sim::ParticleSystemInstance>,
                        Some(desc),
                    ));
                }
            }
            PartContent::Chaos(chaos) => {
                content_extras = json!({ "chaos": {
                    "script": chaos.source,
                    "system": chaos_summary(&chaos.desc),
                    "note": "Chaos particles are simulated, not stored: bake them to see them",
                }});
                if !chaos.meshes.is_empty() {
                    let mut meshes = Vec::new();
                    for desc in &chaos.meshes {
                        let mesh_part = Part {
                            desc,
                            label: format!("{label} mesh {}", desc.name),
                            timeline: Timeline::new(desc, clip, None, options.sample_rate),
                        };
                        let roots = exporter.export_nif(&mesh_part);
                        let mut holder = Node::named(format!("{} (MESH particle)", desc.name));
                        holder.children = roots;
                        meshes.push(exporter.builder.add_node(holder));
                    }
                    let mut holder = Node::named("chaos_meshes");
                    holder.children = meshes;
                    node_children.push(exporter.builder.add_node(holder));
                }
                if let Some(bake) = options.bake {
                    let system = ChaosSystem::prepare(chaos.desc.clone(), &mut Mt19937::new(bake.seed));
                    bake_job = Some((
                        Box::new(ChaosEffectInstance::with_seed(Arc::new(system), bake.seed))
                            as Box<dyn particle_sim::ParticleSystemInstance>,
                        None,
                    ));
                }
            }
            PartContent::Unavailable(reason) => {
                exporter.warnings.push(format!("{label}: {reason}"));
                content_extras = json!({ "unavailable": reason });
            }
        }
        part_node.children = node_children;
        let part_json = json!({
            "label": part.label,
            "offset_x": params.offset_x,
            "offset_y": params.offset_y,
            "camera_offset": params.camera_offset,
            "scale": params.scale,
            "duration_s": params.duration_s,
            "looping": params.looping,
            "node_offset": offset.to_array(),
            "content": content_extras,
        });
        part_node.extras = Some(json!({ "gamebryo": { "effect_part": part_json } }));
        let part_index = exporter.builder.add_node(part_node);
        // A part that stops before the clip ends is hidden at its end.
        if !params.looping && params.duration_s > 0.0 && params.duration_s < clip {
            let values = [scale, scale, scale, 0.0, 0.0, 0.0];
            let times = [0.0, params.duration_s];
            exporter.animation.node_channel(&mut exporter.builder, part_index, "scale", &times, &values, 3, Interpolation::Step);
        }
        children.push(part_index);
        part_extras.push(part_json);
        if let (Some(bake), Some((instance, desc))) = (options.bake, bake_job) {
            let job = BakeJob {
                label: label.clone(),
                instance,
                node: NodeTransform { position: offset, scale, ..NodeTransform::default() },
                duration_s: params.duration_s,
                looping: params.looping,
                desc,
            };
            children.extend(exporter.bake(job, bake_clip, bake.fps));
        }
    }
    let root = axis_root(name, children, &mut exporter.builder);
    finish(exporter, name, root, clip, json!({ "parts": part_extras }))
}

#[cfg(test)]
mod tests;
