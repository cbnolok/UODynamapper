use crate::common;

use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::{
    file::CompressionFlag,
    package::{FileReplacement, UopPackage},
};
use std::{
    fs,
    path::PathBuf,
    sync::{Mutex, Once, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

struct TestLogger {
    messages: Mutex<Vec<String>>,
}

impl TestLogger {
    fn clear(&self) {
        self.messages.lock().unwrap().clear();
    }

    fn snapshot(&self) -> Vec<String> {
        self.messages.lock().unwrap().clone()
    }
}

impl log::Log for TestLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.messages
                .lock()
                .unwrap()
                .push(format!("{}", record.args()));
        }
    }

    fn flush(&self) {}
}

fn test_logger() -> &'static TestLogger {
    static LOGGER: OnceLock<TestLogger> = OnceLock::new();
    static INIT: Once = Once::new();

    let logger = LOGGER.get_or_init(|| TestLogger {
        messages: Mutex::new(Vec::new()),
    });

    INIT.call_once(|| {
        log::set_max_level(log::LevelFilter::Warn);
        let _ = log::set_logger(logger);
    });

    logger
}

fn logger_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn serialize_dictionary_payload(declared_count: u32, strings: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x0123_4567_89AB_CDEFu64.to_le_bytes());
    bytes.extend_from_slice(&declared_count.to_le_bytes());
    bytes.extend_from_slice(&0x48u32.to_le_bytes());
    for string in strings {
        let string_bytes = string.as_bytes();
        bytes.extend_from_slice(&(string_bytes.len() as u16).to_le_bytes());
        bytes.extend_from_slice(string_bytes);
    }
    bytes
}

fn temp_uop_path(test_name: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("uocf_{test_name}_{timestamp}.uop"))
}

fn write_dictionary_uop(test_name: &str, payload: &[u8]) -> PathBuf {
    let path = temp_uop_path(test_name);
    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(
            payload,
            "build/string_dictionary.bin",
            CompressionFlag::Zlib,
        )
        .expect("add string dictionary payload");
    package.finalize_and_save(&path).expect("save package");
    path
}

#[test]
fn string_dictionary_warns_and_clamps_when_declared_count_runs_past_eof() {
    let _guard = logger_lock().lock().unwrap();
    let logger = test_logger();
    logger.clear();

    let payload = serialize_dictionary_payload(3, &["alpha", "beta"]);
    let dictionary = UoStringDictionary::from_payload_bytes(&payload, "test-dictionary")
        .expect("parse dictionary with eof mismatch");

    assert_eq!(dictionary.get_string(0), Some("alpha"));
    assert_eq!(dictionary.get_string(1), Some("beta"));

    let messages = logger.snapshot();
    assert!(messages.iter().any(|message| {
        message.contains("test-dictionary")
            && message.contains("declared 3 strings")
            && message.contains("EOF after 2 complete entries")
    }));
}

#[test]
fn string_dictionary_rejects_truncated_string_payload() {
    let mut payload = serialize_dictionary_payload(1, &["alpha"]);
    payload.pop();

    let error = UoStringDictionary::from_payload_bytes(&payload, "truncated-dictionary")
        .expect_err("truncated string body must fail");

    assert!(error
        .to_string()
        .contains("failed to fill whole buffer"));
}

#[test]
fn string_dictionary_load_reads_single_nonzero_hash_payload_package() {
    let payload = serialize_dictionary_payload(2, &["build/tileart/00000001.dat", "UOSpriteShader"]);
    let path = write_dictionary_uop("string_dictionary_single_entry", &payload);

    let dictionary = UoStringDictionary::load(&path).expect("load dictionary package");

    assert_eq!(dictionary.get_string(0), Some("build/tileart/00000001.dat"));
    assert_eq!(dictionary.get_string(1), Some("UOSpriteShader"));

    let _ = fs::remove_file(path);
}

/// The client's string dictionary package and its one entry.
const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";
const STRING_DICTIONARY_ENTRY: &str = "build/stringdictionary/string_dictionary.bin";

/// The client dictionary keeps its declared count, which exceeds the stored strings, and writes its payload back
/// byte for byte; replacing the entry with that payload leaves it untouched.
#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn client_string_dictionary_writes_back_byte_for_byte() {
    for (env, path) in common::client_files(STRING_DICTIONARY_UOP_NAME) {
        let mut package = UopPackage::load(&path).expect("load string dictionary package");
        let hash = hash_file_name_single(STRING_DICTIONARY_ENTRY);
        let file = package.get_file_by_hash(hash).expect("string dictionary entry");
        let compression = file.compression();
        let payload = file.unpack().expect("unpack string dictionary");
        let dictionary = UoStringDictionary::from_payload_bytes(&payload, &path.display().to_string())
            .expect("parse string dictionary");
        let written = dictionary.to_payload_bytes().expect("write string dictionary");
        assert!(written == payload, "{env}: the written payload differs from the client's");
        assert_eq!(
            package.replace_file(hash, &written, compression).expect("replace"),
            FileReplacement::Unchanged,
            "{env}"
        );
        assert_eq!(dictionary.len(), UoStringDictionary::load(&path).expect("load").len(), "{env}");
        println!(
            "{env}: {} strings declared, {} stored; the {}-byte payload is written back byte for byte",
            dictionary.declared_count(),
            dictionary.len(),
            payload.len()
        );
    }
}
