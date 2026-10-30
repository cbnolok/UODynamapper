//! Compiles a parsed NIF ([`gamebryo_nif::NifFile`]) into a [`NifEffectDesc`].
//!
//! The scene is walked from the header's first root, depth first, parents before children; objects outside it
//! (mesh-particle prototypes) follow. Properties accumulate from the root down as in `NiPropertyState`: the
//! nearest property of each type wins. Controllers are listed in the order the scene update runs them
//! (`NiAVObject::UpdateObjectControllers`, then `UpdatePropertyControllers`). Particle system modifiers are
//! stable-sorted by their order, as `NiParticleSystem::AddModifier` inserts them.

use std::collections::{HashMap, HashSet};

use gamebryo_nif::{
    AnimType, ApplyMode, Block, ClampMode, CycleType as NifCycleType, DecayType, FieldType, ForceType,
    InitialVelocityType, KeyType, LightingMode as NifLightingMode, Link, MaterialColorField as NifMaterialColorField,
    MeshEmissionType, NiAVObject, NiAlphaController, NiAlphaProperty, NiBillboardNode, NiBoolData,
    NiBoolInterpolator, NiColorA, NiColorData, NiFloatData, NiFloatInterpolator, NiGeomMorpherController,
    NiGeometry, NiGeometryData, NiKey, NiMaterialColorController, NiMaterialProperty, NiMeshParticleSystem, NiMorphData,
    NiNode, NiObjectNET, NiPSysAgeDeathModifier, NiPSysBombModifier, NiPSysBoundUpdateModifier, NiPSysBoxEmitter,
    NiPSysColorModifier, NiPSysCylinderEmitter, NiPSysData, NiPSysDragModifier, NiPSysEmitter, NiPSysEmitterCtlr,
    NiPSysEmitterInitialRadiusCtlr, NiPSysEmitterLifeSpanCtlr, NiPSysEmitterSpeedCtlr, NiPSysGravityModifier,
    NiPSysGravityStrengthCtlr, NiPSysGrowFadeModifier, NiPSysInitialRotSpeedCtlr, NiPSysInitialRotSpeedVarCtlr,
    NiPSysMeshEmitter, NiPSysMeshUpdateModifier, NiPSysModifier, NiPSysModifierActiveCtlr, NiPSysModifierCtlr,
    NiPSysPositionModifier, NiPSysResetOnLoopCtlr, NiPSysRotationModifier, NiPSysSpawnModifier, NiPSysSphereEmitter,
    NiPSysUpdateCtlr, NiParticleSystem, NiPathInterpolator, NiPixelData, NiPoint3, NiPoint3Interpolator, NiPosData,
    NiSingleInterpController, NiSourceTexture, NiSpecularProperty, NiStencilProperty, NiStringExtraData,
    NiTextureTransformController, NiTexturingProperty, NiTimeController, NiTransformController, NiTransformData,
    NiTransformInterpolator, NiTriShapeData, NiVertexColorProperty, NiVisController, NiZBufferProperty, NifFile,
    PixelFormatFormat, SourceVertexMode, StencilDrawMode, SymmType, TexturingMap, TransformMember, TransformMethod,
};

use super::desc::{
    BillboardMode, BombDecay, BombSymmetry, BoolInterp, BoolKey, CompareFunction, ControllerTarget, ControllerTiming,
    CullMode, Curve, CycleType, EmitterShape, FloatInterp, INVALID_POSE_FLOAT, Key, KeyKind, LightingMode,
    MaterialColorField, MeshEmission, MeshEmitterVelocity, ModifierParam, NifAlpha, NifController, NifControllerKind,
    NifEffectDesc, NifEmitter, NifInterpolator, NifMaterial, NifMesh, NifModifier, NifModifierKind, NifMorph,
    NifMorphTarget, NifNode, NifNodeKind, NifParticleSystem, NifRenderState, NifSpawn, NifTexture, NifTextureMap,
    NifTextureSource, NifTextureTransform, NifTexturing, NifTransform, NifVertexColor, NifZBuffer, PathInterp,
    Point3Interp, RotKey, RotationCurve, Tcb, TextureAddressing, TextureApplyMode, TextureSlot,
    TextureTransformMember, TextureTransformMethod, TransformInterp, VertexColorSource,
};
use crate::render::{BlendFactor, MeshGeometry, TRIANGLE_VERTICES};

/// `NiBoolInterpolator::INVALID_BOOL`: a stored bool pose of 2 means "no pose".
const INVALID_POSE_BOOL: u8 = 2;
/// Bits per pixel of the `FORMAT_RGBA` / `FORMAT_RGB` pixel data decoded here.
const RGBA_BITS_PER_PIXEL: u8 = 32;
const RGB_BITS_PER_PIXEL: u8 = 24;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const RGB_BYTES_PER_PIXEL: usize = 3;
/// Alpha of expanded RGB pixels.
const OPAQUE_ALPHA: u8 = u8::MAX;
/// Guard against controller chains that loop.
const MAX_CONTROLLER_CHAIN: usize = 1024;
/// Guard against parent chains that loop.
const MAX_PARENT_DEPTH: usize = 1024;

/// A compiled effect and what the compiler could not represent.
#[derive(Debug, Clone, Default)]
pub struct NifCompileOutput {
    pub desc: NifEffectDesc,
    /// Unsupported or malformed objects, one line each.
    pub warnings: Vec<String>,
    /// NIF block index of the `NiSourceTexture` of each [`NifEffectDesc::textures`] entry, by texture index
    /// (for converters that need what the desc does not keep: format preferences, embedded mip levels).
    pub texture_blocks: Vec<usize>,
}

impl NifEffectDesc {
    /// Compiles `nif`; `name` is stored as [`NifEffectDesc::name`].
    pub fn compile(nif: &NifFile, name: &str) -> NifCompileOutput {
        compile(nif, name)
    }
}

