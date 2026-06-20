use std::fs;
use std::path::PathBuf;

use clap::{ArgGroup, Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use uocf::classic::sound::SoundMap;

#[derive(Subcommand, Debug)]
pub enum SoundCmd {
    /// Export one Classic Client sound to a playable audio file.
    #[command(group(
        ArgGroup::new("selector")
            .required(true)
            .args(["slot", "id"])
    ))]
    Export {
        /// Classic Client directory containing soundidx.mul and sound.mul.
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

    println!(
        "Exported sound {} '{}' from slot {}{} to '{}'.",
        lookup.1,
        lookup.0.name,
        lookup.0.slot_id,
        if lookup.2 { " via Sound.def" } else { "" },
        output.display()
    );

    Ok(())
}
