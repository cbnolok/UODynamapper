use std::path::Path;
use std::time::Duration;

use color_eyre::eyre::{self, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use uocf::udd::{BuildProgress, BuildProgressPhase, UddpBuilder};

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

fn phase_message(phase: BuildProgressPhase) -> &'static str {
    match phase {
        BuildProgressPhase::TrainingDictionaries => "training compression dictionaries",
        BuildProgressPhase::CompressingFiles => "compressing package payloads",
        BuildProgressPhase::Assembling => "assembling package bytes",
    }
}

pub fn build_and_write_package(builder: &mut UddpBuilder, out_file: &Path) -> eyre::Result<()> {
    let bar = ProgressBar::new(1);
    bar.set_style(build_style());

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
            bar.set_message(phase_message(progress.phase).to_string());
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
    bar.finish_and_clear();
    println!("saved {}", out_file.display());

    Ok(())
}
