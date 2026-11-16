//! `kr_delta.uddp`: the Kingdom Reborn client files Dynamapper packs, as a delta over an Enhanced Client.
//!
//! The packers read six Kingdom Reborn client files ([`KR_DELTA_CLIENT_FILES`]). This package rebuilds them from an
//! Enhanced Client, so the Enhanced packages carry the Kingdom Reborn data without the Kingdom Reborn client
//! (`udd_conv::kr_delta` makes and applies it). A file is its layout (every byte that is not an entry payload: the
//! UOP header, block tables, data block headers and gaps) and its entries' payloads in file order, each the entry's
//! bytes compressed as the client stores them (`uocf::uop_container::codec::encode_payload` reproduces every client
//! zlib stream). An entry's bytes are stored here, or taken from the Enhanced entry of the same name
//! ([`KrDeltaSource`]): a texture that is the Enhanced one, byte for byte or re-encoded (within
//! `udd_conv::textures::kr_items::KR_IMAGE_EQUIVALENCE_THRESHOLD`), is the Enhanced entry. So the rebuilt data files
//! (tileart, string dictionary, terrain definitions) are the Kingdom Reborn ones byte for byte, and a texture file is
//! too unless it held textures Kingdom Reborn only re-encoded, which become the Enhanced Client's.
//!
//! Layout (lookup by virtual path):
//! - [`KR_DELTA_MANIFEST_ENTRY_PATH`]: magic [`KR_DELTA_MAGIC`], `u32` [`KR_DELTA_FORMAT_VERSION`], the Kingdom Reborn
//!   and Enhanced client descriptions (information only, never checked; `u16` length and UTF-8 each), `u32` file
//!   count, then per file: its name (`u8` length, UTF-8), `u64` length and `u64` XXH64 of the whole Kingdom Reborn
//!   file, `u32` entry count, then per entry with a payload, in file order of the payloads: `u8` [`KrDeltaSource`]
//!   and `u64` [`KrDeltaEntry::hash64`]. The block tables in the layout give each entry's name, lengths and
//!   compression.
//! - [`layout_entry_path`]: the file's layout bytes, concatenated.
//! - [`stored_entry_path`]: the entries the file stores itself ([`KrDeltaSource::Stored`] decompressed,
//!   [`KrDeltaSource::Verbatim`] as the client stores them), concatenated in file order.
//!
//! A package of another magic or version fails to load with a message to make it again.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use udd_container::UddpReader;

use crate::common::{ensure_manifest_version, read_path_entry};

/// The file name of the package, which `udd-cli tool validate` recognises it by.
pub const KR_DELTA_FILE_NAME: &str = "kr_delta.uddp";
/// The manifest entry.
pub const KR_DELTA_MANIFEST_ENTRY_PATH: &str = "metadata/kr_delta.bin";
pub const KR_DELTA_MAGIC: [u8; 4] = *b"KRDL";
/// Version of the whole package, stored after the manifest magic.
pub const KR_DELTA_FORMAT_VERSION: u32 = 1;

/// What a Kingdom Reborn file holds: textures may be the Enhanced entries of the same name, while the other files are
/// stored whole, so they never depend on the Enhanced Client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KrDeltaFileKind {
    Textures,
    Data,
}

/// The Kingdom Reborn client files the packers read, which the package rebuilds.
pub const KR_DELTA_CLIENT_FILES: [(&str, KrDeltaFileKind); 6] = [
    ("TerrainDefinition.uop", KrDeltaFileKind::Data),
    ("Texture.uop", KrDeltaFileKind::Textures),
    ("tileart.uop", KrDeltaFileKind::Data),
    ("string_dictionary.uop", KrDeltaFileKind::Data),
    ("TerrainTexture.uop", KrDeltaFileKind::Textures),
    ("SystemTextures.uop", KrDeltaFileKind::Textures),
];

/// The kind of the client file `name`, `None` for a file the package does not rebuild.
pub fn client_file_kind(name: &str) -> Option<KrDeltaFileKind> {
    KR_DELTA_CLIENT_FILES.iter().find(|(known, _)| *known == name).map(|&(_, kind)| kind)
}

