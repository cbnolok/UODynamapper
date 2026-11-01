use std::path::{Path, PathBuf};

use clap::Subcommand;
use color_eyre::eyre::{self, WrapErr};
use uocf::animation_sequence::STORED_DIRECTION_COUNT;
use uocf::classic::anim::classic_animation_source_index_from_identity;
use uocf::classic::anim_frames::folder::{
    export_mul_folder, export_uop_folder, import_folder, FrameSource, ImportedClassicFrames, FIRST_MUL_FILE,
    SIDECAR_FILE_NAME,
};
use uocf::classic::anim_frames::{ColourReport, OPAQUE_BLACK, USABLE_PALETTE_COLOURS};
use uocf::classic::anim_frames::mul::{read_mul_entry, write_mul_entries};
use uocf::classic::anim_frames::uop::{read_uop_entry, write_uop_entry, ClassicUopEntry};
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::enhanced::animationframe::package::{write_sequence_frame_count, EntryChange};
use uocf::uop_container::package::UopPackage;

use crate::animframe::{require_absolute, SequenceArgs};

/// The extension of a Classic Client animation package; anything else is an `anim*.idx`.
const UOP_EXTENSION: &str = "uop";
/// The extension of the data file of an `anim*.idx`.
const MUL_EXTENSION: &str = "mul";
/// The file stem of `anim.idx`; `animN.idx` is file N.
const MUL_STEM: &str = "anim";

#[derive(Subcommand, Debug)]
pub enum ClassicanimCmd {
    /// Write the Classic frames of a body and action as a folder of RGBA PNGs plus `classicanimation.kdl`
    /// (anchors, sizes, frame numbers, palettes): from an AnimationFrame*.uop `animationlegacyframe` entry, or
    /// from the five direction entries of an anim*.idx / anim*.mul pair.
    Export {
        /// Source AnimationFrame*.uop, or anim.idx / animN.idx (its .mul next to it, or `--mul`).
        source: PathBuf,
        /// The body as the source numbers it (MUL: the body of that file, after any mobtypes or bodyconv mapping).
        #[arg(long)]
        body: u32,
        #[arg(long)]
        action: u16,
        /// Output folder, created when missing; existing files are never overwritten.
        #[arg(long)]
        output: PathBuf,
        /// The data file of an anim*.idx source, when it is not next to it.
        #[arg(long)]
        mul: Option<PathBuf>,
        /// Leave the palettes out of the sidecar: the import then builds them from the images.
        #[arg(long)]
        no_palette: bool,
        #[command(flatten)]
        sequence: SequenceArgs,
    },
    /// Encode a folder written by `export` (edited or not) and write a copy of the source with those entries
    /// replaced or added; everything else keeps its bytes. UOP: when the frames per direction change, the
    /// AnimationSequence entry must be updated too (`--sequence`, `--sequence-output`). MUL: changed entries are
    /// appended to the data file and the index points there.
    Import {
        /// Source AnimationFrame*.uop, or the anim*.idx of the folder's file (its .mul next to it, or `--mul`).
        source: PathBuf,
        /// Folder with classicanimation.kdl.
        input: PathBuf,
        /// Output AnimationFrame*.uop or anim*.idx: an absolute path, never the source.
        #[arg(long)]
        output: PathBuf,
        /// The data file of an anim*.idx source, when it is not next to it.
        #[arg(long)]
        mul: Option<PathBuf>,
        /// Output anim*.mul of an anim*.idx source: an absolute path, never the source.
        #[arg(long)]
        mul_output: Option<PathBuf>,
        /// Output AnimationSequence.uop when its frame count changes: an absolute path, never the source.
        #[arg(long)]
        sequence_output: Option<PathBuf>,
        #[command(flatten)]
        sequence: SequenceArgs,
    },
}

fn is_uop(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(UOP_EXTENSION))
}

/// The file number of an `anim*.idx` path: 1 for `anim.idx`, N for `animN.idx`.
fn mul_file_number(idx: &Path) -> eyre::Result<u8> {
    let stem = idx.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_ascii_lowercase();
    let number = stem
        .strip_prefix(MUL_STEM)
        .ok_or_else(|| eyre::eyre!("{} is neither a .uop nor an anim*.idx", idx.display()))?;
    if number.is_empty() {
        return Ok(FIRST_MUL_FILE);
    }
    let file: u8 = number.parse().wrap_err_with(|| format!("{} is not anim.idx or animN.idx", idx.display()))?;
    eyre::ensure!(file > FIRST_MUL_FILE, "{} is not anim.idx or animN.idx", idx.display());
    Ok(file)
}

