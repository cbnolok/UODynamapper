//! Every DDS payload of every UOP under the client roots (found by its magic after decompression) against the DDS
//! writer: the header the writer gives for its format, size and convention equals the shipped header byte for
//! byte; uncompressed textures decode and re-encode byte for byte; block-compressed ones re-encode within a stated
//! error; the shipped mip chains are compared with both mip filters. A texture of a real package is replaced and
//! the package saved to a copy.
//!
//! The roots come from `UO_CLIENT_ROOTS` (a path list, `:`-separated on Unix), searched recursively for `*.uop`
//! files; they are only read. The copy goes to `std::env::temp_dir()` (set `TMPDIR` to choose it) and is deleted
//! afterwards. Pixel tests skip payloads already seen in another package (EC and EC fp share most).

use crate::common;

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use image::RgbaImage;
use rayon::prelude::*;
use uocf::enhanced::dds_census::find_packages;
use uocf::enhanced::dds_writer::{
    decode_dds_levels, decode_surface, downsample_level, encode_surface, mip_level_size, replace_texture_in_uop,
    DdsDescription, DdsHeaderConvention, DdsHeaderStyle, MipFilter, SizeChange, DDS_MAGIC,
    SHIPPED_MIP_FILTER,
};
use uocf::uop_container::data_block_header::DataBlockHeaderRecord;
use uocf::uop_container::package::{LoadMode, UopPackage};

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
/// The shipped headers outside the [`DdsHeaderStyle`] presets: `fly.dds` of EC fp `Interface.uop` writes the
/// LINEARSIZE flag with the row bytes (one payload).
const NON_PRESET_HEADERS: usize = 1;
const RGBA_CHANNELS: usize = 4;
const COLOR_CHANNELS: usize = 3;
const ALPHA_CHANNEL: usize = 3;
const OPAQUE: u8 = u8::MAX;
/// Error bounds of a block-compressed texture re-encoded from its decoded pixels, against those pixels: the
/// root-mean-square channel error of each format over every texel of every level, and the largest error of a
/// single channel.
const BC_RMSE_BOUND: f64 = 0.25;
const BC_MAX_ERROR_BOUND: u8 = 64;
/// The packages the replacement test edits, in each the DDS with the most mip levels: single-level terrain
/// textures, full-chain effect textures (`DDSX` headers).
const REPLACED_PACKAGES: [&str; 2] = ["TerrainTexture.uop", "EffectTexture.uop"];
/// The mean channel error allowed for the replaced mip 0 against the new image.
const REPLACED_MEAN_ERROR_BOUND: f64 = 2.0;

/// Every `*.uop` under the client roots.
fn client_packages() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let packages: Vec<PathBuf> =
        std::env::split_paths(&roots).flat_map(|root| find_packages(&root).unwrap_or_default()).collect();
    assert!(!packages.is_empty(), "no UOP packages under {CLIENT_ROOTS_ENV}");
    packages
}

/// Calls `visit` with every DDS payload of every package (in parallel), with its package and entry hash. With
/// `unique`, a payload already seen is skipped.
fn for_each_dds(unique: bool, visit: impl Fn(&Path, u64, &[u8]) + Sync) -> usize {
    let seen = Mutex::new(HashSet::new());
    let mut visited = 0;
    for path in client_packages() {
        let package = UopPackage::load_with_mode(&path, LoadMode::Lazy).unwrap();
        let hashes: Vec<u64> =
            package.iter_files().filter(|file| file.has_size()).map(|file| file.filename_hash()).collect();
        visited += hashes
            .par_iter()
            .map_init(
                || package.open_payload_reader().unwrap(),
                |reader, &hash| {
                    let payload = package.unpack_file_by_hash_from_reader(hash, reader).unwrap().unwrap();
                    if !payload.starts_with(DDS_MAGIC) {
                        return 0;
                    }
                    if unique {
                        let mut hasher = DefaultHasher::new();
                        payload.hash(&mut hasher);
                        if !seen.lock().unwrap().insert(hasher.finish()) {
                            return 0;
                        }
                    }
                    visit(&path, hash, &payload);
                    1
                },
            )
            .sum::<usize>();
    }
    visited
}

