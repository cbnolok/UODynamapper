//! Template-based brute-forcing for UOP hashes.
//!
//! This module provides utilities to generate candidate strings based on templates
//! and check them against a set of target hashes in parallel.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use rayon::prelude::*;
use crate::uop_container::hash;

/// A template for generating candidate strings.
/// Supports placeholders like `{:08}` or `{:d}`.
pub struct UopTemplate {
    pub template: String,
    pub range: Option<std::ops::RangeInclusive<u64>>,
}

impl UopTemplate {
    pub fn new(template: impl Into<String>, range: Option<std::ops::RangeInclusive<u64>>) -> Self {
        Self {
            template: template.into(),
            range,
        }
    }

    /// Checks if the template contains any known placeholders.
    pub fn has_placeholders(&self) -> bool {
        self.template.contains("{:08}") ||
        self.template.contains("{:07}") ||
        self.template.contains("{:06}") ||
        self.template.contains("{:05}") ||
        self.template.contains("{:04}") ||
        self.template.contains("{:d}")
    }

    /// Infers the maximum range value based on the placeholders in the template.
    pub fn infer_max_range(&self) -> u64 {
        if self.template.contains("{:08}") {
            99_999_999
        } else if self.template.contains("{:07}") {
            9_999_999
        } else if self.template.contains("{:06}") {
            999_999
        } else if self.template.contains("{:05}") {
            99_999
        } else if self.template.contains("{:04}") {
            9_999
        } else if self.template.contains("{:d}") {
            1_000_000 // Reasonable default for variable length if not specified
        } else {
            0
        }
    }

    /// Brute-force the template against a set of hashes.
    /// Returns a map of found hashes to their original strings.
    pub fn crack(
        &self,
        target_hashes: &HashSet<u64>,
        stop_signal: &Arc<AtomicBool>,
    ) -> HashMap<u64, String> {
        if !self.has_placeholders() {
            // Static string, check once
            let hash = hash::hash_file_name_single(&self.template);
            if target_hashes.contains(&hash) {
                let mut found = HashMap::new();
                found.insert(hash, self.template.clone());
                return found;
            }
            return HashMap::new();
        }

        // Determine range: explicit or inferred
        let range = self.range.clone().unwrap_or_else(|| {
            0..=self.infer_max_range()
        });
        
        let start = *range.start();
        let end = *range.end();
        let count = if end >= start { (end - start) + 1 } else { 0 };
        
        let chunk_size = 1024;
        
        // Use usize for Rayon's IndexedParallelIterator support
        (0..count as usize).into_par_iter()
            .chunks(chunk_size)
            .filter_map(|chunk: Vec<usize>| {
                if stop_signal.load(Ordering::Relaxed) {
                    return None;
                }

                let mut candidates = Vec::with_capacity(chunk.len());
                for offset in chunk {
                    let i = start + offset as u64;
                    let candidate = self.template.replace("{:08}", &format!("{:08}", i))
                        .replace("{:07}", &format!("{:07}", i))
                        .replace("{:06}", &format!("{:06}", i))
                        .replace("{:05}", &format!("{:05}", i))
                        .replace("{:04}", &format!("{:04}", i))
                        .replace("{:d}", &format!("{}", i));
                    candidates.push(candidate);
                }

                let candidate_strs: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
                let hashes = hash::hash_file_name_simd_batch_strs(&candidate_strs);
                
                let mut found = HashMap::new();
                for (j, &h) in hashes.iter().enumerate() {
                    if target_hashes.contains(&h) {
                        found.insert(h, candidates[j].clone());
                    }
                }
                
                if found.is_empty() {
                    None
                } else {
                    Some(found)
                }
            })
            .reduce(HashMap::new, |mut acc: HashMap<u64, String>, map| {
                acc.extend(map);
                acc
            })
    }
}
