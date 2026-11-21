//! `udd-cli tool strides`: the mobiles' measured strides in `mobile_anim_cc.uddp` and `mobile_anim_ec.uddp`
//! (`udd_assets::animation::mobile_strides`): measured on the GPU (`udd_conv::animation::mobile_strides`), as a text
//! table in and out, and which of them are stale.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use clap::{Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use log::info;
use udd_assets::animation::mobile_strides::{
    format_stride_table, load_mobile_anim_clips, parse_stride_table, plan_strides, stride_clip_hashes,
    stride_table_fields, write_package_strides, MobileStrideRecord,
};
use udd_conv::animation::mobile_strides::{hash_threads, measure_package_strides, MeasureOptions, StrideProgress};

/// Which clips `strides measure` measures.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasureMode {
    /// The clips whose frames are new or changed since their stride was measured; the other strides are kept.
    Changed,
    /// Every clip again.
    Full,
}

/// Clips hashed between progress lines.
const HASH_PROGRESS_EVERY: usize = 500;

#[derive(Subcommand)]
pub enum StridesCommand {
    /// Measure the strides on the GPU and store them in the packages: hashes every clip on all threads, then measures
    /// only what the mode asks for (a clip sharing the frames of one kept or measured takes its measurement), drops
    /// the strides of clips gone, and rewrites a package only when its strides changed. Prints each clip measured as a
    /// text table line with the facings' spread and the verdict, then a summary and the timings.
    Measure {
        mode: MeasureMode,
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Only the clips of this server body (repeatable); the other bodies' strides stay as they are.
        #[arg(long = "body")]
        bodies: Vec<u32>,
        /// Write the packages into this folder, under their names; by default each replaces itself through a
        /// `.partial` file beside it.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Write each package's strides into this folder as a text table, `mobile_strides_<cc|ec>.tsv`.
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Write a package's strides as a text table: source, body, action, the stride of each side facing (south,
    /// south-west, west), the stride and the hash of the frames measured; `-` for none.
    Export {
        file: PathBuf,
        /// By default the standard output.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Store the strides of a text table (an export, or a `measure --report`) in a package: each
    /// row whose hash matches the clip's frames now replaces the package's stride of that clip; stale rows are
    /// skipped. The other entries are copied as stored.
    Import {
        file: PathBuf,
        #[arg(long, required = true)]
        table: PathBuf,
        /// Take rows without a hash as measured on the frames the package has now.
        #[arg(long)]
        trust_unhashed: bool,
        /// By default the package itself, replaced through a `.partial` file beside it.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Hash every clip a stride is measured for and say which strides are stale: unchanged, changed (other frames
    /// now), new (no stride yet), removed (the clip is gone), and the clips to measure.
    Status {
        file: PathBuf,
        /// List the clips to measure, one per line.
        #[arg(long)]
        list: bool,
    },
}

pub fn run(command: StridesCommand) -> eyre::Result<()> {
    match command {
        StridesCommand::Measure { mode, files, bodies, output, report } => {
            measure(mode, &files, bodies, output.as_deref(), report.as_deref())
        }
        StridesCommand::Export { file, output } => export(&file, output.as_deref()),
        StridesCommand::Import { file, table, trust_unhashed, output } => {
            import(&file, &table, trust_unhashed, output.as_deref().unwrap_or(&file))
        }
        StridesCommand::Status { file, list } => status(&file, list),
    }
}

fn measure(mode: MeasureMode, files: &[PathBuf], bodies: Vec<u32>, output: Option<&Path>, report: Option<&Path>)
    -> eyre::Result<()> {
    let started = Instant::now();
    let options = MeasureOptions { everything: mode == MeasureMode::Full,
        bodies: (!bodies.is_empty()).then(|| bodies.into_iter().collect::<BTreeSet<u32>>()) };
    let never = AtomicBool::new(false);
    let mut measurer = None;
    for file in files {
        let target = match output {
            Some(folder) => folder.join(file.file_name().ok_or_else(|| eyre::eyre!("{} names no file",
                file.display()))?),
            None => file.clone(),
        };
        let label = file.display().to_string();
        let progress = |progress: StrideProgress| match progress {
            StrideProgress::Hashed { done, total } if done % HASH_PROGRESS_EVERY == 0 => {
                info!("{label}: hashed {done} of {total} clips");
            }
            StrideProgress::GpuStarted { adapter } => info!("adapter: {adapter}"),
            StrideProgress::Measured { source, record, measurement } => {
                let spread = measurement.spread.map_or("-".to_string(), |spread| format!("{spread:.2}"));
                println!("{}\t{spread}\t{}", stride_table_fields(source, record), measurement.verdict());
            }
            _ => {}
        };
        let Some(done) = measure_package_strides(file, &target, &options, report, &mut measurer, &never, &progress)?
            else { unreachable!("nothing cancels the command line's measurement") };
        let label = done.strides.source.label();
        info!("{}", done.strides.summary());
        info!("{}", done.strides.timing(done.load_secs));
        if let Some(report) = &done.report {
            info!("{label}: wrote the report {}", report.display());
        }
        match done.write_secs {
            Some(secs) => info!("{label}: wrote {} in {secs:.1} s", target.display()),
            None => info!("{label}: the strides are unchanged; {} is left as it is", file.display()),
        }
    }
    info!("done in {:.1} s", started.elapsed().as_secs_f32());
    Ok(())
}

fn export(file: &Path, output: Option<&Path>) -> eyre::Result<()> {
    let package = load_mobile_anim_clips(file)?;
    let records = package.stride_records();
    let comments = [format!("The strides of {}, in tiles per cycle.", file.display())];
    let text = format_stride_table(package.stride_source(), records, &comments);
    match output {
        Some(output) => {
            std::fs::write(output, text).wrap_err_with(|| format!("write {}", output.display()))?;
            info!("Wrote the {} strides of {} to {}.", records.len(), file.display(), output.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

fn import(file: &Path, table: &Path, trust_unhashed: bool, output: &Path) -> eyre::Result<()> {
    let text = std::fs::read_to_string(table).wrap_err_with(|| format!("read {}", table.display()))?;
    let rows = parse_stride_table(&table.display().to_string(), &text)?;
    let package = load_mobile_anim_clips(file)?;
    let source = package.stride_source();
    let (rows, other_source): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| row.source == source);
    let clips: Vec<(u32, u16)> = rows.iter().map(|row| (row.body_id, row.action_id)).collect();
    let hashing = Instant::now();
    let hashes = stride_clip_hashes(&*package, &clips, hash_threads(), |_, _, _| {});
    let hash_secs = hashing.elapsed().as_secs_f32();

    let mut records: BTreeMap<(u32, u16), MobileStrideRecord> = package.stride_records().iter()
        .map(|record| ((record.body_id, record.action_id), *record)).collect();
    let (mut imported, mut stale, mut unhashed, mut without_frames) = (0, 0, 0, 0);
    for (row, hash) in rows.iter().zip(hashes) {
        let clip = format!("body {} action {}", row.body_id, row.action_id);
        let Some(hash_now) = hash.wrap_err_with(|| clip.clone())? else {
            without_frames += 1;
            continue;
        };
        match row.clip_hash {
            Some(measured) if measured != hash_now => {
                stale += 1;
                continue;
            }
            None if !trust_unhashed => {
                unhashed += 1;
                continue;
            }
            _ => {}
        }
        records.insert((row.body_id, row.action_id), MobileStrideRecord { body_id: row.body_id,
            action_id: row.action_id, clip_hash: hash_now, stride_tiles: row.stride_tiles,
            facing_tiles: row.facing_tiles });
        imported += 1;
    }
    info!("{}: {imported} of {} rows imported; skipped {stale} measured on other frames, {unhashed} without a hash \
        (--trust-unhashed takes them), {without_frames} of clips without frames, {} of the other source; hashed in \
        {hash_secs:.1} s.", source.label(), rows.len(), other_source.len());
    let records: Vec<MobileStrideRecord> = records.into_values().collect();
    drop(package);
    let writing = Instant::now();
    write_package_strides(file, output, &records)?;
    info!("Wrote {} with {} strides in {:.1} s.", output.display(), records.len(), writing.elapsed().as_secs_f32());
    Ok(())
}

fn status(file: &Path, list: bool) -> eyre::Result<()> {
    let package = load_mobile_anim_clips(file)?;
    let clips = package.stride_clips();
    let hashing = Instant::now();
    let hashes = stride_clip_hashes(&*package, &clips, hash_threads(), |_, _, _| {});
    let current = clips.iter().zip(hashes)
        .map(|(&(body, action), hash)| Ok((body, action, hash.wrap_err_with(|| format!("body {body} action {action}"))?)))
        .collect::<eyre::Result<Vec<_>>>()?;
    let plan = plan_strides(&current, package.stride_records(), false);
    info!("{}: {} clips hashed in {:.1} s: {} unchanged, {} changed, {} new, {} removed, {} without frames; {} to \
        measure.", package.stride_source().label(), clips.len(), hashing.elapsed().as_secs_f32(), plan.unchanged.len(),
        plan.changed, plan.new, plan.removed.len(), plan.without_frames, plan.to_measure.len());
    if list {
        for (body, action, hash) in &plan.to_measure {
            println!("{body}\t{action}\t{hash:016x}");
        }
    }
    Ok(())
}
