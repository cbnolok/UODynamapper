//! Lossy conditioning of the BC7 pages, measured against the source frames: the packer's own pages in RGBA8888 (a
//! `--raw` package packed beside the BC7 one, so both share one layout), encoded here with the converter's BC7
//! encoder, then changed by a chain of steps that trade error for a smaller coded page (the steps are
//! `uot_texture_codecs::bc7::condition`'s, which the mobile animation packers apply as `--bc7-lossy`). Measurement only: nothing
//! here writes a package (`--store` writes the pages, which `condition` puts into a copy of the package, and
//! `--ladder-dir` the visual comparison videos, see `ladder.rs`). The findings are in
//! `docs/agent_notes/bc7_anim_supercompression.md`, "Lossy conditioning".
//!
//! A variant is a chain of steps joined by `+`, each applied to the result of the previous one; every step keeps the
//! alpha of every texel exactly as the plain encode decodes it, except the library RDO, whose alpha changes are
//! counted:
//! - `rdo:<lambda>:<look-back blocks|page>`: `uot_texture_codecs::bc7::rdo` on one window over the whole page.
//! - `reuse:<rms>[:<radius>]`: a block takes the block of an adjacent frame of its clip at the same place relative to
//!   the frame anchor (or up to `radius` blocks beside it, default 1), when that block comes earlier on the page and
//!   its error over the visible texels is at most `rms` levels above the plain encode's; the candidate at the offset
//!   the block before it used wins a tie, so rows of taken blocks stay one Zstd match.
//! - `zrdo:<lambda>:<rms>`: whole-block reuse priced by a model of Zstd's sequences: candidates at the last match
//!   offset, to the left, above, in adjacent frames (as `reuse`) and in a hash of earlier blocks of the same mode and
//!   mean over the whole page; the cheapest of distortion plus `lambda` times the estimated bits, within `rms`.
//! - `fields:<lambda>:<rms>`: the same pricing for the `fields` layout: a block takes the endpoint fields of an
//!   earlier block of its mode (its indices picked again), or its index bits (its endpoints fitted again), so one of
//!   its two records repeats in its mode's stream.
//! - `lrdo:<lambda>:<rms>` and `lfields:<lambda>:<rms>`: `zrdo` and `fields` priced by a model of the LZ4 block
//!   format instead (raw literals, a token and a 2-byte offset per match, 255-byte length extensions, no repeat
//!   offsets, a 64 KiB window).
//! - `coarse:<bits>:<rms>`: colour endpoint values rounded to multiples of `1 << bits`, indices picked again.
//! - `flat:<rms>`: an opaque block within `rms` of its mean becomes one solid mode 6 block with zero indices.
//!
//! `--layout aligned` lays the frames out again so that each frame's anchor falls on a block corner: every frame of a
//! clip then shares one block grid relative to its anchor, which `reuse` needs. The extra padding is reported.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use clap::{Args, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use ssimulacra2::{ColorPrimaries, LinearRgb, Rgb, TransferCharacteristic, compute_frame_ssimulacra2};
use udd_assets::animation::move_actions::{GROUP_PEACE, MOVE_ACTION_GROUPS, MOVE_RUN, MOVE_STAND, MOVE_WALK};
use udd_assets::animation::{mobile_anim_cc, mobile_anim_ec};
use uot_texture_codecs::bc7::condition::{Coder, DEFAULT_REUSE_RADIUS, PageConditioner, Placement, Step};
use uot_texture_codecs::bc7::{Bc7EncoderBackend, ImageExtent, RawImageFormat, decode_bc7_to_rgba8888, encode_to_bc7};

use crate::corpus::{Anim, BLOCK_SIDE, Kind, Package};
use crate::ladder;
use crate::repack::{self, SlotFrame};
use crate::study::{Classes, body_sides};
use crate::transforms::BLOCK_BYTES;
use crate::{Method, PERCENT, mib, store};

const RGBA_BYTES: usize = 4;
const ALPHA: usize = 3;
const PEAK: f64 = 255.0;
const COLOUR_CHANNELS: f64 = 3.0;
/// Pages built and measured at once: each holds its source pages, its pixels and every variant's blocks.
const PAGE_THREADS: usize = 20;
/// Pages between progress lines.
const PROGRESS_PAGES: usize = 20;
/// The level `--store` holds a variant's pages at, as the crowd study holds built pages (`study.rs`).
const STORE_ZSTD_LEVEL: i32 = 1;
/// The SSIMULACRA2 canvas: frames are composited over grey on at least this many pixels a side (as `gumps.rs`).
const SSIM_MIN_SIDE: usize = 64;
const SSIM_BACKGROUND: f32 = 128.0 / 255.0;
/// The clip videos: the upscale, the playback rate, the loops and the gap between panels in source pixels.
const VIDEO_SCALE: usize = 4;
const VIDEO_FPS: u32 = 8;
const VIDEO_LOOPS: usize = 6;
const VIDEO_GAP: usize = 4;
const VIDEO_BACKGROUND: [u8; 3] = [96, 96, 96];
const VIDEO_CRF: &str = "14";
/// The font of the `--ladder-dir` videos' labels.
const LABEL_FONT: &str = "/usr/share/fonts/TTF/DejaVuSansMono.ttf";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SlotLayout {
    /// The packer's layout, as both packages store it.
    Stored,
    /// The packer replayed on frames padded so that each anchor falls on a block corner.
    Aligned,
}

#[derive(Args)]
pub struct LossyArgs {
    /// The RGBA8888 package packed beside `bc7` (`pack-mobile-anims --raw`): the source pixels.
    rgba: PathBuf,
    /// The BC7 package: the records, and the reference the plain encode is checked against.
    bc7: PathBuf,
    #[arg(value_enum)]
    kind: Kind,
    #[arg(long, value_enum, default_value = "stored")]
    layout: SlotLayout,
    /// Every n-th page of the layout.
    #[arg(long, default_value_t = 1)]
    every: usize,
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// Comma-separated variants (see the module documentation); the plain encode is always measured.
    #[arg(long, value_delimiter = ',')]
    variants: Vec<String>,
    #[arg(long, value_delimiter = ',', default_value = "96,192,384")]
    bounds: Vec<u32>,
    #[arg(long, value_delimiter = ',', default_value = "none+zstd:9,none+zstd:19,fields+zstd:19,fields+lz4hc:12")]
    methods: Vec<String>,
    /// SSIMULACRA2 on every n-th placement (0: none).
    #[arg(long, default_value_t = 0)]
    ssim_every: usize,
    /// Checks the plain encode against the BC7 package's pages, and the built pixels against the RGBA package's.
    #[arg(long)]
    check: bool,
    /// Writes each variant's pages (stored layout) for `crowd --pages`: `<dir>/<variant>/<page position>.zst`.
    #[arg(long)]
    store: Option<PathBuf>,
    /// Clips to render as side-by-side videos, `body:action:direction`, the action a number or walk, run or stand
    /// (source, plain encode, then each variant).
    #[arg(long = "clip")]
    clips: Vec<String>,
    /// Where the videos go.
    #[arg(long)]
    clip_dir: Option<PathBuf>,
    /// The videos' nearest-neighbour upscale.
    #[arg(long, default_value_t = VIDEO_SCALE)]
    clip_scale: usize,
    /// Writes the visual comparison of the `--clip` clips here: a ladder and a blink video per body (`ladder.rs`).
    #[arg(long)]
    ladder_dir: Option<PathBuf>,
    /// The ladder videos' file name prefix and title, e.g. `classic` or `enhanced`.
    #[arg(long, default_value = "")]
    ladder_prefix: String,
    /// A line under a version's name in the ladder videos, `version=text` (`source`, `plain` or a variant).
    #[arg(long = "note")]
    notes: Vec<String>,
    /// A body's name in the ladder videos' titles and file names, `body=name`.
    #[arg(long = "body-name")]
    body_names: Vec<String>,
    /// The ladder videos' label font.
    #[arg(long, default_value = LABEL_FONT)]
    font: PathBuf,
}

