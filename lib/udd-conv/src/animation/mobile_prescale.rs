//! The 2x frames of a mobile animation package made ahead of time (`--prescale`), stored beside its 1x frames
//! ([`udd_assets::animation::mobile_upscaled`]). Off by default and not recommended: Dynamapper makes the same 2x
//! frames at run time; packed, they grow the package a lot (4 times the texels of the frames chosen and their border:
//! the Classic bodies of at most 48 pixels a side add 41 % to a BC7 Classic package, 127 % to one of RGBA pages),
//! for bodies whose run-time upscale would show for a moment on screen.
//!
//! The stage runs on a package already written (the packers call it last; `udd-cli tool prescale-mobile-anims` on a
//! package as it is): it upscales the stored 1x frames, as decoded from the pages, which is what the renderer upscales
//! at run time from its frame atlas, with the same network ([`FrameUpscaler`]), canvas and border, on the workspace of
//! the frame's clip (its animation record's largest frame), in tiles past the largest canvas upscaled whole. So the
//! packaged 2x frames are the run-time ones but for the page encode (BC7 here at the packer's quality, the renderer's
//! fast GPU BC7 there). The 1x pages and records are copied as stored, the strides with them.
//!
//! Which frames: those of the bodies `MobilePrescale.kdl` lists as "always" ([`super::prescale_table`]), and with
//! `max_side` every body whose largest stored frame is at most that many pixels wide and high, but those listed as
//! "never". A body is followed to its stored frames, as `MobileFrameFilters.kdl`'s are, and chosen whole.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use color_eyre::eyre::{self, WrapErr};
use log::{info, warn};
use rayon::prelude::*;
use udd_assets::animation::mobile_strides::PageMemo;
use udd_assets::animation::mobile_upscaled::{
    upscaled_page_entry_path, UpscaledFrameRecord, UpscaledFrameTable, UpscaledPageRecord,
    UPSCALED_FRAMES_ENTRY_PATH,
};
use udd_assets::mobile_anim_cc::MOBILE_ANIM_CC_FLAG_UOP_SOURCE;
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::{MobileAnimCcPackage, MobileAnimEcPackage};
use udd_container::{AddOwnedFileRequest, CompressedEntry, CompressionFlag, DataType, UddpBuilder, UddpReader};
use uot_gpu_compute::frame_upscales::{upscale_workspace_canvas, FrameUpscaler};
use uot_gpu_compute::strides::StrideFrame;

use super::frame_filter_table::FramePackage;
use super::prescale_table::{load_mobile_prescale, BodyPrescale, ResolvedPrescale};
use crate::atlas::bc_encode_device::on_bc7_devices;
use crate::page_compression::mobile_anim_page_compression;
use crate::bc7::{
    encode_for_vram_with_bc7_rdo_options_and_stage_progress_timed, preferred_bc7_encoder_backend, Bc7RdoOptions,
    ImageExtent, RawImageFormat, VramTextureEncoding,
};

/// The table's compression; the pages take the packer's.
const TABLE_COMPRESSION: CompressionFlag = CompressionFlag::ZstdNoDict;
/// The 2x canvases start on BC7 block corners and take whole blocks, in either format, so a frame's blocks are its own.
const SLOT_ALIGN: u32 = 4;
const RGBA_BYTES: usize = 4;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;
/// Bytes a 2x texel takes in a page before compression, by format: what the size estimate counts.
const BC7_TEXEL_BYTES: u64 = 1;
const RGBA8_TEXEL_BYTES: u64 = 4;
/// The 2x pages kept before they are encoded together, one a thread.
fn encode_batch() -> usize {
    rayon::current_num_threads().max(1)
}

/// Whether and which frames get their 2x frames ahead of time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrescaleOptions {
    /// Make them at all. Off by default.
    pub enabled: bool,
    /// The size rule: each body whose largest stored frame is at most this many pixels wide and high. `None`: the
    /// table's "always" bodies alone.
    pub max_side: Option<u16>,
    /// The MobilePrescale.kdl to read; when unset, the copy shipped with the tools.
    pub table_path: Option<PathBuf>,
}

