//! Whole-client check of `AnimationSequence.uop` against the frame packages (`AnimationFrame1..6.uop`): frame counts
//! against the frames stored, broken aliases, plain actions without a frame file, frame bodies without a sequence of
//! their own, and riders whose mounted cycles differ from a mount's.
//!
//! Which frames a sequence plays comes from the `tileart.uop` creature placeholders when the folder has them
//! (sequence = the placeholder's body type, frames = its appearance, body `FALLBACK_FRAMES_BODY` when that has no
//! frames: `crate::enhanced::mobile_body`), else from the frame body numbered as the sequence. Each finding says how
//! its frame body pairs with the sequence ([`Pairing`]): a body drawn through another body's sequence (sequence 400
//! drawn with frame body 800, say) only plays the actions it has frames for, so its counts read apart.
//!
//! The data names no mounts (the mount item to body mapping is server side), and every walking creature has
//! [`AnimationSequence::mount_move_action`]s. Riders are the person sequences with a mounted group: those of the
//! placeholders with a race (human, elf, gargoyle), or [`RIDER_REFERENCE_BODY`] without placeholders. Mounts are
//! the [`MOUNT_REFERENCE_BODIES`] and every other sequence with a mounted group (its ridden actions, as the ostard's
//! 29 / 30 / 31).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{eyre, Result, WrapErr};
use rayon::prelude::*;

use super::summary::{movement_name, ride_timings, AliasState, RideTiming, StoredFrames};
use super::{AnimationSequence, GROUP_MOUNTED};
use crate::classic::animationframe_cc::AnimationFrameCc;
use crate::enhanced::animationframe::{AnimationFrame, MAX_ACTION_ID_FALLBACK, MAX_BODY_ID};
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::mobile_body::{MobileBodyMap, FALLBACK_FRAMES_BODY};
use crate::enhanced::tileart::TILEART_UOP_NAME;
use crate::uop_container::hash::{hash_file_name_simd_batch_strs_into, hash_file_name_single};
use crate::uop_container::package::{LoadMode, UopPackage};
use crate::utils::path::find_file_ignoring_case;

/// The sequence package of a client folder.
pub const SEQUENCE_UOP_NAME: &str = "AnimationSequence.uop";
/// The frame packages of a client folder, in the order the clients search them.
pub const FRAME_UOP_NAMES: [&str; 6] = [
    "AnimationFrame1.uop",
    "AnimationFrame2.uop",
    "AnimationFrame3.uop",
    "AnimationFrame4.uop",
    "AnimationFrame5.uop",
    "AnimationFrame6.uop",
];
/// Sequence entry ids searched (body types); the shipped ids stay far below.
pub const MAX_SEQUENCE_ID: u32 = 100_000;
/// The rider of a folder without creature placeholders: the human male body.
pub const RIDER_REFERENCE_BODY: u32 = 400;
/// Mounts compared even without a mounted group of their own: horses 200 and 204, ostard 210, llama 220.
pub const MOUNT_REFERENCE_BODIES: [u32; 4] = [200, 204, 210, 220];

/// How a frame body pairs with the sequence it is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pairing {
    /// The frame body numbered as the sequence.
    Own,
    /// Another frame body a placeholder draws with the sequence.
    Placeholder,
    /// `FALLBACK_FRAMES_BODY`, drawn when a placeholder's frame body has no frames.
    Fallback,
}

impl Pairing {
    pub const ALL: [Self; 3] = [Self::Own, Self::Placeholder, Self::Fallback];

    pub fn of(sequence: u32, frames_body: u32) -> Self {
        if sequence == frames_body {
            Self::Own
        } else if frames_body == FALLBACK_FRAMES_BODY {
            Self::Fallback
        } else {
            Self::Placeholder
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Own => "own",
            Self::Placeholder => "placeholder",
            Self::Fallback => "fallback",
        }
    }
}

