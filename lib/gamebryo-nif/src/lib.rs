//! Gamebryo NIF reader and writer.
//!
//! The block, struct and enum types are generated for one NIF version, [`NIF_VERSION`], from a
//! schema written from the Gamebryo SDK's stream loading code (see `codegen/` and the crate
//! README). Field names are the SDK member names in snake_case. Reading then writing an
//! unmodified file reproduces it byte for byte.
//!
//! ```no_run
//! use gamebryo_nif::{NifFile, NiNode, NiSourceTexture};
//!
//! let bytes = std::fs::read("effect.nif").unwrap();
//! let mut nif = NifFile::read(&bytes).unwrap();
//! for (index, node) in nif.blocks_of::<NiNode>() {
//!     // Inherited fields are reachable directly (NiNode derefs to NiAVObject, ...).
//!     println!("{index}: {} with {} children", node.name, node.children.len());
//!     for child in &node.children {
//!         let _child = nif.get(*child); // Option<&NiAVObject>
//!     }
//! }
//! for block in &mut nif.blocks {
//!     if let Some(texture) = block.get_mut::<NiSourceTexture>() {
//!         texture.filename.set("textures/other.dds");
//!     }
//! }
//! let out = nif.write().unwrap();
//! # let _ = out;
//! ```

mod error;
mod file;
mod generated;
mod io;
pub mod reflect;
mod types;

pub use error::{BlockContext, ErrorKind, NifError, PathSegment, Result};
pub use file::{ENDIAN_LITTLE, Header, LinkIssue, NifFile, read, write};
pub use generated::*;
pub use io::{Ev, NifIo, Reader, Writer};
pub use reflect::{AsValue, Field, FieldInfo, FieldMut, FieldType, NifEnum, Reflect, Value, ValueMut};
pub use types::{Link, LinkKind, NULL_LINK, NiBool, NiObjectType, NifString, Ptr, Ref};

/// Formats a packed version number as `a.b.c.d`.
pub fn version_string(version: u32) -> String {
    let [a, b, c, d] = version.to_be_bytes();
    format!("{a}.{b}.{c}.{d}")
}

/// Implements the shared traits of a generated enum, bitflags or bitfield newtype.
macro_rules! nif_enum {
    ($name:ident, $storage:ty, $kind:expr) => {
        impl $crate::io::NifIo for $name {
            const MIN_SIZE: usize = std::mem::size_of::<$storage>();

            fn read(r: &mut $crate::io::Reader<'_>, _arg: i64) -> $crate::error::Result<Self> {
                Ok(Self(<$storage as $crate::io::NifIo>::read(r, 0)?))
            }

            fn write(&self, w: &mut $crate::io::Writer, _arg: i64) -> $crate::error::Result<()> {
                $crate::io::NifIo::write(&self.0, w, 0)
            }
        }

        impl $crate::io::Ev for $name {
            fn ev(&self) -> i64 {
                self.0 as i64
            }
        }

        impl $crate::reflect::NifEnum for $name {
            fn type_name(&self) -> &'static str {
                stringify!($name)
            }

            fn kind(&self) -> $crate::reflect::EnumKind {
                $kind
            }

            fn raw(&self) -> u64 {
                self.0 as u64
            }

            fn set_raw(&mut self, value: u64) {
                self.0 = value as $storage;
            }
        }

        impl $crate::reflect::AsValue for $name {
            fn as_value(&self, _arg: i64) -> $crate::reflect::Value<'_> {
                $crate::reflect::Value::Enum(self)
            }

            fn as_value_mut(&mut self, _arg: i64) -> $crate::reflect::ValueMut<'_> {
                $crate::reflect::ValueMut::Enum(self)
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match $crate::reflect::NifEnum::option_name(self) {
                    Some(option) => write!(f, "{}::{}", stringify!($name), option),
                    None => write!(f, "{}({:#x})", stringify!($name), self.0),
                }
            }
        }
    };
}

pub(crate) use nif_enum;

#[cfg(test)]
mod tests;