fn context(path: &Path, hash: u64) -> String {
    format!("{} {hash:016X}", path.display())
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn shipped_dds_headers_are_written_byte_for_byte() {
    let styles: Mutex<BTreeMap<String, usize>> = Mutex::default();
    let non_preset: Mutex<Vec<String>> = Mutex::default();
    let failures: Mutex<Vec<String>> = Mutex::default();
    let total = for_each_dds(false, |path, hash, payload| {
        let description = DdsDescription::parse(payload).unwrap();
        let convention = DdsHeaderConvention::of_header(&description).unwrap();
        let written = convention.header_bytes(description.width(), description.height()).unwrap();
        if written[..] != payload[..description.header_len] {
            failures.lock().unwrap().push(context(path, hash));
        }
        let style = DdsHeaderStyle::of_convention(&convention);
        if style.is_none() {
            non_preset.lock().unwrap().push(format!("{} {convention:?}", context(path, hash)));
        }
        let key = format!("{:?} {}", style, description.format.label());
        *styles.lock().unwrap().entry(key).or_default() += 1;
    });
    println!("{total} DDS headers");
    for (style, count) in styles.lock().unwrap().iter() {
        println!("  {style}: {count}");
    }
    let failures = failures.into_inner().unwrap();
    assert!(failures.is_empty(), "{} headers differ, first {:?}", failures.len(), failures.first());
    let non_preset = non_preset.into_inner().unwrap();
    assert_eq!(non_preset.len(), NON_PRESET_HEADERS, "headers outside the presets: {non_preset:#?}");
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn uncompressed_dds_decode_and_encode_byte_for_byte() {
    let counts: Mutex<BTreeMap<String, usize>> = Mutex::default();
    let failures: Mutex<Vec<String>> = Mutex::default();
    for_each_dds(true, |path, hash, payload| {
        let description = DdsDescription::parse(payload).unwrap();
        if description.format.is_block_compressed() {
            return;
        }
        let (_, levels) = decode_dds_levels(payload).unwrap();
        let convention = DdsHeaderConvention::of_header(&description).unwrap();
        let mut written = convention.header_bytes(description.width(), description.height()).unwrap();
        for level in &levels {
            written.extend(encode_surface(description.format, level.width(), level.height(), level.as_raw()).unwrap());
        }
        if written != payload {
            failures.lock().unwrap().push(format!("{} {}", context(path, hash), description.format.label()));
        }
        *counts.lock().unwrap().entry(description.format.label()).or_default() += 1;
    });
    println!("uncompressed DDS re-encoded byte for byte: {:?}", counts.lock().unwrap());
    let failures = failures.into_inner().unwrap();
    assert!(failures.is_empty(), "{} payloads differ: {failures:#?}", failures.len());
}

/// Channel error totals of re-encoded block-compressed surfaces.
#[derive(Default, Debug)]
struct ErrorStats {
    textures: usize,
    channels: u64,
    squared: f64,
    absolute: u64,
    max: u8,
    blocks: u64,
    identical_blocks: u64,
}

impl ErrorStats {
    fn rmse(&self) -> f64 {
        (self.squared / self.channels as f64).sqrt()
    }

    fn mean_absolute(&self) -> f64 {
        self.absolute as f64 / self.channels as f64
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn block_compressed_dds_reencode_within_bound() {
    let stats: Mutex<BTreeMap<String, ErrorStats>> = Mutex::default();
    for_each_dds(true, |_, _, payload| {
        let description = DdsDescription::parse(payload).unwrap();
        let Some(block_bytes) = description.format.block_bytes() else {
            return;
        };
        let (_, levels) = decode_dds_levels(payload).unwrap();
        let mut local = ErrorStats { textures: 1, ..Default::default() };
        let mut offset = description.header_len;
        for level in &levels {
            let (width, height) = level.dimensions();
            let encoded = encode_surface(description.format, width, height, level.as_raw()).unwrap();
            let shipped = &payload[offset..offset + encoded.len()];
            offset += encoded.len();
            local.blocks += (encoded.len() / block_bytes as usize) as u64;
            local.identical_blocks += encoded
                .chunks_exact(block_bytes as usize)
                .zip(shipped.chunks_exact(block_bytes as usize))
                .filter(|(ours, theirs)| ours == theirs)
                .count() as u64;
            let decoded = decode_surface(description.format, width, height, &encoded).unwrap();
            for (&ours, &theirs) in decoded.iter().zip(level.as_raw()) {
                let error = ours.abs_diff(theirs);
                local.channels += 1;
                local.absolute += u64::from(error);
                local.squared += f64::from(error) * f64::from(error);
                local.max = local.max.max(error);
            }
        }
        let mut stats = stats.lock().unwrap();
        let total = stats.entry(description.format.label()).or_default();
        total.textures += local.textures;
        total.channels += local.channels;
        total.absolute += local.absolute;
        total.squared += local.squared;
        total.max = total.max.max(local.max);
        total.blocks += local.blocks;
        total.identical_blocks += local.identical_blocks;
    });
    for (format, stats) in stats.lock().unwrap().iter() {
        println!(
            "{format}: {} textures, RMSE {:.3}, mean abs {:.3}, max {}, identical blocks {:.1}%",
            stats.textures,
            stats.rmse(),
            stats.mean_absolute(),
            stats.max,
            100.0 * stats.identical_blocks as f64 / stats.blocks as f64
        );
        assert!(stats.rmse() <= BC_RMSE_BOUND, "{format}: RMSE {:.3} above {BC_RMSE_BOUND}", stats.rmse());
        assert!(stats.max <= BC_MAX_ERROR_BOUND, "{format}: max error {} above {BC_MAX_ERROR_BOUND}", stats.max);
    }
}

/// Colour error totals of one mip filter against the shipped mips.
#[derive(Default, Debug, Clone, Copy)]
struct FilterStats {
    texels: u64,
    absolute: u64,
    exact: u64,
    opaque_texels: u64,
    opaque_absolute: u64,
}

impl FilterStats {
    fn add(&mut self, other: &Self) {
        self.texels += other.texels;
        self.absolute += other.absolute;
        self.exact += other.exact;
        self.opaque_texels += other.opaque_texels;
        self.opaque_absolute += other.opaque_absolute;
    }

    fn mean(&self) -> f64 {
        self.absolute as f64 / (self.texels * COLOR_CHANNELS as u64) as f64
    }

    fn opaque_mean(&self) -> f64 {
        self.opaque_absolute as f64 / (self.opaque_texels * COLOR_CHANNELS as u64).max(1) as f64
    }

    fn exact_share(&self) -> f64 {
        100.0 * self.exact as f64 / self.texels as f64
    }
}

const FILTERS: [MipFilter; 2] = [MipFilter::BoxGamma, MipFilter::BoxLinear];

/// Each shipped mip level against the level above filtered with each [`MipFilter`]: the mean colour error, the
/// share of texels whose colour is exact, and the mean over opaque texels. The shipped filter is the closer one.
#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn shipped_mips_are_closer_to_the_shipped_filter() {
    let stats: Mutex<BTreeMap<String, [FilterStats; FILTERS.len()]>> = Mutex::default();
    for_each_dds(true, |_, _, payload| {
        let description = DdsDescription::parse(payload).unwrap();
        if description.mip_count < 2 {
            return;
        }
        let (_, levels) = decode_dds_levels(payload).unwrap();
        let mut local = [FilterStats::default(); FILTERS.len()];
        for pair in levels.windows(2) {
            let (above, shipped) = (&pair[0], &pair[1]);
            for (filter, local) in FILTERS.iter().zip(local.iter_mut()) {
                let ours = downsample_level(above.width(), above.height(), above.as_raw(), *filter);
                for (ours, theirs) in ours.chunks_exact(RGBA_CHANNELS).zip(shipped.as_raw().chunks_exact(RGBA_CHANNELS)) {
                    let error: u64 = (0..COLOR_CHANNELS).map(|channel| u64::from(ours[channel].abs_diff(theirs[channel]))).sum();
                    local.texels += 1;
                    local.absolute += error;
                    local.exact += u64::from(error == 0);
                    if theirs[ALPHA_CHANNEL] == OPAQUE {
                        local.opaque_texels += 1;
                        local.opaque_absolute += error;
                    }
                }
            }
        }
        let mut stats = stats.lock().unwrap();
        let totals = stats.entry(description.format.label()).or_default();
        for (total, local) in totals.iter_mut().zip(&local) {
            total.add(local);
        }
    });
    let mut overall = [FilterStats::default(); FILTERS.len()];
    for (format, totals) in stats.lock().unwrap().iter() {
        for (filter, total) in FILTERS.iter().zip(totals) {
            println!(
                "{format} {filter:?}: {} texels, mean abs {:.3}, exact {:.2}%, opaque mean abs {:.3}",
                total.texels,
                total.mean(),
                total.exact_share(),
                total.opaque_mean()
            );
        }
        for (overall, total) in overall.iter_mut().zip(totals) {
            overall.add(total);
        }
    }
    let shipped = FILTERS.iter().position(|filter| *filter == SHIPPED_MIP_FILTER).unwrap();
    for (index, filter) in FILTERS.iter().enumerate() {
        println!("all {filter:?}: mean abs {:.3}, exact {:.2}%", overall[index].mean(), overall[index].exact_share());
        assert!(overall[shipped].mean() <= overall[index].mean(), "{filter:?} is closer than {SHIPPED_MIP_FILTER:?}");
    }
}

/// Replaces a texture of each real [`REPLACED_PACKAGES`] package with an inverted copy and saves the package to a
/// copy: the entry keeps its format, mip rule, header convention and compression and gets a fresh timestamp;
/// every other entry is unchanged. An image of another size is refused.
#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn a_texture_of_a_client_package_is_replaced() {
    let dir = common::temp_dir("dds_replace_texture");
    let mut replaced = 0;
    let replaced_package = |path: &PathBuf| REPLACED_PACKAGES.iter().any(|name| path.file_name().unwrap() == *name);
    for source in client_packages().into_iter().filter(replaced_package) {
        let package = UopPackage::load(&source).unwrap();
        // KR's EffectTexture.uop holds only TGA images.
        let Some((hash, original)) = package
            .iter_files()
            .filter(|file| file.has_size())
            .map(|file| (file.filename_hash(), file.unpack().unwrap()))
            .filter(|(_, payload)| payload.starts_with(DDS_MAGIC))
            .max_by_key(|(_, payload)| DdsDescription::parse(payload).unwrap().mip_count)
        else {
            continue;
        };
        let (description, levels) = decode_dds_levels(&original).unwrap();
        let mut image: RgbaImage = levels[0].clone();
        image.pixels_mut().for_each(|pixel| (0..COLOR_CHANNELS).for_each(|channel| pixel[channel] = !pixel[channel]));
        let output = dir.join(format!("replaced_{replaced}.uop"));
        let wrong_size = RgbaImage::new(image.width() / 2, image.height());
        assert!(replace_texture_in_uop(&source, hash, &wrong_size, SizeChange::Refuse, &output).is_err());
        let replacement = replace_texture_in_uop(&source, hash, &image, SizeChange::Refuse, &output).unwrap();
        assert_eq!(replacement.format, description.format);
        assert_eq!(replacement.mip_count, description.mip_count);
        assert!(replacement.warnings().is_empty());

        let written = UopPackage::load(&output).unwrap();
        let file = written.get_file_by_hash(hash).unwrap();
        assert_eq!(file.compression(), package.get_file_by_hash(hash).unwrap().compression());
        let header = file.parse_data_block_header().unwrap().unwrap();
        assert!(matches!(header.records(), [DataBlockHeaderRecord::Timestamp(_)]), "{:?}", header.records());
        let payload = file.unpack().unwrap();
        assert_eq!(payload, replacement.bytes);
        let (new_description, new_levels) = decode_dds_levels(&payload).unwrap();
        assert_eq!(new_description.header_len, description.header_len);
        let convention = DdsHeaderConvention::of_header(&description).unwrap();
        assert_eq!(DdsHeaderConvention::of_header(&new_description).unwrap(), convention);
        assert_eq!(new_levels.len(), levels.len());
        let error: u64 = new_levels[0].as_raw().iter().zip(image.as_raw()).map(|(a, b)| u64::from(a.abs_diff(*b))).sum();
        assert!((error as f64 / image.as_raw().len() as f64) < REPLACED_MEAN_ERROR_BOUND);
        for other in package.iter_files().filter(|file| file.has_size() && file.filename_hash() != hash) {
            let kept = written.get_file_by_hash(other.filename_hash()).unwrap();
            assert_eq!(kept.unpack().unwrap(), other.unpack().unwrap());
            assert_eq!(kept.data_block_hash(), other.data_block_hash());
        }
        let (width, height) = mip_level_size(image.width(), image.height(), description.mip_count - 1);
        println!(
            "{}: replaced {hash:016X} ({} {}x{}, {} levels down to {width}x{height}, {:?})",
            source.display(),
            description.format.label(),
            image.width(),
            image.height(),
            replacement.mip_count,
            replacement.style
        );
        replaced += 1;
    }
    std::fs::remove_dir_all(dir).unwrap();
    assert!(replaced > 0, "no {REPLACED_PACKAGES:?} under {CLIENT_ROOTS_ENV}");
}