/// How a client names its frame entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameNaming {
    /// `build/animationframe/%06d/%02d.bin` (Enhanced Client, Kingdom Reborn).
    Enhanced,
    /// `build/animationlegacyframe/%06d/%02d.bin` (Classic Client).
    Classic,
}

impl FrameNaming {
    pub fn path(self, body: u32, action: u16) -> String {
        match self {
            Self::Enhanced => AnimationFrame::animationframe_path(body, action),
            Self::Classic => AnimationFrameCc::animationframe_path(body, action),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Enhanced => "animationframe",
            Self::Classic => "animationlegacyframe",
        }
    }
}

/// The frames a client stores for one (frame body, action): read, or why not.
pub type FrameRead = std::result::Result<StoredFrames, String>;

/// The sequences and frame entries of a client folder.
pub struct ClientSequences {
    pub dir: PathBuf,
    pub dialect: ClientDialect,
    pub sequences: BTreeMap<u32, AnimationSequence>,
    /// Sequence entries that do not parse, with the error.
    pub unreadable_sequences: Vec<(u32, String)>,
    /// How the frame entries are named (the naming most entries use).
    pub naming: FrameNaming,
    /// Every frame entry of the frame packages by (frame body, action), the first package holding it.
    pub frames: BTreeMap<(u32, u16), FrameRead>,
    /// Entries of the frame packages with another name (other naming, out of range, or unknown).
    pub other_frame_entries: usize,
    /// The creature placeholders of `tileart.uop`, when the folder has one that reads.
    pub mobile_bodies: Option<MobileBodyMap>,
}

impl ClientSequences {
    /// Reads `AnimationSequence.uop`, every frame entry's frame table and `tileart.uop` of `dir`; the dialect is the
    /// folder's ([`ClientDialect::of_client_dir`]). Fails without `AnimationSequence.uop`.
    pub fn load(dir: &Path) -> Result<Self> {
        let dialect = ClientDialect::of_client_dir(dir);
        let sequence_path = find_file_ignoring_case(dir, SEQUENCE_UOP_NAME)
            .ok_or_else(|| eyre!("{} has no {SEQUENCE_UOP_NAME}", dir.display()))?;
        let package = UopPackage::load(&sequence_path).wrap_err_with(|| format!("failed to load {}", sequence_path.display()))?;
        let (sequences, unreadable_sequences) = read_sequences(&package, dialect)?;

        let packages: Vec<UopPackage> = FRAME_UOP_NAMES
            .iter()
            .filter_map(|name| find_file_ignoring_case(dir, name))
            .map(|path| {
                UopPackage::load_with_mode(&path, LoadMode::Lazy).wrap_err_with(|| format!("failed to load {}", path.display()))
            })
            .collect::<Result<_>>()?;
        let names = frame_names();
        let mut located: BTreeMap<(u32, u16), (usize, u64)> = BTreeMap::new();
        let mut naming_counts = [0usize; 2];
        let mut other_frame_entries = 0;
        for (package_index, frame_package) in packages.iter().enumerate() {
            for (&hash, file) in frame_package.files_by_hash() {
                if !file.has_size() {
                    continue;
                }
                match names.get(&hash) {
                    Some(&(naming, body, action)) => {
                        naming_counts[usize::from(naming == FrameNaming::Classic)] += 1;
                        located.entry((body, action)).or_insert((package_index, hash));
                    }
                    None => other_frame_entries += 1,
                }
            }
        }
        let naming = if naming_counts[1] > naming_counts[0] { FrameNaming::Classic } else { FrameNaming::Enhanced };
        let frames = located
            .into_par_iter()
            .map(|(key, (package_index, hash))| {
                let read = packages[package_index]
                    .unpack_file_by_hash(hash)
                    .map_err(|error| error.to_string())
                    .and_then(|payload| payload.ok_or_else(|| "no payload".to_string()))
                    .and_then(|payload| StoredFrames::read(&payload).map_err(|error| format!("{error:#}")));
                (key, read)
            })
            .collect();

        // tileart.uop is optional: without it the sequences pair with the frame body of their own number.
        let mobile_bodies = find_file_ignoring_case(dir, TILEART_UOP_NAME).and_then(|path| match MobileBodyMap::load(&path) {
            Ok(map) if !map.definitions.is_empty() => Some(map),
            Ok(_) => None,
            Err(error) => {
                log::warn!("{}: {error:#}", path.display());
                None
            }
        });
        Ok(Self { dir: dir.to_path_buf(), dialect, sequences, unreadable_sequences, naming, frames, other_frame_entries, mobile_bodies })
    }

