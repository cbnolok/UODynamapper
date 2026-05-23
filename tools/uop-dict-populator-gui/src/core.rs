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

impl PopulatorConfig {
    pub fn validate(&self) -> color_eyre::eyre::Result<()> {
        if self.packages.is_empty() {
            color_eyre::eyre::bail!("configuration does not define any UOP packages");
        }

        for (package_name, package_config) in &self.packages {
            if package_name.trim().is_empty() {
                color_eyre::eyre::bail!("configuration contains an empty UOP package name");
            }
            if package_config.candidates.is_empty() {
                color_eyre::eyre::bail!("{} has no candidate templates", package_name);
            }
            if package_config.candidates.iter().any(|candidate| candidate.trim().is_empty()) {
                color_eyre::eyre::bail!("{} contains an empty candidate template", package_name);
            }
            if let Some([start, end]) = package_config.range {
                if start > end {
                    color_eyre::eyre::bail!(
                        "{} has an invalid range: start {} is greater than end {}",
                        package_name,
                        start,
                        end,
                    );
                }
            }
        }

        Ok(())
    }
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

pub enum TaskProgress {
    MissingHashes(usize),
    TryingTemplate(String),
    TemplateMatches {
        template: String,
        found: usize,
    },
    Stopped,
}

impl PopulatorTask {
    pub fn run_with_progress(
        self,
        mut progress: impl FnMut(TaskProgress),
    ) -> color_eyre::eyre::Result<HashMap<u64, String>> {
        let package = UopPackage::load(&self.uop_path)?;
        let missing_hashes: HashSet<u64> = package.iter_files()
            .map(|f| f.filename_hash())
            .filter(|h| !self.dictionary.contains(*h))
            .collect();
        progress(TaskProgress::MissingHashes(missing_hashes.len()));

        if missing_hashes.is_empty() {
            return Ok(HashMap::new());
        }

        let mut all_found = HashMap::new();
        for template_str in &self.config.candidates {
            if self.stop_signal.load(Ordering::Relaxed) {
                progress(TaskProgress::Stopped);
                break;
            }

            progress(TaskProgress::TryingTemplate(template_str.clone()));
            let range = self.config.range.map(|r| r[0]..=r[1]);
            let template = UopTemplate::new(template_str, range);
            let found = template.crack(&missing_hashes, &self.stop_signal);
            progress(TaskProgress::TemplateMatches {
                template: template_str.clone(),
                found: found.len(),
            });
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

    #[test]
    fn populator_config_rejects_inverted_ranges() {
        let config = PopulatorConfig {
            packages: HashMap::from([(
                "Texture.uop".to_string(),
                PackageConfig {
                    candidates: vec!["build/worldart/{:08}.dds".to_string()],
                    range: Some([10, 1]),
                },
            )]),
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn populator_config_requires_candidate_templates() {
        let config = PopulatorConfig {
            packages: HashMap::from([(
                "Texture.uop".to_string(),
                PackageConfig {
                    candidates: Vec::new(),
                    range: Some([0, 10]),
                },
            )]),
        };

        assert!(config.validate().is_err());
    }
}
