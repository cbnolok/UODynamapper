//! The measured strides of the mobiles' walk and run clips, stored in `mobile_anim_cc.uddp` and `mobile_anim_ec.uddp`
//! (`metadata/strides.bin`), and the clips they are measured on.
//!
//! A stride is how far a body's planted feet travel in one cycle of a clip, in tiles; Dynamapper plays a walk or run
//! cycle in the time the steps take to cover it (its RIFE measurement writes them). A stride belongs to the frames it
//! was measured on: each record carries the [`stride_clip_hash`] of the clip's side facings, and a stride is used only
//! while the package's clip still hashes to it. A package without the entry has no strides; its clips keep the
//! clients' timing.
//!
//! The text table (`format_stride_table`, `parse_stride_table`) is how strides move in and out of a package by hand
//! (`udd-cli tool strides`).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};
use udd_container::{DataType, UddpBuilder, UddpReader};
use xxhash_rust::xxh64::Xxh64;

use crate::common::{ensure_manifest_version, read_path_entry_cow};

pub const STRIDE_MANIFEST_ENTRY_PATH: &str = "metadata/strides.bin";
pub const STRIDE_MANIFEST_MAGIC: [u8; 4] = *b"MOST";
pub const MOBILE_STRIDES_VERSION: u32 = 1;
/// The facings a stride is measured on, in order: south, south-west and west, across the screen. Toward and away from
/// the camera the feet barely move on screen.
pub const STRIDE_FACINGS: [u8; 3] = [4, 5, 6];
pub const STRIDE_FACING_COUNT: usize = STRIDE_FACINGS.len();
/// The seed of [`stride_clip_hash`].
const CLIP_HASH_SEED: u64 = 0;
/// What [`stride_clip_hash`] hashes for a facing without frames, and before a facing's frames.
const ABSENT_FACING: u8 = 0;
const PRESENT_FACING: u8 = 1;
/// The suffix of a package being written next to the one it replaces.
const PARTIAL_SUFFIX: &str = ".partial";

/// A clip's measured stride: `stride_tiles` is the median of the side facings that measured, `None` when the clip was
/// measured and no stride taken (too few facings, too short, not positive): it keeps the clients' timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobileStrideRecord {
    /// The server body.
    pub body_id: u32,
    /// The action the clients' tables give for the movement.
    pub action_id: u16,
    /// [`stride_clip_hash`] of the frames measured.
    pub clip_hash: u64,
    pub stride_tiles: Option<f32>,
    /// The stride of each of [`STRIDE_FACINGS`], `None` where it did not measure.
    pub facing_tiles: [Option<f32>; STRIDE_FACING_COUNT],
}

/// A clip: the frames of an action of a body in one direction, as the clients show them (empty and skipped frames
/// dropped).
#[derive(Debug, Clone, PartialEq)]
pub struct MobileClip {
    pub frames: Vec<MobileClipFrame>,
    /// Drawn mirrored horizontally about the anchor.
    pub mirrored: bool,
    /// The clip's frames per second, 0 when the client keeps its own frame time (Classic MUL bodies).
    pub fps: f32,
    /// Which stored frames the clip shows among those of its body and action: a Classic clip's stored direction, an
    /// Enhanced clip's first frame record. A mirrored direction shares its twin's.
    pub stored: u32,
}

/// A frame of a clip: its size, its anchor (the point on the ground, from the frame's top-left corner) and its
/// straight-alpha RGBA pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct MobileClipFrame {
    pub width: u16,
    pub height: u16,
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub rgba: Vec<u8>,
}

/// The hash a stride is keyed to: of each of [`STRIDE_FACINGS`] in order, whether it has frames, its mirroring, and
/// each frame's size, anchor and pixels in order; all fixed-width little-endian, XXH64 with seed 0, so the same frames
/// hash the same on every platform and build. `None` when no facing has frames.
pub fn stride_clip_hash(clips: &[Option<MobileClip>; STRIDE_FACING_COUNT]) -> Option<u64> {
    if clips.iter().all(Option::is_none) {
        return None;
    }
    let mut hasher = Xxh64::new(CLIP_HASH_SEED);
    for clip in clips {
        let Some(clip) = clip else {
            hasher.update(&[ABSENT_FACING]);
            continue;
        };
        hasher.update(&[PRESENT_FACING, u8::from(clip.mirrored)]);
        hasher.update(&(clip.frames.len() as u32).to_le_bytes());
        for frame in &clip.frames {
            hasher.update(&frame.width.to_le_bytes());
            hasher.update(&frame.height.to_le_bytes());
            hasher.update(&frame.anchor_x.to_le_bytes());
            hasher.update(&frame.anchor_y.to_le_bytes());
            hasher.update(&frame.rgba);
        }
    }
    Some(hasher.digest())
}

