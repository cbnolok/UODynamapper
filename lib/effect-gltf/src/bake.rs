//! Optional particle bake: the simulation run at a fixed rate, each frame's sprites written as a mesh of
//! camera-facing quads on its own node, shown during its frame only (a STEP scale animation 0 → 1 → 0).
//!
//! Limits:
//! - The quads face the default client camera (looking along Gamebryo +Y, the Blender front view); from
//!   another view they show as flat cards.
//! - One random run (seed [`crate::BakeOptions::seed`]); the client's particles differ every time.
//! - Sprites only: NIF triangle meshes are already exported as meshes, and Chaos MESH particles (`.nif` clones)
//!   are not baked.
//! - Materials: unlit, double-sided, `COLOR_0` the sprite colour, the batch texture; additive and other blend
//!   factors show as alpha blending (the factors are in the material's `extras`).
//! - The output grows with the frame count and the particle count: a long or dense effect makes a large file.

use particle_sim::nif::default_camera;
use particle_sim::nif::desc::{FILTER_BILERP, TextureAddressing};
use particle_sim::render::{BlendFactor, QUAD_INDICES, QuadVertex, expand_frame};
use particle_sim::{NifEffectDesc, NodeTransform, ParticleSystemInstance, RenderFrame};
use serde_json::{Value, json};

use crate::anim::Interpolation;
use crate::glb::{MODE_TRIANGLES, Node, TARGET_ARRAY_BUFFER};
use crate::material::EXT_MATERIALS_UNLIT;
use crate::scene::Exporter;

/// Vertices of a sprite quad.
const QUAD_VERTICES: usize = 4;
/// Milliseconds per second.
const MS_PER_SECOND: f32 = 1000.0;
/// Baked sprite textures clamp at the edges (sprite atlases).
const SPRITE_ADDRESSING: TextureAddressing = TextureAddressing { wrap_s: false, wrap_t: false };
/// Most frames baked, whatever the clip and rate (bounds the file size).
pub const MAX_BAKE_FRAMES: usize = 1200;

/// A simulation to bake.
pub struct BakeJob<'a> {
    pub label: String,
    pub instance: Box<dyn ParticleSystemInstance + 'a>,
    pub node: NodeTransform,
    pub duration_s: f32,
    pub looping: bool,
    /// The NIF whose textures the batches name (embedded textures resolve through it).
    pub desc: Option<&'a NifEffectDesc>,
}

