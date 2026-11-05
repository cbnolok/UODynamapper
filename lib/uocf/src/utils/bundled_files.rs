//! Finding the data files shipped with the tools: the conversion tables and the hash dictionary.
//!
//! The unpacked tools release keeps the executables in `bin/` and these files in `shared/`, each at
//! [`BundledFile::relative_path`]. A source checkout keeps them below [`BundledFile::repo_dir`] at the same relative
//! path. [`resolve`] looks, in order:
//! 1. at the path the user gave (a command-line option or a setting), and only there: a missing file is an error;
//! 2. next to the executable, then in the `shared/` folder beside the executable's folder (the release layout);
//! 3. in the source checkout the tool was built from (development builds, wherever the target directory is).
//!
//! The release packaging (the justfile's `tool_shared`) copies the folders and files named here into `shared/`.

use color_eyre::eyre;
use std::path::{Path, PathBuf};

/// The release folder holding the data files, beside the `bin/` folder of the executables.
pub const RELEASE_SHARED_DIR: &str = "shared";
/// Folder of the conversion tables, below [`RELEASE_SHARED_DIR`] and below the Dynamapper assets.
pub const CONVERSION_TABLES_DIR: &str = "cc_ec_convtables";
/// The Dynamapper assets, relative to the workspace root.
const DYNAMAPPER_ASSETS_REPO_DIR: &str = "dynamapper/assets";
/// The assets shared by the tools, relative to the workspace root.
const TOOLS_SHARED_ASSETS_REPO_DIR: &str = "tools/_shared_assets";
/// How deep this crate sits below the workspace root (`lib/uocf`).
const CRATE_DEPTH_IN_WORKSPACE: usize = 2;

/// A data file the tools ship with.
#[derive(Clone, Copy, Debug)]
pub struct BundledFile {
    /// The file name.
    pub name: &'static str,
    /// The folder holding it below [`RELEASE_SHARED_DIR`] and below `repo_dir`; empty when it sits directly there.
    pub subdir: &'static str,
    /// The folder holding `subdir` in the source checkout, relative to the workspace root.
    pub repo_dir: &'static str,
}

impl BundledFile {
    /// The path below [`RELEASE_SHARED_DIR`], and below [`Self::repo_dir`].
    pub fn relative_path(&self) -> PathBuf {
        Path::new(self.subdir).join(self.name)
    }

    /// This file in `dir`, for an option that names the folder holding it.
    pub fn in_dir(&self, dir: &Path) -> PathBuf {
        dir.join(self.name)
    }
}

/// Classic land id to EC material routing.
pub const TERRAIN_TRANSCODE_KDL: BundledFile =
    BundledFile { name: "TerrainTranscode.kdl", subdir: CONVERSION_TABLES_DIR, repo_dir: DYNAMAPPER_ASSETS_REPO_DIR };
/// Reviewed EC terrain layer substitutions.
pub const EC_TERRAIN_OVERRIDES_KDL: BundledFile =
    BundledFile { name: "EcTerrainOverrides.kdl", subdir: CONVERSION_TABLES_DIR, repo_dir: DYNAMAPPER_ASSETS_REPO_DIR };
/// Reviewed EC surface art redirections.
pub const EC_SURFACE_OVERRIDES_KDL: BundledFile =
    BundledFile { name: "EcSurfaceOverrides.kdl", subdir: CONVERSION_TABLES_DIR, repo_dir: DYNAMAPPER_ASSETS_REPO_DIR };
/// EC mobile animation item metadata.
pub const EC_MOBILE_ANIMATIONS_KDL: BundledFile =
    BundledFile { name: "EcMobileAnimations.kdl", subdir: CONVERSION_TABLES_DIR, repo_dir: DYNAMAPPER_ASSETS_REPO_DIR };
/// The starting-point UOP hash dictionary.
pub const HASH_DICTIONARY: BundledFile =
    BundledFile { name: "Dictionary.dic", subdir: "", repo_dir: TOOLS_SHARED_ASSETS_REPO_DIR };

/// Every file above, for the checks that the source checkout holds them.
pub const ALL_BUNDLED_FILES: &[BundledFile] =
    &[TERRAIN_TRANSCODE_KDL, EC_TERRAIN_OVERRIDES_KDL, EC_SURFACE_OVERRIDES_KDL, EC_MOBILE_ANIMATIONS_KDL, HASH_DICTIONARY];

/// The path of `file`: `given` when set (it must exist), else the first existing default location. The error names
/// every place looked at.
pub fn resolve(given: Option<&Path>, file: BundledFile) -> eyre::Result<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    resolve_from(given, file, exe_dir.as_deref(), source_checkout_root())
}

/// The workspace root of the checkout this crate was built from.
fn source_checkout_root() -> Option<&'static Path> {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(CRATE_DEPTH_IN_WORKSPACE)
}

fn resolve_from(
    given: Option<&Path>,
    file: BundledFile,
    exe_dir: Option<&Path>,
    checkout_root: Option<&Path>,
) -> eyre::Result<PathBuf> {
    if let Some(path) = given {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        eyre::bail!("{}: not found at the given path {}", file.name, path.display());
    }
    let candidates = default_locations(file, exe_dir, checkout_root);
    if let Some(found) = candidates.iter().find(|path| path.is_file()) {
        return Ok(found.clone());
    }
    let looked_in: Vec<String> = candidates.iter().map(|path| format!("  {}", path.display())).collect();
    eyre::bail!(
        "{}: not found; give its path, or keep the tools next to their {RELEASE_SHARED_DIR}/ folder. Looked in:\n{}",
        file.name,
        looked_in.join("\n")
    )
}

