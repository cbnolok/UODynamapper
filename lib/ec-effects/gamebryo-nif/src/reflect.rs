//! Field reflection, so a UI can list and edit any block without per-type code.
//!
//! Every block and compound implements [`Reflect`]: `fields()` returns each field with its schema
//! name and type, whether it is present (its schema condition holds for the current values), and
//! a typed [`Value`] view. `fields_mut()` returns [`ValueMut`] views for editing in place.
//!
//! Conditional fields always exist in the Rust structs; when their condition is false they are
//! not read or written and keep their default value. Toggling the controlling field (for example
//! `has_texture_transform`) makes them present again. Lists stored with their own element count
//! (schema type `T[u32]`, [`FieldInfo::length`] `None`) are written with their current length.
//! Other lists take their length from other fields: after resizing one, update the fields named
//! in [`FieldInfo::length`], or writing reports a length mismatch.
//!
//! ```
//! use gamebryo_nif::reflect::{Field, Value};
//! use gamebryo_nif::{Block, NiNode};
//!
//! /// Prints a property tree, the way a generic property grid would walk it.
//! fn show(fields: &[Field<'_>], depth: usize) {
//!     for field in fields.iter().filter(|f| f.present) {
//!         let pad = "  ".repeat(depth);
//!         match field.value {
//!             Value::F32(v) => println!("{pad}{} = {v}", field.name()),
//!             Value::Enum(e) => println!("{pad}{} = {:?}", field.name(), e.option_name()),
//!             Value::Struct(s) => {
//!                 println!("{pad}{} ({})", field.name(), s.type_name());
//!                 show(&s.fields(), depth + 1);
//!             }
//!             Value::List(list) => println!("{pad}{} [{}]", field.name(), list.len()),
//!             _ => println!("{pad}{} ({})", field.name(), field.info.nif_type),
//!         }
//!     }
//! }
//!
//! show(&Block::from(NiNode::default()).fields(), 0);
//! ```

use crate::io::NifIo;
use crate::types::{LinkKind, NiBool, NiObjectType, NifString, Ptr, Ref};

/// A reflected struct: a block or a compound.
pub trait Reflect {
    /// The schema type name (for structs, without template arguments).
    fn type_name(&self) -> &'static str;

    /// The fields in file order. `arg` is the schema `arg` the parent passes (0 for blocks).
    fn fields(&self, arg: i64) -> Vec<Field<'_>>;

    fn fields_mut(&mut self, arg: i64) -> Vec<FieldMut<'_>>;
}

/// Static description of a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldInfo {
    /// The schema field name (the SDK member name in snake_case), e.g. `vertices`.
    pub name: &'static str,
    /// The schema type, with its template and array dimensions, e.g. `NiKey<f32>[]`,
    /// `Ref<NiAVObject>[u32]` (a list stored with a `u32` count in front) or `Ptr<NiNode>`.
    pub nif_type: &'static str,
    /// The struct or object that declares the field (an ancestor for inherited fields).
    pub declared_in: &'static str,
    /// The length expression of arrays whose length comes from other fields, e.g. `vertices`.
    pub length: Option<&'static str>,
    /// The condition text, if the field is conditional.
    pub condition: Option<&'static str>,
}

/// A field and a read-only view of its value.
#[derive(Clone, Copy)]
pub struct Field<'a> {
    pub info: &'static FieldInfo,
    /// Whether the field is part of the file with the current values.
    pub present: bool,
    pub value: Value<'a>,
}

/// A field and a mutable view of its value.
pub struct FieldMut<'a> {
    pub info: &'static FieldInfo,
    pub present: bool,
    pub value: ValueMut<'a>,
}

impl Field<'_> {
    pub fn name(&self) -> &'static str {
        self.info.name
    }
}

impl FieldMut<'_> {
    pub fn name(&self) -> &'static str {
        self.info.name
    }
}

