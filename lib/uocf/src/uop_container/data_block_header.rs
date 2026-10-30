//! UOP data block headers: the `data_block_length` bytes between an entry's `data_block_address` and its
//! stored payload.
//!
//! Binary layout (as `UOSA.exe` reads it, `UOP::MypArchive_GetHeaderRecord` 0x0098d76e):
//! - A sequence of records, each:
//!   - `u16 record_type` (little-endian)
//!   - `u16 length` (little-endian)
//!   - `length` bytes of data
//! - Record types:
//!   - 1: a string ([`RECORD_TYPE_STRING`]); read by the client, found in no client package;
//!   - 3: a `u64` FILETIME ([`RECORD_TYPE_TIMESTAMP`]); the only record of version-4 packages;
//!   - 4: a signature ([`RECORD_TYPE_SIGNATURE`]); the only record of version-5 packages. Its data is
//!     `u16 key_id`, `u16 length` (both big-endian) and `length` bytes: a big-endian unsigned integer
//!     without leading zero bytes, so it is 126 to 129 bytes long and the record 134 to 137 bytes. Key 20
//!     signs Enhanced Client packages, key 21 Classic Client packages. The scheme and the public key are not
//!     in the client.
//!
//! The entry's `data_block_hash` is a hash of the whole header, chosen by its first record: the Adler-32 of
//! a timestamp header, the lookup3 `hashlittle` (initial value 0) of a signature header. All 282002 entries
//! of the Enhanced and Classic client packages of the reference release match.
//!
//! The client never checks either: every payload read starts at `data_block_address + data_block_length`,
//! no code reads a signature record, and `data_block_hash` is only compared with copies stored in the string
//! table and audio caches, to decide whether to rebuild them (see
//! `scripts/dev-helpers/ec-research/ghidra-uop-header/results/uosa_uop_header.md`).

use std::io;

use crate::uop_container::hash::{hash_data_block, hash_data_block_lookup3};

/// Size of a record's `u16 record_type` + `u16 length` prefix.
pub const RECORD_PREFIX_SIZE: usize = 4;
/// Record holding a string (`UOP::MypArchive_GetHeaderString` 0x0098d8d1 reads it).
pub const RECORD_TYPE_STRING: u16 = 1;
/// Record holding a `u64` FILETIME (`UOP::MypArchive_GetEntryTimestamp` 0x0098d53d reads it).
pub const RECORD_TYPE_TIMESTAMP: u16 = 3;
/// Record holding a signature; found as the only record of every version-5 package entry.
pub const RECORD_TYPE_SIGNATURE: u16 = 4;
/// Size of the FILETIME of a timestamp record.
pub const TIMESTAMP_SIZE: usize = 8;
/// Size of a signature record's big-endian `u16 key_id` + `u16 length` prefix.
pub const SIGNATURE_PREFIX_SIZE: usize = 4;
/// Signature key id of the Enhanced Client packages of the reference release.
pub const SIGNATURE_KEY_ID_ENHANCED: u16 = 20;
/// Signature key id of the Classic Client packages of the reference release.
pub const SIGNATURE_KEY_ID_CLASSIC: u16 = 21;

/// The signature of a signature record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataBlockSignature {
    /// Which key signed the entry ([`SIGNATURE_KEY_ID_ENHANCED`] or [`SIGNATURE_KEY_ID_CLASSIC`]).
    pub key_id: u16,
    /// The signature value: a big-endian unsigned integer without leading zero bytes.
    pub value: Vec<u8>,
}

/// One record of a data block header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataBlockHeaderRecord {
    /// A [`RECORD_TYPE_STRING`] record.
    String(Vec<u8>),
    /// A [`RECORD_TYPE_TIMESTAMP`] record of [`TIMESTAMP_SIZE`] bytes: a Windows FILETIME.
    Timestamp(u64),
    /// A [`RECORD_TYPE_SIGNATURE`] record whose inner length matches its data.
    Signature(DataBlockSignature),
    /// Any other record, or a timestamp or signature record whose data does not have the expected layout.
    Unknown { record_type: u16, data: Vec<u8> },
}

impl DataBlockHeaderRecord {
    /// The record's type.
    pub fn record_type(&self) -> u16 {
        match self {
            Self::String(_) => RECORD_TYPE_STRING,
            Self::Timestamp(_) => RECORD_TYPE_TIMESTAMP,
            Self::Signature(_) => RECORD_TYPE_SIGNATURE,
            Self::Unknown { record_type, .. } => *record_type,
        }
    }

