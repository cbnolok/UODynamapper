//! Lossless supercompression experiments on the BC7 pages of the mobile animation packages: how the pages would store
//! and decode under other entropy coders, levels, chunkings and dictionaries, and after reversible transforms of the
//! block stream. Measurement only: nothing here writes a package. The findings are in
//! `docs/agent_notes/bc7_anim_supercompression.md`.
//!
//! - `audit <package> <cc|ec>`: the entries by kind and codec, the page extents and how many pages a clip reads.
//! - `blocks <package> <cc|ec>`: BC7 modes, fully transparent blocks and their encodings, repeated blocks.
//! - `sizes <package> <cc|ec> --methods ...`: every page through each method on the whole pool, each round trip
//!   checked byte for byte; total bytes against the stored pages.
//! - `timing <package> <cc|ec> --methods ...`: the decode of sampled pages on one thread (per-page percentiles) and on
//!   the background pool's thread count (pages a second).
//!
//! - `classes <package> <cc|ec> --bounds ...`: the bodies by size class (their largest frame side), each class's share
//!   of the pages; `--check-every N` replays the packer's layout (`repack.rs`) against the package's.
//! - `bodies <package> <cc|ec> --bodies ... --methods ...`: each body's frames on pages of their own, as the packer
//!   would lay them out, and its clips read from them on one thread (whole-page inverse, or per clip for `fields`).
//! - `crowd <package> <cc|ec> --config name=spec ...`: every page through each method on the game's worker count, then
//!   crowds appearing under the loader the user chose, with and without a cache of decoded pages; `--layout classes`
//!   puts each size class on pages of its own, and a spec can name a method per class (see `study.rs`).
//! - `lossy <rgba package> <bc7 package> <cc|ec> --variants ...`: lossy conditioning of the pages against the source
//!   frames (the library RDO, coder-aware block and field reuse, temporal block reuse on an anchor-aligned layout,
//!   endpoint coarsening, flat blocks), its size under each method and its quality (see `lossy.rs`).
//!
//! A method is `stored` (the page as the package holds it, read through the container) or
//! `<transform>+<backend>`, e.g. `none+zstd:9`, `split1+lz4hc:12`, `fields-shuf+zstd:19` (see `transforms.rs` and
//! `backends.rs` for the names).

mod backends;
mod bc7block;
mod corpus;
mod lossy;
mod repack;
mod study;
mod transforms;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{self, WrapErr};
use rayon::prelude::*;
use udd_assets::bc7::{ImageExtent, decode_bc7_to_rgba8888};

use backends::{Backend, Dictionary, Encoded};
use corpus::{Kind, Package, Page};
use transforms::{BLOCK_BYTES, MODE_SLOTS, Transform, block_mode};

const BYTES_PER_MIB: f64 = (1u64 << 20) as f64;
const BYTES_PER_MB: f64 = 1_000_000.0;
const MS_PER_S: f64 = 1000.0;
const PERCENT: f64 = 100.0;
const PERCENTILES: [f64; 3] = [50.0, 95.0, 99.0];
const RGBA_BYTES: usize = 4;
const ALPHA: usize = 3;
const BLOCK_EDGE: usize = 4;
const BLOCK_TEXELS: usize = BLOCK_EDGE * BLOCK_EDGE;
const KIB: usize = 1024;
/// The background pool's most workers (`dynamapper/src/app/background.rs`, `WORKERS_MAX`): the threads that decode
/// clips in the game.
const GAME_DECODE_THREADS: usize = 4;
/// The bytes of a transformed stream's length when a transform changes it, which a stored payload would need beside
/// the container's raw size.
const TRANSFORMED_LENGTH_BYTES: usize = 4;

#[derive(Parser)]
#[command(about = "Lossless supercompression experiments on BC7 mobile animation pages")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Audit(Target),
    Blocks(BlocksArgs),
    Sizes(MethodArgs),
    Timing(TimingArgs),
    Classes(study::ClassesArgs),
    Bodies(study::BodiesArgs),
    Crowd(study::CrowdArgs),
    Lossy(lossy::LossyArgs),
}

#[derive(Args)]
struct Target {
    package: PathBuf,
    #[arg(value_enum)]
    kind: Kind,
}

#[derive(Args)]
struct BlocksArgs {
    #[command(flatten)]
    target: Target,
    /// Every n-th page.
    #[arg(long, default_value_t = 1)]
    every: usize,
}

