//! Tests on small synthetic files.

use crate::reflect::{EnumKind, Reflect, Value, ValueMut};
use crate::*;

const HEADER_LINE: &[u8] = b"Gamebryo File Format, Version 20.0.0.4";
const NODE_NAME: &str = "Scene Root";
const NODE_SCALE: f32 = 2.5;
const NODE_FLAGS: u16 = 0x000e;
const NO_LINK: i32 = NULL_LINK;

/// Little-endian byte builder for hand-written expected files.
#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f32(&mut self, v: f32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn string(&mut self, s: &str) -> &mut Self {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
        self
    }
    fn header(&mut self, block_types: &[&str], type_indices: &[u16]) -> &mut Self {
        self.0.extend_from_slice(HEADER_LINE);
        self.u8(b'\n').u32(NIF_VERSION).u8(ENDIAN_LITTLE).u32(NIF_USER_VERSION);
        self.u32(type_indices.len() as u32).u16(block_types.len() as u16);
        for name in block_types {
            self.string(name);
        }
        for &index in type_indices {
            self.u16(index);
        }
        self.u32(0) // groups
    }
}

/// The bytes of a file holding one `NiNode` with no children, as `NiNode::LoadBinary` reads it.
fn single_node_bytes() -> Vec<u8> {
    let mut b = Bytes::default();
    b.header(&["NiNode"], &[0]);
    // NiObjectNET
    b.string(NODE_NAME).u32(0).i32(NO_LINK);
    // NiAVObject: flags, translate, rotate (identity), scale, property list, collision object
    b.u16(NODE_FLAGS).f32(0.0).f32(0.0).f32(0.0);
    for row in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
        for v in row {
            b.f32(v);
        }
    }
    b.f32(NODE_SCALE).u32(0).i32(NO_LINK);
    // NiNode: children, effect list
    b.u32(0).u32(0);
    // footer: one root, block 0
    b.u32(1).i32(0);
    b.0
}

fn single_node() -> NiNode {
    let mut node = NiNode::default();
    node.name = NifString::from(NODE_NAME);
    node.flags = NODE_FLAGS;
    node.scale = NODE_SCALE;
    node
}

fn file_of(blocks: Vec<Block>) -> NifFile {
    NifFile { header: Header::default(), blocks, roots: vec![Ref::new(0)] }
}

#[test]
fn reads_hand_written_file() {
    let nif = NifFile::read(&single_node_bytes()).unwrap();
    assert_eq!(nif.header.header_string.as_bytes(), HEADER_LINE);
    assert_eq!(nif.blocks.len(), 1);
    let node = nif.blocks[0].get::<NiNode>().unwrap();
    assert_eq!(node.name.to_string_lossy(), NODE_NAME);
    assert_eq!(node.flags, NODE_FLAGS);
    assert_eq!(node.scale, NODE_SCALE);
    // The identity rotation is NiAVObject's default (NiMatrix3 itself defaults to zero).
    assert_eq!(node.rotate, NiNode::default().rotate);
    assert_eq!(node.rotate.m00, 1.0);
    assert!(node.controllers.is_none());
    assert_eq!(nif.roots, vec![Ref::new(0)]);
    assert_eq!(nif, file_of(vec![single_node().into()]));
}

#[test]
fn writes_hand_written_file() {
    let out = file_of(vec![single_node().into()]).write().unwrap();
    assert_eq!(out, single_node_bytes());
}

