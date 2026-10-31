//! Decodes and re-encodes every frame of every `AnimationFrame*.uop` under the client roots (Enhanced Client and
//! Kingdom Reborn `animationframe` entries; the Classic `animationlegacyframe` entries as containers only, since
//! their frames hold Classic palettes and RLE).
//!
//! The roots come from `UO_CLIENT_ROOTS` (a path list, `:`-separated on Unix), searched recursively; they are only
//! read. For each EC/KR entry:
//!
//! - container: [`AmouFile::parse`] then [`AmouFile::to_bytes`] gives the entry back;
//! - indexed path: every frame decoded to indices ([`AnimationFrame::decode_frame_indexed`]) and re-encoded with
//!   the original palette gives the entry back byte for byte, and decodes to the same pixels;
//! - RGBA path: every frame decoded to RGBA and hue mask, then indexed again ([`index_frames`]) with the original
//!   palette (byte-identical) and without it (a palette built from the pixels: same pixels, palette order differs);
//! - the header box is the union of the frame boxes and their mirrors.
//!
//! Counts are printed (run with `--nocapture`). The package test rewrites one entry through the PNG folder and the
//! package writer, and one `AnimationSequence.uop` frame count; outputs go to `std::env::temp_dir()` (set `TMPDIR`
//! to choose it) and are deleted afterwards.

use crate::common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use uocf::animation_sequence::AnimationSequence;
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::enhanced::animationframe::folder::{export_folder, import_folder};
use uocf::enhanced::animationframe::package::{
    read_entry, sequence_frame_count, write_entry, write_sequence_frame_count, EntryChange,
};
use uocf::enhanced::animationframe::{
    index_frames, AmouFile, AnimationFrame, RgbaFrame, MAX_ACTION_ID_FALLBACK, MAX_BODY_ID,
};
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::uop_container::package::UopPackage;

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const PACKAGE_PREFIX: &str = "animationframe";
const PACKAGE_EXTENSION: &str = ".uop";
const SEQUENCE_UOP_NAME: &str = "AnimationSequence.uop";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scheme {
    /// `build/animationframe/%06d/%02d.bin` (EC, KR).
    Enhanced,
    /// `build/animationlegacyframe/%06d/%02d.bin` (Classic).
    Legacy,
}

/// Every `AnimationFrame*.uop` under the client roots, in path order.
fn packages() -> Vec<PathBuf> {
    fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                collect(&path, found);
            } else if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                let name = name.to_ascii_lowercase();
                name.starts_with(PACKAGE_PREFIX) && name.ends_with(PACKAGE_EXTENSION)
            }) {
                found.push(path);
            }
        }
    }
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let mut found = Vec::new();
    for root in std::env::split_paths(&roots) {
        collect(&root, &mut found);
    }
    assert!(!found.is_empty(), "no AnimationFrame*.uop under {CLIENT_ROOTS_ENV}");
    found
}

/// Entry hash -> (scheme, body, action) for every name both schemes can have.
fn names() -> HashMap<u64, (Scheme, u32, u16)> {
    let mut names = HashMap::new();
    for body in 0..MAX_BODY_ID {
        for action in 0..MAX_ACTION_ID_FALLBACK {
            names.insert(AnimationFrame::animationframe_hash(body, action), (Scheme::Enhanced, body, action));
            names.insert(AnimationFrameCc::animationframe_hash(body, action), (Scheme::Legacy, body, action));
        }
    }
    names
}

#[derive(Default)]
struct Counts {
    entries: AtomicUsize,
    frames: AtomicUsize,
    container_identical: AtomicUsize,
    canonical_layout: AtomicUsize,
    union_bounds: AtomicUsize,
    indexed_identical: AtomicUsize,
    pixels_identical_frames: AtomicUsize,
    rgba_original_palette_identical: AtomicUsize,
    rgba_rebuilt_pixels_identical: AtomicUsize,
    rgba_rebuilt_identical: AtomicUsize,
    legacy_entries: AtomicUsize,
    legacy_identical: AtomicUsize,
    unnamed: AtomicUsize,
}

fn add(counter: &AtomicUsize, value: usize) {
    counter.fetch_add(value, Ordering::Relaxed);
}

/// RGBA frames and hue masks as the decoder gives them.
fn rgba_frames(animation: &AnimationFrame) -> Vec<RgbaFrame> {
    animation
        .frames
        .iter()
        .map(|entry| {
            let decoded = animation.decode_frame(entry).unwrap();
            RgbaFrame {
                width: decoded.width,
                height: decoded.height,
                center_x: decoded.center_x,
                center_y: decoded.center_y,
                rgba: decoded.data,
                hue_mask: Some(decoded.hue_mask),
            }
        })
        .collect()
}

