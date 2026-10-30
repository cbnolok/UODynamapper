//! Exports of synthetic scenes, checked structurally and by value.

use gamebryo_nif::{
    Header, NiBool, NiColor, NiMaterialProperty, NiNode, NiPoint3, NiTriShape, NiTriShapeData, NifFile, NifString, Ref,
};
use particle_sim::nif::desc::{
    BoolInterp, BoolKey, CompareFunction, ControllerTarget, ControllerTiming, Curve, CycleType, FloatInterp, Key, KeyKind,
    LightingMode, MaterialColorField, NifAlpha, NifController, NifControllerKind, NifInterpolator, NifLight,
    NifLightKind, NifMaterial, NifMesh, NifNode, NifNodeKind, NifRenderState, NifTexture, NifTextureMap,
    NifTextureSource, NifTextureTransform, NifTexturing, NifTransform, NifVertexColor, Point3Interp, Tcb,
    TextureAddressing, TextureApplyMode, TextureSlot, TextureTransformMember, TextureTransformMethod, TransformInterp,
    VertexColorSource,
};
use particle_sim::nif::keys::Quat;
use particle_sim::nif::texture_transform_matrix;
use particle_sim::render::{BlendFactor, MeshGeometry};
use particle_sim::{NifEffectDesc, ParticleEffectParams};
use serde_json::Value;

use super::*;
use crate::validate::{glb_json, validate_glb};

const EPSILON: f32 = 1e-4;
/// The synthetic triangle's half side.
const HALF: f32 = 20.0;
const EMISSIVE: NiColor = NiColor { r: 1.0, g: 0.5, b: 0.0 };
/// The mover's translation keys: (time, z).
const MOVE_KEYS: [(f32, f32); 2] = [(0.0, 0.0), (1.0, 10.0)];
/// The blinker hides at this time.
const HIDE_TIME: f32 = 0.5;
const MATERIAL_ALPHA: f32 = 0.75;
const ALPHA_REFERENCE: u8 = 128;

fn no_textures(_: &str) -> Option<Vec<u8>> {
    None
}

fn f(value: &Value) -> f32 {
    value.as_f64().unwrap() as f32
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < EPSILON
}

/// A node with an emissive triangle, as a parsed NIF.
fn triangle_nif() -> NifFile {
    let mut root = NiNode::default();
    root.name = NifString::from("Scene Root");
    root.children = vec![Ref::new(1)];
    let mut shape = NiTriShape::default();
    shape.name = NifString::from("Triangle");
    shape.model_data = Ref::new(2);
    shape.property_list = vec![Ref::new(3)];
    let mut data = NiTriShapeData::default();
    data.has_vertex = NiBool::TRUE;
    data.vertex = vec![
        NiPoint3 { x: -HALF, y: 0.0, z: -HALF },
        NiPoint3 { x: HALF, y: 0.0, z: -HALF },
        NiPoint3 { x: -HALF, y: 0.0, z: HALF },
    ];
    data.vertices = data.vertex.len() as u16;
    data.has_list = NiBool::TRUE;
    data.tri_list = vec![0, 1, 2];
    data.tri_list_length = data.tri_list.len() as u32;
    data.triangles = 1;
    let mut material = NiMaterialProperty::default();
    material.emit = EMISSIVE;
    NifFile {
        header: Header::default(),
        blocks: vec![root.into(), shape.into(), data.into(), material.into()],
        roots: vec![Ref::new(0)],
    }
}

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

fn timing(cycle: CycleType, stop_time: f32) -> ControllerTiming {
    ControllerTiming {
        cycle,
        active: true,
        app_init: false,
        reverse_direction: false,
        frequency: 1.0,
        phase: 0.0,
        start_time: 0.0,
        stop_time,
    }
}

fn key<T: Copy>(time: f32, value: T) -> Key<T> {
    Key { time, value, in_tangent: value, out_tangent: value, tcb: Tcb::default() }
}

