//! The mobiles' measured strides in the animation packages (`udd_assets::animation::mobile_strides`): what a repack
//! keeps of the strides measured on the package it replaces (those whose clip's frames still hash the same; the
//! others, and the new clips, get none until measured again), and the measurement itself, with the measurers of
//! `uot_gpu_compute::strides`: the planted foot's on the CPU (the default), or the flow's on the GPU.
//!
//! [`measure_strides`] hashes every clip of a package on all threads, and measures the clips whose frames are new or
//! changed since their stride was measured (or every clip again), keeping the others and dropping the strides of
//! clips gone: the planted foot on all threads, the flow on a GPU device of its own started only when something needs
//! measuring. A clip sharing the frames of another takes its measurement. [`measure_package_strides`] does that to a
//! package file and writes it only when its strides changed. `udd-cli tool strides measure`, the inspector and the
//! converter GUI run it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use color_eyre::eyre::{self, WrapErr};
use log::{info, warn};
use rayon::prelude::*;
use udd_assets::animation::mobile_strides::{
    format_stride_table, load_mobile_anim_clips, plan_strides, read_package_strides, sort_stride_records,
    stride_clip_hashes, strides_still_matching, write_package_strides, MobileAnimClips, MobileClip,
    MobileStrideRecord, StridePlan, StrideSource, STRIDE_FACINGS, STRIDE_FACING_COUNT,
};
use uot_gpu_compute::strides::planted_foot::measure_planted_foot;
use uot_gpu_compute::strides::{Measurement, StrideClip, StrideFrame, StrideMeasurer};

/// The strides of the package at `path` (none without one, or when it does not read, which is logged): read before a
/// repack replaces it.
pub fn strides_before_repack(path: &Path) -> Vec<MobileStrideRecord> {
    if !path.is_file() {
        return Vec::new();
    }
    read_package_strides(path).unwrap_or_else(|error| {
        warn!("The strides of {} do not read, so the repack keeps none: {error:#}", path.display());
        Vec::new()
    })
}

/// Stores in the package at `path`, loaded by `load`, those of `previous` whose clip's frames hash the same in it,
/// over its own strides of the same clips. Gives how many were kept; the package is rewritten (its other entries
/// copied as stored) only when its strides change.
pub fn keep_matching_strides<P: MobileAnimClips + Sync>(path: &Path, load: impl FnOnce(&Path) -> eyre::Result<P>,
    previous: &[MobileStrideRecord]) -> eyre::Result<usize> {
    if previous.is_empty() {
        return Ok(0);
    }
    let package = load(path).wrap_err_with(|| format!("load {} to keep its strides", path.display()))?;
    keep_loaded_strides(path, Box::new(package), previous)
}

/// Carries matching strides from the destination package into a staged replacement before it is installed.
pub fn keep_staged_strides(staged: &Path, destination: &Path) -> eyre::Result<usize> {
    let previous = strides_before_repack(destination);
    if previous.is_empty() {
        return Ok(0);
    }
    keep_loaded_strides(staged, load_mobile_anim_clips(staged)?, &previous)
}

fn keep_loaded_strides<P: MobileAnimClips + Sync + ?Sized>(path: &Path, package: Box<P>,
    previous: &[MobileStrideRecord]) -> eyre::Result<usize> {
    let threads = std::thread::available_parallelism().map_or(1, |threads| threads.get());
    let kept = strides_still_matching(&*package, previous, threads);
    let own = package.stride_records().to_vec();
    drop(package);
    let mut records: BTreeMap<(u32, u16), MobileStrideRecord> = own.iter()
        .map(|record| ((record.body_id, record.action_id), *record)).collect();
    records.extend(kept.iter().map(|record| ((record.body_id, record.action_id), *record)));
    let records: Vec<MobileStrideRecord> = records.into_values().collect();
    info!("{}: kept {} of {} strides measured before; {} were measured on other frames or clips now gone and need \
        measuring again", path.display(), kept.len(), previous.len(), previous.len() - kept.len());
    if records != own {
        write_package_strides(path, path, &records)?;
    }
    Ok(kept.len())
}