#[derive(Args)]
struct MethodArgs {
    #[command(flatten)]
    target: Target,
    /// Comma-separated methods.
    #[arg(long, value_delimiter = ',', required = true)]
    methods: Vec<String>,
    /// Every n-th page (of the held-out pages when a method has a dictionary).
    #[arg(long, default_value_t = 1)]
    every: usize,
    /// Pages compressed together as one payload (consecutive pages).
    #[arg(long, default_value_t = 1)]
    group: usize,
    /// Size of the trained dictionary in KiB; dictionary methods train on the even pages and are measured on the odd.
    #[arg(long, default_value_t = 112)]
    dict_kib: usize,
    /// Training pages: every n-th even page.
    #[arg(long, default_value_t = 4)]
    dict_train_every: usize,
    /// Per-page CSV.
    #[arg(long)]
    csv: Option<PathBuf>,
    /// Writes the trained dictionaries and the first measured pages' blocks here, to check against the zstd tool.
    #[arg(long)]
    dump: Option<PathBuf>,
    /// Measured pages `--dump` writes.
    #[arg(long, default_value_t = 3)]
    dump_pages: usize,
}

#[derive(Args)]
struct TimingArgs {
    #[command(flatten)]
    methods: MethodArgs,
    /// Decodes of each page per method; the fastest counts.
    #[arg(long, default_value_t = 5)]
    reps: usize,
    /// Passes over the sample on the background pool's thread count, for pages a second.
    #[arg(long, default_value_t = 3)]
    parallel_passes: usize,
}

/// A way to store a page.
#[derive(Clone, Copy)]
enum Method {
    Stored,
    Coded { transform: Transform, backend: Backend },
}

impl Method {
    fn parse(text: &str) -> eyre::Result<Self> {
        if text == "stored" {
            return Ok(Self::Stored);
        }
        let (transform, backend) = text.split_once('+').ok_or_else(|| eyre::eyre!("method {text}: no '+'"))?;
        Ok(Self::Coded {
            transform: Transform::parse(transform).ok_or_else(|| eyre::eyre!("unknown transform {transform}"))?,
            backend: Backend::parse(backend).ok_or_else(|| eyre::eyre!("unknown backend {backend}"))?,
        })
    }

    fn name(self) -> String {
        match self {
            Self::Stored => "stored".into(),
            Self::Coded { transform, backend } => format!("{}+{backend}", transform.name()),
        }
    }

    fn dictionary_transform(self) -> Option<Transform> {
        match self {
            Self::Coded { transform, backend } if backend.needs_dictionary() => Some(transform),
            _ => None,
        }
    }
}

/// A page (or group of pages) stored by a method, ready to decode.
struct StoredPage {
    raw_len: usize,
    transform: Transform,
    /// The transformed stream's length: what the length field counted in `bytes` holds when the transform changes it.
    transformed_len: usize,
    /// Blocks per row of the page, which the reader knows from the page record.
    blocks_wide: usize,
    encoded: Encoded,
    /// Bytes counted for the page: the payload plus any transformed-length field.
    bytes: usize,
}

fn store(
    transform: Transform,
    backend: Backend,
    raw: &[u8],
    blocks_wide: usize,
    dictionary: Option<&Dictionary>,
    scratch: &mut Vec<u8>,
) -> eyre::Result<StoredPage> {
    transform.forward(raw, blocks_wide, scratch).map_err(|error| eyre::eyre!("{error}"))?;
    let encoded = backends::encode(backend, scratch, dictionary).map_err(|error| eyre::eyre!("{error}"))?;
    let extra = if transform.keeps_length() { 0 } else { TRANSFORMED_LENGTH_BYTES };
    Ok(StoredPage {
        raw_len: raw.len(),
        blocks_wide,
        transform,
        transformed_len: scratch.len(),
        bytes: encoded.bytes.len() + extra,
        encoded,
    })
}