    /// Whether frame body `body` has any frame entry.
    pub fn has_frames(&self, body: u32) -> bool {
        self.frames.range((body, 0)..=(body, u16::MAX)).next().is_some()
    }

    /// The (sequence, frame body) pairs the client draws, each with the server bodies drawn so: from the
    /// placeholders when there are any, plus every sequence no placeholder uses with the frame body of its number.
    /// A frame body without any frame entry is not drawn from these packages (a Classic MUL body) and is left out.
    pub fn sequence_frame_pairs(&self) -> BTreeMap<(u32, u32), Vec<u32>> {
        let mut pairs: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
        if let Some(bodies) = &self.mobile_bodies {
            for definition in bodies.definitions.values() {
                let Some(frames) = definition.resolve_frames_body(|body| self.has_frames(body)) else {
                    continue;
                };
                if self.sequences.contains_key(&definition.sequence_id) {
                    pairs.entry((definition.sequence_id, frames.body)).or_default().push(definition.body);
                }
            }
        }
        let used: BTreeSet<u32> = pairs.keys().map(|&(sequence, _)| sequence).collect();
        for &sequence in self.sequences.keys().filter(|&&sequence| !used.contains(&sequence) && self.has_frames(sequence)) {
            pairs.entry((sequence, sequence)).or_default();
        }
        pairs
    }
}

/// Every parsable sequence entry, and the ids of those that do not parse.
fn read_sequences(package: &UopPackage, dialect: ClientDialect) -> Result<(BTreeMap<u32, AnimationSequence>, Vec<(u32, String)>)> {
    let mut sequences = BTreeMap::new();
    let mut unreadable = Vec::new();
    for id in 0..MAX_SEQUENCE_ID {
        let hash = hash_file_name_single(&AnimationSequence::entry_path(id));
        let Some(payload) = package.unpack_file_by_hash(hash).wrap_err_with(|| format!("failed to unpack sequence {id}"))? else {
            continue;
        };
        match AnimationSequence::parse_dialect(&payload, dialect) {
            Ok(sequence) => {
                sequences.insert(id, sequence);
            }
            Err(error) => unreadable.push((id, format!("{error:#}"))),
        }
    }
    Ok((sequences, unreadable))
}

/// Name hash of every frame entry name of both namings, bodies below `MAX_BODY_ID` and actions below
/// `MAX_ACTION_ID_FALLBACK`.
fn frame_names() -> BTreeMap<u64, (FrameNaming, u32, u16)> {
    let keys: Vec<(FrameNaming, u32, u16)> = [FrameNaming::Enhanced, FrameNaming::Classic]
        .into_iter()
        .flat_map(|naming| (0..MAX_BODY_ID).flat_map(move |body| (0..MAX_ACTION_ID_FALLBACK).map(move |action| (naming, body, action))))
        .collect();
    let paths: Vec<String> = keys.iter().map(|&(naming, body, action)| naming.path(body, action)).collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let mut hashes = vec![0; refs.len()];
    hash_file_name_simd_batch_strs_into(&refs, &mut hashes);
    hashes.into_iter().zip(keys).collect()
}

/// A plain action whose frame entry does not hold its frame count in every direction.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameCountMismatch {
    pub sequence: u32,
    pub frames_body: u32,
    pub pairing: Pairing,
    pub action: u32,
    pub frame_count: u32,
    pub stored: StoredFrames,
}

