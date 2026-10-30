//! Round trips of synthetic descs that use every tag, and the decoder's rejections.

use std::fmt::Debug;

use super::*;
use crate::chaos::desc::ChaosSequenceDesc;

/// Writes `value` alone and reads it back.
fn round_trip<T: Codec + PartialEq + Debug>(value: &T) {
    let mut writer = Writer { bytes: Vec::new() };
    value.encode(&mut writer);
    let mut reader = Reader { bytes: &writer.bytes, pos: 0 };
    assert_eq!(&T::decode(&mut reader).expect("decodes"), value);
    reader.finish().expect("no trailing bytes");
}

#[test]
fn every_fieldless_enum_value_round_trips() {
    BlendFactor::ALL.iter().for_each(round_trip);
    [
        ChaosParticleType::None,
        ChaosParticleType::Point,
        ChaosParticleType::Mesh,
        ChaosParticleType::Billboard,
        ChaosParticleType::Emitter,
        ChaosParticleType::Axial,
        ChaosParticleType::Light,
        ChaosParticleType::Test,
    ]
    .iter()
    .for_each(round_trip);
    [
        ChaosEventKind::Scale,
        ChaosEventKind::Color,
        ChaosEventKind::RedColor,
        ChaosEventKind::GreenColor,
        ChaosEventKind::BlueColor,
        ChaosEventKind::Alpha,
        ChaosEventKind::Velocity,
        ChaosEventKind::VelocityX,
        ChaosEventKind::VelocityY,
        ChaosEventKind::VelocityZ,
        ChaosEventKind::Thrust,
        ChaosEventKind::EventTimer,
        ChaosEventKind::Rotation,
        ChaosEventKind::Length,
    ]
    .iter()
    .for_each(round_trip);
    BillboardMode::ALL.iter().for_each(round_trip);
    CompareFunction::ALL.iter().for_each(round_trip);
    [CullMode::Back, CullMode::Front, CullMode::None].iter().for_each(round_trip);
    [VertexColorSource::Ignore, VertexColorSource::Emissive, VertexColorSource::AmbientDiffuse].iter().for_each(round_trip);
    [LightingMode::Emissive, LightingMode::EmissiveAmbientDiffuse].iter().for_each(round_trip);
    [TextureApplyMode::Replace, TextureApplyMode::Decal, TextureApplyMode::Modulate].iter().for_each(round_trip);
    [TextureTransformMethod::MayaDeprecated, TextureTransformMethod::Max, TextureTransformMethod::Maya]
        .iter()
        .for_each(round_trip);
    [KeyKind::Linear, KeyKind::Bezier, KeyKind::Tcb, KeyKind::Step].iter().for_each(round_trip);
    [CycleType::Loop, CycleType::Reverse, CycleType::Clamp].iter().for_each(round_trip);
    [BombDecay::None, BombDecay::Linear, BombDecay::Exponential].iter().for_each(round_trip);
    [BombSymmetry::Spherical, BombSymmetry::Cylindrical, BombSymmetry::Planar].iter().for_each(round_trip);
    [MeshEmitterVelocity::Normals, MeshEmitterVelocity::Random, MeshEmitterVelocity::Direction]
        .iter()
        .for_each(round_trip);
    [
        MeshEmission::Vertices,
        MeshEmission::FaceCenter,
        MeshEmission::EdgeCenter,
        MeshEmission::FaceSurface,
        MeshEmission::EdgeSurface,
    ]
    .iter()
    .for_each(round_trip);
    [MaterialColorField::Ambient, MaterialColorField::Diffuse, MaterialColorField::Specular, MaterialColorField::Emissive]
        .iter()
        .for_each(round_trip);
    [
        TextureTransformMember::TranslateU,
        TextureTransformMember::TranslateV,
        TextureTransformMember::Rotate,
        TextureTransformMember::ScaleU,
        TextureTransformMember::ScaleV,
    ]
    .iter()
    .for_each(round_trip);
    [
        ModifierParam::EmitterSpeed,
        ModifierParam::EmitterLifeSpan,
        ModifierParam::EmitterInitialRadius,
        ModifierParam::GravityStrength,
        ModifierParam::InitialRotationSpeed,
        ModifierParam::InitialRotationSpeedVar,
    ]
    .iter()
    .for_each(round_trip);
    TextureSlot::FIXED.iter().chain(&[TextureSlot::Decal(0), TextureSlot::Decal(3)]).for_each(round_trip);
}

