//! Package builder that keeps payloads on disk instead of in memory.
//!
//! `UddpBuilder` holds every payload until `build()`, so it can train type
//! dictionaries over all samples. Packers producing gigabytes of payloads use
//! this builder instead: each file is compressed when added and its stored
//! payload is appended to a spill file next to the output. `finish()` then
//! writes the package head followed by the spilled payloads, so memory holds
//! only the per-file layout. The bytes written match what `UddpBuilder` writes
//! for the same files added in the same order.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use super::builder::{
    BuiltFile, FileLayout, HashingWriter, PACKAGE_FILE_WRITE_BUFFER_BYTES,
    PACKAGE_HEAD_ASSEMBLY_STEPS, PendingFile, assembly_step_count,
    compress_owned_files_with_progress, make_pending_file, patch_file_package_hash64,
    serialize_package_head, validate_keys,
};
use super::*;

/// Suffix appended to the output file name to name the payload spill file.
const SPILL_FILE_SUFFIX: &str = ".payloads.tmp";
/// Buffer of payloads appended to the spill file.
const SPILL_WRITE_BUFFER_BYTES: usize = 1024 * 1024;
/// Buffer used to copy spilled payloads into the output file.
const SPILL_COPY_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Builder for runtime `.uddp` containers that streams payloads to disk.
///
/// Files are compressed as they are added, so compressions that need a type
/// dictionary trained over all files (`ZstdDict`, `Auto`) are rejected.
pub struct UddpStreamingBuilder {
    lookup_mode: LookupMode,
    next_dense_id: u32,
    files: Vec<FileLayout>,
    compression_summary: CompressionSummary,
    output_path: PathBuf,
    spill_path: PathBuf,
    spill: Option<BufWriter<File>>,
    spill_size: u64,
}

impl UddpStreamingBuilder {
    /// Create a builder writing the package to `output_path`.
    ///
    /// Payloads are spilled to a temporary file next to the output until
    /// `finish()`; the spill file is removed when the builder is dropped.
    pub fn create(lookup_mode: LookupMode, output_path: impl Into<PathBuf>) -> Result<Self, BuildError> {
        let output_path = output_path.into();
        let mut spill_name = output_path.file_name().unwrap_or_default().to_os_string();
        spill_name.push(SPILL_FILE_SUFFIX);
        let spill_path = output_path.with_file_name(spill_name);
        let spill = File::create(&spill_path).map_err(BuildError::Io)?;
        Ok(Self {
            lookup_mode,
            next_dense_id: 0,
            files: Vec::new(),
            compression_summary: CompressionSummary::default(),
            output_path,
            spill_path,
            spill: Some(BufWriter::with_capacity(SPILL_WRITE_BUFFER_BYTES, spill)),
            spill_size: 0,
        })
    }

    /// Path the package is written to by `finish()`.
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// Compression requested by the files added so far.
    pub fn compression_summary(&self) -> CompressionSummary {
        self.compression_summary
    }

    /// Compress one logical file and append it to the package.
    pub fn add_file(&mut self, req: AddFileRequest<'_>) -> Result<(), BuildError> {
        let file = make_pending_file(
            self.lookup_mode,
            &mut self.next_dense_id,
            req.data_type,
            req.compression,
            req.width,
            req.height,
            req.virtual_path,
            req.path_hash64,
            req.id,
            req.data.to_vec(),
        )?;
        self.add_pending_files(vec![file])
    }

    /// Compress one owned logical file and append it to the package.
    pub fn add_owned_file(&mut self, req: AddOwnedFileRequest) -> Result<(), BuildError> {
        self.add_owned_files(vec![req])
    }

    /// Compress owned logical files in parallel and append them to the package
    /// in the given order.
    pub fn add_owned_files(&mut self, reqs: Vec<AddOwnedFileRequest>) -> Result<(), BuildError> {
        let mut files = Vec::with_capacity(reqs.len());
        for req in reqs {
            files.push(make_pending_file(
                self.lookup_mode,
                &mut self.next_dense_id,
                req.data_type,
                req.compression,
                req.width,
                req.height,
                req.virtual_path.as_deref(),
                req.path_hash64,
                req.id,
                req.data,
            )?);
        }
        self.add_pending_files(files)
    }