/// How the 2x pages are stored: the package's own page format, with the packer's compression and BC7 RDO.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrescaleEncoding {
    pub compression: CompressionFlag,
    pub bc7_rdo_lambda: f32,
    pub bc7_rdo_lookback_blocks: usize,
}

/// How the 2x frames made compare with the run time's, measured when asked ([`prescale_package`]'s `compare`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PrescaleComparison {
    /// Texels of the 2x canvases.
    pub texels: u64,
    /// Texels whose upscale on the frame's own canvas differs from the one on its clip's (the run time's).
    pub canvas_differing: u64,
    /// The largest such difference, in 8-bit steps.
    pub canvas_worst: u8,
    /// Texels the stored page changes from the upscale (the page encode), and the PSNR of the stored frames against
    /// the upscale over RGBA, in dB (infinite when equal).
    pub encode_differing: u64,
    pub encode_psnr_db: f64,
}

/// What a prescale did.
#[derive(Clone, Debug, Default)]
pub struct PrescaleSummary {
    /// The stored frame sets (a body's frames, shared by the bodies drawn with them) chosen.
    pub frame_sets: usize,
    pub frames: usize,
    pub pages: usize,
    /// The package's size before and after.
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub upscale_time: Duration,
    pub encode_time: Duration,
    pub total_time: Duration,
    pub adapter: String,
    pub comparison: Option<PrescaleComparison>,
}

/// The stored frames a body is drawn with: what a choice applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum FramesKey {
    /// A Classic body's records: from `AnimationFrame*.uop` or `anim*.mul` file `file`.
    Classic { uop: bool, file: u8, body: u16 },
    /// An Enhanced `AnimationFrame` body.
    Enhanced(u32),
}

impl FramesKey {
    fn describe(self) -> String {
        match self {
            FramesKey::Classic { uop: true, body, .. } => format!("AnimationFrame body {body}"),
            FramesKey::Classic { uop: false, file, body } => format!("anim file {} body {body}", file + 1),
            FramesKey::Enhanced(body) => format!("AnimationFrame body {body}"),
        }
    }
}

/// A frame record as the stage reads it.
#[derive(Clone, Copy, Debug)]
struct FrameInfo {
    index: u32,
    key: FramesKey,
    /// The animation record that owns the frame: its clip.
    clip: u32,
    page: u32,
    width: u16,
    height: u16,
}

enum Package {
    Classic(MobileAnimCcPackage),
    Enhanced(MobileAnimEcPackage),
}

impl Package {
    fn load(path: &Path) -> eyre::Result<Self> {
        let classic = match MobileAnimCcPackage::load(path) {
            Ok(package) => return Ok(Self::Classic(package)),
            Err(error) => error,
        };
        MobileAnimEcPackage::load(path).map(Self::Enhanced).map_err(|enhanced| eyre::eyre!(
            "{} is not a mobile animation package: as Classic, {classic:#}; as Enhanced, {enhanced:#}",
            path.display()))
    }

    fn kind(&self) -> FramePackage {
        match self {
            Self::Classic(_) => FramePackage::Cc,
            Self::Enhanced(_) => FramePackage::Ec,
        }
    }

    fn pixel_format(&self) -> PagePixelFormat {
        match self {
            Self::Classic(package) => package.pixel_format(),
            Self::Enhanced(package) => package.pixel_format(),
        }
    }

    fn atlas_size(&self) -> (u32, u32) {
        match self {
            Self::Classic(package) => (package.atlas_width(), package.atlas_height()),
            Self::Enhanced(package) => (package.atlas_width(), package.atlas_height()),
        }
    }

    fn upscaled(&self) -> Option<&UpscaledFrameTable> {
        match self {
            Self::Classic(package) => package.upscaled_frames(),
            Self::Enhanced(package) => package.upscaled_frames(),
        }
    }

