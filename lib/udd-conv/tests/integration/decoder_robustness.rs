//! Damaged inputs make the decoders fail, never panic. Release builds abort on a panic, so a panic on a bad entry
//! would end the program. Each decoder gets samples. It reads each sample cut at many lengths, then after rounds
//! of byte changes from a fixed-seed xorshift generator, and must return `Ok` or `Err`. When it returns `Ok`, the
//! accessors the renderer calls run on the result too.
//!
//! The synthetic samples run always. The shipped samples (Classic art, gump and animation RLE, AMOU, TGA, DDS and
//! `tilemeta`) are ignored tests. The roots come from `UO_CLIENT_ROOTS` (a path list,
//! `:`-separated on Unix); the installs are the folders named [`CLASSIC_FOLDER`] and [`ENHANCED_FOLDER`] in one of
//! them. The clients are only read; the packages built go to `std::env::temp_dir()`. Run with `--ignored --release`.
//! A failure names the decoder, the sample and the change, so it can be replayed.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use udd_container::{
    unpack_type, AddFileRequest, CompressionFlag, FileKey, LookupMode, UddpBuilder, UddpReader,
};

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const CLASSIC_FOLDER: &str = "_Ultima Online Classic";
const ENHANCED_FOLDER: &str = "_Ultima Online Enhanced";

/// Seed of the byte changes; each sample's generator starts from it and the sample's index.
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;
/// A sample shorter than this is cut at every length; a longer one at [`TRUNCATION_POINTS`] spread lengths and
/// at every length of its first [`HEADER_BYTES`].
const CUT_AT_EVERY_LENGTH_BELOW: usize = 1024;
const TRUNCATION_POINTS: usize = 192;
/// Rounds of byte changes per sample, and the most bytes one round changes.
const CHANGE_ROUNDS: usize = 256;
const MAX_CHANGES_PER_ROUND: usize = 4;
/// Half of the changes land in the first bytes, where the headers, counts and offsets are.
const HEADER_BYTES: usize = 64;
/// Shipped samples per decoder, spread over the entries found.
const SAMPLES_PER_DECODER: usize = 24;
/// Shipped samples larger than this are left out, to keep the run short.
const MAX_SAMPLE_BYTES: usize = 256 * 1024;
/// Failures listed in the assertion message.
const MAX_LISTED_FAILURES: usize = 16;

/// The 64-bit xorshift generator (Marsaglia, shifts 13, 7, 17).
struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        // A zero state stays zero.
        Self(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A value in `0..bound`; `bound` is not 0.
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

struct Sample {
    label: String,
    bytes: Vec<u8>,
}

impl Sample {
    fn new(label: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self { label: label.into(), bytes }
    }
}

/// The lengths a sample is cut at.
fn truncation_lengths(len: usize) -> Vec<usize> {
    if len < CUT_AT_EVERY_LENGTH_BELOW {
        return (0..len).collect();
    }
    let mut lengths: Vec<usize> = (0..HEADER_BYTES).collect();
    lengths.extend((0..TRUNCATION_POINTS).map(|point| len * point / TRUNCATION_POINTS));
    lengths.push(len - 1);
    lengths.sort_unstable();
    lengths.dedup();
    lengths
}

/// One round of byte changes: (position, new value) pairs.
fn change_round(rng: &mut XorShift64, len: usize) -> Vec<(usize, u8)> {
    let count = 1 + rng.below(MAX_CHANGES_PER_ROUND);
    (0..count)
        .map(|_| {
            let position = if rng.next() & 1 == 0 { rng.below(len.min(HEADER_BYTES)) } else { rng.below(len) };
            let value = match rng.below(4) {
                0 => 0x00,
                1 => 0xFF,
                2 => 1u8 << rng.below(u8::BITS as usize),
                _ => rng.next() as u8,
            };
            (position, value)
        })
        .collect()
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic without a message".to_owned())
}

/// Feeds every sample, cut and changed, to `decode`; returns the inputs that panicked.
fn panics(decoder: &str, samples: &[Sample], decode: &(dyn Fn(&[u8]) + Sync)) -> Vec<String> {
    samples
        .par_iter()
        .enumerate()
        .flat_map_iter(|(index, sample)| {
            let mut found = Vec::new();
            let mut run = |change: String, input: &[u8]| {
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| decode(input))) {
                    found.push(format!("{decoder} {} ({change}): {}", sample.label, panic_message(payload.as_ref())));
                }
            };
            run("unchanged".to_owned(), &sample.bytes);
            let len = sample.bytes.len();
            for cut in truncation_lengths(len) {
                run(format!("cut to {cut} of {len} bytes"), &sample.bytes[..cut]);
            }
            if len > 0 {
                let mut rng = XorShift64::new(SEED ^ index as u64);
                let mut changed = sample.bytes.clone();
                for round in 0..CHANGE_ROUNDS {
                    let changes = change_round(&mut rng, len);
                    for &(position, value) in &changes {
                        changed[position] = value;
                    }
                    run(format!("round {round}, bytes set {changes:?}"), &changed);
                    for &(position, _) in &changes {
                        changed[position] = sample.bytes[position];
                    }
                }
            }
            found
        })
        .collect()
}