/// The data file of an `anim*.idx`: `mul`, else the file next to it with the .mul extension (either case).
fn mul_path(idx: &Path, mul: Option<&Path>) -> eyre::Result<PathBuf> {
    if let Some(mul) = mul {
        return Ok(mul.to_path_buf());
    }
    [MUL_EXTENSION.to_string(), MUL_EXTENSION.to_ascii_uppercase()]
        .into_iter()
        .map(|extension| idx.with_extension(extension))
        .find(|path| path.exists())
        .ok_or_else(|| eyre::eyre!("no .mul next to {}: pass --mul", idx.display()))
}

/// The idx entry of `body` / `action` / `direction` in `anim<file>.idx`.
fn mul_index(file: u8, body: u32, action: u16, direction: u8) -> eyre::Result<u32> {
    let body16 = u16::try_from(body).wrap_err_with(|| format!("body {body} is past the MUL bodies"))?;
    classic_animation_source_index_from_identity(file - FIRST_MUL_FILE, body16, action, direction)
        .ok_or_else(|| eyre::eyre!("body {body} has no action {action} in the layout of anim file {file}"))
}

fn directions() -> impl Iterator<Item = u8> {
    0..STORED_DIRECTION_COUNT as u8
}

fn change_label(change: EntryChange) -> &'static str {
    match change {
        EntryChange::Unchanged => "unchanged",
        EntryChange::Replaced => "replaced",
        EntryChange::Added => "added",
    }
}

fn load_uop(path: &Path) -> eyre::Result<UopPackage> {
    UopPackage::load(path).wrap_err_with(|| format!("failed to load {}", path.display()))
}

pub fn run(cmd: ClassicanimCmd) -> eyre::Result<()> {
    match cmd {
        ClassicanimCmd::Export { source, body, action, output, mul, no_palette, sequence } => {
            if is_uop(&source) {
                let payload = read_uop_entry(&load_uop(&source)?, body, action)?.ok_or_else(|| {
                    eyre::eyre!("{} has no {}", source.display(), AnimationFrameCc::animationframe_path(body, action))
                })?;
                let frames_per_direction = sequence.frame_count(body, action)?;
                let sidecar = export_uop_folder(&payload, body, action, frames_per_direction, !no_palette, &output)?;
                println!(
                    "Wrote {} frames of body {body} action {action}{} to {}.",
                    sidecar.groups.iter().map(|group| group.frames.len()).sum::<usize>(),
                    sidecar.frames_per_direction.map_or(String::new(), |count| format!(" ({count} per direction)")),
                    output.display()
                );
            } else {
                eyre::ensure!(sequence.is_empty(), "--sequence options are for AnimationFrame*.uop sources");
                let file = mul_file_number(&source)?;
                let mul = mul_path(&source, mul.as_deref())?;
                let mut entries = Vec::new();
                for direction in directions() {
                    if let Some(payload) = read_mul_entry(&source, &mul, mul_index(file, body, action, direction)?)? {
                        entries.push((direction, payload));
                    }
                }
                eyre::ensure!(!entries.is_empty(), "{} has no body {body} action {action}", source.display());
                let sidecar = export_mul_folder(&entries, file, body, action, !no_palette, &output)?;
                println!(
                    "Wrote {} frames in {} directions of body {body} action {action} to {}.",
                    sidecar.groups.iter().map(|group| group.frames.len()).sum::<usize>(),
                    sidecar.groups.len(),
                    output.display()
                );
            }
        }
        ClassicanimCmd::Import { source, input, output, mul, mul_output, sequence_output, sequence } => {
            require_absolute(&output, "--output")?;
            let imported = import_folder(&input)?;
            match imported.sidecar.source {
                FrameSource::Uop => {
                    eyre::ensure!(is_uop(&source), "the folder holds an animationlegacyframe entry: the source is a .uop");
                    eyre::ensure!(mul.is_none() && mul_output.is_none(), "--mul and --mul-output are for anim*.idx sources");
                    import_uop(&source, &output, &imported, sequence_output.as_deref(), &sequence)?;
                }
                FrameSource::Mul { file } => {
                    eyre::ensure!(!is_uop(&source), "the folder holds anim*.mul frames: the source is an anim*.idx");
                    eyre::ensure!(
                        sequence.is_empty() && sequence_output.is_none(),
                        "--sequence options are for AnimationFrame*.uop sources"
                    );
                    eyre::ensure!(
                        mul_file_number(&source)? == file,
                        "the folder holds frames of anim file {file}, not of {}",
                        source.display()
                    );
                    let mul_output = mul_output.ok_or_else(|| eyre::eyre!("pass --mul-output for an anim*.idx source"))?;
                    require_absolute(&mul_output, "--mul-output")?;
                    let mul = mul_path(&source, mul.as_deref())?;
                    import_mul(&source, &mul, &output, &mul_output, &imported, file)?;
                }
            }
        }
    }
    Ok(())
}