/// An alias the client resolves to the lowest plain action instead of its target.
#[derive(Debug, Clone, PartialEq)]
pub struct BrokenAlias {
    pub sequence: u32,
    pub action: u32,
    pub state: AliasState,
    pub resolved: Option<u32>,
}

/// A plain action with no frame entry for a frame body it is drawn with.
#[derive(Debug, Clone, PartialEq)]
pub struct MissingFrames {
    pub sequence: u32,
    pub frames_body: u32,
    pub pairing: Pairing,
    pub action: u32,
    pub frame_count: u32,
}

/// A frame body with frame entries and no sequence entry of the same number.
#[derive(Debug, Clone, PartialEq)]
pub struct FramesWithoutSequence {
    pub frames_body: u32,
    pub actions: Vec<u16>,
    /// The sequences placeholders draw it with.
    pub drawn_with: Vec<u32>,
}

/// A rider movement whose cycle differs from a mount's.
#[derive(Debug, Clone, PartialEq)]
pub struct RideMismatch {
    pub rider: u32,
    pub mount: u32,
    pub timing: RideTiming,
}

/// The findings of a client scan.
pub struct ScanReport {
    pub dir: PathBuf,
    pub dialect: ClientDialect,
    pub naming: FrameNaming,
    pub sequences: usize,
    pub frame_entries: usize,
    pub other_frame_entries: usize,
    pub placeholders: Option<usize>,
    pub unreadable_sequences: Vec<(u32, String)>,
    pub unreadable_frames: Vec<((u32, u16), String)>,
    pub frame_count_mismatches: Vec<FrameCountMismatch>,
    pub broken_aliases: Vec<BrokenAlias>,
    pub missing_frames: Vec<MissingFrames>,
    pub frames_without_sequence: Vec<FramesWithoutSequence>,
    /// Rider sequences (see the module documentation).
    pub riders: Vec<u32>,
    /// Mounts as (body, sequence) (see the module documentation).
    pub mounts: Vec<(u32, u32)>,
    pub ride_mismatches: Vec<RideMismatch>,
}

impl ScanReport {
    pub fn new(client: &ClientSequences) -> Self {
        let pairs = client.sequence_frame_pairs();
        let mut frame_count_mismatches = Vec::new();
        let mut missing_frames = Vec::new();
        for &(sequence_id, frames_body) in pairs.keys() {
            let pairing = Pairing::of(sequence_id, frames_body);
            let sequence = &client.sequences[&sequence_id];
            for (&action, record) in sequence.actions.iter().filter(|(_, record)| record.alias.is_none()) {
                let stored = u16::try_from(action).ok().and_then(|action| client.frames.get(&(frames_body, action)));
                match stored {
                    Some(Ok(stored)) if !stored.matches(record.frame_count) => frame_count_mismatches.push(FrameCountMismatch {
                        sequence: sequence_id,
                        frames_body,
                        pairing,
                        action,
                        frame_count: record.frame_count,
                        stored: *stored,
                    }),
                    Some(_) => {}
                    None => missing_frames.push(MissingFrames { sequence: sequence_id, frames_body, pairing, action, frame_count: record.frame_count }),
                }
            }
        }
        let broken_aliases = client
            .sequences
            .iter()
            .flat_map(|(&sequence_id, sequence)| {
                sequence.action_summaries().into_iter().filter(|summary| summary.alias.is_broken()).map(move |summary| BrokenAlias {
                    sequence: sequence_id,
                    action: summary.action,
                    state: summary.alias,
                    resolved: summary.resolved,
                })
            })
            .collect();

        let mut bodies_with_frames: BTreeMap<u32, Vec<u16>> = BTreeMap::new();
        for &(body, action) in client.frames.keys() {
            bodies_with_frames.entry(body).or_default().push(action);
        }
        let frames_without_sequence = bodies_with_frames
            .into_iter()
            .filter(|(body, _)| !client.sequences.contains_key(body))
            .map(|(frames_body, actions)| FramesWithoutSequence {
                frames_body,
                actions,
                drawn_with: pairs.keys().filter(|&&(_, frames)| frames == frames_body).map(|&(sequence, _)| sequence).collect(),
            })
            .collect();

        let (riders, mounts) = riders_and_mounts(client);
        let mut ride_mismatches = Vec::new();
        for &rider in &riders {
            for &(mount, mount_sequence) in &mounts {
                let timings = ride_timings(&client.sequences[&rider], &client.sequences[&mount_sequence]);
                ride_mismatches.extend(timings.into_iter().filter(RideTiming::cycles_differ).map(|timing| RideMismatch { rider, mount, timing }));
            }
        }

        let mut unreadable_frames: Vec<_> =
            client.frames.iter().filter_map(|(&key, read)| read.as_ref().err().map(|error| (key, error.clone()))).collect();
        unreadable_frames.sort_by_key(|(key, _)| *key);
        Self {
            dir: client.dir.clone(),
            dialect: client.dialect,
            naming: client.naming,
            sequences: client.sequences.len(),
            frame_entries: client.frames.len(),
            other_frame_entries: client.other_frame_entries,
            placeholders: client.mobile_bodies.as_ref().map(|bodies| bodies.definitions.len()),
            unreadable_sequences: client.unreadable_sequences.clone(),
            unreadable_frames,
            frame_count_mismatches,
            broken_aliases,
            missing_frames,
            frames_without_sequence,
            riders,
            mounts,
            ride_mismatches,
        }
    }

