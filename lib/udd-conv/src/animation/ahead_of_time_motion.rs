//! What SPAN 2x and RIFE in-betweens made ahead of time would cost a mobile animation package: its size (BC7 + Zstd,
//! the packers' defaults) and the conversion time on this machine, measured on a stratified sample of a package's clips
//! and extrapolated to the whole package. Ignored; run with
//! `AOT_PACKAGE=... cargo test -p udd-conv --release ahead_of_time -- --ignored --nocapture`.
//!
//! Method: the package's clips (each stored clip once: owner records, frames shared between bodies counted once) are
//! split into [`STRATA`] strata of equal 1x texels, from the smallest clips to the largest, and [`AOT_SAMPLES`] clips
//! are drawn at random from each. Every option's frames of a stratum's sample are packed into atlas pages by the EC
//! packer at the package's page size and gutter, BC7-encoded by the packer's encoder and Zstd'd; a stratum's bytes and
//! times are scaled by its texels over the sample's, and the bytes are calibrated so the 1x estimate equals the package,
//! which should therefore be a BC7 package with Zstd (an RGBA8888 one makes the sizes too large by its ratio to BC7; the
//! "x 1x" ratios hold either way).

use super::*;
use udd_assets::animation::mobile_strides::MobileClip;
use udd_assets::{MobileAnimCcPackage, MobileAnimEcPackage};
use uot_gpu_compute::frame_upscales::FrameUpscaler;
use uot_gpu_compute::in_betweens::{InBetweenMaker, SequenceFrame};
use uot_gpu_compute::strides::StrideFrame;

const PACKAGE_VARIABLE: &str = "AOT_PACKAGE";
/// Clips drawn from each stratum (default [`DEFAULT_SAMPLES`]).
const SAMPLES_VARIABLE: &str = "AOT_SAMPLES";
const DEFAULT_SAMPLES: usize = 40;
const STRATA: usize = 8;
const SAMPLE_SEED: u64 = 0x5EED_A07;
const ZSTD_LEVEL: i32 = 9;
/// The largest 1x frame side the upscaler takes.
const UPSCALER_MAX_SIDE: u32 = 1024;
/// The game's cost model of a SPAN frame step (upscales/render.rs, from uot-gpu-compute's benchmark fit).
const MS_PER_DISPATCH: f64 = 0.027;
const MS_PER_GIGA_MULTIPLY_ADD: f64 = 1.0;
const SPAN_DISPATCHES: f64 = 24.0;
const RIFE_X2: u32 = 2;
const RIFE_X4: u32 = 4;
const RGBA_BYTES: usize = 4;
const BYTES_PER_MB: f64 = 1e6;

/// One stored clip: the record it is read through and its 1x texels.
#[derive(Clone, Copy)]
struct StoredClip {
    record: usize,
    texels: u64,
    frames: u32,
}

enum Package {
    Classic(MobileAnimCcPackage),
    Enhanced(MobileAnimEcPackage),
}

impl Package {
    fn load(path: &Path) -> eyre::Result<Self> {
        match MobileAnimCcPackage::load(path) {
            Ok(package) => Ok(Self::Classic(package)),
            Err(_) => Ok(Self::Enhanced(MobileAnimEcPackage::load(path)?)),
        }
    }

    fn classic(&self) -> bool {
        matches!(self, Self::Classic(_))
    }

    fn page_count(&self) -> usize {
        match self {
            Self::Classic(package) => package.pages().len(),
            Self::Enhanced(package) => package.pages().len(),
        }
    }

    fn page_layout(&self) -> (u32, u32, u16) {
        match self {
            Self::Classic(package) => (package.atlas_width(), package.atlas_height(), package.gutter()),
            Self::Enhanced(package) => (package.atlas_width(), package.atlas_height(), package.gutter()),
        }
    }