/// Where an entry's bytes come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum KrDeltaSource {
    /// The Enhanced entry of the same name, which holds the same bytes.
    EcCopy = 0,
    /// The Enhanced entry of the same name in place of a texture Kingdom Reborn only re-encoded.
    EcSubstitute = 1,
    /// The file's stored blob ([`stored_entry_path`]), decompressed.
    Stored = 2,
    /// The file's stored blob, exactly as the client stores it: for a payload its compression does not reproduce.
    Verbatim = 3,
}

impl KrDeltaSource {
    pub const ALL: [Self; 4] = [Self::EcCopy, Self::EcSubstitute, Self::Stored, Self::Verbatim];

    pub fn from_u8(value: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|source| *source as u8 == value)
    }

    /// Whether the entry is the Enhanced entry of its name.
    pub fn uses_enhanced(self) -> bool {
        matches!(self, Self::EcCopy | Self::EcSubstitute)
    }

    /// The entry's bytes in the stored blob: the decompressed bytes, the stored payload, or none.
    pub fn stored_length(self, payload: &UopPayload) -> u64 {
        match self {
            Self::Stored => u64::from(payload.decompressed_length),
            Self::Verbatim => u64::from(payload.stored_length),
            Self::EcCopy | Self::EcSubstitute => 0,
        }
    }
}

/// One entry of a Kingdom Reborn file with a payload, in file order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KrDeltaEntry {
    pub source: KrDeltaSource,
    /// XXH64 of the bytes the entry is rebuilt from: the decompressed bytes of the Kingdom Reborn entry
    /// ([`KrDeltaSource::EcCopy`], which are the Enhanced entry's too, and [`KrDeltaSource::Stored`]), of the
    /// Enhanced entry ([`KrDeltaSource::EcSubstitute`]), or the stored payload ([`KrDeltaSource::Verbatim`]).
    pub hash64: u64,
}

/// Bytes of one [`KrDeltaEntry`] in the manifest.
const ENTRY_SIZE: usize = size_of::<u8>() + size_of::<u64>();

/// One Kingdom Reborn file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrDeltaFile {
    /// The client file name, one of [`KR_DELTA_CLIENT_FILES`].
    pub name: String,
    pub file_length: u64,
    /// XXH64 of the whole file.
    pub file_hash64: u64,
    pub entries: Vec<KrDeltaEntry>,
}

impl KrDeltaFile {
    pub fn kind(&self) -> KrDeltaFileKind {
        client_file_kind(&self.name).expect("the manifest names only the client files")
    }
}

/// [`KR_DELTA_MANIFEST_ENTRY_PATH`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrDeltaManifest {
    /// The Kingdom Reborn client the files are from, for information.
    pub kr_client: String,
    /// The Enhanced Client the delta was made against, for information.
    pub ec_client: String,
    pub files: Vec<KrDeltaFile>,
}

/// The file's layout bytes.
pub fn layout_entry_path(file: &str) -> String {
    format!("files/{file}/layout.bin")
}

/// The entries the file stores itself.
pub fn stored_entry_path(file: &str) -> String {
    format!("files/{file}/stored.bin")
}

/// Appends `text` after its length in `length_bytes` little-endian bytes.
fn put_string(bytes: &mut Vec<u8>, text: &str, length_bytes: usize) -> eyre::Result<()> {
    let max = (1u64 << (u8::BITS as usize * length_bytes)) - 1;
    eyre::ensure!(text.len() as u64 <= max, "`{text}` is longer than {max} bytes");
    bytes.extend_from_slice(&(text.len() as u64).to_le_bytes()[..length_bytes]);
    bytes.extend_from_slice(text.as_bytes());
    Ok(())
}

