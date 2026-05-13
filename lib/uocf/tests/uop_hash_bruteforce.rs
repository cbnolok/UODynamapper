use uocf::uop_container::hash_bruteforce::*;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

// Example from C code: Hash=(0x)280F5FD7008898E6 | prefix=build/gumpartlegacymul/ | suffix=.tga | min_len=1 | max_len=8 | charset=0123456789 | result = build/gumpartlegacymul/00001283.tga
const TEST_HASH: u64 = 0x280F5FD7008898E6;
const TEST_PREFIX: &str = "build/gumpartlegacymul/";
const TEST_SUFFIX: &str = ".tga";
const TEST_CHARSET: &str = "0123456789";
const TEST_MIN_LEN: usize = 1;
const TEST_MAX_LEN: usize = 8;
const TEST_EXPECTED_RESULT: &str = "build/gumpartlegacymul/00001283.tga";

#[test]
#[ignore = "slow brute-force coverage; run explicitly with --ignored"]
fn test_bruteforce_hash_recursive() {
    let stop_signal: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let result: Option<String> = bruteforce_hash_recursive(
        TEST_HASH,
        TEST_PREFIX,
        TEST_SUFFIX,
        TEST_CHARSET,
        TEST_MIN_LEN,
        TEST_MAX_LEN,
        1, // Single thread for recursive test
        stop_signal,
    );
    assert_eq!(result, Some(TEST_EXPECTED_RESULT.to_string()));
}

#[test]
#[ignore = "slow brute-force coverage; run explicitly with --ignored"]
fn test_bruteforce_hash_simd() {
    let stop_signal: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let result: Option<String> = bruteforce_hash_simd(
        TEST_HASH,
        TEST_PREFIX,
        TEST_SUFFIX,
        TEST_CHARSET,
        TEST_MIN_LEN,
        TEST_MAX_LEN,
        4, // Use 4 threads for SIMD test
        stop_signal,
    );
    assert_eq!(result, Some(TEST_EXPECTED_RESULT.to_string()));
}

#[test]
#[ignore = "slow brute-force coverage; run explicitly with --ignored"]
fn test_bruteforce_hash_recursive_not_found() {
    let stop_signal: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let result: Option<String> = bruteforce_hash_recursive(
        0x1234567890ABCDEF, // A hash that should not be found
        TEST_PREFIX,
        TEST_SUFFIX,
        TEST_CHARSET,
        TEST_MIN_LEN,
        TEST_MAX_LEN,
        1,
        stop_signal,
    );
    assert_eq!(result, None);
}

#[test]
#[ignore = "slow brute-force coverage; run explicitly with --ignored"]
fn test_bruteforce_hash_simd_not_found() {
    let stop_signal: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let result: Option<String> = bruteforce_hash_simd(
        0x1234567890ABCDEF, // A hash that should not be found
        TEST_PREFIX,
        TEST_SUFFIX,
        TEST_CHARSET,
        TEST_MIN_LEN,
        TEST_MAX_LEN,
        4,
        stop_signal,
    );
    assert_eq!(result, None);
}
