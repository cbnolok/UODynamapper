//! Package-level edits for frame imports: read or write one `AnimationFrame*.uop` entry, and set an action's frame
//! count in `AnimationSequence.uop` when an import changes it. Writers never overwrite their source: they write a
//! copy to a new path, with the edited entry under a fresh timestamp header (`uop_entries`).

use std::path::Path;

use color_eyre::eyre::{self, WrapErr};

use super::AnimationFrame;
use crate::animation_sequence::AnimationSequence;
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::uop_entries::{self, EntryPut};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::package::UopPackage;

/// Compression of a new entry in a package without sized entries (every shipped frame and sequence package is
/// zlib-compressed).
const DEFAULT_ENTRY_COMPRESSION: CompressionFlag = CompressionFlag::Zlib;

/// What a package write did to its entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryChange {
    /// The entry already held the payload; the output is a copy of the source.
    Unchanged,
    Replaced,
    Added,
}

/// The payload of the `body` / `action` entry, if the package has it.
pub fn read_entry(package: &UopPackage, body: u32, action: u16) -> eyre::Result<Option<Vec<u8>>> {
    let path = AnimationFrame::animationframe_path(body, action);
    package
        .unpack_file_by_hash(AnimationFrame::animationframe_hash(body, action))
        .wrap_err_with(|| format!("failed to unpack {path}"))
}

/// Writes a copy of `source` to `output` with `payload` as the `body` / `action` entry (replaced, keeping its
/// compression, or added with the compression of the package's entries). `output` must not be `source`.
pub fn write_entry(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    body: u32,
    action: u16,
    payload: &[u8],
) -> eyre::Result<EntryChange> {
    AnimationFrame::load(payload).wrap_err("the payload is not an AnimationFrame entry")?;
    write_payload(source, output, AnimationFrame::animationframe_hash(body, action), payload)
}

/// Writes a copy of `source` to `output` with `payload` as the entry `hash`, unchecked (the callers check the
/// payload's format): replaced keeping its compression, or added with the compression of the package's entries.
/// `output` must not be `source`.
pub fn write_payload(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    hash: u64,
    payload: &[u8],
) -> eyre::Result<EntryChange> {
    let (source, output) = (source.as_ref(), output.as_ref());
    uop_entries::refuse_to_overwrite(source, output)?;
    let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
    let change = put(&mut package, hash, payload)?;
    uop_entries::save_package_as(&mut package, source, output)?;
    Ok(change)
}

/// Frame count of `action` in the `sequence_id` entry of an `AnimationSequence.uop` package, in `dialect`;
/// `None` when the entry or the action is missing.
pub fn sequence_frame_count(
    package: &UopPackage,
    sequence_id: u32,
    action: u32,
    dialect: ClientDialect,
) -> eyre::Result<Option<u32>> {
    Ok(read_sequence(package, sequence_id, dialect)?
        .and_then(|sequence| sequence.actions.get(&action).map(|record| record.frame_count)))
}

/// Writes a copy of the `source` `AnimationSequence.uop` to `output` with `action` of the `sequence_id` entry set
/// to `frame_count` frames per direction; returns the previous count. Fails on a missing entry or action, an
/// alias action (it owns no frames), and frame effects or skipped frames at or past the new count.
pub fn write_sequence_frame_count(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    sequence_id: u32,
    action: u32,
    frame_count: u32,
    dialect: ClientDialect,
) -> eyre::Result<u32> {
    let (source, output) = (source.as_ref(), output.as_ref());
    uop_entries::refuse_to_overwrite(source, output)?;
    let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
    let path = AnimationSequence::entry_path(sequence_id);
    let mut sequence = read_sequence(&package, sequence_id, dialect)?
        .ok_or_else(|| eyre::eyre!("{} has no {path}", source.display()))?;
    let record = sequence.actions.get_mut(&action).ok_or_else(|| eyre::eyre!("{path} has no action {action}"))?;
    eyre::ensure!(record.alias.is_none(), "{path} action {action} is an alias: it owns no frames");
    if let Some(frame) = record.frame_effects.keys().chain(&record.skipped_frames).find(|&&frame| frame >= frame_count) {
        eyre::bail!("{path} action {action} has effects or a skipped frame at frame {frame}, past {frame_count} frames");
    }
    let previous = std::mem::replace(&mut record.frame_count, frame_count);
    let payload = sequence.to_bytes_dialect(dialect).wrap_err_with(|| format!("failed to encode {path}"))?;
    put(&mut package, crate::uop_container::hash::hash_file_name_single(&path), &payload)?;
    uop_entries::save_package_as(&mut package, source, output)?;
    Ok(previous)
}

fn read_sequence(package: &UopPackage, sequence_id: u32, dialect: ClientDialect) -> eyre::Result<Option<AnimationSequence>> {
    let path = AnimationSequence::entry_path(sequence_id);
    let payload = package
        .unpack_file_by_hash(crate::uop_container::hash::hash_file_name_single(&path))
        .wrap_err_with(|| format!("failed to unpack {path}"))?;
    payload
        .map(|payload| AnimationSequence::parse_dialect(&payload, dialect).wrap_err_with(|| format!("failed to parse {path}")))
        .transpose()
}

fn put(package: &mut UopPackage, hash: u64, payload: &[u8]) -> eyre::Result<EntryChange> {
    let current = package.unpack_file_by_hash(hash).wrap_err_with(|| format!("failed to unpack {hash:016X}"))?;
    if current.as_deref() == Some(payload) {
        return Ok(EntryChange::Unchanged);
    }
    let compression = package
        .iter_files()
        .find(|file| file.has_size())
        .map_or(DEFAULT_ENTRY_COMPRESSION, |file| file.compression());
    Ok(match uop_entries::put_entry(package, hash, payload, compression)? {
        EntryPut::Replaced => EntryChange::Replaced,
        EntryPut::Added => EntryChange::Added,
    })
}
