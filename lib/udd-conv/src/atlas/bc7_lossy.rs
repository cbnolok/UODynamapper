//! The lossy conditioning of the BC7 mobile animation pages (`uot_texture_codecs::bc7::condition`): after the BC7
//! encode (and the RDO, when asked for), before the page coding, a chain of steps changes the blocks so the page codes
//! smaller, each bounded against the encode's error on the visible texels, alpha kept exact. The pages decode as any
//! BC7 page; the format and the readers do not change. The levels and their measurements are in
//! `docs/agent_notes/bc7_anim_supercompression.md`, "Lossy conditioning" (sizes under the field split with Zstd 17,
//! whole packages, Classic / Enhanced): `minimal` −9.6 / −6.6 %, `slight` −13.9 / −11.7 %, `moderate`
//! −21.4 / −17.7 %. The user's choice of 2026-11-27 is `minimal` by default, the others and `lossless` as options.
//!
//! The steps take each page alone, so the result depends on the page and not on the thread count; the temporal
//! candidates come from the frames' clips ([`ClipNeighbours`]), as the study's harness reads them from a package.
//! Only BC7 pages are conditioned (an RGBA8888 package ignores the level), and only the 1x pages: the 2x pages of
//! `--prescale` are written as encoded.

use std::collections::{HashMap, HashSet};

use uot_texture_codecs::bc7::condition::{Coder, ConditionError, PageConditioner, Placement, Step};

/// `minimal`: field-split-aware reuse at λ 30 within 3 levels (the study's `fields:30:3`).
pub const MINIMAL_FIELDS_LAMBDA: f64 = 30.0;
pub const MINIMAL_FIELDS_RMS: f64 = 3.0;
/// `slight`: endpoints rounded to multiples of 4 within 2 levels, then Zstd-priced block reuse at λ 30 within 3
/// (`coarse:2:2+zrdo:30:3`).
pub const SLIGHT_COARSE_BITS: u32 = 2;
pub const SLIGHT_COARSE_RMS: f64 = 2.0;
pub const SLIGHT_ZRDO_LAMBDA: f64 = 30.0;
pub const SLIGHT_ZRDO_RMS: f64 = 3.0;
/// `moderate`: endpoints rounded to multiples of 4 within 4 levels, then block reuse at λ 100 within 6
/// (`coarse:2:4+zrdo:100:6`).
pub const MODERATE_COARSE_BITS: u32 = 2;
pub const MODERATE_COARSE_RMS: f64 = 4.0;
pub const MODERATE_ZRDO_LAMBDA: f64 = 100.0;
pub const MODERATE_ZRDO_RMS: f64 = 6.0;

const MINIMAL_STEPS: [Step; 1] =
    [Step::Fields { lambda: MINIMAL_FIELDS_LAMBDA, rms: MINIMAL_FIELDS_RMS, coder: Coder::Zstd }];
const SLIGHT_STEPS: [Step; 2] = [
    Step::Coarse { bits: SLIGHT_COARSE_BITS, rms: SLIGHT_COARSE_RMS },
    Step::Zrdo { lambda: SLIGHT_ZRDO_LAMBDA, rms: SLIGHT_ZRDO_RMS, coder: Coder::Zstd },
];
const MODERATE_STEPS: [Step; 2] = [
    Step::Coarse { bits: MODERATE_COARSE_BITS, rms: MODERATE_COARSE_RMS },
    Step::Zrdo { lambda: MODERATE_ZRDO_LAMBDA, rms: MODERATE_ZRDO_RMS, coder: Coder::Zstd },
];

/// How much the BC7 mobile animation pages are conditioned after the encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Bc7LossyLevel {
    /// The blocks as encoded.
    Lossless,
    /// Decent compression and the best quality, with minimal loss: the default.
    #[default]
    Minimal,
    /// Slight quality loss.
    Slight,
    /// Moderate quality loss.
    Moderate,
}

/// The level of the mobile animation packers unless they are asked for another (the user, 2026-11-27).
pub const MOBILE_ANIM_BC7_LOSSY_LEVEL: Bc7LossyLevel = Bc7LossyLevel::Minimal;

impl Bc7LossyLevel {
    pub const ALL: [Self; 4] = [Self::Lossless, Self::Minimal, Self::Slight, Self::Moderate];

    /// The chain of steps, empty for [`Self::Lossless`].
    pub const fn steps(self) -> &'static [Step] {
        match self {
            Self::Lossless => &[],
            Self::Minimal => &MINIMAL_STEPS,
            Self::Slight => &SLIGHT_STEPS,
            Self::Moderate => &MODERATE_STEPS,
        }
    }

    /// The name the CLI and the GUI config use.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Lossless => "lossless",
            Self::Minimal => "minimal",
            Self::Slight => "slight",
            Self::Moderate => "moderate",
        }
    }
}

