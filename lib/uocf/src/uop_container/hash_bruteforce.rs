//! Brute-force implementation to find a string that matches a given UOP hash.

use crate::uop_container::hash::{hash_file_name_single, hash_file_name_simd_batch_strs};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// --- Recursive Implementation ---

/// Attempts to find the original string that produces the given hash by brute-force using a recursive approach.
/// This function is parallelized and will use the specified number of threads.
///
/// # Arguments
///
/// * `hash` - The target hash value.
/// * `prefix` - A known prefix of the string.
/// * `suffix` - A known suffix of the string.
/// * `charset` - The set of characters to use for the variable part of the string.
/// * `min_len` - The minimum length of the variable part.
/// * `max_len` - The maximum length of the variable part.
/// * `num_threads` - The number of threads to use for the search.
/// * `stop_signal` - An atomic boolean that can be used to stop the search prematurely.
///
/// # Returns
///
/// An `Option<String>` containing the full string if found, otherwise `None`.
pub fn bruteforce_hash_recursive(
    hash: u64,
    prefix: &str,
    suffix: &str,
    charset: &str,
    min_len: usize,
    max_len: usize,
    num_threads: usize,
    stop_signal: Arc<AtomicBool>,
) -> Option<String> {
    if num_threads > 0 {
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build_global()
            ;
    }

    let prefix_bytes: &[u8] = prefix.as_bytes();
    let suffix_bytes: &[u8] = suffix.as_bytes();
    let charset_bytes: &[u8] = charset.as_bytes();

    for len in min_len..=max_len {
        if stop_signal.load(Ordering::Relaxed) {
            return None;
        }

        let result = (0..charset_bytes.len()).into_par_iter().find_map_any(|i| {
            if stop_signal.load(Ordering::Relaxed) {
                return None;
            }

            let mut candidate_vec =
                Vec::with_capacity(prefix_bytes.len() + len + suffix_bytes.len());
            candidate_vec.extend_from_slice(prefix_bytes);
            candidate_vec.resize(prefix_bytes.len() + len, 0);
            candidate_vec.extend_from_slice(suffix_bytes);

            let variable_part = &mut candidate_vec[prefix_bytes.len()..prefix_bytes.len() + len];
            variable_part[0] = charset_bytes[i];

            if len == 1 {
                let s = std::str::from_utf8(&candidate_vec).unwrap();
                if hash_file_name_single(s) == hash {
                    stop_signal.store(true, Ordering::Relaxed);
                    return Some(String::from_utf8(candidate_vec).unwrap());
                }
            } else if crack_recurse_recursive(
                hash,
                &mut candidate_vec,
                prefix_bytes.len() + 1,
                prefix_bytes.len() + len,
                charset_bytes,
                &stop_signal,
            ) {
                stop_signal.store(true, Ordering::Relaxed);
                return Some(String::from_utf8(candidate_vec).unwrap());
            }
            None
        });

        if let Some(found) = result {
            return Some(found);
        }
    }
    None
}

fn crack_recurse_recursive(
    hash: u64,
    candidate_vec: &mut [u8],
    start_offset: usize,
    end_offset: usize,
    charset: &[u8],
    stop_signal: &Arc<AtomicBool>,
) -> bool {
    if start_offset == end_offset {
        let s = std::str::from_utf8(candidate_vec).unwrap();
        return hash_file_name_single(s) == hash;
    }

    for (i, &char_code) in charset.iter().enumerate() {
        if i % 100 == 0 && stop_signal.load(Ordering::Relaxed) {
            return false;
        }

        candidate_vec[start_offset] = char_code;
        if crack_recurse_recursive(
            hash,
            candidate_vec,
            start_offset + 1,
            end_offset,
            charset,
            stop_signal,
        ) {
            return true;
        }
    }

    false
}

// --- SIMD Implementation ---

/// Attempts to find the original string that produces the given hash by brute-force using an iterative, batch-based, SIMD-accelerated approach.
/// This function is parallelized and will use the specified number of threads.
///
/// # Arguments
///
/// * `hash` - The target hash value.
/// * `prefix` - A known prefix of the string.
/// * `suffix` - A known suffix of the string.
/// * `charset` - The set of characters to use for the variable part of the string.
/// * `min_len` - The minimum length of the variable part.
/// * `max_len` - The maximum length of the variable part.
/// * `num_threads` - The number of threads to use for the search.
/// * `stop_signal` - An atomic boolean that can be used to stop the search prematurely.
///
/// # Returns
///
/// An `Option<String>` containing the full string if found, otherwise `None`.
pub fn bruteforce_hash_simd(
    hash: u64,
    prefix: &str,
    suffix: &str,
    charset: &str,
    min_len: usize,
    max_len: usize,
    num_threads: usize,
    stop_signal: Arc<AtomicBool>,
) -> Option<String> {
    if num_threads > 0 {
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build_global()
            ;
    }

    for len in min_len..=max_len {
        if stop_signal.load(Ordering::Relaxed) {
            return None;
        }

        let result = (0..charset.len())
            .into_par_iter()
            .find_map_any(|i| {
                let mut generator = CandidateGenerator::new(len, charset, prefix, suffix, i);
                while let Some(batch) = generator.next_batch(1024) {
                    if stop_signal.load(Ordering::Relaxed) {
                        return None;
                    }
                    let batch_strs: Vec<&str> = batch.iter().map(|s| s.as_str()).collect();
                    let hashes = hash_file_name_simd_batch_strs(&batch_strs);
                    for (j, &h) in hashes.iter().enumerate() {
                        if h == hash {
                            stop_signal.store(true, Ordering::Relaxed);
                            return Some(batch[j].clone());
                        }
                    }
                }
                None
            });

        if let Some(found) = result {
            return Some(found);
        }
    }

    None
}

struct CandidateGenerator<'a> {
    len: usize,
    charset: &'a str,
    prefix: &'a str,
    suffix: &'a str,
    indices: Vec<usize>,
    exhausted: bool,
}

impl<'a> CandidateGenerator<'a> {
    fn new(len: usize, charset: &'a str, prefix: &'a str, suffix: &'a str, first_char_index: usize) -> Self {
        let mut indices = vec![0; len];
        if len > 0 {
            indices[0] = first_char_index;
        }
        Self {
            len,
            charset,
            prefix,
            suffix,
            indices,
            exhausted: len == 0,
        }
    }

    fn next_batch(&mut self, batch_size: usize) -> Option<Vec<String>> {
        if self.exhausted {
            return None;
        }

        let mut batch = Vec::with_capacity(batch_size);
        let charset_bytes = self.charset.as_bytes();

        for _ in 0..batch_size {
            let mut candidate = String::with_capacity(self.prefix.len() + self.len + self.suffix.len());
            candidate.push_str(self.prefix);
            for &index in &self.indices {
                candidate.push(charset_bytes[index] as char);
            }
            candidate.push_str(self.suffix);
            batch.push(candidate);

            // Increment indices
            if self.len == 1 {
                self.exhausted = true;
                return Some(batch);
            }

            let mut i = self.len - 1;
            loop {
                self.indices[i] += 1;
                if self.indices[i] < self.charset.len() {
                    break;
                }
                self.indices[i] = 0;
                if i == 1 { // We only iterate from the second character
                    self.exhausted = true;
                    return Some(batch);
                }
                i -= 1;
            }
        }

        Some(batch)
    }
}