#[test]
fn fieldless_enum_tags_are_distinct() {
    let mut writer = Writer { bytes: Vec::new() };
    CompareFunction::ALL.iter().for_each(|function| function.encode(&mut writer));
    let mut tags = writer.bytes.clone();
    tags.dedup();
    assert_eq!(tags.len(), CompareFunction::ALL.len());
}

fn chaos_desc() -> ChaosSystemDesc {
    let mut desc = ChaosSystemDesc::new("test system");
    desc.script_version = 1.5;
    desc.min_emit_time = Range::new(0.25, 0.75);
    let mut sequence = ChaosSequenceDesc::new("sparks");
    sequence.particle_type = ChaosParticleType::Axial;
    sequence.resource = "03002012_flare.tga".to_owned();
    sequence.src_blend = BlendFactor::SrcAlpha;
    sequence.dst_blend = BlendFactor::InvSrcAlpha;
    sequence.flags = ChaosSequenceFlags(ChaosSequenceFlags::CEL_ANIMATE | ChaosSequenceFlags::LOCAL);
    sequence.lifetime = Range::new(1.0, 2.0);
    sequence.emit_rate = Range::new(10.0, 20.0);
    sequence.num_particles = 42;
    sequence.loops = 3;
    sequence.gravity = VecRange { min: [0.0, 0.0, -1.0], max: [0.0, 0.0, -2.0] };
    sequence.inherited = VecRange { min: [0.5; 3], max: [0.5; 3] };
    sequence.chaos_velocity = VecRange { min: [-1.0; 3], max: [1.0; 3] };
    sequence.chaos_location = VecRange { min: [-0.5; 3], max: [0.5; 3] };
    sequence.burst = IntRange { min: 2, max: 5 };
    sequence.emission_boost = Range::new(0.1, 0.2);
    sequence.cel = ChaosCelSetup { frames: 16, u_step: 0.25, v_step: 0.25, columns: 4, rows: 4, fps: 12.0, looping: true };
    sequence.events = vec![
        ChaosEventDesc {
            kind: ChaosEventKind::Color,
            fade: false,
            time: Range::ZERO,
            value: ChaosEventValue { min: [1.0, 0.5, 0.25, 1.0], max: [1.0, 0.75, 0.5, 1.0] },
        },
        ChaosEventDesc {
            kind: ChaosEventKind::Scale,
            fade: true,
            time: Range::new(0.5, 1.0),
            value: ChaosEventValue { min: [2.0, 0.0, 0.0, 0.0], max: [3.0, 0.0, 0.0, 0.0] },
        },
    ];
    desc.sequences = vec![sequence, ChaosSequenceDesc::new("empty")];
    desc
}

#[test]
fn chaos_desc_round_trips() {
    let desc = chaos_desc();
    let bytes = encode_chaos_desc(&desc);
    assert_eq!(&bytes[..CHAOS_BLOB_MAGIC.len()], &CHAOS_BLOB_MAGIC);
    assert_eq!(decode_chaos_desc(&bytes).expect("decodes"), desc);
}

fn key<T: Copy + Default>(kind: KeyKind, time: f32, value: T, tangent: T) -> Key<T> {
    let mut key = Key { time, value, in_tangent: T::default(), out_tangent: T::default(), tcb: Tcb::default() };
    match kind {
        KeyKind::Bezier => (key.in_tangent, key.out_tangent) = (tangent, tangent),
        KeyKind::Tcb => key.tcb = Tcb { tension: 0.5, continuity: -0.5, bias: 0.25 },
        KeyKind::Linear | KeyKind::Step => {}
    }
    key
}

fn curve<T: Copy + Default>(kind: KeyKind, values: [T; 2], tangent: T) -> Curve<T> {
    Curve { kind, keys: vec![key(kind, 0.0, values[0], tangent), key(kind, 1.0, values[1], tangent)] }
}

fn float_interp(kind: KeyKind) -> FloatInterp {
    FloatInterp { pose: Some(0.5), keys: Some(curve(kind, [0.0, 1.0], 0.5)) }
}

