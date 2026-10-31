use std::path::{Path, PathBuf};

use uocf::utils::path::find_file_ignoring_case;

pub fn gather_source_dirs(ccdir: Option<&PathBuf>, ecdir: Option<&PathBuf>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(path) = ccdir {
        dirs.push(path.clone());
    }

    if let Some(path) = ecdir {
        if dirs.iter().all(|existing| existing != path) {
            dirs.push(path.clone());
        }
    }

    dirs
}

/// The first of `file_names` found in `source_dirs`, matching names ignoring ASCII case (client folders mix
/// cases, such as UO Gold's `Anim.idx`).
pub fn find_first_existing_file(source_dirs: &[PathBuf], file_names: &[&str]) -> Option<PathBuf> {
    source_dirs.iter().find_map(|dir| {
        file_names
            .iter()
            .find_map(|name| find_file_ignoring_case(dir, name))
    })
}

pub fn find_first_dir_matching(source_dirs: &[PathBuf], required_file_sets: &[&[&str]]) -> Option<PathBuf> {
    source_dirs.iter().find_map(|dir| {
        required_file_sets
            .iter()
            .any(|file_set| file_set.iter().all(|name| find_file_ignoring_case(dir, name).is_some()))
            .then(|| dir.clone())
    })
}

pub fn source_path_label(source_root: &Path, path: &Path) -> String {
    path.strip_prefix(source_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub fn source_path_label_from_dirs(source_dirs: &[PathBuf], path: &Path) -> String {
    source_dirs
        .iter()
        .find_map(|dir| path.strip_prefix(dir).ok())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Resolves a packer output path the way ordinary command-line tools do: an
/// absolute path is kept as is, and a relative path is taken against the current
/// working directory, never against a source (client) directory.
///
/// If the working directory cannot be read, the relative path is returned
/// unchanged; the operating system then resolves it against the same directory.
pub fn resolve_output_path(output: &Path) -> PathBuf {
    if output.is_absolute() {
        return output.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(output))
        .unwrap_or_else(|_| output.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_output_resolves_against_working_directory() {
        let output = Path::new("effects_ec.uddp");
        let cwd = std::env::current_dir().expect("read working directory");

        assert_eq!(resolve_output_path(output), cwd.join(output));
    }

    #[test]
    fn nested_relative_output_resolves_against_working_directory() {
        let output = Path::new("packages").join("hues.uddp");
        let cwd = std::env::current_dir().expect("read working directory");

        assert_eq!(resolve_output_path(&output), cwd.join(&output));
    }

    #[test]
    fn absolute_output_is_kept() {
        let output = std::env::temp_dir().join("tilemeta.uddp");

        assert!(output.is_absolute());
        assert_eq!(resolve_output_path(&output), output);
    }
}
