//! The hand-written spec files of `assets/runtime_specs/` and the user's copies of them. A user copy sits at the same
//! path under the cache dir, `cache/runtime_specs/<folder>/<file>`, as the settings files' copies sit in
//! `cache/settings/`, and Dynamapper reads it in place of the bundled file when it exists. Both are strict
//! (`uot_kdl::strict`). The bundled file is required: a missing or invalid one stops the app. A user copy that does not
//! read leaves the bundled file in force, with a warning in the log and in game; whatever writes the copy (the
//! paperdoll menu) reads it again first and refuses it, so it is neither read nor overwritten until it is fixed.
//!
//! The lines logged go through `console_logger` (Settings), as those of the settings files' copies do: the `log`
//! crate's records below a warning do not reach Dynamapper's log (`bevy_log_filter::LOG_CRATE_MAX_LEVEL`).

use std::path::{Path, PathBuf};

use color_eyre::eyre;

use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::constants;
use crate::ingame_sysmessage_logger;

/// The folder of the runtime specs under the assets dir, and of the user's copies under the cache dir.
pub const RUNTIME_SPECS_DIR: &str = "runtime_specs";

/// A runtime spec file: the bundled file, and the user's copy of it, which may not exist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecFiles {
    pub bundled: PathBuf,
    pub user: PathBuf,
}

/// A spec as read, from the user's copy or the bundled file.
pub struct LoadedSpec<T> {
    pub value: T,
    /// The file it comes from.
    pub path: PathBuf,
    /// Why the user's copy, which exists, was not read: the bundled file applies in its place.
    pub user_error: Option<eyre::Report>,
}

/// What became of a user copy an older Dynamapper kept elsewhere ([`SpecFiles::move_old_copy`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OldCopy {
    /// There is none.
    Absent,
    /// It is the user's copy now.
    Moved,
    /// The user's copy exists already: the old one stays where it is, unread.
    Kept,
}

/// The text of `path`, with the path in the error.
pub fn read_text(path: &Path) -> eyre::Result<String> {
    std::fs::read_to_string(path).map_err(|error| eyre::eyre!("{}: cannot read: {error}", path.display()))
}

impl SpecFiles {
    /// `<folder>/<file_name>` of the runtime specs, under the app's assets and cache dirs.
    pub fn of(folder: &str, file_name: &str) -> Self {
        Self::under(&constants::valid_asset_dir(), &constants::valid_cache_dir(), folder, file_name)
    }

    /// `<folder>/<file_name>` of the runtime specs, under `asset_dir` and `cache_dir`.
    pub fn under(asset_dir: &Path, cache_dir: &Path, folder: &str, file_name: &str) -> Self {
        let relative = Path::new(RUNTIME_SPECS_DIR).join(folder).join(file_name);
        Self { bundled: asset_dir.join(&relative), user: cache_dir.join(relative) }
    }

    /// Decodes the bundled file, which must read, then the user's copy when it exists: the copy's value when it reads,
    /// else the bundled one with the copy's error. `decode` takes the file's path, for its errors, and its text.
    pub fn load<T>(&self, decode: impl Fn(&str, &str) -> eyre::Result<T>) -> eyre::Result<LoadedSpec<T>> {
        let decode_file = |path: &Path| decode(&path.display().to_string(), &read_text(path)?);
        let bundled = decode_file(&self.bundled)?;
        if !self.user.exists() {
            return Ok(LoadedSpec { value: bundled, path: self.bundled.clone(), user_error: None });
        }
        Ok(match decode_file(&self.user) {
            Ok(value) => LoadedSpec { value, path: self.user.clone(), user_error: None },
            Err(error) => LoadedSpec { value: bundled, path: self.bundled.clone(), user_error: Some(error) },
        })
    }

    /// Moves `old`, where an older Dynamapper kept the user's copy, to the user copy's place, only while there is no
    /// user copy.
    pub fn move_old_copy(&self, old: &Path) -> eyre::Result<OldCopy> {
        if !old.exists() {
            return Ok(OldCopy::Absent);
        }
        if self.user.exists() {
            return Ok(OldCopy::Kept);
        }
        if let Some(parent) = self.user.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| eyre::eyre!("{}: cannot create: {error}", parent.display()))?;
        }
        std::fs::rename(old, &self.user)
            .map_err(|error| eyre::eyre!("{}: cannot move to {}: {error}", old.display(), self.user.display()))?;
        Ok(OldCopy::Moved)
    }

    /// [`Self::move_old_copy`] at startup, logged: the move at info, an old copy left unread or a failed move as a
    /// warning.
    pub fn adopt_old_copy(&self, old: &Path) {
        let (severity, message) = match self.move_old_copy(old) {
            Ok(OldCopy::Absent) => return,
            Ok(OldCopy::Moved) => (LogSev::Info, format!("Moved {} to {}", old.display(), self.user.display())),
            Ok(OldCopy::Kept) => (
                LogSev::Warn,
                format!(
                    "{} is no longer read: {} takes its place; delete the old one",
                    old.display(),
                    self.user.display()
                ),
            ),
            Err(error) => (LogSev::Warn, format!("{error:#}")),
        };
        console_logger::one(severity, LogAbout::Settings, &message);
    }
}