fn bool_interp() -> BoolInterp {
    BoolInterp { pose: Some(true), keys: vec![BoolKey { time: 0.0, value: false }, BoolKey { time: 1.0, value: true }] }
}

fn point3_interp() -> Point3Interp {
    Point3Interp { pose: None, keys: Some(curve(KeyKind::Bezier, [[0.0; 3], [1.0, 2.0, 3.0]], [0.5; 3])) }
}

fn render_state() -> NifRenderState {
    NifRenderState {
        material: Some(0),
        texturing: Some(0),
        alpha: NifAlpha {
            blend: true,
            src: BlendFactor::SrcAlpha,
            dst: BlendFactor::One,
            test: true,
            test_function: CompareFunction::Greater,
            test_reference: 128,
            no_sorter: true,
        },
        z_buffer: NifZBuffer { test: true, write: false, function: CompareFunction::LessEqual },
        vertex_color: NifVertexColor { source: VertexColorSource::Emissive, lighting: LightingMode::Emissive },
        cull: CullMode::None,
        specular: true,
    }
}

fn emitter(shape: EmitterShape) -> NifModifier {
    let emitter = NifModifierKind::Emitter(NifEmitter {
        speed: 1.0,
        speed_var: 0.1,
        declination: 0.2,
        declination_var: 0.3,
        planar_angle: 0.4,
        planar_angle_var: 0.5,
        initial_color: [1.0, 0.5, 0.25, 1.0],
        initial_radius: 2.0,
        radius_var: 0.6,
        life_span: 3.0,
        life_span_var: 0.7,
        shape,
    });
    modifier("emitter", emitter)
}

fn modifier(name: &str, kind: NifModifierKind) -> NifModifier {
    NifModifier { name: name.to_owned(), order: 1000, active: true, kind }
}

fn node(name: &str, parent: Option<usize>, kind: NifNodeKind) -> NifNode {
    NifNode {
        name: name.to_owned(),
        parent,
        children: Vec::new(),
        transform: NifTransform { translation: [1.0, 2.0, 3.0], scale: 2.0, ..NifTransform::IDENTITY },
        flags: AV_FLAG_APP_CULLED_FOR_TESTS,
        kind,
        extra_strings: vec!["zMode10".to_owned()],
        in_scene: parent.is_some(),
    }
}

/// Some `NiAVObject` flags value.
const AV_FLAG_APP_CULLED_FOR_TESTS: u16 = crate::nif::desc::AV_FLAG_APP_CULLED;

fn controller(target: ControllerTarget, kind: NifControllerKind) -> NifController {
    NifController {
        owner: Some(0),
        target,
        timing: ControllerTiming {
            cycle: CycleType::Reverse,
            active: true,
            app_init: true,
            reverse_direction: false,
            frequency: 1.0,
            phase: 0.5,
            start_time: 0.0,
            stop_time: 2.0,
        },
        kind,
    }
}

