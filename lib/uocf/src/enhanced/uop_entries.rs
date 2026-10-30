//! Entry-level edits of client UOP packages shared by the effect file writers: replace, add or remove one entry
//! and save the package to a new path. Untouched entries keep their data block headers; replaced and added ones
//! get a fresh timestamp header (`UopPackage::edit_data_block_header`).

use std::path::Path;

use color_eyre::eyre::{self, WrapErr};

use crate::uop_container::block::UopBlock;
use crate::uop_container::file::{CompressionFlag, UopFile};
use crate::uop_container::package::UopPackage;

/// What [`put_entry`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryPut {
    /// An entry with the hash existed; its payload was replaced, keeping its compression flag. An entry that
    /// already held the payload is left untouched.
    Replaced,
    /// A new entry was added.
    Added,
}

/// Stores `payload` as the entry `hash`. An existing entry keeps its compression flag
/// (`UopPackage::replace_file`); a new entry gets `compression`. Both get a fresh timestamp data block header
/// stamped with the package's edit FILETIME, and its Adler-32 as `data_block_hash`, in version-4 and version-5
/// packages alike; an existing entry whose payload is unchanged keeps its header and hash.
pub(crate) fn put_entry(
    package: &mut UopPackage,
    hash: u64,
    payload: &[u8],
    compression: CompressionFlag,
) -> eyre::Result<EntryPut> {
    if let Some(file) = package.get_file_by_hash(hash) {
        let compression = file.compression();
        package
            .replace_file(hash, payload, compression)
            .wrap_err_with(|| format!("failed to replace entry {hash:016X}"))?;
        return Ok(EntryPut::Replaced);
    }

    let mut file = UopFile::new()
        .create_file_from_bytes(payload, hash, compression)
        .wrap_err_with(|| format!("failed to encode entry {hash:016X}"))?;
    file.set_data_block_header_and_hash(&package.edit_data_block_header())?;
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::uop_container::data_block_header::{
        DataBlockHeader, DataBlockHeaderRecord, DataBlockSignature, RECORD_PREFIX_SIZE, SIGNATURE_KEY_ID_ENHANCED,
        TIMESTAMP_SIZE,
    };
    use crate::uop_container::hash::{hash_data_block, hash_file_name_single};
    use crate::uop_container::package::LoadMode;

    /// FILETIME injected as the edit time.
    const EDIT_FILETIME: u64 = 0x01db_2a3b_4c5d_6e7f;
    /// FILETIME of the headers the synthetic version-4 entries start with.
    const SOURCE_FILETIME: u64 = 0x01da_8abf_ed6a_8a60;
    /// Signature value of the headers the synthetic version-5 entries start with.
    const SOURCE_SIGNATURE_LEN: usize = 128;
    const SOURCE_SIGNATURE_BYTE: u8 = 0x5a;
    /// The package versions the client uses: timestamp headers in version 4, signature headers in version 5.
    const PACKAGE_VERSIONS: [u32; 2] = [4, 5];
    /// Block size of the synthetic packages: their three entries fill it, so the added entry opens a new block.
    const BLOCK_SIZE: u32 = 3;
    const UNTOUCHED: &str = "build/test/untouched.bin";
    const REPLACED: &str = "build/test/replaced.bin";
    const SAME_PAYLOAD: &str = "build/test/same_payload.bin";
    const ADDED: &str = "build/test/added.bin";
    const OLD_PAYLOAD: &[u8] = b"old payload";
    const NEW_PAYLOAD: &[u8] = b"new payload";
    const ADDED_PAYLOAD: &[u8] = b"added payload";

    /// The header client packages of `version` store before every payload.
    fn client_header(version: u32) -> DataBlockHeader {
        if version == PACKAGE_VERSIONS[0] {
            DataBlockHeader::from_timestamp(SOURCE_FILETIME)
        } else {
            DataBlockHeader::new(vec![DataBlockHeaderRecord::Signature(DataBlockSignature {
                key_id: SIGNATURE_KEY_ID_ENHANCED,
                value: vec![SOURCE_SIGNATURE_BYTE; SOURCE_SIGNATURE_LEN],
            })])
        }
    }

    /// The bytes an entry occupies in a saved package: its data block header and stored payload.
    fn stored_bytes<'a>(package_bytes: &'a [u8], file: &UopFile) -> &'a [u8] {
        let start = file.data_block_address() as usize;
        &package_bytes[start..start + file.data_block_length() as usize + file.compressed_size() as usize]
    }

    /// An edited entry holds `payload` after one timestamp record of the injected FILETIME, hashed with Adler-32.
    fn assert_fresh_header(file: &UopFile, payload: &[u8], context: &str) {
        let header = file.data_block_header().expect("a header");
        assert_eq!(file.data_block_length() as usize, RECORD_PREFIX_SIZE + TIMESTAMP_SIZE, "{context}");
        assert_eq!(file.data_block_length() as usize, header.len(), "{context}");
        let parsed = DataBlockHeader::parse(header).unwrap();
        assert_eq!(parsed.records(), &[DataBlockHeaderRecord::Timestamp(EDIT_FILETIME)], "{context}");
        assert_eq!(file.data_block_hash(), hash_data_block(header).unwrap(), "{context}: Adler-32 of the header");
        assert_eq!(file.unpack().unwrap(), payload, "{context}");
    }

    #[test]
    fn edited_entries_get_a_fresh_timestamp_header_in_both_versions() {
        let dir = std::env::temp_dir().join(format!("uocf_uop_entries_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for version in PACKAGE_VERSIONS {
            let source = dir.join(format!("source_v{version}.uop"));
            let output = dir.join(format!("output_v{version}.uop"));
            let mut package = UopPackage::new(version, BLOCK_SIZE).unwrap();
            for path in [UNTOUCHED, REPLACED, SAME_PAYLOAD] {
                package.add_file_from_memory(OLD_PAYLOAD, path, CompressionFlag::Zlib).unwrap();
                let file = package.get_file_by_hash_mut(hash_file_name_single(path)).unwrap();
                file.set_data_block_header_and_hash(&client_header(version)).unwrap();
            }
            package.finalize_and_save(&source).unwrap();

            let mut package = UopPackage::load_with_mode(&source, LoadMode::Lazy).unwrap();
            package.set_edit_filetime(Some(EDIT_FILETIME));
            let put = |package: &mut UopPackage, path: &str, payload: &[u8]| {
                put_entry(package, hash_file_name_single(path), payload, CompressionFlag::None).unwrap()
            };
            assert_eq!(put(&mut package, REPLACED, NEW_PAYLOAD), EntryPut::Replaced);
            assert_eq!(put(&mut package, SAME_PAYLOAD, OLD_PAYLOAD), EntryPut::Replaced);
            assert_eq!(put(&mut package, ADDED, ADDED_PAYLOAD), EntryPut::Added);
            package.finalize_and_save(&output).unwrap();

            let (source_bytes, output_bytes) = (std::fs::read(&source).unwrap(), std::fs::read(&output).unwrap());
            let (source, written) = (UopPackage::load(&source).unwrap(), UopPackage::load(&output).unwrap());
            assert_eq!(written.version(), version);
            for path in [UNTOUCHED, SAME_PAYLOAD] {
                let (before, after) = (source.get_file_by_name(path).unwrap(), written.get_file_by_name(path).unwrap());
                let context = format!("version {version}: {path}");
                assert_eq!(stored_bytes(&output_bytes, after), stored_bytes(&source_bytes, before), "{context}");
                assert_eq!(after.data_block_hash(), before.data_block_hash(), "{context}");
                assert_eq!(after.compression(), before.compression(), "{context}");
                assert_eq!(after.unpack().unwrap(), OLD_PAYLOAD, "{context}");
            }
            let replaced = written.get_file_by_name(REPLACED).unwrap();
            assert_fresh_header(replaced, NEW_PAYLOAD, &format!("version {version}: replaced"));
            assert_eq!(replaced.compression(), CompressionFlag::Zlib, "a replaced entry keeps its compression");
            let added = written.get_file_by_name(ADDED).unwrap();
            assert_fresh_header(added, ADDED_PAYLOAD, &format!("version {version}: added"));
            assert_eq!(added.compression(), CompressionFlag::None);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn put_and_remove_entries() {
        let existing = hash_file_name_single("build/test/existing.bin");
        let added = hash_file_name_single("build/test/added.bin");
        let mut package = UopPackage::new_default();
        package.set_edit_filetime(Some(EDIT_FILETIME));
        package.add_file_from_memory(b"old", "build/test/existing.bin", CompressionFlag::None).unwrap();

        assert_eq!(put_entry(&mut package, existing, b"new", CompressionFlag::Zlib).unwrap(), EntryPut::Replaced);
        let file = package.get_file_by_hash(existing).unwrap();
        assert_eq!(file.compression(), CompressionFlag::None, "a replaced entry keeps its compression");
        assert_fresh_header(file, b"new", "an entry without a header gets one");
        let stamped = Arc::clone(file.data_block_header().unwrap());
        package.set_edit_filetime(Some(SOURCE_FILETIME));
        assert_eq!(put_entry(&mut package, existing, b"new", CompressionFlag::Zlib).unwrap(), EntryPut::Replaced);
        let file = package.get_file_by_hash(existing).unwrap();
        assert_eq!(file.data_block_header(), Some(&stamped), "an unchanged payload keeps its stamp");
        package.set_edit_filetime(Some(EDIT_FILETIME));

        assert_eq!(put_entry(&mut package, added, b"added", CompressionFlag::Zlib).unwrap(), EntryPut::Added);
        let file = package.get_file_by_hash(added).unwrap();
        assert_eq!(file.compression(), CompressionFlag::Zlib);
        assert_fresh_header(file, b"added", "a new entry gets a header");

        assert!(remove_entry(&mut package, existing));
        assert!(!remove_entry(&mut package, existing));
        assert!(package.get_file_by_hash(existing).is_none());
        assert_eq!(package.iter_files().filter(|file| !file.is_empty_slot()).count(), 1);
        let empty = hash_file_name_single("build/test/fills_the_slot.bin");
        assert_eq!(put_entry(&mut package, empty, b"slot", CompressionFlag::Zlib).unwrap(), EntryPut::Added);
        assert_eq!(package.iter_files().count(), 2, "the new entry fills the empty slot");
    }
}