impl KrDeltaManifest {
    pub fn encode(&self) -> eyre::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&KR_DELTA_MAGIC);
        bytes.extend_from_slice(&KR_DELTA_FORMAT_VERSION.to_le_bytes());
        put_string(&mut bytes, &self.kr_client, size_of::<u16>())?;
        put_string(&mut bytes, &self.ec_client, size_of::<u16>())?;
        bytes.extend_from_slice(&u32::try_from(self.files.len())?.to_le_bytes());
        for file in &self.files {
            put_string(&mut bytes, &file.name, size_of::<u8>())?;
            bytes.extend_from_slice(&file.file_length.to_le_bytes());
            bytes.extend_from_slice(&file.file_hash64.to_le_bytes());
            bytes.extend_from_slice(&u32::try_from(file.entries.len())?.to_le_bytes());
            for entry in &file.entries {
                bytes.push(entry.source as u8);
                bytes.extend_from_slice(&entry.hash64.to_le_bytes());
            }
        }
        Ok(bytes)
    }

    /// Reads the manifest, refusing another magic or version, a file other than [`KR_DELTA_CLIENT_FILES`] or one
    /// named twice, and an Enhanced entry in a file that is not textures.
    pub fn decode(bytes: &[u8]) -> eyre::Result<Self> {
        let path = KR_DELTA_MANIFEST_ENTRY_PATH;
        let mut reader = ByteReader { bytes, at: 0 };
        eyre::ensure!(
            reader.take(KR_DELTA_MAGIC.len())? == KR_DELTA_MAGIC,
            "{path}: no {} header; make the delta again with `udd-cli pack make-kr-delta`",
            String::from_utf8_lossy(&KR_DELTA_MAGIC)
        );
        ensure_manifest_version(path, reader.u32()?, KR_DELTA_FORMAT_VERSION)?;
        let kr_client_length = usize::from(reader.u16()?);
        let kr_client = reader.string(kr_client_length)?;
        let ec_client_length = usize::from(reader.u16()?);
        let ec_client = reader.string(ec_client_length)?;
        let file_count = reader.u32()?;
        let mut files: Vec<KrDeltaFile> = Vec::new();
        for _ in 0..file_count {
            let name_length = usize::from(reader.u8()?);
            let name = reader.string(name_length)?;
            let kind = client_file_kind(&name);
            eyre::ensure!(
                kind.is_some() && files.iter().all(|file| file.name != name),
                "{path}: unexpected or repeated client file {name}"
            );
            let file_length = reader.u64()?;
            let file_hash64 = reader.u64()?;
            let entry_count = reader.u32()? as usize;
            eyre::ensure!(entry_count.saturating_mul(ENTRY_SIZE) <= bytes.len() - reader.at, "{path}: {name} is truncated");
            let mut entries = Vec::with_capacity(entry_count);
            for index in 0..entry_count {
                let source = reader.u8()?;
                let source = KrDeltaSource::from_u8(source)
                    .ok_or_else(|| eyre::eyre!("{path}: {name} entry {index} has unknown source {source}"))?;
                eyre::ensure!(
                    !source.uses_enhanced() || kind == Some(KrDeltaFileKind::Textures),
                    "{path}: {name} entry {index} is taken from the Enhanced Client, which only textures are"
                );
                entries.push(KrDeltaEntry { source, hash64: reader.u64()? });
            }
            files.push(KrDeltaFile { name, file_length, file_hash64, entries });
        }
        eyre::ensure!(reader.at == bytes.len(), "{path}: {} trailing bytes", bytes.len() - reader.at);
        Ok(Self { kr_client, ec_client, files })
    }
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> ByteReader<'a> {
    fn take(&mut self, length: usize) -> eyre::Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(length)
            .filter(|&end| end <= self.bytes.len())
            .ok_or_else(|| eyre::eyre!("{KR_DELTA_MANIFEST_ENTRY_PATH} is truncated"))?;
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> eyre::Result<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("took N bytes"))
    }

    fn u8(&mut self) -> eyre::Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> eyre::Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> eyre::Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> eyre::Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn string(&mut self, length: usize) -> eyre::Result<String> {
        String::from_utf8(self.take(length)?.to_vec())
            .wrap_err_with(|| format!("{KR_DELTA_MANIFEST_ENTRY_PATH}: a name is not UTF-8"))
    }
}

// --- UOP layout ---------------------------------------------------------------------------------------------------

