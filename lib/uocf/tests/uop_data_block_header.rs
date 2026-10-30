//! Data block headers of UOP entries: parsing, serialization and the `data_block_hash` algorithms, on
//! synthetic headers and on every package of a real client.
//!
//! The client directories come from the variables in `common::CLIENT_DIR_ENVS`.

mod common;

use std::fs;
use std::time::{Duration, UNIX_EPOCH};

use uocf::uop_container::data_block_header::{
    filetime_from_system_time, DataBlockHashAlgorithm, DataBlockHeader, DataBlockHeaderRecord, DataBlockSignature,
    FILETIME_TICK_NANOSECONDS, FILETIME_UNIX_EPOCH, RECORD_TYPE_SIGNATURE, SIGNATURE_KEY_ID_CLASSIC,
    SIGNATURE_KEY_ID_ENHANCED,
};
use uocf::uop_container::package::{LoadMode, UopPackage};

/// A version-4 client timestamp header (`u16 3`, `u16 8`, FILETIME 2024-04-09) and its Adler-32, from the
/// reference release's `tileart.uop`.
const TIMESTAMP_HEADER: [u8; 12] = [0x03, 0x00, 0x08, 0x00, 0x60, 0x8a, 0x6a, 0xed, 0xbf, 0x8a, 0xda, 0x01];
const TIMESTAMP_HEADER_FILETIME: u64 = 0x01da_8abf_ed6a_8a60;
const TIMESTAMP_HEADER_HASH: u32 = 0x14b2_0471;
/// Unix time of `TIMESTAMP_HEADER_FILETIME`, and its sub-second FILETIME ticks.
const TIMESTAMP_HEADER_UNIX_SECONDS: u64 = 1_712_695_988;
const TIMESTAMP_HEADER_SUBSECOND_TICKS: u32 = 7_420_000;
/// Length of the synthetic signature value; its bytes are `(i * SIGNATURE_BYTE_STEP + SIGNATURE_BYTE_BASE)`.
const SIGNATURE_VALUE_LEN: usize = 128;
const SIGNATURE_BYTE_STEP: usize = 7;
const SIGNATURE_BYTE_BASE: usize = 3;
/// lookup3 `hashlittle` (initial value 0) of the synthetic signature header, computed by an independent
/// implementation (`scripts/dev-helpers/ec-research/ghidra-uop-header/verify_uop_headers.py`).
const SIGNATURE_HEADER_HASH: u32 = 0x7ed1_ef50;
/// Signature value lengths seen in client packages: a 1025/1026-bit integer without leading zero bytes.
const SIGNATURE_VALUE_LEN_RANGE: std::ops::RangeInclusive<usize> = 126..=129;
/// A record type no client package uses.
const UNUSED_RECORD_TYPE: u16 = 9;
/// Package file extension.
const UOP_EXTENSION: &str = "uop";

fn synthetic_signature_header() -> Vec<u8> {
    let value: Vec<u8> =
        (0..SIGNATURE_VALUE_LEN).map(|i| ((i * SIGNATURE_BYTE_STEP + SIGNATURE_BYTE_BASE) & 0xff) as u8).collect();
    let mut data = Vec::new();
    data.extend_from_slice(&SIGNATURE_KEY_ID_ENHANCED.to_be_bytes());
    data.extend_from_slice(&(value.len() as u16).to_be_bytes());
    data.extend_from_slice(&value);
    let mut header = Vec::new();
    header.extend_from_slice(&RECORD_TYPE_SIGNATURE.to_le_bytes());
    header.extend_from_slice(&(data.len() as u16).to_le_bytes());
    header.extend_from_slice(&data);
    header
}

#[test]
fn timestamp_header_parses_and_hashes_with_adler32() {
    let header = DataBlockHeader::parse(&TIMESTAMP_HEADER).expect("parse timestamp header");
    assert_eq!(header.records(), &[DataBlockHeaderRecord::Timestamp(TIMESTAMP_HEADER_FILETIME)]);
    assert_eq!(header.timestamp(), Some(TIMESTAMP_HEADER_FILETIME));
    assert_eq!(header.to_bytes(), TIMESTAMP_HEADER);
    assert_eq!(header.hash_algorithm(), Some(DataBlockHashAlgorithm::Adler32));
    assert_eq!(header.data_block_hash().unwrap(), Some(TIMESTAMP_HEADER_HASH));
}

#[test]
fn timestamp_headers_are_built_from_system_time() {
    assert_eq!(filetime_from_system_time(UNIX_EPOCH), FILETIME_UNIX_EPOCH);
    let subsecond_nanoseconds = TIMESTAMP_HEADER_SUBSECOND_TICKS * FILETIME_TICK_NANOSECONDS;
    let time = UNIX_EPOCH + Duration::new(TIMESTAMP_HEADER_UNIX_SECONDS, subsecond_nanoseconds);
    assert_eq!(filetime_from_system_time(time), TIMESTAMP_HEADER_FILETIME);
    assert!(filetime_from_system_time(UNIX_EPOCH - Duration::from_secs(1)) < FILETIME_UNIX_EPOCH);

    let header = DataBlockHeader::from_timestamp(TIMESTAMP_HEADER_FILETIME);
    assert_eq!(header.to_bytes(), TIMESTAMP_HEADER);
    assert_eq!(header.data_block_hash().unwrap(), Some(TIMESTAMP_HEADER_HASH));
}