/// Compiles `nif` into a [`NifEffectDesc`].
pub fn compile(nif: &NifFile, name: &str) -> NifCompileOutput {
    let mut compiler = Compiler {
        nif,
        desc: NifEffectDesc { name: name.to_owned(), ..Default::default() },
        warnings: Vec::new(),
        tables: Tables::default(),
    };
    compiler.run();
    let mut texture_blocks = vec![0; compiler.desc.textures.len()];
    for (&block, &index) in &compiler.tables.texture_of_block {
        texture_blocks[index] = block;
    }
    NifCompileOutput { desc: compiler.desc, warnings: compiler.warnings, texture_blocks }
}

struct Compiler<'a> {
    nif: &'a NifFile,
    desc: NifEffectDesc,
    warnings: Vec<String>,
    tables: Tables,
}

/// Block-to-index maps of the compilation.
#[derive(Default)]
struct Tables {
    /// Block index of each node.
    node_blocks: Vec<usize>,
    node_of_block: HashMap<usize, usize>,
    mesh_of_node: HashMap<usize, usize>,
    system_of_node: HashMap<usize, usize>,
    /// Particle system node block of each system.
    system_blocks: Vec<usize>,
    material_of_block: HashMap<usize, usize>,
    texturing_of_block: HashMap<usize, usize>,
    texture_of_block: HashMap<usize, usize>,
    /// Property blocks of each node, in property list order.
    node_properties: Vec<Vec<usize>>,
    /// The first scene node holding a property block.
    property_owner: HashMap<usize, usize>,
    /// Spawn modifier block -> (system, sorted modifier index).
    spawn_modifiers: HashMap<usize, (usize, usize)>,
    /// (system, sorted modifier index, spawn modifier block) of each age/death modifier with a spawn link.
    pending_spawn_links: Vec<(usize, usize, usize)>,
}

fn link_index(link: impl Link) -> Option<usize> {
    link.index()
}

fn point3(p: &NiPoint3) -> [f32; 3] {
    [p.x, p.y, p.z]
}

fn color4(c: &NiColorA) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