    /// Each stored clip once.
    fn stored_clips(&self) -> Vec<StoredClip> {
        let mut seen = HashSet::new();
        let mut clips = Vec::new();
        let mut add = |record: usize, placements: Vec<(u32, u16, u64)>| {
            if placements.is_empty() {
                return;
            }
            let key: Vec<(u32, u16)> = placements.iter().map(|&(page, slot, _)| (page, slot)).collect();
            if seen.insert(key) {
                clips.push(StoredClip { record, texels: placements.iter().map(|&(_, _, texels)| texels).sum(),
                    frames: placements.len() as u32 });
            }
        };
        match self {
            Self::Classic(package) => for (index, animation) in package.animations().iter().enumerate() {
                let frames = package.animation_frames(animation);
                if frames.first().is_some_and(|frame| frame.animation_index as usize == index) {
                    add(index, frames.iter().filter(|frame| frame.width > 0 && frame.height > 0)
                        .map(|frame| (frame.page_index, frame.page_frame_index,
                            u64::from(frame.width) * u64::from(frame.height))).collect());
                }
            },
            Self::Enhanced(package) => for (index, animation) in package.animations().iter().enumerate() {
                let frames = package.animation_frames(animation);
                if frames.first().is_some_and(|frame| frame.animation_index as usize == index) {
                    add(index, frames.iter().enumerate()
                        .filter(|(at, frame)| !animation.is_frame_skipped(*at as u32) && frame.width > 0 && frame.height > 0)
                        .map(|(_, frame)| (frame.page_index, frame.page_frame_index,
                            u64::from(frame.width) * u64::from(frame.height))).collect());
                }
            },
        }
        clips
    }

    fn clip(&self, record: usize) -> eyre::Result<Option<MobileClip>> {
        match self {
            Self::Classic(package) => package.animation_clip(&package.animations()[record]),
            Self::Enhanced(package) => package.animation_clip(&package.animations()[record]),
        }
    }
}

/// The option measured, and the frames it adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Variant {
    Original,
    Span2x,
    RifeX2,
    RifeX4,
    Span2xRifeX2,
    Span2xRifeX4,
}

const VARIANTS: [Variant; 6] =
    [Variant::Original, Variant::Span2x, Variant::RifeX2, Variant::RifeX4, Variant::Span2xRifeX2, Variant::Span2xRifeX4];

/// What one stratum's sample measured.
#[derive(Default)]
struct StratumResult {
    sample_texels: u64,
    full_texels: u64,
    full_clips: usize,
    sample_clips: usize,
    bytes: BTreeMap<Variant, u64>,
    raw_bytes: BTreeMap<Variant, u64>,
    pages: BTreeMap<Variant, usize>,
    /// Seconds by stage: GPU work, atlas packing, BC7, Zstd.
    gpu: BTreeMap<Variant, f64>,
    span_model_ms: BTreeMap<Variant, f64>,
    pack: BTreeMap<Variant, f64>,
    bc7: BTreeMap<Variant, f64>,
    zstd: BTreeMap<Variant, f64>,
    span_skipped: usize,
}

/// A small deterministic generator, so a rerun draws the same sample.
struct XorShift(u64);

impl XorShift {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

fn stride_frame(frame: &SequenceFrame) -> StrideFrame<'_> {
    StrideFrame { width: frame.width as u16, height: frame.height as u16, anchor_x: frame.anchor_x,
        anchor_y: frame.anchor_y, rgba: &frame.rgba }
}

/// The frame cut to its pixels with alpha, its anchor moved with the cut; `None` when it has none.
fn trimmed(frame: &SequenceFrame) -> Option<SequenceFrame> {
    let (width, height) = (frame.width as usize, frame.height as usize);
    let alpha = |x: usize, y: usize| frame.rgba[(y * width + x) * RGBA_BYTES + 3] != 0;
    let rows: Vec<usize> = (0..height).filter(|&y| (0..width).any(|x| alpha(x, y))).collect();
    let columns: Vec<usize> = (0..width).filter(|&x| (0..height).any(|y| alpha(x, y))).collect();
    let (&top, &bottom) = (rows.first()?, rows.last()?);
    let (&left, &right) = (columns.first()?, columns.last()?);
    let rgba = (top..=bottom).flat_map(|y| frame.rgba[(y * width + left) * RGBA_BYTES..(y * width + right + 1) * RGBA_BYTES]
        .iter().copied()).collect();
    Some(SequenceFrame { width: (right + 1 - left) as u32, height: (bottom + 1 - top) as u32,
        anchor_x: frame.anchor_x - left as i32, anchor_y: frame.anchor_y - top as i32, rgba })
}

