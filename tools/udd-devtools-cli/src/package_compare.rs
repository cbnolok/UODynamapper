//! Every record of a repacked package is compared with the reference, along with the
//! texture pages' stored sizes and decode times in both.
//!
//! Derived from the scratch lossless-zstd/pagecheck diagnostic.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::num::NonZeroUsize;
use clap::Args;
use color_eyre::eyre;
use std::time::Instant;

use rayon::prelude::*;
use udd_container::{
    Codec, DataType, FileKey, FileRecord, LookupMode, UddpReader, UddpReaderOptions, reconstruct_stored_size,
    unpack_codec, unpack_type,
};

const BYTES_PER_MB: f64 = 1_000_000.0;
const MS_PER_S: f64 = 1000.0;
const KEY_KIND_ID: u8 = 0;
const KEY_KIND_PATH_HASH: u8 = 1;

type SortKey = (u8, u64);

fn sort_key(key: &FileKey) -> SortKey {
    match key {
        FileKey::Id(id) => (KEY_KIND_ID, u64::from(*id)),
        FileKey::PathHash(hash) => (KEY_KIND_PATH_HASH, *hash),
    }
}

fn open(path: &Path) -> UddpReader {
    UddpReader::load_in_memory_with_options(path, UddpReaderOptions::disabled()).expect("open package")
}

fn records(reader: &UddpReader) -> BTreeMap<SortKey, FileRecord> {
    reader.records().into_iter().map(|record| (sort_key(&record.key), record)).collect()
}

fn read(reader: &UddpReader, key: &FileKey) -> Vec<u8> {
    match (reader.lookup_mode(), key) {
        (LookupMode::DenseId, FileKey::Id(id)) => reader.read_file_by_dense_id(*id),
        (LookupMode::SparseId, FileKey::Id(id)) => reader.read_file_by_sparse_id(*id),
        (_, FileKey::PathHash(hash)) => reader.read_file_by_path_hash(*hash),
        _ => panic!("unexpected key / lookup combination"),
    }
    .expect("read file")
}

fn stored_size(record: &FileRecord) -> usize {
    reconstruct_stored_size(record.locator.raw_size, record.locator.meta32, record.locator.pos64) as usize
}

fn best_read_ms(reader: &UddpReader, key: &FileKey, reps: usize) -> f64 {
    (0..reps)
        .map(|_| {
            let start = Instant::now();
            let bytes = read(reader, key);
            let ms = start.elapsed().as_secs_f64() * MS_PER_S;
            std::hint::black_box(&bytes);
            ms
        })
        .fold(f64::MAX, f64::min)
}

fn codec_name(codec: Codec) -> String {
    format!("{codec:?}")
}

#[derive(Args, Debug)]
pub struct Arguments {
    pub reference: PathBuf,
    pub candidate: PathBuf,
    /// In-memory reads per texture record; reports the best duration.
    #[arg(long, default_value = "1")]
    pub repetitions: NonZeroUsize,
}

