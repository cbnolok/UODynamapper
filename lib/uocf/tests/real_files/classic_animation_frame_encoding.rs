//! Decodes and re-encodes every Classic mobile animation frame under the client roots: the `animationlegacyframe`
//! entries of every `AnimationFrame*.uop`, and every entry of every `anim*.idx` / `anim*.mul` pair.
//!
//! The roots come from `UO_CLIENT_ROOTS` (a path list, `:`-separated on Unix), searched recursively; they are only
//! read. For each entry ([`uocf::classic::anim_frames`]):
//!
//! - indexed path: the frames decoded to palette indices and written back give the entry byte for byte;
//! - RGBA path: the frames as RGBA, indexed again with the entry's palette, give the entry byte for byte; indexed
//!   with a palette built from the pixels, they give the same pixels;
//! - the RGBA agrees with uocf's frame decoders (`AnimationFrameCc`, `AnimMap`).
//!
//! Counts are printed (run with `--nocapture`). The package test rewrites one UOP entry and one MUL body/action
//! through the PNG folder and the writers; outputs go to `std::env::temp_dir()` (set `TMPDIR` to choose it) and are
//! deleted afterwards.

use crate::common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use uocf::classic::anim::{classic_animation_source_index_from_identity, AnimFrame, AnimMap};
use uocf::classic::anim_frames::folder::{export_mul_folder, export_uop_folder, import_folder};
use uocf::classic::anim_frames::mul::{read_mul_entry, write_mul_entries, ClassicMulEntry};
use uocf::classic::anim_frames::uop::{read_uop_entry, write_uop_entry, ClassicUopEntry};
use uocf::classic::anim_frames::{
    index_rgba_frames, ClassicFrame, ClassicPalette, ClassicRgbaFrame, OPAQUE_BLACK, PALETTE_COLOURS,
};
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::enhanced::animationframe::package::EntryChange;
use uocf::enhanced::animationframe::{MAX_ACTION_ID_FALLBACK, MAX_BODY_ID};
use uocf::uop_container::package::UopPackage;

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const UOP_PREFIX: &str = "animationframe";
const UOP_EXTENSION: &str = ".uop";
const MUL_PREFIX: &str = "anim";
const IDX_EXTENSION: &str = ".idx";
const MUL_EXTENSION: &str = "mul";
const INDEX_ENTRY_BYTES: usize = 12;
const DIRECTIONS: u8 = 5;

fn files(matches: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    fn collect(dir: &Path, matches: &dyn Fn(&str) -> bool, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                collect(&path, matches, found);
            } else if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| matches(&name.to_ascii_lowercase())) {
                found.push(path);
            }
        }
    }
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let mut found = Vec::new();
    for root in std::env::split_paths(&roots) {
        collect(&root, &matches, &mut found);
    }
    found
}

/// Every `AnimationFrame*.uop` under the roots.
fn uop_packages() -> Vec<PathBuf> {
    files(|name| name.starts_with(UOP_PREFIX) && name.ends_with(UOP_EXTENSION))
}

/// Every `anim*.idx` with its data file, and the file number (1 for `anim.idx`, N for `animN.idx`).
fn mul_pairs() -> Vec<(PathBuf, PathBuf, u8)> {
    files(|name| {
        name.strip_prefix(MUL_PREFIX)
            .and_then(|rest| rest.strip_suffix(IDX_EXTENSION))
            .is_some_and(|number| number.chars().all(|c| c.is_ascii_digit()))
    })
    .into_iter()
    .filter_map(|idx| {
        let stem = idx.file_stem()?.to_str()?.to_ascii_lowercase();
        let number = stem.strip_prefix(MUL_PREFIX)?;
        let file = if number.is_empty() { 1 } else { number.parse().ok()? };
        let dir = idx.parent()?;
        let mul = std::fs::read_dir(dir).ok()?.filter_map(|entry| entry.ok()).map(|entry| entry.path()).find(|path| {
            path.file_stem().and_then(|stem| stem.to_str()).is_some_and(|other| other.eq_ignore_ascii_case(&stem))
                && path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case(MUL_EXTENSION))
        })?;
        Some((idx, mul, file))
    })
    .collect()
}

