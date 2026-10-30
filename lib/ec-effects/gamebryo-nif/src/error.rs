//! Errors raised while reading or writing a NIF file.
//!
//! Every error carries what went wrong ([`ErrorKind`]) plus where: the byte offset in the input
//! (reading) or output (writing), the block index and type, and the field path inside the block
//! (for example `keys[3].value`).

use std::fmt;

/// What went wrong.
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorKind {
    /// The input ended before a value could be read.
    UnexpectedEof { needed: usize, available: usize },
    /// The first line is not a `Gamebryo File Format, Version x.x.x.x` header string.
    BadHeaderString,
    /// The file version is not the one this crate was generated for.
    UnsupportedVersion { version: u32 },
    /// The user version changes the layout in ways this crate was not generated for.
    UnsupportedUserVersion { user_version: u32 },
    /// Only little-endian files are supported.
    UnsupportedEndian { endian: u8 },
    /// The header names a block type this crate does not know.
    UnknownBlockType { name: String },
    /// A block type index in the header is outside the block type table, or uses the
    /// PhysX flag bit, which this crate does not support.
    BadBlockTypeIndex { index: u16, num_types: usize },
    /// An array length expression evaluated to a negative number.
    NegativeLength { value: i64 },
    /// An array length exceeds the remaining input, so the file is corrupt or misread.
    LengthPastEof { length: usize, min_bytes: usize, available: usize },
    /// On write, an array does not have the length its count field (or expression) says.
    LengthMismatch { expected: usize, actual: usize },
    /// On write, a count does not fit the integer type that stores it.
    CountOverflow { count: usize },
    /// Bytes are left after the footer.
    TrailingData { bytes: usize },
}

/// One step of a field path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSegment {
    /// A field, by its schema name.
    Field(&'static str),
    /// An array element.
    Index(usize),
}

/// The block an error happened in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockContext {
    pub index: usize,
    pub type_name: String,
}

/// A read or write error with its location.
#[derive(Debug, Clone, PartialEq)]
pub struct NifError {
    pub kind: ErrorKind,
    /// Byte offset in the input (read) or the output written so far (write).
    pub offset: Option<usize>,
    pub block: Option<BlockContext>,
    /// Innermost segment first; use [`NifError::field_path`] for a readable form.
    path: Vec<PathSegment>,
}

pub type Result<T, E = NifError> = std::result::Result<T, E>;

impl NifError {
    pub fn new(kind: ErrorKind, offset: Option<usize>) -> Self {
        Self { kind, offset, block: None, path: Vec::new() }
    }

    /// Adds the enclosing field name (called while the error unwinds).
    pub fn in_field(mut self, name: &'static str) -> Self {
        self.path.push(PathSegment::Field(name));
        self
    }

    /// Adds the enclosing array index (called while the error unwinds).
    pub fn at_index(mut self, index: usize) -> Self {
        self.path.push(PathSegment::Index(index));
        self
    }

    /// Records the block the error happened in, unless an inner block was already recorded.
    pub fn in_block(mut self, index: usize, type_name: &str) -> Self {
        if self.block.is_none() {
            self.block = Some(BlockContext { index, type_name: type_name.to_owned() });
        }
        self
    }

    /// Path segments, outermost first.
    pub fn path(&self) -> impl Iterator<Item = PathSegment> + '_ {
        self.path.iter().rev().copied()
    }

    /// The field path, outermost first, e.g. `keys[3].value`.
    pub fn field_path(&self) -> String {
        let mut out = String::new();
        for segment in self.path() {
            match segment {
                PathSegment::Field(name) => {
                    if !out.is_empty() {
                        out.push('.');
                    }
                    out.push_str(name);
                }
                PathSegment::Index(index) => out.push_str(&format!("[{index}]")),
            }
        }
        out
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof { needed, available } => {
                write!(f, "unexpected end of data (need {needed} bytes, {available} left)")
            }
            Self::BadHeaderString => write!(f, "not a Gamebryo NIF header string"),
            Self::UnsupportedVersion { version } => {
                write!(f, "unsupported NIF version {}", crate::version_string(*version))
            }
            Self::UnsupportedUserVersion { user_version } => {
                write!(f, "unsupported NIF user version {user_version}")
            }
            Self::UnsupportedEndian { endian } => write!(f, "unsupported endian type {endian}"),
            Self::UnknownBlockType { name } => write!(f, "unknown block type '{name}'"),
            Self::BadBlockTypeIndex { index, num_types } => {
                write!(f, "block type index {index:#06x} is invalid ({num_types} block types)")
            }
            Self::NegativeLength { value } => write!(f, "array length {value} is negative"),
            Self::LengthPastEof { length, min_bytes, available } => write!(
                f,
                "array length {length} needs at least {min_bytes} bytes, only {available} left"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                f,
                "array has {actual} elements but its length field says {expected}"
            ),
            Self::CountOverflow { count } => write!(f, "count {count} does not fit its field"),
            Self::TrailingData { bytes } => write!(f, "{bytes} unread bytes after the footer"),
        }
    }
}

impl fmt::Display for NifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(block) = &self.block {
            write!(f, "block {} ({})", block.index, block.type_name)?;
        } else {
            write!(f, "header/footer")?;
        }
        if !self.path.is_empty() {
            write!(f, ", field {}", self.field_path())?;
        }
        if let Some(offset) = self.offset {
            write!(f, ", offset {offset:#x}")?;
        }
        write!(f, ": {}", self.kind)
    }
}

impl std::error::Error for NifError {}