    /// The frame records with pixels.
    fn frames(&self) -> Vec<FrameInfo> {
        let missing = udd_assets::mobile_anim_cc::MISSING_PAGE_INDEX;
        match self {
            Self::Classic(package) => package.frames().iter().enumerate().filter_map(|(index, frame)| {
                let animation = package.animations().get(frame.animation_index as usize)?;
                let key = FramesKey::Classic { uop: animation.flags & MOBILE_ANIM_CC_FLAG_UOP_SOURCE != 0,
                    file: animation.file_index, body: animation.body_id };
                (frame.width > 0 && frame.height > 0 && frame.page_index != missing).then_some(FrameInfo {
                    index: index as u32, key, clip: frame.animation_index, page: frame.page_index,
                    width: frame.width, height: frame.height,
                })
            }).collect(),
            Self::Enhanced(package) => package.frames().iter().enumerate().filter_map(|(index, frame)| {
                let animation = package.animations().get(frame.animation_index as usize)?;
                (frame.width > 0 && frame.height > 0 && frame.page_index != missing).then_some(FrameInfo {
                    index: index as u32, key: FramesKey::Enhanced(animation.source_body_id),
                    clip: frame.animation_index, page: frame.page_index, width: frame.width, height: frame.height,
                })
            }).collect(),
        }
    }

    /// The stored frames the client draws server `body` with.
    fn frames_of(&self, body: u16) -> Option<FramesKey> {
        match self {
            Self::Classic(package) => package.body(body).map(|body| FramesKey::Classic {
                uop: body.uses_uop_frames(),
                file: body.file_index,
                body: body.frames_body_id,
            }),
            Self::Enhanced(package) => package.body(u32::from(body)).map(|body| FramesKey::Enhanced(body.frames_body_id)),
        }
    }

    fn read_frame_rgba(&self, index: u32, pages: &mut PageMemo) -> eyre::Result<Vec<u8>> {
        match self {
            Self::Classic(package) => package.read_frame_rgba_in(&package.frames()[index as usize], pages),
            Self::Enhanced(package) => package.read_frame_rgba_in(&package.frames()[index as usize], pages),
        }
    }
}

/// The frames chosen: of each frame set chosen, its frames.
fn choose_frames(package: &Package, options: &PrescaleOptions) -> eyre::Result<(usize, Vec<FrameInfo>)> {
    let frames = package.frames();
    let mut largest = BTreeMap::<FramesKey, u16>::new();
    for frame in &frames {
        let side = largest.entry(frame.key).or_default();
        *side = (*side).max(frame.width).max(frame.height);
    }
    let (path, table) = load_mobile_prescale(options.table_path.as_deref())?;
    println!("Using the mobile prescale table: {}", path.display());
    let kind = package.kind();
    let resolved: ResolvedPrescale<FramesKey> = table.resolve(kind,
        |body| package.frames_of(body).filter(|key| largest.contains_key(key)), FramesKey::describe)?;
    resolved.log(kind, FramesKey::describe);
    let chosen = largest.iter().filter(|&(&key, &side)| match resolved.choice(key) {
        Some(BodyPrescale::Always) => true,
        Some(BodyPrescale::Never) => false,
        None => options.max_side.is_some_and(|max_side| side <= max_side),
    }).map(|(&key, _)| key).collect::<HashSet<_>>();
    Ok((chosen.len(), frames.into_iter().filter(|frame| chosen.contains(&frame.key)).collect()))
}

/// A 2x page being filled: shelves left to right, top down.
struct PageFill {
    width: u32,
    height: u32,
    shelves: Vec<(u32, u32, u32)>,
    next_shelf_y: u32,
    used: (u32, u32),
    rgba: Vec<u8>,
    frames: Vec<UpscaledFrameRecord>,
}

impl PageFill {
    fn new(width: u32, height: u32) -> Self {
        Self { width, height, shelves: Vec::new(), next_shelf_y: 0, used: (0, 0),
            rgba: vec![0; width as usize * height as usize * RGBA_BYTES], frames: Vec::new() }
    }