/// Packs `frames` into pages as the EC packer does, encodes them to BC7 and Zstd's them: stored bytes, raw BC7 bytes,
/// pages, and the seconds of each stage.
fn pack_and_store(frames: Vec<DecodedMobileAnimEcFrame>, options: &MobileAnimEcAtlasOptions)
    -> eyre::Result<(u64, u64, usize, f64, f64, f64)> {
    let timer = Instant::now();
    let (pages, _) = pack_frames_into_pages(frames, options)?;
    let pack = timer.elapsed().as_secs_f64();
    let timer = Instant::now();
    let (encoded, _, _) = encode_mobile_anim_page_chunk(&pages, options, None, None)?;
    let bc7 = timer.elapsed().as_secs_f64();
    let raw: u64 = encoded.iter().map(|(_, bytes, _, _)| bytes.len() as u64).sum();
    let timer = Instant::now();
    let stored: u64 = encoded.par_iter().map(|(_, bytes, _, _)| zstd::bulk::compress(bytes, ZSTD_LEVEL)
        .map(|compressed| compressed.len() as u64)).collect::<std::io::Result<Vec<_>>>()?.iter().sum();
    let zstd = timer.elapsed().as_secs_f64();
    Ok((stored, raw, pages.len(), pack, bc7, zstd))
}

fn decoded(clip: u32, index: &mut u16, frame: &SequenceFrame) -> DecodedMobileAnimEcFrame {
    *index += 1;
    DecodedMobileAnimEcFrame { body_id: clip, source_frame_index: *index, width: frame.width as u16,
        height: frame.height as u16, center_x: frame.anchor_x as i16, center_y: frame.anchor_y as i16,
        rgba: frame.rgba.clone(), hue_mask: Vec::new() }
}

/// One clip's frames for every variant: the 1x frames, RIFE x2 and x4 sequences (in-betweens trimmed), and their SPAN
/// 2x; the variant's seconds of GPU work added to `gpu` and the cost model's SPAN milliseconds to `span_model`.
struct ClipFrames {
    by_variant: BTreeMap<Variant, Vec<SequenceFrame>>,
}

fn measure_clip(clip: &MobileClip, snap_alpha: bool, maker: &mut InBetweenMaker, upscaler: &mut FrameUpscaler,
    result: &mut StratumResult) -> eyre::Result<ClipFrames> {
    let frames: Vec<SequenceFrame> = clip.frames.iter().map(|frame| SequenceFrame { width: u32::from(frame.width),
        height: u32::from(frame.height), anchor_x: frame.anchor_x, anchor_y: frame.anchor_y, rgba: frame.rgba.clone() })
        .collect();
    let strides: Vec<StrideFrame> = frames.iter().map(stride_frame).collect();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut sequence = |factor: u32, gpu: &mut f64| -> eyre::Result<Vec<SequenceFrame>> {
        let timer = Instant::now();
        let made = maker.sequence(&strides, factor, snap_alpha, &cancel, |_, _| {})?.expect("not cancelled");
        *gpu += timer.elapsed().as_secs_f64();
        // The clip's own frames stay as they are; each in-between is cut to its pixels.
        Ok(made.iter().enumerate().filter_map(|(at, frame)| if at % factor as usize == 0 || frames.len() < 2 {
            Some(frame.clone())
        } else {
            trimmed(frame)
        }).collect())
    };
    let mut rife_x2 = 0.0;
    let x2 = sequence(RIFE_X2, &mut rife_x2)?;
    let mut rife_x4 = 0.0;
    let x4 = sequence(RIFE_X4, &mut rife_x4)?;
    // SPAN of every frame of the x4 sequence: the x2 sequence's in-betweens are its middle ones (same encode, same
    // timestep), and the clip's own frames are every fourth.
    let mut upscaled = Vec::with_capacity(x4.len());
    let mut span_seconds = Vec::with_capacity(x4.len());
    let mut span_model = Vec::with_capacity(x4.len());
    for frame in &x4 {
        let timer = Instant::now();
        match upscaler.upscale(&stride_frame(frame), snap_alpha) {
            Ok(made) => {
                span_seconds.push(timer.elapsed().as_secs_f64());
                let multiply_adds = upscaler.multiply_adds(frame.width, frame.height)? as f64;
                span_model.push(MS_PER_DISPATCH * SPAN_DISPATCHES + multiply_adds / 1e9 * MS_PER_GIGA_MULTIPLY_ADD);
                upscaled.push(trimmed(&made));
            }
            Err(error) => {
                println!("  SPAN skipped a {}x{} frame: {error:#}", frame.width, frame.height);
                result.span_skipped += 1;
                span_seconds.push(0.0);
                span_model.push(0.0);
                upscaled.push(None);
            }
        }
    }
    let x4_factor = RIFE_X4 as usize;
    let pick = |keep: &dyn Fn(usize) -> bool| -> (Vec<SequenceFrame>, f64, f64) {
        let mut frames = Vec::new();
        let (mut seconds, mut model) = (0.0, 0.0);
        for (at, frame) in upscaled.iter().enumerate() {
            if keep(at) {
                seconds += span_seconds[at];
                model += span_model[at];
                frames.extend(frame.clone());
            }
        }
        (frames, seconds, model)
    };
    let one_frame = frames.len() < 2;
    let own = |at: usize| one_frame || at % x4_factor == 0;
    let (span, span_s, span_m) = pick(&own);
    let (span_x2, span_x2_s, span_x2_m) = pick(&|at| own(at) || at % x4_factor == x4_factor / 2);
    let (span_x4, span_x4_s, span_x4_m) = pick(&|_| true);
    for (variant, gpu, model) in [(Variant::Span2x, span_s, span_m), (Variant::RifeX2, rife_x2, 0.0),
        (Variant::RifeX4, rife_x4, 0.0), (Variant::Span2xRifeX2, rife_x2 + span_x2_s, span_x2_m),
        (Variant::Span2xRifeX4, rife_x4 + span_x4_s, span_x4_m)] {
        *result.gpu.entry(variant).or_default() += gpu;
        *result.span_model_ms.entry(variant).or_default() += model;
    }
    let by_variant = BTreeMap::from([(Variant::Original, frames), (Variant::Span2x, span), (Variant::RifeX2, x2),
        (Variant::RifeX4, x4), (Variant::Span2xRifeX2, span_x2), (Variant::Span2xRifeX4, span_x4)]);
    Ok(ClipFrames { by_variant })
}

