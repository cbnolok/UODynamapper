//! The atlas pages a mobile animation package hands its clips' stored frames from ([`MobileAnimClips::stored_clip`]),
//! decoded once and kept in a small cache shared by every thread that reads clips.
//!
//! A BC7 page stored in the field split (container codecs 4 and 5, [`udd_container::bc7_fields`]) is kept split: a
//! clip joins only its frames' blocks from it, never the whole page. Any other page is kept as its bytes. Measured on
//! a crowd of the game's bodies in the loader's order (`docs/agent_notes/bc7_anim_supercompression.md`), a cache of
//! [`MOBILE_PAGE_CACHE_PAGES`] pages cuts the pages decoded as much as more pages do.
//!
//! [`MobileAnimClips::stored_clip`]: crate::animation::mobile_strides::MobileAnimClips::stored_clip

use std::sync::{Arc, Condvar, Mutex, PoisonError};

use color_eyre::eyre::{self, WrapErr};
use udd_container::UddpReader;
use udd_container::bc7_fields::FieldsPage;

use crate::animation::mobile_strides::StoredFramePixels;
use crate::bc7::ImageExtent;
use crate::common::{bc7_payload_extent, extract_atlas_subrect_stored, extract_bc7_fields_subrect_stored,
    read_path_entry};
use crate::tex_art_cc::PagePixelFormat;

/// The decoded pages a mobile animation package keeps for its clips. One BC7 page of a 2048-square atlas is 4 MiB
/// as blocks and up to 4.5 MiB split, so 6 pages hold at most 27 MiB. Measured on 20 crowds of 34 mobiles in the
/// loader's order (2026-11-26), 2 pages already cut a random crowd's page decodes to the floor (Classic 766 to 340,
/// Enhanced 804 to 457; 16 pages make it 332 and 447), but an Enhanced crowd of humans needs 6: 49 decodes uncached,
/// 30 with 2 pages, 6 with 4, and 5 with 6, 8 or 16.
pub const MOBILE_PAGE_CACHE_PAGES: usize = 6;

/// Which of a package's page sets a page belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageSet {
    /// The 1x frames' pages (`pages/`).
    Frames,
    /// The 2x frames' pages (`upscaled_pages/`, [`crate::animation::mobile_upscaled`]).
    Upscaled,
}

/// A page as the clips read it.
pub(crate) enum StoredPage {
    /// The page's bytes, RGBA8 or BC7 blocks.
    Bytes(Vec<u8>),
    /// A BC7 page kept in its field split.
    Bc7Fields(FieldsPage),
}

/// Where a page lies in its atlas, to cut frames from it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageLayout {
    pub pixel_format: PagePixelFormat,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub used_width: u32,
    pub used_height: u32,
}

impl StoredPage {
    /// The page stored under `path` in `package` with `layout`: split when its entry is, else its bytes.
    pub(crate) fn read(package: &UddpReader, path: &str, layout: PageLayout) -> eyre::Result<Self> {
        let hash = udd_container::xxh64_virtual_path(path);
        let split = package.read_bc7_fields_by_path_hash(hash).wrap_err_with(|| format!("read {path}"))?;
        let Some(split) = split else {
            return read_path_entry(package, path).map(Self::Bytes);
        };
        eyre::ensure!(layout.pixel_format == PagePixelFormat::Bc7, "{path} is a BC7 field split of a page that isn't BC7");
        let extent: ImageExtent = bc7_payload_extent(split.raw_len, layout.atlas_width, layout.atlas_height,
            layout.used_width, layout.used_height).wrap_err_with(|| format!("read {path}"))?;
        FieldsPage::new(split.stream, split.raw_len, extent.blocks_wide() as usize)
            .map(Self::Bc7Fields)
            .map_err(|error| eyre::eyre!("read {path}: {error}"))
    }

    /// The frame of `width` by `height` at `x`,`y` as the page stores it ([`StoredFramePixels`]).
    pub(crate) fn frame(&self, layout: PageLayout, x: u16, y: u16, width: u16, height: u16)
        -> eyre::Result<StoredFramePixels> {
        let PageLayout { pixel_format, atlas_width, atlas_height, used_width, used_height } = layout;
        match self {
            Self::Bytes(bytes) => extract_atlas_subrect_stored(bytes, pixel_format, atlas_width, atlas_height,
                used_width, used_height, x, y, width, height),
            Self::Bc7Fields(page) => extract_bc7_fields_subrect_stored(page, atlas_width, atlas_height, used_width,
                used_height, x, y, width, height),
        }
    }
}

type PageKey = (PageSet, u32);

/// The pages kept, most recently used last, and the pages a thread is decoding.
#[derive(Default)]
struct CacheState {
    pages: Vec<(PageKey, Arc<StoredPage>)>,
    loading: Vec<PageKey>,
}

/// A least-recently-used cache of decoded pages. A page another thread is decoding is waited for, not decoded again.
pub(crate) struct StoredPageCache {
    capacity: usize,
    state: Mutex<CacheState>,
    loaded: Condvar,
}

impl Default for StoredPageCache {
    fn default() -> Self {
        Self::new(MOBILE_PAGE_CACHE_PAGES)
    }
}

