//! Editing of the `.nif` files of `EffectTexture.uop` through gamebryo-nif's field reflection: field paths into
//! a block, edits of scalars, enums, strings, links and lists (keeping a list's count field in step when the
//! schema names it), the scene tree by `Ref` links, link candidates, key tables and curves, and the document
//! flow (an edit that does not write becomes a [`NifDraft`] until another edit makes it writable). No egui here.

use std::collections::BTreeMap;
use std::sync::Arc;

use gamebryo_nif::reflect::ListRef;
use gamebryo_nif::{Field, FieldMut, Link, LinkKind, NiBool, NifFile, Value, ValueMut};

use super::effect_files::EffectFiles;
use super::effects_editor::{EffectsDocument, EffectsEditKey, FileOverride, NifDraft};

/// Schema struct names of key types, shown as key tables.
pub const KEY_TYPE_NAMES: [&str; 4] = ["NiKey", "NiBoolKey", "NiRotKey", "NiQuatKey"];
/// Name of the key time field.
pub const KEY_TIME_FIELD: &str = "time";
/// Name of the key value field.
pub const KEY_VALUE_FIELD: &str = "value";
/// Name of the field holding a block's name (`NiObjectNET`).
const NAME_FIELD: &str = "name";

/// One step of a path from a block to a value: a field (by its index in the `fields()` list of the block or
/// struct) or a list element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldStep {
    Field(usize),
    Item(usize),
}

