//! Finding the data files shipped with the tools.
//!
//! The unpacked tools release keeps the executables in `bin/` and these files in `shared/`, each at
//! [`BundledFile::relative_path`]. A source checkout keeps them in [`BundledFile::checkout_dir`] at the same relative
//! path. [`resolve`] looks, in order:
//! 1. at the path the user gave (a command-line option or a setting), and only there: a missing file is an error;
//! 2. next to the executable, then in the `shared/` folder beside the executable's folder (the release layout);
//! 3. in the source checkout the defining crate was built from (development builds, wherever the target directory is).
//!
//! The crate that owns a file defines its [`BundledFile`]; this crate defines [`HASH_DICTIONARY`]. The release packaging
//! (the justfile's `tool_shared`) copies the files into `shared/`.

use color_eyre::eyre;
use std::path::{Component, Path, PathBuf};

/// The release folder holding the data files, beside the `bin/` folder of the executables.
pub const RELEASE_SHARED_DIR: &str = "shared";
/// The assets shared by the tools in the source checkout, relative to this crate's folder.
const TOOLS_SHARED_ASSETS_FROM_CRATE: &str = "../../tools/_shared_assets";

/// A data file the tools ship with.
#[derive(Clone, Copy, Debug)]
pub struct BundledFile {
    /// The file name.
    pub name: &'static str,
    /// The folder holding it below [`RELEASE_SHARED_DIR`] and below [`Self::checkout_dir`]; empty when it sits directly
    /// there.
    pub subdir: &'static str,
    /// The folder of the crate defining this file: its `env!("CARGO_MANIFEST_DIR")`.
    pub crate_dir: &'static str,
    /// The folder holding `subdir` in the source checkout, relative to `crate_dir`.
    pub checkout_dir_from_crate: &'static str,
}

impl BundledFile {
    /// The path below [`RELEASE_SHARED_DIR`], and below [`Self::checkout_dir`].
    pub fn relative_path(&self) -> PathBuf {
        Path::new(self.subdir).join(self.name)
    }

    /// This file in `dir`, for an option that names the folder holding it.
    pub fn in_dir(&self, dir: &Path) -> PathBuf {
        dir.join(self.name)
    }

    /// The folder holding `subdir` in the source checkout the defining crate was built from.
    pub fn checkout_dir(&self) -> PathBuf {
        let mut dir = PathBuf::from(self.crate_dir);
        for component in Path::new(self.checkout_dir_from_crate).components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    dir.pop();
                }
                other => dir.push(other),
            }
        }
        dir
    }
}

/// The starting-point UOP hash dictionary.
pub const HASH_DICTIONARY: BundledFile = BundledFile {
    name: "Dictionary.dic",
    subdir: "",
    crate_dir: env!("CARGO_MANIFEST_DIR"),
    checkout_dir_from_crate: TOOLS_SHARED_ASSETS_FROM_CRATE,
};

/// The path of `file`: `given` when set (it must exist), else the first existing default location. The error names
/// every place looked at.
pub fn resolve(given: Option<&Path>, file: BundledFile) -> eyre::Result<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    resolve_from(given, file, exe_dir.as_deref(), &file.checkout_dir())
}