/// root → mover (translation keys) → blinker (visibility off at [`HIDE_TIME`]) with a blended, alpha-tested
/// triangle whose material alpha is animated.
fn animated_desc() -> NifEffectDesc {
    let mut mover = node("Mover", Some(0), vec![2], NifNodeKind::Node);
    mover.transform.translation = [1.0, 2.0, 3.0];
    let blinker = node("Blinker", Some(1), vec![], NifNodeKind::Mesh { mesh: 0 });
    let mesh = NifMesh {
        node: 2,
        geometry: MeshGeometry {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            normals: vec![[0.0, -2.0, 0.0]; 3],
            colors: vec![[1.0, 0.0, 0.0, 0.5]; 3],
            uv_sets: vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]],
            triangles: vec![[0, 1, 2]],
        },
        bound_center: [0.0; 3],
        morph: None,
        render_state: NifRenderState {
            material: Some(0),
            alpha: NifAlpha {
                blend: true,
                src: BlendFactor::SrcAlpha,
                dst: BlendFactor::One,
                test: true,
                test_function: CompareFunction::Greater,
                test_reference: ALPHA_REFERENCE,
                no_sorter: false,
            },
            ..NifRenderState::default()
        },
    };
    let translation = Curve { kind: KeyKind::Linear, keys: MOVE_KEYS.iter().map(|&(t, z)| key(t, [0.0, 0.0, z])).collect() };
    let move_controller = NifController {
        owner: Some(1),
        target: ControllerTarget::Node(1),
        timing: timing(CycleType::Clamp, MOVE_KEYS[1].0),
        kind: NifControllerKind::Transform(NifInterpolator::Transform(TransformInterp {
            translation: Some(translation),
            ..TransformInterp::default()
        })),
    };
    let visibility = NifController {
        owner: Some(2),
        target: ControllerTarget::Node(2),
        timing: timing(CycleType::Clamp, MOVE_KEYS[1].0),
        kind: NifControllerKind::Visibility(BoolInterp {
            pose: None,
            keys: vec![BoolKey { time: 0.0, value: true }, BoolKey { time: HIDE_TIME, value: false }],
        }),
    };
    let alpha = NifController {
        owner: Some(2),
        target: ControllerTarget::Material(0),
        timing: timing(CycleType::Clamp, MOVE_KEYS[1].0),
        kind: NifControllerKind::Alpha(particle_sim::nif::desc::FloatInterp {
            pose: None,
            keys: Some(Curve { kind: KeyKind::Linear, keys: vec![key(0.0, 1.0), key(1.0, 0.0)] }),
        }),
    };
    NifEffectDesc {
        name: "animated.nif".to_owned(),
        nodes: vec![node("Root", None, vec![1], NifNodeKind::Node), mover, blinker],
        root: Some(0),
        meshes: vec![mesh],
        materials: vec![NifMaterial { emissive: [0.2, 0.4, 0.6], alpha: MATERIAL_ALPHA, ..NifMaterial::default() }],
        controllers: vec![move_controller, visibility, alpha],
        ..NifEffectDesc::default()
    }
}

fn node_named<'a>(document: &'a Value, name: &str) -> (usize, &'a Value) {
    document["nodes"].as_array().unwrap().iter().enumerate().find(|(_, node)| node["name"] == name).unwrap()
}

/// The floats of accessor `accessor` of a GLB.
fn accessor_floats(bytes: &[u8], document: &Value, accessor: usize) -> Vec<f32> {
    let accessor = &document["accessors"][accessor];
    let view = &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
    let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let bin = &bytes[glb::GLB_HEADER_BYTES + glb::CHUNK_HEADER_BYTES * 2 + json_length..];
    let offset = view["byteOffset"].as_u64().unwrap() as usize;
    let length = view["byteLength"].as_u64().unwrap() as usize;
    bin[offset..offset + length].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
}

/// (times, values) of the channel of `node`'s `path`.
fn channel(bytes: &[u8], document: &Value, node: usize, path: &str) -> Option<(Vec<f32>, Vec<f32>, String)> {
    let animation = &document["animations"][0];
    let channel = animation["channels"].as_array()?.iter().find(|c| c["target"]["node"] == node && c["target"]["path"] == path)?;
    let sampler = &animation["samplers"][channel["sampler"].as_u64()? as usize];
    let times = accessor_floats(bytes, document, sampler["input"].as_u64()? as usize);
    let values = accessor_floats(bytes, document, sampler["output"].as_u64()? as usize);
    Some((times, values, sampler["interpolation"].as_str()?.to_owned()))
}

