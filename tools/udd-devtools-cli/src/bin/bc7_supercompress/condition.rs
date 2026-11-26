//! A conditioned copy of a mobile animation package, for looking at a lossy variant in the game: the package with its
//! BC7 pages replaced by the blocks `lossy --store` wrote for one variant (`<pages>/<page position>.zst`), each page
//! stored again with the codec it has in the package, every other entry copied as stored. The format, the records and
//! the layout stay the package's; only the blocks change. Rewriting the plain encode's pages gives back the package
//! byte for byte when the packer and this tool agree (`lossy --check` confirms the blocks, this the coding).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use clap::Args;
use color_eyre::eyre::{self, WrapErr};
use rayon::prelude::*;
use udd_assets::animation::mobile_anim_cc;
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_container::{AddOwnedFileRequest, Codec, CompressedEntry, CompressionFlag, UddpBuilder, xxh64_virtual_path};

use crate::corpus::{Kind, Package};
use crate::{PERCENT, mib};

/// The level the packer stores a page's field split at with Zstd (udd-conv `MOBILE_ANIM_PAGE_ZSTD_LEVEL`), which the
/// container does not record.
const PAGE_ZSTD_LEVEL: i32 = 17;
/// Pages between progress lines.
const PROGRESS_PAGES: usize = 200;

#[derive(Args)]
pub struct ConditionArgs {
    /// The BC7 package the pages were built from (`lossy`'s `bc7`).
    bc7: PathBuf,
    #[arg(value_enum)]
    kind: Kind,
    /// One variant's folder of `lossy --store`: every page of the stored layout.
    pages: PathBuf,
    /// The package to write.
    output: PathBuf,
}

pub fn condition(args: &ConditionArgs) -> eyre::Result<()> {
    eyre::ensure!(args.output != args.bc7, "the output must not be the package read");
    let package = Package::open(&args.bc7, args.kind)?;
    let done = AtomicUsize::new(0);
    let entries: Vec<(CompressedEntry, u64)> = package.pages.par_iter().enumerate().map(|(at, page)| {
        let compression = match page.entry.codec {
            Codec::Bc7FieldsZstd => CompressionFlag::Bc7FieldsZstdLevel(PAGE_ZSTD_LEVEL),
            Codec::Bc7FieldsLz4Hc => CompressionFlag::Bc7FieldsLz4Hc,
            Codec::Lz4Hc => CompressionFlag::Lz4Hc,
            other => eyre::bail!("page {}: codec {other:?}, whose level the package does not record", page.index),
        };
        let file = args.pages.join(format!("{at}.zst"));
        let raw_size = page.entry.raw_size as usize;
        let blocks = zstd::bulk::decompress(&std::fs::read(&file).wrap_err_with(|| format!("read {}", file.display()))?,
            raw_size)?;
        eyre::ensure!(blocks.len() == raw_size, "{}: {} bytes, the page has {raw_size}", file.display(), blocks.len());
        let entry = CompressedEntry::new(AddOwnedFileRequest {
            data_type: page.entry.data_type,
            compression,
            width: page.used_width,
            height: page.used_height,
            virtual_path: None,
            path_hash64: Some(xxh64_virtual_path(&mobile_anim_cc::page_entry_path(page.index, PagePixelFormat::Bc7))),
            id: None,
            data: blocks,
        }).map_err(|error| eyre::eyre!("page {}: {error:?}", page.index))?;
        let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
        if finished % PROGRESS_PAGES == 0 {
            eprintln!("{finished} of {} pages", package.pages.len());
        }
        Ok((entry, u64::from(page.entry.stored_size)))
    }).collect::<eyre::Result<_>>()?;
    let (before, after) = entries.iter().fold((0u64, 0u64), |(before, after), (entry, stored)| {
        (before + stored, after + entry.stored_len() as u64)
    });
    let entries: Vec<CompressedEntry> = entries.into_iter().map(|(entry, _)| entry).collect();
    UddpBuilder::write_with_compressed_entries(&package.reader, &entries, |_| false, &args.output)
        .map_err(|error| eyre::eyre!("write {}: {error:?}", args.output.display()))?;
    let package_before = std::fs::metadata(&args.bc7)?.len();
    let package_after = std::fs::metadata(&args.output)?.len();
    println!("{} pages: {:.2} -> {:.2} MiB ({:+.2} %); package {:.2} -> {:.2} MiB ({:+.2} %): {}",
        entries.len(), mib(before), mib(after), (after as f64 / before as f64 - 1.0) * PERCENT, mib(package_before),
        mib(package_after), (package_after as f64 / package_before as f64 - 1.0) * PERCENT, args.output.display());
    Ok(())
}