/// Where `file` is looked for when no path is given, in order.
fn default_locations(file: BundledFile, exe_dir: Option<&Path>, checkout_root: Option<&Path>) -> Vec<PathBuf> {
    let relative = file.relative_path();
    let mut locations = Vec::new();
    if let Some(exe_dir) = exe_dir {
        locations.push(exe_dir.join(&relative));
        if let Some(release_root) = exe_dir.parent() {
            locations.push(release_root.join(RELEASE_SHARED_DIR).join(&relative));
        }
    }
    if let Some(root) = checkout_root {
        locations.push(root.join(file.repo_dir).join(&relative));
    }
    locations
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE_BIN_DIR: &str = "bin";
    const CHECKOUT_DIR: &str = "checkout";

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uocf_bundled_files_{test}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    /// A release folder (`bin/` and `shared/`) and a checkout, each holding `file`.
    fn release_and_checkout(test: &str, file: BundledFile) -> (PathBuf, PathBuf, PathBuf) {
        let dir = temp_dir(test);
        let exe_dir = dir.join(RELEASE_BIN_DIR);
        std::fs::create_dir_all(&exe_dir).unwrap();
        let shared = dir.join(RELEASE_SHARED_DIR).join(file.relative_path());
        write(&shared);
        let checkout = dir.join(CHECKOUT_DIR);
        write(&checkout.join(file.repo_dir).join(file.relative_path()));
        (dir, exe_dir, checkout)
    }

    #[test]
    fn the_given_path_wins_and_must_exist() {
        let (dir, exe_dir, checkout) = release_and_checkout("given", EC_TERRAIN_OVERRIDES_KDL);
        let given = dir.join("edited.kdl");
        write(&given);
        let found = resolve_from(Some(&given), EC_TERRAIN_OVERRIDES_KDL, Some(&exe_dir), Some(&checkout)).unwrap();
        assert_eq!(found, given);

        let missing = dir.join("missing.kdl");
        let error = resolve_from(Some(&missing), EC_TERRAIN_OVERRIDES_KDL, Some(&exe_dir), Some(&checkout))
            .unwrap_err()
            .to_string();
        assert!(error.contains("EcTerrainOverrides.kdl") && error.contains(&missing.display().to_string()), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_release_shared_folder_comes_before_the_checkout() {
        let (dir, exe_dir, checkout) = release_and_checkout("release", EC_MOBILE_ANIMATIONS_KDL);
        let found = resolve_from(None, EC_MOBILE_ANIMATIONS_KDL, Some(&exe_dir), Some(&checkout)).unwrap();
        assert_eq!(found, dir.join(RELEASE_SHARED_DIR).join(CONVERSION_TABLES_DIR).join("EcMobileAnimations.kdl"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_next_to_the_executable_comes_first() {
        let (dir, exe_dir, checkout) = release_and_checkout("flat", HASH_DICTIONARY);
        let beside = exe_dir.join("Dictionary.dic");
        write(&beside);
        assert_eq!(resolve_from(None, HASH_DICTIONARY, Some(&exe_dir), Some(&checkout)).unwrap(), beside);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_checkout_is_the_last_resort() {
        let dir = temp_dir("checkout");
        let exe_dir = dir.join("target").join("debug");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let checkout = dir.join(CHECKOUT_DIR);
        let in_checkout = checkout.join(DYNAMAPPER_ASSETS_REPO_DIR).join(CONVERSION_TABLES_DIR).join(TERRAIN_TRANSCODE_KDL.name);
        write(&in_checkout);
        assert_eq!(resolve_from(None, TERRAIN_TRANSCODE_KDL, Some(&exe_dir), Some(&checkout)).unwrap(), in_checkout);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_names_every_place_looked_at() {
        let dir = temp_dir("missing");
        let exe_dir = dir.join(RELEASE_BIN_DIR);
        let checkout = dir.join(CHECKOUT_DIR);
        let error = resolve_from(None, EC_SURFACE_OVERRIDES_KDL, Some(&exe_dir), Some(&checkout)).unwrap_err().to_string();
        for place in default_locations(EC_SURFACE_OVERRIDES_KDL, Some(&exe_dir), Some(&checkout)) {
            assert!(error.contains(&place.display().to_string()), "{error}");
        }
        let in_release = dir.join(RELEASE_SHARED_DIR).join(CONVERSION_TABLES_DIR).join(EC_SURFACE_OVERRIDES_KDL.name);
        assert!(error.contains(&in_release.display().to_string()), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The checkout this crate builds from holds every bundled file where the release packaging copies it from.
    #[test]
    fn the_source_checkout_holds_every_bundled_file() {
        for file in ALL_BUNDLED_FILES {
            let path = resolve_from(None, *file, None, source_checkout_root()).unwrap_or_else(|error| panic!("{error}"));
            assert!(path.ends_with(file.relative_path()));
        }
    }
}