/// The fixed UOP header: magic, version, misc, `u64` first block address, block size, file count.
const UOP_HEADER_SIZE: u64 = 28;
/// Offset of the first block address in the header.
const UOP_FIRST_BLOCK_OFFSET: u64 = 12;
/// A block table's header: `u32` record count, `u64` next block address.
const UOP_BLOCK_HEADER_SIZE: u64 = 12;
/// Offset of the next block address in a block table's header.
const UOP_NEXT_BLOCK_OFFSET: u64 = 4;
/// A block table record: `u64` data block address, `u32` data block length, `u32` compressed size, `u32`
/// decompressed size, `u64` name hash, `u32` data block hash, `i16` compression.
const UOP_RECORD_SIZE: u64 = 34;
/// Offsets of the record fields read here.
const RECORD_DATA_BLOCK_LENGTH_OFFSET: u64 = 8;
const RECORD_COMPRESSED_SIZE_OFFSET: u64 = 12;
const RECORD_DECOMPRESSED_SIZE_OFFSET: u64 = 16;
const RECORD_NAME_HASH_OFFSET: u64 = 20;
const RECORD_COMPRESSION_OFFSET: u64 = 32;

/// An entry's payload in a UOP file, from its block table record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UopPayload {
    /// Where the payload starts: the record's data block address plus its data block length.
    pub address: u64,
    pub stored_length: u32,
    pub decompressed_length: u32,
    pub name_hash: u64,
    /// The UOP compression flag.
    pub compression: i16,
}