/// Every frame decodes to the same RGBA and hue mask in both entries.
fn same_pixels(a: &AnimationFrame, b: &AnimationFrame) -> bool {
    a.frames.len() == b.frames.len()
        && a.frames.iter().zip(&b.frames).all(|(x, y)| {
            let (x, y) = (a.decode_frame(x).unwrap(), b.decode_frame(y).unwrap());
            (x.width, x.height, x.center_x, x.center_y) == (y.width, y.height, y.center_x, y.center_y)
                && x.data == y.data
                && x.hue_mask == y.hue_mask
        })
}

fn check_enhanced_entry(payload: &[u8], counts: &Counts, context: &str) {
    let (file, layout) = AmouFile::parse(payload).unwrap();
    add(&counts.entries, 1);
    add(&counts.canonical_layout, usize::from(layout.is_canonical()));
    add(&counts.container_identical, usize::from(file.to_bytes().unwrap() == payload));
    add(&counts.union_bounds, usize::from(file.bounds == file.frame_union_bounds()));

    let animation = AnimationFrame::load(payload).unwrap();
    add(&counts.frames, animation.frames.len());
    let keyed: Vec<_> = animation
        .frames
        .iter()
        .map(|entry| (entry.action, entry.frame_number, animation.decode_frame_indexed(entry).unwrap()))
        .collect();
    let indexed = AmouFile::from_indexed_frames(file.animation_id, file.colours.clone(), &keyed).unwrap();
    let indexed_bytes = indexed.to_bytes().unwrap();
    add(&counts.indexed_identical, usize::from(indexed_bytes == payload));
    let reencoded = AnimationFrame::load(&indexed_bytes).unwrap();
    let identical_pixels = same_pixels(&animation, &reencoded);
    assert!(identical_pixels, "{context}: indexed re-encode changes pixels");
    add(&counts.pixels_identical_frames, animation.frames.len());

    let rgba = rgba_frames(&animation);
    let encode = |palette: Option<&[[u8; 4]]>| {
        let (palette, frames, report) = index_frames(&rgba, palette).unwrap();
        assert_eq!(report.approximated, 0, "{context}");
        let keyed: Vec<_> = animation
            .frames
            .iter()
            .zip(frames)
            .map(|(entry, frame)| (entry.action, entry.frame_number, frame))
            .collect();
        AmouFile::from_indexed_frames(file.animation_id, palette, &keyed).unwrap().to_bytes().unwrap()
    };
    let with_palette = encode(Some(&file.colours));
    add(&counts.rgba_original_palette_identical, usize::from(with_palette == payload));
    let rebuilt = encode(None);
    add(&counts.rgba_rebuilt_identical, usize::from(rebuilt == payload));
    let rebuilt_same = same_pixels(&animation, &AnimationFrame::load(&rebuilt).unwrap());
    assert!(rebuilt_same, "{context}: the rebuilt palette changes pixels");
    add(&counts.rgba_rebuilt_pixels_identical, 1);
}

