//! UDDP package container layered on top of the existing UOP-oriented codec stack.
//!
//! Binary layout covered by this module:
//! - Fixed package header with magic/version/alignment and section offsets.
//! - Dictionary index section with one record per stored dictionary.
//! - Dictionary blob section, aligned for mmap-friendly access.
//! - Entry index section with one record per payload entry.
//! - Data section with aligned compressed payload blobs.
//! - Entry records carry both an `entry_key` hash and `codec_bits`, so the
//!   package can route lookup and decode without relying on legacy UOP block chains.
//! - The actual file payload of each entry is written in the final data section,
//!   at `data_offset`, after header, dictionary index, dictionary blobs, and
//!   entry index have all been serialized.

use crate::uop::codec::{align_up, decode_payload, encode_payload, CodecBits, UddpCompression, UddpContentId, UDDP_DEFAULT_ALIGNMENT};
use crate::uop::hash::{hash_data_block, hash_file_name_single};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use indexmap::IndexMap;
use nohash_hasher::BuildNoHashHasher;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

const UDDP_MAGIC: &[u8; 4] = b"UDDP";
const UDDP_VERSION: u32 = 1;
const HEADER_SIZE: u64 = 64;
const DICTIONARY_INDEX_ENTRY_SIZE: u64 = 24;
const ENTRY_INDEX_ENTRY_SIZE: u64 = 40;

#[derive(Debug, Clone)]
pub struct UddpDictionary {
    content_id: u16,
    checksum: u32,
    data: Arc<[u8]>,
}

impl UddpDictionary {
    pub fn content_id(&self) -> u16 {
        self.content_id
    }

    pub fn checksum(&self) -> u32 {
        self.checksum
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Debug, Clone)]
pub struct UddpEntry {
    entry_key: u64,
    /// Semantic payload kind. This duplicates the information encoded in
    /// `codec_bits` to keep entry inspection straightforward while debugging.
    content_id: u16,
    codec_bits: CodecBits,
    /// Absolute file offset of the compressed payload blob inside the final
    /// UDDP data section.
    data_offset: u64,
    compressed_size: u32,
    raw_size: u32,
    checksum: u32,
    data: Option<Arc<[u8]>>,
}

impl UddpEntry {
    pub fn entry_key(&self) -> u64 {
        self.entry_key
    }

    pub fn content_id(&self) -> u16 {
        self.content_id
    }

    pub fn codec_bits(&self) -> CodecBits {
        self.codec_bits
    }

    pub fn typed_content_id(&self) -> Option<UddpContentId> {
        UddpContentId::from_u16(self.content_id)
    }

    pub fn data_offset(&self) -> u64 {
        self.data_offset
    }

    pub fn compressed_size(&self) -> u32 {
        self.compressed_size
    }

    pub fn raw_size(&self) -> u32 {
        self.raw_size
    }

    pub fn checksum(&self) -> u32 {
        self.checksum
    }

    pub fn data(&self) -> Option<&Arc<[u8]>> {
        self.data.as_ref()
    }

    pub fn unpack(&self) -> io::Result<Vec<u8>> {
        let data = self.data.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "UDDP entry data has not been loaded into memory",
            )
        })?;

        let compression = self.codec_bits.compression()?;
        decode_payload(data, self.raw_size as usize, compression)
    }
}

#[derive(Debug, Clone)]
pub struct UddpPackage {
    version: u32,
    alignment: u64,
    dictionaries: Vec<UddpDictionary>,
    entries: IndexMap<u64, UddpEntry, BuildNoHashHasher<u64>>,
}

impl UddpPackage {
    pub fn new() -> Self {
        Self {
            version: UDDP_VERSION,
            alignment: UDDP_DEFAULT_ALIGNMENT,
            dictionaries: Vec::new(),
            entries: IndexMap::with_hasher(BuildNoHashHasher::default()),
        }
    }

