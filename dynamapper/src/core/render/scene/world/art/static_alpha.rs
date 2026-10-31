//! On-demand CPU alpha pages for textured object picking. No GPU uploads or
//! per-object RGBA copies; package replacement invalidates the bounded cache.
use bevy::{ecs::change_detection::Tick, prelude::*};
use std::collections::HashMap;

const MAX_ALPHA_BYTES: usize = 32 * 1024 * 1024;
const MAX_ALPHA_PAGES: usize = 128;

struct AlphaPage {
    width: usize,
    height: usize,
    alpha: Vec<u8>,
}

impl AlphaPage {
    fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let len = (width as usize).checked_mul(height as usize)?;
        if len == 0 || rgba.len() != len.checked_mul(4)? { return None; }
        Some(Self { width: width as usize, height: height as usize,
            alpha: rgba.chunks_exact(4).map(|pixel| pixel[3]).collect() })
    }

    fn sample(&self, pixel: Vec2) -> f32 {
        // GPU bilinear coordinates address texel centers at n + 0.5.
        let p = pixel - Vec2::splat(0.5);
        let base = p.floor();
        let weight = p - base;
        let at = |x: f32, y: f32| {
            let x = x.clamp(0.0, (self.width - 1) as f32) as usize;
            let y = y.clamp(0.0, (self.height - 1) as f32) as usize;
            f32::from(self.alpha[y * self.width + x]) / 255.0
        };
        let top = at(base.x, base.y) * (1.0 - weight.x) + at(base.x + 1.0, base.y) * weight.x;
        let bottom = at(base.x, base.y + 1.0) * (1.0 - weight.x) + at(base.x + 1.0, base.y + 1.0) * weight.x;
        top * (1.0 - weight.y) + bottom * weight.y
    }
}

#[derive(Resource, Default)]
pub(crate) struct StaticAlphaCache {
    revisions: [Option<Tick>; 2],
    pages: HashMap<(u8, u32), (u64, Option<AlphaPage>)>,
    bytes: usize,
    access: u64,
}

impl StaticAlphaCache {
    pub(crate) fn sync_packages(&mut self, revisions: [Option<Tick>; 2]) {
        if self.revisions != revisions {
            self.pages.clear();
            self.bytes = 0;
            self.revisions = revisions;
        }
    }

    pub(crate) fn sample(&mut self, key: (u8, u32), pixel: Vec2,
        load: impl FnOnce() -> Option<(u32, u32, Vec<u8>)>) -> Option<f32> {
        self.access = self.access.saturating_add(1);
        if !self.pages.contains_key(&key) {
            let page = load().and_then(|(w, h, rgba)| AlphaPage::from_rgba(w, h, rgba));
            let bytes = page.as_ref().map_or(0, |page| page.alpha.len());
            while !self.pages.is_empty() && (self.bytes + bytes > MAX_ALPHA_BYTES || self.pages.len() >= MAX_ALPHA_PAGES) {
                let oldest = *self.pages.iter().min_by_key(|(_, (access, _))| access).unwrap().0;
                let (_, removed) = self.pages.remove(&oldest).unwrap();
                self.bytes -= removed.as_ref().map_or(0, |page| page.alpha.len());
            }
            if page.is_none() { warn!("Cannot decode static picking alpha page {:?}", key); }
            self.bytes += bytes;
            self.pages.insert(key, (self.access, page));
        }
        let (access, page) = self.pages.get_mut(&key)?;
        *access = self.access;
        Some(page.as_ref()?.sample(pixel))
    }
}

/// Same nearest-interior/bilinear-seam filter as pixel_art_atlas_uv_iq.
pub(crate) fn sprite_sample_position(uv: Vec2, dx: Vec2, dy: Vec2, size: Vec2) -> Vec2 {
    let pixel = uv.clamp(Vec2::ZERO, Vec2::ONE) * size;
    let seam = (pixel + Vec2::splat(0.5)).floor();
    let width = ((dx.abs() + dy.abs()) * size).max(Vec2::splat(0.000001));
    seam + ((pixel - seam) / width).clamp(Vec2::splat(-0.5), Vec2::splat(0.5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_holes_and_fractional_edges_follow_the_sampled_alpha() {
        let page = AlphaPage::from_rgba(2, 2, vec![
            255, 0, 0, 0, 0, 255, 0, 255,
            0, 0, 255, 255, 0, 0, 0, 0,
        ]).unwrap();
        assert_eq!(page.sample(Vec2::splat(0.5)), 0.0);
        assert_eq!(page.sample(Vec2::new(1.5, 0.5)), 1.0);
        assert_eq!(page.sample(Vec2::ONE), 0.5);
        let sample = sprite_sample_position(Vec2::new(0.375, 0.125),
            Vec2::new(0.1, 0.0), Vec2::new(0.0, 0.1), Vec2::splat(2.0));
        assert_eq!(sample, Vec2::new(0.5, 0.5));
        assert!(AlphaPage::from_rgba(2, 2, vec![0; 4]).is_none());
    }

    #[test]
    fn alpha_pages_are_shared_and_package_replacement_invalidates_them() {
        let mut cache = StaticAlphaCache::default();
        cache.sync_packages([Some(Tick::new(1)), None]);
        assert_eq!(cache.sample((0, 2), Vec2::splat(0.5), || Some((1, 1, vec![0,0,0,255]))), Some(1.0));
        assert_eq!(cache.sample((0, 2), Vec2::splat(0.5), || panic!("cached page reloaded")), Some(1.0));
        cache.sync_packages([Some(Tick::new(2)), None]);
        assert_eq!(cache.sample((0, 2), Vec2::splat(0.5), || Some((1, 1, vec![255,255,255,0]))), Some(0.0));
        for page in 3..140 { cache.sample((0, page), Vec2::ZERO, || Some((1,1,vec![0;4]))); }
        assert_eq!(cache.pages.len(), MAX_ALPHA_PAGES);
        assert_eq!(cache.bytes, MAX_ALPHA_PAGES);
    }
}
