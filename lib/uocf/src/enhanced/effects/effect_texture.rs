//! `EffectTexture.uop`: the particle effect files (`data/effects/*`): Gamebryo `.nif` models, Chaos `.ems`
//! scripts and their `.dds`/`.tga`/`.psd` textures.
//!
//! The package stores only name hashes. Names are resolved by hashing candidates: names the caller knows, a
//! [`HashDictionary`], the string dictionary (particle effect names) and file names harvested from the payloads
//! themselves (texture names inside models and scripts). See `ENHANCED_CLIENT_FILES.md`.
//!
//! Edits are entry level ([`EffectTexturePackage::put`], [`EffectTexturePackage::remove`]) and the package is
//! only ever written to a new path ([`EffectTexturePackage::save_as`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};

use super::effect_definition::PARTICLE_RESOURCE_DIR;
use crate::enhanced::string_dictionary::UoStringDictionary;
use crate::enhanced::uop_entries;
pub use crate::enhanced::uop_entries::EntryPut;
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::hash_dictionary::HashDictionary;
use crate::uop_container::package::{LoadMode, UopPackage};

/// File name of the package in the client directory.
pub const EFFECT_TEXTURE_UOP_NAME: &str = "EffectTexture.uop";
/// File name extensions of effect files, for name harvesting (lowercase).
pub const EFFECT_FILE_EXTENSIONS: [&str; 10] = ["nif", "ems", "emi", "tga", "dds", "psd", "kf", "bmp", "png", "jpg"];
/// Compression of entries added to the package (every client entry with a payload is zlib-compressed).
pub const EFFECT_TEXTURE_NEW_ENTRY_COMPRESSION: CompressionFlag = CompressionFlag::Zlib;

/// File name of the string dictionary package in the client directory.
const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";
/// Longest harvested name run, in bytes; longer runs of name characters are not names.
const MAX_HARVESTED_NAME_LENGTH: usize = 200;
/// Path separators inside harvested names.
const PATH_SEPARATORS: [char; 3] = ['/', '\\', ':'];
const NIF_MAGIC: &[u8] = b"Gamebryo File Format";
const DDS_MAGIC: &[u8] = b"DDS ";
const PSD_MAGIC: &[u8] = b"8BPS";
/// The first keyword of a Chaos script (case-insensitive).
const CHAOS_SCRIPT_MAGIC: &[u8] = b"particlesystem";

/// The package path of an effect file: `data/effects/` and the lowercase base name (any directory in
/// `file_name` is dropped, as the client does for particle names).
pub fn effect_texture_path(file_name: &str) -> String {
    let base_name = file_name.rsplit(PATH_SEPARATORS).next().unwrap_or(file_name);
    format!("{PARTICLE_RESOURCE_DIR}{}", base_name.trim().to_ascii_lowercase())
}

/// The name hash of [`effect_texture_path`].
pub fn effect_texture_hash(file_name: &str) -> u64 {
    hash_file_name_single(&effect_texture_path(file_name))
}

/// What an effect file holds, from its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectFileKind {
    /// A Gamebryo NIF model.
    Nif,
    /// A Chaos particle script ([`super::chaos_script`]).
    ChaosScript,
    Dds,
    Psd,
    /// Anything else (TGA textures have no magic).
    Other,
}

impl EffectFileKind {
    pub fn detect(payload: &[u8]) -> Self {
        let trimmed = payload.trim_ascii_start();
        if payload.starts_with(NIF_MAGIC) {
            Self::Nif
        } else if payload.starts_with(DDS_MAGIC) {
            Self::Dds
        } else if payload.starts_with(PSD_MAGIC) {
            Self::Psd
        } else if trimmed.len() >= CHAOS_SCRIPT_MAGIC.len()
            && trimmed[..CHAOS_SCRIPT_MAGIC.len()].eq_ignore_ascii_case(CHAOS_SCRIPT_MAGIC)
        {
            Self::ChaosScript
        } else {
            Self::Other
        }
    }
}

/// One entry of the package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectTextureEntry {
    pub hash: u64,
    /// The resolved path (`data/effects/<name>`), if known.
    pub name: Option<String>,
    pub compression: CompressionFlag,
    pub compressed_size: u32,
    /// 0 for the empty placeholder entries the client package has.
    pub decompressed_size: u32,
}

impl EffectTextureEntry {
    /// Whether the entry has a payload.
    pub fn has_payload(&self) -> bool {
        self.compressed_size != 0 && self.decompressed_size != 0
    }

    /// The base name of the resolved path.
    pub fn file_name(&self) -> Option<&str> {
        self.name.as_deref().map(|name| name.strip_prefix(PARTICLE_RESOURCE_DIR).unwrap_or(name))
    }
}

/// `EffectTexture.uop` opened for listing, reading and entry edits.
pub struct EffectTexturePackage {
    package: UopPackage,
    source: PathBuf,
    names: HashMap<u64, String>,
}