impl Compiler<'_> {
    fn block(&self, index: usize) -> Option<&Block> {
        self.nif.blocks.get(index)
    }

    fn warn(&mut self, message: String) {
        self.warnings.push(message);
    }

    fn run(&mut self) {
        let nif = self.nif;
        let mut parent_of_block = HashMap::new();
        for (index, node) in nif.blocks_of::<NiNode>() {
            for child in &node.children {
                if let Some(child) = link_index(*child) {
                    parent_of_block.entry(child).or_insert(index);
                }
            }
        }
        let root = nif.roots.first().and_then(|root| link_index(*root));
        let root = root.filter(|&root| self.block(root).and_then(Block::get::<NiAVObject>).is_some());
        if let Some(root) = root {
            self.desc.root = Some(self.visit(root, None, true, &NifRenderState::default()));
        } else {
            self.warn("the first root is not an NiAVObject: no scene".to_owned());
        }
        let others: Vec<usize> = nif.blocks_of::<NiAVObject>().map(|(index, _)| index).collect();
        for block in others {
            if self.tables.node_of_block.contains_key(&block) {
                continue;
            }
            let mut top = block;
            for _ in 0..MAX_PARENT_DEPTH {
                match parent_of_block.get(&top) {
                    Some(&parent) if !self.tables.node_of_block.contains_key(&parent) => top = parent,
                    _ => break,
                }
            }
            if !self.tables.node_of_block.contains_key(&top) {
                self.visit(top, None, false, &NifRenderState::default());
            }
        }
        for system in 0..self.tables.system_blocks.len() {
            self.compile_modifiers(system);
        }
        for (system, modifier, spawn_block) in std::mem::take(&mut self.tables.pending_spawn_links) {
            let link = self.tables.spawn_modifiers.get(&spawn_block).copied();
            if let NifModifierKind::AgeDeath { spawn_modifier, .. } =
                &mut self.desc.particle_systems[system].modifiers[modifier].kind
            {
                *spawn_modifier = link;
            }
        }
        for node in 0..self.desc.nodes.len() {
            self.compile_node_controllers(node);
        }
    }

    /// Adds the node of `block` and its subtree; returns its index.
    fn visit(&mut self, block: usize, parent: Option<usize>, in_scene: bool, inherited: &NifRenderState) -> usize {
        let nif = self.nif;
        let av = nif.blocks[block].get::<NiAVObject>().expect("visit is called on NiAVObject blocks");
        let index = self.desc.nodes.len();
        self.tables.node_of_block.insert(block, index);
        self.tables.node_blocks.push(block);
        let rotate = &av.rotate;
        let transform = NifTransform {
            translation: point3(&av.translate),
            rotation: [
                [rotate.m00, rotate.m01, rotate.m02],
                [rotate.m10, rotate.m11, rotate.m12],
                [rotate.m20, rotate.m21, rotate.m22],
            ],
            scale: av.scale,
        };
        let extra_strings = av
            .extra
            .iter()
            .filter_map(|extra| nif.get(extra.cast::<NiStringExtraData>()))
            .map(|extra| extra.string.to_string_lossy())
            .collect();
        self.desc.nodes.push(NifNode {
            name: av.name.to_string_lossy(),
            parent,
            children: Vec::new(),
            transform,
            flags: av.flags,
            kind: NifNodeKind::Node,
            extra_strings,
            in_scene,
        });
        let mut state = inherited.clone();
        let mut properties = Vec::new();
        for property in &av.property_list {
            let Some(property) = link_index(*property) else { continue };
            properties.push(property);
            if in_scene {
                self.tables.property_owner.entry(property).or_insert(index);
            }
            self.apply_property(property, &mut state);
        }
        self.tables.node_properties.push(properties);
        let block_ref = &nif.blocks[block];
        let kind = if let Some(system) = block_ref.get::<NiParticleSystem>() {
            NifNodeKind::ParticleSystem { system: self.add_system(block, index, system, &state) }
        } else if let Some(geometry) = block_ref.get::<NiGeometry>() {
            match self.add_mesh(index, geometry, &state) {
                Some(mesh) => NifNodeKind::Mesh { mesh },
                None => NifNodeKind::Other { type_name: block_ref.type_name().to_owned() },
            }
        } else if let Some(billboard) = block_ref.get::<NiBillboardNode>() {
            let face_mode = billboard.flags.face_mode();
            NifNodeKind::Billboard(BillboardMode::from_raw(face_mode).unwrap_or_else(|| {
                self.warnings.push(format!("billboard node {index}: unknown face mode {face_mode}"));
                BillboardMode::AlwaysFaceCamera
            }))
        } else if block_ref.get::<NiNode>().is_some() {
            NifNodeKind::Node
        } else {
            NifNodeKind::Other { type_name: block_ref.type_name().to_owned() }
        };
        self.desc.nodes[index].kind = kind;
        if let Some(node) = block_ref.get::<NiNode>() {
            for child in &node.children {
                let Some(child) = link_index(*child) else { continue };
                if self.block(child).and_then(Block::get::<NiAVObject>).is_none() {
                    continue;
                }
                if self.tables.node_of_block.contains_key(&child) {
                    self.warn(format!("block {child} has several parents; kept under the first"));
                    continue;
                }
                let child = self.visit(child, Some(index), in_scene, &state);
                self.desc.nodes[index].children.push(child);
            }
        }
        index
    }

    fn apply_property(&mut self, property: usize, state: &mut NifRenderState) {
        let nif = self.nif;
        let block = &nif.blocks[property];
        if let Some(alpha) = block.get::<NiAlphaProperty>() {
            let flags = alpha.flags;
            let defaults = NifAlpha::default();
            state.alpha = NifAlpha {
                blend: flags.alpha_blend(),
                src: u8::try_from(flags.src_blend()).ok().and_then(BlendFactor::from_value).unwrap_or(defaults.src),
                dst: u8::try_from(flags.dest_blend()).ok().and_then(BlendFactor::from_value).unwrap_or(defaults.dst),
                test: flags.test_enable(),
                test_function: CompareFunction::from_raw(flags.test_func()).unwrap_or(defaults.test_function),
                test_reference: alpha.alpha_test_ref,
                no_sorter: flags.no_sorter(),
            };
        } else if let Some(material) = block.get::<NiMaterialProperty>() {
            let next = self.desc.materials.len();
            let index = *self.tables.material_of_block.entry(property).or_insert(next);
            if index == next {
                let color = |c: &gamebryo_nif::NiColor| [c.r, c.g, c.b];
                self.desc.materials.push(NifMaterial {
                    name: material.name.to_string_lossy(),
                    ambient: color(&material.amb),
                    diffuse: color(&material.diff),
                    specular: color(&material.spec),
                    emissive: color(&material.emit),
                    shine: material.shine,
                    alpha: material.alpha,
                });
            }
            state.material = Some(index);
        } else if let Some(texturing) = block.get::<NiTexturingProperty>() {
            let index = match self.tables.texturing_of_block.get(&property) {
                Some(&index) => index,
                None => {
                    let compiled = self.compile_texturing(texturing);
                    let index = self.desc.texturings.len();
                    self.desc.texturings.push(compiled);
                    self.tables.texturing_of_block.insert(property, index);
                    index
                }
            };
            state.texturing = Some(index);
        } else if let Some(z_buffer) = block.get::<NiZBufferProperty>() {
            state.z_buffer = NifZBuffer {
                test: z_buffer.flags.z_buffer_test(),
                write: z_buffer.flags.z_buffer_write(),
                function: u16::try_from(z_buffer.test_function.0)
                    .ok()
                    .and_then(CompareFunction::from_raw)
                    .unwrap_or(NifZBuffer::default().function),
            };
        } else if let Some(vertex_color) = block.get::<NiVertexColorProperty>() {
            let source = match vertex_color.source_mode {
                SourceVertexMode::SOURCE_EMISSIVE => VertexColorSource::Emissive,
                SourceVertexMode::SOURCE_AMB_DIFF => VertexColorSource::AmbientDiffuse,
                _ => VertexColorSource::Ignore,
            };
            let lighting = if vertex_color.lighting_mode == NifLightingMode::LIGHTING_E {
                LightingMode::Emissive
            } else {
                LightingMode::EmissiveAmbientDiffuse
            };
            state.vertex_color = NifVertexColor { source, lighting };
        } else if let Some(stencil) = block.get::<NiStencilProperty>() {
            state.cull = match stencil.draw_mode {
                StencilDrawMode::DRAW_CW => CullMode::Front,
                StencilDrawMode::DRAW_BOTH => CullMode::None,
                _ => CullMode::Back,
            };
        } else if let Some(specular) = block.get::<NiSpecularProperty>() {
            state.specular = specular.flags.specular();
        }
    }

    fn compile_texturing(&mut self, texturing: &NiTexturingProperty) -> NifTexturing {
        let apply_mode = match texturing.apply_mode {
            ApplyMode::APPLY_REPLACE => TextureApplyMode::Replace,
            ApplyMode::APPLY_DECAL => TextureApplyMode::Decal,
            _ => TextureApplyMode::Modulate,
        };
        let mut maps = Vec::new();
        let fixed = [
            (texturing.has_base_map.get(), &texturing.base_map),
            (texturing.has_dark_map.get(), &texturing.dark_map),
            (texturing.has_detail_map.get(), &texturing.detail_map),
            (texturing.has_gloss_map.get(), &texturing.gloss_map),
            (texturing.has_glow_map.get(), &texturing.glow_map),
            (texturing.has_bump_map.get(), &texturing.bump_map.base),
        ];
        for ((present, map), slot) in fixed.into_iter().zip(TextureSlot::FIXED) {
            if present {
                maps.push(self.compile_map(slot, map));
            }
        }
        for (index, decal) in texturing.decal_maps.iter().enumerate() {
            if decal.has_map.get() {
                maps.push(self.compile_map(TextureSlot::Decal(index as u32), &decal.map));
            }
        }
        NifTexturing { apply_mode, maps }
    }

    fn compile_map(&mut self, slot: TextureSlot, map: &TexturingMap) -> NifTextureMap {
        let texture = link_index(map.texture).and_then(|block| self.texture(block));
        let clamp = map.clamp_mode;
        let transform = map.has_texture_transform.get().then(|| {
            let transform = &map.texture_transform;
            NifTextureTransform {
                translate: [transform.translate.x, transform.translate.y],
                scale: [transform.scale.x, transform.scale.y],
                rotate: transform.rotate,
                method: match transform.method {
                    TransformMethod::MAYA_DEPRECATED_TRANSFORM => TextureTransformMethod::MayaDeprecated,
                    TransformMethod::MAX_TRANSFORM => TextureTransformMethod::Max,
                    _ => TextureTransformMethod::Maya,
                },
                center: [transform.center.x, transform.center.y],
            }
        });
        NifTextureMap {
            slot,
            texture,
            addressing: TextureAddressing {
                wrap_s: clamp == ClampMode::WRAP_S_CLAMP_T || clamp == ClampMode::WRAP_S_WRAP_T,
                wrap_t: clamp == ClampMode::CLAMP_S_WRAP_T || clamp == ClampMode::WRAP_S_WRAP_T,
            },
            filter: map.filter_mode.0,
            uv_set: map.texture_index,
            transform,
        }
    }

    /// The texture of an `NiSourceTexture` block.
    fn texture(&mut self, block: usize) -> Option<usize> {
        if let Some(&index) = self.tables.texture_of_block.get(&block) {
            return Some(index);
        }
        let nif = self.nif;
        let Some(source) = nif.blocks[block].get::<NiSourceTexture>() else {
            self.warn(format!("texture block {block} is a {}", nif.blocks[block].type_name()));
            return None;
        };
        let index = self.desc.textures.len();
        let source = if source.external.get() || source.src_pixel_data.is_none() {
            NifTextureSource::External { file_name: source.filename.to_string_lossy() }
        } else {
            match nif.get(source.src_pixel_data) {
                Some(pixels) => decode_pixels(pixels),
                None => NifTextureSource::External { file_name: source.filename.to_string_lossy() },
            }
        };
        self.desc.textures.push(NifTexture { index, source });
        self.tables.texture_of_block.insert(block, index);
        Some(index)
    }

    fn add_mesh(&mut self, node: usize, geometry: &NiGeometry, state: &NifRenderState) -> Option<usize> {
        let nif = self.nif;
        let block = link_index(geometry.model_data)?;
        let Some(shape) = nif.blocks[block].get::<NiTriShapeData>() else {
            self.warn(format!("node {node}: geometry data {} is not drawn", nif.blocks[block].type_name()));
            return None;
        };
        let data: &NiGeometryData = shape;
        let count = data.vertex.len();
        let geometry = MeshGeometry {
            positions: data.vertex.iter().map(point3).collect(),
            normals: if data.has_normal.get() { data.normal.iter().take(count).map(point3).collect() } else { Vec::new() },
            colors: if data.has_color.get() { data.color.iter().map(color4).collect() } else { Vec::new() },
            uv_sets: data.texture.iter().map(|set| set.iter().map(|uv| [uv.x, uv.y]).collect()).collect(),
            triangles: shape.tri_list.chunks_exact(TRIANGLE_VERTICES).map(|t| [t[0], t[1], t[2]]).collect(),
        };
        let mesh = self.desc.meshes.len();
        self.desc.meshes.push(NifMesh { node, geometry, morph: None, render_state: state.clone() });
        self.tables.mesh_of_node.insert(node, mesh);
        Some(mesh)
    }

    fn add_system(&mut self, block: usize, node: usize, system: &NiParticleSystem, state: &NifRenderState) -> usize {
        let nif = self.nif;
        let data = link_index(system.model_data).and_then(|data| nif.blocks[data].get::<NiPSysData>());
        if data.is_none() {
            self.warn(format!("particle system {node} has no NiPSysData"));
        }
        let index = self.desc.particle_systems.len();
        self.desc.particle_systems.push(NifParticleSystem {
            node,
            world_space: system.world_space.get(),
            capacity: data.map_or(0, |data| data.vertices),
            has_colors: data.is_some_and(|data| data.has_color.get()),
            has_radii: data.is_some_and(|data| data.has_radii.get()),
            has_sizes: data.is_some_and(|data| data.has_sizes.get()),
            has_rotation_angles: data.is_some_and(|data| data.has_rotation_angles.get()),
            has_rotation_axes: data.is_some_and(|data| data.has_rotation_axes.get()),
            modifiers: Vec::new(),
            render_state: state.clone(),
            mesh_particles: nif.blocks[block].get::<NiMeshParticleSystem>().is_some(),
        });
        self.tables.system_of_node.insert(node, index);
        self.tables.system_blocks.push(block);
        index
    }

    fn node(&self, link: impl Link) -> Option<usize> {
        link_index(link).and_then(|block| self.tables.node_of_block.get(&block).copied())
    }

    fn compile_modifiers(&mut self, system: usize) {
        let nif = self.nif;
        let block = self.tables.system_blocks[system];
        let psys = nif.blocks[block].get::<NiParticleSystem>().expect("system blocks are particle systems");
        let mut modifiers: Vec<(usize, NifModifier, Option<usize>)> = Vec::new();
        for link in &psys.modifier_list {
            let Some(modifier_block) = link_index(*link) else { continue };
            let Some(modifier) = nif.blocks[modifier_block].get::<NiPSysModifier>() else { continue };
            let (kind, spawn_block) = self.modifier_kind(modifier_block);
            modifiers.push((
                modifier_block,
                NifModifier {
                    name: modifier.name.to_string_lossy(),
                    order: modifier.order.0,
                    active: modifier.active.get(),
                    kind,
                },
                spawn_block,
            ));
        }
        modifiers.sort_by_key(|(_, modifier, _)| modifier.order);
        for (index, (modifier_block, modifier, spawn_block)) in modifiers.into_iter().enumerate() {
            if nif.blocks[modifier_block].get::<NiPSysSpawnModifier>().is_some() {
                self.tables.spawn_modifiers.entry(modifier_block).or_insert((system, index));
            }
            if let Some(spawn_block) = spawn_block {
                self.tables.pending_spawn_links.push((system, index, spawn_block));
            }
            self.desc.particle_systems[system].modifiers.push(modifier);
        }
    }

    /// The modifier of `block`, and the spawn modifier block of an age/death modifier.
    fn modifier_kind(&mut self, block: usize) -> (NifModifierKind, Option<usize>) {
        let nif = self.nif;
        let b = &nif.blocks[block];
        let kind = if let Some(emitter) = b.get::<NiPSysEmitter>() {
            let shape = if let Some(e) = b.get::<NiPSysBoxEmitter>() {
                EmitterShape::Box {
                    object: self.node(e.emitter_obj),
                    width: e.emitter_width,
                    height: e.emitter_height,
                    depth: e.emitter_depth,
                }
            } else if let Some(e) = b.get::<NiPSysSphereEmitter>() {
                EmitterShape::Sphere { object: self.node(e.emitter_obj), radius: e.emitter_radius }
            } else if let Some(e) = b.get::<NiPSysCylinderEmitter>() {
                EmitterShape::Cylinder { object: self.node(e.emitter_obj), radius: e.emitter_radius, height: e.emitter_height }
            } else if let Some(e) = b.get::<NiPSysMeshEmitter>() {
                let meshes = e
                    .geom_emitter_array
                    .iter()
                    .filter_map(|geometry| self.node(*geometry))
                    .filter_map(|node| self.tables.mesh_of_node.get(&node).copied())
                    .collect();
                EmitterShape::Mesh {
                    meshes,
                    velocity: match e.init_velocity_type {
                        InitialVelocityType::NI_VELOCITY_USE_RANDOM => MeshEmitterVelocity::Random,
                        InitialVelocityType::NI_VELOCITY_USE_DIRECTION => MeshEmitterVelocity::Direction,
                        _ => MeshEmitterVelocity::Normals,
                    },
                    emission: match e.emission_type {
                        MeshEmissionType::NI_EMIT_FROM_FACE_CENTER => MeshEmission::FaceCenter,
                        MeshEmissionType::NI_EMIT_FROM_EDGE_CENTER => MeshEmission::EdgeCenter,
                        MeshEmissionType::NI_EMIT_FROM_FACE_SURFACE => MeshEmission::FaceSurface,
                        MeshEmissionType::NI_EMIT_FROM_EDGE_SURFACE => MeshEmission::EdgeSurface,
                        _ => MeshEmission::Vertices,
                    },
                    emit_axis: point3(&e.emit_axis),
                }
            } else {
                return (NifModifierKind::Unsupported { type_name: b.type_name().to_owned() }, None);
            };
            NifModifierKind::Emitter(NifEmitter {
                speed: emitter.speed,
                speed_var: emitter.speed_var,
                declination: emitter.declination,
                declination_var: emitter.declination_var,
                planar_angle: emitter.planar_angle,
                planar_angle_var: emitter.planar_angle_var,
                initial_color: color4(&emitter.initial_color),
                initial_radius: emitter.initial_radius,
                radius_var: emitter.radius_var,
                life_span: emitter.life_span,
                life_span_var: emitter.life_span_var,
                shape,
            })
        } else if let Some(m) = b.get::<NiPSysAgeDeathModifier>() {
            let spawn = link_index(m.spawn_modifier);
            return (NifModifierKind::AgeDeath { spawn_on_death: m.spawn_on_death.get(), spawn_modifier: None }, spawn);
        } else if let Some(m) = b.get::<NiPSysSpawnModifier>() {
            NifModifierKind::Spawn(NifSpawn {
                num_spawn_generations: m.num_spawn_generations,
                percentage_spawned: m.percentage_spawned,
                min_num_to_spawn: m.min_num_to_spawn,
                max_num_to_spawn: m.max_num_to_spawn,
                spawn_speed_chaos: m.spawn_speed_chaos,
                spawn_dir_chaos: m.spawn_dir_chaos,
                life_span: m.life_span,
                life_span_var: m.life_span_var,
            })
        } else if b.get::<NiPSysPositionModifier>().is_some() {
            NifModifierKind::Position
        } else if b.get::<NiPSysBoundUpdateModifier>().is_some() {
            NifModifierKind::BoundUpdate
        } else if let Some(m) = b.get::<NiPSysGrowFadeModifier>() {
            NifModifierKind::GrowFade {
                grow_time: m.grow_time,
                grow_generation: m.grow_generation,
                fade_time: m.fade_time,
                fade_generation: m.fade_generation,
            }
        } else if let Some(m) = b.get::<NiPSysColorModifier>() {
            let colors = nif.get(m.color_data).and_then(|data: &NiColorData| self.curve(data.key_type, &data.keys, color4));
            NifModifierKind::Color { colors }
        } else if let Some(m) = b.get::<NiPSysRotationModifier>() {
            NifModifierKind::Rotation {
                initial_speed: m.initial_rot_speed,
                initial_speed_var: m.initial_rot_speed_var,
                initial_angle: m.initial_rot_angle,
                initial_angle_var: m.initial_rot_angle_var,
                random_speed_sign: m.random_rot_speed_sign.get(),
                random_initial_axis: m.random_initial_axis.get(),
                initial_axis: point3(&m.initial_axis),
            }
        } else if let Some(m) = b.get::<NiPSysGravityModifier>() {
            NifModifierKind::Gravity {
                object: self.node(m.gravity_obj),
                axis: point3(&m.gravity_axis),
                decay: m.decay,
                strength: m.strength,
                spherical: m.r#type == ForceType::FORCE_SPHERICAL,
                turbulence: m.turbulence,
                turbulence_scale: m.scale,
            }
        } else if let Some(m) = b.get::<NiPSysDragModifier>() {
            NifModifierKind::Drag {
                object: self.node(m.drag_obj),
                axis: point3(&m.drag_axis),
                percentage: m.percentage,
                range: m.range,
                falloff: m.falloff,
            }
        } else if let Some(m) = b.get::<NiPSysBombModifier>() {
            NifModifierKind::Bomb {
                object: self.node(m.bomb_obj),
                axis: point3(&m.bomb_axis),
                decay: m.decay,
                delta_v: m.delta_v,
                decay_type: match m.decay_type {
                    DecayType::LINEAR => BombDecay::Linear,
                    DecayType::EXPONENTIAL => BombDecay::Exponential,
                    _ => BombDecay::None,
                },
                symmetry: match m.symm_type {
                    SymmType::CYLINDRICAL => BombSymmetry::Cylindrical,
                    SymmType::PLANAR => BombSymmetry::Planar,
                    _ => BombSymmetry::Spherical,
                },
            }
        } else if let Some(m) = b.get::<NiPSysMeshUpdateModifier>() {
            NifModifierKind::MeshUpdate { meshes: m.meshes.iter().filter_map(|mesh| self.node(*mesh)).collect() }
        } else {
            self.warn(format!("modifier {} is not simulated", b.type_name()));
            NifModifierKind::Unsupported { type_name: b.type_name().to_owned() }
        };
        (kind, None)
    }

    /// The controllers that `node`'s update runs: its own, then its properties' (first holder only).
    fn compile_node_controllers(&mut self, node: usize) {
        let nif = self.nif;
        let in_scene = self.desc.nodes[node].in_scene;
        let owner = in_scene.then_some(node);
        let block = self.tables.node_blocks[node];
        let target = match self.tables.system_of_node.get(&node) {
            Some(&system) => ControllerTarget::ParticleSystem(system),
            None => ControllerTarget::Node(node),
        };
        if let Some(net) = nif.blocks[block].get::<NiObjectNET>() {
            self.compile_chain(net, owner, target, node);
        }
        for property in self.tables.node_properties[node].clone() {
            let first_holder = self.tables.property_owner.get(&property).copied();
            if in_scene && first_holder != Some(node) {
                continue;
            }
            if !in_scene && first_holder.is_some() {
                continue;
            }
            let target = if let Some(&material) = self.tables.material_of_block.get(&property) {
                ControllerTarget::Material(material)
            } else if let Some(&texturing) = self.tables.texturing_of_block.get(&property) {
                ControllerTarget::Texturing(texturing)
            } else {
                continue;
            };
            if let Some(net) = nif.blocks[property].get::<NiObjectNET>() {
                self.compile_chain(net, owner, target, node);
            }
        }
    }

    fn compile_chain(&mut self, net: &NiObjectNET, owner: Option<usize>, target: ControllerTarget, node: usize) {
        let nif = self.nif;
        let mut seen = HashSet::new();
        let mut next = link_index(net.controllers);
        while let Some(block) = next {
            if !seen.insert(block) || seen.len() > MAX_CONTROLLER_CHAIN {
                self.warn(format!("node {node}: controller chain loops"));
                break;
            }
            let Some(controller) = nif.blocks[block].get::<NiTimeController>() else { break };
            next = link_index(controller.next);
            let flags = controller.flags;
            let cycle = match NifCycleType(u32::from(flags.cycle_type())) {
                NifCycleType::LOOP => CycleType::Loop,
                NifCycleType::REVERSE => CycleType::Reverse,
                _ => CycleType::Clamp,
            };
            let timing = ControllerTiming {
                cycle,
                active: flags.active(),
                app_init: u32::from(flags.anim_type()) == AnimType::APP_INIT.0,
                reverse_direction: flags.direction(),
                frequency: controller.frequency,
                phase: controller.phase,
                start_time: controller.lo_key_time,
                stop_time: controller.hi_key_time,
            };
            let kind = self.controller_kind(block, target, node);
            if let NifControllerKind::Unsupported { type_name } = &kind {
                self.warnings.push(format!("controller {type_name} is not run"));
            }
            // A particle system is also a scene node: its transform and visibility controllers drive the node.
            let target = match (&kind, target) {
                (NifControllerKind::Transform(_) | NifControllerKind::Visibility(_), ControllerTarget::ParticleSystem(_)) => {
                    ControllerTarget::Node(node)
                }
                _ => target,
            };
            self.desc.controllers.push(NifController { owner, target, timing, kind });
        }
    }

    fn controller_kind(&mut self, block: usize, target: ControllerTarget, node: usize) -> NifControllerKind {
        let nif = self.nif;
        let b = &nif.blocks[block];
        let system = match target {
            ControllerTarget::ParticleSystem(system) => Some(system),
            _ => None,
        };
        let interpolator = b.get::<NiSingleInterpController>().and_then(|c| link_index(c.interpolator));
        let modifier = b.get::<NiPSysModifierCtlr>().and_then(|c| {
            let name = c.modifier_name.to_string_lossy();
            system.and_then(|system| self.desc.particle_systems[system].modifier_index(&name))
        });
        if b.get::<NiPSysUpdateCtlr>().is_some() {
            NifControllerKind::ParticleUpdate
        } else if b.get::<NiPSysResetOnLoopCtlr>().is_some() {
            NifControllerKind::ResetOnLoop
        } else if let Some(c) = b.get::<NiPSysEmitterCtlr>() {
            NifControllerKind::Emitter {
                modifier,
                birth_rate: interpolator.and_then(|i| self.float_interp(i)),
                active: link_index(c.emitter_active_interpolator).and_then(|i| self.bool_interp(i)),
            }
        } else if b.get::<NiPSysModifierActiveCtlr>().is_some() {
            NifControllerKind::ModifierActive { modifier, interp: interpolator.and_then(|i| self.bool_interp(i)) }
        } else if let Some(param) = modifier_param(b) {
            NifControllerKind::ModifierFloat { modifier, param, interp: interpolator.and_then(|i| self.float_interp(i)) }
        } else if b.get::<NiTransformController>().is_some() {
            match interpolator.and_then(|i| self.interpolator(i)) {
                Some(interp) => NifControllerKind::Transform(interp),
                None => NifControllerKind::Unsupported { type_name: b.type_name().to_owned() },
            }
        } else if b.get::<NiVisController>().is_some() {
            NifControllerKind::Visibility(interpolator.and_then(|i| self.bool_interp(i)).unwrap_or_default())
        } else if b.get::<NiAlphaController>().is_some() {
            NifControllerKind::Alpha(interpolator.and_then(|i| self.float_interp(i)).unwrap_or_default())
        } else if let Some(c) = b.get::<NiMaterialColorController>() {
            let field = match NifMaterialColorField(u32::from(c.flags.field())) {
                NifMaterialColorField::AMB => MaterialColorField::Ambient,
                NifMaterialColorField::DIFF => MaterialColorField::Diffuse,
                NifMaterialColorField::SPEC => MaterialColorField::Specular,
                _ => MaterialColorField::Emissive,
            };
            NifControllerKind::MaterialColor { field, interp: interpolator.and_then(|i| self.point3_interp(i)).unwrap_or_default() }
        } else if let Some(c) = b.get::<NiTextureTransformController>() {
            let member = match c.member {
                TransformMember::TT_TRANSLATE_U => TextureTransformMember::TranslateU,
                TransformMember::TT_TRANSLATE_V => TextureTransformMember::TranslateV,
                TransformMember::TT_ROTATE => TextureTransformMember::Rotate,
                TransformMember::TT_SCALE_U => TextureTransformMember::ScaleU,
                _ => TextureTransformMember::ScaleV,
            };
            NifControllerKind::TextureTransform {
                slot: TextureSlot::from_map_index(c.map_index),
                member,
                interp: interpolator.and_then(|i| self.float_interp(i)).unwrap_or_default(),
            }
        } else if let Some(c) = b.get::<NiGeomMorpherController>() {
            let Some(&mesh) = self.tables.mesh_of_node.get(&node) else {
                return NifControllerKind::Unsupported { type_name: b.type_name().to_owned() };
            };
            if let Some(data) = nif.get(c.morph_data) {
                self.desc.meshes[mesh].morph = Some(morph(data));
            }
            let weights = c.interpolators.iter().map(|i| link_index(*i).and_then(|i| self.float_interp(i))).collect();
            NifControllerKind::Morph { mesh, weights }
        } else {
            NifControllerKind::Unsupported { type_name: b.type_name().to_owned() }
        }
    }

    fn interpolator(&mut self, block: usize) -> Option<NifInterpolator> {
        let nif = self.nif;
        let b = &nif.blocks[block];
        if let Some(i) = b.get::<NiTransformInterpolator>() {
            let value = &i.transform_value;
            let data: Option<&NiTransformData> = nif.get(i.data);
            let mut interp = TransformInterp {
                translation_pose: (value.translate.x != INVALID_POSE_FLOAT).then(|| point3(&value.translate)),
                rotation_pose: (value.rotate.w != INVALID_POSE_FLOAT)
                    .then(|| [value.rotate.w, value.rotate.x, value.rotate.y, value.rotate.z]),
                scale_pose: (value.scale != INVALID_POSE_FLOAT).then_some(value.scale),
                ..Default::default()
            };
            if let Some(data) = data {
                interp.translation = self.curve(data.pos_type, &data.pos_keys, point3);
                interp.scale = self.curve(data.scale_type, &data.scale_keys, |v: &f32| *v);
                interp.rotation = self.rotation_curve(data);
            }
            Some(NifInterpolator::Transform(interp))
        } else if let Some(i) = b.get::<NiPathInterpolator>() {
            let path = nif.get(i.path_data).and_then(|data: &NiPosData| self.curve(data.key_type, &data.keys, point3));
            let percent =
                nif.get(i.pct_data).and_then(|data: &NiFloatData| self.curve(data.key_type, &data.keys, |v: &f32| *v));
            Some(NifInterpolator::Path(PathInterp { path, percent, flags: i.flags.0, follow_axis: i.follow_axis }))
        } else if b.get::<NiPoint3Interpolator>().is_some() {
            self.point3_interp(block).map(NifInterpolator::Point3)
        } else if b.get::<NiFloatInterpolator>().is_some() {
            self.float_interp(block).map(NifInterpolator::Float)
        } else if b.get::<NiBoolInterpolator>().is_some() {
            self.bool_interp(block).map(NifInterpolator::Bool)
        } else {
            self.warn(format!("interpolator {} is not supported", b.type_name()));
            None
        }
    }

    fn float_interp(&mut self, block: usize) -> Option<FloatInterp> {
        let nif = self.nif;
        let Some(i) = nif.blocks[block].get::<NiFloatInterpolator>() else {
            self.warn(format!("expected a float interpolator, found {}", nif.blocks[block].type_name()));
            return None;
        };
        let keys = nif.get(i.float_data).and_then(|data: &NiFloatData| self.curve(data.key_type, &data.keys, |v: &f32| *v));
        Some(FloatInterp { pose: (i.float_value != INVALID_POSE_FLOAT).then_some(i.float_value), keys })
    }

    fn point3_interp(&mut self, block: usize) -> Option<Point3Interp> {
        let nif = self.nif;
        let Some(i) = nif.blocks[block].get::<NiPoint3Interpolator>() else {
            self.warn(format!("expected a point3 interpolator, found {}", nif.blocks[block].type_name()));
            return None;
        };
        let keys = nif.get(i.point3_data).and_then(|data: &NiPosData| self.curve(data.key_type, &data.keys, point3));
        let pose = (i.point3_value.x != INVALID_POSE_FLOAT).then(|| point3(&i.point3_value));
        Some(Point3Interp { pose, keys })
    }

    fn bool_interp(&mut self, block: usize) -> Option<BoolInterp> {
        let nif = self.nif;
        let Some(i) = nif.blocks[block].get::<NiBoolInterpolator>() else {
            self.warn(format!("expected a bool interpolator, found {}", nif.blocks[block].type_name()));
            return None;
        };
        let keys = nif.get(i.bool_data).map_or_else(Vec::new, |data: &NiBoolData| {
            data.keys.iter().map(|key| BoolKey { time: key.time, value: key.bool.get() }).collect()
        });
        let pose = (i.bool_value.0 != INVALID_POSE_BOOL).then(|| i.bool_value.get());
        Some(BoolInterp { pose, keys })
    }

    fn key_kind(&mut self, key_type: KeyType) -> KeyKind {
        match key_type {
            KeyType::LINKEY => KeyKind::Linear,
            KeyType::BEZKEY => KeyKind::Bezier,
            KeyType::TCBKEY => KeyKind::Tcb,
            KeyType::STEPKEY => KeyKind::Step,
            other => {
                self.warn(format!("key type {} read as stepped keys", other.0));
                KeyKind::Step
            }
        }
    }

    fn curve<S: FieldType, T>(&mut self, key_type: KeyType, keys: &[NiKey<S>], value: impl Fn(&S) -> T) -> Option<Curve<T>> {
        if keys.is_empty() {
            return None;
        }
        let kind = self.key_kind(key_type);
        let keys = keys
            .iter()
            .map(|key| Key {
                time: key.time,
                value: value(&key.value),
                in_tangent: value(&key.in_tan),
                out_tangent: value(&key.out_tan),
                tcb: Tcb { tension: key.tension, continuity: key.continuity, bias: key.bias },
            })
            .collect();
        Some(Curve { kind, keys })
    }

    fn rotation_curve(&mut self, data: &NiTransformData) -> Option<RotationCurve> {
        if data.rot_keys.is_empty() {
            return None;
        }
        if data.rot_type == KeyType::EULERKEY {
            let euler = &data.rot_keys[0];
            let mut axes: [Option<Curve<f32>>; 3] = [None, None, None];
            for (axis, keys) in axes.iter_mut().zip(&euler.axes) {
                *axis = self.curve(keys.key_type, &keys.keys, |v: &f32| *v);
            }
            return Some(RotationCurve::Euler { axes });
        }
        let kind = self.key_kind(data.rot_type);
        let keys = data
            .rot_keys
            .iter()
            .map(|key| RotKey {
                time: key.time,
                value: [key.quat.w, key.quat.x, key.quat.y, key.quat.z],
                tcb: Tcb { tension: key.tension, continuity: key.continuity, bias: key.bias },
            })
            .collect();
        Some(RotationCurve::Quaternion { kind, keys })
    }
}