/// The atlas pages last read for a clip's frames, a few at a time: the frames of a clip, and of its facings, mostly
/// share a page, so they read it once instead of once a frame.
#[derive(Default)]
pub(crate) struct PageMemo {
    pages: Vec<(u32, Vec<u8>)>,
}

/// The pages a [`PageMemo`] keeps.
const MEMO_PAGES: usize = 2;

impl PageMemo {
    /// The bytes of page `page_index`, read by `read` unless kept.
    pub(crate) fn page(&mut self, page_index: u32, read: impl FnOnce(u32) -> eyre::Result<Vec<u8>>)
        -> eyre::Result<&[u8]> {
        let index = match self.pages.iter().position(|(kept, _)| *kept == page_index) {
            Some(index) => index,
            None => {
                if self.pages.len() >= MEMO_PAGES {
                    self.pages.remove(0);
                }
                self.pages.push((page_index, read(page_index)?));
                self.pages.len() - 1
            }
        };
        Ok(&self.pages[index].1)
    }
}

/// Which client's package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StrideSource {
    Classic,
    Enhanced,
}

impl StrideSource {
    pub const ALL: [Self; 2] = [Self::Classic, Self::Enhanced];

    /// The name in the text table.
    pub fn token(self) -> &'static str {
        match self {
            Self::Classic => "cc",
            Self::Enhanced => "ec",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.token() == token)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Enhanced => "Enhanced",
        }
    }
}

/// A mobile animation package's clips and strides.
pub trait MobileAnimClips {
    fn stride_source(&self) -> StrideSource;
    /// The package's strides, sorted by body and action; empty without the entry.
    fn stride_records(&self) -> &[MobileStrideRecord];
    /// The clips a stride is measured for: each server body's walk and run actions (peace and war, and an Enhanced
    /// mount's mounted moves), as the client asks for them, sorted.
    fn stride_clips(&self) -> Vec<(u32, u16)>;
    /// The clip of `action` of server `body` facing `direction` (0 north, clockwise); `None` without frames.
    fn clip(&self, body: u32, action: u16, direction: u8) -> eyre::Result<Option<MobileClip>>;

    fn stride_record(&self, body: u32, action: u16) -> Option<&MobileStrideRecord> {
        let records = self.stride_records();
        records.binary_search_by_key(&(body, action), |record| (record.body_id, record.action_id)).ok()
            .map(|index| &records[index])
    }

    /// The clips of [`STRIDE_FACINGS`].
    fn side_clips(&self, body: u32, action: u16) -> eyre::Result<[Option<MobileClip>; STRIDE_FACING_COUNT]> {
        let mut clips: [Option<MobileClip>; STRIDE_FACING_COUNT] = Default::default();
        for (clip, direction) in clips.iter_mut().zip(STRIDE_FACINGS) {
            *clip = self.clip(body, action, direction)?;
        }
        Ok(clips)
    }

    /// [`stride_clip_hash`] of the clip's side facings now.
    fn stride_clip_hash(&self, body: u32, action: u16) -> eyre::Result<Option<u64>> {
        Ok(stride_clip_hash(&self.side_clips(body, action)?))
    }
}

/// A Classic or an Enhanced mobile animation package, whichever the file at `path` is.
pub fn load_mobile_anim_clips(path: &Path) -> eyre::Result<Box<dyn MobileAnimClips + Send + Sync>> {
    let classic = match crate::animation::mobile_anim_cc::MobileAnimCcPackage::load(path) {
        Ok(package) => return Ok(Box::new(package)),
        Err(error) => error,
    };
    crate::animation::mobile_anim_ec::MobileAnimEcPackage::load(path)
        .map(|package| Box::new(package) as Box<dyn MobileAnimClips + Send + Sync>)
        .map_err(|enhanced| eyre::eyre!("{} is not a mobile animation package: as Classic, {classic:#}; as \
            Enhanced, {enhanced:#}", path.display()))
}

