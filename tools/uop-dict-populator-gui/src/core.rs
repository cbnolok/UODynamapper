use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use uocf::uop_container::hash_dictionary::HashDictionary;
use uocf::uop_container::package::UopPackage;
use uocf::uop_container::template::UopTemplate;

#[derive(Deserialize, Debug, Clone)]
pub struct PopulatorConfig {
    #[serde(flatten)]
    pub packages: HashMap<String, PackageConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PackageConfig {
    pub candidates: Vec<String>,
    pub range: Option<[u64; 2]>,
}

#[derive(Debug, Default, Clone)]
pub struct UopDictionary {
    hash_dictionary: HashDictionary,
}

impl UopDictionary {
    pub fn load(path: impl AsRef<Path>) -> color_eyre::eyre::Result<Self> {
        Ok(Self {
            hash_dictionary: HashDictionary::load(path.as_ref())?,
        })
    }

    pub fn save(&self, path: impl AsRef<Path>) -> color_eyre::eyre::Result<()> {
        self.hash_dictionary.save(path.as_ref())
    }

    pub fn contains(&self, hash: u64) -> bool {
        self.hash_dictionary.contains(hash)
    }

    pub fn set(&mut self, hash: u64, name: impl Into<String>) -> bool {
        self.hash_dictionary.set(hash, name)
    }

    pub fn len(&self) -> usize {
        self.hash_dictionary.len()
    }

    pub fn named_len(&self) -> usize {
        self.hash_dictionary.named_len()
    }
}

pub struct PopulatorTask {
    pub uop_path: PathBuf,
    pub config: PackageConfig,
    pub dictionary: Arc<UopDictionary>,
    pub stop_signal: Arc<AtomicBool>,
}

impl PopulatorTask {
    pub fn run(self) -> color_eyre::eyre::Result<HashMap<u64, String>> {
        let package = UopPackage::load(&self.uop_path)?;
        let missing_hashes: HashSet<u64> = package.iter_files()
            .map(|f| f.filename_hash())
            .filter(|h| !self.dictionary.contains(*h))
            .collect();

        if missing_hashes.is_empty() {
            return Ok(HashMap::new());
        }

        let mut all_found = HashMap::new();
        for template_str in &self.config.candidates {
            if self.stop_signal.load(Ordering::Relaxed) {
                break;
            }

            let range = self.config.range.map(|r| r[0]..=r[1]);
            let template = UopTemplate::new(template_str, range);
            let found = template.crack(&missing_hashes, &self.stop_signal);
            all_found.extend(found);
        }

        Ok(all_found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uop_dictionary_preserves_unknown_hash_entries() {
        let mut source = HashDictionary::new();
        source.insert_unknown(0x1111);
        source.set(0x2222, "known/name.dds");

        let dictionary = UopDictionary {
            hash_dictionary: source.clone(),
        };

        let bytes = dictionary.hash_dictionary.to_bytes().expect("serialize dictionary");
        let parsed = HashDictionary::from_bytes(&bytes).expect("parse dictionary");

        assert_eq!(parsed, source);
    }
}
