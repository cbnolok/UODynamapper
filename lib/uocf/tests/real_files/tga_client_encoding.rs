//! Every TGA entry of the client packages under `UO_TGA_CLIENT_ROOTS` (directories separated like `PATH`: the EC,
//! EC fp and KR clients) written again with its own convention: from its stored pixels, byte for byte where the
//! convention reproduces it, and from the RGBA the image crate decodes, as a replacement PNG would be. Prints the
//! rates and the styles per package.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rayon::prelude::*;
use uocf::enhanced::image_census::{classify_content, find_packages, ContentFormat};
use uocf::enhanced::textures::decode_tga_rgba8;
use uocf::enhanced::tga_writer::{reencode, TgaConvention};
use uocf::uop_container::package::UopPackage;

const CLIENT_ROOTS_ENV: &str = "UO_TGA_CLIENT_ROOTS";

#[derive(Default)]
struct Rates {
    tgas: usize,
    placeholders: usize,
    reproduced: usize,
    /// No known RLE style gives them back.
    unreproduced: Vec<String>,
    /// From the image crate's RGBA.
    from_rgba: usize,
    styles: BTreeMap<String, usize>,
}

#[test]
#[ignore = "needs real clients: set UO_TGA_CLIENT_ROOTS and run with --ignored"]
fn every_shipped_tga_encodes_back() {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    for root in std::env::split_paths(&roots).collect::<Vec<PathBuf>>() {
        for path in find_packages(&root).unwrap() {
            let package = UopPackage::load(&path).unwrap();
            let hashes: Vec<u64> = package.iter_files().filter(|file| !file.is_empty_slot()).map(|file| file.filename_hash()).collect();
            let results: Vec<Option<(String, bool, bool, bool)>> = hashes
                .par_iter()
                .map(|&hash| {
                    let payload = package.unpack_file_by_hash(hash).unwrap().unwrap();
                    if !matches!(classify_content(&payload).format, ContentFormat::Tga | ContentFormat::TgaTrailing) {
                        return None;
                    }
                    let convention = TgaConvention::of(&payload).unwrap_or_else(|error| panic!("{} {hash:016X}: {error:#}", path.display()));
                    if convention.width() == 0 || convention.height() == 0 {
                        return Some(("0x0 placeholder".to_owned(), true, true, true));
                    }
                    let again = reencode(&payload).unwrap();
                    assert_eq!(again == payload, convention.reproduces(), "{} {hash:016X}: {}", path.display(), convention.label());
                    let rgba = decode_tga_rgba8(&payload).unwrap();
                    let from_rgba = convention.encode_rgba(&rgba).map(|bytes| bytes == payload).unwrap_or(false);
                    let label = convention.label();
                    Some((if convention.reproduces() { label } else { format!("{label} {hash:016X}") }, convention.reproduces(), from_rgba, false))
                })
                .collect();
            let mut rates = Rates::default();
            for (label, reproduced, from_rgba, placeholder) in results.into_iter().flatten() {
                rates.tgas += 1;
                if placeholder {
                    rates.placeholders += 1;
                    continue;
                }
                if reproduced {
                    rates.reproduced += 1;
                    *rates.styles.entry(label).or_default() += 1;
                } else {
                    rates.unreproduced.push(label);
                }
                rates.from_rgba += usize::from(from_rgba);
            }
            if rates.tgas == 0 {
                continue;
            }
            let images = rates.tgas - rates.placeholders;
            println!(
                "{}: {} TGAs ({} 0x0 placeholders); {} of {images} byte for byte from their pixels, {} from the decoded RGBA",
                path.display(),
                rates.tgas,
                rates.placeholders,
                rates.reproduced,
                rates.from_rgba
            );
            for (style, count) in &rates.styles {
                println!("    {count} {style}");
            }
            for label in &rates.unreproduced {
                println!("    not reproduced: {label}");
            }
        }
    }
}
