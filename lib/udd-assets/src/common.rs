use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use color_eyre::eyre::{self, WrapErr};
use bytemuck::Pod;
use udd_container::UddpReader;

use crate::bc7::{decode_bc7_to_rgba8888, extract_bc7_subrect, extract_rgba8_subrect, ImageExtent};
use crate::tex_art_cc::PagePixelFormat;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtlasCacheOptions {
    pub enable_page_cache: bool,
}

impl AtlasCacheOptions {
    pub const fn disabled() -> Self {
        Self {
            enable_page_cache: false,
        }
    }

    pub const fn enabled() -> Self {
        Self {
            enable_page_cache: true,
        }
    }
}

pub fn read_path_entry(package: &UddpReader, path: &str) -> eyre::Result<Vec<u8>> {
    package
        .read_file_by_path_hash(udd_container::xxh64_virtual_path(path))
        .wrap_err_with(|| format!("read {path}"))
}

pub(crate) struct AtlasPageCache {
    enabled: bool,
    pages: RwLock<HashMap<u32, Arc<[u8]>>>,
}

impl AtlasPageCache {
    pub(crate) fn new(options: AtlasCacheOptions) -> Self {
        Self {
            enabled: options.enable_page_cache,
            pages: RwLock::new(HashMap::new()),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn clear(&self) {
        self.pages.write().expect("atlas page cache poisoned").clear();
    }

    pub(crate) fn read_page_bytes<F>(&self, page_index: u32, load_page: F) -> eyre::Result<Vec<u8>>
    where
        F: FnOnce() -> eyre::Result<Vec<u8>>,
    {
        Ok(self.read_page_bytes_arc(page_index, load_page)?.as_ref().to_vec())
    }

    pub(crate) fn read_page_bytes_arc<F>(
        &self,
        page_index: u32,
        load_page: F,
    ) -> eyre::Result<Arc<[u8]>>
    where
        F: FnOnce() -> eyre::Result<Vec<u8>>,
    {
        if !self.enabled {
            return Ok(Arc::from(load_page()?));
        }

        if let Some(bytes) = self
            .pages
            .read()
            .expect("atlas page cache poisoned")
            .get(&page_index)
            .cloned()
        {
            return Ok(bytes);
        }

        let bytes: Arc<[u8]> = Arc::from(load_page()?);
        let mut pages = self.pages.write().expect("atlas page cache poisoned");
        Ok(pages.entry(page_index).or_insert_with(|| bytes.clone()).clone())
    }
}

pub(crate) fn decode_atlas_page_rgba(
    page_bytes: &[u8],
    pixel_format: PagePixelFormat,
    atlas_width: u32,
    atlas_height: u32,
    used_width: u32,
    used_height: u32,
) -> eyre::Result<Vec<u8>> {
    match pixel_format {
        PagePixelFormat::Rgba8888 => Ok(page_bytes.to_vec()),
        PagePixelFormat::Bc7 => {
            let extent = ImageExtent::new(atlas_width, atlas_height)
                .map_err(|error| eyre::eyre!("invalid page extent {atlas_width}x{atlas_height}: {error}"))?;
            let decoded = decode_bc7_to_rgba8888(page_bytes, extent)
                .map_err(|error| eyre::eyre!("decode BC7 page {atlas_width}x{atlas_height}: {error}"))?;
            Ok(extract_rgba8_subrect(&decoded, atlas_width, 0, 0, used_width, used_height))
        }
    }
}

pub(crate) fn extract_atlas_subrect_rgba(
    page_bytes: &[u8],
    pixel_format: PagePixelFormat,
    atlas_width: u32,
    atlas_height: u32,
    used_width: u32,
    used_height: u32,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
) -> eyre::Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Ok(Vec::new());
    }

    let x = x as u32;
    let y = y as u32;
    let width = width as u32;
    let height = height as u32;

    if x + width > used_width || y + height > used_height {
        eyre::bail!(
            "slot rect {}x{} at {},{} exceeds page extent {}x{}",
            width,
            height,
            x,
            y,
            used_width,
            used_height
        );
    }

    match pixel_format {
        PagePixelFormat::Rgba8888 => Ok(extract_rgba8_subrect(page_bytes, used_width, x, y, width, height)),
        PagePixelFormat::Bc7 => {
            let page_extent = ImageExtent::new(atlas_width, atlas_height)
                .map_err(|error| eyre::eyre!("invalid page extent {atlas_width}x{atlas_height}: {error}"))?;

            let aligned_x = (x / 4) * 4;
            let aligned_y = (y / 4) * 4;
            let aligned_right = (x + width).div_ceil(4) * 4;
            let aligned_bottom = (y + height).div_ceil(4) * 4;
            let aligned_extent = ImageExtent::new(aligned_right - aligned_x, aligned_bottom - aligned_y)
                .map_err(|error| {
                    eyre::eyre!(
                        "invalid aligned BC7 rect {}x{}: {error}",
                        aligned_right - aligned_x,
                        aligned_bottom - aligned_y
                    )
                })?;

            let blocks = extract_bc7_subrect(page_bytes, page_extent, aligned_x, aligned_y, aligned_extent);
            let decoded = decode_bc7_to_rgba8888(&blocks, aligned_extent)
                .map_err(|error| eyre::eyre!("decode BC7 slot rect: {error}"))?;
            Ok(extract_rgba8_subrect(
                &decoded,
                aligned_extent.width(),
                x - aligned_x,
                y - aligned_y,
                width,
                height,
            ))
        }
    }
}