#[test]
fn round_trips_keys_conditions_and_links() {
    let mut data = NiFloatData::default();
    data.key_type = KeyType::BEZKEY;
    data.keys = vec![
        NiKey { time: 0.0, value: 1.0, in_tan: 0.5, out_tan: -0.5, ..Default::default() },
        NiKey { time: 1.0, value: 3.0, ..Default::default() },
    ];
    data.num_keys = data.keys.len() as u32;

    let mut texturing = NiTexturingProperty::default();
    texturing.map_count = 1;
    texturing.has_base_map = NiBool::TRUE;
    texturing.base_map.texture = Ref::new(3);
    texturing.base_map.has_texture_transform = NiBool::TRUE;
    texturing.base_map.texture_transform.rotate = NODE_SCALE;

    let mut texture = NiSourceTexture::default();
    texture.filename = NifString::from("fx.dds");

    let mut node = single_node();
    node.property_list = vec![Ref::new(2)];

    let nif = file_of(vec![node.into(), data.into(), texturing.into(), texture.into()]);
    let bytes = nif.write().unwrap();
    let back = NifFile::read(&bytes).unwrap();
    assert_eq!(back, nif);
    assert_eq!(back.write().unwrap(), bytes);
    assert!(back.link_issues().is_empty());

    let node: &NiNode = back.get(Ref::<NiNode>::new(0)).unwrap();
    let property = back.get(node.property_list[0]).unwrap(); // &NiProperty
    assert_eq!(property.name.as_bytes(), b"");
    let texturing = back.blocks[2].get::<NiTexturingProperty>().unwrap();
    let texture = back.get(texturing.base_map.texture).unwrap(); // &NiTexture
    assert_eq!(texture.name.as_bytes(), b"");
    let source = back.blocks[3].get::<NiSourceTexture>().unwrap();
    assert_eq!(source.filename.to_string_lossy(), "fx.dds");
}

#[test]
fn absent_fields_are_not_written() {
    let mut float_data = NiFloatData::default();
    float_data.key_type = KeyType::LINKEY;
    float_data.keys = vec![NiKey { time: 0.0, value: 1.0, in_tan: 9.0, ..Default::default() }];
    float_data.num_keys = 1;
    let linear = file_of(vec![float_data.clone().into()]).write().unwrap();
    float_data.key_type = KeyType::BEZKEY;
    let bezier = file_of(vec![float_data.into()]).write().unwrap();
    // A Bezier key also stores its in and out tangents.
    assert_eq!(bezier.len() - linear.len(), 2 * std::mem::size_of::<f32>());
    // Read back, the linear key's tangent was not stored and is the default again.
    let back = NifFile::read(&linear).unwrap();
    assert_eq!(back.blocks[0].get::<NiFloatData>().unwrap().keys[0].in_tan, 0.0);
}

#[test]
fn unknown_block_type_is_named() {
    let mut b = Bytes::default();
    b.header(&["NiFancyThing"], &[0]);
    let err = NifFile::read(&b.0).unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnknownBlockType { name: "NiFancyThing".into() });
    assert_eq!(err.field_path(), "rtti[0]");
}

#[test]
fn truncated_block_reports_block_field_and_offset() {
    let bytes = single_node_bytes();
    // Cut inside the rotation matrix of block 0, in its second entry (row 0, column 1).
    let header_len = {
        let mut b = Bytes::default();
        b.header(&["NiNode"], &[0]);
        b.0.len()
    };
    let name_len = std::mem::size_of::<u32>() + NODE_NAME.len();
    let cut = header_len + name_len + 2 * std::mem::size_of::<u32>() + std::mem::size_of::<u16>()
        + 4 * std::mem::size_of::<f32>();
    let err = NifFile::read(&bytes[..cut + 1]).unwrap_err();
    assert_eq!(err.block, Some(BlockContext { index: 0, type_name: "NiNode".into() }));
    assert_eq!(err.field_path(), "rotate.m01");
    assert_eq!(err.offset, Some(cut));
    assert!(matches!(err.kind, ErrorKind::UnexpectedEof { .. }));
    let text = err.to_string();
    assert!(text.contains("block 0 (NiNode)") && text.contains("rotate.m01"), "{text}");
}

#[test]
fn rejects_other_versions_and_trailing_bytes() {
    let mut bytes = single_node_bytes();
    bytes.push(0);
    let err = NifFile::read(&bytes).unwrap_err();
    assert_eq!(err.kind, ErrorKind::TrailingData { bytes: 1 });

    let mut bytes = single_node_bytes();
    let version_at = HEADER_LINE.len() + 1;
    bytes[version_at..version_at + 4].copy_from_slice(&(NIF_VERSION + 1).to_le_bytes());
    let err = NifFile::read(&bytes).unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnsupportedVersion { version: NIF_VERSION + 1 });
}