#[test]
fn signature_header_parses_and_hashes_with_lookup3() {
    let bytes = synthetic_signature_header();
    let header = DataBlockHeader::parse(&bytes).expect("parse signature header");
    let signature = header.signature().expect("signature record");
    assert_eq!(signature.key_id, SIGNATURE_KEY_ID_ENHANCED);
    assert_eq!(signature.value.len(), SIGNATURE_VALUE_LEN);
    assert_eq!(header.to_bytes(), bytes);
    assert_eq!(header.hash_algorithm(), Some(DataBlockHashAlgorithm::Lookup3));
    assert_eq!(header.data_block_hash().unwrap(), Some(SIGNATURE_HEADER_HASH));
}

#[test]
fn malformed_and_unknown_records_are_kept_or_rejected() {
    let mut truncated = synthetic_signature_header();
    truncated.pop();
    assert!(DataBlockHeader::parse(&truncated).is_err());
    assert!(DataBlockHeader::parse(&TIMESTAMP_HEADER[..TIMESTAMP_HEADER.len() - 1]).is_err());

    // A signature record whose inner length disagrees with its data stays an unknown record, byte for byte.
    let mut bad_inner_length = synthetic_signature_header();
    bad_inner_length[7] ^= 1;
    let header = DataBlockHeader::parse(&bad_inner_length).expect("parse");
    assert!(matches!(header.records(), [DataBlockHeaderRecord::Unknown { record_type: RECORD_TYPE_SIGNATURE, .. }]));
    assert_eq!(header.to_bytes(), bad_inner_length);

    let unknown = DataBlockHeader::new(vec![DataBlockHeaderRecord::Unknown {
        record_type: UNUSED_RECORD_TYPE,
        data: vec![1, 2, 3],
    }]);
    assert_eq!(DataBlockHeader::parse(&unknown.to_bytes()).unwrap(), unknown);
    assert_eq!(unknown.data_block_hash().unwrap(), None);
    assert_eq!(DataBlockHeader::parse(&[]).unwrap().data_block_hash().unwrap(), None);

    let built = DataBlockHeader::new(vec![DataBlockHeaderRecord::Signature(DataBlockSignature {
        key_id: SIGNATURE_KEY_ID_CLASSIC,
        value: vec![0x80; SIGNATURE_VALUE_LEN],
    })]);
    assert_eq!(DataBlockHeader::parse(&built.to_bytes()).unwrap(), built);
}

/// Every entry of every package of the client directories: the header parses into known records, serializes
/// back to the same bytes and its hash is the stored `data_block_hash`.
#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn client_package_headers_match_their_data_block_hash() {
    let mut checked_any = false;
    for env in common::CLIENT_DIR_ENVS {
        let Some(dir) = std::env::var_os(env) else { continue };
        let mut paths: Vec<_> = fs::read_dir(&dir)
            .expect("read client directory")
            .map(|entry| entry.expect("directory entry").path())
            .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(UOP_EXTENSION)))
            .collect();
        paths.sort();
        let (mut entries, mut matched) = (0usize, 0usize);
        for path in paths {
            let package = UopPackage::load_with_mode(&path, LoadMode::Eager).expect("load package");
            for file in package.iter_files() {
                let Some(bytes) = file.data_block_header() else { continue };
                entries += 1;
                let header = DataBlockHeader::parse(bytes).expect("parse header");
                assert_eq!(header.to_bytes(), bytes.as_ref(), "{}: header round trip", path.display());
                assert!(
                    !header.records().iter().any(|record| matches!(record, DataBlockHeaderRecord::Unknown { .. })),
                    "{}: unknown record",
                    path.display()
                );
                if let Some(signature) = header.signature() {
                    assert!(
                        [SIGNATURE_KEY_ID_ENHANCED, SIGNATURE_KEY_ID_CLASSIC].contains(&signature.key_id),
                        "{}: key id {}",
                        path.display(),
                        signature.key_id
                    );
                    assert!(SIGNATURE_VALUE_LEN_RANGE.contains(&signature.value.len()), "{}", path.display());
                }
                if file.data_block_hash_matches_header().expect("hash") == Some(true) {
                    matched += 1;
                }
            }
        }
        println!("{env}: {matched} of {entries} entry headers match their data_block_hash");
        assert_eq!(matched, entries, "{env}: data_block_hash mismatches");
        checked_any |= entries > 0;
    }
    assert!(checked_any, "set {} to a client directory", common::CLIENT_DIR_ENVS.join(" and/or "));
}