fn assert_no_panics(decoder: &str, samples: &[Sample], decode: &(dyn Fn(&[u8]) + Sync)) {
    assert!(!samples.is_empty(), "{decoder}: no samples");
    let found = panics(decoder, samples, decode);
    println!("{decoder}: {} samples, {} panics", samples.len(), found.len());
    assert!(
        found.is_empty(),
        "{} damaged inputs made a decoder panic: {:#?}",
        found.len(),
        &found[..found.len().min(MAX_LISTED_FAILURES)]
    );
}

/// `count` items spread evenly over `items`.
fn spread<T>(items: Vec<T>, count: usize) -> Vec<T> {
    let step = items.len().div_ceil(count).max(1);
    items.into_iter().step_by(step).collect()
}

// --- Decoders, as the readers call them -------------------------------------------------------------------------

fn decode_land_art(bytes: &[u8]) {
    let mut pixels = [0u8; uocf::classic::art::LAND_DIMENSION
        * uocf::classic::art::LAND_DIMENSION
        * uocf::classic::art::RGBA_BYTES_PER_PIXEL];
    let _ = uocf::classic::art::decode_land_tile_from_raw(bytes, &mut pixels);
}

fn decode_static_art(bytes: &[u8]) {
    let _ = uocf::classic::art::decode_static_tile_from_raw(bytes);
}

/// A Classic gump payload with the size its index gives, in a 4-byte prefix (width, then height).
fn decode_gump(bytes: &[u8]) {
    let Some((size, raw)) = bytes.split_first_chunk::<GUMP_SIZE_PREFIX_BYTES>() else { return };
    let width = u16::from_le_bytes([size[0], size[1]]);
    let height = u16::from_le_bytes([size[2], size[3]]);
    let _ = uocf::classic::gump::decode_gump_from_raw(raw, width, height);
}
const GUMP_SIZE_PREFIX_BYTES: usize = 4;

fn gump_sample(label: String, width: u16, height: u16, raw: &[u8]) -> Sample {
    let mut bytes = Vec::with_capacity(GUMP_SIZE_PREFIX_BYTES + raw.len());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(raw);
    Sample::new(label, bytes)
}

fn decode_uop_gump(bytes: &[u8]) {
    let _ = uocf::classic::gump::decode_uop_gump(bytes);
}

fn decode_classic_mul_animation(bytes: &[u8]) {
    if let Ok(entry) = uocf::classic::anim_frames::mul::ClassicMulEntry::parse(bytes) {
        for frame in entry.frames.iter().flatten() {
            let _ = frame.to_rgba(&entry.palette);
        }
    }
}