    /// Room for a `width` by `height` canvas, in whole blocks.
    fn place(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        let (width, height) = (width.next_multiple_of(SLOT_ALIGN), height.next_multiple_of(SLOT_ALIGN));
        let at = match self.shelves.iter_mut()
            .find(|(_, shelf_height, next_x)| *shelf_height >= height && *next_x + width <= self.width) {
            Some((y, _, next_x)) => {
                let at = (*next_x, *y);
                *next_x += width;
                at
            }
            None if self.next_shelf_y + height <= self.height && width <= self.width => {
                let y = self.next_shelf_y;
                self.next_shelf_y += height;
                self.shelves.push((y, height, width));
                (0, y)
            }
            None => return None,
        };
        self.used = (self.used.0.max(at.0 + width), self.used.1.max(at.1 + height));
        Some(at)
    }

    fn blit(&mut self, x: u32, y: u32, width: u32, rgba: &[u8]) {
        let row = width as usize * RGBA_BYTES;
        let pitch = self.width as usize * RGBA_BYTES;
        for (line, source) in rgba.chunks_exact(row).enumerate() {
            let at = (y as usize + line) * pitch + x as usize * RGBA_BYTES;
            self.rgba[at..at + row].copy_from_slice(source);
        }
    }

    /// The used part of the page, rows of `used.0` texels.
    fn used_rgba(&self) -> Vec<u8> {
        let (used_width, used_height) = self.used;
        let pitch = self.width as usize * RGBA_BYTES;
        self.rgba.chunks_exact(pitch).take(used_height as usize)
            .flat_map(|row| &row[..used_width as usize * RGBA_BYTES]).copied().collect()
    }
}

/// A finished 2x page: its record, its frames and its used texels.
struct FinishedPage {
    index: u32,
    record: UpscaledPageRecord,
    frames: Vec<UpscaledFrameRecord>,
    rgba: Vec<u8>,
}

fn finish(fill: PageFill, index: u32) -> FinishedPage {
    let rgba = fill.used_rgba();
    FinishedPage { index, record: UpscaledPageRecord { used_width: fill.used.0, used_height: fill.used.1 },
        frames: fill.frames, rgba }
}

/// A 2x page encoded and compressed for the package: its record, its frames and its entry.
struct EncodedPage {
    index: u32,
    record: UpscaledPageRecord,
    frames: Vec<UpscaledFrameRecord>,
    entry: CompressedEntry,
}

/// Encodes `pages` in the package's format and compresses them, one a thread, adding the encode's difference to
/// `comparison`. Only the stored bytes are kept, so the pages made wait for the write in about their share of the
/// package.
fn encode_pages(pages: Vec<FinishedPage>, format: PagePixelFormat, encoding: &PrescaleEncoding,
    comparison: Option<&mut EncodeTally>) -> eyre::Result<Vec<EncodedPage>> {
    let rdo = on_bc7_devices(Bc7RdoOptions::sparse_atlas(encoding.bc7_rdo_lambda, encoding.bc7_rdo_lookback_blocks));
    let compare = comparison.is_some();
    let encoded = pages.into_par_iter().map(|mut page| {
        let mut tally = EncodeTally::default();
        let bytes = match format {
            PagePixelFormat::Rgba8888 => std::mem::take(&mut page.rgba),
            PagePixelFormat::Bc7 => {
                let extent = ImageExtent::new(page.record.used_width, page.record.used_height)
                    .map_err(|error| eyre::eyre!("{error}"))?;
                let (encoded, _) = encode_for_vram_with_bc7_rdo_options_and_stage_progress_timed(&page.rgba, extent,
                    RawImageFormat::Rgba8888, VramTextureEncoding::Bc7(preferred_bc7_encoder_backend()), &rdo,
                    |_, _| {})
                    .map_err(|error| eyre::eyre!("BC7 encode upscaled page {}: {error}", page.index))?;
                let bytes = encoded.into_bytes().to_vec();
                if compare {
                    let decoded = udd_assets::bc7::decode_bc7_to_rgba8888(&bytes, extent)
                        .map_err(|error| eyre::eyre!("decode upscaled page {}: {error}", page.index))?;
                    tally.add(&page, &decoded, extent.width());
                }
                bytes
            }
        };
        let entry = CompressedEntry::new(AddOwnedFileRequest { data_type: DataType::Texture as u8,
            compression: mobile_anim_page_compression(encoding.compression, format), width: page.record.used_width,
            height: page.record.used_height,
            virtual_path: Some(upscaled_page_entry_path(page.index, format)), path_hash64: None, id: None,
            data: bytes })
            .map_err(|error| eyre::eyre!("compress upscaled page {}: {error}", page.index))?;
        Ok((EncodedPage { index: page.index, record: page.record, frames: page.frames, entry }, tally))
    }).collect::<eyre::Result<Vec<_>>>()?;
    if let Some(comparison) = comparison {
        for (_, tally) in &encoded {
            comparison.merge(tally);
        }
    }
    Ok(encoded.into_iter().map(|(page, _)| page).collect())
}

