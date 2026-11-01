//! `classic-image`: Classic art and gumps as PNG, in either container (`uocf::classic::image_files`).

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use uocf::classic::image_files::{ClassicImageContainer, ClassicImageFiles, ClassicImageKind, ImageWrite};
use uocf::enhanced::dds_writer::SizeChange;

#[derive(Subcommand, Debug)]
pub enum ClassicImageCmd {
    /// Export the image an art or gump id stores as a PNG (the colors as stored, unhued).
    Export {
        #[command(flatten)]
        source: SourceArgs,
        /// Output PNG.
        #[arg(long)]
        output: PathBuf,
    },
    /// Write copies of the art or gump files with an id set from an image, encoded as the shipped records are. Art
    /// below 0x4000 is a land tile: a 44x44 image, the diamond opaque, the corners transparent. Statics (item id +
    /// 0x4000) and gumps take alpha 0 or 255 only; channels round to 5 bits; opaque black is stored as 0x0001. A MUL
    /// copy appends the record and points its index there; a UOP copy replaces or adds the entry.
    Import {
        #[command(flatten)]
        source: SourceArgs,
        /// The image (PNG, BMP, TGA, JPEG), converted to RGBA.
        #[arg(long)]
        image: PathBuf,
        /// Folder the copies are written to (absolute; not the source folder), under the source file names.
        #[arg(long)]
        output_dir: PathBuf,
        /// Accept an image of another size than the record (gump layouts may assume the size).
        #[arg(long)]
        allow_resize: bool,
    },
}

#[derive(clap::Args, Debug)]
pub struct SourceArgs {
    /// Classic client folder with artidx.mul + art.mul / artLegacyMUL.uop or gumpidx.mul + gumpart.mul /
    /// gumpartLegacyMUL.uop.
    #[arg(long)]
    ccdir: PathBuf,
    #[arg(long, value_enum)]
    kind: KindArg,
    /// Art id (statics: item id + 0x4000) or gump id; decimal or 0x hex.
    #[arg(long, value_parser = parse_id)]
    id: u32,
    /// Container read (default: the only one found; MUL and UOP both found need it).
    #[arg(long, value_enum)]
    container: Option<ContainerArg>,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum KindArg {
    Art,
    Gump,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ContainerArg {
    Mul,
    Uop,
}

const HEX_PREFIX: &str = "0x";
const HEX_RADIX: u32 = 16;

fn parse_id(text: &str) -> Result<u32, String> {
    match text.strip_prefix(HEX_PREFIX) {
        Some(digits) => u32::from_str_radix(digits, HEX_RADIX),
        None => text.parse(),
    }
    .map_err(|error| format!("{text}: {error}"))
}

impl SourceArgs {
    fn files(&self) -> eyre::Result<ClassicImageFiles> {
        let kind = match self.kind {
            KindArg::Art => ClassicImageKind::Art,
            KindArg::Gump => ClassicImageKind::Gump,
        };
        let found = ClassicImageFiles::find(&self.ccdir, kind);
        let mut matching = found.into_iter().filter(|files| match (self.container, &files.container) {
            (None, _) => true,
            (Some(ContainerArg::Mul), ClassicImageContainer::Mul { .. }) | (Some(ContainerArg::Uop), ClassicImageContainer::Uop(_)) => true,
            _ => false,
        });
        let first = matching.next().ok_or_else(|| eyre::eyre!("{}: no {} files of that container", self.ccdir.display(), kind.label()))?;
        eyre::ensure!(matching.next().is_none(), "{} holds both the MUL and UOP {} files: pass --container", self.ccdir.display(), kind.label());
        Ok(first)
    }
}

pub fn run(cmd: ClassicImageCmd) -> eyre::Result<()> {
    match cmd {
        ClassicImageCmd::Export { source, output } => {
            let files = source.files()?;
            let image = files.read(source.id)?.ok_or_else(|| eyre::eyre!("{}: {} {:#x} holds no image", files.container.label(), files.kind.label(), source.id))?;
            image::save_buffer_with_format(&output, &image.rgba, image.width, image.height, image::ColorType::Rgba8, image::ImageFormat::Png)
                .wrap_err_with(|| format!("write {}", output.display()))?;
            println!("Wrote {} ({}, {}x{}).", output.display(), image.format.label(), image.width, image.height);
        }
        ClassicImageCmd::Import { source, image, output_dir, allow_resize } => {
            eyre::ensure!(output_dir.is_absolute(), "--output-dir must be an absolute path: {}", output_dir.display());
            let files = source.files()?;
            let rgba = image::open(&image).wrap_err_with(|| format!("read {}", image.display()))?.into_rgba8();
            let size_change = if allow_resize { SizeChange::Allow } else { SizeChange::Refuse };
            println!("{}", files.kind.rules(source.id));
            let prepared = files.prepare(source.id, &rgba, size_change)?;
            for note in &prepared.notes {
                println!("note: {note}");
            }
            std::fs::create_dir_all(&output_dir).wrap_err_with(|| format!("create {}", output_dir.display()))?;
            let output = files.container.in_dir(&output_dir);
            let write = files.write_copy(&prepared, &output)?;
            let what = match write {
                ImageWrite::Added => "added",
                ImageWrite::Replaced => "replaced",
                ImageWrite::Unchanged => "unchanged (the record was already this one)",
            };
            println!("Wrote {}: {} {:#x} {what}, a {} {}x{}.", output.label(), files.kind.label(), source.id, prepared.format.label(), prepared.width, prepared.height);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_parse_as_decimal_or_hex() {
        assert_eq!(parse_id("0x4005"), Ok(0x4005));
        assert_eq!(parse_id("12"), Ok(12));
        assert!(parse_id("0xZ").is_err());
    }
}