/// A frame's pixels on a page: one placement (Enhanced records of one source frame share it).
#[derive(Debug, Clone, Copy)]
struct Slot {
    page: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// The anchor in the frame's pixels.
    anchor: (i32, i32),
    order: u64,
    class: usize,
}

impl Slot {
    /// The padding before the content that puts the anchor on a block corner.
    fn phase(&self) -> (u32, u32) {
        let side = BLOCK_SIDE as i32;
        ((-self.anchor.0).rem_euclid(side) as u32, (-self.anchor.1).rem_euclid(side) as u32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Action {
    Move,
    Stand,
    Other,
}

const ACTIONS: [Action; 3] = [Action::Move, Action::Stand, Action::Other];

/// The placements and how the clips run through them.
struct Frames {
    slots: Vec<Slot>,
    /// Consecutive frames of a clip, once each: (earlier, later, the clip's action).
    pairs: Vec<(usize, usize, Action)>,
    /// Each placement's neighbours in its clips, with the action.
    adjacent: Vec<Vec<(usize, Action)>>,
    /// Per clip of `--clip`: its placements in order.
    clips: Vec<(String, Vec<usize>)>,
}

fn anchors(package: &Package) -> Vec<(i32, i32)> {
    match &package.anim {
        Anim::Cc(anim) => anim.frames().iter()
            .map(|frame| (i32::from(frame.center_x), i32::from(frame.height) + i32::from(frame.center_y))).collect(),
        Anim::Ec(anim) => anim.frames().iter().map(|frame| (i32::from(frame.center_x), i32::from(frame.center_y))).collect(),
    }
}

/// Every animation record as (body, action, direction, frame start, frame count).
fn records(package: &Package) -> Vec<(u32, u16, u8, u32, u32)> {
    match &package.anim {
        Anim::Cc(anim) => anim.animations().iter().map(|record| (u32::from(record.body_id), record.action_id,
            record.direction, record.frame_start, u32::from(record.frame_count))).collect(),
        Anim::Ec(anim) => anim.animations().iter().map(|record| (record.body_id, record.action_id, record.direction,
            record.frame_start, u32::from(record.frame_count))).collect(),
    }
}

fn action_kind(package: &Package, body: u32, action: u16, memo: &mut HashMap<u32, (HashSet<u16>, HashSet<u16>)>)
    -> Action {
    let (moves, stands) = memo.entry(body).or_insert_with(|| {
        let mut moves = HashSet::new();
        let mut stands = HashSet::new();
        for movement in [MOVE_WALK, MOVE_RUN, MOVE_STAND] {
            let found: Vec<u16> = match &package.anim {
                Anim::Cc(anim) => u16::try_from(body).ok().map(|body| [false, true].iter()
                    .filter_map(|&mounted| anim.movement_action(body, movement, mounted)).collect()).unwrap_or_default(),
                Anim::Ec(anim) => MOVE_ACTION_GROUPS.iter()
                    .filter_map(|&group| anim.move_action(body, group, movement)).collect(),
            };
            if movement == MOVE_STAND { stands.extend(found) } else { moves.extend(found) }
        }
        (moves, stands)
    });
    if moves.contains(&action) {
        Action::Move
    } else if stands.contains(&action) {
        Action::Stand
    } else {
        Action::Other
    }
}

impl Frames {
    fn new(package: &Package, classes: &Classes, clip_names: &[String]) -> eyre::Result<Self> {
        let sides = body_sides(package);
        let anchors = anchors(package);
        let mut slot_of_corner: HashMap<(u32, u16, u16), usize> = HashMap::new();
        let mut slot_of_frame = vec![None; package.frames.len()];
        let mut slots: Vec<Slot> = Vec::new();
        // In the packer's order, so a placement's first record is the one with the least key.
        let mut by_order: Vec<usize> = (0..package.frames.len()).filter(|&frame| package.frames[frame].placed()).collect();
        by_order.sort_by_key(|&frame| package.frames[frame].order);
        for frame in by_order {
            let info = package.frames[frame];
            let slot = *slot_of_corner.entry((info.page, info.x, info.y)).or_insert_with(|| {
                slots.push(Slot {
                    page: info.page, x: u32::from(info.x), y: u32::from(info.y), width: u32::from(info.width),
                    height: u32::from(info.height), anchor: anchors[frame], order: info.order,
                    class: classes.of(sides.get(&info.owner_body).copied().unwrap_or(0)),
                });
                slots.len() - 1
            });
            slot_of_frame[frame] = Some(slot);
        }
        let mut memo = HashMap::new();
        let mut seen_runs = HashSet::new();
        let mut pairs = Vec::new();
        let mut seen_pairs = HashSet::new();
        let wanted: HashMap<(u32, u16, u8), &String> = clip_names.iter().map(|name| {
            let parts: Vec<&str> = name.split(':').collect();
            eyre::ensure!(parts.len() == 3, "clip {name}: not body:action:direction");
            let body: u32 = parts[0].parse()?;
            let movement = match parts[1] {
                "walk" => Some(MOVE_WALK),
                "run" => Some(MOVE_RUN),
                "stand" => Some(MOVE_STAND),
                _ => None,
            };
            let action = match movement {
                Some(movement) => match &package.anim {
                    Anim::Cc(anim) => u16::try_from(body).ok().and_then(|body| anim.movement_action(body, movement, false)),
                    Anim::Ec(anim) => anim.move_action(body, GROUP_PEACE, movement),
                }.ok_or_else(|| eyre::eyre!("clip {name}: body has no such movement"))?,
                None => parts[1].parse()?,
            };
            Ok(((body, action, parts[2].parse()?), name))
        }).collect::<eyre::Result<_>>()?;
        let mut clips = Vec::new();
        for (body, action, direction, start, count) in records(package) {
            let run: Vec<usize> = (start..start + count).filter_map(|frame| slot_of_frame.get(frame as usize).copied().flatten())
                .collect();
            // The first record of a clip (a body's later records of the same clip repeat its frames).
            if let Some(name) = wanted.get(&(body, action, direction))
                && !clips.iter().any(|(kept, _): &(String, Vec<usize>)| kept == *name) {
                clips.push(((*name).clone(), run.clone()));
            }
            if !seen_runs.insert((start, count)) {
                continue;
            }
            let kind = action_kind(package, body, action, &mut memo);
            for pair in run.windows(2) {
                if pair[0] != pair[1] && seen_pairs.insert((pair[0], pair[1])) {
                    pairs.push((pair[0], pair[1], kind));
                }
            }
        }
        let mut adjacent = vec![Vec::new(); slots.len()];
        for &(a, b, kind) in &pairs {
            adjacent[a].push((b, kind));
            adjacent[b].push((a, kind));
        }
        Ok(Self { slots, pairs, adjacent, clips })
    }
}

/// A page of the layout: its used extent, its placements with their content corners, and the package page it is
/// when the layout is the stored one.
struct LaidPage {
    width: u32,
    height: u32,
    placed: Vec<(usize, u32, u32)>,
    stored: Option<usize>,
}

fn align_block(extent: u32) -> u32 {
    extent.next_multiple_of(BLOCK_SIDE)
}

fn lay_out(package: &Package, frames: &Frames, layout: SlotLayout) -> eyre::Result<Vec<LaidPage>> {
    match layout {
        SlotLayout::Stored => {
            let mut pages: Vec<LaidPage> = package.pages.iter().enumerate().map(|(at, page)| LaidPage {
                width: page.used_width, height: page.used_height, placed: Vec::new(), stored: Some(at),
            }).collect();
            let position: HashMap<u32, usize> = package.pages.iter().enumerate().map(|(at, page)| (page.index, at)).collect();
            for (slot, info) in frames.slots.iter().enumerate() {
                pages[position[&info.page]].placed.push((slot, info.x, info.y));
            }
            for page in &pages {
                eyre::ensure!(page.width % BLOCK_SIDE == 0 && page.height % BLOCK_SIDE == 0, "a page's extent is not in blocks");
            }
            Ok(pages)
        }
        SlotLayout::Aligned => {
            let slots: Vec<SlotFrame> = frames.slots.iter().enumerate().map(|(slot, info)| {
                let (pad_x, pad_y) = info.phase();
                SlotFrame { frame: slot, key: info.order, width: (info.width + pad_x) as u16,
                    height: (info.height + pad_y) as u16 }
            }).collect();
            let planned = repack::plan(&slots, package.gutter).map_err(|error| eyre::eyre!(error))?;
            Ok(planned.into_iter().map(|page| LaidPage {
                width: page.used_width, height: page.used_height,
                placed: page.frames.iter().map(|placed| {
                    let (pad_x, pad_y) = frames.slots[placed.frame].phase();
                    (placed.frame, placed.x + pad_x, placed.y + pad_y)
                }).collect(),
                stored: None,
            }).collect())
        }
    }
}

/// A placement's allocation on its laid page, in pixels: the content's phase padding, the content padded to blocks
/// and the gutter on each side.
fn allocation(slot: &Slot, x: u32, y: u32, gutter: u16, aligned: bool) -> (u32, u32, u32, u32) {
    let (pad_x, pad_y) = if aligned { slot.phase() } else { (0, 0) };
    let lead = align_block(u32::from(gutter));
    (x - pad_x - lead, y - pad_y - lead, align_block(slot.width + pad_x) + 2 * lead,
        align_block(slot.height + pad_y) + 2 * lead)
}

/// `udd_conv`'s `extrude_rgba_rect_edges`: every pixel of the rect outside the content takes the nearest content pixel.
#[allow(clippy::too_many_arguments)]
fn extrude(pixels: &mut [u8], width: u32, height: u32, rect: (u32, u32, u32, u32), content: (u32, u32, u32, u32)) {
    let (rect_x, rect_y, rect_width, rect_height) = rect;
    let (content_x, content_y, content_width, content_height) = content;
    if content_width == 0 || content_height == 0 {
        return;
    }
    let rect_right = (rect_x + rect_width).min(width);
    let rect_bottom = (rect_y + rect_height).min(height);
    let (content_right, content_bottom) = (content_x + content_width, content_y + content_height);
    let left_end = content_x.clamp(rect_x, rect_right);
    let right_start = content_right.clamp(rect_x, rect_right);
    let offset = |x: u32, y: u32| (y as usize * width as usize + x as usize) * RGBA_BYTES;
    for y in rect_y..rect_bottom {
        let sample_y = y.clamp(content_y, content_bottom - 1);
        if !(y >= content_y && y < content_bottom) && left_end < right_start {
            let source = offset(left_end, sample_y);
            let len = (right_start - left_end) as usize * RGBA_BYTES;
            pixels.copy_within(source..source + len, offset(left_end, y));
        }
        for (start, count, from) in [(rect_x, left_end - rect_x, content_x), (right_start, rect_right - right_start,
            content_right - 1)] {
            let from = offset(from, sample_y);
            let pixel: [u8; RGBA_BYTES] = pixels[from..from + RGBA_BYTES].try_into().expect("a pixel");
            for x in start..start + count {
                let at = offset(x, y);
                pixels[at..at + RGBA_BYTES].copy_from_slice(&pixel);
            }
        }
    }
}

/// A package page's RGBA8888 pixels and their row length.
struct SourcePixels {
    stride: u32,
    rgba: Vec<u8>,
}

enum PixelPackage {
    Cc(mobile_anim_cc::MobileAnimCcPackage),
    Ec(mobile_anim_ec::MobileAnimEcPackage),
}

impl PixelPackage {
    fn open(path: &PathBuf, kind: Kind) -> eyre::Result<Self> {
        Ok(match kind {
            Kind::Cc => Self::Cc(mobile_anim_cc::MobileAnimCcPackage::load(path)?),
            Kind::Ec => Self::Ec(mobile_anim_ec::MobileAnimEcPackage::load(path)?),
        })
    }

    fn page(&self, package: &Package, position: usize) -> eyre::Result<SourcePixels> {
        let page = &package.pages[position];
        let rgba = match self {
            Self::Cc(anim) => anim.read_page_rgba(page.index)?,
            Self::Ec(anim) => anim.read_page_rgba(page.index)?,
        };
        let stride = if rgba.len() == (page.used_width * page.used_height) as usize * RGBA_BYTES {
            page.used_width
        } else {
            page.atlas_width
        };
        Ok(SourcePixels { stride, rgba })
    }
}

/// The source pages the pages being built need, each decoded once and dropped when its last page is done.
struct SourceCache<'a> {
    pixels: &'a PixelPackage,
    package: &'a Package,
    held: Mutex<HashMap<usize, Arc<SourcePixels>>>,
    users: Mutex<HashMap<usize, usize>>,
}

impl SourceCache<'_> {
    fn get(&self, source: usize) -> eyre::Result<Arc<SourcePixels>> {
        if let Some(held) = self.held.lock().expect("lock").get(&source) {
            return Ok(held.clone());
        }
        let decoded = Arc::new(self.pixels.page(self.package, source)?);
        Ok(self.held.lock().expect("lock").entry(source).or_insert(decoded).clone())
    }

    fn release(&self, source: usize) {
        let mut users = self.users.lock().expect("lock");
        let left = users.get_mut(&source).expect("a counted source");
        *left -= 1;
        if *left == 0 {
            self.held.lock().expect("lock").remove(&source);
        }
    }
}

/// A laid page's pixels, built from the source pages as the packer builds a page.
fn build(page: &LaidPage, frames: &Frames, gutter: u16, aligned: bool, position: &HashMap<u32, usize>,
    sources: &HashMap<usize, Arc<SourcePixels>>) -> Vec<u8> {
    let mut pixels = vec![0u8; (page.width * page.height) as usize * RGBA_BYTES];
    for &(slot, x, y) in &page.placed {
        let info = &frames.slots[slot];
        let source = &sources[&position[&info.page]];
        for row in 0..info.height {
            let from = (((info.y + row) * source.stride + info.x) as usize) * RGBA_BYTES;
            let to = (((y + row) * page.width + x) as usize) * RGBA_BYTES;
            let len = info.width as usize * RGBA_BYTES;
            pixels[to..to + len].copy_from_slice(&source.rgba[from..from + len]);
        }
    }
    for &(slot, x, y) in &page.placed {
        let info = &frames.slots[slot];
        extrude(&mut pixels, page.width, page.height, allocation(info, x, y, gutter, aligned),
            (x, y, info.width, info.height));
    }
    pixels
}

fn parse_variant(text: &str) -> eyre::Result<Vec<Step>> {
    text.split('+').map(|step| {
        let parts: Vec<&str> = step.split(':').collect();
        let number = |at: usize| -> eyre::Result<f64> {
            parts.get(at).ok_or_else(|| eyre::eyre!("{step}: missing value {at}"))?.parse::<f64>()
                .wrap_err_with(|| format!("{step}: value {at}"))
        };
        Ok(match parts[0] {
            "rdo" => Step::Rdo {
                lambda: number(1)? as f32,
                lookback: match parts.get(2).copied() {
                    Some("page") => None,
                    _ => Some(number(2)? as usize),
                },
            },
            "reuse" => Step::Reuse { rms: number(1)?, radius: if parts.len() > 2 { number(2)? as i32 } else {
                DEFAULT_REUSE_RADIUS } },
            "zrdo" => Step::Zrdo { lambda: number(1)?, rms: number(2)?, coder: Coder::Zstd },
            "lrdo" => Step::Zrdo { lambda: number(1)?, rms: number(2)?, coder: Coder::Lz4 },
            "fields" => Step::Fields { lambda: number(1)?, rms: number(2)?, coder: Coder::Zstd },
            "lfields" => Step::Fields { lambda: number(1)?, rms: number(2)?, coder: Coder::Lz4 },
            "coarse" => Step::Coarse { bits: number(1)? as u32, rms: number(2)? },
            "flat" => Step::Flat { rms: number(1)? },
            other => eyre::bail!("unknown step {other}"),
        })
    }).collect()
}

#[derive(Debug, Default, Clone)]
struct StepStats {
    taken: BTreeMap<Action, u64>,
    visible: u64,
    /// Visible blocks by the action of the clip that owns them.
    visible_by: BTreeMap<Action, u64>,
    seconds: f64,
    rdo_modified: u64,
}

/// Quality over one class of bodies (or all).
#[derive(Debug, Default, Clone)]
struct Quality {
    colour_error: f64,
    visible_texels: u64,
    alpha_changed: u64,
    coverage_flips: u64,
    ssim_sum: f64,
    ssim_frames: u64,
    flicker_error: f64,
    flicker_texels: u64,
}

impl Quality {
    fn add(&mut self, other: &Self) {
        self.colour_error += other.colour_error;
        self.visible_texels += other.visible_texels;
        self.alpha_changed += other.alpha_changed;
        self.coverage_flips += other.coverage_flips;
        self.ssim_sum += other.ssim_sum;
        self.ssim_frames += other.ssim_frames;
        self.flicker_error += other.flicker_error;
        self.flicker_texels += other.flicker_texels;
    }

