//! Patch package builder and applier.
//!
//! Patch containers are intentionally simple: they are ordinary dense-id UDDP
//! packages whose first file contains a manifest and whose remaining files hold
//! complete replacement payloads. There is no binary diffing layer. That keeps
//! debugging straightforward and avoids fragile patch logic tied to the exact
//! byte layout of compressed payloads.

use std::collections::BTreeMap;

use xxhash_rust::xxh64::xxh64;

use super::builder::codec_to_compression_flag;
use super::support::{patch_header_magic, patch_header_package_hash64};
use super::*;

#[derive(Debug, Clone, Copy)]
enum PatchKey {
    Path(u64),
    Id(u32),
}

#[derive(Debug, Clone)]
struct PendingPatchReplacement {
    key: PatchKey,
    old_file_hash64: u64,
    new_file_hash64: u64,
    payload: Vec<u8>,
    data_type: u8,
}

#[derive(Debug, Clone)]
struct ParsedPatchManifest {
    header: PatchManifestHeader,
    path_records: Vec<PatchPathRecord>,
    id_records: Vec<PatchIdRecord>,
}

impl ParsedPatchManifest {
    fn header_mode(&self) -> Result<LookupMode, PatchError> {
        LookupMode::from_u8(self.header.target_lookup_mode as u8).map_err(PatchError::Format)
    }

    fn expected_old_hash(&self, key: FileKey) -> Option<u64> {
        match key {
            FileKey::PathHash(hash) => self
                .path_records
                .iter()
                .find(|record| record.path_hash64 == hash)
                .map(|record| record.old_file_hash64),
            FileKey::Id(id) => self
                .id_records
                .iter()
                .find(|record| record.id == id)
                .map(|record| record.old_file_hash64),
        }
    }
}

#[derive(Debug, Clone)]
struct ReplacementData {
    raw: Vec<u8>,
}

/// Builder for `.uddpi` patch containers.
///
/// The patch builder records only logical replacements. It delegates the actual
/// payload packing to `UddpBuilder`, which means patches inherit the same
/// container machinery, compression policy, and canonical hashing rules as a
/// normal runtime package.
pub struct UddpiBuilder {
    target_lookup_mode: LookupMode,
    old_package_hash64: u64,
    new_package_hash64: u64,
    replacements: Vec<PendingPatchReplacement>,
}

impl UddpiBuilder {
    /// Create a patch builder for packages that use one specific lookup mode.
    pub fn new(target_lookup_mode: LookupMode, old_package_hash64: u64, new_package_hash64: u64) -> Self {
        Self {
            target_lookup_mode,
            old_package_hash64,
            new_package_hash64,
            replacements: Vec::new(),
        }
    }

    /// Record a replacement addressed by normalized path hash.
    pub fn add_path_replacement(
        &mut self,
        path_hash64: u64,
        old_file_hash64: u64,
        new_file_hash64: u64,
        payload: Vec<u8>,
        data_type: u8,
    ) {
        self.replacements.push(PendingPatchReplacement {
            key: PatchKey::Path(path_hash64),
            old_file_hash64,
            new_file_hash64,
            payload,
            data_type,
        });
    }

    /// Record a replacement addressed by logical id.
    pub fn add_id_replacement(
        &mut self,
        id: u32,
        old_file_hash64: u64,
        new_file_hash64: u64,
        payload: Vec<u8>,
        data_type: u8,
    ) {
        self.replacements.push(PendingPatchReplacement {
            key: PatchKey::Id(id),
            old_file_hash64,
            new_file_hash64,
            payload,
            data_type,
        });
    }