/// Which placements of a package follow one another in a clip, by a placement key of the packer's (one per placed
/// frame position on a page), in the order the study's harness reads them: the clips in animation record order, a
/// clip whose frame run another one already had skipped, its unplaced frames dropped, each ordered pair of different
/// placements once.
pub struct ClipNeighbours {
    pairs: Vec<(u64, u64)>,
    pairs_of: HashMap<u64, Vec<u32>>,
}

impl ClipNeighbours {
    /// `runs` are the animation records' frame runs (first frame record, count) in record order; `placement_of`
    /// gives a frame record's placement key, `None` when it has no pixels on a page.
    pub fn new(runs: impl IntoIterator<Item = (u32, u32)>, placement_of: impl Fn(u32) -> Option<u64>) -> Self {
        let mut seen_runs = HashSet::new();
        let mut seen_pairs = HashSet::new();
        let mut pairs = Vec::new();
        let mut pairs_of: HashMap<u64, Vec<u32>> = HashMap::new();
        for (start, count) in runs {
            if !seen_runs.insert((start, count)) {
                continue;
            }
            let run: Vec<u64> = (start..start.saturating_add(count)).filter_map(&placement_of).collect();
            for pair in run.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if a != b && seen_pairs.insert((a, b)) {
                    let at = pairs.len() as u32;
                    pairs.push((a, b));
                    pairs_of.entry(a).or_default().push(at);
                    pairs_of.entry(b).or_default().push(at);
                }
            }
        }
        Self { pairs, pairs_of }
    }

    /// The neighbours on one page of each of its placements `keys` (in the page's order), by position in `keys`.
    pub fn on_page(&self, keys: &[u64]) -> Vec<Vec<(usize, ())>> {
        let local: HashMap<u64, usize> = keys.iter().enumerate().map(|(at, &key)| (key, at)).collect();
        let mut on_page: Vec<u32> = keys.iter().filter_map(|key| self.pairs_of.get(key)).flatten().copied().collect();
        on_page.sort_unstable();
        on_page.dedup();
        let mut adjacent = vec![Vec::new(); keys.len()];
        for at in on_page {
            let (a, b) = self.pairs[at as usize];
            if let (Some(&a), Some(&b)) = (local.get(&a), local.get(&b)) {
                adjacent[a].push((b, ()));
                adjacent[b].push((a, ()));
            }
        }
        adjacent
    }
}

/// A page's frames for the conditioning: each placement's allocation and anchor, and its clip neighbours on the page.
#[derive(Debug, Clone, Default)]
pub struct PageClips {
    pub placements: Vec<Placement>,
    pub adjacent: Vec<Vec<(usize, ())>>,
}

/// `blocks`, the BC7 encode of the page `rgba` (`width` × `height` RGBA8888), conditioned at `level`.
pub fn condition_page(level: Bc7LossyLevel, rgba: &[u8], width: u32, height: u32, blocks: Vec<u8>, clips: &PageClips)
    -> Result<Vec<u8>, ConditionError> {
    if level == Bc7LossyLevel::Lossless {
        return Ok(blocks);
    }
    let conditioner = PageConditioner::new(rgba, width, height, &blocks, &clips.placements, &clips.adjacent)?;
    Ok(conditioner.condition(level.steps()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_the_study_chains() {
        assert_eq!(Bc7LossyLevel::default(), MOBILE_ANIM_BC7_LOSSY_LEVEL);
        assert_eq!(MOBILE_ANIM_BC7_LOSSY_LEVEL, Bc7LossyLevel::Minimal);
        assert!(Bc7LossyLevel::Lossless.steps().is_empty());
        assert_eq!(Bc7LossyLevel::Minimal.steps(), [Step::Fields { lambda: 30.0, rms: 3.0, coder: Coder::Zstd }]);
        assert_eq!(Bc7LossyLevel::Slight.steps(), [Step::Coarse { bits: 2, rms: 2.0 },
            Step::Zrdo { lambda: 30.0, rms: 3.0, coder: Coder::Zstd }]);
        assert_eq!(Bc7LossyLevel::Moderate.steps(), [Step::Coarse { bits: 2, rms: 4.0 },
            Step::Zrdo { lambda: 100.0, rms: 6.0, coder: Coder::Zstd }]);
    }

    #[test]
    fn clip_neighbours_follow_the_runs_once() {
        // Frames 0..6; frame 3 is unplaced, frames 4 and 5 share a placement; the second record repeats the first run.
        let placement = [Some(10), Some(11), Some(12), None, Some(14), Some(14)];
        let neighbours = ClipNeighbours::new([(0, 3), (0, 3), (2, 4), (1, 1)], |frame| placement[frame as usize]);
        assert_eq!(neighbours.pairs, [(10, 11), (11, 12), (12, 14)]);
        // A page holding 12, 10 and 14 (not 11): only the pairs with both ends on it.
        assert_eq!(neighbours.on_page(&[12, 10, 14]), [vec![(2, ())], vec![], vec![(0, ())]]);
    }
}