#[test]
fn parsed_nif_exports_hierarchy_mesh_and_material() {
    let export = export_nif_file(&triangle_nif(), "triangle.nif", &no_textures, &ExportOptions::default());
    let summary = validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!((summary.meshes, summary.materials, summary.animations), (1, 1, 0));
    let document = glb_json(&export.bytes).unwrap();
    // Axis root → scene root → triangle.
    let scene_roots = document["scenes"][0]["nodes"].as_array().unwrap();
    assert_eq!(scene_roots.len(), 1);
    let axis = &document["nodes"][scene_roots[0].as_u64().unwrap() as usize];
    let rotation: Vec<f32> = axis["rotation"].as_array().unwrap().iter().map(f).collect();
    assert!(rotation.iter().zip(AXIS_ROTATION).all(|(a, b)| close(*a, b)));
    let (root, root_json) = node_named(&document, "Scene Root");
    assert_eq!(axis["children"][0], root);
    let (triangle, triangle_json) = node_named(&document, "Triangle");
    assert_eq!(root_json["children"][0], triangle);
    assert_eq!(triangle_json["extras"]["gamebryo"]["kind"], "NiTriShape");
    let mesh = &document["meshes"][triangle_json["mesh"].as_u64().unwrap() as usize];
    let position = mesh["primitives"][0]["attributes"]["POSITION"].as_u64().unwrap() as usize;
    assert_eq!(document["accessors"][position]["count"], 3);
    assert_eq!(document["accessors"][position]["max"][0], f64::from(HALF));
    // Material: unlit, the emissive (converted to linear) as the base colour, the Gamebryo state in extras.
    let material = &document["materials"][mesh["primitives"][0]["material"].as_u64().unwrap() as usize];
    let base: Vec<f32> = material["pbrMetallicRoughness"]["baseColorFactor"].as_array().unwrap().iter().map(f).collect();
    let [r, g, b] = material::linear_rgb([EMISSIVE.r, EMISSIVE.g, EMISSIVE.b]);
    assert_eq!(base, vec![r, g, b, 1.0]);
    assert!(material["extensions"].get(material::EXT_MATERIALS_UNLIT).is_some());
    assert_eq!(material.get("alphaMode"), None);
    let render_state = &material["extras"]["gamebryo"]["render_state"];
    assert_eq!(render_state["material"]["emissive"][1], f64::from(EMISSIVE.g));
    assert_eq!(render_state["cull"], "Back");
}

#[test]
fn animation_samples_transforms_visibility_and_material() {
    let desc = animated_desc();
    let export = export_nif(&desc, &no_textures, &ExportOptions::default());
    let summary = validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(summary.animations, 1);
    let document = glb_json(&export.bytes).unwrap();
    assert_eq!(document["animations"][0]["name"], "animated.nif");

    // The root's stored transform is replaced; the mover's keys replace its translation.
    let (mover, mover_json) = node_named(&document, "Mover");
    assert_eq!(mover_json.get("translation"), None, "the value at time 0 is the first key, (0, 0, 0)");
    let (times, values, interpolation) = channel(&export.bytes, &document, mover, "translation").unwrap();
    assert_eq!(interpolation, "LINEAR");
    assert!(close(*times.last().unwrap(), MOVE_KEYS[1].0));
    let at = |time: f32| times.iter().position(|&t| (t - time).abs() < anim::TIME_EPSILON).unwrap();
    assert!(close(values[at(0.5) * 3 + 2], 5.0));
    assert!(close(values[at(1.0) * 3 + 2], MOVE_KEYS[1].1));
    assert!(values.chunks(3).all(|v| v[0] == 0.0 && v[1] == 0.0));
    assert!(channel(&export.bytes, &document, mover, "rotation").is_none(), "constant channels are dropped");

    // Visibility: STEP scale 1 → 0 at the key.
    let (blinker, _) = node_named(&document, "Blinker");
    let (times, values, interpolation) = channel(&export.bytes, &document, blinker, "scale").unwrap();
    assert_eq!(interpolation, "STEP");
    for (time, scale) in times.iter().zip(values.chunks(3)) {
        assert_eq!(scale[0], if *time < HIDE_TIME { 1.0 } else { 0.0 }, "at {time}");
    }

    // Material: blended (BLEND wins over the test), the alpha controller as an animation pointer.
    let material = &document["materials"][0];
    assert_eq!(material["alphaMode"], "BLEND");
    let gamebryo = &material["extras"]["gamebryo"]["render_state"]["alpha"];
    assert_eq!(gamebryo["dst_blend"]["name"], "One");
    assert_eq!(gamebryo["alpha_test_reference"], ALPHA_REFERENCE);
    assert_eq!(material["extras"]["gamebryo"]["controllers"][0]["kind"]["type"], "Alpha");
    let pointer = document["animations"][0]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["target"]["path"] == "pointer")
        .unwrap();
    assert_eq!(
        pointer["target"]["extensions"]["KHR_animation_pointer"]["pointer"],
        "/materials/0/pbrMetallicRoughness/baseColorFactor"
    );
    // Vertex colours are ignored by this material (source IGNORE): no COLOR_0, the raw colours kept.
    let attributes = &document["meshes"][0]["primitives"][0]["attributes"];
    assert!(attributes.get("COLOR_0").is_none());
    assert!(attributes.get(scene::RAW_COLOR_ATTRIBUTE).is_some());
    assert!(attributes.get("TEXCOORD_0").is_some());
    // The controller description is on the owning node.
    let (_, blinker_json) = node_named(&document, "Blinker");
    assert_eq!(blinker_json["extras"]["gamebryo"]["controllers"][0]["kind"]["type"], "Visibility");
}