/// A desc that uses every node kind, texture source, modifier kind, emitter shape, interpolator, rotation curve,
/// key kind, controller target and controller kind.
fn nif_desc() -> NifEffectDesc {
    let mut root = node("root", None, NifNodeKind::Node);
    root.children = vec![1, 2, 3, 4];
    let nodes = vec![
        root,
        node("billboard", Some(0), NifNodeKind::Billboard(BillboardMode::RigidFaceCenter)),
        node("mesh", Some(0), NifNodeKind::Mesh { mesh: 0 }),
        node("particles", Some(0), NifNodeKind::ParticleSystem { system: 0 }),
        node("light", Some(0), NifNodeKind::Other { type_name: "NiPointLight".to_owned() }),
    ];
    let geometry = MeshGeometry {
        positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        colors: vec![[1.0; 4]; 3],
        uv_sets: vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]],
        triangles: vec![[0, 1, 2]],
    };
    let morph = NifMorph {
        relative_targets: true,
        targets: vec![
            NifMorphTarget { name: "base".to_owned(), initial_weight: 1.0, vertices: geometry.positions.clone() },
            NifMorphTarget { name: "up".to_owned(), initial_weight: 0.0, vertices: vec![[0.0, 0.0, 1.0]; 3] },
        ],
    };
    let meshes = vec![NifMesh { node: 2, geometry, morph: Some(morph), render_state: render_state() }];
    let modifiers = vec![
        emitter(EmitterShape::Box { object: Some(4), width: 1.0, height: 2.0, depth: 3.0 }),
        emitter(EmitterShape::Sphere { object: None, radius: 1.5 }),
        emitter(EmitterShape::Cylinder { object: Some(0), radius: 1.0, height: 4.0 }),
        emitter(EmitterShape::Mesh {
            meshes: vec![0],
            velocity: MeshEmitterVelocity::Direction,
            emission: MeshEmission::FaceSurface,
            emit_axis: [0.0, 0.0, 1.0],
        }),
        modifier("age", NifModifierKind::AgeDeath { spawn_on_death: true, spawn_modifier: Some((0, 6)) }),
        modifier("age2", NifModifierKind::AgeDeath { spawn_on_death: false, spawn_modifier: None }),
        modifier(
            "spawn",
            NifModifierKind::Spawn(NifSpawn {
                num_spawn_generations: 2,
                percentage_spawned: 0.5,
                min_num_to_spawn: 1,
                max_num_to_spawn: 3,
                spawn_speed_chaos: 0.1,
                spawn_dir_chaos: 0.2,
                life_span: 1.0,
                life_span_var: 0.3,
            }),
        ),
        modifier("position", NifModifierKind::Position),
        modifier("bounds", NifModifierKind::BoundUpdate),
        modifier(
            "grow",
            NifModifierKind::GrowFade { grow_time: 0.1, grow_generation: 1, fade_time: 0.2, fade_generation: 2 },
        ),
        modifier(
            "color",
            NifModifierKind::Color { colors: Some(curve(KeyKind::Linear, [[1.0; 4], [0.0; 4]], [0.0; 4])) },
        ),
        modifier("no colour", NifModifierKind::Color { colors: None }),
        modifier(
            "rotation",
            NifModifierKind::Rotation {
                initial_speed: 1.0,
                initial_speed_var: 0.1,
                initial_angle: 0.2,
                initial_angle_var: 0.3,
                random_speed_sign: true,
                random_initial_axis: false,
                initial_axis: [0.0, 1.0, 0.0],
            },
        ),
        modifier(
            "gravity",
            NifModifierKind::Gravity {
                object: Some(1),
                axis: [0.0, 0.0, -1.0],
                decay: 0.1,
                strength: 9.8,
                spherical: true,
                turbulence: 0.5,
                turbulence_scale: 2.0,
            },
        ),
        modifier(
            "drag",
            NifModifierKind::Drag { object: None, axis: [1.0, 0.0, 0.0], percentage: 0.5, range: 10.0, falloff: 20.0 },
        ),
        modifier(
            "bomb",
            NifModifierKind::Bomb {
                object: Some(0),
                axis: [0.0, 1.0, 0.0],
                decay: 0.2,
                delta_v: 3.0,
                decay_type: BombDecay::Exponential,
                symmetry: BombSymmetry::Cylindrical,
            },
        ),
        modifier("meshes", NifModifierKind::MeshUpdate { meshes: vec![2] }),
        modifier("other", NifModifierKind::Unsupported { type_name: "NiPSysColliderManager".to_owned() }),
    ];
    let particle_systems = vec![NifParticleSystem {
        node: 3,
        world_space: true,
        capacity: 64,
        has_colors: true,
        has_radii: true,
        has_sizes: false,
        has_rotation_angles: true,
        has_rotation_axes: false,
        modifiers,
        render_state: render_state(),
        mesh_particles: true,
    }];
    let materials = vec![NifMaterial { name: "material".to_owned(), emissive: [1.0; 3], ..NifMaterial::default() }];
    let texturings = vec![NifTexturing {
        apply_mode: TextureApplyMode::Decal,
        maps: vec![
            NifTextureMap {
                slot: TextureSlot::Base,
                texture: Some(0),
                addressing: TextureAddressing { wrap_s: true, wrap_t: false },
                filter: 2,
                uv_set: 0,
                transform: Some(NifTextureTransform { rotate: 0.5, ..NifTextureTransform::default() }),
            },
            NifTextureMap {
                slot: TextureSlot::Decal(2),
                texture: None,
                addressing: TextureAddressing { wrap_s: false, wrap_t: true },
                filter: 1,
                uv_set: 1,
                transform: None,
            },
        ],
    }];
    let textures = vec![
        NifTexture { index: 0, source: NifTextureSource::External { file_name: "flare.tga".to_owned() } },
        NifTexture { index: 1, source: NifTextureSource::Embedded { width: 2, height: 1, rgba: (0..8).collect() } },
        NifTexture { index: 2, source: NifTextureSource::Undecoded { format: 4 } },
    ];
    let transform = TransformInterp {
        translation_pose: Some([1.0, 2.0, 3.0]),
        rotation_pose: Some([1.0, 0.0, 0.0, 0.0]),
        scale_pose: None,
        translation: Some(curve(KeyKind::Tcb, [[0.0; 3], [1.0; 3]], [0.0; 3])),
        rotation: Some(RotationCurve::Quaternion {
            kind: KeyKind::Tcb,
            keys: vec![RotKey { time: 0.0, value: [1.0, 0.0, 0.0, 0.0], tcb: Tcb { tension: 0.1, continuity: 0.2, bias: 0.3 } }],
        }),
        scale: Some(curve(KeyKind::Step, [1.0, 2.0], 0.0)),
    };
    let euler = TransformInterp {
        rotation: Some(RotationCurve::Euler {
            axes: [Some(curve(KeyKind::Linear, [0.0, 1.0], 0.0)), None, Some(curve(KeyKind::Bezier, [0.0, 2.0], 0.5))],
        }),
        ..TransformInterp::default()
    };
    let path = PathInterp {
        path: Some(curve(KeyKind::Bezier, [[0.0; 3], [1.0; 3]], [0.25; 3])),
        percent: Some(curve(KeyKind::Linear, [0.0, 1.0], 0.0)),
        flags: 0x13,
        follow_axis: -1,
    };
    let controllers = vec![
        controller(ControllerTarget::Node(1), NifControllerKind::Transform(NifInterpolator::Transform(transform))),
        controller(ControllerTarget::Node(1), NifControllerKind::Transform(NifInterpolator::Transform(euler))),
        controller(ControllerTarget::Node(1), NifControllerKind::Transform(NifInterpolator::Path(path))),
        controller(ControllerTarget::Node(1), NifControllerKind::Transform(NifInterpolator::Float(float_interp(KeyKind::Step)))),
        controller(ControllerTarget::Node(1), NifControllerKind::Transform(NifInterpolator::Point3(point3_interp()))),
        controller(ControllerTarget::Node(1), NifControllerKind::Transform(NifInterpolator::Bool(bool_interp()))),
        controller(ControllerTarget::Node(4), NifControllerKind::Visibility(bool_interp())),
        controller(ControllerTarget::Material(0), NifControllerKind::Alpha(float_interp(KeyKind::Linear))),
        controller(
            ControllerTarget::Material(0),
            NifControllerKind::MaterialColor { field: MaterialColorField::Emissive, interp: point3_interp() },
        ),
        controller(
            ControllerTarget::Texturing(0),
            NifControllerKind::TextureTransform {
                slot: TextureSlot::Decal(2),
                member: TextureTransformMember::ScaleV,
                interp: float_interp(KeyKind::Tcb),
            },
        ),
        controller(
            ControllerTarget::Node(2),
            NifControllerKind::Morph { mesh: 0, weights: vec![None, Some(float_interp(KeyKind::Bezier))] },
        ),
        controller(
            ControllerTarget::ParticleSystem(0),
            NifControllerKind::Emitter { modifier: Some(0), birth_rate: Some(float_interp(KeyKind::Linear)), active: None },
        ),
        controller(
            ControllerTarget::ParticleSystem(0),
            NifControllerKind::ModifierFloat {
                modifier: Some(13),
                param: ModifierParam::GravityStrength,
                interp: Some(float_interp(KeyKind::Linear)),
            },
        ),
        controller(
            ControllerTarget::ParticleSystem(0),
            NifControllerKind::ModifierActive { modifier: None, interp: Some(bool_interp()) },
        ),
        controller(ControllerTarget::ParticleSystem(0), NifControllerKind::ParticleUpdate),
        controller(ControllerTarget::ParticleSystem(0), NifControllerKind::ResetOnLoop),
        controller(ControllerTarget::Node(0), NifControllerKind::Unsupported { type_name: "NiLookAtController".to_owned() }),
    ];
    NifEffectDesc {
        name: "00005001_test.nif".to_owned(),
        nodes,
        root: Some(0),
        meshes,
        particle_systems,
        materials,
        texturings,
        textures,
        controllers,
    }
}

