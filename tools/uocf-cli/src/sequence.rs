use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use color_eyre::eyre::{self, WrapErr};
use uocf::animation_sequence::scan::{ClientSequences, ScanReport};
use uocf::animation_sequence::AnimationSequence;
use uocf::enhanced::animationframe::package::{read_sequence, write_sequence};
use uocf::uop_container::package::UopPackage;

use crate::animframe::{require_absolute, DialectArg};

/// Extension of the TSV written beside a scan's markdown report.
const TSV_EXTENSION: &str = "tsv";

#[derive(Subcommand, Debug)]
pub enum SequenceCmd {
    /// Print one entry: what each action plays (frames, fps, cycle, alias, skipped frames, effects, offsets) and
    /// the groups with their moves, transitions and typed actions.
    Dump {
        /// AnimationSequence.uop to read.
        uop: PathBuf,
        /// Sequence entry (body type).
        body: u32,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Sequence format: `auto` is Kingdom Reborn next to UOKR.exe, else Enhanced Client.
        #[arg(long, value_enum, default_value_t = DialectArg::Auto)]
        dialect: DialectArg,
    },
    /// Write a copy of an AnimationSequence.uop with one action of one entry edited (never the source).
    Set {
        /// Source AnimationSequence.uop (absolute path).
        input: PathBuf,
        /// New AnimationSequence.uop (absolute path, not the source).
        output: PathBuf,
        /// Sequence entry (body type).
        body: u32,
        /// Action id.
        action: u32,
        /// Frames per second (the cycle lasts shown frames / fps).
        #[arg(long)]
        fps: Option<f32>,
        /// Frames per direction of a plain action. The frame file's numbering depends on it: direction d holds
        /// frames (d - 3) * frame_count + 1 onwards.
        #[arg(long)]
        frame_count: Option<u32>,
        /// Action whose frames this one borrows, or `none` for a plain action.
        #[arg(long)]
        alias: Option<String>,
        /// Skipped frames (0-based, comma separated), or `none`.
        #[arg(long)]
        skip: Option<String>,
        /// Sequence format: `auto` is Kingdom Reborn next to UOKR.exe, else Enhanced Client.
        #[arg(long, value_enum, default_value_t = DialectArg::Auto)]
        dialect: DialectArg,
    },
    /// Check a client folder's AnimationSequence.uop against its AnimationFrame*.uop: frame counts against
    /// stored frames, broken aliases, plain actions without frames, frame bodies without a sequence, rider/mount
    /// cycle mismatches. Writes a markdown report and a TSV beside it (same name, .tsv).
    Scan {
        /// Client folder (absolute path).
        client: PathBuf,
        /// Markdown report (absolute path, outside the client folder).
        report: PathBuf,
    },
}

/// "none" or empty for `None`, else the parsed value.
fn parse_optional<T: std::str::FromStr>(text: &str, what: &str) -> eyre::Result<Option<T>> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    text.parse().map(Some).map_err(|_| eyre::eyre!("invalid {what}: {text}"))
}

/// Comma-separated frame indices; "none" or empty for none.
fn parse_frames(text: &str) -> eyre::Result<BTreeSet<u32>> {
    if parse_optional::<String>(text, "skipped frames")?.is_none() {
        return Ok(BTreeSet::new());
    }
    text.split(',').map(|frame| frame.trim().parse().map_err(|_| eyre::eyre!("invalid skipped frame: {frame}"))).collect()
}

fn load_sequence(uop: &Path, body: u32, dialect: DialectArg) -> eyre::Result<AnimationSequence> {
    let package = UopPackage::load(uop).wrap_err_with(|| format!("failed to load {}", uop.display()))?;
    read_sequence(&package, body, dialect.resolve(uop))?
        .ok_or_else(|| eyre::eyre!("{} has no {}", uop.display(), AnimationSequence::entry_path(body)))
}

pub fn run(cmd: SequenceCmd) -> eyre::Result<()> {
    match cmd {
        SequenceCmd::Dump { uop, body, json, dialect } => {
            let sequence = load_sequence(&uop, body, dialect)?;
            if json {
                println!("{:#}", sequence.dump_json());
            } else {
                print!("{}", sequence.dump_text());
            }
        }
        SequenceCmd::Set { input, output, body, action, fps, frame_count, alias, skip, dialect } => {
            require_absolute(&input, "the source AnimationSequence.uop")?;
            require_absolute(&output, "the output AnimationSequence.uop")?;
            let mut sequence = load_sequence(&input, body, dialect)?;
            let mut changes = Vec::new();
            if let Some(alias) = alias {
                let alias = parse_optional(&alias, "alias")?;
                let previous = sequence.set_alias(action, alias)?;
                changes.push(format!("alias {previous:?} -> {alias:?}"));
            }
            if let Some(frame_count) = frame_count {
                let previous = sequence.set_frame_count(action, frame_count)?;
                changes.push(format!("frame count {previous} -> {frame_count}"));
            }
            if let Some(fps) = fps {
                let previous = sequence.set_fps(action, fps)?;
                changes.push(format!("fps {previous} -> {fps}"));
            }
            if let Some(skip) = skip {
                let frames = parse_frames(&skip)?;
                let previous = sequence.set_skipped_frames(action, frames.clone())?;
                changes.push(format!("skipped frames {previous:?} -> {frames:?}"));
            }
            eyre::ensure!(!changes.is_empty(), "nothing to set: give --fps, --frame-count, --alias or --skip");
            write_sequence(&input, &output, body, &sequence, dialect.resolve(&input))?;
            println!("Wrote {} with sequence {body} action {action}: {}.", output.display(), changes.join(", "));
        }
        SequenceCmd::Scan { client, report } => {
            require_absolute(&client, "the client folder")?;
            require_absolute(&report, "the report")?;
            eyre::ensure!(!report.starts_with(&client), "the report must be outside the client folder: {}", report.display());
            let tsv = report.with_extension(TSV_EXTENSION);
            eyre::ensure!(tsv != report, "the report must not have the .{TSV_EXTENSION} extension");
            let scan = ScanReport::new(&ClientSequences::load(&client)?);
            std::fs::write(&report, scan.to_markdown()).wrap_err_with(|| format!("failed to write {}", report.display()))?;
            std::fs::write(&tsv, scan.to_tsv()).wrap_err_with(|| format!("failed to write {}", tsv.display()))?;
            for (label, count) in scan.summary() {
                println!("{count:>8}  {label}");
            }
            println!("Wrote {} and {}.", report.display(), tsv.display());
        }
    }
    Ok(())
}