fn le<const N: usize>(bytes: &[u8], at: u64) -> Option<[u8; N]> {
    let at = usize::try_from(at).ok()?;
    bytes.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn block_table_end(address: u64, count: u64) -> u64 {
    address + UOP_BLOCK_HEADER_SIZE + count * UOP_RECORD_SIZE
}

/// The block table at `address`: its payloads (records with stored payload bytes) and the next block address.
/// `None` while `bytes` ends before the table does.
fn read_block_table(bytes: &[u8], address: u64) -> Option<(Vec<UopPayload>, u64)> {
    let count = u64::from(u32::from_le_bytes(le(bytes, address)?));
    let next = u64::from_le_bytes(le(bytes, address + UOP_NEXT_BLOCK_OFFSET)?);
    if (bytes.len() as u64) < block_table_end(address, count) {
        return None;
    }
    let field = |record: u64, offset: u64| -> [u8; 8] {
        let mut value = [0u8; 8];
        let at = (record + offset) as usize;
        let length = (bytes.len() - at).min(value.len());
        value[..length].copy_from_slice(&bytes[at..at + length]);
        value
    };
    let u32_at = |record, offset| u32::from_le_bytes(field(record, offset)[..4].try_into().expect("four bytes"));
    let payloads = (0..count)
        .map(|index| address + UOP_BLOCK_HEADER_SIZE + index * UOP_RECORD_SIZE)
        .filter(|&record| u32_at(record, RECORD_COMPRESSED_SIZE_OFFSET) != 0)
        .map(|record| UopPayload {
            address: u64::from_le_bytes(field(record, 0)) + u64::from(u32_at(record, RECORD_DATA_BLOCK_LENGTH_OFFSET)),
            stored_length: u32_at(record, RECORD_COMPRESSED_SIZE_OFFSET),
            decompressed_length: u32_at(record, RECORD_DECOMPRESSED_SIZE_OFFSET),
            name_hash: u64::from_le_bytes(field(record, RECORD_NAME_HASH_OFFSET)),
            compression: i16::from_le_bytes(field(record, RECORD_COMPRESSION_OFFSET)[..2].try_into().expect("two bytes")),
        })
        .collect();
    Some((payloads, next))
}

/// The payloads of a whole UOP file in file order, refusing overlapping ones and ones past its end.
pub fn uop_payloads(file: &[u8]) -> eyre::Result<Vec<UopPayload>> {
    let mut payloads = Vec::new();
    let mut next = u64::from_le_bytes(le(file, UOP_FIRST_BLOCK_OFFSET).ok_or_else(|| eyre::eyre!("no UOP header"))?);
    let mut tables = 0usize;
    while next != 0 {
        let (table, following) =
            read_block_table(file, next).ok_or_else(|| eyre::eyre!("the block table at {next} ends past the file"))?;
        payloads.extend(table);
        next = following;
        tables += 1;
        eyre::ensure!(tables <= file.len(), "the block tables loop");
    }
    payloads.sort_unstable_by_key(|payload| payload.address);
    let mut end = UOP_HEADER_SIZE;
    for payload in &payloads {
        eyre::ensure!(payload.address >= end, "the payload at {} overlaps the one before it or the header", payload.address);
        end = payload.address + u64::from(payload.stored_length);
    }
    eyre::ensure!(end <= file.len() as u64, "a payload ends past the end of the file");
    Ok(payloads)
}

/// The layout bytes of `file`: everything but its `payloads` (from [`uop_payloads`]).
pub fn uop_layout(file: &[u8], payloads: &[UopPayload]) -> Vec<u8> {
    let mut layout = Vec::new();
    let mut at = 0usize;
    for payload in payloads {
        layout.extend_from_slice(&file[at..payload.address as usize]);
        at = payload.address as usize + payload.stored_length as usize;
    }
    layout.extend_from_slice(&file[at..]);
    layout
}

/// Rebuilds a UOP file from its layout bytes, asking `payload` for each payload (its index in file order and its
/// record) as the walk reaches it. The block tables are read from the layout as it is copied, so each must come
/// before the payloads it lists, as in the client files. Fails when a payload has another length than its record
/// gives or the layout runs out.
pub fn rebuild_uop<'a>(
    layout: &[u8],
    mut payload: impl FnMut(usize, &UopPayload) -> eyre::Result<Cow<'a, [u8]>>,
) -> eyre::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(layout.len());
    let mut layout_at = 0usize;
    let mut copy_to = |out: &mut Vec<u8>, target: u64| -> eyre::Result<()> {
        let count = usize::try_from(target.saturating_sub(out.len() as u64))?;
        let bytes = layout
            .get(layout_at..layout_at.saturating_add(count))
            .ok_or_else(|| eyre::eyre!("the layout ends at {} bytes, before the file does", layout.len()))?;
        out.extend_from_slice(bytes);
        layout_at += count;
        Ok(())
    };
    copy_to(&mut out, UOP_HEADER_SIZE)?;
    let first_table = u64::from_le_bytes(le(&out, UOP_FIRST_BLOCK_OFFSET).expect("the header is copied"));
    let mut pending_table = (first_table != 0).then_some(first_table);
    let mut known: BTreeMap<u64, UopPayload> = BTreeMap::new();
    let mut index = 0usize;
    let mut tables = 0usize;
    loop {
        while let Some(table) = pending_table {
            let Some((payloads, next)) = read_block_table(&out, table) else { break };
            for found in payloads {
                eyre::ensure!(found.address >= out.len() as u64, "the block table at {table} lists a payload before it");
                eyre::ensure!(known.insert(found.address, found).is_none(), "two entries share the payload at {}", found.address);
            }
            pending_table = (next != 0).then_some(next);
            tables += 1;
            eyre::ensure!(tables <= layout.len(), "the block tables loop");
        }
        // The end of the next block table, or of its header while its record count is not copied yet.
        let table_end = pending_table.map(|table| match le::<4>(&out, table) {
            Some(count) => block_table_end(table, u64::from(u32::from_le_bytes(count))),
            None => table + UOP_BLOCK_HEADER_SIZE,
        });
        let next_payload = known
            .first_key_value()
            .map(|(_, &found)| found)
            .filter(|found| table_end.is_none_or(|end| found.address < end));
        if let Some(found) = next_payload {
            known.remove(&found.address);
            eyre::ensure!(found.address >= out.len() as u64, "the payload at {} overlaps the one before it", found.address);
            copy_to(&mut out, found.address)?;
            let bytes = payload(index, &found)?;
            eyre::ensure!(
                bytes.len() == found.stored_length as usize,
                "entry {:016x}: {} payload bytes, its record gives {}",
                found.name_hash,
                bytes.len(),
                found.stored_length
            );
            out.extend_from_slice(&bytes);
            index += 1;
        } else if let Some(end) = table_end {
            copy_to(&mut out, end)?;
        } else {
            eyre::ensure!(known.is_empty(), "a payload is listed past the end of the layout");
            out.extend_from_slice(&layout[layout_at..]);
            return Ok(out);
        }
    }
}

// --- Contents ---------------------------------------------------------------------------------------------------

/// A file's manifest record with its layout and stored blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrDeltaFileContents {
    pub file: KrDeltaFile,
    pub layout: Vec<u8>,
    pub stored: Vec<u8>,
}