    fn psnr(&self) -> f64 {
        let mse = self.colour_error / (self.visible_texels as f64 * COLOUR_CHANNELS);
        10.0 * (PEAK * PEAK / mse).log10()
    }

    fn ssim(&self) -> f64 {
        self.ssim_sum / self.ssim_frames.max(1) as f64
    }

    fn flicker(&self) -> f64 {
        (self.flicker_error / (self.flicker_texels.max(1) as f64 * COLOUR_CHANNELS)).sqrt()
    }
}

/// One variant's outcome on the measured pages.
#[derive(Debug, Default, Clone)]
struct Outcome {
    bytes: Vec<u64>,
    quality: BTreeMap<usize, Quality>,
    flicker_by_action: BTreeMap<Action, Quality>,
    steps: Vec<StepStats>,
}

fn crop(rgba: &[u8], width: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h) as usize * RGBA_BYTES);
    for row in y..y + h {
        let at = ((row * width + x) as usize) * RGBA_BYTES;
        out.extend_from_slice(&rgba[at..at + w as usize * RGBA_BYTES]);
    }
    out
}

fn ssim_canvas(width: usize, height: usize, rgba: &[u8]) -> LinearRgb {
    let (canvas_width, canvas_height) = (width.max(SSIM_MIN_SIDE), height.max(SSIM_MIN_SIDE));
    let mut data = vec![[SSIM_BACKGROUND; 3]; canvas_width * canvas_height];
    for y in 0..height {
        for x in 0..width {
            let pixel = &rgba[(y * width + x) * RGBA_BYTES..][..RGBA_BYTES];
            let alpha = f32::from(pixel[ALPHA]) / PEAK as f32;
            let target = &mut data[y * canvas_width + x];
            for channel in 0..3 {
                target[channel] = SSIM_BACKGROUND * (1.0 - alpha) + f32::from(pixel[channel]) / PEAK as f32 * alpha;
            }
        }
    }
    let rgb = Rgb::new(data, canvas_width, canvas_height, TransferCharacteristic::SRGB, ColorPrimaries::BT709)
        .expect("canvas");
    LinearRgb::try_from(rgb).expect("linear canvas")
}

