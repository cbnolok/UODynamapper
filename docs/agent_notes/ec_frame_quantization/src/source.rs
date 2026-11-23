//! The lossless source: `AnimationFrame*.uop` entries of a client folder (read only), decoded by uocf as the client
//! decodes them, with each pixel's palette index, coverage and hue class.

use std::path::Path;

use uocf::enhanced::animationframe::{AnimationFrame, FrameEntry, HUE_MASK_HUEABLE};
use uocf::uop_container::package::{LoadMode, UopPackage};

pub const RGBA_BYTES: usize = 4;
const PACKAGE_COUNT: usize = 6;
/// Highest action id probed per body (`MAX_ACTION_ID_FALLBACK`).
pub const MAX_ACTION: u16 = uocf::enhanced::animationframe::MAX_ACTION_ID_FALLBACK;
pub const MAX_BODY: u32 = uocf::enhanced::animationframe::MAX_BODY_ID;
/// Directions an entry stores (3..7 of the eight).
pub const STORED_DIRECTIONS: usize = 5;
/// Coverage of a solid (not edge) pixel, in sixteenths.
const SOLID_ALPHA: u8 = u8::MAX;
/// Palette neighbours a colour's spacing averages over.
const SPACING_NEIGHBOURS: usize = 3;

pub struct Sources {
    packages: Vec<UopPackage>,
}

impl Sources {
    pub fn open(dir: &str) -> Self {
        let packages = (1..=PACKAGE_COUNT)
            .map(|n| Path::new(dir).join(format!("AnimationFrame{n}.uop")))
            .filter(|path| path.exists())
            .map(|path| UopPackage::load_with_mode(&path, LoadMode::Lazy).unwrap())
            .collect::<Vec<_>>();
        assert!(!packages.is_empty(), "no AnimationFrame*.uop in {dir}");
        Self { packages }
    }

    pub fn entry(&self, body: u32, action: u16) -> Option<AnimationFrame> {
        let hash = AnimationFrame::animationframe_hash(body, action);
        self.packages.iter().find_map(|package| package.unpack_file_by_hash(hash).unwrap())
            .map(|bytes| AnimationFrame::load(&bytes).unwrap())
    }
}

/// An action's palette as floats, and each colour's spacing: the mean distance to its nearest other colours, the
/// size of the quantization cell around it.
pub struct Palette {
    pub rgb: Vec<[f32; 3]>,
    pub spacing: Vec<f32>,
}

impl Palette {
    pub fn of(animation: &AnimationFrame) -> Self {
        // Stored [b0, b1, b2, flag]; decode_frame writes them as RGBA in the order the client does (b0 = red).
        Self::from_rgb(animation.colours.iter().map(|c| [f32::from(c[0]), f32::from(c[1]), f32::from(c[2])]).collect())
    }

    pub fn from_rgb(rgb: Vec<[f32; 3]>) -> Self {
        let spacing = rgb.iter().map(|a| {
            let mut d: Vec<f32> = rgb.iter().map(|b| distance(a, b)).filter(|&d| d > 0.0).collect();
            d.sort_by(f32::total_cmp);
            let n = d.len().min(SPACING_NEIGHBOURS).max(1);
            d.iter().take(n).sum::<f32>() / n as f32
        }).collect();
        Self { rgb, spacing }
    }
}

pub fn distance(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// A decoded frame: straight-alpha RGBA, and per pixel its palette index, whether it is solid (coverage 16) and
/// whether it takes the hue.
#[derive(Clone)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub rgba: Vec<u8>,
    pub index: Vec<u8>,
    pub solid: Vec<bool>,
    pub hueable: Vec<bool>,
}

impl Frame {
    pub fn decode(animation: &AnimationFrame, entry: &FrameEntry) -> Self {
        let decoded = animation.decode_frame(entry).unwrap();
        let indexed = animation.decode_frame_indexed(entry).unwrap();
        let count = usize::from(decoded.width) * usize::from(decoded.height);
        let solid = (0..count).map(|i| decoded.data[i * RGBA_BYTES + 3] == SOLID_ALPHA).collect();
        let hueable = if decoded.hue_mask.is_empty() { vec![false; count] } else {
            decoded.hue_mask.iter().map(|&m| m == HUE_MASK_HUEABLE).collect()
        };
        Self { width: usize::from(decoded.width), height: usize::from(decoded.height),
            anchor_x: i32::from(decoded.center_x), anchor_y: i32::from(decoded.center_y), rgba: decoded.data,
            index: indexed.indices, solid, hueable }
    }
}

/// The frame entry of `stored_direction` (0..5) and `frame` of an action: numbers run direction-major from 1.
pub fn find_entry(animation: &AnimationFrame, stored_direction: usize, frame: usize) -> Option<FrameEntry> {
    let per_direction = frame_count(animation);
    let number = (stored_direction * per_direction + frame + 1) as u16;
    animation.frames.iter().find(|entry| entry.frame_number == number).copied()
}

pub fn frame_count(animation: &AnimationFrame) -> usize {
    let highest = animation.frames.iter().map(|entry| usize::from(entry.frame_number)).max().unwrap_or(0);
    highest.div_ceil(STORED_DIRECTIONS)
}

/// The non-empty frames of one stored direction, in order.
pub fn direction_frames(animation: &AnimationFrame, stored_direction: usize) -> Vec<Frame> {
    (0..frame_count(animation)).filter_map(|f| find_entry(animation, stored_direction, f))
        .filter(|entry| entry.size().map(|(w, h)| w > 0 && h > 0).unwrap_or(false))
        .map(|entry| Frame::decode(animation, &entry)).collect()
}

/// Every non-empty frame of the entry.
pub fn all_frames(animation: &AnimationFrame) -> Vec<Frame> {
    animation.frames.iter().filter(|entry| entry.size().map(|(w, h)| w > 0 && h > 0).unwrap_or(false))
        .map(|entry| Frame::decode(animation, entry)).collect()
}