/// Entry hash -> (body, action) of every `animationlegacyframe` name.
fn legacy_names() -> HashMap<u64, (u32, u16)> {
    let mut names = HashMap::new();
    for body in 0..MAX_BODY_ID {
        for action in 0..MAX_ACTION_ID_FALLBACK {
            names.insert(AnimationFrameCc::animationframe_hash(body, action), (body, action));
        }
    }
    names
}

#[derive(Default)]
struct Counts {
    entries: AtomicUsize,
    frames: AtomicUsize,
    indexed_identical: AtomicUsize,
    rgba_identical: AtomicUsize,
    rebuilt_pixels_identical: AtomicUsize,
    rebuilt_identical: AtomicUsize,
    decoder_agrees: AtomicUsize,
    rebuilt_quantized: AtomicUsize,
    colour_zero_frames: AtomicUsize,
    rebuilt_black: AtomicUsize,
}

fn add(counter: &AtomicUsize, value: usize) {
    counter.fetch_add(value, Ordering::Relaxed);
}

fn get(counter: &AtomicUsize) -> usize {
    counter.load(Ordering::Relaxed)
}

fn percent(part: usize, whole: usize) -> f64 {
    const PERCENT: f64 = 100.0;
    if whole == 0 { PERCENT } else { part as f64 * PERCENT / whole as f64 }
}

fn rgba_of(frame: &ClassicFrame, palette: &ClassicPalette) -> ClassicRgbaFrame {
    ClassicRgbaFrame {
        center_x: frame.center_x,
        center_y: frame.center_y,
        width: frame.width,
        height: frame.height,
        rgba: frame.to_rgba(palette).data,
    }
}

/// Whether uocf's decoder output `old` is the frame as the client draws it (a colour of 0 opaque black); counts the
/// frames with such pixels.
fn same_image(old: &AnimFrame, frame: &ClassicFrame, palette: &ClassicPalette, counts: &Counts) -> bool {
    let new = frame.to_rgba(palette);
    let colour_zero = frame.pixels.iter().flatten().any(|&index| palette[usize::from(index)] == 0);
    add(&counts.colour_zero_frames, usize::from(colour_zero));
    (old.width, old.height, old.data.as_slice()) == (new.width, new.height, new.data.as_slice())
        && (old.width == 0 || (old.center_x, old.center_y) == (new.center_x, new.center_y))
}

/// The RGBA and rebuilt-palette checks shared by both formats; `encode` writes the entry from a palette and frames.
fn check_rgba(
    palette: &ClassicPalette,
    frames: &[&ClassicFrame],
    payload: &[u8],
    counts: &Counts,
    context: &str,
    encode: impl Fn(ClassicPalette, Vec<ClassicFrame>) -> Vec<u8>,
) {
    let rgba: Vec<ClassicRgbaFrame> = frames.iter().map(|frame| rgba_of(frame, palette)).collect();
    let (same, indexed, report) = index_rgba_frames(&rgba, Some(palette)).unwrap();
    assert_eq!(report.approximated, 0, "{context}: every colour is in the palette");
    add(&counts.rgba_identical, usize::from(encode(same, indexed) == payload));
    let (built, indexed, report) = index_rgba_frames(&rgba, None).unwrap();
    add(&counts.rebuilt_quantized, usize::from(report.quantized));
    // A new palette stores black as OPAQUE_BLACK.
    const BLACK: [u8; 4] = [0, 0, 0, u8::MAX];
    let mut black_word = [0u16; PALETTE_COLOURS];
    black_word[1] = OPAQUE_BLACK;
    let stored_black = ClassicFrame { center_x: 0, center_y: 0, width: 1, height: 1, pixels: vec![Some(1)] }
        .to_rgba(&black_word)
        .data;
    let pixels = indexed.iter().zip(&rgba).all(|(frame, original)| {
        let expected: Vec<u8> = original
            .rgba
            .chunks_exact(BLACK.len())
            .flat_map(|pixel| if pixel == BLACK { stored_black.clone() } else { pixel.to_vec() })
            .collect();
        frame.to_rgba(&built).data == expected
    });
    if !report.quantized {
        assert!(pixels, "{context}: a rebuilt palette keeps the pixels (black as OPAQUE_BLACK)");
    }
    add(&counts.rebuilt_black, usize::from(report.black > 0));
    add(&counts.rebuilt_pixels_identical, usize::from(pixels));
    add(&counts.rebuilt_identical, usize::from(encode(built, indexed) == payload));
}