fn resolve_from(given: Option<&Path>, file: BundledFile, exe_dir: Option<&Path>, checkout_dir: &Path) -> eyre::Result<PathBuf> {
    if let Some(path) = given {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        eyre::bail!("{}: not found at the given path {}", file.name, path.display());
    }
    let candidates = default_locations(file, exe_dir, checkout_dir);
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
fn default_locations(file: BundledFile, exe_dir: Option<&Path>, checkout_dir: &Path) -> Vec<PathBuf> {
    let relative = file.relative_path();
    let mut locations = Vec::new();
    if let Some(exe_dir) = exe_dir {
        locations.push(exe_dir.join(&relative));
        if let Some(release_root) = exe_dir.parent() {
            locations.push(release_root.join(RELEASE_SHARED_DIR).join(&relative));
        }
    }
    locations.push(checkout_dir.join(&relative));
    locations
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE_BIN_DIR: &str = "bin";
    const CHECKOUT_DIR: &str = "checkout";
    const TABLES_DIR: &str = "tables";
    /// A file kept in a subfolder, like the converters' tables; the tests give its checkout folder themselves.
    const TABLE: BundledFile =
        BundledFile { name: "Table.kdl", subdir: TABLES_DIR, crate_dir: env!("CARGO_MANIFEST_DIR"), checkout_dir_from_crate: "" };

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

    /// A release folder (`bin/` and `shared/`) and a checkout folder, each holding `file`.
    fn release_and_checkout(test: &str, file: BundledFile) -> (PathBuf, PathBuf, PathBuf) {
        let dir = temp_dir(test);
        let exe_dir = dir.join(RELEASE_BIN_DIR);
        std::fs::create_dir_all(&exe_dir).unwrap();
        write(&dir.join(RELEASE_SHARED_DIR).join(file.relative_path()));
        let checkout = dir.join(CHECKOUT_DIR);
        write(&checkout.join(file.relative_path()));
        (dir, exe_dir, checkout)
    }

    #[test]
    fn the_given_path_wins_and_must_exist() {
        let (dir, exe_dir, checkout) = release_and_checkout("given", TABLE);
        let given = dir.join("edited.kdl");
        write(&given);
        assert_eq!(resolve_from(Some(&given), TABLE, Some(&exe_dir), &checkout).unwrap(), given);

        let missing = dir.join("missing.kdl");
        let error = resolve_from(Some(&missing), TABLE, Some(&exe_dir), &checkout).unwrap_err().to_string();
        assert!(error.contains(TABLE.name) && error.contains(&missing.display().to_string()), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_release_shared_folder_comes_before_the_checkout() {
        let (dir, exe_dir, checkout) = release_and_checkout("release", TABLE);
        let found = resolve_from(None, TABLE, Some(&exe_dir), &checkout).unwrap();
        assert_eq!(found, dir.join(RELEASE_SHARED_DIR).join(TABLES_DIR).join(TABLE.name));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_next_to_the_executable_comes_first() {
        let (dir, exe_dir, checkout) = release_and_checkout("flat", HASH_DICTIONARY);
        let beside = exe_dir.join(HASH_DICTIONARY.name);
        write(&beside);
        assert_eq!(resolve_from(None, HASH_DICTIONARY, Some(&exe_dir), &checkout).unwrap(), beside);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_checkout_is_the_last_resort() {
        let dir = temp_dir("checkout");
        let exe_dir = dir.join("target").join("debug");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let checkout = dir.join(CHECKOUT_DIR);
        let in_checkout = checkout.join(TABLES_DIR).join(TABLE.name);
        write(&in_checkout);
        assert_eq!(resolve_from(None, TABLE, Some(&exe_dir), &checkout).unwrap(), in_checkout);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_names_every_place_looked_at() {
        let dir = temp_dir("missing");
        let exe_dir = dir.join(RELEASE_BIN_DIR);
        let checkout = dir.join(CHECKOUT_DIR);
        let error = resolve_from(None, TABLE, Some(&exe_dir), &checkout).unwrap_err().to_string();
        for place in default_locations(TABLE, Some(&exe_dir), &checkout) {
            assert!(error.contains(&place.display().to_string()), "{error}");
        }
        let in_release = dir.join(RELEASE_SHARED_DIR).join(TABLES_DIR).join(TABLE.name);
        assert!(error.contains(&in_release.display().to_string()), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_checkout_folder_is_relative_to_the_defining_crate() {
        let file = BundledFile {
            name: "Data.bin",
            subdir: "",
            crate_dir: "workspace/lib/some-crate",
            checkout_dir_from_crate: "../../assets/./data",
        };
        assert_eq!(file.checkout_dir(), Path::new("workspace").join("assets").join("data"));
    }

    /// The checkout this crate builds from holds the dictionary where the release packaging copies it from.
    #[test]
    fn the_source_checkout_holds_the_hash_dictionary() {
        let path = HASH_DICTIONARY.checkout_dir().join(HASH_DICTIONARY.relative_path());
        assert!(path.is_file(), "{}", path.display());
    }
}
