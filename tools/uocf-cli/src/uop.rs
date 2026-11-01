use std::fs;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use uocf::audio::validate::FormatPolicy;
use uocf::enhanced::dds_writer::{replace_texture_in_uop, SizeChange};
use uocf::enhanced::entry_edit::{EntryCheck, EntryEdit, EntryTarget, ParseCheck, UopEntryEditor};
use uocf::enhanced::image_census::ContentFormat;
use uocf::uop_container::file::CompressionFlag;

#[derive(Subcommand, Debug)]
pub enum UopCmd {
    /// Replace a DDS texture of a package with an image, keeping the entry's pixel format, mip rule (one level or
    /// the full chain) and DDS header conventions, and save the package to a new file. The entry keeps its
    /// compression and gets a fresh timestamp; the other entries are untouched. The official patcher re-downloads
    /// edited entries, so keep the edited package in a client copy the patcher never runs on.
    ReplaceTexture {
        /// Source package (only read).
        uop: PathBuf,
        /// Entry name (`build/worldart/00001234.dds`) or hash (`0x` and hex, or 16 hex digits).
        entry: String,
        /// The new image (PNG, BMP), converted to RGBA.
        image: PathBuf,
        /// Output package; must not be the source.
        #[arg(long)]
        output: PathBuf,
        /// Accept an image of another size than the texture (the client may assume the original size).
        #[arg(long)]
        allow_resize: bool,
    },
    /// Write a copy of a package with an image entry replaced by an image converted to the entry's format, then
    /// checked as replace-entry checks a file: a `gumpartLegacyMUL.uop` entry becomes a Classic gump encoded as the
    /// shipped ones (alpha 0 or 255 only; 5 bits per channel; opaque black stored as 0x0001), a TGA entry a TGA of
    /// its type, depth, RLE packing and ending (footer, extension area, padding). DDS textures: replace-texture.
    ReplaceImage {
        /// Source package (only read).
        uop: PathBuf,
        /// Entry name (`build/gumpartlegacymul/00001234.tga`) or hash (`0x` and hex, or 16 hex digits).
        entry: String,
        /// The new image (PNG, BMP), converted to RGBA.
        image: PathBuf,
        /// Output package: an absolute path, not the source.
        #[arg(long)]
        output: PathBuf,
        /// Accept an image of another size than the entry (gump and button layouts may assume the size).
        #[arg(long)]
        allow_resize: bool,
    },
    /// Write a copy of a package with one entry replaced by a file of the same kind, checked by its content: the
    /// kind its extension or the current entry has, and, where uocf has a strict reader for the entry (DDS,
    /// images, Chaos scripts, the audio tables, the legacy terrain map and documents, hue bitmaps, tileart,
    /// animation, effect, terrain, facet, multi, waypoint and string records, and NIF models: they must read and
    /// link), a parse. The entry keeps its
    /// compression flag and gets a fresh timestamp; the other entries are untouched.
    ReplaceEntry {
        #[command(flatten)]
        edit: EntryArgs,
    },
    /// Write a copy of a package with a new entry, checked like replace-entry against its name's extension and
    /// the package's readers, stored with the flag the package's entries of its kind have unless --compression.
    AddEntry {
        #[command(flatten)]
        edit: EntryArgs,
        /// The new entry's compression.
        #[arg(long, value_enum)]
        compression: Option<EntryCompression>,
    },
}

