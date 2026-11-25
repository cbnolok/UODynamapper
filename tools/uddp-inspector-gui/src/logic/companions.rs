//! Companion packages: the packages a view reads alongside the opened one (a tilemeta's image packages, an effects
//! package's `hues.uddp`), looked up beside it, then in the package folder setting.

use std::path::{Path, PathBuf};

/// `file_name` beside `package_path`, else in `companion_dir`; `None` when neither holds it.
pub fn find_companion(package_path: &Path, companion_dir: Option<&Path>, file_name: &str) -> Option<PathBuf> {
    package_path.parent().into_iter().chain(companion_dir).map(|dir| dir.join(file_name)).find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_companion_beside_the_package_wins_over_the_package_folder() {
        let root = std::env::temp_dir().join(format!("uddp_inspector_companions_{}", std::process::id()));
        let (beside, folder) = (root.join("beside"), root.join("folder"));
        for dir in [&beside, &folder] {
            std::fs::create_dir_all(dir).expect("create dir");
        }
        std::fs::write(beside.join("both.uddp"), []).expect("write");
        std::fs::write(folder.join("both.uddp"), []).expect("write");
        std::fs::write(folder.join("folder_only.uddp"), []).expect("write");
        let package = beside.join("tilemeta.uddp");

        assert_eq!(find_companion(&package, Some(&folder), "both.uddp"), Some(beside.join("both.uddp")));
        assert_eq!(find_companion(&package, Some(&folder), "folder_only.uddp"), Some(folder.join("folder_only.uddp")));
        assert_eq!(find_companion(&package, None, "folder_only.uddp"), None);
        assert_eq!(find_companion(&package, Some(&folder), "absent.uddp"), None);
        std::fs::remove_dir_all(&root).expect("remove test dirs");
    }
}
