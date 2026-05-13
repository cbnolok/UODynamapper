use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
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

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct UopDictionary {
    pub hash_to_name: HashMap<u64, String>,
}

impl UopDictionary {
    pub fn load(path: impl AsRef<Path>) -> color_eyre::eyre::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let dict = if content.trim().starts_with('{') {
            serde_json::from_str(&content)?
        } else {
            // Assume simple text format (one name per line)
            let mut hash_to_name = HashMap::new();
            for line in content.lines() {
                let name = line.trim();
                if !name.is_empty() {
                    let hash = uocf::uop_container::hash::hash_file_name_single(name);
                    hash_to_name.insert(hash, name.to_string());
                }
            }
            Self { hash_to_name }
        };
        Ok(dict)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> color_eyre::eyre::Result<()> {
        let json = serde_json::to_string_pretty(&self.hash_to_name)?;
        std::fs::write(path, json)?;
        Ok(())
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
            .filter(|h| !self.dictionary.hash_to_name.contains_key(h))
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