/// The text report of a package's strides: `mobile_strides_<cc|ec>.tsv`.
const REPORT_PREFIX: &str = "mobile_strides_";
const REPORT_EXTENSION: &str = "tsv";
const MILLIS_PER_SEC: f32 = 1000.0;

/// How a clip's stride is measured (`uot_gpu_compute::strides`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StrideMethod {
    /// Follows the planted foot's ground contact from frame to frame, on the CPU's threads.
    #[default]
    PlantedFoot,
    /// Reads the RIFE flow of the feet, on the GPU: measures the human walks a fifth to a third short.
    Flow,
}

/// Which clips a measurement takes, and how.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeasureOptions {
    /// Measure every clip again, not only the changed and new ones. Strides measured by the other method are kept
    /// unless their clips are measured again.
    pub everything: bool,
    /// Only the clips of these server bodies; the strides of the others stay as they are. `None`: every body.
    pub bodies: Option<BTreeSet<u32>>,
    pub method: StrideMethod,
}

/// The stride measurement of a clip's side facings, [`STRIDE_FACINGS`].
pub type SideMeasurement = Measurement<STRIDE_FACING_COUNT>;

/// What a measurement is doing, for a progress display. Hashing reports come from the hashing threads.
#[derive(Debug, Clone, Copy)]
pub enum StrideProgress<'a> {
    /// `done` of `total` clips hashed.
    Hashed { done: usize, total: usize },
    /// The GPU device is up (the first clip that needs measuring started it).
    GpuStarted { adapter: &'a str },
    /// Clip `index` of `total` to measure begins (the planted foot's on several threads, `index` counting them
    /// as they begin): measured, or taking the measurement of a clip with the same frames.
    Measuring { index: usize, total: usize, body: u32, action: u16 },
    /// A clip of a `source` package measured.
    Measured { source: StrideSource, record: &'a MobileStrideRecord, measurement: &'a SideMeasurement },
}

/// A package's strides after a measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredStrides {
    pub source: StrideSource,
    /// Every stride of the package now, sorted by body and action.
    pub records: Vec<MobileStrideRecord>,
    /// The strides it had.
    pub existing: Vec<MobileStrideRecord>,
    /// Clips a stride is measured for (of the bodies asked for).
    pub clips: usize,
    pub without_frames: usize,
    pub unchanged: usize,
    pub changed: usize,
    pub new: usize,
    pub removed: usize,
    /// Clips measured, and those that took the measurement of a clip with the same frames.
    pub measured: usize,
    pub shared: usize,
    pub threads: usize,
    pub hash_secs: f32,
    pub measure_secs: f32,
    /// The clip slowest to hash and its time.
    pub slowest: (u32, u16, Duration),
}

impl MeasuredStrides {
    /// Whether the strides differ from those the package had.
    pub fn changed_package(&self) -> bool {
        self.records != self.existing
    }

    /// One line: the clips and what became of their strides.
    pub fn summary(&self) -> String {
        let with_stride = self.records.iter().filter(|record| record.stride_tiles.is_some()).count();
        format!("{}: {} clips, {} without frames; {} unchanged, {} changed, {} new, {} removed; {} measured, {} \
            sharing the frames of another; {with_stride} with a stride, {} without", self.source.label(), self.clips,
            self.without_frames, self.unchanged, self.changed, self.new, self.removed, self.measured, self.shared,
            self.records.len() - with_stride)
    }

    /// One line: the time of each step, the package's loading taking `load_secs`.
    pub fn timing(&self, load_secs: f32) -> String {
        let (body, action, took) = self.slowest;
        format!("{}: loaded in {load_secs:.1} s, hashed in {:.1} s on {} threads (slowest: body {body} action \
            {action}, {:.1} ms), measured in {:.1} s", self.source.label(), self.hash_secs, self.threads,
            took.as_secs_f32() * MILLIS_PER_SEC, self.measure_secs)
    }
}

/// The threads clip hashes run on: every hardware thread.
pub fn hash_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |threads| threads.get())
}