/// The quality of a decoded page against its source, per placement's class, and the flicker of each pair of
/// consecutive frames on it: the change between their decodes at texels whose source does not change, relative to
/// the anchor.
#[allow(clippy::too_many_arguments)]
fn measure(page: &LaidPage, frames: &Frames, source: &[u8], decoded: &[u8], ssim_every: usize,
    on_page: &HashMap<usize, (u32, u32)>, pairs: &[(usize, usize, Action)], outcome: &mut Outcome) -> eyre::Result<()> {
    for &(slot, x, y) in &page.placed {
        let info = &frames.slots[slot];
        let quality = outcome.quality.entry(info.class).or_default();
        for row in y..y + info.height {
            for column in x..x + info.width {
                let at = ((row * page.width + column) as usize) * RGBA_BYTES;
                let (a, b) = (&source[at..at + RGBA_BYTES], &decoded[at..at + RGBA_BYTES]);
                quality.alpha_changed += u64::from(a[ALPHA] != b[ALPHA]);
                quality.coverage_flips += u64::from((a[ALPHA] == 0) != (b[ALPHA] == 0));
                if a[ALPHA] > 0 {
                    quality.visible_texels += 1;
                    quality.colour_error += (0..3).map(|channel| (f64::from(a[channel]) - f64::from(b[channel])).powi(2))
                        .sum::<f64>();
                }
            }
        }
        if ssim_every > 0 && slot % ssim_every == 0 {
            let reference = crop(source, page.width, x, y, info.width, info.height);
            let candidate = crop(decoded, page.width, x, y, info.width, info.height);
            let (w, h) = (info.width as usize, info.height as usize);
            let value = compute_frame_ssimulacra2(ssim_canvas(w, h, &reference), ssim_canvas(w, h, &candidate))?;
            quality.ssim_sum += value;
            quality.ssim_frames += 1;
        }
    }
    for &(a, b, action) in pairs {
        let (Some(&(ax, ay)), Some(&(bx, by))) = (on_page.get(&a), on_page.get(&b)) else { continue };
        let (slot_a, slot_b) = (&frames.slots[a], &frames.slots[b]);
        let flicker = outcome.flicker_by_action.entry(action).or_default();
        let class_flicker = &mut outcome.quality.entry(slot_a.class).or_default();
        let (mut error, mut texels) = (0f64, 0u64);
        for v in 0..slot_a.height as i32 {
            for u in 0..slot_a.width as i32 {
                // The same place relative to the anchor in frame b.
                let (ub, vb) = (u - slot_a.anchor.0 + slot_b.anchor.0, v - slot_a.anchor.1 + slot_b.anchor.1);
                if ub < 0 || vb < 0 || ub >= slot_b.width as i32 || vb >= slot_b.height as i32 {
                    continue;
                }
                let at_a = (((ay as i32 + v) as u32 * page.width + (ax as i32 + u) as u32) as usize) * RGBA_BYTES;
                let at_b = (((by as i32 + vb) as u32 * page.width + (bx as i32 + ub) as u32) as usize) * RGBA_BYTES;
                let (source_a, source_b) = (&source[at_a..at_a + RGBA_BYTES], &source[at_b..at_b + RGBA_BYTES]);
                if source_a != source_b || source_a[ALPHA] == 0 {
                    continue;
                }
                texels += 1;
                error += (0..3).map(|channel| (f64::from(decoded[at_a + channel]) - f64::from(decoded[at_b + channel])).powi(2))
                    .sum::<f64>();
            }
        }
        flicker.flicker_error += error;
        flicker.flicker_texels += texels;
        class_flicker.flicker_error += error;
        class_flicker.flicker_texels += texels;
    }
    Ok(())
}

