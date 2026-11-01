use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::hues_mul_uop::{
    convert_hues_mul_to_uop, convert_hues_mul_to_uop_on_base, convert_hues_uop_to_mul, HueRampMode,
};

#[derive(Subcommand, Debug)]
pub enum HuesCmd {
    /// Convert a Classic hues.mul to an Enhanced Client Hues.uop, in EA's layout plus a metadata entry that
    /// makes `uop-to-mul` give back the identical hues.mul.
    MulToUop {
        /// Source hues.mul.
        #[arg(long)]
        mul: PathBuf,
        /// Output Hues.uop.
        #[arg(long)]
        uop: PathBuf,
        /// How the 32 colours of a hue fill its 256-texel table row.
        #[arg(long, value_enum, default_value_t = HueRamp::Linear)]
        hue_ramp: HueRamp,
        /// An existing Hues.uop (EA's) to write on: every hue the hues.mul leaves unedited keeps that package's
        /// bitmap, atlas row, swatch and huenames.csv line byte for byte; the others are written by the rules
        /// with the linear ramp. Not the output.
        #[arg(long)]
        base: Option<PathBuf>,
    },
    /// Convert an Enhanced Client Hues.uop back to a Classic hues.mul. Packages without the metadata entry (EA's)
    /// give exact colours and default table start/end, names from huenames.csv, and zero group headers.
    UopToMul {
        /// Source Hues.uop.
        #[arg(long)]
        uop: PathBuf,
        /// Output hues.mul.
        #[arg(long)]
        mul: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum HueRamp {
    /// Colour i at x = 8i, ramping linearly to colour i + 1 (EA's Hues.uop).
    #[default]
    Linear,
    /// Each colour repeated over 8 texels.
    Flat,
}

impl From<HueRamp> for HueRampMode {
    fn from(ramp: HueRamp) -> Self {
        match ramp {
            HueRamp::Linear => HueRampMode::Linear,
            HueRamp::Flat => HueRampMode::Flat,
        }
    }
}

pub fn run(cmd: HuesCmd) -> eyre::Result<()> {
    match cmd {
        HuesCmd::MulToUop { mul, uop, hue_ramp, base: Some(base) } => {
            eyre::ensure!(hue_ramp == HueRamp::Linear, "--base keeps EA's bitmaps and rebuilds the others with the linear ramp");
            let reuse = convert_hues_mul_to_uop_on_base(&mul, &base, &uop, None).wrap_err_with(|| {
                format!("failed to convert {} to {} on {}", mul.display(), uop.display(), base.display())
            })?;
            println!(
                "Wrote {}: {} bitmaps and {} name lines kept from {}; {} bitmaps and {} name lines rebuilt.",
                uop.display(),
                reuse.bitmaps_kept,
                reuse.names_kept,
                base.display(),
                reuse.bitmaps_rebuilt.len(),
                reuse.names_rebuilt.len()
            );
        }
        HuesCmd::MulToUop { mul, uop, hue_ramp, base: None } => {
            convert_hues_mul_to_uop(&mul, &uop, hue_ramp.into(), None)
                .wrap_err_with(|| format!("failed to convert {} to {}", mul.display(), uop.display()))?;
            println!("Wrote {} ({hue_ramp:?} ramp).", uop.display());
        }
        HuesCmd::UopToMul { uop, mul } => {
            let recovered = convert_hues_uop_to_mul(&uop, &mul)
                .wrap_err_with(|| format!("failed to convert {} to {}", uop.display(), mul.display()))?;
            println!(
                "Wrote {}: {} hues, metadata {}, {} colour sets from the atlas, {} hues without colours.",
                mul.display(),
                recovered.hues.hue_count(),
                if recovered.had_metadata { "restored" } else { "defaulted (no metadata entry)" },
                recovered.colors_from_atlas.len(),
                recovered.missing_colors.len()
            );
        }
    }
    Ok(())
}