impl EffectTexturePackage {
    /// Opens a package; payloads are read on demand.
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let source = path.as_ref().to_path_buf();
        let package = UopPackage::load_with_mode(&source, LoadMode::Lazy)
            .wrap_err_with(|| format!("failed to load {}", source.display()))?;
        Ok(Self { package, source, names: HashMap::new() })
    }

    /// Opens `EffectTexture.uop` of a client directory.
    pub fn load_from_client_dir(client_dir: impl AsRef<Path>) -> eyre::Result<Self> {
        Self::load(client_dir.as_ref().join(EFFECT_TEXTURE_UOP_NAME))
    }

    /// Opens the package of a client directory and resolves every name it can without an external dictionary:
    /// the string dictionary of the directory (when present) and the names harvested from the payloads.
    pub fn load_with_names(client_dir: impl AsRef<Path>) -> eyre::Result<Self> {
        let client_dir = client_dir.as_ref();
        let mut package = Self::load_from_client_dir(client_dir)?;
        let strings_path = client_dir.join(STRING_DICTIONARY_UOP_NAME);
        if strings_path.exists() {
            let strings = UoStringDictionary::load(&strings_path)
                .wrap_err_with(|| format!("failed to load {}", strings_path.display()))?;
            package.resolve_names_from_string_dictionary(&strings);
        }
        package.harvest_names_from_payloads()?;
        Ok(package)
    }

    pub fn source_path(&self) -> &Path {
        &self.source
    }

    /// The underlying package.
    pub fn package(&self) -> &UopPackage {
        &self.package
    }

    /// Sets the FILETIME stamped into the data block header of the entries [`Self::put`] stores from now on;
    /// None (the default) stamps the current time (`UopPackage::set_edit_filetime`).
    pub fn set_edit_filetime(&mut self, filetime: Option<u64>) {
        self.package.set_edit_filetime(filetime);
    }

    /// Resolves the entries whose hash matches one of `candidates` (file names or `data/effects/` paths,
    /// normalized by [`effect_texture_path`]); returns how many entries got a name.
    pub fn resolve_names<S: AsRef<str>>(&mut self, candidates: impl IntoIterator<Item = S>) -> usize {
        let mut resolved = 0;
        for candidate in candidates {
            let path = effect_texture_path(candidate.as_ref());
            let hash = hash_file_name_single(&path);
            if !self.names.contains_key(&hash) && self.package.get_file_by_hash(hash).is_some() {
                self.names.insert(hash, path);
                resolved += 1;
            }
        }
        resolved
    }

    /// Resolves names from a hash dictionary (only its paths under `data/effects/`).
    pub fn resolve_names_from_dictionary(&mut self, dictionary: &HashDictionary) -> usize {
        let mut resolved = 0;
        for file in self.package.iter_files() {
            let hash = file.filename_hash();
            if self.names.contains_key(&hash) {
                continue;
            }
            if let Some(name) = dictionary.resolve(hash).filter(|name| name.starts_with(PARTICLE_RESOURCE_DIR)) {
                self.names.insert(hash, name.to_owned());
                resolved += 1;
            }
        }
        resolved
    }

    /// Resolves names from the string dictionary: the particle names of the effect definitions are there.
    pub fn resolve_names_from_string_dictionary(&mut self, strings: &UoStringDictionary) -> usize {
        let candidates = strings.iter().flat_map(|(_, text)| harvest_candidates(text.as_bytes())).collect::<Vec<_>>();
        self.resolve_names(candidates)
    }

    /// Resolves names from the file names the payloads mention (texture names in models and scripts, model
    /// names in scripts), each tried with every [`EFFECT_FILE_EXTENSIONS`] extension. Reads every payload.
    pub fn harvest_names_from_payloads(&mut self) -> eyre::Result<usize> {
        let mut reader = self.package.open_payload_reader()?;
        let hashes = self.package.iter_files().filter(|file| file.has_size()).map(|file| file.filename_hash());
        let mut candidates = Vec::new();
        for hash in hashes.collect::<Vec<_>>() {
            let payload = self
                .package
                .unpack_file_by_hash_from_reader(hash, &mut reader)
                .wrap_err_with(|| format!("failed to read entry {hash:016X}"))?
                .unwrap_or_default();
            candidates.extend(harvest_candidates(&payload));
        }
        candidates.sort_unstable();
        candidates.dedup();
        Ok(self.resolve_names(candidates))
    }

    /// The resolved path of an entry.
    pub fn name(&self, hash: u64) -> Option<&str> {
        self.names.get(&hash).map(String::as_str)
    }

    /// Every entry, in package order.
    pub fn entries(&self) -> Vec<EffectTextureEntry> {
        self.package
            .iter_files()
            .map(|file| EffectTextureEntry {
                hash: file.filename_hash(),
                name: self.names.get(&file.filename_hash()).cloned(),
                compression: file.compression(),
                compressed_size: file.compressed_size(),
                decompressed_size: file.decompressed_size(),
            })
            .collect()
    }

    /// Whether the package has an entry for `file_name`.
    pub fn contains(&self, file_name: &str) -> bool {
        self.package.get_file_by_hash(effect_texture_hash(file_name)).is_some()
    }

    /// The payload of `file_name`, or `None` without such an entry.
    pub fn read(&self, file_name: &str) -> eyre::Result<Option<Vec<u8>>> {
        self.read_by_hash(effect_texture_hash(file_name))
    }

    /// The payload of an entry, or `None` without such an entry.
    pub fn read_by_hash(&self, hash: u64) -> eyre::Result<Option<Vec<u8>>> {
        self.package.unpack_file_by_hash(hash).wrap_err_with(|| format!("failed to read entry {hash:016X}"))
    }

    /// Replaces the payload of `file_name` (keeping its compression flag) or adds it
    /// ([`EFFECT_TEXTURE_NEW_ENTRY_COMPRESSION`]). Either way the entry gets a timestamp data block header
    /// stamped now (or at [`Self::set_edit_filetime`]) instead of its signature header; an entry whose payload is
    /// unchanged is left untouched.
    pub fn put(&mut self, file_name: &str, payload: &[u8]) -> eyre::Result<EntryPut> {
        let path = effect_texture_path(file_name);
        let hash = hash_file_name_single(&path);
        let put = uop_entries::put_entry(&mut self.package, hash, payload, EFFECT_TEXTURE_NEW_ENTRY_COMPRESSION)
            .wrap_err_with(|| format!("failed to store {path}"))?;
        self.names.insert(hash, path);
        Ok(put)
    }

    /// Removes `file_name`; returns whether it existed.
    pub fn remove(&mut self, file_name: &str) -> bool {
        let hash = effect_texture_hash(file_name);
        self.names.remove(&hash);
        uop_entries::remove_entry(&mut self.package, hash)
    }

    /// Writes the package with its edits to `output`, a new path: the source package is never overwritten.
    /// Unchanged entries keep their payload bytes, compression flag and data block header.
    pub fn save_as(&mut self, output: impl AsRef<Path>) -> eyre::Result<()> {
        uop_entries::save_package_as(&mut self.package, &self.source, output.as_ref())
    }
}

