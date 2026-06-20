mod legacy_mul;
mod mul_convert;
mod multimap;
mod sound;

use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::eyre::{self, Context};
use std::path::{Path, PathBuf};
use uocf::classic::michelangelo_uop_codec::{
    export_anim_blocks_from_mul, MichelangeloPatch, MichelangeloPatchEntry,
};
use uocf::classic::vd_codec::VdFile;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Export animation payloads as .vd or Michelangelo/UOAnimTool .uop patches.
    ExportAnimPatch {
        /// Source animation format.
        #[arg(long, value_enum)]
        source: AnimationPatchSource,

        /// Source animation index file for classic-mul, for example anim.idx.
        #[arg(long)]
        idx: Option<PathBuf>,

        /// Source animation data file for classic-mul, for example anim.mul.
        #[arg(long)]
        mul: Option<PathBuf>,

        /// Raw animation block index for classic-mul. Repeat for multi-entry Michelangelo UOP output.
        #[arg(long, value_name = "INDEX")]
        block: Vec<i32>,

        /// Source body id used when remapping classic-mul block indices.
        #[arg(long, default_value_t = 0)]
        source_anim: i32,

        /// Target body id used when remapping classic-mul block indices. Defaults to source-anim.
        #[arg(long)]
        target_anim: Option<i32>,

        /// Source AnimationFrame*.uop package for cc-animation-frame or ec-animation-frame.
        #[arg(long)]
        uop: Option<PathBuf>,

        /// Animation body id for cc-animation-frame or ec-animation-frame sources.
        #[arg(long)]
        body: Option<u32>,

        /// Classic Client AnimationFrame group id, used in build/animationlegacyframe/{body}/{group}.bin.
        #[arg(long, default_value_t = 0)]
        group: u8,

        /// Patch entry index for AnimationFrame sources. Defaults to body.
        #[arg(long)]
        target_index: Option<i32>,

        /// Patch entry extra value for AnimationFrame sources.
        #[arg(long, default_value_t = 0)]
        extra: i32,

        /// Output patch format.
        #[arg(long, value_enum)]
        format: AnimationPatchExportFormat,

        /// Output .vd or Michelangelo/UOAnimTool .uop path.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Classic Client sound.mul/soundidx.mul utility.
    #[command(subcommand)]
    Sound(sound::SoundCmd),
    /// Classic Client multimap.rle converter.
    #[command(subcommand)]
    Multimap(multimap::MultimapCmd),
    /// UO Legacy MUL/UOP format converter.
    #[command(subcommand)]
    MulConvert(mul_convert::MulConvertCmd),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AnimationPatchSource {
    ClassicMul,
    CcAnimationFrame,
    EcAnimationFrame,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AnimationPatchExportFormat {
    Vd,
    MichelangeloUop,
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let _ = udd_logging::install_tracing_indicatif_logger();
    let cli = Cli::parse();

    match cli.command {
        Commands::ExportAnimPatch {
            source,
            idx,
            mul,
            block,
            source_anim,
            target_anim,
            uop,
            body,
            group,
            target_index,
            extra,
            format,
            output,
        } => {
            let patch = build_animation_patch(
                source,
                idx.as_deref(),
                mul.as_deref(),
                &block,
                source_anim,
                target_anim.unwrap_or(source_anim),
                uop.as_deref(),
                body,
                group,
                target_index,
                extra,
            )?;
            let entry_count = write_animation_patch(patch, format, &output)?;
            println!(
                "Exported {} animation patch entr{} to {}.",
                entry_count,
                if entry_count == 1 { "y" } else { "ies" },
                output.display()
            );
        }
        Commands::Sound(cmd) => sound::run(cmd)?,
        Commands::Multimap(cmd) => multimap::run(cmd)?,
        Commands::MulConvert(cmd) => mul_convert::run(cmd)?,
    }

    Ok(())
}

fn build_animation_patch(
    source: AnimationPatchSource,
    idx: Option<&Path>,
    mul: Option<&Path>,
    block_ids: &[i32],
    source_anim: i32,
    target_anim: i32,
    uop: Option<&Path>,
    body: Option<u32>,
    group: u8,
    target_index: Option<i32>,
    extra: i32,
) -> eyre::Result<MichelangeloPatch> {
    match source {
        AnimationPatchSource::ClassicMul => {
            if source_anim < 0 {
                return Err(eyre::eyre!("source-anim cannot be negative: {source_anim}"));
            }
            if target_anim < 0 {
                return Err(eyre::eyre!("target-anim cannot be negative: {target_anim}"));
            }
            if block_ids.is_empty() {
                return Err(eyre::eyre!("classic-mul export requires at least one --block"));
            }
            let idx = idx.ok_or_else(|| eyre::eyre!("classic-mul export requires --idx"))?;
            let mul = mul.ok_or_else(|| eyre::eyre!("classic-mul export requires --mul"))?;
            export_anim_blocks_from_mul(idx.to_path_buf(), mul, block_ids, source_anim, target_anim)
        }
        AnimationPatchSource::CcAnimationFrame => {
            let body = body
                .ok_or_else(|| eyre::eyre!("cc-animation-frame export requires --body"))?;
            let uop = uop
                .ok_or_else(|| eyre::eyre!("cc-animation-frame export requires --uop"))?;
            let payload = read_animationframe_payload(uop, &cc_animationframe_path(body, group))?;
            Ok(single_payload_patch(target_index.unwrap_or(body as i32), extra, payload))
        }
        AnimationPatchSource::EcAnimationFrame => {
            let body = body
                .ok_or_else(|| eyre::eyre!("ec-animation-frame export requires --body"))?;
            let uop = uop
                .ok_or_else(|| eyre::eyre!("ec-animation-frame export requires --uop"))?;
            let payload = read_animationframe_payload(uop, &ec_animationframe_path(body))?;
            Ok(single_payload_patch(target_index.unwrap_or(body as i32), extra, payload))
        }
    }
}

fn write_animation_patch(
    patch: MichelangeloPatch,
    format: AnimationPatchExportFormat,
    output: &Path,
) -> eyre::Result<usize> {
    match format {
        AnimationPatchExportFormat::Vd => {
            if patch.entries.len() != 1 {
                return Err(eyre::eyre!(
                    ".vd export requires exactly one animation patch entry, got {}",
                    patch.entries.len()
                ));
            }
            let entry = patch.entries.into_iter().next().unwrap();
            VdFile::for_anim(entry.index, entry.extra, entry.data)?.save(output)?;
            Ok(1)
        }
        AnimationPatchExportFormat::MichelangeloUop => {
            let entry_count = patch.entries.len();
            patch.save(output)?;
            Ok(entry_count)
        }
    }
}

fn read_animationframe_payload(uop_path: &Path, internal_path: &str) -> eyre::Result<Vec<u8>> {
    let package = UopPackage::load(uop_path)
        .with_context(|| format!("failed to load {}", uop_path.display()))?;
    let hash = hash_file_name_single(internal_path);
    let file = package.get_file_by_hash(hash).ok_or_else(|| {
        eyre::eyre!(
            "AnimationFrame entry '{}' (0x{:016x}) not found in {}",
            internal_path,
            hash,
            uop_path.display()
        )
    })?;
    file.unpack()
        .with_context(|| format!("failed to unpack AnimationFrame entry '{internal_path}'"))
}

fn single_payload_patch(index: i32, extra: i32, payload: Vec<u8>) -> MichelangeloPatch {
    MichelangeloPatch {
        entries: vec![MichelangeloPatchEntry::anim(index, extra, payload)],
    }
}

fn cc_animationframe_path(body: u32, group: u8) -> String {
    format!("build/animationlegacyframe/{:06}/{:02}.bin", body, group)
}

fn ec_animationframe_path(body: u32) -> String {
    format!("data/animationframe/{:06}.bin", body)
}
