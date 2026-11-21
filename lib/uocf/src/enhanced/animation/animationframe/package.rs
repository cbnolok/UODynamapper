//! Package-level edits for frame imports: read or write one `AnimationFrame*.uop` entry, and set an action's frame
//! count in `AnimationSequence.uop` when an import changes it, or write a whole edited sequence entry. Writers never
//! overwrite their source: they write a copy to a new path, with the edited entry under a fresh timestamp header
//! (`uop_entries`).

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
/// alias action (it owns no frames), and frame effects or skipped frames at or past the new count
/// ([`AnimationSequence::set_frame_count`]).
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
    let previous = sequence.set_frame_count(action, frame_count).wrap_err_with(|| path.clone())?;
    put_sequence(&mut package, sequence_id, &sequence, dialect)?;
    uop_entries::save_package_as(&mut package, source, output)?;
    Ok(previous)
}

/// Writes a copy of the `source` `AnimationSequence.uop` to `output` with `sequence` as the `sequence_id` entry,
/// encoded in `dialect` (replaced keeping its compression, or added). Refuses a sequence with
/// [`AnimationSequence::edit_errors`] or one the dialect's layout cannot hold. `output` must not be `source`.
pub fn write_sequence(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    sequence_id: u32,
    sequence: &AnimationSequence,
    dialect: ClientDialect,
) -> eyre::Result<EntryChange> {
    let (source, output) = (source.as_ref(), output.as_ref());
    uop_entries::refuse_to_overwrite(source, output)?;
    let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
    let change = put_sequence(&mut package, sequence_id, sequence, dialect)?;
    uop_entries::save_package_as(&mut package, source, output)?;
    Ok(change)
}

/// Checks and encodes `sequence`, and puts it in `package` as the `sequence_id` entry.
fn put_sequence(
    package: &mut UopPackage,
    sequence_id: u32,
    sequence: &AnimationSequence,
    dialect: ClientDialect,
) -> eyre::Result<EntryChange> {
    let path = AnimationSequence::entry_path(sequence_id);
    let errors = sequence.edit_errors();
    eyre::ensure!(errors.is_empty(), "{path}: {}", errors.join("; "));
    let payload = sequence.to_bytes_dialect(dialect).wrap_err_with(|| format!("failed to encode {path}"))?;
    put(package, crate::uop_container::hash::hash_file_name_single(&path), &payload)
}

/// The `sequence_id` entry of an `AnimationSequence.uop` package, parsed in `dialect`; `None` when it is missing.
pub fn read_sequence(package: &UopPackage, sequence_id: u32, dialect: ClientDialect) -> eyre::Result<Option<AnimationSequence>> {
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