#[test]
fn looping_parts_wrap_their_clock() {
    const PERIOD: f32 = 1.0;
    const CLIP: f32 = 2.5;
    let desc = animated_desc();
    let timeline = Timeline::new(&desc, CLIP, Some(PERIOD), anim::SAMPLE_RATE_HZ);
    assert!(timeline.times.windows(2).all(|pair| pair[1] > pair[0]));
    for (&t, &local) in timeline.times.iter().zip(&timeline.local) {
        assert!((0.0..PERIOD).contains(&local), "{t} → {local}");
        let expected = t - (t / PERIOD + anim::TIME_EPSILON).floor() * PERIOD;
        assert!(close(local, expected.max(0.0)), "{t} → {local}");
    }
    // The key time 0.5 of each cycle is sampled, with the step edge before it.
    for cycle in 0..2 {
        let key = cycle as f32 * PERIOD + HIDE_TIME;
        assert!(timeline.times.iter().any(|&t| close(t, key)));
        assert!(timeline.times.iter().any(|&t| close(t, key - anim::STEP_EDGE_SECONDS)));
    }
}

#[test]
fn texture_transforms_decompose_into_offset_rotation_scale() {
    const ROTATION: f32 = 0.7;
    for method in [TextureTransformMethod::Maya, TextureTransformMethod::MayaDeprecated, TextureTransformMethod::Max] {
        let transform = NifTextureTransform {
            translate: [0.25, -0.5],
            scale: [2.0, 2.0],
            rotate: ROTATION,
            method,
            center: [0.5, 0.5],
        };
        let (offset, rotation, scale, exact) = material::texture_transform_members(&transform);
        assert!(exact, "{method:?}");
        // glTF: T * R * S with R = [[c, s], [-s, c]].
        let (s, c) = rotation.sin_cos();
        let rebuilt = [[c * scale[0], s * scale[1], offset[0]], [-s * scale[0], c * scale[1], offset[1]]];
        let matrix = texture_transform_matrix(&transform).rows;
        for (row, expected) in rebuilt.iter().zip(matrix) {
            for (a, b) in row.iter().zip(expected) {
                assert!(close(*a, b), "{method:?}: {rebuilt:?} vs {matrix:?}");
            }
        }
    }
}

#[test]
fn rotation_matrices_become_quaternions() {
    let quat = Quat::from_euler_xyz(0.3, -1.1, 2.0);
    let matrix = quat.to_mat3();
    let ([x, y, z, w], sign) = anim::matrix_to_quaternion(&matrix);
    assert_eq!(sign, 1.0);
    let same = [w, x, y, z].iter().zip([quat.w, quat.x, quat.y, quat.z]).all(|(a, b)| close(*a, b));
    let opposite = [w, x, y, z].iter().zip([quat.w, quat.x, quat.y, quat.z]).all(|(a, b)| close(*a, -b));
    assert!(same || opposite);
    // A reflection: the negated rotation with a negative scale.
    let mut reflected = matrix;
    reflected.rows = reflected.rows.map(|row| row.map(|value| -value));
    let (_, sign) = anim::matrix_to_quaternion(&reflected);
    assert_eq!(sign, -1.0);
}

#[test]
fn effects_place_their_parts() {
    const DURATION: f32 = 0.5;
    const CLIP: f32 = 1.0;
    let params = ParticleEffectParams { offset_x: 1.0, scale: -2.0, duration_s: DURATION, ..ParticleEffectParams::default() };
    let parts = vec![
        EffectPart { label: "0 animated.nif".to_owned(), params, content: PartContent::Nif(animated_desc()) },
        EffectPart {
            label: "1 missing.nif".to_owned(),
            params: ParticleEffectParams { looping: true, ..ParticleEffectParams::default() },
            content: PartContent::Unavailable("not found".to_owned()),
        },
    ];
    let options = ExportOptions { clip_length: Some(CLIP), ..ExportOptions::default() };
    let export = export_effect("effect", &parts, &no_textures, &options);
    validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert!(export.warnings.iter().any(|warning| warning.contains("not found")));
    let document = glb_json(&export.bytes).unwrap();
    let (part, part_json) = node_named(&document, "part 0: 0 animated.nif");
    let placement = EffectPlacement::seen_by(Vec3::ZERO, &default_camera());
    let offset = params.node_offset(&placement).to_array();
    let translation: Vec<f32> = part_json["translation"].as_array().unwrap().iter().map(f).collect();
    assert!(translation.iter().zip(offset).all(|(a, b)| close(*a, b)));
    assert_eq!(part_json["scale"][0], 2.0);
    // The non-looping part is hidden at its duration.
    let (times, values, interpolation) = channel(&export.bytes, &document, part, "scale").unwrap();
    assert_eq!((times, interpolation), (vec![0.0, DURATION], "STEP".to_owned()));
    assert_eq!(values, vec![2.0, 2.0, 2.0, 0.0, 0.0, 0.0]);
    assert_eq!(part_json["extras"]["gamebryo"]["effect_part"]["duration_s"], f64::from(DURATION));
}