/// The page encode's difference over the frames' texels.
#[derive(Default)]
struct EncodeTally {
    differing: u64,
    squared_error: f64,
    samples: u64,
}

impl EncodeTally {
    fn merge(&mut self, other: &Self) {
        self.differing += other.differing;
        self.squared_error += other.squared_error;
        self.samples += other.samples;
    }

    fn add(&mut self, page: &FinishedPage, decoded: &[u8], decoded_width: u32) {
        let pitch = page.record.used_width as usize * RGBA_BYTES;
        let decoded_pitch = decoded_width as usize * RGBA_BYTES;
        for frame in &page.frames {
            for y in 0..usize::from(frame.height) {
                let at = |pitch: usize| (usize::from(frame.y) + y) * pitch + usize::from(frame.x) * RGBA_BYTES;
                let bytes = usize::from(frame.width) * RGBA_BYTES;
                let made = &page.rgba[at(pitch)..at(pitch) + bytes];
                let stored = &decoded[at(decoded_pitch)..at(decoded_pitch) + bytes];
                for (made, stored) in made.chunks_exact(RGBA_BYTES).zip(stored.chunks_exact(RGBA_BYTES)) {
                    self.differing += u64::from(made != stored);
                    for (a, b) in made.iter().zip(stored) {
                        self.squared_error += (f64::from(*a) - f64::from(*b)).powi(2);
                    }
                    self.samples += RGBA_BYTES as u64;
                }
            }
        }
    }

    fn psnr_db(&self) -> f64 {
        const PEAK: f64 = 255.0;
        if self.squared_error == 0.0 {
            return f64::INFINITY;
        }
        10.0 * (PEAK * PEAK / (self.squared_error / self.samples as f64)).log10()
    }
}

/// The bytes a 2x canvas of a `width` by `height` frame takes in a page of `format` before compression.
fn canvas_bytes(width: u16, height: u16, format: PagePixelFormat) -> u64 {
    let (canvas_width, canvas_height) = UpscaledFrameTable::canvas_size(width, height);
    let texel = match format {
        PagePixelFormat::Bc7 => BC7_TEXEL_BYTES,
        PagePixelFormat::Rgba8888 => RGBA8_TEXEL_BYTES,
    };
    u64::from(canvas_width.next_multiple_of(SLOT_ALIGN)) * u64::from(canvas_height.next_multiple_of(SLOT_ALIGN)) * texel
}

