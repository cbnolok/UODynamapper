//! Entry-level edits of client UOP packages shared by the effect file writers: replace, add or remove one entry
//! and save the package to a new path, keeping the client's data block headers.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use color_eyre::eyre::{self, WrapErr};

use crate::uop_container::block::UopBlock;
use crate::uop_container::file::{CompressionFlag, UopFile};
use crate::uop_container::hash::hash_data_block;
use crate::uop_container::package::UopPackage;

/// Size of the data block header the client packages store before each payload: `u16 3`, `u16 8`, `u64`
/// FILETIME. Their `data_block_hash` is the Adler-32 of these bytes.
pub(crate) const CLIENT_DATA_BLOCK_HEADER_SIZE: usize = 12;
/// First field of the client data block header (observed constant; meaning unknown).
const CLIENT_DATA_BLOCK_HEADER_KIND: u16 = 3;
/// Second field of the client data block header: the size of the FILETIME that follows.
const CLIENT_DATA_BLOCK_HEADER_TIME_SIZE: u16 = 8;
/// FILETIME of the Unix epoch: 100 ns intervals since 1601-01-01.
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;
/// FILETIME intervals per second.
const FILETIME_INTERVALS_PER_SECOND: u64 = 10_000_000;
/// Nanoseconds per FILETIME interval.
const NANOSECONDS_PER_FILETIME_INTERVAL: u32 = 100;

/// What [`put_entry`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryPut {
    /// An entry with the hash existed; its payload was replaced (compression flag and data block header kept).
    Replaced,
    /// A new entry was added.
    Added,
}

/// Stores `payload` as the entry `hash`. An existing entry keeps its compression flag, data block header and
/// header hash (the client hashes the header, not the payload). A new entry gets `compression` and, when the
/// package's entries carry client data block headers, a fresh one stamped with the current time.
pub(crate) fn put_entry(
    package: &mut UopPackage,
    hash: u64,
    payload: &[u8],
    compression: CompressionFlag,
) -> eyre::Result<EntryPut> {
    if package.get_file_by_hash(hash).is_some() {
        package.ensure_file_data_loaded_by_hash(hash).wrap_err_with(|| format!("failed to load entry {hash:016X}"))?;
        let file = package.get_file_by_hash_mut(hash).expect("the entry was just found");
        let mut replacement = UopFile::new()
            .create_file_from_bytes(payload, hash, file.compression())
            .wrap_err_with(|| format!("failed to encode entry {hash:016X}"))?;
        if let Some(header) = file.data_block_header() {
            replacement.set_data_block_header(Arc::clone(header));
            replacement.set_data_block_hash(file.data_block_hash());
        }
        *file = replacement;
        return Ok(EntryPut::Replaced);
    }

    let uses_client_headers =
        package.iter_files().any(|file| file.data_block_length() as usize == CLIENT_DATA_BLOCK_HEADER_SIZE);
    let mut file = UopFile::new()
        .create_file_from_bytes(payload, hash, compression)
        .wrap_err_with(|| format!("failed to encode entry {hash:016X}"))?;
    if uses_client_headers {
        let header = client_data_block_header(current_filetime());
        file.set_data_block_hash(hash_data_block(&header)?);
        file.set_data_block_header(Arc::from(header.as_slice()));
    }
    let block_size = package.block_size() as usize;
    let blocks = package.blocks_mut();
    // Client packages pad their last block with empty slots: fill one, as the client's packer would.
    if let Some(slot) = blocks.iter_mut().flat_map(|block| block.files_mut().iter_mut()).find(|file| file.is_empty_slot()) {
        *slot = file;
        return Ok(EntryPut::Added);
    }
    match blocks.last_mut() {
        Some(block) if block.files().len() < block_size => block.add_file(file),
        _ => {
            let mut block = UopBlock::new();
            block.add_file(file);
            blocks.push(block);
        }
    }
    Ok(EntryPut::Added)
}

/// Removes the entry `hash`; returns whether it existed. Its slot becomes an empty slot, as in client packages.
pub(crate) fn remove_entry(package: &mut UopPackage, hash: u64) -> bool {
    if package.get_file_by_hash(hash).is_none() {
        return false;
    }
    for file in package.iter_files_mut().filter(|file| file.filename_hash() == hash) {
        *file = UopFile::new();
    }
    true
}

