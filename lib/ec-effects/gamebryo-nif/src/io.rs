//! Little-endian byte reader/writer and the [`NifIo`] trait every field type implements.
//!
//! The generated code calls the `rd*`/`wr*` helpers, which attach the field name (and array
//! index) to any error so it reports a path like `Data.Keys[3].Value`.

use crate::error::{ErrorKind, NifError, Result};

/// Reads bytes from a NIF buffer, tracking the position for error offsets.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn error(&self, kind: ErrorKind) -> NifError {
        NifError::new(kind, Some(self.pos))
    }

    pub fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        if len > self.remaining() {
            return Err(self.error(ErrorKind::UnexpectedEof {
                needed: len,
                available: self.remaining(),
            }));
        }
        let out = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(out)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self.bytes(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    /// Reads up to (and consumes) the next `\n`, returning the line without it.
    pub fn line(&mut self) -> Result<&'a [u8]> {
        let rest = &self.data[self.pos..];
        let Some(end) = rest.iter().position(|&b| b == b'\n') else {
            return Err(self.error(ErrorKind::BadHeaderString));
        };
        self.pos += end + 1;
        Ok(&rest[..end])
    }
}

/// Collects the bytes of a NIF file being written.
#[derive(Default)]
pub struct Writer {
    out: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn position(&self) -> usize {
        self.out.len()
    }

    pub fn error(&self, kind: ErrorKind) -> NifError {
        NifError::new(kind, Some(self.out.len()))
    }

    pub fn bytes(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.out
    }
}

/// Binary serialization of a NIF value.
///
/// `arg` is a value the enclosing field passes down (for example a key array passes its key
/// type to each key, as the SDK's key create functions do). Types that do not use it ignore it.
pub trait NifIo: Sized {
    /// Minimum encoded size in bytes, used to reject absurd array lengths before allocating.
    const MIN_SIZE: usize;

    fn read(r: &mut Reader<'_>, arg: i64) -> Result<Self>;
    fn write(&self, w: &mut Writer, arg: i64) -> Result<()>;
}

/// Integer view of a value used in schema condition and length expressions.
pub trait Ev {
    fn ev(&self) -> i64;
}

macro_rules! impl_primitive {
    ($($ty:ty),*) => {$(
        impl NifIo for $ty {
            const MIN_SIZE: usize = std::mem::size_of::<$ty>();

            fn read(r: &mut Reader<'_>, _arg: i64) -> Result<Self> {
                Ok(<$ty>::from_le_bytes(r.array()?))
            }

            fn write(&self, w: &mut Writer, _arg: i64) -> Result<()> {
                w.bytes(&self.to_le_bytes());
                Ok(())
            }
        }
    )*};
}

impl_primitive!(u8, i8, u16, i16, u32, i32, u64, i64, f32);

macro_rules! impl_ev_int {
    ($($ty:ty),*) => {$(
        impl Ev for $ty {
            fn ev(&self) -> i64 {
                *self as i64
            }
        }
    )*};
}

impl_ev_int!(u8, i8, u16, i16, u32, i32, u64, i64);

/// Converts an evaluated length expression to an element count.
fn length(value: i64, offset: usize) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_| NifError::new(ErrorKind::NegativeLength { value }, Some(offset)))
}

/// Reads one field.
#[inline]
pub(crate) fn rd<T: NifIo>(r: &mut Reader<'_>, name: &'static str, arg: i64) -> Result<T> {
    T::read(r, arg).map_err(|e| e.in_field(name))
}

/// Reads `count` elements, without the field name in errors.
fn read_elements<T: NifIo>(r: &mut Reader<'_>, count: i64, arg: i64) -> Result<Vec<T>> {
    let count = length(count, r.position())?;
    let min_bytes = count.saturating_mul(T::MIN_SIZE);
    if min_bytes > r.remaining() {
        return Err(r.error(ErrorKind::LengthPastEof {
            length: count,
            min_bytes,
            available: r.remaining(),
        }));
    }
    let mut out = Vec::with_capacity(count.min(r.remaining()));
    for index in 0..count {
        out.push(T::read(r, arg).map_err(|e| e.at_index(index))?);
    }
    Ok(out)
}

