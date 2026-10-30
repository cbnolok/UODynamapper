//! Hand-written value types that the schema's basic types (strings, NiBool, links) map to.

use std::fmt;
use std::marker::PhantomData;

use crate::Block;
use crate::error::Result;
use crate::io::{Ev, NifIo, Reader, Writer};

/// Raw value of a link that points at no block.
pub const NULL_LINK: i32 = -1;

/// A length-prefixed (`uint`) string, kept as raw bytes so files round-trip exactly.
///
/// NIF strings are 8-bit (in practice ASCII/Latin-1). Before 20.1.0.1 the SDK stores every string
/// inline this way (`NiStream::LoadCStringAsFixedString`); the schema's `string` maps to this type.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct NifString(pub Vec<u8>);

impl NifString {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The text, decoding each byte as Latin-1.
    pub fn to_string_lossy(&self) -> String {
        self.0.iter().map(|&b| b as char).collect()
    }

    /// Replaces the text; characters outside Latin-1 become `?`.
    pub fn set(&mut self, text: &str) {
        *self = Self::from(text);
    }
}

impl From<&str> for NifString {
    fn from(text: &str) -> Self {
        Self(text.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect())
    }
}

impl fmt::Debug for NifString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.to_string_lossy(), f)
    }
}

impl fmt::Display for NifString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}

impl NifIo for NifString {
    const MIN_SIZE: usize = std::mem::size_of::<u32>();

    fn read(r: &mut Reader<'_>, _arg: i64) -> Result<Self> {
        let len = u32::read(r, 0)?;
        Ok(Self(r.bytes(len as usize)?.to_vec()))
    }

    fn write(&self, w: &mut Writer, _arg: i64) -> Result<()> {
        let len = u32::try_from(self.0.len())
            .map_err(|_| w.error(crate::ErrorKind::CountOverflow { count: self.0.len() }))?;
        len.write(w, 0)?;
        w.bytes(&self.0);
        Ok(())
    }
}

/// The SDK's `NiBool` (`typedef unsigned char`), one byte. Kept raw because files store values
/// other than 0 and 1 (for example `NiBoolInterpolator::bool_value` uses 2 for "invalid").
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NiBool(pub u8);

impl NiBool {
    pub const FALSE: Self = Self(0);
    pub const TRUE: Self = Self(1);

    pub fn get(self) -> bool {
        self.0 != 0
    }
}

impl From<bool> for NiBool {
    fn from(value: bool) -> Self {
        Self(u8::from(value))
    }
}

impl fmt::Debug for NiBool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0 => f.write_str("false"),
            1 => f.write_str("true"),
            raw => write!(f, "NiBool({raw})"),
        }
    }
}

impl NifIo for NiBool {
    const MIN_SIZE: usize = std::mem::size_of::<u8>();

    fn read(r: &mut Reader<'_>, _arg: i64) -> Result<Self> {
        Ok(Self(u8::read(r, 0)?))
    }

    fn write(&self, w: &mut Writer, _arg: i64) -> Result<()> {
        self.0.write(w, 0)
    }
}

impl Ev for NiBool {
    fn ev(&self) -> i64 {
        i64::from(self.0)
    }
}

/// A NIF object type (concrete block or abstract ancestor) that a link can point at.
///
/// Implemented by every generated object struct, and by uninhabited marker types for object
/// types that are referenced by generated fields but not themselves generated.
pub trait NiObjectType: 'static {
    /// The SDK class name.
    const NAME: &'static str;

    /// The block viewed as this type, if it is this type or a subtype.
    fn from_block(block: &Block) -> Option<&Self>;

    fn from_block_mut(block: &mut Block) -> Option<&mut Self>;
}

/// Whether a link is a `Ref` (owning, points down the hierarchy) or a `Ptr` (points up).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkKind {
    Ref,
    Ptr,
}

/// Common interface of [`Ref`] and [`Ptr`].
pub trait Link: Copy {
    type Target: NiObjectType;

    /// The raw stored value; [`NULL_LINK`] means none.
    fn raw(&self) -> i32;

    /// The block index, or `None` for a null (or invalid negative) link.
    fn index(&self) -> Option<usize> {
        usize::try_from(self.raw()).ok()
    }
}

macro_rules! link_type {
    ($name:ident, $kind:expr, $doc:literal) => {
        #[doc = $doc]
        pub struct $name<T> {
            raw: i32,
            target: PhantomData<fn() -> T>,
        }

        impl<T> $name<T> {
            /// A link to no block.
            pub const NONE: Self = Self { raw: NULL_LINK, target: PhantomData };

            pub fn new(index: usize) -> Self {
                Self::from_raw(i32::try_from(index).unwrap_or(NULL_LINK))
            }

            pub fn from_raw(raw: i32) -> Self {
                Self { raw, target: PhantomData }
            }

            pub fn is_none(&self) -> bool {
                self.raw < 0
            }

            pub(crate) fn raw_mut(&mut self) -> &mut i32 {
                &mut self.raw
            }

            /// The same link, retyped (for example to a more derived type after a check).
            pub fn cast<U>(self) -> $name<U> {
                $name::from_raw(self.raw)
            }
        }

        impl<T: NiObjectType> Link for $name<T> {
            type Target = T;

            fn raw(&self) -> i32 {
                self.raw
            }
        }

        impl<T> $name<T> {
            pub const KIND: LinkKind = $kind;
        }

        impl<T> Clone for $name<T> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<T> Copy for $name<T> {}

        impl<T> PartialEq for $name<T> {
            fn eq(&self, other: &Self) -> bool {
                self.raw == other.raw
            }
        }

        impl<T> Eq for $name<T> {}

        impl<T> std::hash::Hash for $name<T> {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                self.raw.hash(state);
            }
        }

        impl<T> Default for $name<T> {
            fn default() -> Self {
                Self::NONE
            }
        }

        impl<T> fmt::Debug for $name<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if self.raw == NULL_LINK {
                    write!(f, "{}(none)", stringify!($name))
                } else {
                    write!(f, "{}({})", stringify!($name), self.raw)
                }
            }
        }

        impl<T> NifIo for $name<T> {
            const MIN_SIZE: usize = std::mem::size_of::<i32>();

            fn read(r: &mut Reader<'_>, _arg: i64) -> Result<Self> {
                Ok(Self::from_raw(i32::read(r, 0)?))
            }

            fn write(&self, w: &mut Writer, _arg: i64) -> Result<()> {
                self.raw.write(w, 0)
            }
        }
    };
}

link_type!(Ref, LinkKind::Ref, "A link the object owns (an SDK smart pointer or reference-counted list; schema `Ref`).");
link_type!(Ptr, LinkKind::Ptr, "A link the object does not own (an SDK raw pointer; schema `Ptr`).");
