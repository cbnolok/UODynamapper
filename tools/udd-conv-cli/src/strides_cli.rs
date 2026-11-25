//! `udd-cli tool strides`: the mobiles' measured strides in `mobile_anim_cc.uddp` and `mobile_anim_ec.uddp`
//! (`udd_assets::animation::mobile_strides`): measured (`udd_conv::animation::mobile_strides`), as a text table in
//! and out, which of them are stale, the stride clip sources a repack carries them over by, and the stride cache the
//! packers fill new packages from (`udd_conv::animation::stride_cache`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use clap::{Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use log::{info, warn};
use udd_assets::animation::mobile_strides::{
    format_stride_table, load_mobile_anim_clips, parse_stride_table, plan_strides, read_package_stride_sources,
    stride_clip_hashes, stride_table_fields, write_package_stride_entries, write_package_strides,
    MobileStrideRecord, StrideSource, StrideSourceRecord, STRIDE_FACINGS, STRIDE_FACING_COUNT,
};
use udd_conv::animation::mobile_strides::{
    hash_threads, inspect_side_clips, measure_package_strides, side_tiles_of_pairs, ClipInspection, MeasureOptions,
    StrideMethod, StrideProgress,
};
use udd_conv::animation::stride_cache::StrideCache;
use udd_conv::animation::stride_sources::stride_sources;

/// Which clips `strides measure` measures.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasureMode {
    /// The clips whose frames are new or changed since their stride was measured; the other strides are kept.
    Changed,
    /// Every clip again.
    Full,
}

/// How `strides measure` measures a clip.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasureMethod {
    /// Follows the planted foot's ground contact from frame to frame, on the CPU's threads.
    PlantedFoot,
    /// Reads the RIFE flow of the feet on the GPU; measures the human walks a fifth to a third short.
    Flow,
}

impl From<MeasureMethod> for StrideMethod {
    fn from(method: MeasureMethod) -> Self {
        match method {
            MeasureMethod::PlantedFoot => Self::PlantedFoot,
            MeasureMethod::Flow => Self::Flow,
        }
    }
}

/// Clips hashed between progress lines.
const HASH_PROGRESS_EVERY: usize = 500;