/// Takes a page off the loading list when its decode ends, also by a panic, and wakes the threads waiting for it.
struct LoadingGuard<'a> {
    cache: &'a StoredPageCache,
    key: PageKey,
}

impl Drop for LoadingGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.cache.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.loading.retain(|key| *key != self.key);
        drop(state);
        self.cache.loaded.notify_all();
    }
}

impl StoredPageCache {
    pub(crate) fn new(capacity: usize) -> Self {
        Self { capacity, state: Mutex::default(), loaded: Condvar::new() }
    }

    /// Page `key`, decoded by `load` unless kept or being decoded by another thread. A failed decode is not kept:
    /// the threads waiting for it try again.
    pub(crate) fn page(&self, key: PageKey, load: impl FnOnce() -> eyre::Result<StoredPage>)
        -> eyre::Result<Arc<StoredPage>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(index) = state.pages.iter().position(|(kept, _)| *kept == key) {
                let entry = state.pages.remove(index);
                let page = Arc::clone(&entry.1);
                state.pages.push(entry);
                return Ok(page);
            }
            if !state.loading.contains(&key) {
                break;
            }
            state = self.loaded.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
        state.loading.push(key);
        drop(state);
        let guard = LoadingGuard { cache: self, key };
        let page = Arc::new(load()?);
        if self.capacity > 0 {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.pages.len() >= self.capacity {
                state.pages.remove(0);
            }
            state.pages.push((key, Arc::clone(&page)));
        }
        drop(guard);
        Ok(page)
    }
}

/// The pages one clip's frames were cut from, a few at a time: its frames mostly share a page, so they look it up
/// once instead of once a frame.
#[derive(Default)]
pub(crate) struct ClipPages {
    pages: Vec<(PageKey, Arc<StoredPage>)>,
}

/// The pages a [`ClipPages`] keeps, as `PageMemo` keeps them.
const CLIP_PAGES: usize = 2;

impl ClipPages {
    /// Page `key` through `cache`, decoded by `load` unless kept.
    pub(crate) fn page(&mut self, cache: &StoredPageCache, key: PageKey,
        load: impl FnOnce() -> eyre::Result<StoredPage>) -> eyre::Result<Arc<StoredPage>> {
        if let Some((_, page)) = self.pages.iter().find(|(kept, _)| *kept == key) {
            return Ok(Arc::clone(page));
        }
        let page = cache.page(key, load)?;
        if self.pages.len() >= CLIP_PAGES {
            self.pages.remove(0);
        }
        self.pages.push((key, Arc::clone(&page)));
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    const KEY: PageKey = (PageSet::Frames, 3);

    fn page(byte: u8) -> eyre::Result<StoredPage> {
        Ok(StoredPage::Bytes(vec![byte]))
    }

    fn bytes(page: &StoredPage) -> &[u8] {
        match page {
            StoredPage::Bytes(bytes) => bytes,
            StoredPage::Bc7Fields(_) => panic!("a split page"),
        }
    }

    #[test]
    fn the_least_recently_used_page_goes_first() {
        let cache = StoredPageCache::new(2);
        let loads = AtomicUsize::new(0);
        let load = |key: PageKey| cache.page(key, || {
            loads.fetch_add(1, Ordering::Relaxed);
            page(key.1 as u8)
        }).unwrap();
        load((PageSet::Frames, 1));
        load((PageSet::Frames, 2));
        load((PageSet::Frames, 1));
        // Page 2 is the least recently used now, so page 3 takes its place.
        load((PageSet::Frames, 3));
        assert_eq!(loads.load(Ordering::Relaxed), 3);
        load((PageSet::Frames, 1));
        assert_eq!(loads.load(Ordering::Relaxed), 3);
        load((PageSet::Frames, 2));
        assert_eq!(loads.load(Ordering::Relaxed), 4);
        // The page sets don't share pages.
        assert_eq!(bytes(&load((PageSet::Upscaled, 2))), [2]);
        assert_eq!(loads.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn threads_asking_for_a_page_being_decoded_wait_for_it() {
        const THREADS: usize = 4;
        const DECODE_TIME: Duration = Duration::from_millis(50);
        let cache = StoredPageCache::new(1);
        let loads = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    let page = cache.page(KEY, || {
                        loads.fetch_add(1, Ordering::Relaxed);
                        std::thread::sleep(DECODE_TIME);
                        page(7)
                    }).unwrap();
                    assert_eq!(bytes(&page), [7]);
                });
            }
        });
        assert_eq!(loads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_failed_or_panicking_decode_is_not_kept() {
        let cache = StoredPageCache::new(1);
        assert!(cache.page(KEY, || eyre::bail!("damaged")).is_err());
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = cache.page(KEY, || panic!("decoder bug"));
        }));
        assert!(panicked.is_err());
        // Neither left the page marked as loading, which would hang this.
        assert_eq!(bytes(&cache.page(KEY, || page(1)).unwrap()), [1]);
    }

    #[test]
    fn a_cache_of_no_pages_decodes_every_time() {
        let cache = StoredPageCache::new(0);
        let loads = AtomicUsize::new(0);
        for _ in 0..2 {
            cache.page(KEY, || {
                loads.fetch_add(1, Ordering::Relaxed);
                page(1)
            }).unwrap();
        }
        assert_eq!(loads.load(Ordering::Relaxed), 2);
    }
}