#[test]
fn nif_desc_round_trips() {
    let desc = nif_desc();
    validate_nif_desc(&desc).expect("the synthetic desc is valid");
    let bytes = encode_nif_desc(&desc);
    assert_eq!(&bytes[..NIF_BLOB_MAGIC.len()], &NIF_BLOB_MAGIC);
    assert_eq!(decode_nif_desc(&bytes).expect("decodes"), desc);
}

#[test]
fn keys_keep_only_what_their_kind_reads() {
    let mut linear = curve(KeyKind::Linear, [0.0_f32, 1.0], 0.0);
    linear.keys[0].in_tangent = 5.0;
    linear.keys[0].tcb.bias = 1.0;
    let mut writer = Writer { bytes: Vec::new() };
    linear.encode(&mut writer);
    let mut reader = Reader { bytes: &writer.bytes, pos: 0 };
    let decoded = Curve::<f32>::decode(&mut reader).expect("decodes");
    assert_eq!(decoded, curve(KeyKind::Linear, [0.0, 1.0], 0.0));
}

#[test]
fn decoder_rejects_bad_headers() {
    let bytes = encode_chaos_desc(&chaos_desc());
    assert!(matches!(decode_nif_desc(&bytes), Err(DescDecodeError::BadMagic { .. })));

    let desc_version_offset = CHAOS_BLOB_MAGIC.len();
    let mut wrong_desc = bytes.clone();
    wrong_desc[desc_version_offset] ^= 1;
    assert!(matches!(decode_chaos_desc(&wrong_desc), Err(DescDecodeError::DescVersion { .. })));

    let codec_version_offset = desc_version_offset + size_of::<u32>();
    let mut wrong_codec = bytes.clone();
    wrong_codec[codec_version_offset] ^= 1;
    assert!(matches!(decode_chaos_desc(&wrong_codec), Err(DescDecodeError::CodecVersion { .. })));
}