/// Makes the 2x frames `options` choose of the package at `path` and writes the package with them to `output` (which
/// may be `path`), through a partial file beside it, replacing any 2x frames it had; without any chosen, `output` is
/// the package without them. Off unless `options.enabled`. With `compare`, measures the 2x frames against the
/// upscale on each frame's own canvas and the page encode's difference ([`PrescaleComparison`]).
pub fn prescale_package(path: &Path, output: &Path, options: &PrescaleOptions, encoding: &PrescaleEncoding,
    compare: bool) -> eyre::Result<PrescaleSummary> {
    let started = Instant::now();
    let mut summary = PrescaleSummary {
        bytes_before: std::fs::metadata(path).wrap_err_with(|| format!("read {}", path.display()))?.len(),
        ..Default::default()
    };
    if !options.enabled {
        return Ok(summary);
    }
    let package = Package::load(path)?;
    let format = package.pixel_format();
    let (frame_sets, mut frames) = choose_frames(&package, options)?;
    summary.frame_sets = frame_sets;
    summary.frames = frames.len();
    let estimate = frames.iter().map(|frame| canvas_bytes(frame.width, frame.height, format)).sum::<u64>();
    warn!("Prescale: {} frames of {} bodies' frame sets of {} get 2x frames packed beside them: about {:.1} MiB of \
        pages before compression, +{:.0} % of the package's {:.1} MiB. The renderer makes the same 2x frames at run \
        time; packed, they only spare it the upscale of these bodies.", frames.len(), frame_sets, path.display(),
        estimate as f64 / BYTES_PER_MIB, estimate as f64 * 100.0 / summary.bytes_before.max(1) as f64,
        summary.bytes_before as f64 / BYTES_PER_MIB);

    // Each clip's workspace canvas, as the renderer's job has it: its largest frame's.
    let mut clip_canvas = BTreeMap::<u32, [u32; 2]>::new();
    for frame in package.frames() {
        let canvas = clip_canvas.entry(frame.clip).or_insert([0; 2]);
        let own = upscale_workspace_canvas(u32::from(frame.width), u32::from(frame.height));
        *canvas = [canvas[0].max(own[0]), canvas[1].max(own[1])];
    }
    // The frames by page, so each 1x page is read once, the tallest first within it.
    frames.sort_by_key(|frame| (frame.page, std::cmp::Reverse(frame.height), frame.index));
    let snap_alpha = package.kind() == FramePackage::Cc;
    let max_side = frames.iter().map(|frame| u32::from(frame.width.max(frame.height))).max().unwrap_or(1);
    let (atlas_width, atlas_height) = package.atlas_size();

    let mut pages_out = Vec::<EncodedPage>::new();
    let mut tally = compare.then(EncodeTally::default);
    let mut canvas_comparison = PrescaleComparison::default();
    if !frames.is_empty() {
        let mut upscaler = FrameUpscaler::new(max_side)?;
        summary.adapter = upscaler.adapter().to_string();
        info!("Prescale: upscaling on {}", summary.adapter);
        let mut memo = PageMemo::default();
        let mut fill = PageFill::new(atlas_width, atlas_height);
        let mut pending = Vec::new();
        let mut next_page = 0u32;
        for (done, frame) in frames.iter().enumerate() {
            let rgba = package.read_frame_rgba(frame.index, &mut memo)?;
            let stride_frame = StrideFrame { width: frame.width, height: frame.height, anchor_x: 0, anchor_y: 0,
                rgba: &rgba };
            let upscale_started = Instant::now();
            let canvas = clip_canvas[&frame.clip];
            let upscaled = upscaler.upscale_on(&stride_frame, snap_alpha, canvas)
                .wrap_err_with(|| format!("upscale frame record {}", frame.index))?;
            if compare {
                let own = upscaler.upscale(&stride_frame, snap_alpha)?;
                for (a, b) in upscaled.rgba.iter().zip(&own.rgba) {
                    canvas_comparison.canvas_worst = canvas_comparison.canvas_worst.max(a.abs_diff(*b));
                }
                canvas_comparison.canvas_differing += upscaled.rgba.chunks_exact(RGBA_BYTES)
                    .zip(own.rgba.chunks_exact(RGBA_BYTES)).filter(|(a, b)| a != b).count() as u64;
                canvas_comparison.texels += u64::from(upscaled.width * upscaled.height);
            }
            summary.upscale_time += upscale_started.elapsed();
            let at = match fill.place(upscaled.width, upscaled.height) {
                Some(at) => at,
                None => {
                    pending.push(finish(std::mem::replace(&mut fill, PageFill::new(atlas_width, atlas_height)),
                        next_page));
                    next_page += 1;
                    if pending.len() >= encode_batch() {
                        let encode_started = Instant::now();
                        pages_out.extend(encode_pages(std::mem::take(&mut pending), format, encoding, tally.as_mut())?);
                        summary.encode_time += encode_started.elapsed();
                    }
                    fill.place(upscaled.width, upscaled.height).ok_or_else(|| eyre::eyre!(
                        "a {}x{} 2x canvas does not fit a {atlas_width}x{atlas_height} page", upscaled.width,
                        upscaled.height))?
                }
            };
            fill.blit(at.0, at.1, upscaled.width, &upscaled.rgba);
            let side = |value: u32| u16::try_from(value).map_err(|_| eyre::eyre!("a 2x canvas side of {value}"));
            fill.frames.push(UpscaledFrameRecord { frame: frame.index, page: next_page, x: side(at.0)?, y: side(at.1)?,
                width: side(upscaled.width)?, height: side(upscaled.height)? });
            if (done + 1) % PROGRESS_EVERY == 0 {
                info!("Prescale: {} of {} frames upscaled", done + 1, frames.len());
            }
        }
        pending.push(finish(fill, next_page));
        let encode_started = Instant::now();
        pages_out.extend(encode_pages(pending, format, encoding, tally.as_mut())?);
        summary.encode_time += encode_started.elapsed();
    }
    summary.pages = pages_out.len();
    if let Some(tally) = &tally {
        summary.comparison = Some(PrescaleComparison { encode_differing: tally.differing,
            encode_psnr_db: tally.psnr_db(), ..canvas_comparison });
    }

    // The package with the 2x pages and table, without those it had.
    let base = UddpReader::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
    let old_pages = package.upscaled().map_or(0, |table| table.pages.len() as u32);
    let old_format = package.upscaled().map_or(format, |table| table.pixel_format);
    if pages_out.is_empty() && package.upscaled().is_none() {
        if output != path {
            std::fs::copy(path, output).wrap_err_with(|| format!("copy {} to {}", path.display(), output.display()))?;
        }
        summary.bytes_after = summary.bytes_before;
        summary.total_time = started.elapsed();
        return Ok(summary);
    }
    let mut dropped = (0..old_pages).map(|page| udd_container::xxh64_virtual_path(
        &upscaled_page_entry_path(page, old_format))).collect::<HashSet<_>>();
    let mut table = UpscaledFrameTable { atlas_width, atlas_height, gutter: 0, pixel_format: format,
        pages: Vec::with_capacity(pages_out.len()), frames: Vec::with_capacity(summary.frames) };
    let mut entries = Vec::with_capacity(pages_out.len() + 1);
    pages_out.sort_by_key(|page| page.index);
    for page in pages_out {
        table.pages.push(page.record);
        table.frames.extend(page.frames);
        entries.push(page.entry);
    }
    table.frames.sort_by_key(|record| record.frame);
    if table.frames.is_empty() {
        dropped.insert(udd_container::xxh64_virtual_path(UPSCALED_FRAMES_ENTRY_PATH));
    } else {
        entries.push(CompressedEntry::new(AddOwnedFileRequest { data_type: DataType::Metadata as u8,
            compression: TABLE_COMPRESSION, width: 0, height: 0,
            virtual_path: Some(UPSCALED_FRAMES_ENTRY_PATH.to_string()), path_hash64: None, id: None,
            data: table.encode() }).map_err(|error| eyre::eyre!("compress the 2x frame table: {error}"))?);
    }
    let partial = {
        let mut name = output.as_os_str().to_owned();
        name.push(PARTIAL_SUFFIX);
        PathBuf::from(name)
    };
    let write_started = Instant::now();
    UddpBuilder::write_with_compressed_entries(&base, &entries, |hash| dropped.contains(&hash), &partial)
        .map_err(|error| eyre::eyre!("{error}"))
        .wrap_err_with(|| format!("write {}", partial.display()))?;
    drop(base);
    drop(package);
    std::fs::rename(&partial, output).wrap_err_with(|| format!("replace {}", output.display()))?;
    summary.encode_time += write_started.elapsed();
    summary.bytes_after = std::fs::metadata(output).wrap_err_with(|| format!("read {}", output.display()))?.len();
    summary.total_time = started.elapsed();
    info!("Prescale: {} frames of {} frame sets in {} pages: the package {:.1} MiB -> {:.1} MiB ({:+.2} %), \
        upscale {:.1} s, encode and write {:.1} s, {:.1} s in all.", summary.frames, summary.frame_sets,
        summary.pages, summary.bytes_before as f64 / BYTES_PER_MIB, summary.bytes_after as f64 / BYTES_PER_MIB,
        (summary.bytes_after as f64 / summary.bytes_before.max(1) as f64 - 1.0) * 100.0,
        summary.upscale_time.as_secs_f64(), summary.encode_time.as_secs_f64(), summary.total_time.as_secs_f64());
    Ok(summary)
}