/// The text form of a path, for undo coalescing keys: `3/#1/0`.
pub fn path_key(path: &[FieldStep]) -> String {
    path.iter()
        .map(|step| match step {
            FieldStep::Field(index) => index.to_string(),
            FieldStep::Item(index) => format!("#{index}"),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A new value for a scalar field.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    /// Any integer field; out-of-range values are refused.
    Integer(i128),
    Float(f32),
    Bool(bool),
    String(String),
    /// The raw value of an enum, bitflags or bitfield.
    EnumRaw(u64),
    /// A block index or [`gamebryo_nif::NULL_LINK`].
    Link(i32),
}

/// One edit of a NIF block value.
#[derive(Debug, Clone, PartialEq)]
pub enum NifEdit {
    Set(ScalarValue),
    /// Inserts a default element at the index of the list.
    Insert(usize),
    /// Inserts a copy of the element after it.
    Duplicate(usize),
    Remove(usize),
}

impl NifEdit {
    fn changes_length(&self) -> bool {
        !matches!(self, Self::Set(_))
    }
}

/// What an edit did besides the edit itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditOutcome {
    /// The count field updated to a list's new length: (field name, length).
    pub count_updated: Option<(String, usize)>,
    /// The length expression of a resized list that is not a plain field (not updated automatically).
    pub length_not_updated: Option<String>,
}

/// Applies `edit` to the value at `path` in block `block`.
pub fn apply_edit(nif: &mut NifFile, block: usize, path: &[FieldStep], edit: &NifEdit) -> Result<EditOutcome, String> {
    let block = nif.blocks.get_mut(block).ok_or_else(|| format!("block {block} does not exist"))?;
    apply_in_fields(block.fields_mut(), path, edit)
}

fn apply_in_fields(mut fields: Vec<FieldMut<'_>>, path: &[FieldStep], edit: &NifEdit) -> Result<EditOutcome, String> {
    let Some((&FieldStep::Field(index), rest)) = path.split_first() else {
        return Err("the path does not name a field".to_owned());
    };
    if index >= fields.len() {
        return Err(format!("field {index} does not exist"));
    }
    if rest.is_empty() && edit.changes_length() {
        let info = fields[index].info;
        let ValueMut::List(list) = &mut fields[index].value else {
            return Err(format!("{} is not a list", info.name));
        };
        edit_list(list, edit)?;
        let length = list.len();
        let mut outcome = EditOutcome::default();
        if let Some(expression) = info.length {
            // A count shared by several present lists (`vertices`) is left for the other lists to follow.
            let shared = fields.iter().filter(|field| field.present && field.info.length == Some(expression)).count() > 1;
            match fields.iter_mut().find(|field| !shared && is_identifier(expression) && field.name() == expression) {
                Some(count) => {
                    set_count(&mut count.value, length)?;
                    outcome.count_updated = Some((expression.to_owned(), length));
                }
                None if shared => outcome.length_not_updated = Some(format!("{expression} (shared by other lists)")),
                None => outcome.length_not_updated = Some(expression.to_owned()),
            }
        }
        return Ok(outcome);
    }
    let field = fields.swap_remove(index);
    apply_in_value(field.value, rest, edit)
}

fn apply_in_value(value: ValueMut<'_>, path: &[FieldStep], edit: &NifEdit) -> Result<EditOutcome, String> {
    match (path.split_first(), value) {
        (None, ValueMut::List(mut list)) if edit.changes_length() => {
            edit_list(&mut list, edit)?;
            // A nested list: its length expression is not reflected.
            Ok(EditOutcome { length_not_updated: Some("the width of a nested list".to_owned()), ..EditOutcome::default() })
        }
        (None, value) => {
            let NifEdit::Set(scalar) = edit else { return Err("not a list".to_owned()) };
            set_scalar(value, scalar)?;
            Ok(EditOutcome::default())
        }
        (Some((FieldStep::Field(_), _)), ValueMut::Struct(fields)) => apply_in_fields(fields.into_fields_mut(), path, edit),
        (Some((&FieldStep::Item(index), rest)), ValueMut::List(mut list)) => {
            let item = list.get_mut(index).ok_or_else(|| format!("element {index} does not exist"))?;
            apply_in_value(item, rest, edit)
        }
        _ => Err("the path does not match the value".to_owned()),
    }
}

fn edit_list(list: &mut gamebryo_nif::reflect::ListMut<'_>, edit: &NifEdit) -> Result<(), String> {
    let length = list.len();
    match *edit {
        NifEdit::Insert(index) if index <= length => list.insert_default(index),
        NifEdit::Duplicate(index) if index < length => list.duplicate(index),
        NifEdit::Remove(index) if index < length => list.remove(index),
        NifEdit::Set(_) => return Err("a list is not a scalar".to_owned()),
        _ => return Err(format!("element index out of range (length {length})")),
    }
    Ok(())
}

/// A length expression naming one field.
fn is_identifier(expression: &str) -> bool {
    !expression.is_empty()
        && expression.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !expression.starts_with(|c: char| c.is_ascii_digit())
}

fn set_count(value: &mut ValueMut<'_>, length: usize) -> Result<(), String> {
    let too_long = || format!("{length} elements do not fit the count field");
    match value {
        ValueMut::U8(count) => **count = u8::try_from(length).map_err(|_| too_long())?,
        ValueMut::U16(count) => **count = u16::try_from(length).map_err(|_| too_long())?,
        ValueMut::U32(count) => **count = u32::try_from(length).map_err(|_| too_long())?,
        ValueMut::I32(count) => **count = i32::try_from(length).map_err(|_| too_long())?,
        ValueMut::U64(count) => **count = length as u64,
        _ => return Err("the count field is not an integer".to_owned()),
    }
    Ok(())
}

fn set_scalar(value: ValueMut<'_>, scalar: &ScalarValue) -> Result<(), String> {
    fn integer<T: TryFrom<i128>>(value: i128) -> Result<T, String> {
        T::try_from(value).map_err(|_| format!("{value} is out of range"))
    }
    match (value, scalar) {
        (ValueMut::U8(field), ScalarValue::Integer(v)) => *field = integer(*v)?,
        (ValueMut::I8(field), ScalarValue::Integer(v)) => *field = integer(*v)?,
        (ValueMut::U16(field), ScalarValue::Integer(v)) => *field = integer(*v)?,
        (ValueMut::I16(field), ScalarValue::Integer(v)) => *field = integer(*v)?,
        (ValueMut::U32(field), ScalarValue::Integer(v)) => *field = integer(*v)?,
        (ValueMut::I32(field), ScalarValue::Integer(v)) => *field = integer(*v)?,
        (ValueMut::U64(field), ScalarValue::Integer(v)) => *field = integer(*v)?,
        (ValueMut::F32(field), ScalarValue::Float(v)) => *field = *v,
        (ValueMut::Bool(field), ScalarValue::Bool(v)) => *field = if *v { NiBool::TRUE } else { NiBool::FALSE },
        (ValueMut::String(field), ScalarValue::String(v)) => field.set(v),
        (ValueMut::Enum(field), ScalarValue::EnumRaw(v)) => field.set_raw(*v),
        (ValueMut::Link { raw, .. }, ScalarValue::Link(v)) => *raw = *v,
        _ => return Err("the value does not fit the field type".to_owned()),
    }
    Ok(())
}

/// Writes a NIF; the error names the block and field.
pub fn write_nif(nif: &NifFile) -> Result<Vec<u8>, String> {
    nif.write().map_err(|error| error.to_string())
}

/// A short label of a block: `#3 NiNode "Scene Root"`.
pub fn block_label(nif: &NifFile, index: usize) -> String {
    let Some(block) = nif.blocks.get(index) else { return format!("#{index} (missing)") };
    let name = block.fields().into_iter().find_map(|field| match (field.name(), field.value) {
        (NAME_FIELD, Value::String(name)) if field.present => Some(name.to_string_lossy()),
        _ => None,
    });
    match name.filter(|name| !name.is_empty()) {
        Some(name) => format!("#{index} {} \"{name}\"", block.type_name()),
        None => format!("#{index} {}", block.type_name()),
    }
}

/// The blocks a link of type `target` may point at.
pub fn link_candidates(nif: &NifFile, target: &str) -> Vec<usize> {
    nif.blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| block.block_type().is_a(target))
        .map(|(index, _)| index)
        .collect()
}