fn decode_classic_uop_animation(bytes: &[u8]) {
    use uocf::classic::animationframe_cc::AnimationFrameCc;
    let _ = uocf::classic::anim_frames::uop::ClassicUopEntry::parse(bytes);
    let _ = AnimationFrameCc::parse(bytes);
    let _ = AnimationFrameCc::parse_metadata(bytes);
    for direction in 0..CLASSIC_DIRECTIONS {
        let _ = AnimationFrameCc::decode_direction(bytes, direction);
    }
}
/// Directions a Classic animation stores (the other three are mirrored).
const CLASSIC_DIRECTIONS: u8 = 5;

fn decode_amou(bytes: &[u8]) {
    use uocf::enhanced::animationframe::{AmouFile, AnimationFrame};
    let _ = AmouFile::parse(bytes);
    let _ = AnimationFrame::load_metadata(bytes);
    if let Ok(animation) = AnimationFrame::load(bytes) {
        for frame in &animation.frames {
            let _ = animation.decode_frame(frame);
            let _ = animation.decode_frame_indexed(frame);
        }
    }
}

fn decode_dds(bytes: &[u8]) {
    let _ = uocf::enhanced::textures::decode_dds_rgba8(bytes);
}

fn decode_tga(bytes: &[u8]) {
    let _ = uocf::enhanced::textures::decode_tga_rgba8(bytes);
}

/// The entries of a `tilemeta.uddp`, one of which is replaced by the damaged bytes.
struct TileMetaEntries {
    entries: Vec<(u64, u8, Vec<u8>)>,
}

impl TileMetaEntries {
    fn read(reader: &UddpReader) -> Self {
        let entries = reader
            .records()
            .into_iter()
            .map(|record| {
                let FileKey::PathHash(hash) = record.key else { panic!("tilemeta is path-keyed") };
                let bytes = reader.read_file_by_path_hash(hash).expect("read tilemeta entry");
                (hash, unpack_type(record.locator.meta32), bytes)
            })
            .collect();
        Self { entries }
    }

    /// The package with entry `replaced` holding `bytes`, uncompressed so that the reader sees the damage.
    fn package_with(&self, replaced: usize, bytes: &[u8]) -> Vec<u8> {
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (index, (hash, data_type, data)) in self.entries.iter().enumerate() {
            builder
                .add_file(AddFileRequest {
                    data_type: *data_type,
                    compression: CompressionFlag::None,
                    width: 0,
                    height: 0,
                    virtual_path: None,
                    path_hash64: Some(*hash),
                    id: None,
                    data: if index == replaced { bytes } else { data },
                })
                .expect("add tilemeta entry");
        }
        builder.build().expect("build tilemeta package")
    }
}

/// The tilemeta reader and the accessors the renderer calls.
fn read_tilemeta(package: Vec<u8>) {
    let Ok(reader) = UddpReader::open(package) else { return };
    let Ok(tilemeta) = udd_assets::TileMetaPackage::from_uddp_package(reader) else { return };
    let _ = tilemeta.land_tiles().len();
    let tile_ids: Vec<u32> = tilemeta.item_tiles().iter().map(|item| item.tile_id).collect();
    for tile_id in tile_ids {
        let _ = tilemeta.item_texture_refs(tile_id);
        let _ = tilemeta.main_ec_texture_ref_with_reason(tile_id);
        let _ = tilemeta.main_ec_texture_id_role_aware(tile_id);
        let _ = tilemeta.item_material(tile_id, 0);
    }
}