    /// One line per count; frame count mismatches and plain actions without frames per [`Pairing`].
    pub fn summary(&self) -> Vec<(String, usize)> {
        let mut lines = vec![
            ("sequences".to_string(), self.sequences),
            ("frame entries".to_string(), self.frame_entries),
            ("unreadable sequences".to_string(), self.unreadable_sequences.len()),
            ("unreadable frame entries".to_string(), self.unreadable_frames.len()),
        ];
        for pairing in Pairing::ALL {
            let count = self.frame_count_mismatches.iter().filter(|found| found.pairing == pairing).count();
            lines.push((format!("frame count mismatches ({} frame body)", pairing.label()), count));
        }
        lines.push(("broken aliases".to_string(), self.broken_aliases.len()));
        for pairing in Pairing::ALL {
            let count = self.missing_frames.iter().filter(|found| found.pairing == pairing).count();
            lines.push((format!("plain actions without frames ({} frame body)", pairing.label()), count));
        }
        lines.extend([
            ("frame bodies without a sequence".to_string(), self.frames_without_sequence.len()),
            ("riders".to_string(), self.riders.len()),
            ("mounts".to_string(), self.mounts.len()),
            ("rider/mount cycle mismatches".to_string(), self.ride_mismatches.len()),
        ]);
        lines
    }

    /// Tab-separated rows: `kind`, `sequence`, `frames_body`, `pairing`, `action`, `detail`.
    pub fn to_tsv(&self) -> String {
        let mut out = String::from("kind\tsequence\tframes_body\tpairing\taction\tdetail\n");
        let mut row = |kind: &str, sequence: Option<u32>, frames: Option<u32>, action: Option<u32>, detail: String| {
            let cell = |value: Option<u32>| value.map_or(String::new(), |value| value.to_string());
            let pairing = match (sequence, frames) {
                (Some(sequence), Some(frames)) => Pairing::of(sequence, frames).label(),
                _ => "",
            };
            let _ = writeln!(out, "{kind}\t{}\t{}\t{pairing}\t{}\t{detail}", cell(sequence), cell(frames), cell(action));
        };
        for (id, error) in &self.unreadable_sequences {
            row("unreadable_sequence", Some(*id), None, None, error.clone());
        }
        for ((body, action), error) in &self.unreadable_frames {
            row("unreadable_frames", None, Some(*body), Some(u32::from(*action)), error.clone());
        }
        for found in &self.frame_count_mismatches {
            row("frame_count_mismatch", Some(found.sequence), Some(found.frames_body), Some(found.action), mismatch_detail(found));
        }
        for found in &self.broken_aliases {
            row("broken_alias", Some(found.sequence), None, Some(found.action), alias_detail(found));
        }
        for found in &self.missing_frames {
            row(
                "plain_without_frames",
                Some(found.sequence),
                Some(found.frames_body),
                Some(found.action),
                format!("{} frames per direction", found.frame_count),
            );
        }
        for found in &self.frames_without_sequence {
            row("frames_without_sequence", None, Some(found.frames_body), None, without_sequence_detail(found));
        }
        for found in &self.ride_mismatches {
            row("ride_cycle_mismatch", Some(found.rider), None, None, format!("mount {}: {}", found.mount, ride_detail(found)));
        }
        out
    }

