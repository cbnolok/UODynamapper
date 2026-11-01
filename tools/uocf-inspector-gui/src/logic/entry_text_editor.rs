//! The UOP explorer's text editor: the open text entries ([`EntryTextEdits`]), each decoded with its encoding and
//! line endings ([`EntryText`]) and saved through [`UopEntryEditor`] (its kind and parse checks, and the NIF check
//! uocf leaves to its callers: [`NIF_CHECK`]) to a copy of the package; "Replace file..." saves the same way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use color_eyre::eyre;
use uocf::audio::validate::FormatPolicy;
use uocf::enhanced::entry_edit::{EntryCheck, EntryEdit, EntryTarget, ParseCheck, UopEntryEditor};
use uocf::enhanced::entry_text::EntryText;
use uocf::enhanced::image_census::ContentFormat;
use uocf::uop_container::hash::hash_file_name_single;

/// One text entry being edited.
#[derive(Debug, Clone)]
pub struct EntryTextEdit {
    pub original: EntryText,
    pub buffer: String,
}

impl EntryTextEdit {
    pub fn modified(&self) -> bool {
        self.buffer != self.original.text
    }

    /// The bytes the edited text is stored as.
    pub fn encoded(&self) -> eyre::Result<Vec<u8>> {
        self.original.encode(&self.buffer)
    }
}

/// The text entries opened in the explorer, by package path and entry hash; an entry keeps its edits while
/// another one is shown.
#[derive(Debug, Default)]
pub struct EntryTextEdits {
    open: HashMap<(PathBuf, u64), EntryTextEdit>,
}

impl EntryTextEdits {
    /// The edit of the entry `hash` of `package`, opened from `payload` when it is text; `None` for other kinds.
    pub fn open(&mut self, package: &Path, hash: u64, payload: &[u8]) -> Option<&mut EntryTextEdit> {
        let key = (package.to_path_buf(), hash);
        if !self.open.contains_key(&key) {
            let original = EntryText::decode(payload)?;
            let buffer = original.text.clone();
            self.open.insert(key.clone(), EntryTextEdit { original, buffer });
        }
        self.open.get_mut(&key)
    }

    /// Forgets the edits of the entry.
    pub fn close(&mut self, package: &Path, hash: u64) {
        self.open.remove(&(package.to_path_buf(), hash));
    }

    /// How many entries hold unsaved edits.
    pub fn modified_count(&self) -> usize {
        self.open.values().filter(|edit| edit.modified()).count()
    }
}

/// The entry `hash` by `name` when the name hashes to it, else by hash.
pub fn entry_target(name: &str, hash: u64) -> EntryTarget {
    if hash_file_name_single(name) == hash { EntryTarget::Name(name.to_owned()) } else { EntryTarget::Hash(hash) }
}

/// A replaced NIF must read with `gamebryo-nif` and have no wrong link (the effects import checks the same).
pub const NIF_CHECK: ParseCheck = ParseCheck {
    label: "NIF model",
    packages: &[],
    kinds: &[ContentFormat::Nif],
    name: None,
    dialects: false,
    check: |bytes, _| gamebryo_nif::read_checked(bytes).map(|_| ()).map_err(|error| eyre::eyre!(error)),
};

/// Writes a copy of the package at `source` to `output` with the entry `target` replaced by `bytes`, checked by
/// [`UopEntryEditor::replace`] and [`NIF_CHECK`].
pub fn save_entry_copy(source: &Path, target: &EntryTarget, bytes: &[u8], output: &Path) -> eyre::Result<EntryEdit> {
    let mut editor = UopEntryEditor::load(source)?.with_check(NIF_CHECK);
    let edit = editor.replace(target, bytes, FormatPolicy::VerifiedOnly)?;
    editor.save_as(output)?;
    Ok(edit)
}

/// "checked: <reader>" or "<kind> by content".
pub fn check_summary(check: &EntryCheck) -> String {
    match check {
        EntryCheck::Parsed { reader, .. } => format!("checked: {reader}"),
        EntryCheck::Kind(kind) | EntryCheck::Unchecked(kind) => format!("{} by content", kind.label()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_edits_per_entry_and_encodes_like_the_entry() {
        const HASH: u64 = 1;
        let mut edits = EntryTextEdits::default();
        let package = Path::new("/packages/GameData.uop");
        assert!(edits.open(package, HASH, &[0, 1, 2, 0xff]).is_none(), "binary is not text");
        let edit = edits.open(package, HASH, b"a,b\r\n1,2\r\n").unwrap();
        assert_eq!(edit.buffer, "a,b\n1,2\n");
        edit.buffer.push_str("3,4\n");
        assert_eq!(edits.modified_count(), 1);
        let edit = edits.open(package, HASH, b"ignored once open").unwrap();
        assert_eq!(edit.encoded().unwrap(), b"a,b\r\n1,2\r\n3,4\r\n");
        edits.close(package, HASH);
        assert_eq!(edits.modified_count(), 0);
        assert_eq!(entry_target("data/x.csv", hash_file_name_single("data/x.csv")), EntryTarget::Name("data/x.csv".to_owned()));
        assert_eq!(entry_target("unnamed", HASH), EntryTarget::Hash(HASH));
    }
}