/// The hash now of each clip of `package` a stride is measured for, of `bodies` (`None`: all), on all threads; the
/// clips with their hash (`None` without frames), and the time and clip of the slowest.
pub fn current_clip_hashes<P: MobileAnimClips + Sync + ?Sized>(package: &P, bodies: Option<&BTreeSet<u32>>,
    progress: &(dyn Fn(StrideProgress) + Sync)) -> eyre::Result<(Vec<(u32, u16, Option<u64>)>, (u32, u16, Duration))> {
    let clips: Vec<(u32, u16)> = package.stride_clips().into_iter()
        .filter(|(body, _)| bodies.is_none_or(|bodies| bodies.contains(body))).collect();
    let slowest = Mutex::new((Duration::ZERO, 0));
    let hashes = stride_clip_hashes(package, &clips, hash_threads(), |done, index, took| {
        let mut slowest = slowest.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if took > slowest.0 {
            *slowest = (took, index);
        }
        progress(StrideProgress::Hashed { done, total: clips.len() });
    });
    let current = clips.iter().zip(hashes)
        .map(|(&(body, action), hash)| Ok((body, action, hash.wrap_err_with(|| format!("body {body} action {action}"))?)))
        .collect::<eyre::Result<Vec<_>>>()?;
    let (took, index) = slowest.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (body, action) = clips.get(index).copied().unwrap_or_default();
    Ok((current, (body, action, took)))
}

/// The stored strides of `package` sorted out against its clips now (of `bodies`, `None`: all): which are unchanged,
/// changed, new and removed.
pub fn stride_status<P: MobileAnimClips + Sync + ?Sized>(package: &P, bodies: Option<&BTreeSet<u32>>,
    progress: &(dyn Fn(StrideProgress) + Sync)) -> eyre::Result<StridePlan> {
    let (current, _) = current_clip_hashes(package, bodies, progress)?;
    let existing: Vec<MobileStrideRecord> = package.stride_records().iter()
        .filter(|record| bodies.is_none_or(|bodies| bodies.contains(&record.body_id))).copied().collect();
    Ok(plan_strides(&current, &existing, false))
}