/// A box emitter firing particles that live the whole clip, on node "Particles".
fn fountain_desc() -> NifEffectDesc {
    use particle_sim::nif::desc::{EmitterShape, NifEmitter, NifModifier, NifModifierKind, NifParticleSystem};
    let emitter = NifEmitter {
        speed: 10.0,
        speed_var: 0.0,
        declination: 0.0,
        declination_var: 0.0,
        planar_angle: 0.0,
        planar_angle_var: 0.0,
        initial_color: [1.0; 4],
        initial_radius: 1.0,
        radius_var: 0.0,
        life_span: 10.0,
        life_span_var: 0.0,
        shape: EmitterShape::Box { object: Some(0), width: 1.0, height: 1.0, depth: 1.0 },
    };
    let system = NifParticleSystem {
        node: 1,
        world_space: true,
        capacity: 16,
        has_colors: true,
        has_radii: true,
        has_sizes: true,
        has_rotation_angles: false,
        has_rotation_axes: false,
        modifiers: vec![
            NifModifier { name: "emitter".into(), order: 0, active: true, kind: NifModifierKind::Emitter(emitter) },
            NifModifier { name: "position".into(), order: 1, active: true, kind: NifModifierKind::Position },
        ],
        render_state: NifRenderState::default(),
        mesh_particles: false,
        bound_center: [0.0; 3],
    };
    let birth = NifController {
        owner: Some(1),
        target: ControllerTarget::ParticleSystem(0),
        timing: timing(CycleType::Loop, 1.0),
        kind: NifControllerKind::Emitter {
            modifier: Some(0),
            birth_rate: Some(particle_sim::nif::desc::FloatInterp { pose: Some(20.0), keys: None }),
            active: Some(BoolInterp { pose: Some(true), keys: Vec::new() }),
        },
    };
    let update = NifController {
        owner: Some(1),
        target: ControllerTarget::ParticleSystem(0),
        timing: timing(CycleType::Loop, 1.0),
        kind: NifControllerKind::ParticleUpdate,
    };
    NifEffectDesc {
        name: "fountain.nif".to_owned(),
        nodes: vec![
            node("Root", None, vec![1], NifNodeKind::Node),
            node("Particles", Some(0), vec![], NifNodeKind::ParticleSystem { system: 0 }),
        ],
        root: Some(0),
        particle_systems: vec![system],
        controllers: vec![birth, update],
        ..NifEffectDesc::default()
    }
}

#[test]
fn baked_frames_toggle_their_visibility() {
    let options = ExportOptions { bake: Some(BakeOptions::default()), ..ExportOptions::default() };
    let export = export_nif(&fountain_desc(), &no_textures, &options);
    validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let document = glb_json(&export.bytes).unwrap();
    let (_, particles) = node_named(&document, "Particles");
    assert_eq!(particles["extras"]["gamebryo"]["particle_system"]["modifiers"][0]["kind"]["type"], "Emitter");
    let (_, baked) = node_named(&document, "fountain.nif baked particles");
    let frames = baked["children"].as_array().unwrap();
    assert!(frames.len() > 1, "{} frames", frames.len());
    // Frame k shows from its time until the next frame's.
    let second = frames[1].as_u64().unwrap() as usize;
    let (times, values, interpolation) = channel(&export.bytes, &document, second, "scale").unwrap();
    assert_eq!(interpolation, "STEP");
    assert_eq!(values.chunks(3).map(|v| v[0]).collect::<Vec<_>>(), vec![0.0, 1.0, 0.0]);
    assert!(times[1] < times[2]);
}