/// Frames between progress lines.
const PROGRESS_EVERY: usize = 2000;
/// The partial file's suffix, beside the output until it replaces it.
const PARTIAL_SUFFIX: &str = ".prescale-partial";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvases_take_whole_blocks_on_a_shelf_and_open_a_shelf_when_full() {
        let mut fill = PageFill::new(64, 32);
        // 2x canvases of 5x5 and 3x3 frames: 18 and 14 texels a side, 20 and 16 in whole blocks.
        assert_eq!(fill.place(18, 18), Some((0, 0)));
        assert_eq!(fill.place(14, 14), Some((20, 0)));
        assert_eq!(fill.place(18, 18), Some((36, 0)));
        // The shelf is full: a new one below, then the page.
        assert_eq!(fill.place(14, 10), Some((0, 20)));
        assert_eq!(fill.place(14, 14), None);
        assert_eq!(fill.used, (56, 32));
        assert!(fill.place(80, 4).is_none(), "wider than the page");
    }

    #[test]
    fn a_blit_lands_at_its_place_and_the_used_part_keeps_it() {
        let mut fill = PageFill::new(8, 8);
        let at = fill.place(2, 2).unwrap();
        fill.blit(at.0, at.1, 2, &[1; 2 * 2 * RGBA_BYTES]);
        let used = fill.used_rgba();
        assert_eq!(used.len(), 4 * 4 * RGBA_BYTES);
        assert_eq!(&used[..2 * RGBA_BYTES], &[1; 2 * RGBA_BYTES]);
        assert_eq!(&used[2 * RGBA_BYTES..4 * RGBA_BYTES], &[0; 2 * RGBA_BYTES]);
    }

    #[test]
    fn the_estimate_counts_whole_blocks_of_the_format() {
        // A 6x6 frame: a 20x20 canvas, in whole blocks 20x20.
        assert_eq!(canvas_bytes(6, 6, PagePixelFormat::Bc7), 400);
        assert_eq!(canvas_bytes(6, 6, PagePixelFormat::Rgba8888), 1600);
        // A 5x5 frame: 18x18, 20x20 in whole blocks.
        assert_eq!(canvas_bytes(5, 5, PagePixelFormat::Bc7), 400);
    }

    #[test]
    fn frame_sets_are_named_by_their_source() {
        assert_eq!(FramesKey::Classic { uop: false, file: 1, body: 12 }.describe(), "anim file 2 body 12");
        assert_eq!(FramesKey::Enhanced(59).describe(), "AnimationFrame body 59");
    }
}
