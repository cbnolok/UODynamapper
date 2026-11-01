use crate::models::{AtlasPageInfo, AtlasPixelFormat, DecodedAtlasPage};
use crate::utils::crop_rgba;

/// Bytes per RGBA8888 pixel.
const RGBA_BYTES_PER_PIXEL: usize = 4;

/// The RGBA pixels of `rect` (x, y, width, height) on a page: a BC7 page decodes only the 4x4 blocks covering
/// the rect, which gives the bytes of [`decode_atlas_page`] cropped to it.
pub fn decode_atlas_rect(page_info: AtlasPageInfo, data: &[u8], rect: [usize; 4]) -> Option<Vec<u8>> {
    match page_info.pixel_format {
        AtlasPixelFormat::Bc7 => {
            let extent =
                udd_assets::bc7::ImageExtent::new(page_info.atlas_width, page_info.atlas_height).ok()?;
            let [x, y, width, height] = rect.map(|value| u32::try_from(value).ok());
            let rect = udd_assets::bc7::PixelRect { x: x?, y: y?, width: width?, height: height? };
            udd_assets::bc7::decode_bc7_rect_to_rgba8888(data, extent, rect).ok()
        }
        AtlasPixelFormat::Rgba8888 => {
            let page_width = [
                (page_info.used_width, page_info.used_height),
                (page_info.atlas_width, page_info.atlas_height),
            ]
            .into_iter()
            .find(|&(width, height)| data.len() == width as usize * height as usize * RGBA_BYTES_PER_PIXEL)?
            .0;
            Some(crop_rgba(data, page_width as usize, rect))
        }
    }
}

pub fn decode_atlas_page(page_info: AtlasPageInfo, data: Vec<u8>) -> Option<DecodedAtlasPage> {
    match page_info.pixel_format {
        AtlasPixelFormat::Bc7 => {
            let extent =
                udd_assets::bc7::ImageExtent::new(page_info.atlas_width, page_info.atlas_height)
                    .ok()?;
            let rgba = udd_assets::bc7::decode_bc7_to_rgba8888(&data, extent).ok()?;
            Some(DecodedAtlasPage {
                rgba,
                width: page_info.atlas_width,
                height: page_info.atlas_height,
            })
        }
        AtlasPixelFormat::Rgba8888 => {
            let used_len = page_info.used_width as usize * page_info.used_height as usize * 4;
            let atlas_len = page_info.atlas_width as usize * page_info.atlas_height as usize * 4;
            if data.len() == used_len {
                Some(DecodedAtlasPage {
                    rgba: data,
                    width: page_info.used_width,
                    height: page_info.used_height,
                })
            } else if data.len() == atlas_len {
                Some(DecodedAtlasPage {
                    rgba: data,
                    width: page_info.atlas_width,
                    height: page_info.atlas_height,
                })
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Directory with installed atlas packages, for [`a_tile_decodes_from_its_rect_as_from_the_whole_page`].
    const PACKAGE_DIR_ENV: &str = "UDD_PACKAGE_DIR";
    const ATLAS_PACKAGES: [&str; 2] = ["tex_land_ec.uddp", "tex_art_ec.uddp"];
    /// Timed selections per package.
    const TIMED_SELECTIONS: usize = 16;

    #[test]
    #[ignore = "needs packages: set UDD_PACKAGE_DIR to a folder with tex_land_ec.uddp and tex_art_ec.uddp; run with --release"]
    fn a_tile_decodes_from_its_rect_as_from_the_whole_page() {
        use crate::logic::discovery::load_atlas_package;
        use crate::models::VirtualEntryData;
        use std::collections::HashMap;
        use std::time::{Duration, Instant};

        let dir = std::path::PathBuf::from(std::env::var_os(PACKAGE_DIR_ENV).expect("set UDD_PACKAGE_DIR"));
        for name in ATLAS_PACKAGES {
            let reader = udd_container::UddpReader::load(dir.join(name)).expect("load package");
            let package = load_atlas_package(&reader).expect("an atlas package").expect("read atlas package");
            let page_infos = package.page_infos();
            let mut rects_by_page: HashMap<u32, Vec<[usize; 4]>> = HashMap::new();
            for entry in package.virtual_entries() {
                if let VirtualEntryData::AtlasRect { page_index, x, y, width, height, .. } = entry.data {
                    rects_by_page
                        .entry(page_index)
                        .or_default()
                        .push([x as usize, y as usize, width as usize, height as usize]);
                }
            }
            // The page with the most tiles, BC7 if the package has BC7 pages (the installed tex_art_ec is RGBA).
            let (page_index, rects) = rects_by_page
                .into_iter()
                .max_by_key(|(page_index, rects)| (page_infos[page_index].pixel_format == AtlasPixelFormat::Bc7, rects.len()))
                .expect("a page with tiles");
            let info = page_infos[&page_index];
            let bytes = crate::utils::atlas_page_paths(page_index)
                .iter()
                .find_map(|path| reader.read_file_by_path_hash(udd_container::xxh64_virtual_path(path)).ok())
                .expect("page bytes");

            let started = Instant::now();
            let page = decode_atlas_page(info, bytes.clone()).expect("decode the whole page");
            let whole_page_decode = started.elapsed();
            for rect in &rects {
                let cropped = crop_rgba(&page.rgba, page.width as usize, *rect);
                assert_eq!(decode_atlas_rect(info, &bytes, *rect).as_ref(), Some(&cropped), "{name} page {page_index} {rect:?}");
            }

            // A selection before: the whole page decoded, the tile cropped and the page made an image for the
            // Full Atlas texture. Now: the tile's blocks decoded.
            let timed = &rects[..rects.len().min(TIMED_SELECTIONS)];
            let (mut before, mut after) = (Duration::ZERO, Duration::ZERO);
            for rect in timed {
                let started = Instant::now();
                let page = decode_atlas_page(info, bytes.clone()).unwrap();
                let _tile = crop_rgba(&page.rgba, page.width as usize, *rect);
                let _image = eframe::egui::ColorImage::from_rgba_unmultiplied(
                    [page.width as usize, page.height as usize],
                    &page.rgba,
                );
                before += started.elapsed();
                let started = Instant::now();
                let _tile = decode_atlas_rect(info, &bytes, *rect).unwrap();
                after += started.elapsed();
            }
            let count = timed.len() as u32;
            println!(
                "{name} page {page_index} ({}, {}x{}, {} tiles, all equal): whole-page decode {whole_page_decode:?}; \
                 selection before {:?}, after {:?} (mean of {count})",
                crate::utils::atlas_pixel_format_to_str(info.pixel_format),
                info.atlas_width,
                info.atlas_height,
                rects.len(),
                before / count,
                after / count
            );
        }
    }

    #[test]
    fn decode_atlas_page_uses_manifest_used_size_for_rgba() {
        let info = AtlasPageInfo {
            atlas_width: 4096,
            atlas_height: 2048,
            used_width: 3,
            used_height: 2,
            pixel_format: AtlasPixelFormat::Rgba8888,
        };
        let data = vec![255u8; 3 * 2 * 4];
        let decoded = decode_atlas_page(info, data).unwrap();

        assert_eq!(decoded.width, 3);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.rgba.len(), 24);
    }
}