fn percent(part: usize, whole: usize) -> f64 {
    const PERCENT: f64 = 100.0;
    if whole == 0 { PERCENT } else { part as f64 * PERCENT / whole as f64 }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_frame_round_trips() {
    let names = names();
    for path in packages() {
        let name = path.display().to_string();
        let package = UopPackage::load(&path).unwrap();
        let files: Vec<_> = package.iter_files().filter(|file| file.has_size()).collect();
        let counts = Counts::default();
        files.par_iter().for_each(|file| {
            let payload = file.unpack().unwrap();
            match names.get(&file.filename_hash()) {
                Some(&(Scheme::Enhanced, body, action)) => {
                    check_enhanced_entry(&payload, &counts, &format!("{name}: body {body} action {action}"))
                }
                Some(&(Scheme::Legacy, ..)) => {
                    add(&counts.legacy_entries, 1);
                    let (file, _) = AmouFile::parse(&payload).unwrap();
                    add(&counts.legacy_identical, usize::from(file.to_bytes().unwrap() == payload));
                }
                None => add(&counts.unnamed, 1),
            }
        });
        let get = |counter: &AtomicUsize| counter.load(Ordering::Relaxed);
        let entries = get(&counts.entries);
        let legacy = get(&counts.legacy_entries);
        if entries > 0 {
            println!(
                "{name} ({}): {entries} entries, {} frames; container byte-identical {:.3}% (canonical layout \
                 {:.3}%), header box = mirrored union {:.3}%; indexed re-encode byte-identical {:.3}%, pixels \
                 identical {} frames; RGBA with the original palette byte-identical {:.3}%; RGBA with a rebuilt \
                 palette: pixels identical {:.3}%, byte-identical {:.3}%",
                ClientDialect::of_package_path(&path).label(),
                get(&counts.frames),
                percent(get(&counts.container_identical), entries),
                percent(get(&counts.canonical_layout), entries),
                percent(get(&counts.union_bounds), entries),
                percent(get(&counts.indexed_identical), entries),
                get(&counts.pixels_identical_frames),
                percent(get(&counts.rgba_original_palette_identical), entries),
                percent(get(&counts.rgba_rebuilt_pixels_identical), entries),
                percent(get(&counts.rgba_rebuilt_identical), entries),
            );
            assert_eq!(get(&counts.container_identical), entries, "{name}: containers");
            assert_eq!(get(&counts.indexed_identical), entries, "{name}: indexed re-encode");
            assert_eq!(get(&counts.rgba_original_palette_identical), entries, "{name}: RGBA with the palette");
        }
        if legacy > 0 {
            println!(
                "{name}: {legacy} Classic animationlegacyframe entries, container byte-identical {:.3}%",
                percent(get(&counts.legacy_identical), legacy)
            );
            assert_eq!(get(&counts.legacy_identical), legacy, "{name}: Classic containers");
        }
        assert_eq!(get(&counts.unnamed), 0, "{name}: entries without a known name");
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn folder_import_and_package_writes_change_only_the_edited_entry() {
    let names = names();
    for path in packages() {
        let name = path.display().to_string();
        let source = UopPackage::load(&path).unwrap();
        let Some((body, action)) = source.iter_files().filter(|file| file.has_size()).find_map(|file| {
            match names.get(&file.filename_hash()) {
                Some(&(Scheme::Enhanced, body, action)) => Some((body, action)),
                _ => None,
            }
        }) else {
            continue;
        };
        let dir = common::temp_dir("animation_frame_package");
        let payload = read_entry(&source, body, action).unwrap().unwrap();

        // PNG folder round trip with the palette.
        let folder = dir.join("frames");
        export_folder(&payload, body, action, None, true, &folder).unwrap();
        let imported = import_folder(&folder).unwrap();
        assert!(imported.payload == payload, "{name}: body {body} action {action} through the folder");

        let unchanged = dir.join("unchanged.uop");
        assert_eq!(write_entry(&path, &unchanged, body, action, &payload).unwrap(), EntryChange::Unchanged);
        let identical = std::fs::read(&unchanged).unwrap() == std::fs::read(&path).unwrap();
        assert!(identical, "{name}: an unchanged write is byte-identical");
        assert!(write_entry(&path, &path, body, action, &payload).is_err(), "{name}: never overwrites");

        // Drop the last frame: a changed entry.
        let (mut file, _) = AmouFile::parse(&payload).unwrap();
        file.frames.pop();
        file.bounds = file.frame_union_bounds();
        let edited = file.to_bytes().unwrap();
        let output = dir.join("edited.uop");
        assert_eq!(write_entry(&path, &output, body, action, &edited).unwrap(), EntryChange::Replaced);
        let rewritten = UopPackage::load(&output).unwrap();
        assert!(read_entry(&rewritten, body, action).unwrap().unwrap() == edited, "{name}");
        let edited_hash = AnimationFrame::animationframe_hash(body, action);
        for file in source.iter_files().filter(|file| file.has_size() && file.filename_hash() != edited_hash) {
            let copy = rewritten.get_file_by_hash(file.filename_hash()).unwrap();
            assert_eq!(copy.data(), file.data(), "{name}: {:016X} keeps its bytes", file.filename_hash());
        }
        println!("{name}: body {body} action {action} round trips through PNGs and rewrites alone");

        // The sequence frame count next to the package.
        let dialect = ClientDialect::of_package_path(&path);
        let Some(sequence_path) = path.parent().map(|parent| parent.join(SEQUENCE_UOP_NAME)).filter(|path| path.exists())
        else {
            std::fs::remove_dir_all(dir).unwrap();
            continue;
        };
        let sequences = UopPackage::load(&sequence_path).unwrap();
        if let Some(count) = sequence_frame_count(&sequences, body, u32::from(action), dialect).unwrap() {
            let sequence_output = dir.join(SEQUENCE_UOP_NAME);
            let previous =
                write_sequence_frame_count(&sequence_path, &sequence_output, body, u32::from(action), count + 1, dialect)
                    .unwrap();
            assert_eq!(previous, count);
            let written = UopPackage::load(&sequence_output).unwrap();
            assert_eq!(sequence_frame_count(&written, body, u32::from(action), dialect).unwrap(), Some(count + 1));
            let entry = written
                .unpack_file_by_hash(uocf::uop_container::hash::hash_file_name_single(&AnimationSequence::entry_path(body)))
                .unwrap()
                .unwrap();
            AnimationSequence::parse_dialect(&entry, dialect).unwrap();
            println!("{name}: sequence {body} action {action} frame count {count} -> {} written", count + 1);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