fn import_uop(
    source: &Path,
    output: &Path,
    imported: &ImportedClassicFrames,
    sequence_output: Option<&Path>,
    sequence: &SequenceArgs,
) -> eyre::Result<()> {
    let (body, action) = (imported.sidecar.body, imported.sidecar.action);
    let base = read_uop_entry(&load_uop(source)?, body, action)?;
    let previous = base.as_deref().map(ClassicUopEntry::parse).transpose()?.map(|entry| entry.frames.len());
    let payload = imported.uop_payload(base.as_deref())?;
    let frames = ClassicUopEntry::parse(&payload)?.frames.len();
    let per_direction = imported.sidecar.frames_per_direction.or_else(|| {
        let count = u32::try_from(frames).ok()?;
        (count % STORED_DIRECTION_COUNT == 0).then_some(count / STORED_DIRECTION_COUNT)
    });

    let sequence_count = sequence.frame_count(body, action)?;
    let sequence_change = match (sequence_count, per_direction) {
        (Some(current), Some(new)) if current != new => Some((current, new)),
        (Some(_), None) => eyre::bail!(
            "{frames} frames are not a multiple of {STORED_DIRECTION_COUNT} directions: set frames-per-direction= in \
             {SIDECAR_FILE_NAME}"
        ),
        _ => None,
    };
    if sequence_count.is_none() && previous.is_some_and(|previous| previous != frames) {
        eyre::bail!(
            "the entry had {} frames and now has {frames}: pass --sequence and --sequence-output to update the \
             AnimationSequence frame count",
            previous.unwrap_or_default()
        );
    }
    if let Some((current, new)) = sequence_change {
        let sequence_path = sequence.path().expect("a sequence count comes from --sequence");
        let sequence_output = sequence_output.ok_or_else(|| {
            eyre::eyre!("the frames per direction change from {current} to {new}: pass --sequence-output")
        })?;
        require_absolute(sequence_output, "--sequence-output")?;
        let sequence_id = sequence.sequence_id(body);
        let dialect = sequence.dialect(sequence_path);
        write_sequence_frame_count(sequence_path, sequence_output, sequence_id, u32::from(action), new, dialect)?;
        println!(
            "Wrote {} ({}): sequence {sequence_id} action {action}: {current} -> {new} frames per direction.",
            sequence_output.display(),
            dialect.label()
        );
    }

    let change = write_uop_entry(source, output, body, action, &payload)?;
    let report = &imported.groups[0].report;
    println!(
        "Encoded {frames} frames of body {body} action {action}: {}; wrote {} (entry {}).",
        colour_summary(report),
        output.display(),
        change_label(change)
    );
    Ok(())
}

fn import_mul(
    idx: &Path,
    mul: &Path,
    idx_output: &Path,
    mul_output: &Path,
    imported: &ImportedClassicFrames,
    file: u8,
) -> eyre::Result<()> {
    let (body, action) = (imported.sidecar.body, imported.sidecar.action);
    let entries = imported
        .mul_payloads()?
        .into_iter()
        .map(|(direction, payload)| Ok((mul_index(file, body, action, direction)?, payload)))
        .collect::<eyre::Result<Vec<_>>>()?;
    let changes = write_mul_entries(idx, mul, idx_output, mul_output, &entries)?;
    for (group, change) in imported.groups.iter().zip(changes) {
        println!(
            "Body {body} action {action} direction {}: {} frames, {}; entry {}.",
            group.direction.unwrap_or_default(),
            group.frames.iter().filter(|(_, frame)| frame.is_some()).count(),
            colour_summary(&group.report),
            change_label(change)
        );
    }
    println!("Wrote {} and {}.", idx_output.display(), mul_output.display());
    Ok(())
}

fn colour_summary(report: &ColourReport) -> String {
    let mut text = format!("{} distinct colours", report.distinct_colours);
    if report.quantized {
        text.push_str(&format!(", quantized to {USABLE_PALETTE_COLOURS}"));
    }
    if report.approximated > 0 {
        text.push_str(&format!(", {} pixels took the nearest palette colour", report.approximated));
    }
    if report.black > 0 {
        text.push_str(&format!(", {} black pixels stored as {OPAQUE_BLACK:#06X}", report.black));
    }
    text
}
