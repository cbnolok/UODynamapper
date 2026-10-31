use std::fs;
use std::path::PathBuf;

use clap::{ArgGroup, Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use uocf::classic::sound::SoundMap;
use uocf::classic::sound_edit::ClassicSoundEditor;

#[derive(Subcommand, Debug)]
pub enum SoundCmd {
    /// Export one Classic Client sound to a playable audio file.
    #[command(group(
        ArgGroup::new("selector")
            .required(true)
            .args(["slot", "id"])
    ))]
    Export {
        /// Classic Client directory containing soundidx.mul and sound.mul, or soundLegacyMUL.uop.
        #[arg(long)]
        ccdir: PathBuf,
        /// Direct soundidx.mul slot.
        #[arg(long)]
        slot: Option<u32>,
        /// Logical sound id, using Sound.def translation when needed.
        #[arg(long)]
        id: Option<u32>,
        /// Output audio file.
        #[arg(long)]
        output: PathBuf,
        /// Output format. WAV is lossless and preserves the source PCM.
        #[arg(long, value_enum, default_value_t = SoundExportFormat::Wav)]
        format: SoundExportFormat,
    },
    /// Write copies of the sound files (sound.mul and soundidx.mul, else soundLegacyMUL.uop) with a slot set from
    /// a 16-bit PCM mono WAV at 22050 Hz. A replaced slot keeps its 40-byte header (the name field rewritten with
    /// --name); a new one gets the name and zeros. Verdata is not applied.
    Import {
        /// Classic Client directory containing soundidx.mul and sound.mul, or soundLegacyMUL.uop.
        #[arg(long)]
        ccdir: PathBuf,
        /// Folder for the written files: an absolute path, not the client directory.
        #[arg(long)]
        output: PathBuf,
        /// Direct soundidx.mul slot.
        #[arg(long)]
        slot: u32,
        /// The WAV file.
        #[arg(long)]
        input: PathBuf,
        /// The name in the slot header (up to 31 ASCII characters); required for a new slot.
        #[arg(long)]
        name: Option<String>,
        /// Decode any WAV or MP3 at 22050 Hz (or 22048 Hz) and downmix it to mono. There is no resampling.
        #[arg(long)]
        convert: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum SoundExportFormat {
    Wav,
}

pub fn run(cmd: SoundCmd) -> eyre::Result<()> {
    match cmd {
        SoundCmd::Export { ccdir, slot, id, output, format } => {
            export_sound(ccdir, slot, id, output, format)
        }
        SoundCmd::Import { ccdir, output, slot, input, name, convert } => {
            eyre::ensure!(output.is_absolute(), "--output must be an absolute path: {}", output.display());
            let file = fs::read(&input).wrap_err_with(|| format!("Failed to read {}", input.display()))?;
            let mut editor = ClassicSoundEditor::load(&ccdir)?;
            let put = editor.put_sound(slot, &file, name.as_deref(), convert)?;
            for warning in &put.warnings {
                log::warn!("slot {slot}: {warning}");
            }
            let written = editor.save_to(&output)?;
            log::info!(
                "{:?} slot {slot} '{}' ({} PCM bytes{}); wrote {}",
                put.write,
                put.name,
                put.pcm_bytes,
                if put.converted { ", converted" } else { "" },
                written.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ")
            );
            Ok(())
        }
    }
}

fn export_sound(
    ccdir: PathBuf,
    slot: Option<u32>,
    id: Option<u32>,
    output: PathBuf,
    format: SoundExportFormat,
) -> eyre::Result<()> {
    let sounds = SoundMap::load(&ccdir)?;

    let lookup = match (slot, id) {
        (Some(slot), None) => sounds
            .read_slot(slot)?
            .map(|sound| (sound, slot, false))
            .ok_or_else(|| eyre::eyre!("No sound exists at slot {}.", slot))?,
        (None, Some(id)) => {
            let found = sounds
                .read_id(id)?
                .ok_or_else(|| eyre::eyre!("No sound exists for id {}.", id))?;
            (found.sound, id, found.translated)
        }
        _ => unreachable!("clap enforces exactly one sound selector"),
    };

    let bytes = match format {
        SoundExportFormat::Wav => lookup.0.wav_bytes(),
    };

    fs::write(&output, bytes).wrap_err_with(|| format!("Failed to write {}", output.display()))?;

    log::info!(
        "Exported sound {} '{}' from slot {}{} to '{}'.",
        lookup.1,
        lookup.0.name,
        lookup.0.slot_id,
        if lookup.2 { " via Sound.def" } else { "" },
        output.display()
    );

    Ok(())
}