thread_local! {
    /// The coder's output when a transform follows it, kept between pages as a reader would.
    static TRANSFORMED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Decodes and untransforms `stored`; returns the page and the two times. The page is a new allocation either way, as
/// the game's reader hands each page on; a transformed stream is decoded into a buffer kept between pages.
fn load(stored: &StoredPage, dictionary: Option<&Dictionary>) -> eyre::Result<(Vec<u8>, Duration, Duration)> {
    if stored.transform == Transform::None {
        let start = Instant::now();
        let page = backends::decode(&stored.encoded, stored.transformed_len, dictionary)
            .map_err(|error| eyre::eyre!("{error}"))?;
        return Ok((page, start.elapsed(), Duration::ZERO));
    }
    TRANSFORMED.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        let start = Instant::now();
        backends::decode_into(&stored.encoded, stored.transformed_len, dictionary, &mut scratch)
            .map_err(|error| eyre::eyre!("{error}"))?;
        let decompressed = start.elapsed();
        let start = Instant::now();
        let mut page = Vec::with_capacity(stored.raw_len);
        stored.transform.inverse(&scratch, stored.raw_len, stored.blocks_wide, &mut page)
            .map_err(|error| eyre::eyre!("{error}"))?;
        Ok((page, decompressed, start.elapsed()))
    })
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = ((p / PERCENT) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / BYTES_PER_MIB
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    match Cli::parse().command {
        Command::Audit(target) => audit(&target),
        Command::Blocks(args) => blocks(&args),
        Command::Sizes(args) => sizes(&args),
        Command::Timing(args) => timing(&args),
        Command::Classes(args) => study::classes(&args),
        Command::Bodies(args) => study::bodies(&args),
        Command::Crowd(args) => study::crowd_study(&args),
        Command::Lossy(args) => lossy::lossy(&args),
    }
}

fn audit(target: &Target) -> eyre::Result<()> {
    let package = Package::open(&target.package, target.kind)?;
    let file_bytes = package.reader.package_size_bytes() as u64;
    println!("package {} bytes ({:.2} MiB), {} BC7 pages", file_bytes, mib(file_bytes), package.pages.len());
    println!("\n| Entries | Codec | Count | Raw MiB | Stored MiB | Share of file |\n|---|---|---:|---:|---:|---:|");
    let mut stored_total = 0u64;
    for ((kind, codec), (count, raw, stored)) in package.entry_summary() {
        stored_total += stored;
        println!("| {kind} | {codec} | {count} | {:.2} | {:.2} | {:.2} % |", mib(raw), mib(stored),
            stored as f64 * PERCENT / file_bytes as f64);
    }
    println!("| container (header, index, dictionaries) | | | | {:.2} | |", mib(file_bytes - stored_total));

    let mut extents: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for page in &package.pages {
        *extents.entry((page.used_width, page.used_height)).or_default() += 1;
    }
    let mut by_count: Vec<_> = extents.into_iter().collect();
    by_count.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let shown: Vec<String> = by_count.iter().take(6).map(|((w, h), n)| format!("{w}x{h}: {n}")).collect();
    let atlas: std::collections::BTreeSet<(u32, u32)> =
        package.pages.iter().map(|page| (page.atlas_width, page.atlas_height)).collect();
    println!("\natlas extents {atlas:?}; used extents (most common) {}", shown.join(", "));
    let raw: Vec<f64> = package.pages.iter().map(|page| f64::from(page.entry.raw_size)).collect();
    let stored: Vec<f64> = package.pages.iter().map(|page| f64::from(page.entry.stored_size)).collect();
    for (name, values) in [("raw", raw), ("stored", stored)] {
        let mut sorted = values;
        sorted.sort_by(f64::total_cmp);
        println!("page {name} KiB: min {:.0}, p50 {:.0}, p95 {:.0}, max {:.0}", sorted[0] / KIB as f64,
            percentile(&sorted, 50.0) / KIB as f64, percentile(&sorted, 95.0) / KIB as f64,
            sorted[sorted.len() - 1] / KIB as f64);
    }
    let frames: u64 = package.pages.iter().map(|page| u64::from(page.frames)).sum();
    println!("frames {frames}, {:.1} per page", frames as f64 / package.pages.len() as f64);
    let clips = &package.clip_pages;
    let mut per_page: Vec<f64> = clips.clips_per_page.values().map(|&n| n as f64).collect();
    per_page.sort_by(f64::total_cmp);
    println!("clips {} (body, action, direction): {} page decodes to load each once, {:.2} pages per clip, at most {}; \
        clips per page p50 {:.0}, p95 {:.0}, max {:.0}", clips.clips, clips.page_reads,
        clips.page_reads as f64 / clips.clips as f64, clips.most_pages, percentile(&per_page, 50.0),
        percentile(&per_page, 95.0), per_page.last().copied().unwrap_or(0.0));
    Ok(())
}