/// [`MobileAnimClips::stride_clip_hash`] of each of `clips`, on `threads` threads, in order. `progress` hears of each
/// clip hashed: how many are done, the clip's index in `clips`, and the time it took.
pub fn stride_clip_hashes<P: MobileAnimClips + Sync + ?Sized>(package: &P, clips: &[(u32, u16)], threads: usize,
    progress: impl Fn(usize, usize, Duration) + Sync) -> Vec<eyre::Result<Option<u64>>> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let mut results: Vec<(usize, eyre::Result<Option<u64>>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.max(1)).map(|_| scope.spawn(|| {
            let mut hashed = Vec::new();
            loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(body, action)) = clips.get(index) else { break };
                let started = Instant::now();
                hashed.push((index, package.stride_clip_hash(body, action)));
                progress(done.fetch_add(1, Ordering::Relaxed) + 1, index, started.elapsed());
            }
            hashed
        })).collect();
        workers.into_iter().flat_map(|worker| worker.join().expect("a stride hash worker panicked")).collect()
    });
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

/// What a package's strides are, measured against its clips now.
#[derive(Debug, Default, PartialEq)]
pub struct StridePlan {
    /// Records whose clip still hashes the same, kept as they are.
    pub unchanged: Vec<MobileStrideRecord>,
    /// Clips to measure, with their hash now: those with a record of another hash (`changed`), those without a record
    /// (`new`), and, measuring everything, the rest too.
    pub to_measure: Vec<(u32, u16, u64)>,
    pub changed: usize,
    pub new: usize,
    /// Records of clips that no longer exist or have no frames.
    pub removed: Vec<MobileStrideRecord>,
    /// Clips without frames in any side facing: nothing to measure.
    pub without_frames: usize,
}

/// Sorts out `existing` records against `current`, each clip a stride is measured for with its hash now (`None`
/// without frames). Measuring `everything`, every clip with frames is to measure; otherwise only the changed and new
/// ones.
pub fn plan_strides(current: &[(u32, u16, Option<u64>)], existing: &[MobileStrideRecord], everything: bool)
    -> StridePlan {
    let mut existing: BTreeMap<(u32, u16), MobileStrideRecord> = existing.iter()
        .map(|record| ((record.body_id, record.action_id), *record)).collect();
    let mut plan = StridePlan::default();
    for &(body, action, hash) in current {
        let Some(hash) = hash else {
            plan.without_frames += 1;
            continue;
        };
        match existing.remove(&(body, action)) {
            Some(record) if record.clip_hash == hash && !everything => plan.unchanged.push(record),
            Some(record) => {
                if record.clip_hash != hash {
                    plan.changed += 1;
                }
                plan.to_measure.push((body, action, hash));
            }
            None => {
                plan.new += 1;
                plan.to_measure.push((body, action, hash));
            }
        }
    }
    plan.removed = existing.into_values().collect();
    plan
}

/// Sorts `records` by body and action, as the manifest stores them.
pub fn sort_stride_records(records: &mut [MobileStrideRecord]) {
    records.sort_unstable_by_key(|record| (record.body_id, record.action_id));
}

/// Record: `u32 body_id, u16 action_id, u64 clip_hash, f32 stride_tiles, f32 facing_tiles[3]` (NaN for none).
const STRIDE_RECORD_BYTES: usize = 30;

/// `metadata/strides.bin`: magic, version, `u32 count`, then the records sorted by body and action.
pub fn encode_stride_manifest(records: &[MobileStrideRecord]) -> Vec<u8> {
    let mut sorted = records.to_vec();
    sort_stride_records(&mut sorted);
    let mut bytes = Vec::with_capacity(STRIDE_MANIFEST_MAGIC.len() + 8 + sorted.len() * STRIDE_RECORD_BYTES);
    bytes.extend_from_slice(&STRIDE_MANIFEST_MAGIC);
    bytes.extend_from_slice(&MOBILE_STRIDES_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(sorted.len() as u32).to_le_bytes());
    let tiles = |tiles: Option<f32>| tiles.unwrap_or(f32::NAN).to_le_bytes();
    for record in &sorted {
        bytes.extend_from_slice(&record.body_id.to_le_bytes());
        bytes.extend_from_slice(&record.action_id.to_le_bytes());
        bytes.extend_from_slice(&record.clip_hash.to_le_bytes());
        bytes.extend_from_slice(&tiles(record.stride_tiles));
        for facing in record.facing_tiles {
            bytes.extend_from_slice(&tiles(facing));
        }
    }
    bytes
}