/// Each entry of a tilemeta package, damaged in turn.
fn assert_tilemeta_reader_survives(label: &str, package: &[u8], samples_per_entry_bytes: usize) {
    let reader = UddpReader::open(package.to_vec()).expect("open tilemeta");
    let entries = TileMetaEntries::read(&reader);
    for (index, (hash, _, bytes)) in entries.entries.iter().enumerate() {
        // Long entries are records of one layout: their head and a slice of their records stand for them.
        let sample = bytes[..bytes.len().min(samples_per_entry_bytes)].to_vec();
        let decoder = format!("tilemeta entry {hash:016X}");
        let samples = [Sample::new(format!("{label} entry {index}"), sample.clone())];
        assert_no_panics(&decoder, &samples, &|damaged| {
            let mut data = damaged.to_vec();
            data.extend_from_slice(&bytes[sample.len()..]);
            if damaged.len() < sample.len() {
                // A cut sample is a cut entry.
                data.truncate(damaged.len());
            }
            read_tilemeta(entries.package_with(index, &data));
        });
    }
}

// --- Synthetic samples ------------------------------------------------------------------------------------------

/// A static art tile: flags, size, the row lookup table, then each row's (offset, run) chunks and colours.
fn synthetic_static_art(width: u16, height: u16) -> Vec<u8> {
    let mut rows: Vec<Vec<u16>> = Vec::new();
    for y in 0..height {
        let run = width / 2;
        let mut row = vec![y % 2, run];
        row.extend((0..run).map(|x| 0x8000 | (x * 31 + y)));
        row.extend([0, 0]);
        rows.push(row);
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    let mut lookup = 0u16;
    for row in &rows {
        bytes.extend_from_slice(&lookup.to_le_bytes());
        lookup += row.len() as u16;
    }
    for word in rows.iter().flatten() {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

/// A gump: the row lookup table (in 4-byte units), then each row's (colour, count) pairs.
fn synthetic_gump(width: u16, height: u16) -> Vec<u8> {
    let rows: Vec<Vec<u16>> = (0..height).map(|y| vec![0x7C00 | y, width / 2, 0x001F, width - width / 2]).collect();
    let lookup_words = usize::from(height);
    let mut bytes = Vec::new();
    let mut lookup = lookup_words as u32;
    for row in &rows {
        bytes.extend_from_slice(&lookup.to_le_bytes());
        lookup += (row.len() / 2) as u32;
    }
    for word in rows.iter().flatten() {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn synthetic_classic_mul_animation() -> Vec<u8> {
    use uocf::classic::anim_frames::{mul::ClassicMulEntry, ClassicFrame, PALETTE_COLOURS};
    let palette: [u16; PALETTE_COLOURS] = std::array::from_fn(|index| index as u16 | 0x8000);
    let frame = |width: u16, height: u16| ClassicFrame {
        center_x: (width / 2) as i16,
        center_y: 0,
        width,
        height,
        pixels: (0..usize::from(width) * usize::from(height))
            .map(|index| (index % 3 != 0).then_some((index % 250 + 1) as u8))
            .collect(),
    };
    ClassicMulEntry { palette, frames: vec![Some(frame(12, 9)), None, Some(frame(5, 17))] }
        .to_bytes()
        .expect("encode the synthetic animation")
}

#[test]
fn the_xorshift_generator_repeats_from_its_seed() {
    let first: Vec<u64> = std::iter::repeat_with({
        let mut rng = XorShift64::new(SEED);
        move || rng.next()
    })
    .take(8)
    .collect();
    let mut rng = XorShift64::new(SEED);
    assert!(first.iter().all(|&value| value == rng.next()));
    assert_eq!(truncation_lengths(3), [0, 1, 2]);
    assert!(truncation_lengths(CUT_AT_EVERY_LENGTH_BELOW * 4).len() <= HEADER_BYTES + TRUNCATION_POINTS + 1);
}

#[test]
fn decoders_survive_damaged_synthetic_samples() {
    let land_bytes = (0..uocf::classic::art::LAND_DIAMOND_PIXEL_COUNT)
        .flat_map(|index| (index as u16 | 0x8000).to_le_bytes())
        .collect();
    assert_no_panics("land art", &[Sample::new("synthetic", land_bytes)], &decode_land_art);
    assert_no_panics("static art", &[Sample::new("synthetic 20x11", synthetic_static_art(20, 11))], &decode_static_art);
    assert_no_panics("gump", &[gump_sample("synthetic 9x7".into(), 9, 7, &synthetic_gump(9, 7))], &decode_gump);
    assert_no_panics(
        "Classic MUL animation",
        &[Sample::new("synthetic", synthetic_classic_mul_animation())],
        &decode_classic_mul_animation,
    );
}

// --- Shipped samples ------------------------------------------------------------------------------------------

fn install(folder: &str) -> PathBuf {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    std::env::split_paths(&roots)
        .map(|root| root.join(folder))
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| panic!("no {folder} under {CLIENT_ROOTS_ENV}"))
}

/// The non-empty entries of a UOP package, read whole, at most [`MAX_SAMPLE_BYTES`] each.
fn uop_entries(path: &Path) -> Vec<(u64, Vec<u8>)> {
    use uocf::uop_container::package::UopPackage;
    let package = UopPackage::load(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let hashes: Vec<u64> =
        package.iter_files().filter(|file| !file.is_empty_slot()).map(|file| file.filename_hash()).collect();
    hashes
        .into_par_iter()
        .filter_map(|hash| {
            let payload = package.unpack_file_by_hash(hash).ok().flatten()?;
            (!payload.is_empty() && payload.len() <= MAX_SAMPLE_BYTES).then_some((hash, payload))
        })
        .collect()
}

fn uop_samples(path: &Path, keep: impl Fn(&[u8]) -> bool) -> Vec<Sample> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let mut entries: Vec<(u64, Vec<u8>)> = uop_entries(path).into_iter().filter(|(_, bytes)| keep(bytes)).collect();
    entries.sort_unstable_by_key(|(hash, _)| *hash);
    spread(entries, SAMPLES_PER_DECODER)
        .into_iter()
        .map(|(hash, bytes)| Sample::new(format!("{name} {hash:016X}"), bytes))
        .collect()
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn classic_art_and_gump_decoders_survive_damaged_shipped_entries() {
    use uocf::classic::art::{ArtMap, STATIC_TILE_ID_BASE};
    use uocf::classic::gump::GumpMap;
    let classic = install(CLASSIC_FOLDER);

    let art = ArtMap::load(&classic).expect("load Classic art");
    let raw_art = |ids: Vec<u32>| -> Vec<Sample> {
        let present: Vec<(u32, Vec<u8>)> = ids
            .into_iter()
            .filter_map(|id| {
                let mut raw = Vec::new();
                art.get_raw_art_data(id, &mut raw).ok()?;
                (!raw.is_empty()).then_some((id, raw))
            })
            .collect();
        spread(present, SAMPLES_PER_DECODER)
            .into_iter()
            .map(|(id, raw)| Sample::new(format!("art 0x{id:05X}"), raw))
            .collect()
    };
    assert_no_panics("land art", &raw_art((0..STATIC_TILE_ID_BASE).collect()), &decode_land_art);
    assert_no_panics("static art", &raw_art((STATIC_TILE_ID_BASE..=art.max_id()).collect()), &decode_static_art);

    let gumps = GumpMap::load(&classic).expect("load Classic gumps");
    let present: Vec<u32> = gumps.available_gump_ids();
    let samples: Vec<Sample> = spread(present, SAMPLES_PER_DECODER)
        .into_iter()
        .filter_map(|id| {
            let mut raw = Vec::new();
            let size = gumps.get_raw_gump_data(id, &mut raw).ok()?;
            (raw.len() <= MAX_SAMPLE_BYTES).then(|| gump_sample(format!("gump {id}"), size.width, size.height, &raw))
        })
        .collect();
    assert_no_panics("gump", &samples, &decode_gump);

    let uop_gumps = uop_samples(&classic.join("gumpartLegacyMUL.uop"), |_| true);
    assert_no_panics("UOP gump", &uop_gumps, &decode_uop_gump);
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn classic_animation_decoders_survive_damaged_shipped_entries() {
    use uocf::classic::anim_frames::mul::read_mul_entry;
    /// The entries of `anim.idx` read, spread over the first ones (the high bodies of the file).
    const MUL_ENTRY_PROBES: u32 = 4096;
    let classic = install(CLASSIC_FOLDER);

    let (idx, mul) = (classic.join("anim.idx"), classic.join("anim.mul"));
    let present: Vec<(u32, Vec<u8>)> = (0..MUL_ENTRY_PROBES)
        .filter_map(|index| {
            let bytes = read_mul_entry(&idx, &mul, index).ok().flatten()?;
            (!bytes.is_empty() && bytes.len() <= MAX_SAMPLE_BYTES).then_some((index, bytes))
        })
        .collect();
    let samples: Vec<Sample> = spread(present, SAMPLES_PER_DECODER)
        .into_iter()
        .map(|(index, bytes)| Sample::new(format!("anim.mul entry {index}"), bytes))
        .collect();
    assert_no_panics("Classic MUL animation", &samples, &decode_classic_mul_animation);

    let samples = uop_samples(&classic.join("AnimationFrame1.uop"), |_| true);
    assert_no_panics("Classic UOP animation", &samples, &decode_classic_uop_animation);
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn amou_decoder_survives_damaged_shipped_entries() {
    let enhanced = install(ENHANCED_FOLDER);
    let samples = uop_samples(&enhanced.join("AnimationFrame1.uop"), |_| true);
    assert_no_panics("AMOU", &samples, &decode_amou);
}

/// Packages the image samples come from: world, terrain, effect and legacy textures, and the interface.
const IMAGE_PACKAGES: [&str; 5] =
    ["Texture.uop", "TerrainTexture.uop", "EffectTexture.uop", "LegacyTexture.uop", "Interface.uop"];

/// Samples of the image packages whose content is one of `formats`.
fn image_samples(formats: &'static [uocf::enhanced::image_census::ContentFormat]) -> Vec<Sample> {
    let enhanced = install(ENHANCED_FOLDER);
    IMAGE_PACKAGES
        .iter()
        .flat_map(|package| {
            uop_samples(&enhanced.join(package), |bytes| {
                formats.contains(&uocf::enhanced::image_census::classify_content(bytes).format)
            })
        })
        .collect()
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn dds_decoder_survives_damaged_shipped_entries() {
    use uocf::enhanced::image_census::ContentFormat;
    assert_no_panics("DDS", &image_samples(&[ContentFormat::Dds]), &decode_dds);
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn tga_decoder_survives_damaged_shipped_entries() {
    use uocf::enhanced::image_census::ContentFormat;
    assert_no_panics("TGA", &image_samples(&[ContentFormat::Tga, ContentFormat::TgaTrailing]), &decode_tga);
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn tilemeta_reader_survives_damaged_shipped_packages() {
    use udd_conv::tilemeta::{build_tilemeta_uddp_from_split_sources, TileMetaBuildOptions};
    /// Bytes of each tilemeta entry damaged: its head and the first records.
    const TILEMETA_ENTRY_SAMPLE_BYTES: usize = 4096;
    let classic = install(CLASSIC_FOLDER);
    let enhanced = install(ENHANCED_FOLDER);

    let dir = std::env::temp_dir().join(format!("uddconv_decoder_robustness_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let tilemeta_path = dir.join("tilemeta.uddp");
    build_tilemeta_uddp_from_split_sources(&classic, &enhanced, &tilemeta_path, &TileMetaBuildOptions::default())
        .expect("build tilemeta");
    let package = std::fs::read(&tilemeta_path).expect("read tilemeta");
    std::fs::remove_dir_all(&dir).ok();
    assert_tilemeta_reader_survives("client tilemeta.uddp", &package, TILEMETA_ENTRY_SAMPLE_BYTES);
}