/// Candidate file names in `bytes`: every run of name characters ending in a known extension, as its base name
/// and every suffix of it starting with an alphanumeric character (a run may start with stray bytes), each
/// also with every other known extension.
fn harvest_candidates(bytes: &[u8]) -> Vec<String> {
    let is_name_byte = |byte: u8| byte.is_ascii_alphanumeric() || b"_-. \\/:".contains(&byte);
    let mut candidates = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        if !is_name_byte(bytes[start]) {
            start += 1;
            continue;
        }
        let end = bytes[start..].iter().position(|&byte| !is_name_byte(byte)).map_or(bytes.len(), |length| start + length);
        let run = &bytes[start..end];
        start = end;
        if run.len() > MAX_HARVESTED_NAME_LENGTH {
            continue;
        }
        let run = String::from_utf8_lossy(run).to_ascii_lowercase();
        for extension in EFFECT_FILE_EXTENSIONS {
            let suffix = format!(".{extension}");
            for (index, _) in run.match_indices(&suffix) {
                let name_end = index + suffix.len();
                if run[name_end..].starts_with(|character: char| character.is_ascii_alphanumeric()) {
                    continue;
                }
                let path = &run[..name_end];
                let base = path.rsplit(PATH_SEPARATORS).next().unwrap_or(path);
                let stem = &base[..base.len() - suffix.len()];
                for (offset, character) in stem.char_indices() {
                    if character.is_ascii_alphanumeric() {
                        let stem = &stem[offset..];
                        candidates.extend(EFFECT_FILE_EXTENSIONS.iter().map(|other| format!("{stem}.{other}")));
                    }
                }
            }
        }
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_lowercase_base_names() {
        assert_eq!(effect_texture_path("Fire.NIF"), "data/effects/fire.nif");
        assert_eq!(effect_texture_path("c:\\art\\Spark.tga"), "data/effects/spark.tga");
        assert_eq!(effect_texture_path("data/effects/x.ems"), "data/effects/x.ems");
    }

    #[test]
    fn harvests_names_with_stray_prefixes() {
        let candidates = harvest_candidates(b"\x00\x00\x00.texture \"Blue_Spark.TGA\"\x01x.nifty");
        assert!(candidates.contains(&"blue_spark.tga".to_owned()));
        assert!(candidates.contains(&"blue_spark.dds".to_owned()));
        assert!(candidates.contains(&"spark.tga".to_owned()));
        assert!(!candidates.iter().any(|candidate| candidate.starts_with("x.")), "`.nifty` is no extension");
    }

    #[test]
    fn detects_kinds() {
        assert_eq!(EffectFileKind::detect(b"Gamebryo File Format, Version 20"), EffectFileKind::Nif);
        assert_eq!(EffectFileKind::detect(b"\r\nParticleSystem \"x\" 1 {}"), EffectFileKind::ChaosScript);
        assert_eq!(EffectFileKind::detect(b"DDS \x7c"), EffectFileKind::Dds);
        assert_eq!(EffectFileKind::detect(b"8BPS"), EffectFileKind::Psd);
        assert_eq!(EffectFileKind::detect(b"\x00\x00\x02"), EffectFileKind::Other);
    }
}
