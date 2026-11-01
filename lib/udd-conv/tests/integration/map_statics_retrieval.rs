//! Runtime retrieval speed of map and statics chunks by payload codec.
//!
//! The ignored test repacks the entries of an installed `map0.uddp` and `statics0.uddp` with every
//! codec the builder offers for them: stored, plain Zstd at several levels, and Zstd with the type
//! dictionary. It then reads every chunk back the way dynamapper does
//! (`UddpReader::load`, which maps the file, and `read_file_by_dense_id`, with the decoded-entry
//! cache off), and also from an in-memory reader and through the borrowing `_cow` read, in id
//! order, in a shuffled order and for a 24x24-chunk region around the map centre. Warm
//! passes run with the file in the page cache and report the time per chunk; cold passes first drop
//! the file's pages (`posix_fadvise(DONTNEED)`), so every page is read from disk again, and report
//! the time to open the package and read every chunk once.
//!
//! Run with `--release`:
//! `UDD_PACKAGE_DIR=/home/claudio/test/uddp TMPDIR=<scratch> cargo test --release -p udd-conv --test integration map_statics_retrieval:: -- --ignored --nocapture`.
//! The repacked variants are written under `TMPDIR` and removed at the end.

use std::{
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder, UddpReader};

const PACKAGE_DIR_ENV: &str = "UDD_PACKAGE_DIR";
const PACKAGES: [&str; 2] = ["map0.uddp", "statics0.uddp"];
/// Plain Zstd levels to compare. 7 is the builder's default level (`ZstdNoDict`, `ZstdDict`).
const ZSTD_LEVELS: [i32; 4] = [1, 3, 7, 19];
/// Warm passes per read order; the reported time is the fastest pass.
const WARM_PASSES: usize = 5;
const RANDOM_ORDER_SEED: u64 = 0x2545_F491_4F6C_DD1D;
const NANOS_PER_MICRO: f64 = 1_000.0;
const MILLIS_PER_SECOND: f64 = 1_000.0;
const BYTES_PER_MB: f64 = 1_000_000.0;

/// One chunk of the installed package, decoded.
struct Entry {
    data_type: u8,
    raw: Vec<u8>,
}

struct Variant {
    label: String,
    compression: CompressionFlag,
}

fn variants() -> Vec<Variant> {
    let mut variants = vec![Variant { label: "stored".into(), compression: CompressionFlag::None }];
    variants.extend(ZSTD_LEVELS.iter().map(|&level| Variant {
        label: format!("zstd {level}"),
        compression: CompressionFlag::ZstdNoDictLevel(level),
    }));
    variants.push(Variant { label: "zstd 7 + dict".into(), compression: CompressionFlag::ZstdDict });
    variants
}

fn read_entries(path: &Path) -> Vec<Entry> {
    let reader = UddpReader::load(path).unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
    (0..reader.header().file_count)
        .map(|id| {
            let resolved = reader.find_by_dense_id(id).expect("dense id in range");
            let raw = reader.read_file_by_dense_id(id).expect("decode installed entry");
            Entry { data_type: resolved.data_type, raw }
        })
        .collect()
}

fn map_height_chunks(map_path: &Path) -> u32 {
    let reader = UddpReader::load(map_path).expect("open map package");
    let metadata_id = reader.header().file_count - 1;
    let metadata = udd_assets::map_metadata::decode_map_package_metadata(
        &reader.read_file_by_dense_id(metadata_id).expect("read map metadata"),
    )
    .expect("decode map metadata");
    metadata.height_tiles.div_ceil(CHUNK_TILE_DIM)
}

/// Repacks the entries with one codec. Metadata and empty chunks stay stored, as the packers do.
fn repack(entries: &[Entry], compression: CompressionFlag, path: &Path) {
    let mut builder = UddpBuilder::new(LookupMode::DenseId);
    for (id, entry) in entries.iter().enumerate() {
        let is_chunk = entry.data_type != DataType::Metadata as u8 && !entry.raw.is_empty();
        builder
            .add_file(AddFileRequest {
                data_type: entry.data_type,
                compression: if is_chunk { compression } else { CompressionFlag::None },
                width: 0,
                height: 0,
                virtual_path: None,
                path_hash64: None,
                id: Some(id as u32),
                data: &entry.raw,
            })
            .expect("add entry");
    }
    builder.into_file(path).expect("write variant");
}

/// Where the region read order sits: a square of chunks around the map centre, read row by row as a
/// view would need them after a teleport. Chunk ids run down columns (`x * height_chunks + y`), so a
/// row crosses columns.
const REGION_CHUNKS: u32 = 24;
const CHUNK_TILE_DIM: u32 = 32;

/// The chunk ids (metadata excluded) in id order, in a fixed shuffled order, and for one region.
fn read_orders(entries: &[Entry], height_chunks: u32) -> [(&'static str, Vec<u32>); 3] {
    let sequential: Vec<u32> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.data_type != DataType::Metadata as u8)
        .map(|(id, _)| id as u32)
        .collect();
    let mut shuffled = sequential.clone();
    let mut state = RANDOM_ORDER_SEED;
    for index in (1..shuffled.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        shuffled.swap(index, (state % (index as u64 + 1)) as usize);
    }
    let width_chunks = sequential.len() as u32 / height_chunks;
    let (origin_x, origin_y) = ((width_chunks - REGION_CHUNKS) / 2, (height_chunks - REGION_CHUNKS) / 2);
    let region = (origin_y..origin_y + REGION_CHUNKS)
        .flat_map(|y| (origin_x..origin_x + REGION_CHUNKS).map(move |x| x * height_chunks + y))
        .collect();
    [("sequential", sequential), ("random", shuffled), ("region", region)]
}