    fn add_pending_files(&mut self, files: Vec<PendingFile>) -> Result<(), BuildError> {
        for file in &files {
            if matches!(file.compression, CompressionFlag::ZstdDict | CompressionFlag::Auto) {
                return Err(BuildError::DictionaryCompressionWhileStreaming(file.compression));
            }
        }
        for file in &files {
            self.compression_summary.add(file.compression);
        }
        let no_dictionaries = HashMap::new();
        let built_files = compress_owned_files_with_progress(files, &no_dictionaries, |_| {})?;
        for file in built_files {
            self.append(file)?;
        }
        Ok(())
    }

    fn append(&mut self, file: BuiltFile) -> Result<(), BuildError> {
        let spill = self.spill.as_mut().ok_or(BuildError::MalformedOutput)?;
        let layout = file.layout();
        let spill_size = self.spill_size + layout.stored_size as u64;
        if spill_size > MAX_PACKAGE_SIZE {
            return Err(BuildError::PackageTooLarge(spill_size));
        }
        spill.write_all(&file.encoded_payload).map_err(BuildError::Io)?;
        self.spill_size = spill_size;
        self.files.push(layout);
        Ok(())
    }

    /// Write the package to the output path and remove the spill file.
    pub fn finish(self) -> Result<(), BuildError> {
        self.finish_with_progress(|_| {})
    }

    /// Write the package to the output path and remove the spill file, reporting
    /// assembly progress.
    pub fn finish_with_progress<F>(mut self, mut progress: F) -> Result<(), BuildError>
    where
        F: FnMut(BuildProgress),
    {
        let spill = self.spill.take().ok_or(BuildError::MalformedOutput)?;
        spill
            .into_inner()
            .map_err(|error| BuildError::Io(error.into_error()))?;
        validate_keys(self.lookup_mode, &self.files)?;

        let total_steps = assembly_step_count(0, self.files.len());
        progress(BuildProgress {
            phase: BuildProgressPhase::Assembling,
            completed: 0,
            total: total_steps,
            active_file: None,
        });
        let head = serialize_package_head(self.lookup_mode, &[], &self.files)?;
        let mut completed = 0;
        while completed < PACKAGE_HEAD_ASSEMBLY_STEPS {
            completed += 1;
            progress(BuildProgress {
                phase: BuildProgressPhase::Assembling,
                completed,
                total: total_steps,
                active_file: None,
            });
        }

        let output = File::create(&self.output_path).map_err(BuildError::Io)?;
        let mut writer = HashingWriter::new(BufWriter::with_capacity(
            PACKAGE_FILE_WRITE_BUFFER_BYTES,
            output,
        ));
        writer.write_all(&head).map_err(BuildError::Io)?;
        drop(head);

        let mut spill = File::open(&self.spill_path).map_err(BuildError::Io)?;
        let mut buffer = vec![0u8; SPILL_COPY_BUFFER_BYTES];
        for file in &self.files {
            let mut remaining = file.stored_size as usize;
            while remaining > 0 {
                let chunk = &mut buffer[..remaining.min(SPILL_COPY_BUFFER_BYTES)];
                spill.read_exact(chunk).map_err(BuildError::Io)?;
                writer.write_all(chunk).map_err(BuildError::Io)?;
                remaining -= chunk.len();
            }
            completed += 1;
            progress(BuildProgress {
                phase: BuildProgressPhase::Assembling,
                completed,
                total: total_steps,
                active_file: None,
            });
        }
        drop(spill);

        let (writer, package_hash64) = writer.finish();
        patch_file_package_hash64(writer, package_hash64)?;
        std::fs::remove_file(&self.spill_path).map_err(BuildError::Io)
    }
}

impl Drop for UddpStreamingBuilder {
    fn drop(&mut self) {
        // Close the spill before removing it; after `finish()` it is already gone.
        self.spill = None;
        let _ = std::fs::remove_file(&self.spill_path);
    }
}