    /// Build the final `.uddpi` package bytes.
    pub fn build(&self) -> Result<Vec<u8>, BuildError> {
        // A patch package is itself a dense-id UDDP image:
        // - file 0 contains the manifest
        // - files 1..N contain the replacement payloads referenced by the manifest
        let manifest = self.encode_manifest()?;

        let mut builder = UddpBuilder::new(LookupMode::DenseId);
        builder.add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            apply_planar: false,
            virtual_path: None,
            path_hash64: None,
            id: Some(0),
            data: &manifest,
        })?;

        for (index, replacement) in self.replacements.iter().enumerate() {
            builder.add_file(AddFileRequest {
                data_type: replacement.data_type,
                compression: CompressionFlag::Auto,
                apply_planar: false,
                virtual_path: None,
                path_hash64: None,
                id: Some((index as u32) + 1),
                data: &replacement.payload,
            })?;
        }

        let mut bytes = builder.build()?;
        patch_header_magic(&mut bytes, UDPI_MAGIC)?;
        let hash = canonical_package_hash64(&bytes);
        patch_header_package_hash64(&mut bytes, hash)?;
        Ok(bytes)
    }

    fn encode_manifest(&self) -> Result<Vec<u8>, BuildError> {
        let mut out = Vec::new();
        let header = PatchManifestHeader {
            magic: PMAN_MAGIC,
            version_major: 1,
            version_minor: 0,
            old_package_hash64: self.old_package_hash64,
            new_package_hash64: self.new_package_hash64,
            record_count: self.replacements.len() as u32,
            target_lookup_mode: self.target_lookup_mode as u32,
        };
        header.write_to(&mut out);

        match self.target_lookup_mode {
            LookupMode::VirtualPathHash => {
                for (index, replacement) in self.replacements.iter().enumerate() {
                    let PatchKey::Path(path_hash64) = replacement.key else {
                        return Err(BuildError::PatchKeyModeMismatch);
                    };
                    PatchPathRecord {
                        path_hash64,
                        old_file_hash64: replacement.old_file_hash64,
                        new_file_hash64: replacement.new_file_hash64,
                        payload_file_id: (index as u32) + 1,
                        new_raw_size: replacement.payload.len() as u32,
                    }
                    .write_to(&mut out);
                }
            }
            LookupMode::DenseId | LookupMode::SparseId => {
                for (index, replacement) in self.replacements.iter().enumerate() {
                    let PatchKey::Id(id) = replacement.key else {
                        return Err(BuildError::PatchKeyModeMismatch);
                    };
                    PatchIdRecord {
                        id,
                        payload_file_id: (index as u32) + 1,
                        old_file_hash64: replacement.old_file_hash64,
                        new_file_hash64: replacement.new_file_hash64,
                        new_raw_size: replacement.payload.len() as u32,
                        reserved0: 0,
                    }
                    .write_to(&mut out);
                }
            }
        }

        Ok(out)
    }
}

/// Apply a `.uddpi` patch to a base `.uddp` image.
pub struct UddpiApplier;