#[test]
fn absurd_array_length_fails_before_allocating() {
    let mut bytes = single_node_bytes();
    let footer_len = std::mem::size_of::<u32>() + std::mem::size_of::<i32>();
    let effects_at = bytes.len() - footer_len - std::mem::size_of::<u32>();
    bytes[effects_at..effects_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let err = NifFile::read(&bytes).unwrap_err();
    assert!(matches!(err.kind, ErrorKind::LengthPastEof { .. }), "{err}");
    assert_eq!(err.field_path(), "effect_list");
}

#[test]
fn write_reports_count_mismatch() {
    // `vertex` takes its length from `vertices`; lists with their own count need no update.
    let mut node = single_node();
    node.children.push(Ref::NONE);
    let mut shape_data = NiTriShapeData::default();
    shape_data.has_vertex = NiBool::TRUE;
    shape_data.vertex.push(NiPoint3::default());
    let err = file_of(vec![node.into(), shape_data.into()]).write().unwrap_err();
    assert_eq!(err.kind, ErrorKind::LengthMismatch { expected: 0, actual: 1 });
    assert_eq!(err.block, Some(BlockContext { index: 1, type_name: "NiTriShapeData".into() }));
    assert_eq!(err.field_path(), "vertex");
}

#[test]
fn link_issues_find_wrong_targets() {
    let mut node = single_node();
    node.children = vec![Ref::new(1), Ref::new(7)];
    let nif = file_of(vec![node.into(), NiFloatData::default().into()]);
    let issues = nif.link_issues();
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].field, "children[0]");
    assert_eq!((issues[0].expected, issues[0].found), ("NiAVObject", Some("NiFloatData")));
    assert_eq!((issues[1].block, issues[1].found), (Some(0), None));
    let error = crate::read_checked(&nif.write().unwrap()).unwrap_err();
    assert!(error.starts_with("2 wrong link(s); the first: block 0 children[0] = 1"), "{error}");
    let linked = file_of(vec![single_node().into()]);
    assert!(crate::read_checked(&linked.write().unwrap()).is_ok());
    assert!(crate::read_checked(b"not a NIF\n").is_err());
}

#[test]
fn reflection_lists_inherited_fields_and_presence() {
    let block = Block::from(single_node());
    let fields = block.fields();
    let names: Vec<_> = fields.iter().map(|f| f.name()).collect();
    assert_eq!(&names[..3], ["name", "extra", "controllers"]);
    assert!(names.contains(&"children"));
    let scale = fields.iter().find(|f| f.name() == "scale").unwrap();
    assert_eq!(scale.info.declared_in, "NiAVObject");
    assert!(matches!(scale.value, Value::F32(&NODE_SCALE)));
    let children = fields.iter().find(|f| f.name() == "children").unwrap();
    assert_eq!(children.info.nif_type, "Ref<NiAVObject>[u32]");
    assert_eq!(children.info.length, None);
    let Value::List(children) = children.value else { panic!("list") };
    assert!(children.is_empty());

    // Key fields follow the key type the data passes down as `arg`.
    let mut data = NiFloatData::default();
    data.keys.push(NiKey::default());
    data.num_keys = 1;
    let key_presence = |data: &NiFloatData| {
        let fields = data.fields(0);
        assert_eq!(fields[2].info.length, Some("num_keys"));
        let Value::List(keys) = fields[2].value else { panic!("keys") };
        let Some(Value::Struct(key)) = keys.get(0) else { panic!("key") };
        key.fields().iter().map(|f| (f.name(), f.present)).collect::<Vec<_>>()
    };
    data.key_type = KeyType::LINKEY;
    assert_eq!(key_presence(&data)[2], ("in_tan", false));
    data.key_type = KeyType::BEZKEY;
    assert_eq!(key_presence(&data)[2], ("in_tan", true));
    assert_eq!(key_presence(&data)[4], ("tension", false));
}