    /// A markdown document: the counts, then a table per finding.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# AnimationSequence scan of {}\n", self.dir.display());
        let _ = writeln!(
            out,
            "{} layout, {} frame entries; {}.\n",
            self.dialect.label(),
            self.naming.label(),
            match self.placeholders {
                Some(count) => format!("{count} tileart creature placeholders pair sequences with frame bodies"),
                None => "no tileart creature placeholders: each sequence pairs with the frame body of its number".to_string(),
            }
        );
        let _ = writeln!(out, "| Count | |\n|---:|---|");
        for (label, count) in self.summary() {
            let _ = writeln!(out, "| {count} | {label} |");
        }
        if self.other_frame_entries > 0 {
            let _ = writeln!(out, "| {} | frame package entries with another name |", self.other_frame_entries);
        }
        let _ = writeln!(out, "\nRiders (person sequences with a mounted group): {}.", join(&self.riders));
        let mounts: Vec<String> = self.mounts.iter().map(|(body, sequence)| format!("{body} (sequence {sequence})")).collect();
        let _ = writeln!(out, "Mounts compared: {}.", if mounts.is_empty() { "none".to_string() } else { mounts.join(", ") });

        section(&mut out, "Unreadable sequences", &["Sequence", "Error"], self.unreadable_sequences.iter().map(|(id, error)| vec![id.to_string(), error.clone()]));
        section(
            &mut out,
            "Unreadable frame entries",
            &["Frames body", "Action", "Error"],
            self.unreadable_frames.iter().map(|((body, action), error)| vec![body.to_string(), action.to_string(), error.clone()]),
        );
        section(
            &mut out,
            "Frame count mismatches",
            &["Sequence", "Frames body", "Pairing", "Action", "Detail"],
            self.frame_count_mismatches.iter().map(|found| {
                vec![
                    found.sequence.to_string(),
                    found.frames_body.to_string(),
                    found.pairing.label().to_string(),
                    found.action.to_string(),
                    mismatch_detail(found),
                ]
            }),
        );
        section(
            &mut out,
            "Broken aliases",
            &["Sequence", "Action", "Detail"],
            self.broken_aliases.iter().map(|found| vec![found.sequence.to_string(), found.action.to_string(), alias_detail(found)]),
        );
        section(
            &mut out,
            "Plain actions without frames",
            &["Sequence", "Frames body", "Pairing", "Action", "Frame count"],
            self.missing_frames.iter().map(|found| {
                vec![
                    found.sequence.to_string(),
                    found.frames_body.to_string(),
                    found.pairing.label().to_string(),
                    found.action.to_string(),
                    found.frame_count.to_string(),
                ]
            }),
        );
        section(
            &mut out,
            "Frame bodies without a sequence of their own",
            &["Frames body", "Detail"],
            self.frames_without_sequence.iter().map(|found| vec![found.frames_body.to_string(), without_sequence_detail(found)]),
        );
        section(
            &mut out,
            "Rider / mount cycle mismatches",
            &["Rider", "Mount", "Detail"],
            self.ride_mismatches.iter().map(|found| vec![found.rider.to_string(), found.mount.to_string(), ride_detail(found)]),
        );
        out
    }
}