#[test]
fn baked_sprites_keep_their_texture_stages_in_extras() {
    const TEXEL: [u8; 4] = [u8::MAX; 4];
    // Glow map only, as the client's flash sprites: no base map to export.
    let mut desc = fountain_desc();
    desc.textures = vec![NifTexture {
        index: 0,
        source: NifTextureSource::Embedded { width: 1, height: 1, levels: vec![TEXEL.to_vec()] },
        mipmapped: false,
    }];
    desc.texturings = vec![NifTexturing {
        apply_mode: TextureApplyMode::Modulate,
        maps: vec![NifTextureMap {
            slot: TextureSlot::Glow,
            texture: Some(0),
            addressing: TextureAddressing { wrap_s: false, wrap_t: false },
            filter: 1,
            uv_set: 0,
            transform: None,
        }],
    }];
    desc.particle_systems[0].render_state.texturing = Some(0);
    let options = ExportOptions { bake: Some(BakeOptions::default()), ..ExportOptions::default() };
    let export = export_nif(&desc, &no_textures, &options);
    validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let document = glb_json(&export.bytes).unwrap();
    let sprites = document["materials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|material| material["name"].as_str().unwrap().contains("sprites"))
        .expect("a baked sprite material");
    assert!(sprites["pbrMetallicRoughness"].get("baseColorTexture").is_none());
    let gamebryo = &sprites["extras"]["gamebryo"];
    assert_eq!(gamebryo["apply_mode"], "Modulate");
    assert_eq!(gamebryo["stages"].as_array().unwrap().len(), 1);
    assert_eq!(gamebryo["stages"][0]["slot"], "Glow");
}

/// [`animated_desc`] with an embedded texture as its base and glow maps, both scrolled by a texture transform
/// controller.
fn glowing_desc() -> NifEffectDesc {
    const SCROLL_KEYS: [(f32, f32); 2] = [(0.0, 0.0), (1.0, 1.0)];
    const TEXEL: [u8; 4] = [u8::MAX; 4];
    let mut desc = animated_desc();
    desc.textures = vec![NifTexture {
        index: 0,
        source: NifTextureSource::Embedded { width: 1, height: 1, levels: vec![TEXEL.to_vec()] },
        mipmapped: false,
    }];
    let map = |slot| NifTextureMap {
        slot,
        texture: Some(0),
        addressing: TextureAddressing { wrap_s: true, wrap_t: true },
        filter: 1,
        uv_set: 0,
        transform: None,
    };
    desc.texturings = vec![NifTexturing {
        apply_mode: TextureApplyMode::Modulate,
        maps: vec![map(TextureSlot::Base), map(TextureSlot::Glow)],
    }];
    desc.meshes[0].render_state.texturing = Some(0);
    for slot in [TextureSlot::Base, TextureSlot::Glow] {
        desc.controllers.push(NifController {
            owner: Some(2),
            target: ControllerTarget::Texturing(0),
            timing: timing(CycleType::Clamp, SCROLL_KEYS[1].0),
            kind: NifControllerKind::TextureTransform {
                slot,
                member: TextureTransformMember::TranslateU,
                interp: FloatInterp {
                    pose: None,
                    keys: Some(Curve { kind: KeyKind::Linear, keys: SCROLL_KEYS.iter().map(|&(t, u)| key(t, u)).collect() }),
                },
            },
        });
    }
    desc
}

/// `bytes` with its JSON chunk replaced by `document`.
fn with_document(bytes: &[u8], document: &Value) -> Vec<u8> {
    let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let bin = &bytes[glb::GLB_HEADER_BYTES + glb::CHUNK_HEADER_BYTES + json_length..];
    let mut json = serde_json::to_vec(document).unwrap();
    while json.len() % glb::GLB_ALIGNMENT != 0 {
        json.push(b' ');
    }
    let total = glb::GLB_HEADER_BYTES + glb::CHUNK_HEADER_BYTES + json.len() + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&glb::GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&glb::GLB_VERSION.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&glb::CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(bin);
    out
}

#[test]
fn unlit_materials_keep_only_the_base_map() {
    let export = export_nif(&glowing_desc(), &no_textures, &ExportOptions::default());
    validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let document = glb_json(&export.bytes).unwrap();
    let material = &document["materials"][0];
    assert!(material["pbrMetallicRoughness"]["baseColorTexture"]["extensions"][material::EXT_TEXTURE_TRANSFORM].is_object());
    // The glow map is in extras only: no emissive texture or factor, no channel on them.
    assert!(material.get("emissiveTexture").is_none() && material.get("emissiveFactor").is_none());
    let approximations = material["extras"]["gamebryo"]["approximations"].as_array().unwrap();
    assert!(approximations.iter().any(|approximation| approximation == "glow map in extras only"));
    let pointers: Vec<&str> = document["animations"][0]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|channel| channel["target"]["extensions"]["KHR_animation_pointer"]["pointer"].as_str())
        .collect();
    assert!(pointers.contains(&"/materials/0/pbrMetallicRoughness/baseColorTexture/extensions/KHR_texture_transform/offset"));
    assert!(pointers.iter().all(|pointer| !pointer.contains("emissive")), "{pointers:?}");

    // The validator rejects an unlit material with an emissive texture, and a channel animating its transform.
    let mut emissive = document.clone();
    let base = material["pbrMetallicRoughness"]["baseColorTexture"].clone();
    emissive["materials"][0]["emissiveTexture"] = base;
    let errors = validate_glb(&with_document(&export.bytes, &emissive)).unwrap_err();
    assert!(errors.iter().any(|error| error.contains("unlit with /emissiveTexture")), "{errors:#?}");
    let channels = emissive["animations"][0]["channels"].as_array_mut().unwrap();
    let mut channel = channels[0].clone();
    channel["target"] = serde_json::json!({ "path": "pointer", "extensions": { "KHR_animation_pointer": {
        "pointer": "/materials/0/emissiveTexture/extensions/KHR_texture_transform/offset",
    }}});
    channels.push(channel);
    let errors = validate_glb(&with_document(&export.bytes, &emissive)).unwrap_err();
    assert!(errors.iter().any(|error| error.contains("animates /emissiveTexture of unlit material 0")), "{errors:#?}");
    // A zero emissive factor is the default, and allowed.
    let mut zero = document.clone();
    zero["materials"][0]["emissiveFactor"] = serde_json::json!([0.0, 0.0, 0.0]);
    validate_glb(&with_document(&export.bytes, &zero)).unwrap_or_else(|errors| panic!("{errors:#?}"));
}