impl<T> LoadedSpec<T> {
    /// Logs where `what` was read from, then, logged and shown in game, why the user's copy was not read; the error's
    /// text, if any.
    pub fn report(&self, what: &str) -> Option<String> {
        console_logger::one(LogSev::Info, LogAbout::Settings, &format!("Loaded {what} from {}", self.path.display()));
        let error = format!("{:#}", self.user_error.as_ref()?);
        let message = format!("{error}: {} applies until it is fixed.", self.path.display());
        console_logger::one(LogSev::Warn, LogAbout::Settings, &message);
        ingame_sysmessage_logger::warning(message);
        Some(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDER: &str = "paperdoll";
    const FILE_NAME: &str = "Spec.kdl";

    /// A spec of one number, for the tests.
    fn decode_number(source_name: &str, text: &str) -> eyre::Result<u32> {
        text.trim().parse().map_err(|error| eyre::eyre!("{source_name}: {error}"))
    }

    /// A fresh directory with `assets/` and `cache/`, and the files of `FOLDER/FILE_NAME` in it.
    fn scratch(name: &str) -> (PathBuf, SpecFiles) {
        let root = std::env::temp_dir().join(format!("dynamapper-runtime-specs-{name}-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        let files = SpecFiles::under(&root.join("assets"), &root.join("cache"), FOLDER, FILE_NAME);
        std::fs::create_dir_all(files.bundled.parent().unwrap()).unwrap();
        (root, files)
    }

    #[test]
    fn the_users_copy_mirrors_the_bundled_path_under_the_cache() {
        let files = SpecFiles::under(Path::new("/app/assets"), Path::new("/app/cache"), FOLDER, FILE_NAME);
        assert_eq!(files.bundled, Path::new("/app/assets/runtime_specs/paperdoll/Spec.kdl"));
        assert_eq!(files.user, Path::new("/app/cache/runtime_specs/paperdoll/Spec.kdl"));
    }

    /// The user's copy is read in place of the bundled file when it exists and reads; one that does not read leaves the
    /// bundled file in force, with its error; the bundled file must read either way.
    #[test]
    fn the_users_copy_is_read_in_place_of_the_bundled_file() {
        let (root, files) = scratch("load");
        std::fs::write(&files.bundled, "1").unwrap();
        let bundled = files.load(decode_number).unwrap();
        assert_eq!((bundled.value, bundled.path, bundled.user_error.is_none()), (1, files.bundled.clone(), true));

        std::fs::create_dir_all(files.user.parent().unwrap()).unwrap();
        std::fs::write(&files.user, "2").unwrap();
        let user = files.load(decode_number).unwrap();
        assert_eq!((user.value, user.path, user.user_error.is_none()), (2, files.user.clone(), true));

        std::fs::write(&files.user, "two").unwrap();
        let invalid = files.load(decode_number).unwrap();
        assert_eq!((invalid.value, invalid.path.clone()), (1, files.bundled.clone()));
        let error = format!("{:#}", invalid.user_error.unwrap());
        assert!(error.starts_with(&files.user.display().to_string()), "{error}");

        std::fs::write(&files.user, "2").unwrap();
        std::fs::write(&files.bundled, "one").unwrap();
        assert!(files.load(decode_number).is_err());
        std::fs::remove_file(&files.bundled).unwrap();
        assert!(files.load(decode_number).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// An old copy becomes the user's copy once, and never replaces one.
    #[test]
    fn an_old_copy_moves_to_the_users_copy_once() {
        let (root, files) = scratch("move");
        let old = root.join("cache").join("settings").join(FILE_NAME);
        assert_eq!(files.move_old_copy(&old).unwrap(), OldCopy::Absent);

        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, "3").unwrap();
        assert_eq!(files.move_old_copy(&old).unwrap(), OldCopy::Moved);
        assert!(!old.exists());
        assert_eq!(std::fs::read_to_string(&files.user).unwrap(), "3");
        assert_eq!(files.move_old_copy(&old).unwrap(), OldCopy::Absent);

        std::fs::write(&old, "4").unwrap();
        assert_eq!(files.move_old_copy(&old).unwrap(), OldCopy::Kept);
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "4");
        assert_eq!(std::fs::read_to_string(&files.user).unwrap(), "3");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
