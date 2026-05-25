use std::path::{Path, PathBuf};

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

pub fn find_first_existing_file(source_dirs: &[PathBuf], file_names: &[&str]) -> Option<PathBuf> {
    source_dirs.iter().find_map(|dir| {
        file_names
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

pub fn find_first_dir_matching(source_dirs: &[PathBuf], required_file_sets: &[&[&str]]) -> Option<PathBuf> {
    source_dirs.iter().find_map(|dir| {
        required_file_sets
            .iter()
            .any(|file_set| file_set.iter().all(|name| dir.join(name).is_file()))
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

pub fn resolve_output_path(source_dirs: &[PathBuf], output: &Path) -> PathBuf {
    if output.is_absolute() {
        output.to_path_buf()
    } else {
        source_dirs
            .first()
            .map(|dir| dir.join(output))
            .unwrap_or_else(|| output.to_path_buf())
    }
}