/// Measures the strides of `package` as `options` say, starting `measurer` the first time a clip needs the GPU (the
/// flow). `None` when `cancel` was set: nothing is kept of a cancelled measurement.
pub fn measure_strides<P: MobileAnimClips + Sync + ?Sized>(package: &P, options: &MeasureOptions,
    measurer: &mut Option<StrideMeasurer>, cancel: &AtomicBool, progress: &(dyn Fn(StrideProgress) + Sync))
    -> eyre::Result<Option<MeasuredStrides>> {
    let bodies = options.bodies.as_ref();
    let hashing = Instant::now();
    let (current, slowest) = current_clip_hashes(package, bodies, progress)?;
    let hash_secs = hashing.elapsed().as_secs_f32();
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }

    let existing = package.stride_records().to_vec();
    let (asked, others): (Vec<MobileStrideRecord>, Vec<MobileStrideRecord>) = existing.iter()
        .partition(|record| bodies.is_none_or(|bodies| bodies.contains(&record.body_id)));
    let plan = plan_strides(&current, &asked, options.everything);
    // Clips sharing their frames share a measurement: those kept give theirs to the changed and new. Measuring
    // everything again, only the clips measured now give theirs.
    let mut known: HashMap<u64, ([Option<f32>; STRIDE_FACING_COUNT], Option<f32>)> = if options.everything {
        HashMap::new()
    } else {
        plan.unchanged.iter().chain(&others)
            .map(|record| (record.clip_hash, (record.facing_tiles, record.stride_tiles))).collect()
    };
    let measuring = Instant::now();
    // The planted foot measures the clips of frames not known yet on all threads first, one clip of each.
    let mut planted: HashMap<u64, SideMeasurement> = HashMap::new();
    if options.method == StrideMethod::PlantedFoot {
        let mut seen = BTreeSet::new();
        let unknown: Vec<(u32, u16, u64)> = plan.to_measure.iter().copied()
            .filter(|&(_, _, clip_hash)| !known.contains_key(&clip_hash) && seen.insert(clip_hash)).collect();
        let begun = std::sync::atomic::AtomicUsize::new(0);
        let measurements = unknown.par_iter().map(|&(body, action, clip_hash)| {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let index = begun.fetch_add(1, Ordering::Relaxed);
            progress(StrideProgress::Measuring { index, total: unknown.len(), body, action });
            let clips = package.side_clips(body, action).wrap_err_with(|| format!("body {body} action {action}"))?;
            Ok(Some((clip_hash, measure_side_clips_planted(&clips))))
        }).collect::<eyre::Result<Vec<_>>>()?;
        for measurement in measurements {
            let Some((clip_hash, measurement)) = measurement else { return Ok(None) };
            planted.insert(clip_hash, measurement);
        }
    }
    let (mut measured, mut shared) = (0, 0);
    let mut records = plan.unchanged.clone();
    records.extend(&others);
    for (index, &(body, action, clip_hash)) in plan.to_measure.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if options.method == StrideMethod::Flow {
            progress(StrideProgress::Measuring { index, total: plan.to_measure.len(), body, action });
        }
        let (facing_tiles, stride_tiles) = match known.get(&clip_hash) {
            Some(&known) => {
                shared += 1;
                known
            }
            None => {
                let measurement = match options.method {
                    StrideMethod::PlantedFoot => *planted.get(&clip_hash).expect("measured above"),
                    StrideMethod::Flow => {
                        let measurer = match measurer {
                            Some(measurer) => measurer,
                            None => {
                                let started = measurer.insert(StrideMeasurer::new()?);
                                progress(StrideProgress::GpuStarted { adapter: started.adapter() });
                                started
                            }
                        };
                        measure_side_clips(measurer, &package.side_clips(body, action)?)
                            .wrap_err_with(|| format!("measure body {body} action {action}"))?
                    }
                };
                measured += 1;
                let record = MobileStrideRecord { body_id: body, action_id: action, clip_hash,
                    stride_tiles: measurement.stride.ok(), facing_tiles: measurement.facings };
                progress(StrideProgress::Measured { source: package.stride_source(), record: &record,
                    measurement: &measurement });
                known.insert(clip_hash, (record.facing_tiles, record.stride_tiles));
                (record.facing_tiles, record.stride_tiles)
            }
        };
        records.push(MobileStrideRecord { body_id: body, action_id: action, clip_hash, stride_tiles, facing_tiles });
    }
    sort_stride_records(&mut records);
    Ok(Some(MeasuredStrides {
        source: package.stride_source(), records, existing, clips: current.len(),
        without_frames: plan.without_frames, unchanged: plan.unchanged.len(), changed: plan.changed, new: plan.new,
        removed: plan.removed.len(), measured, shared, threads: hash_threads(), hash_secs,
        measure_secs: measuring.elapsed().as_secs_f32(), slowest,
    }))
}

/// Measures a clip's side facings, as [`MobileAnimClips::side_clips`] gives them, by the flow.
pub fn measure_side_clips(measurer: &mut StrideMeasurer, clips: &[Option<MobileClip>; STRIDE_FACING_COUNT])
    -> eyre::Result<SideMeasurement> {
    measurer.measure(&stride_clips(clips))
}

/// Measures a clip's side facings, as [`MobileAnimClips::side_clips`] gives them, by the planted foot.
pub fn measure_side_clips_planted(clips: &[Option<MobileClip>; STRIDE_FACING_COUNT]) -> SideMeasurement {
    measure_planted_foot(&stride_clips(clips))
}

fn stride_clips(clips: &[Option<MobileClip>; STRIDE_FACING_COUNT]) -> [Option<StrideClip<'_>>; STRIDE_FACING_COUNT] {
    std::array::from_fn(|index| {
        clips[index].as_ref().map(|clip| StrideClip {
            facing: STRIDE_FACINGS[index], mirrored: clip.mirrored,
            frames: clip.frames.iter().map(|frame| StrideFrame { width: frame.width, height: frame.height,
                anchor_x: frame.anchor_x, anchor_y: frame.anchor_y, rgba: &frame.rgba }).collect(),
        })
    })
}

/// The report file of a package's strides in `folder`.
pub fn report_path(folder: &Path, source: StrideSource) -> PathBuf {
    folder.join(format!("{REPORT_PREFIX}{}.{REPORT_EXTENSION}", source.token()))
}