impl UddpiApplier {
    /// Rebuild a patched runtime package from a base package and a patch package.
    ///
    /// The patch applier verifies:
    /// - that the patch container itself is a dense-id patch package
    /// - that the base package hash matches the manifest expectation
    /// - that every replacement payload matches the manifest-declared file hash
    /// - that the rebuilt package hash matches the manifest-declared result hash
    pub fn apply_patch(base_uddp: &[u8], patch_uddpi: &[u8]) -> Result<Vec<u8>, PatchError> {
        let base_reader = UddpReader::open(base_uddp.to_vec()).map_err(PatchError::Format)?;
        let patch_reader = UddpReader::open(patch_uddpi.to_vec()).map_err(PatchError::Format)?;

        if patch_reader.lookup_mode() != LookupMode::DenseId {
            return Err(PatchError::InvalidPatchContainer);
        }

        let manifest_bytes = patch_reader.read_file_by_dense_id(0).map_err(PatchError::Format)?;
        let manifest = parse_patch_manifest(&manifest_bytes)?;

        let old_hash = canonical_package_hash64(base_uddp);
        if old_hash != manifest.header.old_package_hash64 {
            return Err(PatchError::WrongBasePackageHash {
                expected: manifest.header.old_package_hash64,
                got: old_hash,
            });
        }

        let mut replacements = BTreeMap::<FileKey, ReplacementData>::new();
        match manifest.header_mode()? {
            LookupMode::VirtualPathHash => {
                for record in &manifest.path_records {
                    let payload = patch_reader
                        .read_file_by_dense_id(record.payload_file_id)
                        .map_err(PatchError::Format)?;
                    let new_hash = xxh64(&payload, 0);
                    if new_hash != record.new_file_hash64 {
                        return Err(PatchError::ReplacementHashMismatch);
                    }
                    replacements.insert(record_key_path(record.path_hash64), ReplacementData { raw: payload });
                }
            }
            LookupMode::DenseId | LookupMode::SparseId => {
                for record in &manifest.id_records {
                    let payload = patch_reader
                        .read_file_by_dense_id(record.payload_file_id)
                        .map_err(PatchError::Format)?;
                    let new_hash = xxh64(&payload, 0);
                    if new_hash != record.new_file_hash64 {
                        return Err(PatchError::ReplacementHashMismatch);
                    }
                    replacements.insert(FileKey::Id(record.id), ReplacementData { raw: payload });
                }
            }
        }

        let mut builder = UddpBuilder::new(base_reader.lookup_mode());
        builder.set_version(base_reader.header().version_major, base_reader.header().version_minor);

        // Canonical package hashes depend on physical blob order, not just logical key order.
        // Rebuild in original payload-offset order so unchanged files stay byte-for-byte stable.
        let mut records = base_reader.records();
        records.sort_by_key(|record| unpack_offset40(record.locator.pos64));

        for record in records {
            let old_raw = match record.key {
                FileKey::PathHash(hash) => base_reader.read_file_by_path_hash(hash).map_err(PatchError::Format)?,
                FileKey::Id(id) => match base_reader.lookup_mode() {
                    LookupMode::DenseId => base_reader.read_file_by_dense_id(id).map_err(PatchError::Format)?,
                    LookupMode::SparseId => base_reader.read_file_by_sparse_id(id).map_err(PatchError::Format)?,
                    LookupMode::VirtualPathHash => return Err(PatchError::InvalidPatchManifest),
                },
            };

            let old_hash64 = xxh64(&old_raw, 0);
            let data_type = unpack_type(record.locator.meta32);
            let original_flag = codec_to_compression_flag(unpack_codec(record.locator.meta32));

            let payload = replacements.get(&record.key).map(|replacement| replacement.raw.as_slice()).unwrap_or(&old_raw);

            match record.key {
                FileKey::PathHash(path_hash64) => builder.add_file(AddFileRequest {
                    data_type,
                    compression: original_flag,
                    apply_planar: unpack_planar(record.locator.meta32),
                    virtual_path: None,
                    path_hash64: Some(path_hash64),
                    id: None,
                    data: payload,
                }),
                FileKey::Id(id) => builder.add_file(AddFileRequest {
                    data_type,
                    compression: original_flag,
                    apply_planar: unpack_planar(record.locator.meta32),
                    virtual_path: None,
                    path_hash64: None,
                    id: Some(id),
                    data: payload,
                }),
            }
            .map_err(PatchError::Build)?;

            if let Some(expected_old) = manifest.expected_old_hash(record.key) {
                if expected_old != old_hash64 {
                    return Err(PatchError::OldFileHashMismatch);
                }
            }
        }

        let rebuilt = builder.build().map_err(PatchError::Build)?;
        let new_hash = canonical_package_hash64(&rebuilt);
        if new_hash != manifest.header.new_package_hash64 {
            return Err(PatchError::WrongResultPackageHash {
                expected: manifest.header.new_package_hash64,
                got: new_hash,
            });
        }

        Ok(rebuilt)
    }
}

fn record_key_path(path_hash64: u64) -> FileKey {
    FileKey::PathHash(path_hash64)
}

/// Parse the patch manifest stored in dense-id file 0.
fn parse_patch_manifest(bytes: &[u8]) -> Result<ParsedPatchManifest, PatchError> {
    let mut cur = support::Cursor::new(bytes);
    let header = PatchManifestHeader::read_from(&mut cur).map_err(PatchError::Format)?;
    if header.magic != PMAN_MAGIC {
        return Err(PatchError::InvalidPatchManifest);
    }

    let mode = LookupMode::from_u8(header.target_lookup_mode as u8).map_err(PatchError::Format)?;
    let mut path_records = Vec::new();
    let mut id_records = Vec::new();

    match mode {
        LookupMode::VirtualPathHash => {
            for _ in 0..header.record_count {
                path_records.push(PatchPathRecord::read_from(&mut cur).map_err(PatchError::Format)?);
            }
        }
        LookupMode::DenseId | LookupMode::SparseId => {
            for _ in 0..header.record_count {
                id_records.push(PatchIdRecord::read_from(&mut cur).map_err(PatchError::Format)?);
            }
        }
    }

    Ok(ParsedPatchManifest {
        header,
        path_records,
        id_records,
    })
}