    /// The record's data, without the type and length prefix.
    pub fn data(&self) -> Vec<u8> {
        match self {
            Self::String(data) | Self::Unknown { data, .. } => data.clone(),
            Self::Timestamp(filetime) => filetime.to_le_bytes().to_vec(),
            Self::Signature(signature) => {
                let mut data = Vec::with_capacity(SIGNATURE_PREFIX_SIZE + signature.value.len());
                data.extend_from_slice(&signature.key_id.to_be_bytes());
                data.extend_from_slice(&(signature.value.len() as u16).to_be_bytes());
                data.extend_from_slice(&signature.value);
                data
            }
        }
    }

    fn from_data(record_type: u16, data: &[u8]) -> Self {
        match record_type {
            RECORD_TYPE_STRING => return Self::String(data.to_vec()),
            RECORD_TYPE_TIMESTAMP => {
                if let Ok(filetime) = <[u8; TIMESTAMP_SIZE]>::try_from(data) {
                    return Self::Timestamp(u64::from_le_bytes(filetime));
                }
            }
            RECORD_TYPE_SIGNATURE if data.len() >= SIGNATURE_PREFIX_SIZE => {
                let key_id = u16::from_be_bytes([data[0], data[1]]);
                let length = u16::from_be_bytes([data[2], data[3]]) as usize;
                if length == data.len() - SIGNATURE_PREFIX_SIZE {
                    let value = data[SIGNATURE_PREFIX_SIZE..].to_vec();
                    return Self::Signature(DataBlockSignature { key_id, value });
                }
            }
            _ => {}
        }
        Self::Unknown { record_type, data: data.to_vec() }
    }
}

/// How an entry's `data_block_hash` is computed from its data block header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataBlockHashAlgorithm {
    /// Adler-32 of the header: headers starting with a timestamp record (version-4 packages).
    Adler32,
    /// lookup3 `hashlittle` with initial value 0 of the header: headers starting with a signature record
    /// (version-5 packages).
    Lookup3,
}

impl DataBlockHashAlgorithm {
    /// The `data_block_hash` of `header` bytes.
    pub fn hash(self, header: &[u8]) -> io::Result<u32> {
        match self {
            Self::Adler32 => hash_data_block(header),
            Self::Lookup3 => Ok(hash_data_block_lookup3(header)),
        }
    }
}

/// A parsed data block header.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DataBlockHeader {
    records: Vec<DataBlockHeaderRecord>,
}

impl DataBlockHeader {
    /// A header of `records`.
    pub fn new(records: Vec<DataBlockHeaderRecord>) -> Self {
        Self { records }
    }

    /// Parses header bytes. A record running past the end is an error; [`Self::to_bytes`] gives the bytes back.
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let mut records = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            let Some(prefix) = bytes.get(offset..offset + RECORD_PREFIX_SIZE) else {
                return Err(invalid(format!("data block header record prefix truncated at byte {offset}")));
            };
            let record_type = u16::from_le_bytes([prefix[0], prefix[1]]);
            let length = u16::from_le_bytes([prefix[2], prefix[3]]) as usize;
            let start = offset + RECORD_PREFIX_SIZE;
            let Some(data) = bytes.get(start..start + length) else {
                return Err(invalid(format!(
                    "data block header record of type {record_type} at byte {offset} needs {length} bytes, {} left",
                    bytes.len() - start
                )));
            };
            records.push(DataBlockHeaderRecord::from_data(record_type, data));
            offset = start + length;
        }
        Ok(Self { records })
    }

    /// The header bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for record in &self.records {
            let data = record.data();
            bytes.extend_from_slice(&record.record_type().to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&data);
        }
        bytes
    }

    /// The records in file order.
    pub fn records(&self) -> &[DataBlockHeaderRecord] {
        &self.records
    }

    /// The FILETIME of the first timestamp record.
    pub fn timestamp(&self) -> Option<u64> {
        self.records.iter().find_map(|record| match record {
            DataBlockHeaderRecord::Timestamp(filetime) => Some(*filetime),
            _ => None,
        })
    }

    /// The first signature record.
    pub fn signature(&self) -> Option<&DataBlockSignature> {
        self.records.iter().find_map(|record| match record {
            DataBlockHeaderRecord::Signature(signature) => Some(signature),
            _ => None,
        })
    }

    /// The `data_block_hash` algorithm of this header, from its first record. None for an empty header or one
    /// starting with another record: client packages have no such header, so their hash is not established.
    pub fn hash_algorithm(&self) -> Option<DataBlockHashAlgorithm> {
        match self.records.first()?.record_type() {
            RECORD_TYPE_TIMESTAMP => Some(DataBlockHashAlgorithm::Adler32),
            RECORD_TYPE_SIGNATURE => Some(DataBlockHashAlgorithm::Lookup3),
            _ => None,
        }
    }

    /// The `data_block_hash` of this header, when its algorithm is known ([`Self::hash_algorithm`]).
    pub fn data_block_hash(&self) -> io::Result<Option<u32>> {
        self.hash_algorithm().map(|algorithm| algorithm.hash(&self.to_bytes())).transpose()
    }
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