pub fn parse_stride_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileStrideRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != STRIDE_MANIFEST_MAGIC { eyre::bail!("invalid mobile stride manifest magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("mobile stride manifest", version, MOBILE_STRIDES_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut records = Vec::with_capacity(count);
    let tiles = |cursor: &mut Cursor<&[u8]>| -> eyre::Result<Option<f32>> {
        let tiles = cursor.read_f32::<LittleEndian>()?;
        Ok((!tiles.is_nan()).then_some(tiles))
    };
    for _ in 0..count {
        let body_id = cursor.read_u32::<LittleEndian>()?;
        let action_id = cursor.read_u16::<LittleEndian>()?;
        let clip_hash = cursor.read_u64::<LittleEndian>()?;
        let stride_tiles = tiles(&mut cursor)?;
        let mut facing_tiles = [None; STRIDE_FACING_COUNT];
        for facing in &mut facing_tiles {
            *facing = tiles(&mut cursor)?;
        }
        records.push(MobileStrideRecord { body_id, action_id, clip_hash, stride_tiles, facing_tiles });
    }
    eyre::ensure!(records.is_sorted_by_key(|record| (record.body_id, record.action_id)),
        "mobile stride manifest: the records are not sorted by body and action");
    Ok(records)
}

/// The strides of `package`, empty when it has no stride entry.
pub(crate) fn read_stride_records(package: &UddpReader) -> eyre::Result<Vec<MobileStrideRecord>> {
    if package.find_by_path_hash(udd_container::xxh64_virtual_path(STRIDE_MANIFEST_ENTRY_PATH)).is_none() {
        return Ok(Vec::new());
    }
    parse_stride_manifest(&read_path_entry_cow(package, STRIDE_MANIFEST_ENTRY_PATH)?)
        .wrap_err_with(|| format!("read {STRIDE_MANIFEST_ENTRY_PATH}"))
}

/// The strides stored in the package at `path`, read alone (none without the entry), for a repack that may keep them.
pub fn read_package_strides(path: &Path) -> eyre::Result<Vec<MobileStrideRecord>> {
    let package = UddpReader::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
    read_stride_records(&package).wrap_err_with(|| format!("read the strides of {}", path.display()))
}

/// Writes the package at `package` with `records` as its strides to `output`, which may be `package` itself: the
/// other entries are copied as stored ([`UddpBuilder::write_with_entries`]), to a partial file next to `output` that
/// then replaces it.
pub fn write_package_strides(package: &Path, output: &Path, records: &[MobileStrideRecord]) -> eyre::Result<()> {
    let manifest = encode_stride_manifest(records);
    let partial = {
        let mut name = output.as_os_str().to_owned();
        name.push(PARTIAL_SUFFIX);
        PathBuf::from(name)
    };
    {
        let base = UddpReader::load(package).wrap_err_with(|| format!("load {}", package.display()))?;
        let entry = (udd_container::xxh64_virtual_path(STRIDE_MANIFEST_ENTRY_PATH), DataType::Metadata as u8,
            manifest.as_slice());
        UddpBuilder::write_with_entries(&base, &[entry], &partial)
            .map_err(|error| eyre::eyre!("{error}"))
            .wrap_err_with(|| format!("write {}", partial.display()))?;
    }
    std::fs::rename(&partial, output).wrap_err_with(|| format!("replace {}", output.display()))
}

/// The strides of `previous` whose clip in `package` still hashes the same: what a repack keeps of the strides
/// measured on the package it replaces. The others need measuring again.
pub fn strides_still_matching<P: MobileAnimClips + Sync + ?Sized>(package: &P, previous: &[MobileStrideRecord],
    threads: usize) -> Vec<MobileStrideRecord> {
    let clips: Vec<(u32, u16)> = previous.iter().map(|record| (record.body_id, record.action_id)).collect();
    let hashes = stride_clip_hashes(package, &clips, threads, |_, _, _| {});
    previous.iter().zip(hashes)
        .filter(|(record, hash)| matches!(hash, Ok(Some(hash)) if *hash == record.clip_hash))
        .map(|(record, _)| *record)
        .collect()
}

/// A line of the text table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrideTableRow {
    pub source: StrideSource,
    pub body_id: u32,
    pub action_id: u16,
    pub facing_tiles: [Option<f32>; STRIDE_FACING_COUNT],
    pub stride_tiles: Option<f32>,
    /// `None` in a table measured without hashes (the measurement report of the first strides).
    pub clip_hash: Option<u64>,
}