/// Rider sequences and mounts as (body, sequence); see the module documentation.
pub fn riders_and_mounts(client: &ClientSequences) -> (Vec<u32>, Vec<(u32, u32)>) {
    let has_mounted_group =
        |sequence: &u32| client.sequences.get(sequence).is_some_and(|sequence| sequence.groups.contains_key(&GROUP_MOUNTED));
    let riders: BTreeSet<u32> = match &client.mobile_bodies {
        Some(bodies) => bodies
            .definitions
            .values()
            .filter(|definition| definition.race.is_some())
            .map(|definition| definition.sequence_id)
            .filter(has_mounted_group)
            .collect(),
        None => [RIDER_REFERENCE_BODY].into_iter().filter(has_mounted_group).collect(),
    };
    let mount_sequence = |body: u32| {
        client.mobile_bodies.as_ref().and_then(|bodies| bodies.definition(body)).map_or(body, |definition| definition.sequence_id)
    };
    let mut mounts: BTreeMap<u32, u32> = MOUNT_REFERENCE_BODIES
        .iter()
        .map(|&body| (body, mount_sequence(body)))
        .filter(|(_, sequence)| client.sequences.contains_key(sequence))
        .collect();
    for &sequence in client.sequences.keys().filter(|sequence| has_mounted_group(sequence) && !riders.contains(sequence)) {
        if !mounts.values().any(|&mounted| mounted == sequence) {
            mounts.insert(sequence, sequence);
        }
    }
    (riders.into_iter().collect(), mounts.into_iter().collect())
}

fn join(values: &[u32]) -> String {
    if values.is_empty() {
        return "none".to_string();
    }
    values.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
}

fn section(out: &mut String, title: &str, header: &[&str], rows: impl Iterator<Item = Vec<String>>) {
    let rows: Vec<Vec<String>> = rows.collect();
    let _ = writeln!(out, "\n## {title} ({})\n", rows.len());
    if rows.is_empty() {
        let _ = writeln!(out, "None.");
        return;
    }
    let _ = writeln!(out, "| {} |", header.join(" | "));
    let _ = writeln!(out, "|{}", "---|".repeat(header.len()));
    for row in rows {
        let _ = writeln!(out, "| {} |", row.iter().map(|cell| cell.replace('|', "\\|")).collect::<Vec<_>>().join(" | "));
    }
}

fn mismatch_detail(found: &FrameCountMismatch) -> String {
    format!(
        "frame count {}, stored {} per direction ({} frames, highest number {})",
        found.frame_count,
        found.stored.per_direction_text(),
        found.stored.count,
        found.stored.highest_number
    )
}

fn alias_detail(found: &BrokenAlias) -> String {
    let resolved = found.resolved.map_or("nothing".to_string(), |action| format!("action {action}"));
    match found.state {
        AliasState::AliasOfAlias(target) => format!("alias of alias {target}: plays {resolved}"),
        AliasState::MissingTarget(target) => format!("alias of missing action {target}: plays {resolved}"),
        AliasState::Plain | AliasState::Alias(_) => String::new(),
    }
}

fn without_sequence_detail(found: &FramesWithoutSequence) -> String {
    let actions: Vec<String> = found.actions.iter().map(u16::to_string).collect();
    let drawn = if found.drawn_with.is_empty() {
        "no placeholder draws it".to_string()
    } else {
        format!("drawn with sequence {}", join(&found.drawn_with))
    };
    let fallback = if found.frames_body == FALLBACK_FRAMES_BODY { " (the fallback frame body)" } else { "" };
    format!("{} actions ({}); {drawn}{fallback}", found.actions.len(), actions.join(" "))
}

fn ride_detail(found: &RideMismatch) -> String {
    let side = |summary: Option<super::summary::ActionSummary>| match summary {
        Some(summary) => format!(
            "action {} ({} frames at {} fps, {:.3} s)",
            summary.action,
            summary.shown_frames(),
            summary.fps,
            summary.cycle_seconds().unwrap_or_default()
        ),
        None => "none".to_string(),
    };
    format!("{}: rider {}, mount {}", movement_name(found.timing.movement), side(found.timing.rider), side(found.timing.mount))
}