/// What [`measure_package_strides`] did to a package file.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageStrides {
    pub strides: MeasuredStrides,
    pub load_secs: f32,
    /// The time writing the package took; `None` when its strides were unchanged and it was left as it is.
    pub write_secs: Option<f32>,
    pub report: Option<PathBuf>,
}

/// Measures the strides of the mobile animation package at `path` and writes the package with them to `output`
/// (which may be `path`: replaced through a partial file beside it) when they changed or `output` is another file;
/// with `report`, writes their text table into that folder too. `None` when cancelled, with nothing written.
pub fn measure_package_strides(path: &Path, output: &Path, options: &MeasureOptions, report: Option<&Path>,
    measurer: &mut Option<StrideMeasurer>, cancel: &AtomicBool, progress: &(dyn Fn(StrideProgress) + Sync))
    -> eyre::Result<Option<PackageStrides>> {
    let loading = Instant::now();
    let package = load_mobile_anim_clips(path)?;
    let load_secs = loading.elapsed().as_secs_f32();
    let Some(strides) = measure_strides(&*package, options, measurer, cancel, progress)
        .wrap_err_with(|| format!("measure the strides of {}", path.display()))? else {
        return Ok(None);
    };
    drop(package);

    let report = report.map(|folder| -> eyre::Result<PathBuf> {
        std::fs::create_dir_all(folder).wrap_err_with(|| format!("create {}", folder.display()))?;
        let report = report_path(folder, strides.source);
        let comments = [format!("The strides of {}, in tiles per cycle (udd-cli tool strides measure).",
            path.display()), strides.summary()];
        std::fs::write(&report, format_stride_table(strides.source, &strides.records, &comments))
            .wrap_err_with(|| format!("write {}", report.display()))?;
        Ok(report)
    }).transpose()?;
    if !strides.changed_package() && output == path {
        return Ok(Some(PackageStrides { strides, load_secs, write_secs: None, report }));
    }
    if let Some(folder) = output.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder).wrap_err_with(|| format!("create {}", folder.display()))?;
    }
    let writing = Instant::now();
    write_package_strides(path, output, &strides.records)?;
    Ok(Some(PackageStrides { strides, load_secs, write_secs: Some(writing.elapsed().as_secs_f32()), report }))
}

/// What became of a clip's stride since it was measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ClipStrideState {
    /// The clip's frames hash as when its stride was measured.
    Unchanged,
    /// Its frames changed: the stride needs measuring again.
    Changed,
    /// It has no stride yet.
    New,
    /// The clip is gone, or has no frames: the next measurement drops its stride.
    Removed,
}

impl ClipStrideState {
    pub const ALL: [Self; 4] = [Self::Unchanged, Self::Changed, Self::New, Self::Removed];

    pub fn label(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Changed => "changed",
            Self::New => "new",
            Self::Removed => "removed",
        }
    }
}

/// Each clip of a [`stride_status`] `plan` and each stride of `existing` (the package's, the plan made from them), by
/// body and action: its state, and its stored stride if any.
pub fn clip_stride_states(plan: &StridePlan, existing: &[MobileStrideRecord])
    -> BTreeMap<(u32, u16), (ClipStrideState, Option<MobileStrideRecord>)> {
    let stored: BTreeMap<(u32, u16), MobileStrideRecord> = existing.iter()
        .map(|record| ((record.body_id, record.action_id), *record)).collect();
    let mut states = BTreeMap::new();
    for record in &plan.unchanged {
        states.insert((record.body_id, record.action_id), (ClipStrideState::Unchanged, Some(*record)));
    }
    for &(body, action, _) in &plan.to_measure {
        let record = stored.get(&(body, action)).copied();
        let state = if record.is_some() { ClipStrideState::Changed } else { ClipStrideState::New };
        states.insert((body, action), (state, record));
    }
    for record in &plan.removed {
        states.insert((record.body_id, record.action_id), (ClipStrideState::Removed, Some(*record)));
    }
    states
}

/// Where a measurement is, for a progress bar.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StrideJobProgress {
    /// What it does now: hashing the clips, starting the GPU, measuring a clip.
    pub activity: String,
    /// How far the current step is, 0 to 1.
    pub fraction: f32,
}