#[test]
#[ignore = "needs AOT_PACKAGE and a GPU; minutes to hours"]
fn ahead_of_time_motion_costs() -> eyre::Result<()> {
    let Some(path) = std::env::var_os(PACKAGE_VARIABLE).map(PathBuf::from) else {
        println!("skipped: {PACKAGE_VARIABLE} is not set");
        return Ok(());
    };
    let samples = std::env::var(SAMPLES_VARIABLE).ok().and_then(|value| value.parse().ok()).unwrap_or(DEFAULT_SAMPLES);
    let package_bytes = std::fs::metadata(&path)?.len();
    let package = Package::load(&path)?;
    let snap_alpha = package.classic();
    let (atlas_width, atlas_height, gutter) = package.page_layout();
    let options = MobileAnimEcAtlasOptions { atlas_width, atlas_height, gutter, pixel_format: PagePixelFormat::Bc7,
        compression: CompressionFlag::ZstdNoDictLevel(ZSTD_LEVEL), ..Default::default() };

    let mut clips = package.stored_clips();
    clips.sort_by_key(|clip| clip.texels);
    let total_texels: u64 = clips.iter().map(|clip| clip.texels).sum();
    let total_frames: u64 = clips.iter().map(|clip| u64::from(clip.frames)).sum();
    println!("{}: {} bytes, {} stored clips, {} frames, {:.1} Mtexels at 1x, pages {}x{} gutter {gutter}",
        path.display(), package_bytes, clips.len(), total_frames, total_texels as f64 / 1e6, atlas_width, atlas_height);
    let mut strata: Vec<Vec<StoredClip>> = vec![Vec::new(); STRATA];
    let mut running = 0u64;
    for clip in &clips {
        let stratum = ((running as u128 * STRATA as u128) / total_texels.max(1) as u128) as usize;
        strata[stratum.min(STRATA - 1)].push(*clip);
        running += clip.texels;
    }

    let mut maker = InBetweenMaker::new(|_| {})?;
    let mut upscaler = FrameUpscaler::new(UPSCALER_MAX_SIDE)?;
    println!("RIFE on {}, SPAN on {}", maker.adapter(), upscaler.adapter());
    let mut random = XorShift(SAMPLE_SEED);
    let mut results = Vec::new();
    for (stratum_index, stratum) in strata.iter().enumerate() {
        let mut pool: Vec<StoredClip> = stratum.clone();
        let mut chosen = Vec::new();
        while chosen.len() < samples && !pool.is_empty() {
            chosen.push(pool.swap_remove(random.below(pool.len())));
        }
        let mut result = StratumResult { full_texels: stratum.iter().map(|clip| clip.texels).sum(),
            full_clips: stratum.len(), sample_clips: chosen.len(), ..Default::default() };
        println!("stratum {stratum_index}: {} clips of {}..{} texels, {:.1} Mtexels; sampling {}", stratum.len(),
            stratum.first().map_or(0, |clip| clip.texels), stratum.last().map_or(0, |clip| clip.texels),
            result.full_texels as f64 / 1e6, chosen.len());
        let mut variant_frames: BTreeMap<Variant, Vec<DecodedMobileAnimEcFrame>> = BTreeMap::new();
        for (clip_index, stored) in chosen.iter().enumerate() {
            let Some(clip) = package.clip(stored.record)? else { continue };
            result.sample_texels += stored.texels;
            let made = measure_clip(&clip, snap_alpha, &mut maker, &mut upscaler, &mut result)?;
            for (variant, frames) in &made.by_variant {
                let mut index = 0u16;
                let list = variant_frames.entry(*variant).or_default();
                list.extend(frames.iter().map(|frame| decoded(clip_index as u32, &mut index, frame)));
            }
        }
        for variant in VARIANTS {
            let frames = variant_frames.remove(&variant).unwrap_or_default();
            let (stored, raw, pages, pack, bc7, zstd) = pack_and_store(frames, &options)?;
            result.bytes.insert(variant, stored);
            result.raw_bytes.insert(variant, raw);
            result.pages.insert(variant, pages);
            result.pack.insert(variant, pack);
            result.bc7.insert(variant, bc7);
            result.zstd.insert(variant, zstd);
            println!("  {variant:?}: {pages} pages, {:.2} MB BC7, {:.2} MB stored; GPU {:.1} s (SPAN model {:.1} s), \
                pack {pack:.1} s, BC7 {bc7:.1} s, Zstd {zstd:.1} s", raw as f64 / BYTES_PER_MB,
                stored as f64 / BYTES_PER_MB, result.gpu.get(&variant).copied().unwrap_or(0.0),
                result.span_model_ms.get(&variant).copied().unwrap_or(0.0) / 1e3);
        }
        results.push(result);
    }

    // Extrapolated: each stratum scaled by its texels over its sample's; bytes calibrated to the package at 1x.
    let scaled = |pick: &dyn Fn(&StratumResult) -> f64| -> f64 {
        results.iter().filter(|result| result.sample_texels > 0)
            .map(|result| pick(result) * result.full_texels as f64 / result.sample_texels as f64).sum()
    };
    // GPU time also scaled by clips, as its per-frame overhead goes by frames rather than texels.
    let by_clips = |pick: &dyn Fn(&StratumResult) -> f64| -> f64 {
        results.iter().filter(|result| result.sample_clips > 0)
            .map(|result| pick(result) * result.full_clips as f64 / result.sample_clips as f64).sum()
    };
    let original = scaled(&|result| result.bytes[&Variant::Original] as f64);
    let calibration = package_bytes as f64 / original;
    let original_raw = scaled(&|result| result.raw_bytes[&Variant::Original] as f64);
    let page_bytes = u64::from(atlas_width) * u64::from(atlas_height);
    println!("\nwhole package (calibration {calibration:.3}, {} pages at 1x); times in minutes on this machine",
        package.page_count());
    println!("| variant | size MB | x 1x | pages | BC7 page GiB | GPU min (by texels) | GPU min (by clips) | \
        SPAN model min | pack min | BC7 min | Zstd min |");
    for variant in VARIANTS {
        let bytes = scaled(&|result| result.bytes[&variant] as f64) * calibration;
        let pages = package.page_count() as f64 * scaled(&|result| result.raw_bytes[&variant] as f64) / original_raw;
        let minutes = |map: &dyn Fn(&StratumResult) -> f64| scaled(map) / 60.0;
        println!("| {variant:?} | {:.0} | {:.2} | {pages:.0} | {:.2} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
            bytes / BYTES_PER_MB, bytes / package_bytes as f64, pages * page_bytes as f64 / f64::from(1u32 << 30),
            minutes(&|result| result.gpu.get(&variant).copied().unwrap_or(0.0)),
            by_clips(&|result| result.gpu.get(&variant).copied().unwrap_or(0.0)) / 60.0,
            minutes(&|result| result.span_model_ms.get(&variant).copied().unwrap_or(0.0) / 1e3),
            minutes(&|result| result.pack[&variant]), minutes(&|result| result.bc7[&variant]),
            minutes(&|result| result.zstd[&variant]));
    }
    let skipped: usize = results.iter().map(|result| result.span_skipped).sum();
    println!("SPAN skipped {skipped} frames");
    for (index, result) in results.iter().enumerate() {
        let ratio = |variant| result.bytes[&variant] as f64 / result.bytes[&Variant::Original] as f64;
        println!("stratum {index}: {} of {} clips; stored x1: SPAN {:.2}, RIFE x2 {:.2}, x4 {:.2}, SPAN+x2 {:.2}, \
            SPAN+x4 {:.2}", result.sample_clips, result.full_clips, ratio(Variant::Span2x), ratio(Variant::RifeX2),
            ratio(Variant::RifeX4), ratio(Variant::Span2xRifeX2), ratio(Variant::Span2xRifeX4));
    }
    Ok(())
}