#[derive(Args, Debug)]
pub struct EntryArgs {
    /// Source package (only read).
    uop: PathBuf,
    /// Entry name (lowercase, `/` separators) or hash (`0x` and hex, or 16 hex digits).
    entry: String,
    /// The file the entry will hold.
    file: PathBuf,
    /// Output package: an absolute path, not the source.
    #[arg(long)]
    output: PathBuf,
    /// For MP3 and WAV entries: accept audio formats no shipped entry has (a file of another kind is still
    /// refused).
    #[arg(long)]
    allow_unverified_format: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum EntryCompression {
    None,
    Zlib,
}

impl From<EntryCompression> for CompressionFlag {
    fn from(compression: EntryCompression) -> Self {
        match compression {
            EntryCompression::None => CompressionFlag::None,
            EntryCompression::Zlib => CompressionFlag::Zlib,
        }
    }
}

impl EntryArgs {
    fn run(&self, put: impl FnOnce(&mut UopEntryEditor, &EntryTarget, &[u8], FormatPolicy) -> eyre::Result<EntryEdit>) -> eyre::Result<()> {
        eyre::ensure!(self.output.is_absolute(), "--output must be an absolute path: {}", self.output.display());
        let target = EntryTarget::parse(&self.entry)?;
        let file = fs::read(&self.file).wrap_err_with(|| format!("Failed to read {}", self.file.display()))?;
        let policy = if self.allow_unverified_format { FormatPolicy::AllowUnverified } else { FormatPolicy::VerifiedOnly };
        let mut editor = UopEntryEditor::load(&self.uop)?.with_check(NIF_CHECK);
        let edit = put(&mut editor, &target, &file, policy)?;
        for warning in &edit.warnings {
            log::warn!("{warning}");
        }
        editor.save_as(&self.output)?;
        let check = match &edit.check {
            EntryCheck::Parsed { reader, dialect: Some(dialect) } => format!("checked: {reader} ({})", dialect.label()),
            EntryCheck::Parsed { reader, dialect: None } => format!("checked: {reader}"),
            EntryCheck::Kind(kind) => format!("{} (no reader for this entry)", kind.label()),
            EntryCheck::Unchecked(kind) => format!("{}, unchecked (no extension to check against)", kind.label()),
        };
        println!(
            "Wrote {}: entry {} {:?}, {:?} compression, {check}.",
            self.output.display(),
            edit.name.clone().unwrap_or_else(|| format!("{:016X}", edit.hash)),
            edit.write,
            edit.compression
        );
        Ok(())
    }
}

/// A replaced or added NIF must read with `gamebryo-nif` and have no wrong link.
pub const NIF_CHECK: ParseCheck = ParseCheck {
    label: "NIF model",
    packages: &[],
    kinds: &[ContentFormat::Nif],
    name: None,
    dialects: false,
    check: |bytes, _| gamebryo_nif::read_checked(bytes).map(|_| ()).map_err(|error| eyre::eyre!(error)),
    keeps: None,
};

/// The hash an entry argument names.
fn entry_hash(entry: &str) -> eyre::Result<u64> {
    Ok(EntryTarget::parse(entry)?.hash())
}

pub fn run(cmd: UopCmd) -> eyre::Result<()> {
    match cmd {
        UopCmd::ReplaceTexture { uop, entry, image, output, allow_resize } => {
            let hash = entry_hash(&entry)?;
            let rgba = image::open(&image).wrap_err_with(|| format!("failed to read {}", image.display()))?.into_rgba8();
            let size_change = if allow_resize { SizeChange::Allow } else { SizeChange::Refuse };
            let replacement = replace_texture_in_uop(&uop, hash, &rgba, size_change, &output)
                .wrap_err_with(|| format!("failed to replace {entry} in {}", uop.display()))?;
            for warning in replacement.warnings() {
                log::warn!("{warning}");
            }
            println!(
                "Wrote {}: entry {hash:016X} is now {} {}x{}, {} mip level{}, header {}.",
                output.display(),
                replacement.format.label(),
                rgba.width(),
                rgba.height(),
                replacement.mip_count,
                if replacement.mip_count == 1 { "" } else { "s" },
                replacement.style.map_or("as the original (not a shipped style)".to_owned(), |style| format!("{style:?}"))
            );
        }
        UopCmd::ReplaceImage { uop, entry, image, output, allow_resize } => {
            eyre::ensure!(output.is_absolute(), "--output must be an absolute path: {}", output.display());
            let target = EntryTarget::parse(&entry)?;
            let rgba = image::open(&image).wrap_err_with(|| format!("failed to read {}", image.display()))?.into_rgba8();
            let size_change = if allow_resize { SizeChange::Allow } else { SizeChange::Refuse };
            let mut editor = UopEntryEditor::load(&uop)?;
            let (converted, edit) = editor.replace_image(&target, &rgba, size_change)?;
            for note in converted.notes.iter().chain(&edit.warnings) {
                log::warn!("{note}");
            }
            editor.save_as(&output)?;
            println!(
                "Wrote {}: entry {} is now a {} {}x{} ({} bytes), {:?} compression.",
                output.display(),
                target.label(),
                converted.format.label(),
                rgba.width(),
                rgba.height(),
                converted.bytes.len(),
                edit.compression
            );
        }
        UopCmd::ReplaceEntry { edit } => edit.run(|editor, target, file, policy| editor.replace(target, file, policy))?,
        UopCmd::AddEntry { edit, compression } => {
            edit.run(|editor, target, file, policy| editor.add(target, file, compression.map(Into::into), policy))?
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::uop_container::hash::hash_file_name_single;

    const HASH: u64 = 0xBEA6_C96E_A9B7_27F4;
    const NAME: &str = "data/interface/login/textures/login_02.dds";

    #[test]
    fn entries_are_named_by_hash_or_name() {
        assert_eq!(entry_hash("0xBEA6C96EA9B727F4").unwrap(), HASH);
        assert_eq!(entry_hash("bea6c96ea9b727f4").unwrap(), HASH);
        assert_eq!(entry_hash(NAME).unwrap(), hash_file_name_single(NAME));
        assert!(entry_hash("0xnothex").is_err());
    }
}