#[derive(Default)]
struct BlockTally {
    blocks: u64,
    modes: [u64; MODE_SLOTS],
    opaque_modes: [u64; MODE_SLOTS],
    transparent: u64,
    transparent_encodings: HashMap<[u8; BLOCK_BYTES], u64>,
    repeated_in_page: u64,
    repeated_in_earlier_page: u64,
    non_transparent: u64,
}

fn blocks(args: &BlocksArgs) -> eyre::Result<()> {
    let package = Package::open(&args.target.package, args.target.kind)?;
    let sample = package.sample(args.every, 0);
    // Per page: the tally and the set of its visible blocks, in page order for the cross-page count.
    let per_page: Vec<(BlockTally, HashSet<[u8; BLOCK_BYTES]>)> = sample.par_iter().map(|page| {
        let raw = package.raw(page)?;
        let blocks_total = raw.len() / BLOCK_BYTES;
        let (width, height) = if blocks_total as u32 == page.used_width.div_ceil(4) * page.used_height.div_ceil(4) {
            (page.used_width.next_multiple_of(4), page.used_height.next_multiple_of(4))
        } else {
            (page.atlas_width.next_multiple_of(4), page.atlas_height.next_multiple_of(4))
        };
        let extent = ImageExtent::new(width, height).map_err(|error| eyre::eyre!("{error}"))?;
        let rgba = decode_bc7_to_rgba8888(&raw, extent).map_err(|error| eyre::eyre!("{error}"))?;
        let blocks_wide = width as usize / BLOCK_EDGE;
        let mut tally = BlockTally::default();
        let mut seen = HashSet::new();
        for (index, block) in raw.chunks_exact(BLOCK_BYTES).enumerate() {
            let (bx, by) = (index % blocks_wide, index / blocks_wide);
            let transparent = (0..BLOCK_TEXELS).all(|texel| {
                let (x, y) = (bx * BLOCK_EDGE + texel % BLOCK_EDGE, by * BLOCK_EDGE + texel / BLOCK_EDGE);
                rgba[(y * width as usize + x) * RGBA_BYTES + ALPHA] == 0
            });
            let mode = block_mode(block) as usize;
            let key: [u8; BLOCK_BYTES] = block.try_into().expect("a block");
            tally.blocks += 1;
            tally.modes[mode] += 1;
            if transparent {
                tally.transparent += 1;
                *tally.transparent_encodings.entry(key).or_default() += 1;
            } else {
                tally.non_transparent += 1;
                tally.opaque_modes[mode] += 1;
                if !seen.insert(key) {
                    tally.repeated_in_page += 1;
                }
            }
        }
        Ok((tally, seen))
    }).collect::<eyre::Result<_>>()?;

    let mut total = BlockTally::default();
    let mut earlier: HashSet<[u8; BLOCK_BYTES]> = HashSet::new();
    for (tally, seen) in per_page {
        total.blocks += tally.blocks;
        total.transparent += tally.transparent;
        total.non_transparent += tally.non_transparent;
        total.repeated_in_page += tally.repeated_in_page;
        for mode in 0..MODE_SLOTS {
            total.modes[mode] += tally.modes[mode];
            total.opaque_modes[mode] += tally.opaque_modes[mode];
        }
        for (key, count) in tally.transparent_encodings {
            *total.transparent_encodings.entry(key).or_default() += count;
        }
        total.repeated_in_earlier_page += seen.iter().filter(|key| earlier.contains(*key)).count() as u64;
        earlier.extend(seen);
    }
    let share = |n: u64, of: u64| n as f64 * PERCENT / of.max(1) as f64;
    println!("{} pages, {} blocks; fully transparent {:.2} %", sample.len(), total.blocks,
        share(total.transparent, total.blocks));
    let mut modes = String::from("| Mode | All blocks | Not fully transparent |\n|---|---:|---:|\n");
    for mode in 0..MODE_SLOTS {
        let label = if mode == transforms::RESERVED_MODE as usize { "reserved".to_string() } else { mode.to_string() };
        writeln!(modes, "| {label} | {:.2} % | {:.2} % |", share(total.modes[mode], total.blocks),
            share(total.opaque_modes[mode], total.non_transparent))?;
    }
    println!("{modes}");
    let mut encodings: Vec<_> = total.transparent_encodings.iter().collect();
    encodings.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    println!("transparent encodings: {} distinct; the most common covers {:.4} % of them ({:02x?})", encodings.len(),
        share(encodings.first().map_or(0, |(_, n)| **n), total.transparent),
        encodings.first().map(|(key, _)| **key));
    println!("visible blocks repeating one earlier in their page: {:.2} %; distinct visible blocks of a page found in an \
        earlier sampled page: {:.2} % of the visible blocks", share(total.repeated_in_page, total.non_transparent),
        share(total.repeated_in_earlier_page, total.non_transparent));
    Ok(())
}