pub fn run(args: Arguments) -> eyre::Result<()> {
    let (old_path, new_path, reps) = (&args.reference, &args.candidate, args.repetitions.get());
    let old = open(old_path);
    let new = open(new_path);
    let old_records = records(&old);
    let new_records = records(&new);

    let only_old = old_records.keys().filter(|k| !new_records.contains_key(k)).count();
    let only_new = new_records.keys().filter(|k| !old_records.contains_key(k)).count();
    let type_mismatches = old_records
        .iter()
        .filter(|(k, r)| new_records.get(k).is_some_and(|n| unpack_type(n.locator.meta32) != unpack_type(r.locator.meta32)))
        .count();
    let shared: Vec<&FileRecord> = old_records.iter().filter(|(k, _)| new_records.contains_key(k)).map(|(_, r)| r).collect();
    let mismatched: Vec<SortKey> = shared
        .par_iter()
        .filter(|record| read(&old, &record.key) != read(&new, &record.key))
        .map(|record| sort_key(&record.key))
        .collect();
    let identical = only_old == 0 && only_new == 0 && type_mismatches == 0 && mismatched.is_empty();
    let verdict = if identical { "IDENTICAL" } else { "DIFFERENT" };
    println!(
        "{verdict}: {} records old, {} new, {} only old, {} only new, {} data type mismatches, {} decoded mismatches {:?}",
        old_records.len(),
        new_records.len(),
        only_old,
        only_new,
        type_mismatches,
        mismatched.len(),
        mismatched.iter().take(5).collect::<Vec<_>>()
    );

    let pages: Vec<&FileRecord> = shared
        .iter()
        .copied()
        .filter(|record| unpack_type(record.locator.meta32) == DataType::Texture as u8)
        .collect();
    let (mut old_bytes, mut new_bytes, mut old_ms, mut new_ms) = (0usize, 0usize, 0.0f64, 0.0f64);
    for record in &pages {
        let new_record = &new_records[&sort_key(&record.key)];
        let (old_size, new_size) = (stored_size(record), stored_size(new_record));
        let (o_ms, n_ms) = (best_read_ms(&old, &record.key, reps), best_read_ms(&new, &record.key, reps));
        println!(
            "page {:?}\t{} {:.3} MB {:.1} ms\t{} {:.3} MB {:.1} ms",
            sort_key(&record.key),
            codec_name(unpack_codec(record.locator.meta32)),
            old_size as f64 / BYTES_PER_MB,
            o_ms,
            codec_name(unpack_codec(new_record.locator.meta32)),
            new_size as f64 / BYTES_PER_MB,
            n_ms
        );
        old_bytes += old_size;
        new_bytes += new_size;
        old_ms += o_ms;
        new_ms += n_ms;
    }
    println!(
        "TOTAL {} pages: old {:.2} MB {:.0} ms, new {:.2} MB {:.0} ms; package old {} B, new {} B (rayon threads {})",
        pages.len(),
        old_bytes as f64 / BYTES_PER_MB,
        old_ms,
        new_bytes as f64 / BYTES_PER_MB,
        new_ms,
        old.package_size_bytes(),
        new.package_size_bytes(),
        rayon::current_num_threads()
    );
    eyre::ensure!(identical, "decoded packages differ");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_container::{AddFileRequest, CompressionFlag, UddpBuilder};

    #[test]
    fn comparisons_ignore_compression_but_reject_payload_key_and_type_changes() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("udd_package_compare_{nonce}"));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, compression, payload: &[u8], data_type, extra| {
            let mut builder = UddpBuilder::new(LookupMode::SparseId);
            for id in if extra { vec![7, 8] } else { vec![7] } {
                builder.add_file(AddFileRequest {
                    data_type, compression, width: 0, height: 0,
                    virtual_path: None, path_hash64: None, id: Some(id), data: payload,
                }).unwrap();
            }
            let path = dir.join(name);
            std::fs::write(&path, builder.build().unwrap()).unwrap();
            path
        };
        let payload = vec![42; 1024];
        let reference = write("reference.uddp", CompressionFlag::None, &payload, DataType::Metadata as u8, false);
        for (name, compression, data, data_type, extra, expected) in [
            ("recompressed", CompressionFlag::ZstdNoDict, payload.as_slice(), DataType::Metadata as u8, false, true),
            ("payload", CompressionFlag::None, b"changed".as_slice(), DataType::Metadata as u8, false, false),
            ("keys", CompressionFlag::None, payload.as_slice(), DataType::Metadata as u8, true, false),
            ("type", CompressionFlag::None, payload.as_slice(), DataType::Texture as u8, false, false),
        ] {
            let candidate = write(name, compression, data, data_type, extra);
            let result = run(Arguments { reference: reference.clone(), candidate, repetitions: NonZeroUsize::new(1).unwrap() });
            assert_eq!(result.is_ok(), expected, "{name}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