/// Everything a delta holds, in memory: what `udd_conv::kr_delta` makes, writes and rebuilds from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrDeltaContents {
    pub kr_client: String,
    pub ec_client: String,
    pub files: Vec<KrDeltaFileContents>,
}

impl KrDeltaContents {
    pub fn manifest(&self) -> KrDeltaManifest {
        KrDeltaManifest {
            kr_client: self.kr_client.clone(),
            ec_client: self.ec_client.clone(),
            files: self.files.iter().map(|contents| contents.file.clone()).collect(),
        }
    }

    /// The file `name`.
    pub fn file(&self, name: &str) -> Option<&KrDeltaFileContents> {
        self.files.iter().find(|contents| contents.file.name == name)
    }

    /// Checks what can be checked without the Enhanced Client: every client file is there, each layout walks to as
    /// many payloads as its file has entries and to the file's length, and the stored blob holds exactly the stored
    /// entries.
    pub fn check(&self) -> eyre::Result<()> {
        for (name, _) in KR_DELTA_CLIENT_FILES {
            eyre::ensure!(self.file(name).is_some(), "the delta lacks {name}");
        }
        for contents in &self.files {
            let file = &contents.file;
            let mut stored_length = 0u64;
            let mut payloads = 0usize;
            let rebuilt = rebuild_uop(&contents.layout, |index, payload| {
                let entry = file.entries.get(index).ok_or_else(|| eyre::eyre!("more payloads than entries"))?;
                stored_length += entry.source.stored_length(payload);
                payloads += 1;
                Ok(Cow::Owned(vec![0u8; payload.stored_length as usize]))
            })
            .wrap_err_with(|| format!("walk the layout of {}", file.name))?;
            eyre::ensure!(payloads == file.entries.len(), "{}: {payloads} payloads, {} entries", file.name, file.entries.len());
            eyre::ensure!(
                stored_length == contents.stored.len() as u64,
                "{}: the stored entries take {stored_length} bytes, {} holds {}",
                file.name,
                stored_entry_path(&file.name),
                contents.stored.len()
            );
            eyre::ensure!(
                rebuilt.len() as u64 == file.file_length,
                "{}: the layout rebuilds {} bytes, the file has {}",
                file.name,
                rebuilt.len(),
                file.file_length
            );
        }
        Ok(())
    }
}

/// Reader for `kr_delta.uddp`.
pub struct KrDeltaPackage {
    contents: KrDeltaContents,
}