#[derive(Subcommand)]
pub enum StridesCommand {
    /// Measure the strides (in side tiles a cycle) and store them in the packages: hashes every clip on all threads,
    /// then measures only what the mode asks for (a clip sharing the frames of one kept or measured takes its
    /// measurement), drops the strides of clips gone, and rewrites a package only when its strides changed. Prints
    /// each clip measured as a text table line with the facings' spread and the verdict, then a summary and the
    /// timings.
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
        #[arg(long, value_enum, default_value_t = MeasureMethod::PlantedFoot)]
        method: MeasureMethod,
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
    /// Show what the planted foot follows on a clip's side facings (south, south-west, west), pair by pair: each
    /// pair's backward travel in screen px, its ground band pixels and the pixels that voted for the travel, and the
    /// facing's stride. With `--pair`, also the stride counted on those pairs alone (their median travel times the
    /// frames, over a side tile's screen x), the count for a clip whose other pairs follow a wing, a tail or a robe.
    /// Any action of the body, measured or not.
    Inspect {
        file: PathBuf,
        #[arg(long)]
        body: u32,
        #[arg(long)]
        action: u16,
        /// Only this side facing, 4, 5 or 6 (repeatable); by default all three.
        #[arg(long = "facing")]
        facings: Vec<u8>,
        /// Count a stride on the pairs starting at these frames (repeatable) of each facing shown.
        #[arg(long = "pair")]
        pairs: Vec<usize>,
        /// Write each facing's frames side by side into this folder as `strip_<cc|ec>_<body>_<action>_<facing>.png`:
        /// the ground band tinted blue, the pixels that voted for the pair's travel red, and under each frame a bar
        /// the pair's travel long (grey without one).
        #[arg(long)]
        strip: Option<PathBuf>,
    },
    /// Hash every clip a stride is measured for and say which strides are stale: unchanged, changed (other frames
    /// now), new (no stride yet), removed (the clip is gone), and the clips to measure.
    Status {
        file: PathBuf,
        /// List the clips to measure, one per line.
        #[arg(long)]
        list: bool,
    },
    /// Record in the packages their stride clip sources, which a repack carries the strides over by to new frames
    /// made from the same client frames: each clip's hash next to the hash of the client frames it was made from,
    /// read from the client files the package was packed from. A package packed before the entry existed needs it
    /// once; the packers record it. The strides and the other entries are copied as stored.
    Sources {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// The Classic client folder a Classic package was packed from.
        #[arg(long)]
        ccdir: Option<PathBuf>,
        /// The Enhanced client folder an Enhanced package was packed from.
        #[arg(long)]
        ecdir: Option<PathBuf>,
        /// Write the packages into this folder, under their names; by default each replaces itself through a
        /// `.partial` file beside it.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Write the stride cache the packers fill new packages from (MobileStrides.tsv of the conversion tables): each
    /// stride the packages hold that was measured on their frames, keyed by package kind, body, action and the hash
    /// of the client frames the clip was made from, at full precision. Measure the packages first (`measure full`).
    /// A clip's client frames come from its client folder when given, else from the stride clip sources the package
    /// records. Strides of several packages and tables (several client versions) merge; the same client frames with
    /// other values are an error.
    CacheExport {
        /// Measured packages; none merges the --merge tables alone.
        #[arg(required_unless_present = "merges")]
        files: Vec<PathBuf>,
        /// The Classic client folder the Classic packages were packed from.
        #[arg(long)]
        ccdir: Option<PathBuf>,
        /// The Enhanced client folder the Enhanced packages were packed from.
        #[arg(long)]
        ecdir: Option<PathBuf>,
        /// A stride cache to merge in (repeatable): the shipped table, or the export of another client version's
        /// packages.
        #[arg(long = "merge")]
        merges: Vec<PathBuf>,
        /// By default the standard output.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

pub fn run(command: StridesCommand) -> eyre::Result<()> {
    match command {
        StridesCommand::Measure { mode, files, bodies, output, report, method } => {
            measure(mode, method, &files, bodies, output.as_deref(), report.as_deref())
        }
        StridesCommand::Export { file, output } => export(&file, output.as_deref()),
        StridesCommand::Import { file, table, trust_unhashed, output } => {
            import(&file, &table, trust_unhashed, output.as_deref().unwrap_or(&file))
        }
        StridesCommand::Inspect { file, body, action, facings, pairs, strip } => {
            inspect(&file, body, action, &facings, &pairs, strip.as_deref())
        }
        StridesCommand::Status { file, list } => status(&file, list),
        StridesCommand::Sources { files, ccdir, ecdir, output } => {
            sources(&files, ccdir.as_deref(), ecdir.as_deref(), output.as_deref())
        }
        StridesCommand::CacheExport { files, ccdir, ecdir, merges, output } => {
            cache_export(&files, ccdir.as_deref(), ecdir.as_deref(), &merges, output.as_deref())
        }
    }
}

/// The side facings' names, in the order of `STRIDE_FACINGS`.
const FACING_NAMES: [&str; STRIDE_FACING_COUNT] = ["south", "south-west", "west"];
/// `inspect --strip`: each canvas pixel drawn this many times over, and in canvas px the room kept around the
/// silhouettes, the travel bar's height under each frame and the gap between frames.
const STRIP_SCALE: u32 = 2;
const STRIP_PAD_PX: i32 = 4;
const STRIP_BAR_PX: u32 = 3;
const STRIP_GAP_PX: u32 = 2;
/// The strip's colours: behind the frames, behind them in the ground band, the voting pixels (blended over the frame
/// at [`STRIP_VOTER_OPACITY`]), a pair's travel bar, and the bar of a pair without a travel.
const STRIP_BACKDROP: [u8; 3] = [48, 48, 48];
const STRIP_BAND: [u8; 3] = [36, 60, 120];
const STRIP_VOTER: [u8; 3] = [255, 32, 32];
const STRIP_VOTER_OPACITY: f32 = 0.75;
const STRIP_TRAVEL: [u8; 3] = [64, 220, 96];
const STRIP_NO_TRAVEL: [u8; 3] = [128, 128, 128];
/// A frame pixel counts toward the silhouette's bounds above this alpha.
const STRIP_OPAQUE_ALPHA: u8 = 127;
const MAX_ALPHA: f32 = 255.0;
const RGBA_BYTES: usize = 4;
const RGB_BYTES: usize = 3;

fn tiles_text(tiles: Option<f32>) -> String {
    tiles.map_or("none".to_string(), |tiles| format!("{tiles:.3} tiles"))
}

fn inspect(file: &Path, body: u32, action: u16, facings: &[u8], pairs: &[usize], strip: Option<&Path>)
    -> eyre::Result<()> {
    if let Some(facing) = facings.iter().find(|facing| !STRIDE_FACINGS.contains(facing)) {
        eyre::bail!("facing {facing} is not a side facing ({STRIDE_FACINGS:?})");
    }
    let package = load_mobile_anim_clips(file)?;
    let source = package.stride_source().token();
    let clips = package.side_clips(body, action).wrap_err_with(|| format!("body {body} action {action}"))?;
    drop(package);
    if let Some(folder) = strip {
        std::fs::create_dir_all(folder).wrap_err_with(|| format!("create {}", folder.display()))?;
    }
    for ((facing, name), inspection) in STRIDE_FACINGS.iter().zip(FACING_NAMES).zip(inspect_side_clips(&clips)) {
        if !facings.is_empty() && !facings.contains(facing) {
            continue;
        }
        let clip = format!("{source} body {body} action {action} facing {facing} ({name})");
        let Some(inspection) = inspection else {
            println!("{clip}: nothing to measure (no frames, fewer than two, or no travel across the screen)");
            continue;
        };
        let frames = inspection.frames.len();
        println!("{clip}: {frames} frames, a side tile {:.1} px across; stride {}", inspection.side_tile_x,
            tiles_text(inspection.stride));
        println!("pair\ttravel_px\tvoters\tband_px");
        for pair in &inspection.pairs {
            let travel = pair.travel_px.map_or("-".to_string(), |travel| format!("{travel:.2}"));
            println!("{}-{}\t{travel}\t{}\t{}", pair.from, pair.to, pair.voters.len(), pair.band_pixels);
        }
        if !pairs.is_empty() {
            if let Some(pair) = pairs.iter().find(|&&pair| pair >= frames) {
                eyre::bail!("{clip} has no pair {pair}: its pairs start at 0 to {}", frames - 1);
            }
            let travels: Vec<f32> = inspection.pairs.iter().filter(|pair| pairs.contains(&pair.from))
                .filter_map(|pair| pair.travel_px).collect();
            println!("pairs {pairs:?}: {} of {} with a travel; stride {}", travels.len(), pairs.len(),
                tiles_text(side_tiles_of_pairs(&travels, frames, inspection.side_tile_x)));
        }
        if let Some(folder) = strip {
            let path = folder.join(format!("strip_{source}_{body}_{action}_{facing}.png"));
            write_strip(&inspection, &path)?;
            info!("Wrote {}.", path.display());
        }
    }
    Ok(())
}

/// The frames of `inspection` side by side, cropped to the silhouettes: the ground band tinted, the pixels that
/// voted for each pair's travel marked on its first frame, and under each frame a bar its pair's travel long.
fn write_strip(inspection: &ClipInspection, path: &Path) -> eyre::Result<()> {
    let width = inspection.width;
    let opaque = |frame: &[u8], at: usize| frame[at * RGBA_BYTES + 3] > STRIP_OPAQUE_ALPHA;
    let (mut left, mut top, mut right, mut bottom) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for frame in &inspection.frames {
        for at in (0..frame.len() / RGBA_BYTES).filter(|&at| opaque(frame, at)) {
            let (x, y) = (at as i32 % width, at as i32 / width);
            (left, top, right, bottom) = (left.min(x), top.min(y), right.max(x), bottom.max(y));
        }
    }
    eyre::ensure!(left <= right, "the clip has no opaque pixel");
    let (left, top) = ((left - STRIP_PAD_PX).max(0), (top - STRIP_PAD_PX).max(0));
    let (right, bottom) = ((right + STRIP_PAD_PX).min(width - 1), (bottom + STRIP_PAD_PX).min(inspection.height - 1));
    let (panel_w, panel_h) = ((right - left + 1) as u32, (bottom - top + 1) as u32);
    let panel_stride = panel_w + STRIP_GAP_PX;
    let (strip_w, strip_h) =
        (panel_stride * inspection.frames.len() as u32 * STRIP_SCALE, (panel_h + STRIP_BAR_PX) * STRIP_SCALE);
    let mut rgb = vec![0u8; (strip_w * strip_h) as usize * RGB_BYTES];
    let mut put = |x: u32, y: u32, colour: [u8; RGB_BYTES]| {
        for dy in 0..STRIP_SCALE {
            for dx in 0..STRIP_SCALE {
                let at = ((y * STRIP_SCALE + dy) * strip_w + x * STRIP_SCALE + dx) as usize * RGB_BYTES;
                rgb[at..at + RGB_BYTES].copy_from_slice(&colour);
            }
        }
    };
    let blend = |under: [u8; RGB_BYTES], over: [u8; RGB_BYTES], opacity: f32| -> [u8; RGB_BYTES] {
        std::array::from_fn(|channel| (f32::from(under[channel]) * (1.0 - opacity)
            + f32::from(over[channel]) * opacity).round() as u8)
    };
    for (index, (frame, pair)) in inspection.frames.iter().zip(&inspection.pairs).enumerate() {
        let panel_x = index as u32 * panel_stride;
        let voters: BTreeSet<usize> = pair.voters.iter().copied().collect();
        for y in top..=bottom {
            for x in left..=right {
                let at = (y * width + x) as usize;
                let pixel = &frame[at * RGBA_BYTES..(at + 1) * RGBA_BYTES];
                let behind = if inspection.band[at] { STRIP_BAND } else { STRIP_BACKDROP };
                let mut colour = blend(behind, [pixel[0], pixel[1], pixel[2]], f32::from(pixel[3]) / MAX_ALPHA);
                if voters.contains(&at) {
                    colour = blend(colour, STRIP_VOTER, STRIP_VOTER_OPACITY);
                }
                put(panel_x + (x - left) as u32, (y - top) as u32, colour);
            }
        }
        let (length, colour) = match pair.travel_px {
            Some(travel) => ((travel.round() as u32).min(panel_w), STRIP_TRAVEL),
            None => (panel_w, STRIP_NO_TRAVEL),
        };
        for y in panel_h..panel_h + STRIP_BAR_PX {
            for x in 0..length {
                put(panel_x + x, y, colour);
            }
        }
    }
    image::save_buffer_with_format(path, &rgb, strip_w, strip_h, image::ColorType::Rgb8, image::ImageFormat::Png)
        .wrap_err_with(|| format!("write {}", path.display()))
}

fn measure(mode: MeasureMode, method: MeasureMethod, files: &[PathBuf], bodies: Vec<u32>, output: Option<&Path>,
    report: Option<&Path>) -> eyre::Result<()> {
    let started = Instant::now();
    let options = MeasureOptions { everything: mode == MeasureMode::Full,
        bodies: (!bodies.is_empty()).then(|| bodies.into_iter().collect::<BTreeSet<u32>>()), method: method.into() };
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

/// The client folder of `source` among `ccdir` and `ecdir`, with the option naming it.
fn client_dir_of<'a>(source: StrideSource, ccdir: Option<&'a Path>, ecdir: Option<&'a Path>)
    -> (Option<&'a Path>, &'static str) {
    match source {
        StrideSource::Classic => (ccdir, "--ccdir"),
        StrideSource::Enhanced => (ecdir, "--ecdir"),
    }
}

fn cache_export(files: &[PathBuf], ccdir: Option<&Path>, ecdir: Option<&Path>, merges: &[PathBuf],
    output: Option<&Path>) -> eyre::Result<()> {
    let mut cache = StrideCache::default();
    for file in files {
        let started = Instant::now();
        let package = load_mobile_anim_clips(file)?;
        let source = package.stride_source();
        let (client_dir, argument) = client_dir_of(source, ccdir, ecdir);
        let sources: Vec<StrideSourceRecord> = match client_dir {
            Some(client_dir) => stride_sources(&*package, client_dir)
                .wrap_err_with(|| format!("the stride sources of {}", file.display()))?,
            None => read_package_stride_sources(file)?,
        };
        eyre::ensure!(!sources.is_empty(), "{} records no stride clip sources: name its client folder with \
            {argument}", file.display());
        let records = package.stride_records();
        let before = cache.len();
        let added = cache.add_package(source, records, &sources)
            .wrap_err_with(|| format!("add the strides of {}", file.display()))?;
        let measured: BTreeSet<(u32, u16)> = records.iter().map(|record| (record.body_id, record.action_id)).collect();
        let unmeasured = sources.iter().filter(|clip| !measured.contains(&(clip.body_id, clip.action_id))).count();
        info!("{}: {added} of {} stride clips with a stride measured on their frames, {} new to the cache; {} \
            strides of other frames left out, {unmeasured} clips never measured; in {:.1} s.", file.display(),
            sources.len(), cache.len() - before, records.len() - added, started.elapsed().as_secs_f32());
        if unmeasured != 0 {
            warn!("{}: {unmeasured} clips have no stride record and stay to measure on every pack: measure the \
                package (`udd-cli tool strides measure changed`) before exporting it.", file.display());
        }
    }
    for table in merges {
        let before = cache.len();
        cache.merge(StrideCache::load(table)?).wrap_err_with(|| format!("merge {}", table.display()))?;
        info!("{}: {} strides new to the cache.", table.display(), cache.len() - before);
    }
    let text = cache.format();
    match output {
        Some(output) => {
            std::fs::write(output, text).wrap_err_with(|| format!("write {}", output.display()))?;
            info!("Wrote the {} cached strides to {}.", cache.len(), output.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

fn sources(files: &[PathBuf], ccdir: Option<&Path>, ecdir: Option<&Path>, output: Option<&Path>) -> eyre::Result<()> {
    for file in files {
        let target = match output {
            Some(folder) => folder.join(file.file_name().ok_or_else(|| eyre::eyre!("{} names no file",
                file.display()))?),
            None => file.clone(),
        };
        let started = Instant::now();
        let package = load_mobile_anim_clips(file)?;
        let source = package.stride_source();
        let (client_dir, argument) = match source {
            StrideSource::Classic => (ccdir, "--ccdir"),
            StrideSource::Enhanced => (ecdir, "--ecdir"),
        };
        let client_dir = client_dir.ok_or_else(|| eyre::eyre!("{} is a {} package: name its client folder with \
            {argument}", file.display(), source.label()))?;
        let sources = stride_sources(&*package, client_dir)
            .wrap_err_with(|| format!("the stride sources of {}", file.display()))?;
        let clip_hashes: BTreeMap<(u32, u16), u64> = sources.iter()
            .map(|record| ((record.body_id, record.action_id), record.clip_hash)).collect();
        let records = package.stride_records();
        let holding = records.iter()
            .filter(|record| clip_hashes.get(&(record.body_id, record.action_id)) == Some(&record.clip_hash)).count();
        info!("{}: the sources of {} clips read from {} in {:.1} s; {holding} of the {} strides are of the frames the \
            package has, and a repack carries them over to new frames of the same sources.", source.label(),
            sources.len(), client_dir.display(), started.elapsed().as_secs_f32(), records.len());
        drop(package);
        let writing = Instant::now();
        write_package_stride_entries(file, &target, None, Some(&sources))?;
        info!("Wrote {} in {:.1} s.", target.display(), writing.elapsed().as_secs_f32());
    }
    Ok(())
}