fn modifier_param(block: &Block) -> Option<ModifierParam> {
    if block.get::<NiPSysEmitterSpeedCtlr>().is_some() {
        Some(ModifierParam::EmitterSpeed)
    } else if block.get::<NiPSysEmitterLifeSpanCtlr>().is_some() {
        Some(ModifierParam::EmitterLifeSpan)
    } else if block.get::<NiPSysEmitterInitialRadiusCtlr>().is_some() {
        Some(ModifierParam::EmitterInitialRadius)
    } else if block.get::<NiPSysGravityStrengthCtlr>().is_some() {
        Some(ModifierParam::GravityStrength)
    } else if block.get::<NiPSysInitialRotSpeedVarCtlr>().is_some() {
        Some(ModifierParam::InitialRotationSpeedVar)
    } else if block.get::<NiPSysInitialRotSpeedCtlr>().is_some() {
        Some(ModifierParam::InitialRotationSpeed)
    } else {
        None
    }
}

fn morph(data: &NiMorphData) -> NifMorph {
    NifMorph {
        relative_targets: data.relative_targets.get(),
        targets: data
            .morph_targets
            .iter()
            .map(|target| NifMorphTarget {
                name: target.name.to_string_lossy(),
                initial_weight: target.legacy_weight,
                vertices: target.target_verts.iter().map(point3).collect(),
            })
            .collect(),
    }
}