fn check_uop_entry(payload: &[u8], counts: &Counts, context: &str) {
    let entry = ClassicUopEntry::parse(payload).unwrap_or_else(|error| panic!("{context}: {error:#}"));
    add(&counts.entries, 1);
    add(&counts.frames, entry.frames.len());
    add(&counts.indexed_identical, usize::from(entry.to_bytes().unwrap() == payload));

    let decoded = AnimationFrameCc::parse(payload).unwrap();
    let agrees = decoded.frames.len() == entry.frames.len()
        && decoded.frames.iter().zip(&entry.frames).all(|(old, new)| same_image(old, &new.frame, &entry.palette, counts));
    add(&counts.decoder_agrees, usize::from(agrees));

    let frames: Vec<&ClassicFrame> = entry.frames.iter().map(|frame| &frame.frame).collect();
    check_rgba(&entry.palette, &frames, payload, counts, context, |palette, indexed| {
        let mut copy = entry.clone();
        copy.palette = palette;
        for (frame, new) in copy.frames.iter_mut().zip(indexed) {
            frame.frame = new;
        }
        copy.to_bytes().unwrap()
    });
}

fn check_mul_entry(payload: &[u8], legacy: Option<Vec<AnimFrame>>, counts: &Counts, context: &str) {
    let entry = ClassicMulEntry::parse(payload).unwrap_or_else(|error| panic!("{context}: {error:#}"));
    add(&counts.entries, 1);
    add(&counts.frames, entry.frames.len());
    add(&counts.indexed_identical, usize::from(entry.to_bytes().unwrap() == payload));

    let agrees = legacy.is_some_and(|legacy| {
        legacy.len() == entry.frames.len()
            && legacy.iter().zip(&entry.frames).all(|(old, new)| match new {
                Some(new) => same_image(old, new, &entry.palette, counts),
                None => old.width == 0 && old.height == 0,
            })
    });
    add(&counts.decoder_agrees, usize::from(agrees));

    let present: Vec<&ClassicFrame> = entry.frames.iter().flatten().collect();
    check_rgba(&entry.palette, &present, payload, counts, context, |palette, indexed| {
        let mut indexed = indexed.into_iter();
        let frames = entry.frames.iter().map(|frame| frame.as_ref().map(|_| indexed.next().unwrap())).collect();
        ClassicMulEntry { palette, frames }.to_bytes().unwrap()
    });
}