/// The pages a run measures: every `every`-th, of the odd pages when a method needs a dictionary trained on the even.
fn measured_pages<'a>(package: &'a Package, args: &MethodArgs, methods: &[Method]) -> Vec<&'a Page> {
    let held_out = methods.iter().any(|method| method.dictionary_transform().is_some());
    if held_out {
        package.pages.iter().filter(|page| page.index % 2 == 1).step_by(args.every.max(1)).collect()
    } else {
        package.sample(args.every, 0)
    }
}

/// A dictionary per transform that a method needs one for, trained on the even pages in that transform's form.
fn dictionaries(package: &Package, args: &MethodArgs, methods: &[Method])
    -> eyre::Result<HashMap<String, Arc<Dictionary>>> {
    let mut out = HashMap::new();
    const SAMPLE_CHUNK: usize = 64 * KIB;
    const SAMPLE_EVERY: usize = 4;
    for transform in methods.iter().filter_map(|method| method.dictionary_transform()) {
        if out.contains_key(&transform.name()) {
            continue;
        }
        let training: Vec<&Page> =
            package.pages.iter().filter(|page| page.index % 2 == 0).step_by(args.dict_train_every.max(1)).collect();
        let pages: Vec<Arc<Vec<u8>>> = training.par_iter().map(|page| {
            let raw = package.raw(page)?;
            let mut transformed = Vec::new();
            transform.forward(&raw, page.blocks_wide(raw.len()), &mut transformed).map_err(|error| eyre::eyre!("{error}"))?;
            Ok(Arc::new(transformed))
        }).collect::<eyre::Result<_>>()?;
        let start = Instant::now();
        let dictionary = backends::train_dictionary(&pages, SAMPLE_CHUNK, SAMPLE_EVERY, args.dict_kib * KIB)
            .map_err(|error| eyre::eyre!("train: {error}"))?;
        eprintln!("dictionary for {}: {} bytes from {} pages in {:.1} s", transform.name(), dictionary.bytes.len(),
            pages.len(), start.elapsed().as_secs_f64());
        out.insert(transform.name(), Arc::new(dictionary));
    }
    Ok(out)
}

