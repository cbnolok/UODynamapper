//! The UOP explorer's text editor: the open text entries ([`EntryTextEdits`]), each decoded with its encoding and
//! line endings ([`EntryText`]) and saved through [`UopEntryEditor`] (its kind and parse checks, and the NIF check
//! uocf leaves to its callers: [`NIF_CHECK`]) to a copy of the package; "Replace file..." saves the same way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use color_eyre::eyre;
use uocf::audio::validate::FormatPolicy;
use image::{ImageFormat, RgbaImage};
use uocf::enhanced::dds_writer::SizeChange;
use uocf::enhanced::entry_edit::{EntryCheck, EntryEdit, EntryTarget, ParseCheck, UopEntryEditor};
use uocf::enhanced::entry_image::EntryImage;
use uocf::enhanced::entry_text::EntryText;
use uocf::enhanced::image_census::{name_extension, ContentFormat};
use uocf::enhanced::syntax_check::{lua_errors, xml_error, SyntaxError};
use uocf::uop_container::hash::hash_file_name_single;

/// The syntax uocf checks a text entry in ([`uocf::enhanced::syntax_check`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSyntax {
    Lua,
    Xml,
}

impl TextSyntax {
    /// The syntax of the text entry `name` whose original text is `original`: by its extension, else the one the
    /// original text reads in (an entry without a name).
    pub fn of(name: &str, original: &str) -> Option<Self> {
        match name_extension(name).as_deref() {
            Some(LUA_EXTENSION) => Some(Self::Lua),
            Some(XML_EXTENSION) => Some(Self::Xml),
            Some(_) => None,
            None if original.trim_start().starts_with(XML_START) && xml_error(original).is_none() => Some(Self::Xml),
            None => lua_errors(original).is_empty().then_some(Self::Lua),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Lua => "Lua",
            Self::Xml => "XML",
        }
    }

    pub fn first_error(self, text: &str) -> Option<SyntaxError> {
        match self {
            Self::Lua => lua_errors(text).into_iter().next(),
            Self::Xml => xml_error(text),
        }
    }
}

const LUA_EXTENSION: &str = "lua";
const XML_EXTENSION: &str = "xml";
const XML_START: char = '<';

/// One text entry being edited.
#[derive(Debug, Clone)]
pub struct EntryTextEdit {
    pub original: EntryText,
    pub buffer: String,
    /// The entry's name in the explorer.
    pub name: String,
    /// The text of the last saved copy.
    saved: Option<String>,
    /// The syntax of the entry, once known, and the text last checked with its first error.
    syntax: Option<Option<TextSyntax>>,
    checked: Option<(String, Option<SyntaxError>)>,
}

impl EntryTextEdit {
    pub fn modified(&self) -> bool {
        self.buffer != self.original.text
    }

    /// The text differs from the last saved copy, or from the package when no copy was saved.
    pub fn has_unsaved_changes(&self) -> bool {
        self.buffer != *self.saved.as_ref().unwrap_or(&self.original.text)
    }

    /// Records the current text as saved to a copy: the loaded package does not change, so it stays `modified`.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.buffer.clone());
    }

    /// The syntax of the entry named `name` and the first syntax error of the edited text, checked again only when
    /// the text changed.
    pub fn syntax_error(&mut self, name: &str) -> Option<(TextSyntax, Option<&SyntaxError>)> {
        let syntax = (*self.syntax.get_or_insert_with(|| TextSyntax::of(name, &self.original.text)))?;
        if self.checked.as_ref().is_none_or(|(text, _)| *text != self.buffer) {
            self.checked = Some((self.buffer.clone(), syntax.first_error(&self.buffer)));
        }
        Some((syntax, self.checked.as_ref().and_then(|(_, error)| error.as_ref())))
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
            self.open.insert(key.clone(), EntryTextEdit { original, buffer, name: String::new(), saved: None, syntax: None, checked: None });
        }
        self.open.get_mut(&key)
    }

    /// Forgets the edits of the entry.
    pub fn close(&mut self, package: &Path, hash: u64) {
        self.open.remove(&(package.to_path_buf(), hash));
    }

    pub fn any_unsaved(&self) -> bool {
        self.open.values().any(EntryTextEdit::has_unsaved_changes)
    }

    /// Package, hash and name of the entries with unsaved changes.
    pub fn unsaved_entries(&self) -> Vec<(PathBuf, u64, String)> {
        let unsaved = self.open.iter().filter(|(_, edit)| edit.has_unsaved_changes());
        unsaved.map(|((package, hash), edit)| (package.clone(), *hash, edit.name.clone())).collect()
    }

    /// Forgets the entries with unsaved changes: they open again as they are in the package.
    pub fn discard_unsaved(&mut self) {
        self.open.retain(|_, edit| !edit.has_unsaved_changes());
    }

    /// How many entries hold unsaved edits.
    #[cfg(test)]
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
    extensions: &[],
    dialects: false,
    check: |bytes, _| gamebryo_nif::read_checked(bytes).map(|_| ()).map_err(|error| eyre::eyre!(error)),
    keeps: None,
};