/// The text table's columns: tab-separated under a header line, `-` for none, the hash as 16 hex digits.
const COLUMN_SOURCE: &str = "source";
const COLUMN_BODY: &str = "body";
const COLUMN_ACTION: &str = "action";
const COLUMN_FACINGS: [&str; STRIDE_FACING_COUNT] = ["south", "south-west", "west"];
const COLUMN_STRIDE: &str = "stride";
const COLUMN_HASH: &str = "hash";
/// Columns the measurement report adds, read and ignored.
const COLUMN_SPREAD: &str = "spread";
const COLUMN_VERDICT: &str = "verdict";
const NONE_FIELD: &str = "-";
const COMMENT_PREFIX: char = '#';
const HASH_HEX_DIGITS: usize = 16;

/// The text table header line, without the extra columns of a report.
pub fn stride_table_header() -> String {
    [COLUMN_SOURCE, COLUMN_BODY, COLUMN_ACTION].into_iter().chain(COLUMN_FACINGS)
        .chain([COLUMN_STRIDE, COLUMN_HASH]).collect::<Vec<_>>().join("\t")
}

/// A record's text table fields, in the header's order.
pub fn stride_table_fields(source: StrideSource, record: &MobileStrideRecord) -> String {
    let tiles = |tiles: Option<f32>| tiles.map_or(NONE_FIELD.to_string(), |tiles| format!("{tiles:.3}"));
    let mut line = format!("{}\t{}\t{}", source.token(), record.body_id, record.action_id);
    for facing in record.facing_tiles {
        write!(line, "\t{}", tiles(facing)).unwrap();
    }
    write!(line, "\t{}\t{:0width$x}", tiles(record.stride_tiles), record.clip_hash, width = HASH_HEX_DIGITS).unwrap();
    line
}

/// The text table of `records`: `comments` as `#` lines, the header, a line per record.
pub fn format_stride_table(source: StrideSource, records: &[MobileStrideRecord], comments: &[String]) -> String {
    let mut text = String::new();
    for comment in comments {
        writeln!(text, "{COMMENT_PREFIX} {comment}").unwrap();
    }
    writeln!(text, "{}", stride_table_header()).unwrap();
    for record in records {
        writeln!(text, "{}", stride_table_fields(source, record)).unwrap();
    }
    text
}