    pub fn with_alignment(alignment: u64) -> io::Result<Self> {
        align_up(0, alignment)?;
        Ok(Self {
            alignment,
            ..Self::new()
        })
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn alignment(&self) -> u64 {
        self.alignment
    }

    pub fn dictionaries(&self) -> &[UddpDictionary] {
        &self.dictionaries
    }

    pub fn entries(&self) -> &IndexMap<u64, UddpEntry, BuildNoHashHasher<u64>> {
        &self.entries
    }

    pub fn add_dictionary(&mut self, content_id: u16, data: &[u8]) -> io::Result<()> {
        let checksum = hash_data_block(data)?;
        self.dictionaries.retain(|dict| dict.content_id != content_id);
        self.dictionaries.push(UddpDictionary {
            content_id,
            checksum,
            data: Arc::from(data.to_vec()),
        });
        Ok(())
    }

    pub fn get_dictionary(&self, content_id: u16) -> Option<&UddpDictionary> {
        self.dictionaries
            .iter()
            .find(|dictionary| dictionary.content_id == content_id)
    }

    pub fn add_entry_from_memory(
        &mut self,
        payload: &[u8],
        internal_path: &str,
        content_id: u16,
        compression: UddpCompression,
    ) -> io::Result<u64> {
        let entry_key = hash_file_name_single(internal_path);
        self.add_entry_with_key(payload, entry_key, content_id, compression)?;
        Ok(entry_key)
    }

    pub fn add_typed_entry_from_memory(
        &mut self,
        payload: &[u8],
        internal_path: &str,
        content_id: UddpContentId,
        compression: UddpCompression,
    ) -> io::Result<u64> {
        self.add_entry_from_memory(payload, internal_path, content_id.into(), compression)
    }

    pub fn add_entry_with_key(
        &mut self,
        payload: &[u8],
        entry_key: u64,
        content_id: u16,
        compression: UddpCompression,
    ) -> io::Result<()> {
        let codec_bits = CodecBits::new(content_id, compression)?;
        let encoded = encode_payload(payload, compression)?;
        let checksum = hash_data_block(&encoded)?;

        self.entries.insert(
            entry_key,
            UddpEntry {
                entry_key,
                content_id,
                codec_bits,
                data_offset: 0,
                compressed_size: encoded.len() as u32,
                raw_size: payload.len() as u32,
                checksum,
                data: Some(Arc::from(encoded)),
            },
        );
        Ok(())
    }

    pub fn get_entry_by_key(&self, entry_key: u64) -> Option<&UddpEntry> {
        self.entries.get(&entry_key)
    }

    pub fn get_entry_by_path(&self, internal_path: &str) -> Option<&UddpEntry> {
        self.get_entry_by_key(hash_file_name_single(internal_path))
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
        let alignment = self.alignment;
        align_up(0, alignment)?;

        // Phase 1: assign file offsets to dictionary blobs.
        let mut dictionaries = Vec::with_capacity(self.dictionaries.len());
        let dictionary_index_offset = HEADER_SIZE;
        let mut cursor = align_up(
            dictionary_index_offset
                + (self.dictionaries.len() as u64) * DICTIONARY_INDEX_ENTRY_SIZE,
            alignment,
        )?;

        for dictionary in &self.dictionaries {
            let data = dictionary.data.clone();
            let offset = cursor;
            cursor = align_up(offset + data.len() as u64, alignment)?;
            dictionaries.push((dictionary.content_id, dictionary.checksum, offset, data));
        }

        // Phase 2: place the entry index after the dictionary blob area.
        let entry_index_offset = align_up(cursor, alignment)?;
        let mut data_cursor = align_up(
            entry_index_offset + (self.entries.len() as u64) * ENTRY_INDEX_ENTRY_SIZE,
            alignment,
        )?;

        // Phase 3: assign each entry payload a final position inside the data section.
        let mut prepared_entries = Vec::with_capacity(self.entries.len());
        for entry in self.entries.values() {
            let data = entry.data.clone().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("UDDP entry {} has no in-memory payload", entry.entry_key),
                )
            })?;
            let offset = data_cursor;
            data_cursor = align_up(offset + data.len() as u64, alignment)?;
            prepared_entries.push((entry.clone(), offset, data));
        }

        let data_section_offset = prepared_entries
            .first()
            .map(|(_, offset, _)| *offset)
            .unwrap_or_else(|| align_up(entry_index_offset + (self.entries.len() as u64) * ENTRY_INDEX_ENTRY_SIZE, alignment).unwrap_or(entry_index_offset));
        let data_section_size = data_cursor.saturating_sub(data_section_offset);

        // Phase 4: write package header and indices first, then dictionary blobs,
        // then finally the compressed payloads in the data section.
        writer.seek(SeekFrom::Start(0))?;
        writer.write_all(UDDP_MAGIC)?;
        writer.write_u32::<LittleEndian>(self.version)?;
        writer.write_u32::<LittleEndian>(alignment as u32)?;
        writer.write_u32::<LittleEndian>(0)?;
        writer.write_u64::<LittleEndian>(dictionary_index_offset)?;
        writer.write_u32::<LittleEndian>(self.dictionaries.len() as u32)?;
        writer.write_u64::<LittleEndian>(entry_index_offset)?;
        writer.write_u32::<LittleEndian>(self.entries.len() as u32)?;
        writer.write_u64::<LittleEndian>(data_section_offset)?;
        writer.write_u64::<LittleEndian>(data_section_size)?;
        writer.write_u64::<LittleEndian>(0)?;

        for (content_id, checksum, offset, data) in &dictionaries {
            writer.write_u16::<LittleEndian>(*content_id)?;
            writer.write_u16::<LittleEndian>(0)?;
            writer.write_u64::<LittleEndian>(*offset)?;
            writer.write_u32::<LittleEndian>(data.len() as u32)?;
            writer.write_u32::<LittleEndian>(*checksum)?;
            writer.write_u32::<LittleEndian>(0)?;
        }

        pad_to(writer, align_up(
            dictionary_index_offset
                + (self.dictionaries.len() as u64) * DICTIONARY_INDEX_ENTRY_SIZE,
            alignment,
        )?)?;

        for (_, _, offset, data) in &dictionaries {
            pad_to(writer, *offset)?;
            writer.write_all(data)?;
        }

        pad_to(writer, entry_index_offset)?;
        for (entry, offset, data) in &prepared_entries {
            writer.write_u64::<LittleEndian>(entry.entry_key)?;
            writer.write_u16::<LittleEndian>(entry.codec_bits.raw())?;
            writer.write_u16::<LittleEndian>(entry.content_id)?;
            writer.write_u64::<LittleEndian>(*offset)?;
            writer.write_u32::<LittleEndian>(data.len() as u32)?;
            writer.write_u32::<LittleEndian>(entry.raw_size)?;
            writer.write_u32::<LittleEndian>(entry.checksum)?;
            writer.write_u32::<LittleEndian>(0)?;
            writer.write_u32::<LittleEndian>(0)?;
        }

        // This is the actual UDDP payload area: every entry's compressed bytes are
        // emitted here at the `data_offset` recorded above.
        for (_, offset, data) in &prepared_entries {
            pad_to(writer, *offset)?;
            writer.write_all(data)?;
        }

        Ok(())
    }

    pub fn load_from_reader<R: Read + Seek>(reader: &mut R) -> io::Result<Self> {
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != UDDP_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid UDDP magic",
            ));
        }

        let version = reader.read_u32::<LittleEndian>()?;
        let alignment = reader.read_u32::<LittleEndian>()? as u64;
        let _reserved0 = reader.read_u32::<LittleEndian>()?;
        let dictionary_index_offset = reader.read_u64::<LittleEndian>()?;
        let dictionary_count = reader.read_u32::<LittleEndian>()?;
        let entry_index_offset = reader.read_u64::<LittleEndian>()?;
        let entry_count = reader.read_u32::<LittleEndian>()?;
        let _data_section_offset = reader.read_u64::<LittleEndian>()?;
        let _data_section_size = reader.read_u64::<LittleEndian>()?;
        let _reserved1 = reader.read_u64::<LittleEndian>()?;

        align_up(0, alignment)?;

        // Read dictionary index, then jump to each dictionary blob offset.
        reader.seek(SeekFrom::Start(dictionary_index_offset))?;
        let mut dictionaries = Vec::with_capacity(dictionary_count as usize);
        for _ in 0..dictionary_count {
            let content_id = reader.read_u16::<LittleEndian>()?;
            let _reserved = reader.read_u16::<LittleEndian>()?;
            let offset = reader.read_u64::<LittleEndian>()?;
            let size = reader.read_u32::<LittleEndian>()?;
            let checksum = reader.read_u32::<LittleEndian>()?;
            let _reserved2 = reader.read_u32::<LittleEndian>()?;

            let return_pos = reader.stream_position()?;
            reader.seek(SeekFrom::Start(offset))?;
            let mut data = vec![0u8; size as usize];
            reader.read_exact(&mut data)?;
            reader.seek(SeekFrom::Start(return_pos))?;

            dictionaries.push(UddpDictionary {
                content_id,
                checksum,
                data: Arc::from(data),
            });
        }

        // Read entry index, then jump to each payload blob in the data section.
        reader.seek(SeekFrom::Start(entry_index_offset))?;
        let mut entries = IndexMap::with_capacity_and_hasher(
            entry_count as usize,
            BuildNoHashHasher::default(),
        );

        for _ in 0..entry_count {
            let entry_key = reader.read_u64::<LittleEndian>()?;
            let codec_bits = CodecBits::from_raw(reader.read_u16::<LittleEndian>()?);
            let content_id = reader.read_u16::<LittleEndian>()?;
            let data_offset = reader.read_u64::<LittleEndian>()?;
            let compressed_size = reader.read_u32::<LittleEndian>()?;
            let raw_size = reader.read_u32::<LittleEndian>()?;
            let checksum = reader.read_u32::<LittleEndian>()?;
            let _reserved0 = reader.read_u32::<LittleEndian>()?;
            let _reserved1 = reader.read_u32::<LittleEndian>()?;

            let return_pos = reader.stream_position()?;
            reader.seek(SeekFrom::Start(data_offset))?;
            let mut data = vec![0u8; compressed_size as usize];
            reader.read_exact(&mut data)?;
            reader.seek(SeekFrom::Start(return_pos))?;

            entries.insert(
                entry_key,
                UddpEntry {
                    entry_key,
                    content_id,
                    codec_bits,
                    data_offset,
                    compressed_size,
                    raw_size,
                    checksum,
                    data: Some(Arc::from(data)),
                },
            );
        }

        Ok(Self {
            version,
            alignment,
            dictionaries,
            entries,
        })
    }
}

impl Default for UddpPackage {
    fn default() -> Self {
        Self::new()
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