#[test]
fn decoder_rejects_truncated_and_trailing_bytes() {
    let bytes = encode_nif_desc(&nif_desc());
    for len in [BLOB_HEADER_SIZE, bytes.len() / 2, bytes.len() - 1] {
        assert!(decode_nif_desc(&bytes[..len]).is_err(), "{len} bytes of {} decode", bytes.len());
    }
    assert!(matches!(decode_nif_desc(&bytes[..BLOB_HEADER_SIZE - 1]), Err(DescDecodeError::UnexpectedEnd { .. })));
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(decode_nif_desc(&longer), Err(DescDecodeError::TrailingBytes(1)));
}

#[test]
fn decoder_rejects_indices_out_of_range() {
    let mut desc = nif_desc();
    desc.meshes[0].node = desc.nodes.len();
    assert!(matches!(
        decode_nif_desc(&encode_nif_desc(&desc)),
        Err(DescDecodeError::IndexOutOfRange { what: "mesh node", .. })
    ));

    let mut desc = nif_desc();
    desc.controllers.push(controller(
        ControllerTarget::ParticleSystem(0),
        NifControllerKind::ModifierActive { modifier: Some(desc.particle_systems[0].modifiers.len()), interp: None },
    ));
    assert!(matches!(
        decode_nif_desc(&encode_nif_desc(&desc)),
        Err(DescDecodeError::IndexOutOfRange { what: "controller modifier", .. })
    ));

    let mut desc = nif_desc();
    desc.meshes[0].geometry.triangles.push([0, 1, 3]);
    assert!(decode_nif_desc(&encode_nif_desc(&desc)).is_err());
}

#[test]
fn decoder_rejects_invalid_tags() {
    let mut desc = chaos_desc();
    desc.sequences.truncate(1);
    desc.sequences[0].events.clear();
    let bytes = encode_chaos_desc(&desc);
    // The cel `looping` bool sits right before the event count (the last 4 bytes).
    let looping = bytes.len() - size_of::<u32>() - 1;
    let mut invalid = bytes.clone();
    invalid[looping] = BOOL_TRUE + 1;
    assert!(matches!(decode_chaos_desc(&invalid), Err(DescDecodeError::InvalidTag { what: "bool", .. })));
}