/// A display-space mid-tone, RGBA (its linear value differs from it).
const MID_TONE: [f32; 4] = [0.5, 0.25, 0.75, 0.5];
/// The emissive colour controller's Bezier keys: (time, display-space colour).
const EMISSIVE_KEYS: [(f32, [f32; 3]); 2] = [(0.0, [0.2, 0.4, 0.6]), (1.0, [0.8, 0.6, 0.4])];
/// The light's display-space diffuse colour.
const LIGHT_DIFFUSE: [f32; 3] = [0.5, 0.2, 0.8];
const LIGHT_DIMMER: f32 = 0.6;
/// Tolerance of a conversion compared with its reference.
const CONVERSION_EPSILON: f32 = 1e-6;

/// The values of the `KHR_animation_pointer` channel of `pointer`: (times, values).
fn pointer_channel(bytes: &[u8], document: &Value, pointer: &str) -> Option<(Vec<f32>, Vec<f32>)> {
    let animation = &document["animations"][0];
    let channel = animation["channels"]
        .as_array()?
        .iter()
        .find(|c| c["target"]["extensions"]["KHR_animation_pointer"]["pointer"] == pointer)?;
    let sampler = &animation["samplers"][channel["sampler"].as_u64()? as usize];
    let times = accessor_floats(bytes, document, sampler["input"].as_u64()? as usize);
    Some((times, accessor_floats(bytes, document, sampler["output"].as_u64()? as usize)))
}

#[test]
fn srgb_eotf_matches_its_reference_values() {
    // IEC 61966-2-1: the segments meet at 0.04045 (linear 0.0031308); display mid-grey is linear 0.2140.
    const SEGMENT_END: f32 = 0.04045;
    const SEGMENT_END_LINEAR: f32 = 0.003_130_8;
    const MID_GREY: f32 = 0.5;
    const MID_GREY_LINEAR: f32 = 0.214_041_14;
    assert_eq!(material::srgb_to_linear(0.0), 0.0);
    assert!((material::srgb_to_linear(1.0) - 1.0).abs() < CONVERSION_EPSILON);
    assert!((material::srgb_to_linear(MID_GREY) - MID_GREY_LINEAR).abs() < CONVERSION_EPSILON);
    assert!((material::srgb_to_linear(SEGMENT_END) - SEGMENT_END_LINEAR).abs() < CONVERSION_EPSILON);
    assert!((material::srgb_to_linear(SEGMENT_END + f32::EPSILON) - SEGMENT_END_LINEAR).abs() < CONVERSION_EPSILON);
    // Clamped, alpha kept.
    assert_eq!(material::linear_rgba([2.0, -1.0, 0.0, MID_GREY]), [1.0, 0.0, 0.0, MID_GREY]);
}

