//! The 2x frames of a mobile animation package made ahead of time (`udd_conv::animation::mobile_prescale`): the pack
//! commands' `--prescale` options, and `udd-cli tool prescale-mobile-anims` on a package already packed.

use std::path::PathBuf;

use clap::Args;
use color_eyre::eyre;
use log::info;
use udd_conv::animation::mobile_prescale::{prescale_package, PrescaleEncoding, PrescaleOptions, PrescaleSummary};

const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

/// The pack commands' prescale options: off unless one is given.
#[derive(Args, Clone, Debug, Default, PartialEq, Eq)]
pub struct PrescaleArgs {
    /// Also pack 2x frames made ahead of time beside the frames, for the bodies MobilePrescale.kdl lists as "always".
    /// Not recommended: the renderer makes the same 2x frames at run time, and the package grows a lot (the Classic
    /// bodies of at most 48 pixels a side add 41 % to a BC7 Classic package, 127 % to one of RGBA pages).
    #[arg(long)]
    pub prescale: bool,
    /// With --prescale implied: also every body whose largest stored frame is at most this many pixels wide and high
    /// (the smallest bodies), but those MobilePrescale.kdl lists as "never".
    #[arg(long = "prescale-max-side", value_name = "PIXELS")]
    pub prescale_max_side: Option<u16>,
    /// MobilePrescale.kdl to read. Default: the copy shipped with the tools.
    #[arg(long = "prescale-kdl")]
    pub prescale_kdl: Option<PathBuf>,
}

impl From<PrescaleArgs> for PrescaleOptions {
    fn from(args: PrescaleArgs) -> Self {
        Self {
            enabled: args.prescale || args.prescale_max_side.is_some(),
            max_side: args.prescale_max_side,
            table_path: args.prescale_kdl,
        }
    }
}

/// `udd-cli tool prescale-mobile-anims`.
#[derive(Args, Clone, Debug)]
pub struct PrescaleToolArgs {
    /// The mobile_anim_cc.uddp or mobile_anim_ec.uddp.
    pub package: PathBuf,
    /// Where to write the package with its 2x frames. Default: in place.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Every body whose largest stored frame is at most this many pixels wide and high, but those MobilePrescale.kdl
    /// lists as "never". Without it, the "always" bodies alone; with neither, the package loses its 2x frames.
    #[arg(long = "max-side", value_name = "PIXELS")]
    pub max_side: Option<u16>,
    /// MobilePrescale.kdl to read. Default: the copy shipped with the tools.
    #[arg(long)]
    pub kdl: Option<PathBuf>,
    /// BC7 RDO lambda of the 2x pages of a BC7 package. 0: none.
    #[arg(long = "bc7-rdo-lambda", default_value_t = 0.0)]
    pub bc7_rdo_lambda: f32,
    /// Also measure the 2x frames against the upscale on each frame's own canvas, and the page encode's difference.
    #[arg(long)]
    pub compare: bool,
}

pub fn run_tool(args: PrescaleToolArgs) -> eyre::Result<()> {
    let output = args.output.clone().unwrap_or_else(|| args.package.clone());
    let options = PrescaleOptions { enabled: true, max_side: args.max_side, table_path: args.kdl };
    let encoding = PrescaleEncoding {
        compression: udd_conv::page_compression::MOBILE_ANIM_PAGE_COMPRESSION,
        bc7_rdo_lambda: args.bc7_rdo_lambda,
        bc7_rdo_lookback_blocks: udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
    };
    let summary = prescale_package(&args.package, &output, &options, &encoding, args.compare)?;
    report(&summary);
    Ok(())
}

/// Logs what a prescale did.
pub fn report(summary: &PrescaleSummary) {
    info!("Prescaled {} frames of {} frame sets into {} pages: {:.1} MiB -> {:.1} MiB ({:+.1} MiB, {:+.2} %) in \
        {:.1} s (upscale {:.1} s on {}, encode and write {:.1} s).", summary.frames, summary.frame_sets,
        summary.pages, summary.bytes_before as f64 / BYTES_PER_MIB, summary.bytes_after as f64 / BYTES_PER_MIB,
        (summary.bytes_after as f64 - summary.bytes_before as f64) / BYTES_PER_MIB,
        (summary.bytes_after as f64 / summary.bytes_before.max(1) as f64 - 1.0) * 100.0,
        summary.total_time.as_secs_f64(), summary.upscale_time.as_secs_f64(), summary.adapter,
        summary.encode_time.as_secs_f64());
    if let Some(comparison) = summary.comparison {
        info!("Against the upscale on each frame's own canvas: {} of {} texels differ, by at most {} steps. The page \
            encode changes {} texels, PSNR {:.2} dB.", comparison.canvas_differing, comparison.texels,
            comparison.canvas_worst, comparison.encode_differing, comparison.encode_psnr_db);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_max_side_turns_the_prescale_on() {
        assert!(!PrescaleOptions::from(PrescaleArgs::default()).enabled);
        let options = PrescaleOptions::from(PrescaleArgs { prescale_max_side: Some(64), ..Default::default() });
        assert!(options.enabled);
        assert_eq!(options.max_side, Some(64));
        assert!(PrescaleOptions::from(PrescaleArgs { prescale: true, ..Default::default() }).enabled);
    }
}