/// The blocks a block owns through `Ref` links (present fields, in field order, without duplicates): the
/// children of the scene tree.
pub fn ref_children(nif: &NifFile, block: usize) -> Vec<usize> {
    fn visit(fields: &[Field<'_>], count: usize, out: &mut Vec<usize>) {
        for field in fields.iter().filter(|field| field.present) {
            visit_value(field.value, count, out);
        }
    }
    fn visit_value(value: Value<'_>, count: usize, out: &mut Vec<usize>) {
        match value {
            Value::Link { raw, kind: LinkKind::Ref, .. } => {
                if let Ok(index) = usize::try_from(raw) {
                    if index < count && !out.contains(&index) {
                        out.push(index);
                    }
                }
            }
            Value::Struct(nested) => visit(&nested.fields(), count, out),
            Value::List(list) => list.iter().for_each(|item| visit_value(item, count, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(block) = nif.blocks.get(block) {
        visit(&block.fields(), nif.blocks.len(), &mut out);
    }
    out
}

/// The roots of the scene tree (valid footer roots, without duplicates).
pub fn scene_roots(nif: &NifFile) -> Vec<usize> {
    let mut roots = Vec::new();
    for root in &nif.roots {
        if let Ok(index) = usize::try_from(root.raw()) {
            if index < nif.blocks.len() && !roots.contains(&index) {
                roots.push(index);
            }
        }
    }
    roots
}

/// The blocks no root reaches through `Ref` links.
pub fn unreachable_blocks(nif: &NifFile) -> Vec<usize> {
    let mut reached = vec![false; nif.blocks.len()];
    let mut pending = scene_roots(nif);
    while let Some(block) = pending.pop() {
        if std::mem::replace(&mut reached[block], true) {
            continue;
        }
        pending.extend(ref_children(nif, block).into_iter().filter(|&child| !reached[child]));
    }
    reached.iter().enumerate().filter(|(_, reached)| !**reached).map(|(index, _)| index).collect()
}

/// Whether a list holds animation keys (shown as a key table).
pub fn is_key_list(list: &ListRef<'_>) -> bool {
    matches!(list.get(0), Some(Value::Struct(key)) if KEY_TYPE_NAMES.contains(&key.type_name()))
}

/// One plotted series of a key list: (time, value) per key.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyCurve {
    pub label: String,
    pub points: Vec<[f32; 2]>,
}

/// The curves of a key list whose values are floats (one curve) or float compounds such as colours (one per
/// component). Empty for other keys.
pub fn key_curves(list: &ListRef<'_>) -> Vec<KeyCurve> {
    let mut curves: Vec<KeyCurve> = Vec::new();
    for item in list.iter() {
        let Value::Struct(key) = item else { return Vec::new() };
        let fields = key.fields();
        let field = |name: &str| fields.iter().find(|field| field.name() == name).map(|field| field.value);
        let Some(Value::F32(&time)) = field(KEY_TIME_FIELD) else { return Vec::new() };
        let values: Vec<(String, f32)> = match field(KEY_VALUE_FIELD) {
            Some(Value::F32(&value)) => vec![(KEY_VALUE_FIELD.to_owned(), value)],
            Some(Value::Struct(compound)) => {
                let components: Vec<_> = compound
                    .fields()
                    .iter()
                    .map(|component| match component.value {
                        Value::F32(&value) => Some((component.name().to_owned(), value)),
                        _ => None,
                    })
                    .collect();
                match components.into_iter().collect::<Option<Vec<_>>>() {
                    Some(components) => components,
                    None => return Vec::new(),
                }
            }
            _ => return Vec::new(),
        };
        if curves.is_empty() {
            curves = values.iter().map(|(label, _)| KeyCurve { label: label.clone(), points: Vec::new() }).collect();
        }
        if curves.len() != values.len() {
            return Vec::new();
        }
        for (curve, (_, value)) in curves.iter_mut().zip(values) {
            curve.points.push([time, value]);
        }
    }
    curves
}

/// The NIF a file shows now: its draft if it has one, else its payload parsed.
pub fn current_nif(document: &EffectsDocument, files: &EffectFiles, hash: u64) -> Result<Arc<NifFile>, String> {
    if let Some(draft) = document.nif_drafts().get(&hash) {
        return Ok(Arc::clone(&draft.nif));
    }
    let payload = files.payload(document.files(), hash).ok_or("the file has no payload")?;
    NifFile::read(&payload).map(Arc::new).map_err(|error| error.to_string())
}

/// One edit at a path of a block.
pub type PathEdit = (Vec<FieldStep>, NifEdit);

/// Edits a NIF file of the document: `edits` (applied in order, all or none) are one undo step, and repeated
/// value edits of the same field (or the same compound, such as a colour) coalesce. When the edited NIF
/// writes, its bytes become the file's payload (the change is dropped when they equal the package's own);
/// otherwise it is kept as a [`NifDraft`] with the write error.
pub fn edit_nif_file(
    document: &mut EffectsDocument,
    files: &EffectFiles,
    hash: u64,
    block: usize,
    edits: &[PathEdit],
) -> Result<Vec<EditOutcome>, String> {
    let name = files.name(document.files(), hash).ok_or("the file has no name")?;
    let mut nif = Arc::unwrap_or_clone(current_nif(document, files, hash)?);
    let outcome = edits.iter().map(|(path, edit)| apply_edit(&mut nif, block, path, edit)).collect::<Result<Vec<_>, _>>()?;
    let original = files.payload(&BTreeMap::new(), hash);
    // List edits are separate undo steps; value edits coalesce by the path they share.
    let shared = edits.iter().map(|(path, _)| path.as_slice()).reduce(|common, path| {
        let length = common.iter().zip(path).take_while(|(left, right)| left == right).count();
        &common[..length]
    });
    let key = match shared {
        Some(path) if edits.iter().all(|(_, edit)| !edit.changes_length()) => {
            Some(EffectsEditKey::NifField { hash, block, path: path_key(path) })
        }
        _ => None,
    };
    document.edit(key, |data| match write_nif(&nif) {
        Ok(bytes) => {
            if original.as_deref() == Some(bytes.as_slice()) {
                data.restore_file(hash);
            } else {
                data.set_file(hash, FileOverride { name, payload: Some(Arc::from(bytes)) });
            }
        }
        Err(error) => {
            data.nif_drafts.insert(hash, NifDraft { name, nif: Arc::new(nif), error });
        }
    });
    Ok(outcome)
}

/// Replaces a NIF file of the document by an already parsed one (import from disk): written as is.
pub fn replace_nif_file(document: &mut EffectsDocument, hash: u64, name: String, payload: Arc<[u8]>) {
    document.edit(None, |data| data.set_file(hash, FileOverride { name, payload: Some(payload) }));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::logic::effects_editor::{CollectionSource, EffectsDocument};
    use gamebryo_nif::{
        Header, NiColor, NiFloatData, NiFloatInterpolator, NiKey, NiMaterialProperty, NiNode, NiPoint3, NiTriShape,
        NiTriShapeData, NifString, Ref,
    };
    use uocf::enhanced::effect_definition::EffectDefinitionCollection;
    use uocf::enhanced::effect_texture::{effect_texture_hash, effect_texture_path, EFFECT_TEXTURE_UOP_NAME};
    use uocf::uop_container::file::CompressionFlag;
    use uocf::uop_container::package::UopPackage;

    /// Half the side of the test triangle, world units.
    pub(crate) const TEST_MESH_HALF_SIZE: f32 = 20.0;
    pub(crate) const TEST_MESH_NAME: &str = "Triangle";
    const TEST_EMISSIVE: NiColor = NiColor { r: 1.0, g: 0.5, b: 0.0 };
    /// Block indices of the test NIF.
    pub(crate) const ROOT_BLOCK: usize = 0;
    pub(crate) const SHAPE_BLOCK: usize = 1;
    pub(crate) const SHAPE_DATA_BLOCK: usize = 2;
    pub(crate) const MATERIAL_BLOCK: usize = 3;
    pub(crate) const FLOAT_DATA_BLOCK: usize = 5;
    const TEST_KEY_VALUES: [f32; 2] = [1.0, 3.0];
    const EDITED_SCALE: f32 = 4.0;

    /// A small mesh-only NIF: a node with an emissive triangle, and an unreferenced float interpolator with
    /// two keys.
    pub(crate) fn test_nif() -> NifFile {
        let mut root = NiNode::default();
        root.name = NifString::from("Scene Root");
        root.children = vec![Ref::new(SHAPE_BLOCK)];
        let mut shape = NiTriShape::default();
        shape.name = NifString::from(TEST_MESH_NAME);
        shape.model_data = Ref::new(SHAPE_DATA_BLOCK);
        shape.property_list = vec![Ref::new(MATERIAL_BLOCK)];
        let half = TEST_MESH_HALF_SIZE;
        let mut data = NiTriShapeData::default();
        data.has_vertex = NiBool::TRUE;
        data.vertex = vec![
            NiPoint3 { x: -half, y: 0.0, z: -half },
            NiPoint3 { x: half, y: 0.0, z: -half },
            NiPoint3 { x: -half, y: 0.0, z: half },
        ];
        data.vertices = data.vertex.len() as u16;
        data.has_list = NiBool::TRUE;
        data.tri_list = vec![0, 1, 2];
        data.tri_list_length = data.tri_list.len() as u32;
        data.triangles = 1;
        let mut material = NiMaterialProperty::default();
        material.emit = TEST_EMISSIVE;
        let mut interpolator = NiFloatInterpolator::default();
        interpolator.float_data = Ref::new(FLOAT_DATA_BLOCK);
        let mut keys = NiFloatData::default();
        keys.key_type = gamebryo_nif::KeyType::LINKEY;
        keys.keys = TEST_KEY_VALUES
            .iter()
            .enumerate()
            .map(|(index, &value)| NiKey { time: index as f32, value, ..Default::default() })
            .collect();
        keys.num_keys = keys.keys.len() as u32;
        NifFile {
            header: Header::default(),
            blocks: vec![root.into(), shape.into(), data.into(), material.into(), interpolator.into(), keys.into()],
            roots: vec![Ref::new(ROOT_BLOCK)],
        }
    }

    pub(crate) fn test_nif_bytes() -> Vec<u8> {
        test_nif().write().unwrap()
    }

    /// The value at `path` in block `block` (read-only).
    fn value_at<'a>(nif: &'a NifFile, block: usize, path: &[FieldStep]) -> Option<Value<'a>> {
        let mut fields = nif.blocks.get(block)?.fields();
        let mut steps = path.iter();
        let Some(&FieldStep::Field(first)) = steps.next() else { return None };
        let mut value = fields.get(first)?.value;
        for step in steps {
            value = match (*step, value) {
                (FieldStep::Field(index), Value::Struct(nested)) => {
                    fields = nested.fields();
                    fields.get(index)?.value
                }
                (FieldStep::Item(index), Value::List(list)) => list.get(index)?,
                _ => return None,
            };
        }
        Some(value)
    }

    fn field_index(nif: &NifFile, block: usize, name: &str) -> usize {
        nif.blocks[block].fields().iter().position(|field| field.name() == name).unwrap()
    }

    #[test]
    fn scalar_edits_write_and_read_back() {
        let mut nif = test_nif();
        let scale = [FieldStep::Field(field_index(&nif, ROOT_BLOCK, "scale"))];
        apply_edit(&mut nif, ROOT_BLOCK, &scale, &NifEdit::Set(ScalarValue::Float(EDITED_SCALE))).unwrap();
        let emit = field_index(&nif, MATERIAL_BLOCK, "emit");
        let green = [FieldStep::Field(emit), FieldStep::Field(1)];
        apply_edit(&mut nif, MATERIAL_BLOCK, &green, &NifEdit::Set(ScalarValue::Float(1.0))).unwrap();
        let name = [FieldStep::Field(field_index(&nif, SHAPE_BLOCK, "name"))];
        apply_edit(&mut nif, SHAPE_BLOCK, &name, &NifEdit::Set(ScalarValue::String("Edited".into()))).unwrap();
        let back = NifFile::read(&write_nif(&nif).unwrap()).unwrap();
        assert_eq!(back, nif);
        assert_eq!(back.blocks[ROOT_BLOCK].get::<NiNode>().unwrap().scale, EDITED_SCALE);
        assert_eq!(back.blocks[MATERIAL_BLOCK].get::<NiMaterialProperty>().unwrap().emit.g, 1.0);
        assert_eq!(block_label(&back, SHAPE_BLOCK), "#1 NiTriShape \"Edited\"");
        assert!(matches!(value_at(&back, MATERIAL_BLOCK, &green), Some(Value::F32(&1.0))));
        // Wrong types and ranges are refused.
        let vertices = [FieldStep::Field(field_index(&nif, SHAPE_DATA_BLOCK, "vertices"))];
        let too_big = NifEdit::Set(ScalarValue::Integer(i128::from(u16::MAX) + 1));
        assert!(apply_edit(&mut nif, SHAPE_DATA_BLOCK, &vertices, &too_big).is_err());
        assert!(apply_edit(&mut nif, ROOT_BLOCK, &scale, &NifEdit::Set(ScalarValue::Bool(true))).is_err());
    }

    #[test]
    fn list_edits_update_their_count_field() {
        let mut nif = test_nif();
        let keys = [FieldStep::Field(field_index(&nif, FLOAT_DATA_BLOCK, "keys"))];
        let outcome = apply_edit(&mut nif, FLOAT_DATA_BLOCK, &keys, &NifEdit::Duplicate(1)).unwrap();
        assert_eq!(outcome.count_updated, Some(("num_keys".to_owned(), TEST_KEY_VALUES.len() + 1)));
        let data = nif.blocks[FLOAT_DATA_BLOCK].get::<NiFloatData>().unwrap();
        assert_eq!((data.num_keys, data.keys.len()), (3, 3));
        let back = NifFile::read(&write_nif(&nif).unwrap()).unwrap();
        assert_eq!(back, nif);
        // A vertex added updates `vertices`, the only present list counted by it (no normals or colours).
        let vertex = [FieldStep::Field(field_index(&nif, SHAPE_DATA_BLOCK, "vertex"))];
        apply_edit(&mut nif, SHAPE_DATA_BLOCK, &vertex, &NifEdit::Insert(0)).unwrap();
        assert_eq!(nif.blocks[SHAPE_DATA_BLOCK].get::<NiTriShapeData>().unwrap().vertices, 4);
        assert!(write_nif(&nif).is_ok());
        // Self-counted lists need no count.
        let children = [FieldStep::Field(field_index(&nif, ROOT_BLOCK, "children"))];
        let outcome = apply_edit(&mut nif, ROOT_BLOCK, &children, &NifEdit::Insert(1)).unwrap();
        assert_eq!(outcome, EditOutcome::default());
        // The key curve follows the keys.
        let Some(Value::List(list)) = value_at(&nif, FLOAT_DATA_BLOCK, &keys) else { panic!("keys") };
        assert!(is_key_list(&list));
        let curves = key_curves(&list);
        assert_eq!(curves.len(), 1);
        assert_eq!(curves[0].points.iter().map(|point| point[1]).collect::<Vec<_>>(), [1.0, 3.0, 3.0]);
    }

    #[test]
    fn tree_links_and_candidates() {
        let nif = test_nif();
        assert_eq!(scene_roots(&nif), [ROOT_BLOCK]);
        assert_eq!(ref_children(&nif, ROOT_BLOCK), [SHAPE_BLOCK]);
        assert_eq!(ref_children(&nif, SHAPE_BLOCK), [MATERIAL_BLOCK, SHAPE_DATA_BLOCK]);
        assert_eq!(unreachable_blocks(&nif), [4, FLOAT_DATA_BLOCK]);
        assert_eq!(link_candidates(&nif, "NiGeometryData"), [SHAPE_DATA_BLOCK]);
        assert_eq!(link_candidates(&nif, "NiObject").len(), nif.blocks.len());
        assert!(nif.link_issues().is_empty());
    }

    /// A name of the test string dictionary (unnamed files cannot be saved).
    const TEST_FILE_NAME: &str = "smoke.nif";

    /// The document flow: an edit becomes the payload, an unwritable edit a draft, undo and redo restore both.
    #[test]
    fn document_edits_drafts_undo_and_redo() {
        use crate::logic::effect_preview::tests::dictionary;
        use crate::logic::effects_editor::tests::scratch_dir;
        let dir = scratch_dir("nif_document");
        let package_path = dir.join(EFFECT_TEXTURE_UOP_NAME);
        let mut package = UopPackage::new_default();
        package
            .add_file_from_memory(&test_nif_bytes(), &effect_texture_path(TEST_FILE_NAME), CompressionFlag::None)
            .unwrap();
        package.finalize_and_save(&package_path).unwrap();
        let files = EffectFiles::open(&package_path, Some(&dictionary())).unwrap();
        let hash = effect_texture_hash(TEST_FILE_NAME);
        let mut document =
            EffectsDocument::new(EffectDefinitionCollection::default(), CollectionSource::None);
        let nif = current_nif(&document, &files, hash).unwrap();
        assert_eq!(*nif, test_nif());

        // A scalar edit becomes the payload; dragging the same field coalesces.
        let scale = [FieldStep::Field(field_index(&nif, ROOT_BLOCK, "scale"))];
        for value in [2.0, EDITED_SCALE] {
            edit_nif_file(&mut document, &files, hash, ROOT_BLOCK, &[(scale.to_vec(), NifEdit::Set(ScalarValue::Float(value)))]).unwrap();
        }
        let edited = current_nif(&document, &files, hash).unwrap();
        assert_eq!(edited.blocks[ROOT_BLOCK].get::<NiNode>().unwrap().scale, EDITED_SCALE);
        assert!(document.files().contains_key(&hash) && document.nif_drafts().is_empty());

        // Turning colours on makes a list counted by `vertices` with no element: it does not write. The count
        // is shared with `vertex`, so adding colours leaves it, until there is one per vertex.
        let colors = [FieldStep::Field(field_index(&nif, SHAPE_DATA_BLOCK, "color"))];
        let has_color = [FieldStep::Field(field_index(&nif, SHAPE_DATA_BLOCK, "has_color"))];
        edit_nif_file(&mut document, &files, hash, SHAPE_DATA_BLOCK, &[(has_color.to_vec(), NifEdit::Set(ScalarValue::Bool(true)))]).unwrap();
        assert!(document.nif_drafts().contains_key(&hash), "3 vertices, no colours: does not write");
        assert!(document.files_dirty());
        let outcome =
            edit_nif_file(&mut document, &files, hash, SHAPE_DATA_BLOCK, &[(colors.to_vec(), NifEdit::Insert(0))]).unwrap();
        assert_eq!(outcome[0].length_not_updated.as_deref(), Some("vertices (shared by other lists)"));
        assert!(document.nif_drafts().contains_key(&hash), "1 colour for 3 vertices");
        for _ in 1..3 {
            edit_nif_file(&mut document, &files, hash, SHAPE_DATA_BLOCK, &[(colors.to_vec(), NifEdit::Duplicate(0))]).unwrap();
        }
        assert!(document.nif_drafts().is_empty(), "3 colours for 3 vertices write again");
        let written = files.payload(document.files(), hash).unwrap();
        let back = NifFile::read(&written).unwrap();
        assert_eq!(back.blocks[SHAPE_DATA_BLOCK].get::<NiTriShapeData>().unwrap().color.len(), 3);

        // Undo walks back through the draft to the scale edit, redo forward again.
        let steps = 1 + 1 + 2;
        for _ in 0..steps {
            assert!(document.undo());
        }
        assert!(document.nif_drafts().is_empty());
        assert_eq!(current_nif(&document, &files, hash).unwrap(), edited);
        assert!(document.redo());
        assert!(document.nif_drafts().contains_key(&hash));
        for _ in 1..steps {
            assert!(document.redo());
        }
        assert_eq!(current_nif(&document, &files, hash).unwrap().blocks, back.blocks);

        // Setting the scale back to the original drops the change once nothing else differs.
        for _ in 0..steps {
            document.undo();
        }
        edit_nif_file(&mut document, &files, hash, ROOT_BLOCK, &[(scale.to_vec(), NifEdit::Set(ScalarValue::Float(1.0)))]).unwrap();
        assert!(!document.files().contains_key(&hash), "equal to the package's own bytes");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