pub fn read_pod_vec<T: Pod>(bytes: &[u8], entry_path: &str) -> eyre::Result<Vec<T>> {
    let record_size = std::mem::size_of::<T>();
    if bytes.len() % record_size != 0 {
        eyre::bail!(
            "{} size {} is not a multiple of record size {}",
            entry_path,
            bytes.len(),
            record_size,
        );
    }

    Ok(bytes
        .chunks_exact(record_size)
        .map(bytemuck::pod_read_unaligned)
        .collect())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use dds::{ColorFormat, CompressionQuality, EncodeOptions, Format, ImageView, Size};

    #[test]
    fn atlas_page_cache_loads_each_page_once() {
        let cache = AtlasPageCache::new(AtlasCacheOptions::enabled());
        let loads = AtomicUsize::new(0);

        let first = cache
            .read_page_bytes_arc(7, || {
                loads.fetch_add(1, Ordering::Relaxed);
                Ok(vec![1, 2, 3, 4])
            })
            .unwrap();
        let second = cache
            .read_page_bytes_arc(7, || {
                loads.fetch_add(1, Ordering::Relaxed);
                Ok(vec![9, 9, 9, 9])
            })
            .unwrap();

        assert_eq!(&*first, &[1, 2, 3, 4]);
        assert_eq!(&*second, &[1, 2, 3, 4]);
        assert_eq!(loads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn disabled_atlas_page_cache_reloads_every_time() {
        let cache = AtlasPageCache::new(AtlasCacheOptions::disabled());
        let loads = AtomicUsize::new(0);

        let first = cache
            .read_page_bytes_arc(7, || {
                loads.fetch_add(1, Ordering::Relaxed);
                Ok(vec![1, 2, 3, 4])
            })
            .unwrap();
        let second = cache
            .read_page_bytes_arc(7, || {
                loads.fetch_add(1, Ordering::Relaxed);
                Ok(vec![9, 9, 9, 9])
            })
            .unwrap();

        assert_eq!(&*first, &[1, 2, 3, 4]);
        assert_eq!(&*second, &[9, 9, 9, 9]);
        assert_eq!(loads.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn extract_atlas_subrect_rgba_matches_bc7_page_decode_crop() {
        let width = 8;
        let height = 8;
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for chunk in rgba.chunks_exact_mut(4) {
            chunk.copy_from_slice(&[40, 120, 200, 255]);
        }

        let image = ImageView::new(&rgba, Size::new(width, height), ColorFormat::RGBA_U8).unwrap();
        let mut encoded = Vec::new();
        let mut options = EncodeOptions::default();
        options.quality = CompressionQuality::Normal;
        dds::encode(&mut encoded, image, Format::BC7_UNORM, None, &options).unwrap();

        let decoded = decode_atlas_page_rgba(&encoded, PagePixelFormat::Bc7, width, height, width, height).unwrap();
        let expected = extract_rgba8_subrect(&decoded, width, 1, 2, 5, 3);
        let actual = extract_atlas_subrect_rgba(
            &encoded,
            PagePixelFormat::Bc7,
            width,
            height,
            width,
            height,
            1,
            2,
            5,
            3,
        )
        .unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn decode_bc7_page_uses_full_atlas_extent_before_cropping() {
        let atlas_width = 8;
        let atlas_height = 8;
        let used_width = 4;
        let used_height = 4;
        let mut rgba = vec![0u8; (atlas_width * atlas_height * 4) as usize];
        for chunk in rgba.chunks_exact_mut(4) {
            chunk.copy_from_slice(&[12, 34, 56, 255]);
        }

        let image = ImageView::new(&rgba, Size::new(atlas_width, atlas_height), ColorFormat::RGBA_U8).unwrap();
        let mut encoded = Vec::new();
        let mut options = EncodeOptions::default();
        options.quality = CompressionQuality::Normal;
        dds::encode(&mut encoded, image, Format::BC7_UNORM, None, &options).unwrap();

        let decoded = decode_atlas_page_rgba(
            &encoded,
            PagePixelFormat::Bc7,
            atlas_width,
            atlas_height,
            used_width,
            used_height,
        )
        .unwrap();

        assert_eq!(decoded.len(), (used_width * used_height * 4) as usize);
    }
}