/// Writes a copy of the package at `source` to `output` with the entry `target` replaced by `bytes`, checked by
/// [`UopEntryEditor::replace`] and [`NIF_CHECK`].
pub fn save_entry_copy(source: &Path, target: &EntryTarget, bytes: &[u8], output: &Path) -> eyre::Result<EntryEdit> {
    let mut editor = UopEntryEditor::load(source)?.with_check(NIF_CHECK);
    let edit = editor.replace(target, bytes, FormatPolicy::VerifiedOnly)?;
    editor.save_as(output)?;
    Ok(edit)
}

/// The formats "Replace file..." offers to convert to an image entry's format (a TGA or DDS file is stored as it is).
const CONVERTED_IMAGE_FORMATS: [ImageFormat; 3] = [ImageFormat::Png, ImageFormat::Bmp, ImageFormat::Jpeg];

/// The image a picked file holds when it is one to convert (PNG, BMP, JPEG), as RGBA.
pub fn image_to_convert(file: &[u8]) -> eyre::Result<Option<RgbaImage>> {
    match image::guess_format(file) {
        Ok(format) if CONVERTED_IMAGE_FORMATS.contains(&format) => {
            Ok(Some(image::load_from_memory_with_format(file, format).map_err(|error| eyre::eyre!("not a readable {format:?}: {error}"))?.into_rgba8()))
        }
        _ => Ok(None),
    }
}

/// Writes a copy of the package at `source` to `output` with the image entry `target` replaced by `image` converted
/// to its format ([`UopEntryEditor::replace_image`]).
pub fn save_entry_image_copy(
    source: &Path,
    target: &EntryTarget,
    image: &RgbaImage,
    size_change: SizeChange,
    output: &Path,
) -> eyre::Result<(EntryImage, EntryEdit)> {
    let mut editor = UopEntryEditor::load(source)?.with_check(NIF_CHECK);
    let replaced = editor.replace_image(target, image, size_change)?;
    editor.save_as(output)?;
    Ok(replaced)
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

    #[test]
    fn shows_the_first_syntax_error_of_scripts_and_documents() {
        const HASH: u64 = 2;
        let mut edits = EntryTextEdits::default();
        let package = Path::new("/packages/Interface.uop");
        let edit = edits.open(package, HASH, b"x = 1\r\n").unwrap();
        assert_eq!(edit.syntax_error("data/interface/a.lua").map(|(syntax, error)| (syntax, error.is_some())), Some((TextSyntax::Lua, false)));
        edit.buffer.push_str("if x then\n");
        let (_, error) = edit.syntax_error("data/interface/a.lua").unwrap();
        assert_eq!(error.map(|error| (error.line, error.column)), Some((3, 1)));
        assert_eq!(TextSyntax::of("unnamed", "<a b=\"1\"/>"), Some(TextSyntax::Xml));
        assert_eq!(TextSyntax::of("unnamed", "x = 1"), Some(TextSyntax::Lua));
        assert_eq!(TextSyntax::of("data/x.csv", "x = 1"), None);
    }
}
