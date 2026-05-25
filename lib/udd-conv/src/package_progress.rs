use std::path::Path;
use std::time::Duration;

use color_eyre::eyre::{self, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use udd_container::{BuildProgress, BuildProgressPhase, CompressionFlag, CompressionSummary, UddpBuilder};

fn spinner_style() -> ProgressStyle {
    ProgressStyle::default_spinner()
        .template("{spinner:.green} [{elapsed_precise}] {msg}")
        .unwrap()
}

fn build_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.green/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-")
}

fn save_style() -> ProgressStyle {
    ProgressStyle::default_spinner()
        .template("{spinner:.green} [{elapsed_precise}] {msg}")
        .unwrap()
}

fn compression_message(summary: CompressionSummary) -> String {
    let zstd = summary.zstd_no_dict + summary.zstd_dict;
    let mut parts = Vec::new();
    if zstd > 0 {
        parts.push(format!("Zstd payload pass: {zstd} files"));
    }
    if summary.jpeg_xl > 0 {
        parts.push(format!("JPEG XL payload pass: {} files", summary.jpeg_xl));
    }
    if summary.auto > 0 {
        parts.push(format!("auto compression evaluation: {} files", summary.auto));
    }
    if summary.raw > 0 {
        parts.push(format!("raw payload storage: {} files", summary.raw));
    }
    if parts.is_empty() {
        "package payload pass: no files".to_string()
    } else {
        parts.join(", ")
    }
}

fn phase_message(phase: BuildProgressPhase, compression_summary: CompressionSummary) -> String {
    match phase {
        BuildProgressPhase::TrainingDictionaries => "training compression dictionaries".to_string(),
        BuildProgressPhase::CompressingFiles => compression_message(compression_summary),
        BuildProgressPhase::Assembling => "assembling package bytes".to_string(),
    }
}

pub fn atlas_payload_progress_message(
    subject: &str,
    use_bc7: bool,
    compression: CompressionFlag,
    bc7_rdo_lambda: f32,
) -> String {
    if use_bc7 {
        if bc7_rdo_lambda > 0.0 && bc7_rdo_lambda.is_finite() {
            format!("BC7-compressing {subject}; RDO pass follows")
        } else {
            format!("BC7-compressing {subject}")
        }
    } else {
        match compression {
            CompressionFlag::JpegXl => {
                format!("registering {subject} for JPEG XL package compression")
            }
            CompressionFlag::JpegXlZstd | CompressionFlag::JpegXlZstdLevel(_) => {
                format!("registering {subject} for JPEG XL plus Zstd package compression")
            }
            CompressionFlag::None => format!("registering uncompressed {subject}"),
            CompressionFlag::ZstdNoDict
            | CompressionFlag::ZstdNoDictLevel(_)
            | CompressionFlag::ZstdDict
            | CompressionFlag::Auto => format!("registering {subject} for package compression"),
        }
    }
}

pub fn atlas_payload_finish_message(
    subject: &str,
    use_bc7: bool,
    compression: CompressionFlag,
    bc7_rdo_lambda: f32,
) -> String {
    if use_bc7 {
        if bc7_rdo_lambda > 0.0 && bc7_rdo_lambda.is_finite() {
            format!("{subject} BC7-compressed with RDO")
        } else {
            format!("{subject} BC7-compressed")
        }
    } else {
        match compression {
            CompressionFlag::JpegXl => {
                format!("{subject} registered for JPEG XL package compression")
            }
            CompressionFlag::JpegXlZstd | CompressionFlag::JpegXlZstdLevel(_) => {
                format!("{subject} registered for JPEG XL plus Zstd package compression")
            }
            CompressionFlag::None => format!("{subject} registered uncompressed"),
            CompressionFlag::ZstdNoDict
            | CompressionFlag::ZstdNoDictLevel(_)
            | CompressionFlag::ZstdDict
            | CompressionFlag::Auto => format!("{subject} registered for package compression"),
        }
    }
}

pub fn build_and_write_package(builder: &mut UddpBuilder, out_file: &Path) -> eyre::Result<()> {
    let bar = ProgressBar::new(1);
    bar.set_style(build_style());
    let compression_summary = builder.compression_summary();

    let mut active_phase = None;
    let bytes = builder.build_with_progress(|progress: BuildProgress| {
        let total = progress.total.max(1) as u64;
        if active_phase != Some(progress.phase) || bar.length() != Some(total) {
            active_phase = Some(progress.phase);
            match progress.phase {
                BuildProgressPhase::TrainingDictionaries => {
                    bar.set_style(spinner_style());
                    bar.enable_steady_tick(Duration::from_millis(100));
                    bar.set_length(1);
                    bar.set_position(0);
                }
                BuildProgressPhase::CompressingFiles | BuildProgressPhase::Assembling => {
                    bar.disable_steady_tick();
                    bar.set_style(build_style());
                    bar.set_length(total);
                    bar.set_position(0);
                }
            }
            bar.set_message(phase_message(progress.phase, compression_summary));
        }

        match progress.phase {
            BuildProgressPhase::TrainingDictionaries => {
                if progress.completed >= progress.total {
                    bar.set_position(1);
                }
            }
            BuildProgressPhase::CompressingFiles | BuildProgressPhase::Assembling => {
                bar.set_position(progress.completed.min(progress.total) as u64);
            }
        }
    })?;

    bar.set_style(save_style());
    bar.enable_steady_tick(Duration::from_millis(100));
    bar.set_message(format!("writing {}", out_file.display()));
    std::fs::write(out_file, bytes).wrap_err_with(|| format!("save {}", out_file.display()))?;
    bar.finish_with_message(format!("saved {}", out_file.display()));

    Ok(())
}