/// Reads a text table: `#` comment and blank lines aside, a header line naming the columns (source, body, action,
/// south, south-west, west, stride, and optionally hash, spread and verdict, in any order), then a line per clip.
/// Errors name the line.
pub fn parse_stride_table(name: &str, text: &str) -> eyre::Result<Vec<StrideTableRow>> {
    let mut lines = text.lines().enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.trim_start().starts_with(COMMENT_PREFIX));
    let (header_index, header) = lines.next().ok_or_else(|| eyre::eyre!("{name}: no header line"))?;
    let known = [COLUMN_SOURCE, COLUMN_BODY, COLUMN_ACTION, COLUMN_FACINGS[0], COLUMN_FACINGS[1], COLUMN_FACINGS[2],
        COLUMN_STRIDE, COLUMN_HASH, COLUMN_SPREAD, COLUMN_VERDICT];
    let mut columns = HashMap::new();
    for (index, column) in header.split('\t').map(str::trim).enumerate() {
        if !known.contains(&column) {
            eyre::bail!("{name}:{}: unknown column `{column}`; the columns are {}", header_index + 1, known.join(", "));
        }
        if columns.insert(column, index).is_some() {
            eyre::bail!("{name}:{}: column `{column}` twice", header_index + 1);
        }
    }
    let column = |column: &str| columns.get(column).copied()
        .ok_or_else(|| eyre::eyre!("{name}:{}: no `{column}` column", header_index + 1));
    let (source_at, body_at, action_at, stride_at) =
        (column(COLUMN_SOURCE)?, column(COLUMN_BODY)?, column(COLUMN_ACTION)?, column(COLUMN_STRIDE)?);
    let facings_at = [column(COLUMN_FACINGS[0])?, column(COLUMN_FACINGS[1])?, column(COLUMN_FACINGS[2])?];
    let hash_at = columns.get(COLUMN_HASH).copied();

    let mut rows = Vec::new();
    for (index, line) in lines {
        let fields: Vec<&str> = line.split('\t').map(str::trim).collect();
        let error = |what: &str| eyre::eyre!("{name}:{}: {what}: {line:?}", index + 1);
        if fields.len() != columns.len() {
            return Err(error(&format!("expected {} fields", columns.len())));
        }
        let tiles = |at: usize| -> eyre::Result<Option<f32>> {
            match fields[at] {
                NONE_FIELD => Ok(None),
                field => field.parse::<f32>().ok().filter(|tiles| tiles.is_finite()).map(Some)
                    .ok_or_else(|| error("a stride is not a number or `-`")),
            }
        };
        let stride_tiles = tiles(stride_at)?;
        if stride_tiles.is_some_and(|tiles| tiles <= 0.0) {
            return Err(error("the stride is not positive"));
        }
        rows.push(StrideTableRow {
            source: StrideSource::from_token(fields[source_at])
                .ok_or_else(|| error("the source is `cc` or `ec`"))?,
            body_id: fields[body_at].parse().map_err(|_| error("the body is not a number"))?,
            action_id: fields[action_at].parse().map_err(|_| error("the action is not a number"))?,
            facing_tiles: [tiles(facings_at[0])?, tiles(facings_at[1])?, tiles(facings_at[2])?],
            stride_tiles,
            clip_hash: match hash_at {
                Some(at) if fields[at] != NONE_FIELD => Some(u64::from_str_radix(fields[at], 16)
                    .map_err(|_| error("the hash is not 16 hex digits"))?),
                _ => None,
            },
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u16, height: u16, anchor: (i32, i32), fill: u8) -> MobileClipFrame {
        MobileClipFrame { width, height, anchor_x: anchor.0, anchor_y: anchor.1,
            rgba: vec![fill; usize::from(width) * usize::from(height) * 4] }
    }

    fn clip(frames: Vec<MobileClipFrame>) -> MobileClip {
        MobileClip { frames, mirrored: false, fps: 0.0, stored: 0 }
    }

    /// The hash of a fixed tiny clip: if this value changes, every stride in every package is dropped, so it must
    /// only change with a deliberate change of what a stride is keyed to.
    #[test]
    fn the_clip_hash_of_a_fixed_clip_is_stable() {
        const EXPECTED: u64 = 0xdc95_c304_b609_bb83;
        let tiny = clip(vec![frame(2, 1, (1, 1), 0x40), frame(1, 2, (0, 2), 0x80)]);
        let clips = [Some(tiny.clone()), None, Some(MobileClip { mirrored: true, ..tiny.clone() })];
        assert_eq!(stride_clip_hash(&clips), Some(EXPECTED), "{:#018x}", stride_clip_hash(&clips).unwrap());
        assert_eq!(stride_clip_hash(&[None, None, None]), None);

        // Anything a measurement sees changes it: a pixel, an anchor, the order, the mirroring, a facing's presence.
        let mut pixel = clips.clone();
        pixel[0].as_mut().unwrap().frames[1].rgba[0] ^= 1;
        let mut anchor = clips.clone();
        anchor[0].as_mut().unwrap().frames[0].anchor_y += 1;
        let mut order = clips.clone();
        order[0].as_mut().unwrap().frames.reverse();
        let mut mirrored = clips.clone();
        mirrored[2].as_mut().unwrap().mirrored = false;
        let absent = [Some(tiny.clone()), None, None];
        // The frame time does not.
        let mut fps = clips.clone();
        fps[0].as_mut().unwrap().fps = 12.0;
        for changed in [pixel, anchor, order, mirrored, absent] {
            assert_ne!(stride_clip_hash(&changed), stride_clip_hash(&clips));
        }
        assert_eq!(stride_clip_hash(&fps), stride_clip_hash(&clips));
    }

    fn record(body_id: u32, action_id: u16, clip_hash: u64, stride_tiles: Option<f32>) -> MobileStrideRecord {
        MobileStrideRecord { body_id, action_id, clip_hash, stride_tiles, facing_tiles: [stride_tiles, None, Some(1.5)] }
    }

    #[test]
    fn a_plan_keeps_unchanged_strides_and_measures_changed_and_new_clips() {
        let existing = [record(1, 0, 10, Some(2.0)), record(2, 0, 20, None), record(3, 0, 30, Some(1.0)),
            record(4, 0, 40, Some(1.0))];
        // 1 unchanged, 2 changed, 3 lost its frames, 4 is gone, 5 is new, 6 has no frames.
        let current = [(1, 0, Some(10)), (2, 0, Some(21)), (3, 0, None), (5, 0, Some(50)), (6, 0, None)];
        let plan = plan_strides(&current, &existing, false);
        assert_eq!(plan.unchanged, vec![existing[0]]);
        assert_eq!(plan.to_measure, vec![(2, 0, 21), (5, 0, 50)]);
        assert_eq!((plan.changed, plan.new, plan.without_frames), (1, 1, 2));
        assert_eq!(plan.removed, vec![existing[2], existing[3]]);

        let everything = plan_strides(&current, &existing, true);
        assert!(everything.unchanged.is_empty());
        assert_eq!(everything.to_measure, vec![(1, 0, 10), (2, 0, 21), (5, 0, 50)]);
        assert_eq!((everything.changed, everything.new), (1, 1));
    }

    #[test]
    fn the_stride_manifest_round_trips_sorted() {
        let records = [record(7, 19, u64::MAX, Some(2.25)), record(7, 0, 1, None), record(3, 2, 0xabc, Some(0.5))];
        let parsed = parse_stride_manifest(&encode_stride_manifest(&records)).unwrap();
        let mut sorted = records.to_vec();
        sort_stride_records(&mut sorted);
        assert_eq!(parsed, sorted);
        let mut bytes = encode_stride_manifest(&records);
        bytes[4] = 2;
        assert!(parse_stride_manifest(&bytes).is_err(), "another version");
    }

    #[test]
    fn the_text_table_round_trips_and_reads_a_report_without_hashes() {
        let records = [record(3, 2, 0xabc, Some(0.5)), record(7, 0, 1, None)];
        let text = format_stride_table(StrideSource::Enhanced, &records, &["made by a test".to_string()]);
        let rows = parse_stride_table("test", &text).unwrap();
        assert_eq!(rows.len(), 2);
        for (row, record) in rows.iter().zip(&records) {
            assert_eq!((row.source, row.body_id, row.action_id, row.clip_hash, row.stride_tiles, row.facing_tiles),
                (StrideSource::Enhanced, record.body_id, record.action_id, Some(record.clip_hash), record.stride_tiles,
                record.facing_tiles));
        }

        let report = "source\tbody\taction\tsouth\tsouth-west\twest\tspread\tstride\tverdict\n\
            cc\t9\t0\t2.790\t2.800\t-\t0.01\t2.795\tok\n";
        let rows = parse_stride_table("report", report).unwrap();
        assert_eq!((rows[0].source, rows[0].body_id, rows[0].clip_hash, rows[0].stride_tiles),
            (StrideSource::Classic, 9, None, Some(2.795)));
        assert_eq!(rows[0].facing_tiles, [Some(2.79), Some(2.8), None]);

        for bad in ["source\tbody\n", "source\tbody\taction\tsouth\tsouth-west\twest\tstride\tcolour\n",
            "source\tbody\taction\tsouth\tsouth-west\twest\tstride\nkr\t1\t0\t-\t-\t-\t1.0\n",
            "source\tbody\taction\tsouth\tsouth-west\twest\tstride\ncc\t1\t0\t-\t-\t-\t-1.0\n",
            "source\tbody\taction\tsouth\tsouth-west\twest\tstride\ncc\t1\t0\t-\t-\t-\n"] {
            let error = parse_stride_table("bad", bad).unwrap_err().to_string();
            assert!(error.starts_with("bad:"), "{error}");
        }
    }
}