/// `NiPixelData` mip level 0 as RGBA8 (32-bit RGBA and 24-bit RGB; other formats are left undecoded).
fn decode_pixels(pixels: &NiPixelData) -> NifTextureSource {
    match decode_pixel_level(pixels, 0) {
        Some((width, height, rgba)) => NifTextureSource::Embedded { width, height, rgba },
        None => NifTextureSource::Undecoded { format: pixels.pixel_format.format.0 },
    }
}

/// Mip level `level` of `NiPixelData` as RGBA8 rows (width, height, pixels), or `None` when the level is absent,
/// out of the pixel bytes, or in a format other than 32-bit RGBA and 24-bit RGB (alpha 255).
pub fn decode_pixel_level(pixels: &NiPixelData, level: usize) -> Option<(u32, u32, Vec<u8>)> {
    let format = &pixels.pixel_format;
    let level = pixels.mipmaps.get(level)?;
    let (width, height) = (level.width, level.height);
    let count = width as usize * height as usize;
    let offset = level.offset_in_bytes as usize;
    let bytes_per_pixel = match (format.format, format.bits_per_pixel) {
        (PixelFormatFormat::FORMAT_RGBA, RGBA_BITS_PER_PIXEL) => RGBA_BYTES_PER_PIXEL,
        (PixelFormatFormat::FORMAT_RGB, RGB_BITS_PER_PIXEL) => RGB_BYTES_PER_PIXEL,
        _ => return None,
    };
    let source = pixels.pixels.get(offset..offset + count * bytes_per_pixel)?;
    let rgba = if bytes_per_pixel == RGBA_BYTES_PER_PIXEL {
        source.to_vec()
    } else {
        source.chunks_exact(RGB_BYTES_PER_PIXEL).flat_map(|p| [p[0], p[1], p[2], OPAQUE_ALPHA]).collect()
    };
    Some((width, height, rgba))
}