impl Exporter<'_> {
    /// Bakes `job` over `clip` seconds; returns the node holding the frames, if any frame has sprites.
    pub fn bake(&mut self, mut job: BakeJob<'_>, clip: f32, fps: f32) -> Option<usize> {
        let dt_ms = (MS_PER_SECOND / fps).round().max(1.0) as u32;
        let dt = dt_ms as f32 / MS_PER_SECOND;
        let frames = ((clip / dt).ceil() as usize).clamp(1, MAX_BAKE_FRAMES);
        if (clip / dt).ceil() as usize > MAX_BAKE_FRAMES {
            self.warnings.push(format!("{}: bake limited to {MAX_BAKE_FRAMES} frames", job.label));
        }
        job.instance.start(job.duration_s, job.looping);
        let camera = default_camera();
        let mut frame = RenderFrame::default();
        let mut vertices = Vec::new();
        let mut frame_nodes: Vec<(usize, f32)> = Vec::new();
        let mut materials: Vec<(String, BlendFactor, BlendFactor, usize)> = Vec::new();
        let mut skipped_meshes = false;
        for index in 0..frames {
            job.instance.update(dt_ms, &job.node);
            let time = (index + 1) as f32 * dt;
            frame.clear();
            job.instance.render(&mut frame);
            skipped_meshes |= !frame.meshes.is_empty();
            if frame.sprites.is_empty() {
                continue;
            }
            vertices.clear();
            expand_frame(&frame, &camera, &mut vertices);
            let Some(mesh) = self.frame_mesh(&job, &frame, &vertices, &mut materials) else { continue };
            let mut node = Node::named(format!("{} frame {index}", job.label));
            node.mesh = Some(mesh);
            node.extras = Some(json!({ "gamebryo": { "baked_time": time, "sprites": frame.sprites.len() } }));
            frame_nodes.push((self.builder.add_node(node), time));
        }
        if skipped_meshes {
            self.warnings.push(format!("{}: mesh particles are not baked", job.label));
        }
        if frame_nodes.is_empty() {
            return None;
        }
        for (position, &(node, time)) in frame_nodes.iter().enumerate() {
            let next = frame_nodes.get(position + 1).map(|&(_, next)| next);
            // Static scale: only the first frame shows without the animation.
            if position > 0 {
                self.builder.nodes[node].scale = [0.0; 3];
            }
            let mut times = Vec::new();
            let mut scales = Vec::new();
            if time > 0.0 && position > 0 {
                times.push(0.0);
                scales.push(0.0);
            }
            times.push(time);
            scales.push(1.0);
            if let Some(next) = next {
                times.push(next);
                scales.push(0.0);
            }
            if times.len() > 1 || position > 0 {
                let values: Vec<f32> = scales.iter().flat_map(|&scale| [scale; 3]).collect();
                self.animation.node_channel(&mut self.builder, node, "scale", &times, &values, 3, Interpolation::Step);
            }
        }
        let mut holder = Node::named(format!("{} baked particles", job.label));
        holder.children = frame_nodes.iter().map(|&(node, _)| node).collect();
        holder.extras = Some(json!({ "gamebryo": {
            "baked": true,
            "frame_seconds": dt,
            "frames": frames,
            "note": "particle simulation baked to camera-facing quads for the default client view; one random run",
        }}));
        Some(self.builder.add_node(holder))
    }

    fn frame_mesh(
        &mut self,
        job: &BakeJob<'_>,
        frame: &RenderFrame,
        vertices: &[QuadVertex],
        materials: &mut Vec<(String, BlendFactor, BlendFactor, usize)>,
    ) -> Option<usize> {
        let positions: Vec<[f32; 3]> = vertices.iter().map(|vertex| vertex.position.to_array()).collect();
        let uvs: Vec<[f32; 2]> = vertices.iter().map(|vertex| vertex.uv).collect();
        let colors: Vec<[f32; 4]> = vertices.iter().map(|vertex| vertex.color.map(|c| c.clamp(0.0, 1.0))).collect();
        if positions.iter().flatten().any(|value| !value.is_finite()) {
            self.warnings.push(format!("{}: a baked frame has non-finite positions; skipped", job.label));
            return None;
        }
        let position = self.builder.vec3(&positions, Some(TARGET_ARRAY_BUFFER), true);
        let uv = self.builder.vec2(&uvs, Some(TARGET_ARRAY_BUFFER));
        let color = self.builder.vec4(&colors, Some(TARGET_ARRAY_BUFFER));
        let mut primitives = Vec::new();
        let mut first_vertex = 0u32;
        for batch in &frame.batches {
            let count = batch.sprites.len();
            if count == 0 {
                continue;
            }
            let mut indices = Vec::with_capacity(count * QUAD_INDICES.len());
            for sprite in 0..count as u32 {
                let base = first_vertex + sprite * QUAD_VERTICES as u32;
                indices.extend(QUAD_INDICES.iter().map(|&index| base + index));
            }
            first_vertex += (count * QUAD_VERTICES) as u32;
            let material = self.sprite_material(job, &batch.texture, batch.blend.src, batch.blend.dst, materials);
            let indices = self.builder.indices(&indices);
            primitives.push(json!({
                "attributes": { "POSITION": position, "TEXCOORD_0": uv, "COLOR_0": color },
                "indices": indices,
                "material": material,
                "mode": MODE_TRIANGLES,
            }));
        }
        if primitives.is_empty() {
            return None;
        }
        self.builder.meshes.push(json!({ "name": format!("{} sprites", job.label), "primitives": primitives }));
        Some(self.builder.meshes.len() - 1)
    }

    fn sprite_material(
        &mut self,
        job: &BakeJob<'_>,
        texture: &str,
        src: BlendFactor,
        dst: BlendFactor,
        materials: &mut Vec<(String, BlendFactor, BlendFactor, usize)>,
    ) -> usize {
        if let Some(&(_, _, _, index)) = materials.iter().find(|(name, s, d, _)| name == texture && *s == src && *d == dst) {
            return index;
        }
        let gltf_texture = if texture.is_empty() {
            None
        } else if let Some((desc, index)) = job.desc.and_then(|desc| {
            desc.textures.iter().position(|t| t.name().eq_ignore_ascii_case(texture)).map(|index| (desc, Some(index)))
        }) {
            self.cache.nif_texture(&mut self.builder, self.loader, desc, index, FILTER_BILERP, SPRITE_ADDRESSING, &mut self.warnings)
        } else {
            self.cache.file_texture(&mut self.builder, self.loader, texture, FILTER_BILERP, SPRITE_ADDRESSING, &mut self.warnings)
        };
        let mut pbr = json!({ "baseColorFactor": [1.0, 1.0, 1.0, 1.0], "metallicFactor": 0.0, "roughnessFactor": 1.0 });
        if let Some(gltf_texture) = gltf_texture {
            pbr["baseColorTexture"] = json!({ "index": gltf_texture });
        }
        let opaque = src == BlendFactor::One && dst == BlendFactor::Zero;
        let material: Value = json!({
            "name": format!("{} sprites {texture}", job.label),
            "pbrMetallicRoughness": pbr,
            "alphaMode": if opaque { "OPAQUE" } else { "BLEND" },
            "doubleSided": true,
            "extensions": { EXT_MATERIALS_UNLIT: {} },
            "extras": { "gamebryo": {
                "texture": texture,
                "src_blend": crate::describe::name(src),
                "dst_blend": crate::describe::name(dst),
            }},
        });
        self.builder.extensions_used.insert(EXT_MATERIALS_UNLIT);
        self.builder.materials.push(material);
        let index = self.builder.materials.len() - 1;
        materials.push((texture.to_owned(), src, dst, index));
        index
    }
}
