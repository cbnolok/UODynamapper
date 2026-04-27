///! Represents a string dictionary from a UOP file.

use std::{io::Read, path::Path};
use byteorder::{LittleEndian, ReadBytesExt};
use crate::uop::package::UopPackage;
crate::eyre_imports!();

/// Represents a string dictionary from a UOP file.
#[allow(dead_code)]
#[derive(Debug)]
pub struct UoStringDictionary {
    /// Unknown value.
    unk1: u64,
    /// The number of strings in the dictionary.
    strings_count: u32,
    /// Unknown value.
    unk2: u32,
    /// The strings in the dictionary.
    strings: Vec<String>,
}

impl UoStringDictionary {
    /// Loads a `UoStringDictionary` from a UOP file.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the UOP file.
    pub fn load(path: &Path) -> eyre::Result<Self> {
        let package = UopPackage::load(path)?;
        let file = if let Some(file) = package.get_file_by_hash(0) {
            file
        } else {
            let mut candidate_files = package.iter_files().filter(|file| file.has_size());
            let first_file = candidate_files
                .next()
                .ok_or_else(|| eyre::eyre!("File not found in package"))?;

            if let Some(second_file) = candidate_files.next() {
                return Err(eyre::eyre!(
                    "string dictionary package has no hash-0 entry and multiple payloads ({:#018x}, {:#018x}, ...)",
                    first_file.filename_hash(),
                    second_file.filename_hash(),
                ));
            }

            first_file
        };

        // Extracting decompressed payload using zero-copy extraction to a scratch buffer is optimal,
        // but since `unpack()` handles its own z-lib, we take the whole array
        let decompressed_data: Vec<u8> = file.unpack()?;
        Self::from_payload_bytes(&decompressed_data, &path.display().to_string())
    }

    fn from_payload_bytes(bytes: &[u8], source_label: &str) -> eyre::Result<Self> {
        let mut reader = std::io::Cursor::new(bytes);

        let unk1: u64 = reader.read_u64::<LittleEndian>()?;
        let declared_strings_count: u32 = reader.read_u32::<LittleEndian>()?;
        let unk2: u32 = reader.read_u32::<LittleEndian>()?;

        // Allocate string vector capacities in advance
        let mut strings: Vec<String> = Vec::with_capacity(declared_strings_count as usize);
        let mut string_buffer = Vec::new();
        let mut count_eof_mismatch = false;

        for _ in 0..declared_strings_count {
            if reader.position() as usize >= reader.get_ref().len() {
                count_eof_mismatch = true;
                break;
            }

            let string_len: usize = match reader.read_u16::<LittleEndian>() {
                Ok(value) => value as usize,
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    count_eof_mismatch = true;
                    break;
                }
                Err(error) => return Err(error.into()),
            };

            string_buffer.resize(string_len, 0);
            reader.read_exact(&mut string_buffer)?;
            strings.push(String::from_utf8(string_buffer.clone()).map_err(|e| {
                eyre::eyre!("Invalid UTF-8 sequence in dictionary: {}", e)
            })?);
        }

        if count_eof_mismatch {
            log::warn!(
                "uocf: string dictionary '{}' declared {} strings but reached EOF after {} complete entries",
                source_label,
                declared_strings_count,
                strings.len(),
            );
        }

        Ok(UoStringDictionary {
            unk1,
            strings_count: strings.len() as u32,
            unk2,
            strings,
        })
    }

    /// Returns a string from the dictionary by its index.
    ///
    /// # Arguments
    ///
    /// * `string_index` - The index of the string to return.
    pub fn get_string(&self, string_index: usize) -> Option<&str> {
        self.strings.get(string_index).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uop::{file::CompressionFlag, package::UopPackage};
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

        assert_eq!(dictionary.strings_count, 2);
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

        assert_eq!(dictionary.strings_count, 2);
        assert_eq!(dictionary.get_string(0), Some("build/tileart/00000001.dat"));
        assert_eq!(dictionary.get_string(1), Some("UOSpriteShader"));

        let _ = fs::remove_file(path);
    }
}
