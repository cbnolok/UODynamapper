use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use uocf::animation_sequence::STORED_DIRECTION_COUNT;
use uocf::enhanced::animationframe::folder::{export_folder, import_folder, SIDECAR_FILE_NAME};
use uocf::enhanced::animationframe::package::{
    read_entry, sequence_frame_count, write_entry, write_sequence_frame_count, EntryChange,
};
use uocf::enhanced::animationframe::AnimationFrame;
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::uop_container::package::UopPackage;

#[derive(Subcommand, Debug)]
pub enum AnimframeCmd {
    /// Write one AnimationFrame entry (a body and action of AnimationFrame*.uop) as a folder of RGBA PNGs
    /// (`n<frame number>.png`, `n<frame number>_hue.png` hue masks) plus `animationframe.kdl` (anchors, sizes,
    /// frame numbers, palette).
    Export {
        /// Source AnimationFrame*.uop.
        uop: PathBuf,
        #[arg(long)]
        body: u32,
        #[arg(long)]
        action: u16,
        /// Output folder, created when missing; existing files are never overwritten.
        #[arg(long)]
        output: PathBuf,
        /// Leave the palette out of the sidecar: the import then builds one from the images.
        #[arg(long)]
        no_palette: bool,
        #[command(flatten)]
        sequence: SequenceArgs,
    },
    /// Encode a folder written by `export` (edited or not) and write a copy of the package with that entry
    /// replaced or added; other entries keep their bytes and headers. When the frames per direction change, the
    /// AnimationSequence entry must be updated too (`--sequence`, `--sequence-output`).
    Import {
        /// Source AnimationFrame*.uop (the one that holds the entry, or the one to add it to).
        uop: PathBuf,
        /// Folder with animationframe.kdl.
        input: PathBuf,
        /// Output AnimationFrame*.uop: an absolute path, never the source.
        #[arg(long)]
        output: PathBuf,
        /// Output AnimationSequence.uop when its frame count changes: an absolute path, never the source.
        #[arg(long)]
        sequence_output: Option<PathBuf>,
        #[command(flatten)]
        sequence: SequenceArgs,
    },
}

#[derive(clap::Args, Debug)]
pub struct SequenceArgs {
    /// AnimationSequence.uop that holds the action's frames per direction.
    #[arg(long)]
    sequence: Option<PathBuf>,
    /// Sequence entry (body type) of the frames; the frame body when omitted. A server body can use another
    /// sequence than its frame body (tileart.uop creature placeholders).
    #[arg(long)]
    sequence_id: Option<u32>,
    /// Sequence format: `auto` is Kingdom Reborn next to UOKR.exe, else Enhanced Client.
    #[arg(long, value_enum, default_value_t = DialectArg::Auto)]
    dialect: DialectArg,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum DialectArg {
    #[default]
    Auto,
    Enhanced,
    KingdomReborn,
}

impl DialectArg {
    fn resolve(self, uop: &Path) -> ClientDialect {
        match self {
            Self::Auto => ClientDialect::of_package_path(uop),
            Self::Enhanced => ClientDialect::Enhanced,
            Self::KingdomReborn => ClientDialect::KingdomReborn,
        }
    }
}

impl SequenceArgs {
    /// Whether no sequence option was given.
    pub(crate) fn is_empty(&self) -> bool {
        self.sequence.is_none() && self.sequence_id.is_none() && self.dialect == DialectArg::Auto
    }

    /// The AnimationSequence.uop, when given.
    pub(crate) fn path(&self) -> Option<&Path> {
        self.sequence.as_deref()
    }

    /// The sequence entry of the frames of `body`.
    pub(crate) fn sequence_id(&self, body: u32) -> u32 {
        self.sequence_id.unwrap_or(body)
    }

    /// The sequence format of the package at `path`.
    pub(crate) fn dialect(&self, path: &Path) -> ClientDialect {
        self.dialect.resolve(path)
    }