/// A clip frame kept for the videos: the anchor and each version's pixels (source, plain, then each variant).
struct KeptFrame {
    width: u32,
    height: u32,
    anchor: (i32, i32),
    versions: Vec<Vec<u8>>,
}

pub fn lossy(args: &LossyArgs) -> eyre::Result<()> {
    let package = Package::open(&args.bc7, args.kind)?;
    let pixels = PixelPackage::open(&args.rgba, args.kind)?;
    let classes = Classes::new(args.bounds.clone());
    let frames = Frames::new(&package, &classes, &args.clips)?;
    let aligned = args.layout == SlotLayout::Aligned;
    let start = Instant::now();
    let laid = lay_out(&package, &frames, args.layout)?;
    let laid_blocks: u64 = laid.iter().map(|page| u64::from(page.width / BLOCK_SIDE) * u64::from(page.height / BLOCK_SIDE))
        .sum();
    let stored_blocks: u64 = package.pages.iter()
        .map(|page| u64::from(page.used_width / BLOCK_SIDE) * u64::from(page.used_height / BLOCK_SIDE)).sum();
    println!("{:?} layout: {} placements, {} pairs, {} pages and {} blocks (the package: {} pages, {} blocks, {:+.2} %) \
        in {:.1} s", args.layout, frames.slots.len(), frames.pairs.len(), laid.len(), laid_blocks, package.pages.len(),
        stored_blocks, (laid_blocks as f64 / stored_blocks as f64 - 1.0) * PERCENT, start.elapsed().as_secs_f64());

    let variants: Vec<(String, Vec<Step>)> = args.variants.iter()
        .map(|text| Ok((text.clone(), parse_variant(text)?))).collect::<eyre::Result<_>>()?;
    let methods: Vec<Method> = args.methods.iter().map(|text| Method::parse(text)).collect::<eyre::Result<_>>()?;
    let coded: Vec<_> = methods.iter().map(|method| match method {
        Method::Coded { transform, backend } => Ok((*transform, *backend)),
        Method::Stored => eyre::bail!("stored is no method here"),
    }).collect::<eyre::Result<_>>()?;

    let clip_slots: HashSet<usize> = frames.clips.iter().flat_map(|(_, slots)| slots.iter().copied()).collect();
    let mut chosen: Vec<(usize, bool)> = (args.offset..laid.len()).step_by(args.every.max(1)).map(|at| (at, true)).collect();
    let measured: HashSet<usize> = chosen.iter().map(|&(at, _)| at).collect();
    for (at, page) in laid.iter().enumerate() {
        if !measured.contains(&at) && page.placed.iter().any(|(slot, _, _)| clip_slots.contains(slot)) {
            chosen.push((at, false));
        }
    }
    let position: HashMap<u32, usize> = package.pages.iter().enumerate().map(|(at, page)| (page.index, at)).collect();
    let pair_lists: Vec<Vec<(usize, usize, Action)>> = {
        let mut page_of = vec![usize::MAX; frames.slots.len()];
        for (at, page) in laid.iter().enumerate() {
            for &(slot, _, _) in &page.placed {
                page_of[slot] = at;
            }
        }
        let mut lists = vec![Vec::new(); laid.len()];
        for &(a, b, action) in &frames.pairs {
            if page_of[a] == page_of[b] {
                lists[page_of[a]].push((a, b, action));
            }
        }
        lists
    };
    let paired_on_page: usize = pair_lists.iter().map(Vec::len).sum();
    println!("pairs on one page: {paired_on_page} of {} ({:.1} %)", frames.pairs.len(),
        paired_on_page as f64 * PERCENT / frames.pairs.len().max(1) as f64);

    let versions = variants.len() + 1;
    let totals = Mutex::new(vec![Outcome { bytes: vec![0; methods.len()], ..Default::default() }; versions]);
    let encode_seconds = Mutex::new(0f64);
    let kept: Mutex<HashMap<usize, KeptFrame>> = Mutex::new(HashMap::new());
    let mismatches = Mutex::new((0u64, 0u64, 0u64));
    if let Some(dir) = &args.store {
        for (name, _) in &variants {
            std::fs::create_dir_all(dir.join(store_name(name)))?;
        }
        std::fs::create_dir_all(dir.join("plain"))?;
    }
    let wall = Instant::now();
    let needed = |at: usize| -> BTreeSet<usize> {
        laid[at].placed.iter().map(|(slot, _, _)| position[&frames.slots[*slot].page]).collect()
    };
    let mut users: HashMap<usize, usize> = HashMap::new();
    for &(at, _) in &chosen {
        for source in needed(at) {
            *users.entry(source).or_default() += 1;
        }
    }
    let cache = SourceCache { pixels: &pixels, package: &package, held: Mutex::new(HashMap::new()),
        users: Mutex::new(users) };
    let done = AtomicUsize::new(0);
    {
        let process = |at: usize, counted: bool| -> eyre::Result<()> {
            let page = &laid[at];
            let page_sources = needed(at);
            let sources: HashMap<usize, Arc<SourcePixels>> = page_sources.iter()
                .map(|&source| Ok((source, cache.get(source)?))).collect::<eyre::Result<_>>()?;
            let rgba = build(page, &frames, package.gutter, aligned, &position, &sources);
            if args.check && let Some(stored) = page.stored {
                let source = &sources[&stored];
                let same = source.stride == page.width && source.rgba == rgba;
                mismatches.lock().expect("lock").2 += u64::from(!same);
            }
            let extent = ImageExtent::new(page.width, page.height).map_err(|error| eyre::eyre!("{error}"))?;
            let encode = Instant::now();
            let plain = encode_to_bc7(&rgba, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide)
                .map_err(|error| eyre::eyre!("{error}"))?.into_blocks();
            let encoded_in = encode.elapsed().as_secs_f64();
            if args.check && let Some(stored) = page.stored {
                let reference = package.raw(&package.pages[stored])?;
                let differing = plain.chunks_exact(BLOCK_BYTES).zip(reference.chunks_exact(BLOCK_BYTES))
                    .filter(|(a, b)| a != b).count() as u64;
                let mut counts = mismatches.lock().expect("lock");
                counts.0 += differing;
                counts.1 += u64::from(reference.len() != plain.len());
            }
            let wide = (page.width / BLOCK_SIDE) as usize;
            // The page's placements in its order, their neighbours on the page by local index.
            let mut on_page = HashMap::new();
            let mut local = HashMap::new();
            let mut placements = Vec::with_capacity(page.placed.len());
            for (at, &(slot, x, y)) in page.placed.iter().enumerate() {
                let info = &frames.slots[slot];
                on_page.insert(slot, (x, y));
                local.insert(slot, at);
                placements.push(Placement { allocation: allocation(info, x, y, package.gutter, aligned),
                    anchor: (x as i32 + info.anchor.0, y as i32 + info.anchor.1) });
            }
            let adjacent: Vec<Vec<(usize, Action)>> = page.placed.iter().map(|&(slot, _, _)| frames.adjacent[slot].iter()
                .filter_map(|&(other, action)| local.get(&other).map(|&other| (other, action))).collect()).collect();
            let context = PageConditioner::new(&rgba, page.width, page.height, &plain, &placements, &adjacent)?;
            // A block's clip kind: the first clip of its placement's, over the whole package.
            let action_of = |block: usize| context.owner(block)
                .and_then(|at| frames.adjacent[page.placed[at].0].first().map(|&(_, action)| action)).unwrap_or(Action::Other);
            let mut results: Vec<(Vec<u8>, Vec<StepStats>)> = vec![(plain.clone(), Vec::new())];
            for (_, steps) in &variants {
                let mut blocks = context.base_blocks();
                let mut stats = Vec::new();
                for &step in steps {
                    let start = Instant::now();
                    let mut step_stats = StepStats::default();
                    for block in (0..blocks.len()).filter(|&block| context.visible(block)) {
                        step_stats.visible += 1;
                        *step_stats.visible_by.entry(action_of(block)).or_default() += 1;
                    }
                    // Reuse counts a block under the clip it was taken through; Zrdo under that clip's unless it is
                    // Other, then (as every other step) under its own placement's.
                    let reuse = matches!(step, Step::Reuse { .. });
                    let mut taken = |block: usize, action: Option<Action>| {
                        let kind = action.filter(|&action| reuse || action != Action::Other)
                            .unwrap_or_else(|| action_of(block));
                        *step_stats.taken.entry(kind).or_default() += 1;
                    };
                    let modified = context.apply(step, &mut blocks, &mut taken);
                    step_stats.rdo_modified += u64::from(modified);
                    step_stats.seconds += start.elapsed().as_secs_f64();
                    stats.push(step_stats);
                }
                results.push((blocks.iter().flatten().copied().collect(), stats));
            }
            let keep: Vec<usize> = page.placed.iter().map(|&(slot, _, _)| slot).filter(|slot| clip_slots.contains(slot)).collect();
            let mut outcomes = Vec::with_capacity(versions);
            let mut scratch = Vec::new();
            for (version, (blocks, stats)) in results.iter().enumerate() {
                let decoded = decode_bc7_to_rgba8888(blocks, extent).map_err(|error| eyre::eyre!("{error}"))?;
                for &slot in &keep {
                    let info = &frames.slots[slot];
                    let (x, y) = on_page[&slot];
                    let mut kept = kept.lock().expect("lock");
                    let frame = kept.entry(slot).or_insert_with(|| KeptFrame { width: info.width, height: info.height,
                        anchor: info.anchor, versions: vec![crop(&rgba, page.width, x, y, info.width, info.height)] });
                    frame.versions.push(crop(&decoded, page.width, x, y, info.width, info.height));
                }
                if !counted {
                    continue;
                }
                let mut outcome = Outcome { bytes: Vec::with_capacity(coded.len()), steps: stats.clone(), ..Default::default() };
                for &(transform, backend) in &coded {
                    outcome.bytes.push(store(transform, backend, blocks, wide, None, &mut scratch)?.bytes as u64);
                }
                measure(page, &frames, &rgba, &decoded, args.ssim_every, &on_page, &pair_lists[at], &mut outcome)?;
                if let (Some(dir), Some(stored)) = (&args.store, page.stored) {
                    let name = if version == 0 { "plain".to_string() } else { store_name(&variants[version - 1].0) };
                    std::fs::write(dir.join(name).join(format!("{stored}.zst")),
                        zstd::bulk::compress(blocks, STORE_ZSTD_LEVEL)?)?;
                }
                outcomes.push(outcome);
            }
            if counted {
                *encode_seconds.lock().expect("lock") += encoded_in;
                let mut totals = totals.lock().expect("lock");
                for (total, outcome) in totals.iter_mut().zip(outcomes) {
                    total.bytes.iter_mut().zip(&outcome.bytes).for_each(|(sum, bytes)| *sum += bytes);
                    for (class, quality) in outcome.quality {
                        total.quality.entry(class).or_default().add(&quality);
                    }
                    for (action, quality) in outcome.flicker_by_action {
                        total.flicker_by_action.entry(action).or_default().add(&quality);
                    }
                    if total.steps.is_empty() {
                        total.steps = outcome.steps;
                    } else {
                        for (sum, step) in total.steps.iter_mut().zip(outcome.steps) {
                            sum.visible += step.visible;
                            sum.seconds += step.seconds;
                            sum.rdo_modified += step.rdo_modified;
                            for (action, count) in step.taken {
                                *sum.taken.entry(action).or_default() += count;
                            }
                            for (action, count) in step.visible_by {
                                *sum.visible_by.entry(action).or_default() += count;
                            }
                        }
                    }
                }
            }
            drop(sources);
            page_sources.into_iter().for_each(|source| cache.release(source));
            let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
            if finished % PROGRESS_PAGES == 0 {
                eprintln!("{finished} of {} pages in {:.0} s", chosen.len(), wall.elapsed().as_secs_f64());
            }
            Ok(())
        };
        // Plain threads, not a rayon pool: the encoder and the readers use rayon, and a pool thread waiting on them
        // would steal another page, whose reads can then wait on a lock the first page holds.
        let next = AtomicUsize::new(0);
        let failure: Mutex<Option<eyre::Report>> = Mutex::new(None);
        std::thread::scope(|scope| {
            for _ in 0..PAGE_THREADS {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= chosen.len() || failure.lock().expect("lock").is_some() {
                        break;
                    }
                    let (at, counted) = chosen[index];
                    if let Err(error) = process(at, counted) {
                        *failure.lock().expect("lock") = Some(error);
                        break;
                    }
                });
            }
        });
        if let Some(error) = failure.into_inner().expect("lock") {
            return Err(error);
        }
    }
    let measured_pages = chosen.iter().filter(|&&(_, counted)| counted).count();
    let encode_seconds = encode_seconds.into_inner().expect("lock");
    println!("{measured_pages} pages measured in {:.1} s; plain encode {encode_seconds:.1} s summed over the pages \
        ({:.2} s a page)", wall.elapsed().as_secs_f64(), encode_seconds / measured_pages.max(1) as f64);
    if args.check {
        let (blocks, lengths, pixels) = *mismatches.lock().expect("lock");
        println!("check: {blocks} blocks differ from the BC7 package, {lengths} pages of another length, {pixels} built \
            pages differ from the RGBA package");
    }

    let totals = totals.into_inner().expect("lock");
    let names: Vec<String> = std::iter::once("plain".to_string()).chain(variants.iter().map(|(name, _)| name.clone())).collect();
    print_report(&names, &methods, &totals, &classes, measured_pages);
    let kept = kept.into_inner().expect("lock");
    if let Some(dir) = &args.clip_dir {
        render_clips(&frames, &kept, &names, dir, args.clip_scale)?;
    }
    if let Some(dir) = &args.ladder_dir {
        let pairs = |list: &[String]| -> eyre::Result<HashMap<String, String>> {
            list.iter().map(|pair| pair.split_once('=').map(|(key, text)| (key.to_string(), text.to_string()))
                .ok_or_else(|| eyre::eyre!("{pair}: not key=text"))).collect()
        };
        let clips: Vec<(String, Vec<ladder::Frame>)> = frames.clips.iter().map(|(name, slots)| {
            (name.clone(), slots.iter().filter_map(|slot| kept.get(slot)).map(|frame| ladder::Frame {
                width: frame.width, height: frame.height, anchor: frame.anchor, versions: &frame.versions }).collect())
        }).collect();
        ladder::render(&clips, &ladder::Options { dir, prefix: &args.ladder_prefix, names: &names,
            notes: &pairs(&args.notes)?, body_names: &pairs(&args.body_names)?, font: &args.font })?;
    }
    Ok(())
}