fn sizes(args: &MethodArgs) -> eyre::Result<()> {
    let methods: Vec<Method> = args.methods.iter().map(|name| Method::parse(name)).collect::<eyre::Result<_>>()?;
    let package = Package::open(&args.target.package, args.target.kind)?;
    let pages = measured_pages(&package, args, &methods);
    let dictionaries = dictionaries(&package, args, &methods)?;
    if let Some(dir) = &args.dump {
        std::fs::create_dir_all(dir)?;
        for (name, dictionary) in &dictionaries {
            std::fs::write(dir.join(format!("dictionary_{name}.bin")), &dictionary.bytes)?;
        }
        for page in pages.iter().take(args.dump_pages) {
            std::fs::write(dir.join(format!("page_{:05}.bc7", page.index)), package.raw(page)?)?;
        }
    }
    let group = args.group.max(1);
    let groups: Vec<&[&Page]> = pages.chunks(group).collect();
    let encode_nanos: Vec<AtomicU64> = methods.iter().map(|_| AtomicU64::new(0)).collect();
    let lz4_choices: Vec<AtomicU64> = methods.iter().map(|_| AtomicU64::new(0)).collect();
    let start = Instant::now();
    // Per group: per method the bytes, beside the stored pages' bytes and raw bytes.
    let rows: Vec<(Vec<u32>, u64, u64, Vec<u64>)> = groups.par_iter().map(|group_pages| {
        let mut raw = Vec::new();
        let mut blocks_wide = 0;
        for page in group_pages.iter() {
            let page_raw = package.raw(page)?;
            if raw.is_empty() {
                // Grouped pages are all the atlas's width: the first one's grid serves the group.
                blocks_wide = page.blocks_wide(page_raw.len());
            }
            raw.extend_from_slice(&page_raw);
        }
        let stored_bytes: u64 = group_pages.iter().map(|page| u64::from(page.entry.stored_size)).sum();
        let mut scratch = Vec::new();
        let mut per_method = Vec::with_capacity(methods.len());
        for (slot, method) in methods.iter().enumerate() {
            let Method::Coded { transform, backend } = *method else {
                per_method.push(stored_bytes);
                continue;
            };
            let dictionary = method.dictionary_transform().map(|t| dictionaries[&t.name()].as_ref());
            let begin = Instant::now();
            let stored = store(transform, backend, &raw, blocks_wide, dictionary, &mut scratch)
                .wrap_err_with(|| format!("{} on page {}", method.name(), group_pages[0].index))?;
            encode_nanos[slot].fetch_add(begin.elapsed().as_nanos() as u64, Ordering::Relaxed);
            if matches!(stored.encoded.codec, backends::StoredCodec::Lz4 { .. }) {
                lz4_choices[slot].fetch_add(1, Ordering::Relaxed);
            }
            let (back, _, _) = load(&stored, dictionary)?;
            eyre::ensure!(back == raw, "{} did not round-trip page {}", method.name(), group_pages[0].index);
            per_method.push(stored.bytes as u64);
        }
        Ok((group_pages.iter().map(|page| page.index).collect(), raw.len() as u64, stored_bytes, per_method))
    }).collect::<eyre::Result<_>>()?;

    let raw_total: u64 = rows.iter().map(|row| row.1).sum();
    let stored_total: u64 = rows.iter().map(|row| row.2).sum();
    println!("{} pages ({} per payload), raw {:.2} MiB, stored {:.2} MiB, {:.1} s wall; every round trip byte-exact",
        pages.len(), group, mib(raw_total), mib(stored_total), start.elapsed().as_secs_f64());
    println!("\n| Method | MiB | vs stored | Ratio | LZ4 payloads | Encode thread-s |\n|---|---:|---:|---:|---:|---:|");
    for (slot, method) in methods.iter().enumerate() {
        let mut total: u64 = rows.iter().map(|row| row.3[slot]).sum();
        let dictionary_note = match method.dictionary_transform() {
            Some(transform) => {
                let bytes = dictionaries[&transform.name()].bytes.len() as u64;
                total += bytes;
                format!(" (+{:.0} KiB dictionary)", bytes as f64 / KIB as f64)
            }
            None => String::new(),
        };
        println!("| {}{dictionary_note} | {:.2} | {:+.2} % | {:.3} | {} | {:.1} |", method.name(), mib(total),
            (total as f64 / stored_total as f64 - 1.0) * PERCENT, raw_total as f64 / total as f64,
            lz4_choices[slot].load(Ordering::Relaxed),
            encode_nanos[slot].load(Ordering::Relaxed) as f64 / 1e9);
    }
    if let Some(path) = &args.csv {
        let mut csv = String::from("pages,raw,stored");
        for method in &methods {
            write!(csv, ",{}", method.name())?;
        }
        csv.push('\n');
        for (indices, raw, stored, per_method) in &rows {
            let names: Vec<String> = indices.iter().map(u32::to_string).collect();
            write!(csv, "{},{raw},{stored}", names.join(" "))?;
            for bytes in per_method {
                write!(csv, ",{bytes}")?;
            }
            csv.push('\n');
        }
        std::fs::write(path, csv)?;
    }
    Ok(())
}