impl KrDeltaPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load(path.as_ref()).wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(&package)
    }

    /// Reads the manifest and the blobs, and [`KrDeltaContents::check`]s them.
    pub fn from_uddp_package(package: &UddpReader) -> eyre::Result<Self> {
        let manifest = KrDeltaManifest::decode(&read_path_entry(package, KR_DELTA_MANIFEST_ENTRY_PATH)?)?;
        let files = manifest
            .files
            .into_iter()
            .map(|file| {
                Ok(KrDeltaFileContents {
                    layout: read_path_entry(package, &layout_entry_path(&file.name))?,
                    stored: read_path_entry(package, &stored_entry_path(&file.name))?,
                    file,
                })
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        let contents = KrDeltaContents { kr_client: manifest.kr_client, ec_client: manifest.ec_client, files };
        contents.check()?;
        Ok(Self { contents })
    }

    pub fn contents(&self) -> &KrDeltaContents {
        &self.contents
    }

    pub fn into_contents(self) -> KrDeltaContents {
        self.contents
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes of the data block header before each payload of [`uop_file`].
    const TEST_DATA_BLOCK_HEADER: usize = 12;
    /// Filler of the data block headers.
    const TEST_HEADER_FILLER: u8 = 0xAB;

    /// A UOP file like the client's: the header, one block table, then each `(name hash, compression, stored bytes,
    /// decompressed length)` entry after a data block header. An entry without stored bytes has no payload.
    fn uop_file(entries: &[(u64, i16, &[u8], u32)]) -> Vec<u8> {
        let table = UOP_HEADER_SIZE as usize;
        let mut file = vec![0u8; block_table_end(UOP_HEADER_SIZE, entries.len() as u64) as usize];
        file[..4].copy_from_slice(b"MYP\0");
        file[UOP_FIRST_BLOCK_OFFSET as usize..][..8].copy_from_slice(&(table as u64).to_le_bytes());
        file[table..table + 4].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        for (index, &(name_hash, compression, stored, decompressed_length)) in entries.iter().enumerate() {
            let address = file.len() as u64;
            file.extend(std::iter::repeat_n(TEST_HEADER_FILLER, TEST_DATA_BLOCK_HEADER));
            file.extend_from_slice(stored);
            let record = table + (UOP_BLOCK_HEADER_SIZE + index as u64 * UOP_RECORD_SIZE) as usize;
            let mut put = |offset: u64, bytes: &[u8]| file[record + offset as usize..][..bytes.len()].copy_from_slice(bytes);
            put(0, &address.to_le_bytes());
            put(RECORD_DATA_BLOCK_LENGTH_OFFSET, &(TEST_DATA_BLOCK_HEADER as u32).to_le_bytes());
            put(RECORD_COMPRESSED_SIZE_OFFSET, &(stored.len() as u32).to_le_bytes());
            put(RECORD_DECOMPRESSED_SIZE_OFFSET, &decompressed_length.to_le_bytes());
            put(RECORD_NAME_HASH_OFFSET, &name_hash.to_le_bytes());
            put(RECORD_COMPRESSION_OFFSET, &compression.to_le_bytes());
        }
        file
    }

    #[test]
    fn a_uop_file_rebuilds_from_its_layout_and_payloads() {
        let stored: [&[u8]; 2] = [b"first", b"third payload"];
        let file = uop_file(&[(1, 0, stored[0], 5), (2, 0, b"", 0), (3, 0, stored[1], 13)]);
        let found = uop_payloads(&file).unwrap();
        assert_eq!(found.iter().map(|payload| payload.name_hash).collect::<Vec<_>>(), [1, 3], "the empty entry has no payload");
        let layout = uop_layout(&file, &found);
        assert_eq!(layout.len(), file.len() - stored.iter().map(|bytes| bytes.len()).sum::<usize>());
        let rebuilt = rebuild_uop(&layout, |index, payload| {
            assert_eq!(payload, &found[index]);
            Ok(Cow::Borrowed(stored[index]))
        })
        .unwrap();
        assert_eq!(rebuilt, file);

        let error = rebuild_uop(&layout, |_, _| Ok(Cow::Borrowed(b"x".as_slice()))).unwrap_err().to_string();
        assert!(error.contains("payload bytes"), "{error}");
        let error = rebuild_uop(&layout[..layout.len() / 2], |index, _| Ok(Cow::Borrowed(stored[index])))
            .unwrap_err()
            .to_string();
        assert!(error.contains("layout ends"), "{error}");
    }

    fn manifest() -> KrDeltaManifest {
        KrDeltaManifest {
            kr_client: "UOKR.exe 2.58.0.6".into(),
            ec_client: "UOSA.exe 4.0.102.3".into(),
            files: vec![KrDeltaFile {
                name: "Texture.uop".into(),
                file_length: 55,
                file_hash64: 77,
                entries: vec![
                    KrDeltaEntry { source: KrDeltaSource::EcSubstitute, hash64: 5 },
                    KrDeltaEntry { source: KrDeltaSource::Stored, hash64: 3 },
                ],
            }],
        }
    }

    #[test]
    fn the_manifest_reads_back_what_it_wrote() {
        let manifest = manifest();
        assert_eq!(KrDeltaManifest::decode(&manifest.encode().unwrap()).unwrap(), manifest);
    }

    #[test]
    fn the_manifest_refuses_another_version_and_enhanced_entries_outside_textures() {
        let mut bytes = manifest().encode().unwrap();
        bytes[KR_DELTA_MAGIC.len()] = 2;
        let error = KrDeltaManifest::decode(&bytes).unwrap_err().to_string();
        assert!(error.contains("version 2"), "{error}");

        let mut data = manifest();
        data.files[0].name = "tileart.uop".into();
        let error = KrDeltaManifest::decode(&data.encode().unwrap()).unwrap_err().to_string();
        assert!(error.contains("only textures"), "{error}");
    }
}