#[test]
fn colours_are_converted_to_linear_once() {
    // Vertex colours drawn as RGBA: COLOR_0 is the raw colour converted, the raw one kept.
    let mut desc = animated_desc();
    desc.meshes[0].geometry.colors = vec![MID_TONE; 3];
    desc.meshes[0].render_state.vertex_color =
        NifVertexColor { source: VertexColorSource::Emissive, lighting: LightingMode::Emissive };
    let export = export_nif(&desc, &no_textures, &ExportOptions::default());
    validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let document = glb_json(&export.bytes).unwrap();
    let attributes = &document["meshes"][0]["primitives"][0]["attributes"];
    let color = accessor_floats(&export.bytes, &document, attributes["COLOR_0"].as_u64().unwrap() as usize);
    let raw_accessor = attributes[scene::RAW_COLOR_ATTRIBUTE].as_u64().unwrap();
    let raw = accessor_floats(&export.bytes, &document, raw_accessor as usize);
    assert!(color.chunks(4).all(|c| c == material::linear_rgba(MID_TONE)), "{color:?}");
    assert!(raw.chunks(4).all(|c| c == MID_TONE), "{raw:?}");
    assert!(document["materials"][0]["extras"]["gamebryo"]["color_space"].is_string());
    // The validator rejects unconverted vertex colours.
    let mut unconverted = document.clone();
    unconverted["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = serde_json::json!(raw_accessor);
    let errors = validate_glb(&with_document(&export.bytes, &unconverted)).unwrap_err();
    assert!(errors.iter().any(|error| error.contains("converted to linear once")), "{errors:#?}");

    // An emissive colour controller with Bezier keys, and a light.
    let mut desc = animated_desc();
    desc.controllers[2] = NifController {
        owner: Some(2),
        target: ControllerTarget::Material(0),
        timing: timing(CycleType::Clamp, EMISSIVE_KEYS[1].0),
        kind: NifControllerKind::MaterialColor {
            field: MaterialColorField::Emissive,
            interp: Point3Interp {
                pose: None,
                keys: Some(Curve { kind: KeyKind::Bezier, keys: EMISSIVE_KEYS.iter().map(|&(t, c)| key(t, c)).collect() }),
            },
        },
    };
    let lamp = desc.nodes.len();
    desc.nodes[0].children.push(lamp);
    desc.nodes.push(node("Lamp", Some(0), vec![], NifNodeKind::Light { light: 0 }));
    desc.lights = vec![NifLight {
        node: lamp,
        kind: NifLightKind::Directional,
        on: true,
        dimmer: LIGHT_DIMMER,
        ambient: [0.0; 3],
        diffuse: LIGHT_DIFFUSE,
        specular: [0.0; 3],
        unaffected_nodes: Vec::new(),
    }];
    let export = export_nif(&desc, &no_textures, &ExportOptions::default());
    validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let document = glb_json(&export.bytes).unwrap();
    // The static factor and every sample of the channel: the client's colour at that time, converted once.
    let material = &document["materials"][0];
    let base: Vec<f32> = material["pbrMetallicRoughness"]["baseColorFactor"].as_array().unwrap().iter().map(f).collect();
    let [r, g, b] = material::linear_rgb(EMISSIVE_KEYS[0].1);
    assert_eq!(base, vec![r, g, b, MATERIAL_ALPHA]);
    let (times, values) =
        pointer_channel(&export.bytes, &document, "/materials/0/pbrMetallicRoughness/baseColorFactor").unwrap();
    for (&t, value) in times.iter().zip(values.chunks(4)) {
        let [r, g, b] = material::linear_rgb(anim::material_state(&desc, 0, t).emissive);
        assert_eq!(value, [r, g, b, MATERIAL_ALPHA], "at {t}");
    }
    let approximations = material["extras"]["gamebryo"]["approximations"].as_array().unwrap();
    assert!(approximations.iter().any(|a| a.as_str().unwrap().contains("linear space")), "{approximations:?}");
    // The raw keys stay in extras.
    let controller = &material["extras"]["gamebryo"]["controllers"][0];
    assert_eq!(controller["kind"]["type"], "MaterialColor", "{controller:#}");
    // The light: colour converted, intensity the dimmer, the raw colour in the node's extras.
    let light = &document["extensions"][scene::EXT_LIGHTS_PUNCTUAL]["lights"][0];
    let color: Vec<f32> = light["color"].as_array().unwrap().iter().map(f).collect();
    assert_eq!(color, material::linear_rgb(LIGHT_DIFFUSE));
    assert_eq!(f(&light["intensity"]), LIGHT_DIMMER);
    let (_, lamp) = node_named(&document, "Lamp");
    let diffuse: Vec<f32> = lamp["extras"]["gamebryo"]["light"]["diffuse"].as_array().unwrap().iter().map(f).collect();
    assert_eq!(diffuse, LIGHT_DIFFUSE);
    assert!(lamp["extras"]["gamebryo"]["light"]["color_space"].is_string());
}

#[test]
fn baked_particle_colours_are_converted_to_linear() {
    let mut desc = fountain_desc();
    let particle_sim::nif::desc::NifModifierKind::Emitter(emitter) = &mut desc.particle_systems[0].modifiers[0].kind
    else {
        unreachable!("the fountain's first modifier is its emitter")
    };
    emitter.initial_color = MID_TONE;
    // The sprites take the particle colour as it is.
    desc.particle_systems[0].render_state.vertex_color =
        NifVertexColor { source: VertexColorSource::Emissive, lighting: LightingMode::Emissive };
    let options = ExportOptions { bake: Some(BakeOptions::default()), ..ExportOptions::default() };
    let export = export_nif(&desc, &no_textures, &options);
    validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let document = glb_json(&export.bytes).unwrap();
    let (_, baked) = node_named(&document, "fountain.nif baked particles");
    assert!(baked["extras"]["gamebryo"]["color_space"].is_string());
    let expected = material::linear_rgba(MID_TONE);
    for frame in baked["children"].as_array().unwrap() {
        let mesh = &document["meshes"][document["nodes"][frame.as_u64().unwrap() as usize]["mesh"].as_u64().unwrap() as usize];
        let accessor = mesh["primitives"][0]["attributes"]["COLOR_0"].as_u64().unwrap() as usize;
        let colors = accessor_floats(&export.bytes, &document, accessor);
        assert!(colors.chunks(4).all(|c| c.iter().zip(expected).all(|(a, b)| (a - b).abs() < CONVERSION_EPSILON)), "{colors:?}");
    }
}