/// A read-only typed view of a field value.
#[derive(Clone, Copy)]
pub enum Value<'a> {
    U8(&'a u8),
    I8(&'a i8),
    U16(&'a u16),
    I16(&'a i16),
    U32(&'a u32),
    I32(&'a i32),
    U64(&'a u64),
    F32(&'a f32),
    Bool(&'a NiBool),
    String(&'a NifString),
    Enum(&'a dyn NifEnum),
    /// A `Ref`/`Ptr`; `raw` is the block index or -1.
    Link { raw: i32, kind: LinkKind, target: &'static str },
    Struct(StructRef<'a>),
    List(ListRef<'a>),
}

/// A mutable typed view of a field value.
pub enum ValueMut<'a> {
    U8(&'a mut u8),
    I8(&'a mut i8),
    U16(&'a mut u16),
    I16(&'a mut i16),
    U32(&'a mut u32),
    I32(&'a mut i32),
    U64(&'a mut u64),
    F32(&'a mut f32),
    Bool(&'a mut NiBool),
    String(&'a mut NifString),
    Enum(&'a mut dyn NifEnum),
    Link { raw: &'a mut i32, kind: LinkKind, target: &'static str },
    Struct(StructMut<'a>),
    List(ListMut<'a>),
}

/// A nested struct and the `arg` its parent passes to it.
#[derive(Clone, Copy)]
pub struct StructRef<'a> {
    pub value: &'a dyn Reflect,
    pub arg: i64,
}

impl<'a> StructRef<'a> {
    pub fn type_name(&self) -> &'static str {
        self.value.type_name()
    }

    pub fn fields(&self) -> Vec<Field<'a>> {
        self.value.fields(self.arg)
    }
}

pub struct StructMut<'a> {
    pub value: &'a mut dyn Reflect,
    pub arg: i64,
}

impl<'a> StructMut<'a> {
    pub fn type_name(&self) -> &'static str {
        self.value.type_name()
    }

    pub fn fields_mut(&mut self) -> Vec<FieldMut<'_>> {
        self.value.fields_mut(self.arg)
    }

    pub fn into_fields_mut(self) -> Vec<FieldMut<'a>> {
        self.value.fields_mut(self.arg)
    }
}

/// A list field and the `arg` passed to each element.
#[derive(Clone, Copy)]
pub struct ListRef<'a> {
    pub list: &'a dyn ReflectList,
    pub arg: i64,
}

impl<'a> ListRef<'a> {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<Value<'a>> {
        self.list.item(index, self.arg)
    }

    pub fn iter(&self) -> impl Iterator<Item = Value<'a>> + '_ {
        (0..self.len()).filter_map(|i| self.get(i))
    }
}

pub struct ListMut<'a> {
    pub list: &'a mut dyn ReflectList,
    pub arg: i64,
}

impl ListMut<'_> {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.len() == 0
    }

    pub fn get_mut(&mut self, index: usize) -> Option<ValueMut<'_>> {
        self.list.item_mut(index, self.arg)
    }

    /// Inserts a default element at `index` (clamped to the length).
    pub fn insert_default(&mut self, index: usize) {
        self.list.insert_default(index);
    }

    /// Inserts a copy of the element at `index` right after it.
    pub fn duplicate(&mut self, index: usize) {
        self.list.duplicate(index);
    }

    pub fn remove(&mut self, index: usize) {
        self.list.remove(index);
    }
}

/// Type-erased access to a `Vec` field.
pub trait ReflectList {
    fn len(&self) -> usize;
    fn item(&self, index: usize, arg: i64) -> Option<Value<'_>>;
    fn item_mut(&mut self, index: usize, arg: i64) -> Option<ValueMut<'_>>;
    fn insert_default(&mut self, index: usize);
    fn duplicate(&mut self, index: usize);
    fn remove(&mut self, index: usize);
}

impl<T: AsValue + Default + Clone + 'static> ReflectList for Vec<T> {
    fn len(&self) -> usize {
        Vec::len(self)
    }

    fn item(&self, index: usize, arg: i64) -> Option<Value<'_>> {
        self.get(index).map(|v| v.as_value(arg))
    }

    fn item_mut(&mut self, index: usize, arg: i64) -> Option<ValueMut<'_>> {
        self.get_mut(index).map(|v| v.as_value_mut(arg))
    }

    fn insert_default(&mut self, index: usize) {
        let index = index.min(Vec::len(self));
        self.insert(index, T::default());
    }

    fn duplicate(&mut self, index: usize) {
        if let Some(value) = self.get(index).cloned() {
            self.insert(index + 1, value);
        }
    }

    fn remove(&mut self, index: usize) {
        if index < Vec::len(self) {
            Vec::remove(self, index);
        }
    }
}

/// One named value of an enum or bit of a bitflags type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumOption {
    pub name: &'static str,
    pub value: u64,
}

/// One member of a bitfield type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitfieldMember {
    pub name: &'static str,
    pub pos: u32,
    pub width: u32,
    /// The schema type of the member (an enum name, `bool` or `uint`).
    pub nif_type: &'static str,
}

/// How to present an enum-like value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumKind {
    /// One of the named values (other raw values are kept as-is).
    Enum(&'static [EnumOption]),
    /// A set of named bits.
    Bitflags(&'static [EnumOption]),
    /// Packed sub-fields.
    Bitfield(&'static [BitfieldMember]),
}

/// Type-erased access to a generated enum, bitflags or bitfield value.
pub trait NifEnum {
    fn type_name(&self) -> &'static str;
    fn kind(&self) -> EnumKind;
    fn raw(&self) -> u64;
    /// Sets the raw value, truncated to the storage width.
    fn set_raw(&mut self, value: u64);

    /// The option name for the current value (enums only).
    fn option_name(&self) -> Option<&'static str> {
        match self.kind() {
            EnumKind::Enum(options) => {
                options.iter().find(|o| o.value == self.raw()).map(|o| o.name)
            }
            _ => None,
        }
    }
}

/// Conversion of a field value to its reflected view.
pub trait AsValue {
    fn as_value(&self, arg: i64) -> Value<'_>;
    fn as_value_mut(&mut self, arg: i64) -> ValueMut<'_>;
}

/// Everything a field type (and a template argument such as `NiKey<T>`'s `T`) implements.
pub trait FieldType: NifIo + AsValue + Default + Clone + std::fmt::Debug + PartialEq + 'static {}

impl<T: NifIo + AsValue + Default + Clone + std::fmt::Debug + PartialEq + 'static> FieldType for T {}

macro_rules! impl_as_value {
    ($($ty:ty => $variant:ident),*) => {$(
        impl AsValue for $ty {
            fn as_value(&self, _arg: i64) -> Value<'_> {
                Value::$variant(self)
            }

            fn as_value_mut(&mut self, _arg: i64) -> ValueMut<'_> {
                ValueMut::$variant(self)
            }
        }
    )*};
}

impl_as_value!(
    u8 => U8, i8 => I8, u16 => U16, i16 => I16, u32 => U32, i32 => I32, u64 => U64, f32 => F32,
    NiBool => Bool, NifString => String
);

macro_rules! impl_link_value {
    ($($name:ident),*) => {$(
        impl<T: NiObjectType> AsValue for $name<T> {
            fn as_value(&self, _arg: i64) -> Value<'_> {
                Value::Link { raw: crate::types::Link::raw(self), kind: Self::KIND, target: T::NAME }
            }

            fn as_value_mut(&mut self, _arg: i64) -> ValueMut<'_> {
                ValueMut::Link { raw: self.raw_mut(), kind: Self::KIND, target: T::NAME }
            }
        }
    )*};
}

impl_link_value!(Ref, Ptr);

impl<T: AsValue + Default + Clone + 'static> AsValue for Vec<T> {
    fn as_value(&self, arg: i64) -> Value<'_> {
        Value::List(ListRef { list: self, arg })
    }

    fn as_value_mut(&mut self, arg: i64) -> ValueMut<'_> {
        ValueMut::List(ListMut { list: self, arg })
    }
}

/// Generated-code helper: a read-only field entry.
#[inline]
pub(crate) fn field<'a>(info: &'static FieldInfo, present: bool, value: Value<'a>) -> Field<'a> {
    Field { info, present, value }
}

/// Generated-code helper: a mutable field entry.
#[inline]
pub(crate) fn field_mut<'a>(
    info: &'static FieldInfo,
    present: bool,
    value: ValueMut<'a>,
) -> FieldMut<'a> {
    FieldMut { info, present, value }
}