/// Writes the file's dirty pages back and drops its pages from the page cache, so the next read
/// comes from disk. No reader may map the file at that point, or its pages stay resident.
fn evict_from_page_cache(path: &Path) {
    use std::os::fd::AsRawFd;
    let file = fs::File::open(path).expect("open for eviction");
    file.sync_all().expect("write back before eviction");
    // SAFETY: a valid open descriptor; offset 0 with length 0 covers the whole file.
    let result = unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
    assert_eq!(result, 0, "posix_fadvise(DONTNEED) failed for {}", path.display());
}

/// One cold pass: the file is evicted, then mapped (or read whole, for the in-memory reader, whose
/// load time is included) and every id read once.
fn cold_pass(path: &Path, ids: &[u32], in_memory: bool) -> Duration {
    evict_from_page_cache(path);
    let start = Instant::now();
    let reader = if in_memory { UddpReader::load_in_memory(path) } else { UddpReader::load(path) }
        .expect("open variant cold");
    for &id in ids {
        black_box(reader.read_file_by_dense_id(id).expect("read entry"));
    }
    start.elapsed()
}

fn time_pass(reader: &UddpReader, ids: &[u32], borrow: bool) -> Duration {
    let start = Instant::now();
    for &id in ids {
        if borrow {
            black_box(reader.read_file_by_dense_id_cow(id).expect("read entry"));
        } else {
            black_box(reader.read_file_by_dense_id(id).expect("read entry"));
        }
    }
    start.elapsed()
}

fn fastest_pass(reader: &UddpReader, ids: &[u32], borrow: bool) -> Duration {
    (0..WARM_PASSES).map(|_| time_pass(reader, ids, borrow)).min().expect("at least one pass")
}

fn micros_per_entry(elapsed: Duration, ids: &[u32]) -> f64 {
    elapsed.as_nanos() as f64 / NANOS_PER_MICRO / ids.len() as f64
}

fn report(label: &str, path: &Path, entries: &[Entry], height_chunks: u32) {
    let orders = read_orders(entries, height_chunks);
    let size_mb = fs::metadata(path).expect("variant size").len() as f64 / BYTES_PER_MB;
    let mut cells = vec![format!("{label:<13}"), format!("{size_mb:>7.2} MB")];

    // Cold passes first, while no reader maps the file.
    for (order, ids) in &orders {
        let mapped_cold = cold_pass(path, ids, false);
        let memory_cold = cold_pass(path, ids, true);
        cells.push(format!(
            "cold {order}: mmap {:.1} ms, in-memory {:.1} ms (open included)",
            mapped_cold.as_secs_f64() * MILLIS_PER_SECOND,
            memory_cold.as_secs_f64() * MILLIS_PER_SECOND,
        ));
    }

    let mapped = UddpReader::load(path).expect("map variant");
    for (id, entry) in entries.iter().enumerate() {
        assert_eq!(mapped.read_file_by_dense_id(id as u32).expect("read back"), entry.raw, "{label}: entry {id}");
    }
    let in_memory = UddpReader::load_in_memory(path).expect("read variant");
    for (order, ids) in &orders {
        let mapped_warm = micros_per_entry(fastest_pass(&mapped, ids, false), ids);
        let mapped_cow = micros_per_entry(fastest_pass(&mapped, ids, true), ids);
        let memory_warm = micros_per_entry(fastest_pass(&in_memory, ids, false), ids);
        cells.push(format!(
            "warm {order}: mmap {mapped_warm:.2} us (cow {mapped_cow:.2}), in-memory {memory_warm:.2} us"
        ));
    }
    println!("{}", cells.join(" | "));
}

#[test]
#[ignore = "needs packages: set UDD_PACKAGE_DIR to a folder with map0.uddp and statics0.uddp; run with --release"]
fn map_and_statics_chunks_by_codec() {
    let package_dir = PathBuf::from(std::env::var_os(PACKAGE_DIR_ENV).expect("set UDD_PACKAGE_DIR"));
    let work_dir = std::env::temp_dir().join(format!("udd_map_statics_retrieval_{}", std::process::id()));
    fs::create_dir_all(&work_dir).expect("create work dir");
    // Statics share the map's chunk grid.
    let height_chunks = map_height_chunks(&package_dir.join(PACKAGES[0]));

    for package in PACKAGES {
        let installed = package_dir.join(package);
        let entries = read_entries(&installed);
        println!("== {package}: {} entries", entries.len());
        report("installed", &installed, &entries, height_chunks);
        for variant in variants() {
            let path = work_dir.join(format!("{}_{}.uddp", package, variant.label.replace(' ', "_")));
            repack(&entries, variant.compression, &path);
            report(&variant.label, &path, &entries, height_chunks);
            fs::remove_file(&path).expect("remove variant");
        }
    }
    fs::remove_dir(&work_dir).expect("remove work dir");
}