/// Fails when `output` is the `source` package (or a link to it): writers never overwrite client files.
pub(crate) fn refuse_to_overwrite(source: &Path, output: &Path) -> eyre::Result<()> {
    let canonical_source =
        std::fs::canonicalize(source).wrap_err_with(|| format!("failed to resolve {}", source.display()))?;
    if std::fs::canonicalize(output).is_ok_and(|canonical_output| canonical_output == canonical_source) {
        eyre::bail!("refusing to overwrite the source package {}: write to a new path", source.display());
    }
    Ok(())
}

/// Saves `package`, loaded from `source`, to `output`, which must not be `source`.
pub(crate) fn save_package_as(package: &mut UopPackage, source: &Path, output: &Path) -> eyre::Result<()> {
    refuse_to_overwrite(source, output)?;
    package.finalize_and_save(output).wrap_err_with(|| format!("failed to write {}", output.display()))
}

/// A client data block header stamped with `filetime`.
pub(crate) fn client_data_block_header(filetime: u64) -> [u8; CLIENT_DATA_BLOCK_HEADER_SIZE] {
    let mut header = [0; CLIENT_DATA_BLOCK_HEADER_SIZE];
    let (kind, rest) = header.split_at_mut(size_of::<u16>());
    let (time_size, time) = rest.split_at_mut(size_of::<u16>());
    kind.copy_from_slice(&CLIENT_DATA_BLOCK_HEADER_KIND.to_le_bytes());
    time_size.copy_from_slice(&CLIENT_DATA_BLOCK_HEADER_TIME_SIZE.to_le_bytes());
    time.copy_from_slice(&filetime.to_le_bytes());
    header
}

/// The current time as a Windows FILETIME.
fn current_filetime() -> u64 {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    FILETIME_UNIX_EPOCH
        + since_epoch.as_secs() * FILETIME_INTERVALS_PER_SECOND
        + u64::from(since_epoch.subsec_nanos() / NANOSECONDS_PER_FILETIME_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uop_container::hash::hash_file_name_single;

    #[test]
    fn client_header_layout() {
        let header = client_data_block_header(0x0102_0304_0506_0708);
        assert_eq!(header, [3, 0, 8, 0, 8, 7, 6, 5, 4, 3, 2, 1]);
        assert!(current_filetime() > FILETIME_UNIX_EPOCH);
    }

    #[test]
    fn put_and_remove_entries() {
        let existing = hash_file_name_single("build/test/existing.bin");
        let added = hash_file_name_single("build/test/added.bin");
        let mut package = UopPackage::new_default();
        package.add_file_from_memory(b"old", "build/test/existing.bin", CompressionFlag::None).unwrap();
        let header = client_data_block_header(1);
        let file = package.get_file_by_hash_mut(existing).unwrap();
        file.set_data_block_header(Arc::from(header.as_slice()));
        file.set_data_block_hash(hash_data_block(&header).unwrap());

        assert_eq!(put_entry(&mut package, existing, b"new", CompressionFlag::Zlib).unwrap(), EntryPut::Replaced);
        let file = package.get_file_by_hash(existing).unwrap();
        assert_eq!(file.unpack().unwrap(), b"new");
        assert_eq!(file.compression(), CompressionFlag::None, "a replaced entry keeps its compression");
        assert_eq!(file.data_block_header().map(|header| &header[..]), Some(&header[..]));

        assert_eq!(put_entry(&mut package, added, b"added", CompressionFlag::Zlib).unwrap(), EntryPut::Added);
        let file = package.get_file_by_hash(added).unwrap();
        assert_eq!(file.unpack().unwrap(), b"added");
        assert_eq!(file.compression(), CompressionFlag::Zlib);
        let new_header = file.data_block_header().expect("a client header for the new entry");
        assert_eq!(new_header.len(), CLIENT_DATA_BLOCK_HEADER_SIZE);
        assert_eq!(file.data_block_hash(), hash_data_block(new_header).unwrap());

        assert!(remove_entry(&mut package, existing));
        assert!(!remove_entry(&mut package, existing));
        assert!(package.get_file_by_hash(existing).is_none());
        assert_eq!(package.iter_files().filter(|file| !file.is_empty_slot()).count(), 1);
        let empty = hash_file_name_single("build/test/fills_the_slot.bin");
        assert_eq!(put_entry(&mut package, empty, b"slot", CompressionFlag::Zlib).unwrap(), EntryPut::Added);
        assert_eq!(package.iter_files().count(), 2, "the new entry fills the empty slot");
    }
}
