use std::path::PathBuf;

use clap::Subcommand;
use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::dds_writer::{replace_texture_in_uop, SizeChange};
use uocf::uop_container::hash::hash_file_name_single;

const HEX_PREFIX: &str = "0x";
const HEX_RADIX: u32 = 16;
/// Hex digits of an entry hash written without the prefix.
const HASH_HEX_DIGITS: usize = 16;

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
}

/// The hash an entry argument names.
fn entry_hash(entry: &str) -> eyre::Result<u64> {
    if let Some(hex) = entry.strip_prefix(HEX_PREFIX) {
        return u64::from_str_radix(hex, HEX_RADIX).wrap_err_with(|| format!("bad entry hash {entry}"));
    }
    if entry.len() == HASH_HEX_DIGITS && entry.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return Ok(u64::from_str_radix(entry, HEX_RADIX)?);
    }
    Ok(hash_file_name_single(entry))
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
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