fn report(name: &str, counts: &Counts) {
    let entries = get(&counts.entries);
    println!(
        "{name}: {entries} entries, {} frames; indexed re-encode byte-identical {:.3}%; RGBA with the palette \
         byte-identical {:.3}%; RGBA with a rebuilt palette: pixels identical {:.3}% (quantized {} entries, black stored as 0x0001 in {}), \
         byte-identical {:.3}%; agrees with the uocf decoder {:.3}% (frames with colour-0 pixels, opaque black: {})",
        get(&counts.frames),
        percent(get(&counts.indexed_identical), entries),
        percent(get(&counts.rgba_identical), entries),
        percent(get(&counts.rebuilt_pixels_identical), entries),
        get(&counts.rebuilt_quantized),
        get(&counts.rebuilt_black),
        percent(get(&counts.rebuilt_identical), entries),
        percent(get(&counts.decoder_agrees), entries),
        get(&counts.colour_zero_frames),
    );
    assert_eq!(get(&counts.indexed_identical), entries, "{name}: indexed re-encode");
    assert_eq!(get(&counts.rgba_identical), entries, "{name}: RGBA with the palette");
    assert_eq!(get(&counts.decoder_agrees), entries, "{name}: agreement with the uocf decoder");
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_classic_uop_frame_round_trips() {
    let names = legacy_names();
    let mut total = 0;
    for path in uop_packages() {
        let package = UopPackage::load(&path).unwrap();
        let files: Vec<_> = package
            .iter_files()
            .filter(|file| file.has_size())
            .filter_map(|file| names.get(&file.filename_hash()).map(|&key| (file, key)))
            .collect();
        if files.is_empty() {
            continue;
        }
        let counts = Counts::default();
        let name = path.display().to_string();
        files.par_iter().for_each(|(file, (body, action))| {
            let payload = file.unpack().unwrap();
            check_uop_entry(&payload, &counts, &format!("{name}: body {body} action {action}"));
        });
        total += get(&counts.entries);
        report(&name, &counts);
    }
    assert!(total > 0, "no animationlegacyframe entry under {CLIENT_ROOTS_ENV}");
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_classic_mul_frame_round_trips() {
    let pairs = mul_pairs();
    assert!(!pairs.is_empty(), "no anim*.idx under {CLIENT_ROOTS_ENV}");
    for (idx_path, mul_path, file) in pairs {
        let idx = std::fs::read(&idx_path).unwrap();
        let mul = std::fs::read(&mul_path).unwrap();
        let folder = idx_path.parent().unwrap();
        let map = AnimMap::load(folder).unwrap();
        let file_index = file - 1;
        let counts = Counts::default();
        let name = idx_path.display().to_string();
        (0..idx.len() / INDEX_ENTRY_BYTES).into_par_iter().for_each(|index| {
            let field = |n: usize| u32::from_le_bytes(idx[index * INDEX_ENTRY_BYTES + 4 * n..][..4].try_into().unwrap());
            let (lookup, length) = (field(0), field(1));
            if lookup == u32::MAX || length == 0 || length == u32::MAX {
                return;
            }
            let payload = &mul[lookup as usize..lookup as usize + length as usize];
            let legacy = map.decode_animation(file_index, index as u32).ok();
            check_mul_entry(payload, legacy, &counts, &format!("{name}: entry {index}"));
        });
        report(&name, &counts);
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn folder_imports_and_writers_change_only_the_edited_entries() {
    let names = legacy_names();
    // One UOP entry per package.
    for path in uop_packages() {
        let name = path.display().to_string();
        let source = UopPackage::load(&path).unwrap();
        let Some(&(body, action)) =
            source.iter_files().filter(|file| file.has_size()).find_map(|file| names.get(&file.filename_hash()))
        else {
            continue;
        };
        let dir = common::temp_dir("classic_animation_uop");
        let payload = read_uop_entry(&source, body, action).unwrap().unwrap();
        let folder = dir.join("frames");
        export_uop_folder(&payload, body, action, None, true, &folder).unwrap();
        let imported = import_folder(&folder).unwrap();
        assert!(imported.uop_payload(Some(&payload)).unwrap() == payload, "{name}: body {body} action {action}");

        let unchanged = dir.join("unchanged.uop");
        assert_eq!(write_uop_entry(&path, &unchanged, body, action, &payload).unwrap(), EntryChange::Unchanged);
        assert!(std::fs::read(&unchanged).unwrap() == std::fs::read(&path).unwrap(), "{name}: an unchanged write");
        assert!(write_uop_entry(&path, &path, body, action, &payload).is_err(), "{name}: never overwrites");

        let mut entry = ClassicUopEntry::parse(&payload).unwrap();
        entry.frames.pop();
        let edited = entry.to_bytes().unwrap();
        let output = dir.join("edited.uop");
        assert_eq!(write_uop_entry(&path, &output, body, action, &edited).unwrap(), EntryChange::Replaced);
        let rewritten = UopPackage::load(&output).unwrap();
        assert!(read_uop_entry(&rewritten, body, action).unwrap().unwrap() == edited, "{name}");
        let edited_hash = AnimationFrameCc::animationframe_hash(body, action);
        for file in source.iter_files().filter(|file| file.has_size() && file.filename_hash() != edited_hash) {
            let copy = rewritten.get_file_by_hash(file.filename_hash()).unwrap();
            assert_eq!(copy.data(), file.data(), "{name}: {:016X} keeps its bytes", file.filename_hash());
        }
        println!("{name}: body {body} action {action} round trips through PNGs and rewrites alone");
        std::fs::remove_dir_all(dir).unwrap();
    }

    // The smallest MUL pair: its first body/action with all five directions.
    let Some((idx_path, mul_path, file)) =
        mul_pairs().into_iter().min_by_key(|(_, mul, _)| std::fs::metadata(mul).map(|meta| meta.len()).unwrap_or(u64::MAX))
    else {
        return;
    };
    let name = idx_path.display().to_string();
    let file_index = file - 1;
    let slots = (std::fs::metadata(&idx_path).unwrap().len() as usize / INDEX_ENTRY_BYTES) as u32;
    let found = (0..u16::MAX).find_map(|body| {
        let indices: Vec<u32> = (0..DIRECTIONS)
            .map(|direction| classic_animation_source_index_from_identity(file_index, body, 0, direction))
            .collect::<Option<_>>()?;
        if indices.iter().any(|&index| index >= slots) {
            return None;
        }
        let entries: Vec<(u8, Vec<u8>)> = indices
            .iter()
            .zip(0u8..)
            .map(|(&index, direction)| read_mul_entry(&idx_path, &mul_path, index).unwrap().map(|bytes| (direction, bytes)))
            .collect::<Option<_>>()?;
        Some((body, indices, entries))
    });
    let (body, indices, entries) = found.unwrap_or_else(|| panic!("{name}: no body with action 0 in five directions"));
    let dir = common::temp_dir("classic_animation_mul");
    let folder = dir.join("frames");
    export_mul_folder(&entries, file, u32::from(body), 0, true, &folder).unwrap();
    let payloads = import_folder(&folder).unwrap().mul_payloads().unwrap();
    assert!(payloads == entries, "{name}: body {body} through the folder");

    let (idx_out, mul_out) = (dir.join("anim.idx"), dir.join("anim.mul"));
    let unchanged: Vec<(u32, Vec<u8>)> = indices.iter().copied().zip(payloads.into_iter().map(|(_, bytes)| bytes)).collect();
    let changes = write_mul_entries(&idx_path, &mul_path, &idx_out, &mul_out, &unchanged).unwrap();
    assert!(changes.iter().all(|&change| change == EntryChange::Unchanged));
    assert!(std::fs::read(&idx_out).unwrap() == std::fs::read(&idx_path).unwrap(), "{name}: the index is a copy");
    assert!(std::fs::read(&mul_out).unwrap() == std::fs::read(&mul_path).unwrap(), "{name}: the data is a copy");

    let mut edited = ClassicMulEntry::parse(&unchanged[0].1).unwrap();
    edited.frames.pop();
    let edited = edited.to_bytes().unwrap();
    std::fs::remove_file(&idx_out).unwrap();
    std::fs::remove_file(&mul_out).unwrap();
    let changes = write_mul_entries(&idx_path, &mul_path, &idx_out, &mul_out, &[(indices[0], edited.clone())]).unwrap();
    assert_eq!(changes, [EntryChange::Replaced]);
    assert!(read_mul_entry(&idx_out, &mul_out, indices[0]).unwrap().unwrap() == edited, "{name}");
    let (source_idx, output_idx) = (std::fs::read(&idx_path).unwrap(), std::fs::read(&idx_out).unwrap());
    let edited_range = indices[0] as usize * INDEX_ENTRY_BYTES..(indices[0] as usize + 1) * INDEX_ENTRY_BYTES;
    assert_eq!(source_idx[..edited_range.start], output_idx[..edited_range.start], "{name}");
    assert_eq!(source_idx[edited_range.end..], output_idx[edited_range.end..], "{name}");
    let (source_mul, output_mul) = (std::fs::read(&mul_path).unwrap(), std::fs::read(&mul_out).unwrap());
    assert!(output_mul[..source_mul.len()] == source_mul[..] && output_mul[source_mul.len()..] == edited[..], "{name}");
    println!("{name}: body {body} action 0 round trips through PNGs; one direction rewrites alone");
    std::fs::remove_dir_all(dir).unwrap();
}