/// Zstd levels to compare on stored BC7 pages.
const COMPARED_ZSTD_LEVELS: [i32; 6] = [3, 7, 9, 12, 15, 19];
/// Every this many pages of the package is compared.
const COMPARED_PAGE_STEP: usize = 16;

/// The package size, compression time and decode time of the BC7 pages at each of [`COMPARED_ZSTD_LEVELS`], on every
/// [`COMPARED_PAGE_STEP`]th page of `AOT_PACKAGE` (a BC7 mobile animation package).
#[test]
#[ignore = "needs AOT_PACKAGE, a BC7 mobile animation package"]
fn mobile_page_zstd_levels() -> eyre::Result<()> {
    let Some(path) = std::env::var_os(PACKAGE_VARIABLE).map(PathBuf::from) else {
        println!("skipped: {PACKAGE_VARIABLE} is not set");
        return Ok(());
    };
    let package = Package::load(&path)?;
    let pages: Vec<Vec<u8>> = match &package {
        Package::Classic(package) => package.pages().iter().step_by(COMPARED_PAGE_STEP)
            .map(|page| package.read_page_bytes(page.page_index)).collect::<eyre::Result<_>>()?,
        Package::Enhanced(package) => package.pages().iter().step_by(COMPARED_PAGE_STEP)
            .map(|page| package.read_page_bytes(page.page_index)).collect::<eyre::Result<_>>()?,
    };
    let raw: u64 = pages.iter().map(|page| page.len() as u64).sum();
    println!("{}: {} pages compared, {:.1} MB of blocks", path.display(), pages.len(), raw as f64 / BYTES_PER_MB);
    println!("| level | MB | x raw | compress s (all threads) | decode ms per page (one thread) |");
    for level in COMPARED_ZSTD_LEVELS {
        let timer = Instant::now();
        let compressed: Vec<Vec<u8>> = pages.par_iter().map(|page| zstd::bulk::compress(page, level))
            .collect::<std::io::Result<_>>()?;
        let compress = timer.elapsed().as_secs_f64();
        let timer = Instant::now();
        for (page, bytes) in pages.iter().zip(&compressed) {
            let decoded = zstd::bulk::decompress(bytes, page.len())?;
            assert_eq!(decoded.len(), page.len());
        }
        let decode_ms = timer.elapsed().as_secs_f64() * 1e3 / pages.len() as f64;
        let stored: u64 = compressed.iter().map(|bytes| bytes.len() as u64).sum();
        println!("| {level} | {:.1} | {:.3} | {compress:.1} | {decode_ms:.2} |", stored as f64 / BYTES_PER_MB,
            stored as f64 / raw as f64);
    }
    Ok(())
}