    /// The action's frames per direction in the sequence package, when one is given.
    pub(crate) fn frame_count(&self, body: u32, action: u16) -> eyre::Result<Option<u32>> {
        let Some(path) = &self.sequence else {
            return Ok(None);
        };
        let package = UopPackage::load(path).wrap_err_with(|| format!("failed to load {}", path.display()))?;
        let sequence_id = self.sequence_id.unwrap_or(body);
        let count = sequence_frame_count(&package, sequence_id, u32::from(action), self.dialect.resolve(path))?;
        eyre::ensure!(count.is_some(), "{} has no action {action} in sequence {sequence_id}", path.display());
        Ok(count)
    }
}

pub(crate) fn require_absolute(path: &Path, what: &str) -> eyre::Result<()> {
    eyre::ensure!(path.is_absolute(), "{what} must be an absolute path: {}", path.display());
    Ok(())
}

pub fn run(cmd: AnimframeCmd) -> eyre::Result<()> {
    match cmd {
        AnimframeCmd::Export { uop, body, action, output, no_palette, sequence } => {
            let package = UopPackage::load(&uop).wrap_err_with(|| format!("failed to load {}", uop.display()))?;
            let payload = read_entry(&package, body, action)?.ok_or_else(|| {
                eyre::eyre!("{} has no {}", uop.display(), AnimationFrame::animationframe_path(body, action))
            })?;
            let frames_per_direction = sequence.frame_count(body, action)?;
            let sidecar = export_folder(&payload, body, action, frames_per_direction, !no_palette, &output)?;
            println!(
                "Wrote {} frames of body {body} action {action} ({} colours{}) to {}.",
                sidecar.frames.len(),
                sidecar.palette.as_ref().map_or(0, Vec::len),
                sidecar.frames_per_direction.map_or(String::new(), |count| format!(", {count} per direction")),
                output.display()
            );
        }
        AnimframeCmd::Import { uop, input, output, sequence_output, sequence } => {
            require_absolute(&output, "--output")?;
            let imported = import_folder(&input)?;
            let (body, action) = (imported.sidecar.body, imported.sidecar.action);
            let package = UopPackage::load(&uop).wrap_err_with(|| format!("failed to load {}", uop.display()))?;
            let previous = read_entry(&package, body, action)?
                .map(|payload| AnimationFrame::load_metadata(&payload).map(|metadata| metadata.frames.len()))
                .transpose()?;
            drop(package);
            let frames = imported.sidecar.frames.len();
            let per_direction = imported.sidecar.frames_per_direction.or_else(|| {
                let count = u32::try_from(frames).ok()?;
                (count % STORED_DIRECTION_COUNT == 0).then_some(count / STORED_DIRECTION_COUNT)
            });

            let sequence_count = sequence.frame_count(body, action)?;
            let sequence_change = match (sequence_count, per_direction) {
                (Some(current), Some(new)) if current != new => Some((current, new)),
                (Some(_), None) => eyre::bail!(
                    "{frames} frames are not a multiple of {STORED_DIRECTION_COUNT} directions: set \
                     frames-per-direction= in {SIDECAR_FILE_NAME}"
                ),
                _ => None,
            };
            if sequence_count.is_none() && previous.is_some_and(|previous| previous != frames) {
                eyre::bail!(
                    "the entry had {} frames and now has {frames}: pass --sequence and --sequence-output to update \
                     the AnimationSequence frame count",
                    previous.unwrap_or_default()
                );
            }
            if let Some((current, new)) = sequence_change {
                let sequence_path = sequence.sequence.as_ref().expect("a sequence count comes from --sequence");
                let sequence_output = sequence_output.as_ref().ok_or_else(|| {
                    eyre::eyre!("the frames per direction change from {current} to {new}: pass --sequence-output")
                })?;
                require_absolute(sequence_output, "--sequence-output")?;
                let sequence_id = sequence.sequence_id.unwrap_or(body);
                let dialect = sequence.dialect.resolve(sequence_path);
                write_sequence_frame_count(sequence_path, sequence_output, sequence_id, u32::from(action), new, dialect)?;
                println!(
                    "Wrote {} ({}): sequence {sequence_id} action {action}: {current} -> {new} frames per direction.",
                    sequence_output.display(),
                    dialect.label()
                );
            }

            let change = write_entry(&uop, &output, body, action, &imported.payload)?;
            let report = imported.report;
            println!(
                "Encoded {frames} frames of body {body} action {action}: {} distinct colours{}{}; wrote {} (entry {}).",
                report.distinct_colours,
                if report.quantized { ", quantized to 256" } else { "" },
                if report.approximated > 0 {
                    format!(", {} pixels took the nearest palette colour", report.approximated)
                } else {
                    String::new()
                },
                output.display(),
                match change {
                    EntryChange::Unchanged => "unchanged",
                    EntryChange::Replaced => "replaced",
                    EntryChange::Added => "added",
                }
            );
        }
    }
    Ok(())
}
