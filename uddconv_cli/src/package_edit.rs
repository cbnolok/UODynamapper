use std::collections::HashMap;
use std::fs;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use uocf::udd::{
    AddFileRequest, Codec, CompressionFlag, LookupMode, UddpBuilder, UddpReader,
    xxh64_virtual_path,
};
use uocf::udd::uddp::{FileKey, unpack_codec, unpack_offset40, unpack_planar, unpack_type};

pub fn rebuild_package(input: &Path, output: &Path) -> eyre::Result<()> {
    rewrite_package(input, output, &HashMap::new())
}

pub fn replace_virtual_path_file(
    input: &Path,
    output: &Path,
    virtual_path: &str,
    payload: &[u8],
) -> eyre::Result<()> {
    let mut replacements = HashMap::new();
    replacements.insert(xxh64_virtual_path(virtual_path), payload.to_vec());
    rewrite_package(input, output, &replacements)
}

fn rewrite_package(
    input: &Path,
    output: &Path,
    replacements: &HashMap<u64, Vec<u8>>,
) -> eyre::Result<()> {
    let source_bytes = fs::read(input).wrap_err_with(|| format!("read {}", input.display()))?;
    let reader = UddpReader::open(source_bytes).wrap_err_with(|| format!("open {}", input.display()))?;

    let mut builder = UddpBuilder::new(reader.lookup_mode());
    builder.set_version(reader.header().version_major, reader.header().version_minor);

    let mut records = reader.records();
    records.sort_by_key(|record| unpack_offset40(record.locator.pos64));
    let mut replaced_count = 0usize;

    for record in records {
        let raw = match record.key {
            FileKey::PathHash(path_hash64) => reader.read_file_by_path_hash(path_hash64)?,
            FileKey::Id(id) => match reader.lookup_mode() {
                LookupMode::DenseId => reader.read_file_by_dense_id(id)?,
                LookupMode::SparseId => reader.read_file_by_sparse_id(id)?,
                LookupMode::VirtualPathHash => unreachable!("path-hash packages must expose path-hash keys"),
            },
        };
        let payload = match record.key {
            FileKey::PathHash(path_hash64) => {
                if let Some(replacement) = replacements.get(&path_hash64) {
                    replaced_count += 1;
                    replacement.as_slice()
                } else {
                    raw.as_slice()
                }
            }
            FileKey::Id(_) => raw.as_slice(),
        };

        match record.key {
            FileKey::PathHash(path_hash64) => builder.add_file(AddFileRequest {
                data_type: unpack_type(record.locator.meta32),
                compression: codec_to_compression_flag_local(unpack_codec(record.locator.meta32)),
                apply_planar: unpack_planar(record.locator.meta32),
                virtual_path: None,
                path_hash64: Some(path_hash64),
                id: None,
                data: payload,
            })?,
            FileKey::Id(id) => builder.add_file(AddFileRequest {
                data_type: unpack_type(record.locator.meta32),
                compression: codec_to_compression_flag_local(unpack_codec(record.locator.meta32)),
                apply_planar: unpack_planar(record.locator.meta32),
                virtual_path: None,
                path_hash64: None,
                id: Some(id),
                data: payload,
            })?,
        }
    }

    if replaced_count != replacements.len() {
        eyre::bail!("one or more replacement virtual paths were not found in the package");
    }

    let rebuilt = builder.build()?;
    write_output(output, &rebuilt)
}

fn write_output(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).wrap_err_with(|| format!("create {}", parent.display()))?;
    }

    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, bytes).wrap_err_with(|| format!("write {}", temp_path.display()))?;
    fs::rename(&temp_path, path).wrap_err_with(|| {
        format!(
            "replace {} with {}",
            path.display(),
            temp_path.display()
        )
    })?;
    Ok(())
}

fn codec_to_compression_flag_local(codec: Codec) -> CompressionFlag {
    match codec {
        Codec::None => CompressionFlag::None,
        Codec::ZstdNoDict => CompressionFlag::ZstdNoDict,
        Codec::ZstdTypeDict | Codec::Reserved => CompressionFlag::Auto,
    }
}
