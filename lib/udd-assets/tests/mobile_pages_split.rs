//! A mobile animation package whose BC7 pages are stored in the field split (container codecs 4 and 5) hands on every
//! frame and every clip byte for byte as the same package with plain Zstd pages: the per-clip join and the page cache
//! against the whole-page decode. Repack a package both ways (`--page-coding zstd` and the default), then
//!
//! ```sh
//! UDD_MOBILE_ANIM_PLAIN=plain/mobile_anim_ec.uddp UDD_MOBILE_ANIM_SPLIT=split/mobile_anim_ec.uddp \
//!     cargo test --release -p udd-assets --test mobile_pages_split -- --ignored --nocapture
//! ```

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use udd_assets::animation::mobile_strides::{MobileAnimClips, StoredFramePixels};
use udd_assets::mobile_anim_ec::MobileAnimEcPackage;
use udd_assets::MobileAnimCcPackage;
use udd_container::{unpack_codec, Codec, UddpReader};

const PLAIN_ENV: &str = "UDD_MOBILE_ANIM_PLAIN";
const SPLIT_ENV: &str = "UDD_MOBILE_ANIM_SPLIT";
/// Threads comparing at once.
const THREADS: usize = 6;
/// Server directions, 0 north, clockwise.
const DIRECTIONS: u8 = 8;

/// One of the two packages, either kind.
enum Package {
    Classic(MobileAnimCcPackage),
    Enhanced(MobileAnimEcPackage),
}

impl Package {
    fn load(path: &PathBuf) -> Self {
        match MobileAnimCcPackage::load(path) {
            Ok(package) => Self::Classic(package),
            Err(_) => Self::Enhanced(MobileAnimEcPackage::load(path).expect("a mobile animation package")),
        }
    }

    fn clips(&self) -> &(dyn MobileAnimClips + Sync) {
        match self {
            Self::Classic(package) => package,
            Self::Enhanced(package) => package,
        }
    }

    /// Each frame record's page.
    fn frame_pages(&self) -> Vec<u32> {
        match self {
            Self::Classic(package) => package.frames().iter().map(|frame| frame.page_index).collect(),
            Self::Enhanced(package) => package.frames().iter().map(|frame| frame.page_index).collect(),
        }
    }

    fn read_frame(&self, index: usize) -> color_eyre::eyre::Result<StoredFramePixels> {
        match self {
            Self::Classic(package) => package.read_frame_stored(&package.frames()[index]),
            Self::Enhanced(package) => package.read_frame_stored(&package.frames()[index]),
        }
    }

    /// The bodies with animation records.
    fn bodies(&self) -> Vec<u32> {
        let bodies: BTreeSet<u32> = match self {
            Self::Classic(package) => package.animations().iter().map(|record| u32::from(record.body_id)).collect(),
            Self::Enhanced(package) => package.animations().iter().map(|record| record.body_id).collect(),
        };
        bodies.into_iter().collect()
    }
}

/// How many of the package's entries are stored with each codec.
fn entry_codecs(path: &PathBuf) -> Vec<(Codec, usize)> {
    let reader = UddpReader::load(path).expect("open the package");
    let mut counts: Vec<(Codec, usize)> = Vec::new();
    for record in reader.records() {
        let codec = unpack_codec(record.locator.meta32);
        match counts.iter_mut().find(|(kept, _)| *kept == codec) {
            Some((_, count)) => *count += 1,
            None => counts.push((codec, 1)),
        }
    }
    counts
}

/// Runs `work` on `0..count` on [`THREADS`] threads, each taking the next index.
fn on_threads(count: usize, work: impl Fn(usize) + Sync) {
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..THREADS {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= count {
                    break;
                }
                work(index);
            });
        }
    });
}

#[test]
#[ignore = "needs a package repacked both ways: set UDD_MOBILE_ANIM_PLAIN and UDD_MOBILE_ANIM_SPLIT, and run with --ignored"]
fn split_pages_hand_on_every_frame_and_clip_of_plain_pages() {
    let (Some(plain_path), Some(split_path)) = (std::env::var_os(PLAIN_ENV), std::env::var_os(SPLIT_ENV)) else {
        panic!("set {PLAIN_ENV} and {SPLIT_ENV}");
    };
    let (plain_path, split_path) = (PathBuf::from(plain_path), PathBuf::from(split_path));
    let (plain_codecs, split_codecs) = (entry_codecs(&plain_path), entry_codecs(&split_path));
    println!("entries: plain {plain_codecs:?}, split {split_codecs:?}");
    assert!(plain_codecs.iter().all(|(codec, _)| !codec.is_bc7_fields()), "{plain_codecs:?}");
    assert!(split_codecs.iter().any(|(codec, _)| codec.is_bc7_fields()), "{split_codecs:?}");
    let (plain, split) = (Package::load(&plain_path), Package::load(&split_path));

    // Every frame record alone, in page order so that each page is decoded about once.
    let started = Instant::now();
    let pages = plain.frame_pages();
    assert_eq!(pages, split.frame_pages(), "the packages must have the same frame records");
    let mut order: Vec<usize> = (0..pages.len()).collect();
    order.sort_by_key(|&index| pages[index]);
    let chunk = order.len().div_ceil(THREADS * THREADS).max(1);
    let chunks: Vec<&[usize]> = order.chunks(chunk).collect();
    on_threads(chunks.len(), |at| {
        for &index in chunks[at] {
            let expected = plain.read_frame(index).unwrap_or_else(|error| panic!("frame {index}: {error:#}"));
            let actual = split.read_frame(index).unwrap_or_else(|error| panic!("split frame {index}: {error:#}"));
            assert!(actual == expected, "frame {index} differs");
        }
    });
    println!("{} frames identical in {:.1} s", pages.len(), started.elapsed().as_secs_f64());

    // Every clip of every body, action and direction, through the clip reader.
    let started = Instant::now();
    let bodies = plain.bodies();
    assert_eq!(bodies, split.bodies());
    let clips = AtomicUsize::new(0);
    let frames = AtomicUsize::new(0);
    on_threads(bodies.len(), |at| {
        let body = bodies[at];
        let actions = plain.clips().body_actions(body);
        assert_eq!(actions, split.clips().body_actions(body), "body {body}");
        for action in actions {
            for direction in 0..DIRECTIONS {
                let expected = plain.clips().stored_clip(body, action, direction);
                let actual = split.clips().stored_clip(body, action, direction);
                match (expected, actual) {
                    (Ok(expected), Ok(actual)) => {
                        assert!(actual == expected, "clip {body} {action} {direction} differs");
                        if let Some(clip) = expected {
                            clips.fetch_add(1, Ordering::Relaxed);
                            frames.fetch_add(clip.frames.len(), Ordering::Relaxed);
                        }
                    }
                    (Err(expected), Err(actual)) => assert_eq!(format!("{actual:#}"), format!("{expected:#}")),
                    (expected, actual) => panic!("clip {body} {action} {direction}: {expected:?} against {actual:?}"),
                }
            }
        }
    });
    println!("{} clips ({} frames) of {} bodies identical in {:.1} s", clips.into_inner(), frames.into_inner(),
        bodies.len(), started.elapsed().as_secs_f64());
}