#[test]
fn reflection_edits_values() {
    let mut block = Block::from(NiTexturingProperty::default());
    for field in block.fields_mut() {
        match (field.name(), field.value) {
            ("map_count", ValueMut::U32(v)) => *v = 1,
            ("has_base_map", ValueMut::Bool(v)) => *v = NiBool::TRUE,
            ("apply_mode", ValueMut::Enum(e)) => {
                let EnumKind::Enum(options) = e.kind() else { panic!("enum") };
                let replace = options.iter().find(|o| o.name == "APPLY_REPLACE").unwrap();
                e.set_raw(replace.value);
            }
            ("shader_maps", ValueMut::List(mut list)) => list.insert_default(0),
            _ => {}
        }
    }
    let texturing = block.get::<NiTexturingProperty>().unwrap();
    assert_eq!(texturing.apply_mode, ApplyMode::APPLY_REPLACE);
    assert!(texturing.has_base_map.get());
    assert_eq!(texturing.shader_maps.len(), 1);
    let base = block.fields().into_iter().find(|f| f.name() == "base_map").unwrap();
    assert!(base.present);
    let Value::Struct(map) = base.value else { panic!("struct") };
    assert_eq!(map.type_name(), "TexturingMap");
    // A derived struct lists its parent's fields first.
    let bump = block.fields().into_iter().find(|f| f.name() == "bump_map").unwrap();
    assert!(!bump.present);
    let Value::Struct(bump) = bump.value else { panic!("struct") };
    let bump_fields: Vec<_> = bump.fields().iter().map(|f| f.name()).collect();
    assert_eq!((bump_fields[0], bump_fields.last().copied()), ("texture", Some("bump_mat11")));
}

#[test]
fn defaults_follow_sdk_constructors() {
    let texturing = NiTexturingProperty::default();
    assert_eq!(texturing.apply_mode, ApplyMode::APPLY_MODULATE);
    assert_eq!(texturing.base_map.texture_transform.scale, NiPoint2 { x: 1.0, y: 1.0 });
    assert_eq!(texturing.bump_map.luma_scale, 1.0);
    assert_eq!(texturing.bump_map.clamp_mode, ClampMode::WRAP_S_WRAP_T);
    let controller = NiTimeController::default();
    assert_eq!(controller.hi_key_time, -f32::MAX);
    assert_eq!(u32::from(controller.flags.cycle_type()), CycleType::CLAMP.0);
    assert!(controller.flags.active() && controller.flags.compute_scaled_time());
    assert_eq!(NiPathInterpolator::default().bank_dir, BankDir::POSITIVE);
    assert_eq!(format!("{:?}", KeyType::TCBKEY), "KeyType::TCBKEY");
    let mut flags = TimeControllerFlags(0);
    flags.set_active(true);
    flags.set_cycle_type(2);
    assert_eq!(flags.0, 0b1100);
    assert_eq!(flags.cycle_type(), 2);
}

#[test]
fn round_trips_light_blocks() {
    let mut point = NiPointLight::default();
    // SDK constructor: constant 0, linear 1, quadratic 0.
    assert_eq!((point.atten0, point.atten1, point.atten2), (0.0, 1.0, 0.0));
    point.atten2 = NODE_SCALE;
    point.dimmer = NODE_SCALE;
    let mut spot = NiSpotLight::default();
    assert_eq!(spot.spot_exponent, 1.0);
    spot.outer_spot_angle = NODE_SCALE;
    let mut color = NiLightColorController::default();
    color.flags.set_is_ambient(true);
    let nif = file_of(vec![
        single_node().into(),
        NiAmbientLight::default().into(),
        point.clone().into(),
        spot.into(),
        color.into(),
        NiLightDimmerController::default().into(),
    ]);
    let bytes = nif.write().unwrap();
    let back = NifFile::read(&bytes).unwrap();
    assert_eq!(back, nif);
    assert_eq!(back.blocks_of::<NiLight>().count(), 3);
    assert!(back.blocks[4].get::<NiLightColorController>().unwrap().flags.is_ambient());
    // Version 20.0.0.4 stores no inner spot angle: a spot light adds the outer angle and the exponent.
    let point_size = file_of(vec![point.into()]).write().unwrap().len();
    let spot_size = file_of(vec![NiSpotLight::default().into()]).write().unwrap().len();
    // The header names the block type: "NiSpotLight" is one byte shorter than "NiPointLight".
    let name_bytes = "NiPointLight".len() - "NiSpotLight".len();
    assert_eq!(spot_size + name_bytes - point_size, 2 * std::mem::size_of::<f32>());
}