fn store_name(variant: &str) -> String {
    variant.replace([':', '+'], "_")
}

fn print_report(names: &[String], methods: &[Method], totals: &[Outcome], classes: &Classes, pages: usize) {
    let base = &totals[0];
    print!("\n| Variant |");
    for method in methods {
        print!(" {} MiB | Δ |", method.name());
    }
    println!(" PSNR dB | Δ dB | alpha changed | coverage flips | SSIMULACRA2 | flicker RMS |");
    print!("|---|");
    for _ in methods {
        print!("---:|---:|");
    }
    println!("---:|---:|---:|---:|---:|---:|");
    let all = |outcome: &Outcome| {
        let mut sum = Quality::default();
        outcome.quality.values().for_each(|quality| sum.add(quality));
        sum
    };
    let base_all = all(base);
    for (name, outcome) in names.iter().zip(totals) {
        print!("| {name} |");
        for (&bytes, &base_bytes) in outcome.bytes.iter().zip(&base.bytes) {
            print!(" {:.2} | {:+.2} % |", mib(bytes), (bytes as f64 / base_bytes as f64 - 1.0) * PERCENT);
        }
        let quality = all(outcome);
        println!(" {:.2} | {:+.2} | {} | {} | {:.2} | {:.3} |", quality.psnr(), quality.psnr() - base_all.psnr(),
            quality.alpha_changed, quality.coverage_flips, quality.ssim(), quality.flicker());
    }
    println!("\n(Each Δ against the plain encode under the same method, on {pages} pages.)");
    println!("\n| Variant | Class | PSNR dB | Δ dB | SSIMULACRA2 | Δ | flicker RMS | Δ |\n|---|---|---:|---:|---:|---:|---:|---:|");
    for (name, outcome) in names.iter().zip(totals) {
        for (class, quality) in &outcome.quality {
            let reference = &base.quality[class];
            println!("| {name} | {} | {:.2} | {:+.2} | {:.2} | {:+.2} | {:.3} | {:+.3} |", classes.name(*class),
                quality.psnr(), quality.psnr() - reference.psnr(), quality.ssim(), quality.ssim() - reference.ssim(),
                quality.flicker(), quality.flicker() - reference.flicker());
        }
    }
    println!("\n| Variant | Step | Blocks changed, share of the action's visible (move / stand / other) | Share of visible | Step s a page | Flicker \
        RMS move / stand / other |\n|---|---|---:|---:|---:|---:|");
    for (name, outcome) in names.iter().zip(totals).skip(1) {
        let flicker: Vec<String> = ACTIONS.iter().map(|action| outcome.flicker_by_action.get(action)
            .map_or("-".into(), |quality| format!("{:.3}", quality.flicker()))).collect();
        for (at, step) in outcome.steps.iter().enumerate() {
            let taken: Vec<String> = ACTIONS.iter().map(|action| {
                let count = step.taken.get(action).copied().unwrap_or(0);
                let visible = step.visible_by.get(action).copied().unwrap_or(0);
                format!("{count} ({:.1} %)", count as f64 * PERCENT / visible.max(1) as f64)
            }).collect();
            let changed: u64 = step.taken.values().sum::<u64>() + step.rdo_modified;
            println!("| {name} | {at} | {} (+{} RDO) | {:.2} % | {:.3} | {} |", taken.join(" / "), step.rdo_modified,
                changed as f64 * PERCENT / step.visible.max(1) as f64, step.seconds / pages.max(1) as f64,
                flicker.join(" / "));
        }
    }
    let flicker: Vec<String> = ACTIONS.iter().map(|action| base.flicker_by_action.get(action)
        .map_or("-".into(), |quality| format!("{:.3}", quality.flicker()))).collect();
    println!("\nplain flicker RMS move / stand / other: {}", flicker.join(" / "));
}

