//! The longest prefix of a frame list that fits an atlas page, with the per-frame pack candidates computed only as
//! far as the search reaches. A page holds a few thousand frames of a list of up to a million, and computing every
//! remaining frame's candidate for every page and page size made the bucket search the largest stage of a mobile
//! pack.

use rayon::prelude::*;

/// The items of a prefix of `items`, mapped by `map` as they are first needed. The prefix ends at the first item
/// `map` refuses (or at the end of `items`).
pub(crate) struct LazyPrefix<'a, Item, Mapped, Map> {
    items: &'a [Item],
    map: Map,
    mapped: Vec<Mapped>,
    ended: bool,
}

impl<'a, Item, Mapped, Map: FnMut(&Item) -> Option<Mapped>> LazyPrefix<'a, Item, Mapped, Map> {
    pub(crate) fn new(items: &'a [Item], map: Map) -> Self {
        Self { items, map, mapped: Vec::new(), ended: false }
    }

    /// The first `count` mapped items, or all of them when the prefix is shorter.
    pub(crate) fn load(&mut self, count: usize) -> &[Mapped] {
        while !self.ended && self.mapped.len() < count {
            match self.items.get(self.mapped.len()).and_then(&mut self.map) {
                Some(mapped) => self.mapped.push(mapped),
                None => self.ended = true,
            }
        }
        &self.mapped[..count.min(self.mapped.len())]
    }

    /// Whether the prefix has exactly `count` items (it has at least `count`).
    fn ends_at(&mut self, count: usize) -> bool {
        self.load(count + 1).len() == count
    }
}

/// Doubling probes evaluated together. The search over the whole list probes 1, 2, 4, ... in order; the probes of
/// a group run in parallel and are read in that order, so the result is the same.
const SPECULATIVE_DOUBLING_PROBES: usize = 4;
/// Bisection levels evaluated together: every probe the bisection could make in the next levels runs in parallel,
/// then the bisection follows its path through them.
const SPECULATIVE_BISECTION_LEVELS: u32 = 2;
/// Growth factor of the doubling probes.
const GROWTH: usize = 2;

/// The length of the longest prefix of `prefix` that `fits`: doubling from 1 until a prefix does not fit or the
/// whole prefix does, then bisecting. Gives the result of that search over the fully mapped list; `fits` must be a
/// pure function of the prefix, since some probes are evaluated speculatively.
pub(crate) fn longest_fitting_prefix<Item, Mapped, Map>(
    prefix: &mut LazyPrefix<'_, Item, Mapped, Map>,
    fits: impl Fn(&[Mapped]) -> bool + Sync,
) -> usize
where
    Mapped: Sync,
    Map: FnMut(&Item) -> Option<Mapped>,
{
    let mut best = 0usize;
    let mut high = 1usize;
    'doubling: loop {
        // The next probe sizes, as the doubling would visit them, and whether the last one is the whole prefix.
        let mut sizes = Vec::with_capacity(SPECULATIVE_DOUBLING_PROBES);
        let mut size = high;
        let mut ended = false;
        while sizes.len() < SPECULATIVE_DOUBLING_PROBES {
            if prefix.load(size).len() < size {
                break;
            }
            sizes.push(size);
            if prefix.ends_at(size) {
                ended = true;
                break;
            }
            size = prefix.load(size.saturating_mul(GROWTH)).len();
        }
        if sizes.is_empty() {
            break;
        }
        let loaded = prefix.load(sizes[sizes.len() - 1]);
        let results: Vec<bool> = sizes.par_iter().map(|&size| fits(&loaded[..size])).collect();
        for (&size, fit) in sizes.iter().zip(results) {
            if !fit {
                high = size;
                break 'doubling;
            }
            best = size;
        }
        if ended {
            return best;
        }
        high = size;
    }
    if best == 0 {
        return 0;
    }

    let mut low = best + 1;
    let mut high = high.saturating_sub(1);
    let loaded = prefix.load(high);
    while low <= high {
        let mut probes = Vec::new();
        bisection_probes(low, high, SPECULATIVE_BISECTION_LEVELS, &mut probes);
        let results: Vec<(usize, bool)> = probes.par_iter().map(|&mid| (mid, fits(&loaded[..mid]))).collect();
        for _ in 0..SPECULATIVE_BISECTION_LEVELS {
            if low > high {
                break;
            }
            let mid = low + (high - low) / 2;
            let fit = results
                .iter()
                .find_map(|&(probe, fit)| (probe == mid).then_some(fit))
                .expect("every bisection probe of the levels was evaluated");
            if fit {
                best = mid;
                low = mid + 1;
            } else {
                high = mid.saturating_sub(1);
            }
        }
    }
    best
}

/// Every probe a bisection of `low..=high` makes in its next `levels` levels.
fn bisection_probes(low: usize, high: usize, levels: u32, probes: &mut Vec<usize>) {
    if levels == 0 || low > high {
        return;
    }
    let mid = low + (high - low) / 2;
    probes.push(mid);
    bisection_probes(low, mid.saturating_sub(1), levels - 1, probes);
    bisection_probes(mid + 1, high, levels - 1, probes);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// The search over a fully mapped list, as the packers did it.
    fn eager(mapped: &[u32], fits: impl Fn(&[u32]) -> bool) -> usize {
        let mut best = 0usize;
        let mut high = 1usize;
        while high <= mapped.len() {
            if fits(&mapped[..high]) {
                best = high;
                if high == mapped.len() {
                    return best;
                }
                high = high.saturating_mul(2).min(mapped.len());
            } else {
                break;
            }
        }
        if best == 0 {
            return 0;
        }
        let mut low = best + 1;
        let mut high = high.saturating_sub(1);
        while low <= high {
            let mid = low + (high - low) / 2;
            if fits(&mapped[..mid]) {
                best = mid;
                low = mid + 1;
            } else {
                high = mid.saturating_sub(1);
            }
        }
        best
    }

    #[test]
    fn matches_the_search_over_the_whole_list() {
        const MAX_ITEMS: u32 = 70;
        const REFUSED: u32 = 0;
        const STEP: u32 = 7;
        const MODULUS: u32 = 11;
        // Items of 0 are refused by the map; a prefix fits while its sum stays under a capacity.
        for count in 0..MAX_ITEMS {
            let items: Vec<u32> = (0..count).map(|i| (i * STEP + count) % MODULUS).collect();
            for capacity in 0..count * MODULUS {
                let fits = |prefix: &[u32]| prefix.iter().sum::<u32>() <= capacity;
                let mapped: Vec<u32> = items.iter().copied().take_while(|&item| item != REFUSED).collect();
                let calls = AtomicUsize::new(0);
                let mut lazy = LazyPrefix::new(&items, |&item: &u32| (item != REFUSED).then_some(item));
                let found = longest_fitting_prefix(&mut lazy, |prefix| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    fits(prefix)
                });
                let calls = calls.into_inner();
                assert_eq!(found, eager(&mapped, fits), "{count} items, capacity {capacity}");
                let speculative_growth = GROWTH.pow(SPECULATIVE_DOUBLING_PROBES as u32);
                let bound = (found * speculative_growth).max(speculative_growth) + 1;
                assert!(lazy.mapped.len() <= bound, "{count} items: mapped {}", lazy.mapped.len());
                assert!(calls > 0 || mapped.is_empty());
            }
        }
    }
}
