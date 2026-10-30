//! Exports of synthetic scenes, checked structurally and by value.

use gamebryo_nif::{
    Header, NiBool, NiColor, NiMaterialProperty, NiNode, NiPoint3, NiTriShape, NiTriShapeData, NifFile, NifString, Ref,
};
use particle_sim::nif::desc::{
    BoolInterp, BoolKey, CompareFunction, ControllerTarget, ControllerTiming, Curve, CycleType, Key, KeyKind, NifAlpha,
    NifController, NifControllerKind, NifInterpolator, NifMaterial, NifMesh, NifNode, NifNodeKind, NifRenderState,
    NifTextureTransform, NifTransform, Tcb, TextureTransformMethod, TransformInterp,
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
    // Material: unlit, the emissive as the base colour, the Gamebryo state in extras.
    let material = &document["materials"][mesh["primitives"][0]["material"].as_u64().unwrap() as usize];
    let base: Vec<f32> = material["pbrMetallicRoughness"]["baseColorFactor"].as_array().unwrap().iter().map(f).collect();
    assert_eq!(base, vec![EMISSIVE.r, EMISSIVE.g, EMISSIVE.b, 1.0]);
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

#[test]
fn baked_frames_toggle_their_visibility() {
    use particle_sim::nif::desc::{EmitterShape, NifEmitter, NifModifier, NifModifierKind, NifParticleSystem};
    // A box emitter firing particles that live the whole clip.
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
    let desc = NifEffectDesc {
        name: "fountain.nif".to_owned(),
        nodes: vec![
            node("Root", None, vec![1], NifNodeKind::Node),
            node("Particles", Some(0), vec![], NifNodeKind::ParticleSystem { system: 0 }),
        ],
        root: Some(0),
        particle_systems: vec![system],
        controllers: vec![birth, update],
        ..NifEffectDesc::default()
    };
    let options = ExportOptions { bake: Some(BakeOptions::default()), ..ExportOptions::default() };
    let export = export_nif(&desc, &no_textures, &options);
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