impl StrideJobProgress {
    /// The progress a report tells; `None` for a clip measured, which the next report follows.
    pub fn of(step: StrideProgress) -> Option<Self> {
        let (activity, fraction) = match step {
            StrideProgress::Hashed { done, total } => {
                (format!("hashing the clips: {done} of {total}"), done as f32 / total.max(1) as f32)
            }
            StrideProgress::GpuStarted { adapter } => (format!("measuring on {adapter}"), 0.0),
            StrideProgress::Measuring { index, total, body, action } => {
                (format!("measuring body {body} action {action} ({} of {total})", index + 1),
                    index as f32 / total.max(1) as f32)
            }
            StrideProgress::Measured { .. } => return None,
        };
        Some(Self { activity, fraction })
    }
}

/// Work on the strides of a package on a thread of its own, for a GUI: its progress as it goes, a cancel, and its
/// result once done.
pub struct StrideJob<T> {
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<StrideJobProgress>>,
    thread: Option<JoinHandle<eyre::Result<Option<T>>>>,
}

impl<T: Send + 'static> StrideJob<T> {
    /// Runs `work` with the cancel flag and a progress callback, calling `repaint` on each report and at the end.
    /// `work` gives `None` when cancelled.
    pub fn spawn(
        work: impl FnOnce(&AtomicBool, &(dyn Fn(StrideProgress) + Sync)) -> eyre::Result<Option<T>> + Send + 'static,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(StrideJobProgress::default()));
        let thread = {
            let (cancel, progress) = (cancel.clone(), progress.clone());
            std::thread::spawn(move || {
                let report = |step: StrideProgress| {
                    let Some(now) = StrideJobProgress::of(step) else { return };
                    *progress.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = now;
                    repaint();
                };
                let result = work(&cancel, &report);
                repaint();
                result
            })
        };
        Self { cancel, progress, thread: Some(thread) }
    }

    pub fn progress(&self) -> StrideJobProgress {
        self.progress.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Asks the work to stop; it gives `None` once it does.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelling(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The result, once the work ended: `Ok(None)` when it was cancelled.
    pub fn finished(&mut self) -> Option<eyre::Result<Option<T>>> {
        if !self.thread.as_ref().is_some_and(|thread| thread.is_finished()) {
            return None;
        }
        let thread = self.thread.take()?;
        Some(thread.join().unwrap_or_else(|_| Err(eyre::eyre!("the stride work panicked"))))
    }
}

/// A job dropped before it ended is cancelled: a measurement writes nothing.
impl<T> Drop for StrideJob<T> {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Classic package to measure (`mobile_anim_cc.uddp`).
    const MOBILE_ANIM_CC_ENV: &str = "UDD_MOBILE_ANIM_CC";
    /// Classic body 1's walk, and its side facings' strides in the first stride table (the prototype's method).
    const TABLE_BODY: u32 = 1;
    const TABLE_ACTION: u16 = 0;
    const TABLE_FACING_TILES: [f32; STRIDE_FACING_COUNT] = [1.445, 2.105, 1.919];
    /// The table's three decimals.
    const TABLE_TOLERANCE_TILES: f32 = 0.0005;

    #[test]
    #[ignore = "requires a GPU adapter and UDD_MOBILE_ANIM_CC; run explicitly for stride or RIFE changes"]
    fn a_classic_walk_measures_the_strides_of_the_first_table() {
        let path = std::env::var_os(MOBILE_ANIM_CC_ENV).unwrap_or_else(|| panic!("set {MOBILE_ANIM_CC_ENV}"));
        let package = load_mobile_anim_clips(Path::new(&path)).unwrap();
        let mut measurer = StrideMeasurer::new().unwrap();
        let measurement =
            measure_side_clips(&mut measurer, &package.side_clips(TABLE_BODY, TABLE_ACTION).unwrap()).unwrap();
        for (facing, expected) in measurement.facings.iter().zip(TABLE_FACING_TILES) {
            let facing = facing.unwrap_or_else(|| panic!("{:?}", measurement.facings));
            assert!((facing - expected).abs() < TABLE_TOLERANCE_TILES, "{:?}", measurement.facings);
        }
    }
}
