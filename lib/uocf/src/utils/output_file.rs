//! Writing output files beside the files they were made from, without ever damaging those: an output that is a
//! source is refused by file identity ([`refuse_same_file`]), and an output is written whole to a temporary file
//! in its folder before it replaces anything ([`write_through_temp`]).

use std::io;
use std::path::{Path, PathBuf};

crate::eyre_imports!();

/// The suffix of the temporary file [`write_through_temp`] writes first.
const TEMP_SUFFIX: &str = "uocf-partial";

/// Whether `output` is the file `source`: the same path, a symbolic link to it or a hard link, compared by file
/// identity (device and inode, or the Windows volume serial number and file index). A missing `output` is not.
pub fn is_same_file(source: &Path, output: &Path) -> eyre::Result<bool> {
    match std::fs::metadata(output) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).wrap_err_with(|| format!("failed to read {}", output.display())),
        Ok(_) => {}
    }
    same_file::is_same_file(source, output)
        .wrap_err_with(|| format!("failed to compare {} with {}", output.display(), source.display()))
}

/// Fails when `output` is any of `sources` ([`is_same_file`]); `what` names the sources in the message.
pub fn refuse_same_file<'a>(sources: impl IntoIterator<Item = &'a Path>, output: &Path, what: &str) -> eyre::Result<()> {
    for source in sources {
        if is_same_file(source, output)? {
            eyre::bail!(
                "refusing to overwrite the {what} {}: {} is that file (or a link to it); write to a new path",
                source.display(),
                output.display()
            );
        }
    }
    Ok(())
}

/// The temporary file [`write_through_temp`] writes for `output`: hidden, in the same folder.
fn temp_path(output: &Path) -> eyre::Result<PathBuf> {
    let name = output.file_name().ok_or_else(|| eyre!("{} names no file", output.display()))?;
    Ok(output.with_file_name(format!(".{}.{}.{TEMP_SUFFIX}", name.to_string_lossy(), std::process::id())))
}

/// Writes `output` with `write`, which is handed a temporary path in the same folder; once it succeeds, the
/// temporary file is renamed over `output`. When it fails, the temporary file is removed and `output` is left as
/// it was.
pub fn write_through_temp<T>(output: &Path, write: impl FnOnce(&Path) -> eyre::Result<T>) -> eyre::Result<T> {
    let temp = temp_path(output)?;
    let result = write(&temp).and_then(|value| {
        std::fs::rename(&temp, output)
            .wrap_err_with(|| format!("failed to move {} to {}", temp.display(), output.display()))
            .map(|()| value)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_links_and_writes_whole_files() {
        let dir = std::env::temp_dir().join(format!("uocf_output_file_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.bin");
        std::fs::write(&source, b"source").unwrap();
        let hard_link = dir.join("hard_link.bin");
        std::fs::hard_link(&source, &hard_link).unwrap();
        let other = dir.join("other.bin");
        std::fs::write(&other, b"other").unwrap();

        assert!(is_same_file(&source, &source).unwrap());
        assert!(is_same_file(&source, &hard_link).unwrap(), "a hard link is the same file");
        assert!(!is_same_file(&source, &other).unwrap());
        assert!(!is_same_file(&source, &dir.join("missing.bin")).unwrap());
        #[cfg(unix)]
        {
            let symlink = dir.join("symlink.bin");
            std::os::unix::fs::symlink(&source, &symlink).unwrap();
            assert!(is_same_file(&source, &symlink).unwrap(), "a symbolic link is the same file");
        }
        let error = refuse_same_file([other.as_path(), source.as_path()], &hard_link, "source file").unwrap_err().to_string();
        assert!(error.contains("refusing to overwrite the source file"), "{error}");

        // A failed write leaves the output as it was, and no temporary file.
        let failed = write_through_temp(&other, |temp| -> eyre::Result<()> {
            std::fs::write(temp, b"half")?;
            eyre::bail!("failed halfway")
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read(&other).unwrap(), b"other");
        assert!(!temp_path(&other).unwrap().exists());
        write_through_temp(&other, |temp| Ok(std::fs::write(temp, b"new")?)).unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"new");
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
