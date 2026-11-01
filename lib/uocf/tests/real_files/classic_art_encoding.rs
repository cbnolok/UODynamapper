//! The Classic art encoder on real clients of every era: every land and static record of `artLegacyMUL.uop`
//! (Classic clients) or `art.mul` (the others) decodes, encodes back to the same colors, and to the same bytes
//! except for the padding that held old memory (the 24 bytes after a land diamond, the `u16` that pads a static
//! row to 4 bytes). The rates are printed.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a client with neither art file is
//! skipped. Nothing is written to disk. Run with `--ignored --release`.

use std::path::{Path, PathBuf};

use uocf::classic::art::{ArtMap, ArtSource, LAND_DIAMOND_PIXEL_COUNT, decode_land_tile_from_raw, decode_static_tile_from_raw};
use uocf::classic::art_write::{ArtColors, ArtKind, LAND_RECORD_BYTES};
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
const ART_UOP_FILE: &str = "artLegacyMUL.uop";
const ART_MUL_FILE: &str = "art.mul";
const LAND_DIAMOND_BYTES: usize = LAND_DIAMOND_PIXEL_COUNT * size_of::<u16>();
const STATIC_HEIGHT_WORD: usize = 3;
const STATIC_LOOKUP_WORD: usize = 4;
const RGBA_BYTES: usize = 4;
const LAND_RGBA_BYTES: usize = 44 * 44 * RGBA_BYTES;

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

#[derive(Default, Debug)]
struct Counts {
    records: usize,
    undecodable: usize,
    identical: usize,
    padding_only: usize,
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 { 100.0 } else { part as f64 * 100.0 / whole as f64 }
}

/// The word indexes of the row padding in an encoded static: the `u16` after a row's `(0, 0)` that brings the row
/// to a multiple of 4 bytes.
fn static_padding_words(encoded: &[u8]) -> Vec<usize> {
    let words: Vec<u16> = encoded.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
    let height = usize::from(words[STATIC_HEIGHT_WORD]);
    let rows_start = STATIC_LOOKUP_WORD + height;
    let mut starts: Vec<usize> = words[STATIC_LOOKUP_WORD..rows_start].iter().map(|&start| rows_start + usize::from(start)).collect();
    starts.sort_unstable();
    starts.dedup();
    let mut padding = Vec::new();
    for start in starts {
        let mut at = start;
        loop {
            let (offset, run) = (words[at], usize::from(words[at + 1]));
            at += 2;
            if offset == 0 && run == 0 {
                break;
            }
            at += run;
        }
        if (at - start) % 2 == 1 {
            padding.push(at);
        }
    }
    padding
}

/// Whether `encoded` equals `shipped` outside the padding the encoder writes as zeros.
fn equal_but_padding(kind: ArtKind, shipped: &[u8], encoded: &[u8]) -> bool {
    if shipped.len() != encoded.len() {
        return false;
    }
    match kind {
        ArtKind::Land => shipped[..LAND_DIAMOND_BYTES] == encoded[..LAND_DIAMOND_BYTES],
        ArtKind::Static => {
            let mut masked = shipped.to_vec();
            for word in static_padding_words(encoded) {
                masked[word * 2..word * 2 + 2].fill(0);
            }
            masked == encoded
        }
    }
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored --release"]
fn every_shipped_art_record_encodes_back() {
    let mut clients = 0usize;
    for root in client_roots() {
        let name = label(&root);
        let source = if find_file_ignoring_case(&root, ART_UOP_FILE).is_some() {
            ArtSource::CcUop
        } else if find_file_ignoring_case(&root, ART_MUL_FILE).is_some() {
            ArtSource::Mul
        } else {
            println!("{name}: no art, skipped");
            continue;
        };
        clients += 1;
        let art = ArtMap::load(&root).unwrap();
        let (mut land, mut statics) = (Counts::default(), Counts::default());
        let mut shipped = Vec::new();
        let mut land_rgba = [0u8; LAND_RGBA_BYTES];
        for id in 0..art.max_id_for_source(source) {
            if !art.has_id_from_source(id, source) || art.get_raw_art_data_from_source(id, source, &mut shipped).is_err() {
                continue;
            }
            let kind = ArtKind::of_id(id);
            let counts = if kind == ArtKind::Land { &mut land } else { &mut statics };
            counts.records += 1;
            let Ok(colors) = ArtColors::decode(kind, &shipped) else {
                counts.undecodable += 1;
                continue;
            };
            let encoded = colors.encode().unwrap_or_else(|error| panic!("{name}: art {id:#x}: {error:#}"));
            assert_eq!(ArtColors::decode(kind, &encoded).unwrap(), colors, "{name}: art {id:#x}: colors");
            let (_, _, rgba) = colors.to_rgba();
            match kind {
                ArtKind::Land => {
                    decode_land_tile_from_raw(&shipped, &mut land_rgba).unwrap();
                    assert_eq!(rgba, land_rgba, "{name}: land {id:#x}: RGBA");
                    assert_eq!(encoded.len(), LAND_RECORD_BYTES);
                }
                ArtKind::Static => {
                    assert_eq!(rgba, decode_static_tile_from_raw(&shipped).unwrap().2, "{name}: static {id:#x}: RGBA");
                }
            }
            if encoded == shipped {
                counts.identical += 1;
            } else if equal_but_padding(kind, &shipped, &encoded) {
                counts.padding_only += 1;
            } else {
                panic!("{name}: art {id:#x} ({kind:?}): {} bytes shipped, {} encoded", shipped.len(), encoded.len());
            }
        }
        for (what, counts) in [("land", &land), ("static", &statics)] {
            let decoded = counts.records - counts.undecodable;
            println!(
                "{name}: {} {what} records, {} not decodable (kept as they are); of the {decoded} others {} ({:.2}%) byte for byte, {} only the padding differs",
                counts.records,
                counts.undecodable,
                counts.identical,
                percent(counts.identical, decoded),
                counts.padding_only,
            );
        }
        assert!(land.records > 0 && statics.records > 0, "{name}: no art records read");
    }
    assert!(clients > 0, "no client with art");
}
