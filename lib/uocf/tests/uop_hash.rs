use uocf::uop_container::hash::*;
use rand::{Rng, SeedableRng};

#[test]
fn matches_known_uop_reference() {
    assert_eq!(
        hash_file_name_single("build/gumpartlegacymul/00001283.tga"),
        0x280F5FD7008898E6
    );
}

#[test]
fn scalar_and_simd_agree_small() {
    let cases = [
        "",
        "a",
        "file.txt",
        "longer_filename_1234567890.bin",
        "another_file_😀.dat",
        "short",
        "a_very_long_test_filename_with_many_parts_and_numbers_0123456789.uop",
    ];
    let all: Vec<String> = cases.iter().cycle().take(32).map(|s| s.to_string()).collect();
    let refs: Vec<u64> = all.iter().map(|s| hash_file_name_single(s)).collect();
    let s_refs: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
    let simd_hashes = hash_file_name_simd_batch_strs(&s_refs);
    assert_eq!(refs, simd_hashes);
}

#[test]
fn simd_batch_can_write_into_caller_buffer() {
    let samples: Vec<String> = (0..48)
        .map(|index| format!("build/worldart/{index:08}.dds"))
        .collect();
    let refs: Vec<u64> = samples.iter().map(|s| hash_file_name_single(s)).collect();
    let s_refs: Vec<&str> = samples.iter().map(|s| s.as_str()).collect();
    let mut hashes = vec![u64::MAX; s_refs.len()];

    hash_file_name_simd_batch_strs_into(&s_refs, &mut hashes);

    assert_eq!(refs, hashes);
}

#[test]
fn randomized_matches() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x12345678);
    let mut samples: Vec<String> = Vec::new();
    for _ in 0..256 {
        let len = rng.gen_range(0..64);
        let s: String = (0..len).map(|_| (rng.gen_range(32..127) as u8) as char).collect();
        samples.push(s);
    }
    let refs: Vec<u64> = samples.iter().map(|s| hash_file_name_single(s)).collect();
    let s_refs: Vec<&str> = samples.iter().map(|s| s.as_str()).collect();
    let simd_hashes = hash_file_name_simd_batch_strs(&s_refs);
    assert_eq!(refs, simd_hashes);
}

#[test]
fn same_width_batches_match() {
    let samples: Vec<String> = (0..32)
        .map(|index| format!("build/map{:04}.mul", index))
        .collect();
    let refs: Vec<u64> = samples.iter().map(|s| hash_file_name_single(s)).collect();
    let s_refs: Vec<&str> = samples.iter().map(|s| s.as_str()).collect();
    assert_eq!(refs, hash_file_name_simd_batch_strs(&s_refs));
}

#[test]
fn mixed_tail_batches_match() {
    let samples: Vec<String> = vec![
        "build/map0.mul".into(),
        "build/map00.mul".into(),
        "build/map000.mul".into(),
        "build/map0000.mul".into(),
        "build/map00000.mul".into(),
        "build/map000000.mul".into(),
        "build/map0000000.mul".into(),
        "build/map00000000.mul".into(),
        "build/map000000000.mul".into(),
        "build/map0000000000.mul".into(),
        "build/map00000000000.mul".into(),
        "build/map000000000000.mul".into(),
    ];
    let refs: Vec<u64> = samples.iter().map(|s| hash_file_name_single(s)).collect();
    let s_refs: Vec<&str> = samples.iter().map(|s| s.as_str()).collect();
    assert_eq!(refs, hash_file_name_simd_batch_strs(&s_refs));
}
