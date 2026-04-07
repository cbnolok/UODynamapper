//! UDDF single-payload wrapper for standalone artifacts using the UDDP codec model.
//!
//! Binary layout covered by this module:
//! - Fixed wrapper header with magic/version/payload format/codec bits.
//! - `u128` flags split across two `u64` words on disk.
//! - Aligned payload area starting at `payload_offset`.
//! - The payload bytes are stored raw or compressed according to `codec_bits`.
//! - In practice the payload is the very last part of the file: first the fixed
//!   header is written, then zero padding up to `payload_offset`, then the
//!   payload blob itself.

use crate::uop::codec::{align_up, decode_payload, encode_payload, CodecBits, UddpCompression, UddpContentId, UDDP_DEFAULT_ALIGNMENT};
use crate::uop::hash::hash_data_block;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

const UDDF_MAGIC: &[u8; 4] = b"UDDF";
const UDDF_VERSION: u32 = 1;
const UDDF_HEADER_SIZE: u64 = 72;

#[derive(Debug, Clone)]
pub struct UddfFile {
    version: u32,
    alignment: u64,
    payload_format: u32,
    codec_bits: CodecBits,
    flags: u128,
    /// Absolute file offset where the payload blob begins.
    payload_offset: u64,
    compressed_size: u64,
    raw_size: u64,
    checksum: u32,
    payload: Arc<[u8]>,
}

impl UddfFile {
    pub fn wrap_bytes(
        payload: &[u8],
        payload_format: u32,
        content_id: u16,
        compression: UddpCompression,
        flags: u128,
    ) -> io::Result<Self> {
        let codec_bits = CodecBits::new(content_id, compression)?;
        let encoded = encode_payload(payload, compression)?;
        let checksum = hash_data_block(&encoded)?;

        Ok(Self {
            version: UDDF_VERSION,
            alignment: UDDP_DEFAULT_ALIGNMENT,
            payload_format,
            codec_bits,
            flags,
            payload_offset: align_up(UDDF_HEADER_SIZE, UDDP_DEFAULT_ALIGNMENT)?,
            compressed_size: encoded.len() as u64,
            raw_size: payload.len() as u64,
            checksum,
            payload: Arc::from(encoded),
        })
    }

    pub fn wrap_typed_bytes(
        payload: &[u8],
        payload_format: u32,
        content_id: UddpContentId,
        compression: UddpCompression,
        flags: u128,
    ) -> io::Result<Self> {
        Self::wrap_bytes(payload, payload_format, content_id.into(), compression, flags)
    }

    pub fn payload_format(&self) -> u32 {
        self.payload_format
    }

    pub fn codec_bits(&self) -> CodecBits {
        self.codec_bits
    }

    pub fn typed_content_id(&self) -> Option<UddpContentId> {
        self.codec_bits.typed_content_id()
    }

    pub fn flags(&self) -> u128 {
        self.flags
    }

    pub fn payload_offset(&self) -> u64 {
        self.payload_offset
    }

    pub fn compressed_size(&self) -> u64 {
        self.compressed_size
    }

    pub fn raw_size(&self) -> u64 {
        self.raw_size
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn unpack(&self) -> io::Result<Vec<u8>> {
        let compression = self.codec_bits.compression()?;
        decode_payload(&self.payload, self.raw_size as usize, compression)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let mut file = File::create(path)?;
        self.save_to_writer(&mut file)
    }

    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut file = File::open(path)?;
        Self::load_from_reader(&mut file)
    }

    pub fn save_to_writer<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        let payload_offset = align_up(UDDF_HEADER_SIZE, self.alignment)?;

        // Write the fixed header first. The payload itself is not embedded inside
        // the header; it is appended later at `payload_offset`.
        writer.seek(SeekFrom::Start(0))?;
        writer.write_all(UDDF_MAGIC)?;
        writer.write_u32::<LittleEndian>(self.version)?;
        writer.write_u32::<LittleEndian>(self.payload_format)?;
        writer.write_u16::<LittleEndian>(self.codec_bits.raw())?;
        writer.write_u16::<LittleEndian>(0)?;
        writer.write_u64::<LittleEndian>(payload_offset)?;
        writer.write_u64::<LittleEndian>(self.compressed_size)?;
        writer.write_u64::<LittleEndian>(self.raw_size)?;
        writer.write_u32::<LittleEndian>(self.checksum)?;
        writer.write_u32::<LittleEndian>(self.alignment as u32)?;
        writer.write_u64::<LittleEndian>(self.flags as u64)?;
        writer.write_u64::<LittleEndian>((self.flags >> 64) as u64)?;
        writer.write_u64::<LittleEndian>(0)?;

        // Fill the alignment gap, then emit the single wrapped payload blob.
        pad_to(writer, payload_offset)?;
        writer.write_all(&self.payload)?;
        Ok(())
    }

    pub fn load_from_reader<R: Read + Seek>(reader: &mut R) -> io::Result<Self> {
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != UDDF_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid UDDF magic",
            ));
        }

        let version = reader.read_u32::<LittleEndian>()?;
        let payload_format = reader.read_u32::<LittleEndian>()?;
        let codec_bits = CodecBits::from_raw(reader.read_u16::<LittleEndian>()?);
        let _reserved0 = reader.read_u16::<LittleEndian>()?;
        let payload_offset = reader.read_u64::<LittleEndian>()?;
        let compressed_size = reader.read_u64::<LittleEndian>()?;
        let raw_size = reader.read_u64::<LittleEndian>()?;
        let checksum = reader.read_u32::<LittleEndian>()?;
        let alignment = reader.read_u32::<LittleEndian>()? as u64;
        let flags_low = reader.read_u64::<LittleEndian>()?;
        let flags_high = reader.read_u64::<LittleEndian>()?;
        let _reserved1 = reader.read_u64::<LittleEndian>()?;

        align_up(0, alignment)?;

        // After parsing the fixed header, jump to the payload area and read the
        // single blob referenced by `payload_offset` and `compressed_size`.
        reader.seek(SeekFrom::Start(payload_offset))?;
        let mut payload = vec![0u8; compressed_size as usize];
        reader.read_exact(&mut payload)?;

        Ok(Self {
            version,
            alignment,
            payload_format,
            codec_bits,
            flags: ((flags_high as u128) << 64) | flags_low as u128,
            payload_offset,
            compressed_size,
            raw_size,
            checksum,
            payload: Arc::from(payload),
        })
    }
}

fn pad_to<W: Write + Seek>(writer: &mut W, target_offset: u64) -> io::Result<()> {
    let current = writer.stream_position()?;
    if current > target_offset {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("cannot pad backwards from {current} to {target_offset}"),
        ));
    }

    let padding = (target_offset - current) as usize;
    if padding > 0 {
        writer.write_all(&vec![0u8; padding])?;
    }
    Ok(())
}