/// Reads a one-dimensional array field.
pub(crate) fn rd_vec<T: NifIo>(
    r: &mut Reader<'_>,
    name: &'static str,
    count: i64,
    arg: i64,
) -> Result<Vec<T>> {
    read_elements(r, count, arg).map_err(|e| e.in_field(name))
}

/// Reads an array field that stores its own element count (of integer type `C`) in front.
pub(crate) fn rd_counted<C: NifIo + Ev, T: NifIo>(
    r: &mut Reader<'_>,
    name: &'static str,
    arg: i64,
) -> Result<Vec<T>> {
    let read = |r: &mut Reader<'_>| {
        let count = C::read(r, 0)?.ev();
        read_elements(r, count, arg)
    };
    read(r).map_err(|e| e.in_field(name))
}

/// Reads a two-dimensional array field with `count` rows of `width` elements.
pub(crate) fn rd_vec2<T: NifIo>(
    r: &mut Reader<'_>,
    name: &'static str,
    count: i64,
    width: i64,
    arg: i64,
) -> Result<Vec<Vec<T>>> {
    let rows = (|| {
        let count = length(count, r.position())?;
        let mut rows = Vec::with_capacity(count.min(r.remaining()));
        for index in 0..count {
            rows.push(read_elements(r, width, arg).map_err(|e| e.at_index(index))?);
        }
        Ok(rows)
    })();
    rows.map_err(|e: NifError| e.in_field(name))
}

/// Writes one field.
#[inline]
pub(crate) fn wr<T: NifIo>(w: &mut Writer, name: &'static str, value: &T, arg: i64) -> Result<()> {
    value.write(w, arg).map_err(|e| e.in_field(name))
}

/// Writes elements after checking the array has the length its count says.
fn write_elements<T: NifIo>(w: &mut Writer, values: &[T], count: i64, arg: i64) -> Result<()> {
    let expected = length(count, w.position())?;
    if values.len() != expected {
        return Err(w.error(ErrorKind::LengthMismatch { expected, actual: values.len() }));
    }
    for (index, value) in values.iter().enumerate() {
        value.write(w, arg).map_err(|e| e.at_index(index))?;
    }
    Ok(())
}

/// Writes a one-dimensional array field.
pub(crate) fn wr_vec<T: NifIo>(
    w: &mut Writer,
    name: &'static str,
    values: &[T],
    count: i64,
    arg: i64,
) -> Result<()> {
    write_elements(w, values, count, arg).map_err(|e| e.in_field(name))
}

/// Writes an array field preceded by its element count, stored as integer type `C`.
pub(crate) fn wr_counted<C: NifIo + TryFrom<usize>, T: NifIo>(
    w: &mut Writer,
    name: &'static str,
    values: &[T],
    arg: i64,
) -> Result<()> {
    let write = |w: &mut Writer| {
        let count = C::try_from(values.len())
            .map_err(|_| w.error(ErrorKind::CountOverflow { count: values.len() }))?;
        count.write(w, 0)?;
        for (index, value) in values.iter().enumerate() {
            value.write(w, arg).map_err(|e| e.at_index(index))?;
        }
        Ok(())
    };
    write(w).map_err(|e: NifError| e.in_field(name))
}

/// Writes a two-dimensional array field with `count` rows of `width` elements.
pub(crate) fn wr_vec2<T: NifIo>(
    w: &mut Writer,
    name: &'static str,
    rows: &[Vec<T>],
    count: i64,
    width: i64,
    arg: i64,
) -> Result<()> {
    let result = (|| {
        let expected = length(count, w.position())?;
        if rows.len() != expected {
            return Err(w.error(ErrorKind::LengthMismatch { expected, actual: rows.len() }));
        }
        for (index, row) in rows.iter().enumerate() {
            write_elements(w, row, width, arg).map_err(|e| e.at_index(index))?;
        }
        Ok(())
    })();
    result.map_err(|e| e.in_field(name))
}
