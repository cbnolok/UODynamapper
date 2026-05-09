//! Package builder implementation.
//!
//! This module owns the write path for runtime packages and the compression
//! policy used during package construction. The policy code is intentionally
//! colocated with the builder so the reasoning about when a file is stored raw,
//! compressed with plain Zstd, or compressed with a trained dictionary remains
//! next to the code that serializes the final package image.

use std::collections::{HashMap, HashSet};

use rayon::prelude::*;

use super::support::{patch_header_package_hash64, zstd_compress, zstd_compress_with_dict, jxl_compress};
use super::*;

const DICT_TRAIN_MIN_SAMPLE_BYTES: usize = 128;
const DICT_TRAIN_AUTO_MAX_SAMPLE_BYTES: usize = 48 * 1024;
const DICT_TRAIN_MAX_SAMPLE_COUNT: usize = 1024;
const DICT_TRAIN_MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingKey {
    PathHash(u64),
    Id(u32),
}

#[derive(Debug, Clone)]
struct PendingFile {
    key: PendingKey,
    data_type: u8,
    compression: CompressionFlag,
    width: u32,
    height: u32,
    raw_data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct BuiltDictionary {
    data_type: u8,
    codec: Codec,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct BuiltFile {
    key: PendingKey,
    data_type: u8,
    codec: Codec,
    raw_size: u32,
    encoded_payload: Vec<u8>,
}

#[derive(Debug, Clone)]
struct BuildPlan {
    dictionaries: Vec<BuiltDictionary>,
    files: Vec<BuiltFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProgressPhase {
    TrainingDictionaries,
    CompressingFiles,
    Assembling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildProgress {
    pub phase: BuildProgressPhase,
    pub completed: usize,
    pub total: usize,
}

/// Builder for runtime `.uddp` containers.
///
/// The builder collects logical files first and decides on physical encoding
/// only during `build()`. This makes it possible to:
/// - derive trained dictionaries from the full set of same-type samples
/// - validate key uniqueness only once, with full visibility over all files
/// - emit offsets deterministically in a single pass after compression choices
///   have been finalized
pub struct UddpBuilder {
    lookup_mode: LookupMode,
    version_major: u16,
    version_minor: u16,
    files: Vec<PendingFile>,
    next_dense_id: u32,
}

impl UddpBuilder {
    /// Create a builder for one logical lookup mode.
    pub fn new(lookup_mode: LookupMode) -> Self {
        Self {
            lookup_mode,
            version_major: 1,
            version_minor: 0,
            files: Vec::new(),
            next_dense_id: 0,
        }
    }

    /// Override the version written into the output package header.
    pub fn set_version(&mut self, major: u16, minor: u16) {
        self.version_major = major;
        self.version_minor = minor;
    }

    /// Add one logical file to the package.
    ///
    /// The builder accepts either raw virtual paths or already-computed path
    /// hashes for path-addressed packages. The latter is important when tooling
    /// is rebuilding packages from a package image that never stored original
    /// path strings.
    pub fn add_file(&mut self, req: AddFileRequest<'_>) -> Result<(), BuildError> {
        if req.data_type as usize >= MAX_TYPES {
            return Err(BuildError::InvalidType(req.data_type));
        }

        if req.data.len() > MAX_RAW_FILE_SIZE as usize {
            return Err(BuildError::FileTooLarge(req.data.len() as u64));
        }

        let key = match self.lookup_mode {
            LookupMode::VirtualPathHash => {
                let hash = if let Some(hash) = req.path_hash64 {
                    hash
                } else if let Some(path) = req.virtual_path {
                    xxh64_virtual_path(path)
                } else {
                    return Err(BuildError::MissingPathForPathMode);
                };
                PendingKey::PathHash(hash)
            }
            LookupMode::DenseId => {
                let id = req.id.unwrap_or(self.next_dense_id);
                self.next_dense_id = id.saturating_add(1);
                PendingKey::Id(id)
            }
            LookupMode::SparseId => PendingKey::Id(req.id.ok_or(BuildError::MissingIdForSparseMode)?),
        };

        self.files.push(PendingFile {
            key,
            data_type: req.data_type,
            compression: req.compression,
            width: req.width,
            height: req.height,
            raw_data: req.data.to_vec(),
        });

        Ok(())
    }

    /// Build the final package image.
    pub fn build(&mut self) -> Result<Vec<u8>, BuildError> {
        self.build_with_progress(|_| {})
    }

    /// Build the final package image while reporting coarse progress for the
    /// expensive compression and assembly passes.
    pub fn build_with_progress<F>(&mut self, mut progress: F) -> Result<Vec<u8>, BuildError>
    where
        F: FnMut(BuildProgress),
    {
        let plan = self.plan_build_with_progress(&mut progress)?;
        self.emit_package_with_progress(plan, &mut progress)
    }

    fn plan_build(&self) -> Result<BuildPlan, BuildError> {
        self.plan_build_with_progress(|_| {})
    }

    fn plan_build_with_progress<F>(&self, mut progress: F) -> Result<BuildPlan, BuildError>
    where
        F: FnMut(BuildProgress),
    {
        let mut samples_by_type: HashMap<u8, Vec<&[u8]>> = HashMap::new();
        for file in &self.files {
            samples_by_type.entry(file.data_type).or_default().push(&file.raw_data);
        }

        let training_samples_by_type = samples_by_type
            .iter()
            .filter_map(|(&data_type, samples)| {
                let training_samples = select_dict_training_samples(samples);
                should_train_dict(&training_samples)
                    .then_some((data_type, training_samples))
            })
            .collect::<Vec<_>>();

        let mut dicts_by_type: HashMap<u8, Vec<u8>> = HashMap::new();
        if !training_samples_by_type.is_empty() {
            let mut completed = 0usize;
            progress(BuildProgress {
                phase: BuildProgressPhase::TrainingDictionaries,
                completed,
                total: training_samples_by_type.len(),
            });

            for (data_type, samples) in &training_samples_by_type {
                let dict_size = choose_dict_size(samples);
                let dict = train_zstd_dict(samples, dict_size)?;
                if !dict.is_empty() {
                    dicts_by_type.insert(*data_type, dict);
                }
                completed += 1;
                progress(BuildProgress {
                    phase: BuildProgressPhase::TrainingDictionaries,
                    completed,
                    total: training_samples_by_type.len(),
                });
            }
        }

        let mut completed = 0usize;
        progress(BuildProgress {
            phase: BuildProgressPhase::CompressingFiles,
            completed,
            total: self.files.len(),
        });

        let compressed_files = self
            .files
            .par_iter()
            .map(|file| -> Result<BuiltFile, BuildError> {
                let data_to_compress = &file.raw_data;

                let (codec, encoded_payload) = match file.compression {
                    CompressionFlag::None => (Codec::None, data_to_compress.to_vec()),
                    CompressionFlag::ZstdNoDict => {
                        let encoded = zstd_compress(data_to_compress).map_err(BuildError::Io)?;
                        (Codec::ZstdNoDict, encoded)
                    }
                    CompressionFlag::ZstdDict => {
                        let dict = dicts_by_type
                            .get(&file.data_type)
                            .ok_or(BuildError::MissingDictionaryForType(file.data_type))?;
                        let encoded =
                            zstd_compress_with_dict(data_to_compress, dict).map_err(BuildError::Io)?;
                        (Codec::ZstdTypeDict, encoded)
                    }
                    CompressionFlag::JpegXl => {
                        let encoded = jxl_compress(data_to_compress, file.width, file.height)
                            .map_err(BuildError::CodecError)?;
                        (Codec::JpegXl, encoded)
                    }
                    CompressionFlag::Auto => {
                        choose_auto_compression(file, dicts_by_type.get(&file.data_type))?
                    }
                };

                // The packed locator stores only a non-negative `raw_size - stored_size`
                // delta. If compression grows the payload, the file must fall back to raw
                // storage even when the caller requested compression explicitly.
                let (codec, encoded_payload): (Codec, Vec<u8>) =
                    if codec != Codec::None && encoded_payload.len() < file.raw_data.len() {
                        (codec, encoded_payload)
                    } else {
                        (Codec::None, file.raw_data.clone())
                    };

                Ok(BuiltFile {
                    key: file.key,
                    data_type: file.data_type,
                    codec,
                    raw_size: file.raw_data.len() as u32,
                    encoded_payload,
                })
            })
            .collect::<Vec<_>>();

        let mut files = Vec::with_capacity(compressed_files.len());
        for built_file in compressed_files {
            files.push(built_file?);
            completed += 1;
            progress(BuildProgress {
                phase: BuildProgressPhase::CompressingFiles,
                completed,
                total: self.files.len(),
            });
        }

        let mut dictionaries = Vec::new();
        for (data_type, bytes) in dicts_by_type {
            dictionaries.push(BuiltDictionary {
                data_type,
                codec: Codec::ZstdTypeDict,
                bytes,
            });
        }
        dictionaries.sort_by_key(|dict| dict.data_type);

        Ok(BuildPlan { dictionaries, files })
    }

    fn emit_package(&self, plan: BuildPlan) -> Result<Vec<u8>, BuildError> {
        self.emit_package_with_progress(plan, |_| {})
    }

    fn emit_package_with_progress<F>(&self, plan: BuildPlan, mut progress: F) -> Result<Vec<u8>, BuildError>
    where
        F: FnMut(BuildProgress),
    {
        validate_keys(self.lookup_mode, &plan.files)?;

        let total_steps = 2 + plan.dictionaries.len() + plan.files.len();
        let mut completed = 0usize;
        progress(BuildProgress {
            phase: BuildProgressPhase::Assembling,
            completed,
            total: total_steps,
        });

        let dict_table_offset = UddpHeader::SERIALIZED_SIZE as u64;
        let dict_table_size = (plan.dictionaries.len() * UddpDictRef::SERIALIZED_SIZE) as u64;

        let index_offset = dict_table_offset + dict_table_size;
        let index_size = match self.lookup_mode {
            LookupMode::DenseId => (plan.files.len() * UddpLocator::SERIALIZED_SIZE) as u64,
            LookupMode::VirtualPathHash => (plan.files.len() * UddpPathEntry::SERIALIZED_SIZE) as u64,
            LookupMode::SparseId => (plan.files.len() * UddpSparseIdEntry::SERIALIZED_SIZE) as u64,
        };

        let blob_offset = index_offset + index_size;
        if blob_offset >= MAX_PACKAGE_SIZE {
            return Err(BuildError::PackageTooLarge(blob_offset));
        }

        let mut dict_refs = Vec::with_capacity(plan.dictionaries.len());
        let mut locator_records = Vec::with_capacity(plan.files.len());
        let mut blob_cursor = blob_offset;

        for dict in &plan.dictionaries {
            dict_refs.push(UddpDictRef {
                data_type: dict.data_type,
                codec: dict.codec as u8,
                reserved0: 0,
                offset: blob_cursor,
                size: dict.bytes.len() as u32,
                reserved1: 0,
            });
            blob_cursor += dict.bytes.len() as u64;
        }

        for file in &plan.files {
            let stored_size = file.encoded_payload.len() as u32;
            let delta = file.raw_size - stored_size;
            let meta32 = pack_meta32(file.data_type, file.codec, delta);
            let pos64 = pack_pos64(blob_cursor, delta);
            locator_records.push((
                file.key,
                UddpLocator {
                    raw_size: file.raw_size,
                    meta32,
                    pos64,
                },
            ));
            blob_cursor += stored_size as u64;
        }

        if blob_cursor > MAX_PACKAGE_SIZE {
            return Err(BuildError::PackageTooLarge(blob_cursor));
        }

        completed += 1;
        progress(BuildProgress {
            phase: BuildProgressPhase::Assembling,
            completed,
            total: total_steps,
        });

        let header = UddpHeader {
            magic: UDDP_MAGIC,
            version_major: self.version_major,
            version_minor: self.version_minor,
            lookup_mode: self.lookup_mode as u8,
            reserved0: 0,
            dict_count: plan.dictionaries.len() as u16,
            file_count: plan.files.len() as u32,
            dict_table_offset,
            index_offset,
            blob_offset,
            package_hash64: 0,
        };

        let mut out = Vec::new();
        header.write_to(&mut out);
        for dict in &dict_refs {
            dict.write_to(&mut out);
        }

        match self.lookup_mode {
            LookupMode::DenseId => {
                let mut dense = vec![None; plan.files.len()];
                for (key, locator) in &locator_records {
                    let PendingKey::Id(id) = key else {
                        return Err(BuildError::WrongKeyForLookupMode);
                    };
                    let idx = *id as usize;
                    if idx >= dense.len() {
                        return Err(BuildError::DenseIdGap);
                    }
                    dense[idx] = Some(*locator);
                }
                for locator in dense {
                    locator.ok_or(BuildError::DenseIdGap)?.write_to(&mut out);
                }
            }
            LookupMode::VirtualPathHash => {
                let mut entries = Vec::new();
                for (key, locator) in &locator_records {
                    let PendingKey::PathHash(path_hash64) = key else {
                        return Err(BuildError::WrongKeyForLookupMode);
                    };
                    entries.push(UddpPathEntry {
                        path_hash64: *path_hash64,
                        locator: *locator,
                    });
                }
                entries.sort_by_key(|entry| entry.path_hash64);
                for entry in &entries {
                    entry.write_to(&mut out);
                }
            }
            LookupMode::SparseId => {
                let mut entries = Vec::new();
                for (key, locator) in &locator_records {
                    let PendingKey::Id(id) = key else {
                        return Err(BuildError::WrongKeyForLookupMode);
                    };
                    entries.push(UddpSparseIdEntry {
                        id: *id,
                        reserved0: 0,
                        locator: *locator,
                    });
                }
                entries.sort_by_key(|entry| entry.id);
                for entry in &entries {
                    entry.write_to(&mut out);
                }
            }
        }

        completed += 1;
        progress(BuildProgress {
            phase: BuildProgressPhase::Assembling,
            completed,
            total: total_steps,
        });

        for dict in &plan.dictionaries {
            out.extend_from_slice(&dict.bytes);
            completed += 1;
            progress(BuildProgress {
                phase: BuildProgressPhase::Assembling,
                completed,
                total: total_steps,
            });
        }
        for file in &plan.files {
            out.extend_from_slice(&file.encoded_payload);
            completed += 1;
            progress(BuildProgress {
                phase: BuildProgressPhase::Assembling,
                completed,
                total: total_steps,
            });
        }

        let package_hash64 = canonical_package_hash64(&out);
        patch_header_package_hash64(&mut out, package_hash64)?;
        Ok(out)
    }
}

/// Convert an on-disk codec back into the user-facing builder preference.
///
/// Patch application uses this to preserve the original compression intent of
/// unchanged files when rebuilding a patched package.
pub(crate) fn codec_to_compression_flag(codec: Codec) -> CompressionFlag {
    match codec {
        Codec::None => CompressionFlag::None,
        Codec::ZstdNoDict => CompressionFlag::ZstdNoDict,
        Codec::ZstdTypeDict => CompressionFlag::ZstdDict,
        Codec::JpegXl => CompressionFlag::JpegXl,
    }
}

/// Decide whether we have enough same-type samples to justify dictionary training.
fn should_train_dict(samples: &[&[u8]]) -> bool {
    if samples.len() < 16 {
        return false;
    }
    let total: usize = samples.iter().map(|sample| sample.len()).sum();
    total >= 128 * 1024
}

/// Keep dictionary training deterministic and bounded.
///
/// Dictionaries only help the smaller payloads that use dict compression, so
/// avoid training on giant blobs and cap the total corpus size to keep build
/// time predictable.
fn select_dict_training_samples<'a>(samples: &'a [&'a [u8]]) -> Vec<&'a [u8]> {
    let eligible = samples
        .iter()
        .copied()
        .filter(|sample| {
            let len = sample.len();
            (DICT_TRAIN_MIN_SAMPLE_BYTES..=DICT_TRAIN_AUTO_MAX_SAMPLE_BYTES).contains(&len)
        })
        .collect::<Vec<_>>();

    if eligible.len() <= DICT_TRAIN_MAX_SAMPLE_COUNT {
        return cap_training_sample_bytes(eligible);
    }

    let target_count = DICT_TRAIN_MAX_SAMPLE_COUNT;
    let mut evenly_spaced = Vec::with_capacity(target_count);
    for i in 0..target_count {
        let index = i * eligible.len() / target_count;
        evenly_spaced.push(eligible[index]);
    }

    cap_training_sample_bytes(evenly_spaced)
}

fn cap_training_sample_bytes<'a>(samples: Vec<&'a [u8]>) -> Vec<&'a [u8]> {
    let mut total = 0usize;
    let mut capped = Vec::with_capacity(samples.len());

    for sample in samples {
        if total >= DICT_TRAIN_MAX_TOTAL_BYTES {
            break;
        }
        total += sample.len();
        capped.push(sample);
    }

    capped
}

/// Pick a practical dictionary size from the available sample volume.
fn choose_dict_size(samples: &[&[u8]]) -> usize {
    let total: usize = samples.iter().map(|sample| sample.len()).sum();
    if total >= 1024 * 1024 {
        16 * 1024
    } else if total >= 256 * 1024 {
        8 * 1024
    } else {
        4 * 1024
    }
}

/// Apply the current automatic compression policy for one file.
fn choose_auto_compression(
    file: &PendingFile,
    dict: Option<&Vec<u8>>,
) -> Result<(Codec, Vec<u8>), BuildError> {
    let len = file.raw_data.len();

    // Policy chosen for many small game assets:
    // - below 128 bytes, compression metadata is usually not worth the cost
    // - up to 48 KiB, dictionaries are often effective when available
    // - larger blobs usually benefit more from plain Zstd than from per-type dicts
    if len < 128 {
        return Ok((Codec::None, file.raw_data.clone()));
    }

    if len <= 48 * 1024 {
        if let Some(dict) = dict {
            let compressed = zstd_compress_with_dict(&file.raw_data, dict).map_err(BuildError::Io)?;
            if compression_is_worth_it(len, compressed.len()) {
                return Ok((Codec::ZstdTypeDict, compressed));
            }
        }

        let compressed = zstd_compress(&file.raw_data).map_err(BuildError::Io)?;
        if compression_is_worth_it(len, compressed.len()) {
            return Ok((Codec::ZstdNoDict, compressed));
        }

        return Ok((Codec::None, file.raw_data.clone()));
    }

    let compressed = zstd_compress(&file.raw_data).map_err(BuildError::Io)?;
    if compression_is_worth_it(len, compressed.len()) {
        Ok((Codec::ZstdNoDict, compressed))
    } else {
        Ok((Codec::None, file.raw_data.clone()))
    }
}

/// Require a meaningful absolute and relative size win before compressing.
fn compression_is_worth_it(raw_size: usize, stored_size: usize) -> bool {
    if stored_size >= raw_size {
        return false;
    }

    let saved = raw_size - stored_size;
    let pct = (saved * 100) / raw_size.max(1);
    saved >= 32 && pct >= 5
}

/// Validate that all file keys satisfy the invariants of the chosen lookup mode.
fn validate_keys(mode: LookupMode, files: &[BuiltFile]) -> Result<(), BuildError> {
    match mode {
        LookupMode::DenseId => {
            let mut ids: Vec<u32> = files
                .iter()
                .map(|file| match file.key {
                    PendingKey::Id(id) => id,
                    PendingKey::PathHash(_) => u32::MAX,
                })
                .collect();
            ids.sort_unstable();
            for (index, id) in ids.iter().enumerate() {
                if *id != index as u32 {
                    return Err(BuildError::DenseIdGap);
                }
            }
        }
        LookupMode::VirtualPathHash => {
            let mut seen = HashSet::new();
            for file in files {
                let PendingKey::PathHash(hash) = file.key else {
                    return Err(BuildError::WrongKeyForLookupMode);
                };
                if !seen.insert(hash) {
                    return Err(BuildError::DuplicatePathHash(hash));
                }
            }
        }
        LookupMode::SparseId => {
            let mut seen = HashSet::new();
            for file in files {
                let PendingKey::Id(id) = file.key else {
                    return Err(BuildError::WrongKeyForLookupMode);
                };
                if !seen.insert(id) {
                    return Err(BuildError::DuplicateId(id));
                }
            }
        }
    }

    Ok(())
}

/// Train a Zstd dictionary from a set of same-type samples.
fn train_zstd_dict(samples: &[&[u8]], dict_size: usize) -> Result<Vec<u8>, BuildError> {
    let dict = zstd::dict::from_samples(samples, dict_size).map_err(BuildError::Io)?;
    Ok(dict)
}