/// Each `--clip` as a video: source, plain encode and each variant side by side, anchored, nearest-upscaled.
fn render_clips(frames: &Frames, kept: &HashMap<usize, KeptFrame>, names: &[String], dir: &PathBuf, scale: usize)
    -> eyre::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, slots) in &frames.clips {
        let shown: Vec<&KeptFrame> = slots.iter().filter_map(|slot| kept.get(slot)).collect();
        if shown.is_empty() {
            eprintln!("clip {name}: no frames");
            continue;
        }
        // The union of the frames' boxes relative to the anchor.
        let (mut left, mut top, mut right, mut bottom) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for frame in &shown {
            left = left.min(-frame.anchor.0);
            top = top.min(-frame.anchor.1);
            right = right.max(frame.width as i32 - frame.anchor.0);
            bottom = bottom.max(frame.height as i32 - frame.anchor.1);
        }
        let (panel_width, panel_height) = ((right - left) as usize + VIDEO_GAP, (bottom - top) as usize + VIDEO_GAP);
        let panels = shown[0].versions.len();
        let (width, height) = ((panel_width * panels * scale).next_multiple_of(2),
            (panel_height * scale).next_multiple_of(2));
        let file = dir.join(format!("{}_{}.mp4", name.replace(':', "-"), names.len()));
        let mut child = Command::new("ffmpeg").args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24",
            "-s", &format!("{width}x{height}"), "-r", &VIDEO_FPS.to_string(), "-i", "-", "-c:v", "libx264", "-crf",
            VIDEO_CRF, "-pix_fmt", "yuv420p"]).arg(&file).stdin(Stdio::piped()).spawn().wrap_err("start ffmpeg")?;
        let mut stdin = child.stdin.take().expect("piped");
        for _ in 0..VIDEO_LOOPS {
            for frame in &shown {
                let mut canvas = VIDEO_BACKGROUND.repeat(width * height);
                for (panel, pixels) in frame.versions.iter().enumerate() {
                    let origin_x = panel * panel_width + VIDEO_GAP / 2 + (-frame.anchor.0 - left) as usize;
                    let origin_y = VIDEO_GAP / 2 + (-frame.anchor.1 - top) as usize;
                    for y in 0..frame.height as usize {
                        for x in 0..frame.width as usize {
                            let pixel = &pixels[(y * frame.width as usize + x) * RGBA_BYTES..][..RGBA_BYTES];
                            let alpha = u32::from(pixel[ALPHA]);
                            for sy in 0..scale {
                                for sx in 0..scale {
                                    let at = (((origin_y + y) * scale + sy) * width
                                        + (origin_x + x) * scale + sx) * 3;
                                    for channel in 0..3 {
                                        let under = u32::from(canvas[at + channel]);
                                        canvas[at + channel] = ((u32::from(pixel[channel]) * alpha + under * (255 - alpha))
                                            / 255) as u8;
                                    }
                                }
                            }
                        }
                    }
                }
                stdin.write_all(&canvas)?;
            }
        }
        drop(stdin);
        eyre::ensure!(child.wait()?.success(), "ffmpeg failed for {name}");
        println!("clip {name}: {} frames, panels source | {} -> {}", shown.len(), names.join(" | "), file.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extruding_copies_the_nearest_content_pixel() {
        const SIDE: u32 = 8;
        let (rect, content) = ((1, 1, 6, 6), (3, 2, 2, 3));
        let mut pixels = vec![0u8; (SIDE * SIDE) as usize * RGBA_BYTES];
        for y in content.1..content.1 + content.3 {
            for x in content.0..content.0 + content.2 {
                let at = ((y * SIDE + x) as usize) * RGBA_BYTES;
                pixels[at..at + RGBA_BYTES].copy_from_slice(&[x as u8, y as u8, 1, u8::MAX]);
            }
        }
        extrude(&mut pixels, SIDE, SIDE, rect, content);
        for y in 0..SIDE {
            for x in 0..SIDE {
                let at = ((y * SIDE + x) as usize) * RGBA_BYTES;
                let inside = x >= rect.0 && x < rect.0 + rect.2 && y >= rect.1 && y < rect.1 + rect.3;
                let expected = if inside {
                    let nearest_x = x.clamp(content.0, content.0 + content.2 - 1);
                    let nearest_y = y.clamp(content.1, content.1 + content.3 - 1);
                    [nearest_x as u8, nearest_y as u8, 1, u8::MAX]
                } else {
                    [0; RGBA_BYTES]
                };
                assert_eq!(pixels[at..at + RGBA_BYTES], expected, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn variants_parse_into_their_steps() {
        let steps = parse_variant("coarse:2:2+reuse:4+rdo:1:page").expect("a variant");
        assert!(matches!(steps[0], Step::Coarse { bits: 2, .. }));
        assert!(matches!(steps[1], Step::Reuse { radius: DEFAULT_REUSE_RADIUS, .. }));
        assert!(matches!(steps[2], Step::Rdo { lookback: None, .. }));
        assert!(parse_variant("blur:1").is_err());
        assert_eq!(store_name("coarse:2:2+zrdo:30:3"), "coarse_2_2_zrdo_30_3");
    }
}