fn timing(args: &TimingArgs) -> eyre::Result<()> {
    let method_args = &args.methods;
    let methods: Vec<Method> =
        method_args.methods.iter().map(|name| Method::parse(name)).collect::<eyre::Result<_>>()?;
    let package = Package::open(&method_args.target.package, method_args.target.kind)?;
    let pages = measured_pages(&package, method_args, &methods);
    let dictionaries = dictionaries(&package, method_args, &methods)?;
    let reps = args.reps.max(1);
    println!("{} pages, {} decodes each, one thread; then {} passes on {GAME_DECODE_THREADS} threads",
        pages.len(), reps, args.parallel_passes);
    println!("\n| Method | MiB | Decode p50 / p95 / p99 ms | Of which inverse p50 / p99 ms | 1 thread MB/s | Median-of-reps \
        p50 ms | {GAME_DECODE_THREADS} threads pages/s (min–max) | MB/s |\n|---|---:|---:|---:|---:|---:|---:|---:|");
    let pool = rayon::ThreadPoolBuilder::new().num_threads(GAME_DECODE_THREADS).build()?;
    for method in &methods {
        let dictionary = method.dictionary_transform().map(|t| dictionaries[&t.name()].clone());
        let dictionary = dictionary.as_deref();
        // Encode the sample on the whole pool, then time on one thread with the pool idle.
        let stored: Vec<StoredPage> = pages.par_iter().map(|page| {
            let raw = package.raw(page)?;
            match *method {
                Method::Stored => Ok(StoredPage {
                    raw_len: raw.len(),
                    transform: Transform::None,
                    transformed_len: raw.len(),
                    blocks_wide: page.blocks_wide(raw.len()),
                    bytes: page.entry.stored_size as usize,
                    encoded: Encoded { bytes: Vec::new(), codec: backends::StoredCodec::Zstd },
                }),
                Method::Coded { transform, backend } => {
                    store(transform, backend, &raw, page.blocks_wide(raw.len()), dictionary, &mut Vec::new())
                }
            }
        }).collect::<eyre::Result<_>>()?;
        let decode_once = |index: usize| -> eyre::Result<(Duration, Duration)> {
            match method {
                Method::Stored => {
                    let start = Instant::now();
                    let page = package.raw(pages[index])?;
                    let elapsed = start.elapsed();
                    std::hint::black_box(page);
                    Ok((elapsed, Duration::ZERO))
                }
                Method::Coded { .. } => {
                    let (page, decompress, inverse) = load(&stored[index], dictionary)?;
                    std::hint::black_box(page);
                    Ok((decompress, inverse))
                }
            }
        };
        let mut best_total = Vec::with_capacity(pages.len());
        let mut best_inverse = Vec::with_capacity(pages.len());
        let mut median_total = Vec::with_capacity(pages.len());
        for index in 0..pages.len() {
            let mut runs: Vec<(f64, f64)> = (0..reps).map(|_| decode_once(index).map(|(decompress, inverse)| {
                ((decompress + inverse).as_secs_f64() * MS_PER_S, inverse.as_secs_f64() * MS_PER_S)
            })).collect::<eyre::Result<_>>()?;
            runs.sort_by(|a, b| a.0.total_cmp(&b.0));
            best_total.push(runs[0].0);
            best_inverse.push(runs[0].1);
            median_total.push(runs[runs.len() / 2].0);
        }
        let raw_total: usize = stored.iter().map(|page| page.raw_len).sum();
        let stored_total: usize = stored.iter().map(|page| page.bytes).sum();
        let single_ms: f64 = best_total.iter().sum();
        for values in [&mut best_total, &mut best_inverse, &mut median_total] {
            values.sort_by(f64::total_cmp);
        }
        let mut pass_seconds = Vec::new();
        for _ in 0..args.parallel_passes {
            let start = Instant::now();
            pool.install(|| (0..pages.len()).into_par_iter().try_for_each(|index| decode_once(index).map(|_| ())))?;
            pass_seconds.push(start.elapsed().as_secs_f64());
        }
        pass_seconds.sort_by(f64::total_cmp);
        let rate = |seconds: f64| pages.len() as f64 / seconds;
        let p = |values: &[f64]| PERCENTILES.map(|q| percentile(values, q));
        let [t50, t95, t99] = p(&best_total);
        let [i50, _, i99] = p(&best_inverse);
        println!("| {} | {:.2} | {t50:.2} / {t95:.2} / {t99:.2} | {i50:.2} / {i99:.2} | {:.0} | {:.2} | {:.0}–{:.0} | {:.0} |",
            method.name(), mib(stored_total as u64), raw_total as f64 / BYTES_PER_MB / (single_ms / MS_PER_S),
            percentile(&median_total, 50.0), rate(pass_seconds[pass_seconds.len() - 1]), rate(pass_seconds[0]),
            raw_total as f64 / BYTES_PER_MB / pass_seconds[0]);
    }
    Ok(())
}
