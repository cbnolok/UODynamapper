//! Every shipped Lua script and XML document of the Enhanced Client (`UO_EC_CLIENT_DIR`) and Kingdom Reborn
//! (`UO_KR_CLIENT_DIR`) passes the syntax checks of replaced entries (`enhanced::syntax_check`). A Lua script is an
//! entry named `.lua`; an XML document is an entry of the XML kind, or UTF-16 text named `.xml`. Prints the counts
//! per package and the unnamed text entries that parse as Lua.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use uocf::enhanced::image_census::{classify_content, find_packages, name_extension, ContentFormat};
use color_eyre::eyre;
use uocf::enhanced::entry_text::EntryText;
use uocf::enhanced::syntax_check::{check_lua, check_xml, SyntaxError};
use uocf::uop_container::hash_dictionary::HashDictionary;
use uocf::uop_container::package::UopPackage;

const CLIENT_DIR_ENVS: [&str; 2] = ["UO_EC_CLIENT_DIR", "UO_KR_CLIENT_DIR"];
const DICTIONARY: &str = "../../tools/_shared_assets/Dictionary.dic";
const LUA_EXTENSION: &str = "lua";
const XML_EXTENSION: &str = "xml";

#[derive(Default, Debug)]
struct Counts {
    lua: usize,
    xml: usize,
    /// Text entries without a name that parse as Lua, and those that do not.
    unnamed_lua: usize,
    unnamed_other_text: usize,
    /// Named text entries of another extension that parse as Lua.
    other_named_lua: Vec<String>,
    /// UTF-16 text not named `.xml`: (name or hash, whether it reads as XML).
    other_utf16: Vec<(String, bool)>,
}

/// Characters of the failing line shown around the column.
const CONTEXT_CHARS: usize = 60;

/// The text around the line and column the error names.
fn context(payload: &[u8], error: &eyre::Report) -> String {
    let Some(error) = error.downcast_ref::<SyntaxError>() else { return String::new() };
    let Some(text) = EntryText::decode(payload) else { return String::new() };
    let line: Vec<char> = text.text.lines().nth(error.line.saturating_sub(1)).unwrap_or_default().chars().collect();
    let start = error.column.saturating_sub(CONTEXT_CHARS / 2).min(line.len());
    let end = (start + CONTEXT_CHARS).min(line.len());
    format!("\n    | {}", line[start..end].iter().collect::<String>())
}

#[test]
#[ignore = "needs real clients: set UO_EC_CLIENT_DIR and/or UO_KR_CLIENT_DIR and run with --ignored"]
fn every_shipped_script_and_document_passes() {
    let names = HashDictionary::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join(DICTIONARY)).unwrap();
    let mut failures = Vec::new();
    for dir in CLIENT_DIR_ENVS.iter().filter_map(std::env::var_os).map(PathBuf::from) {
        let mut counts: BTreeMap<String, Counts> = BTreeMap::new();
        for path in find_packages(&dir).unwrap() {
            let package = UopPackage::load(&path).unwrap();
            let package_name = path.file_name().unwrap().to_string_lossy().into_owned();
            for file in package.iter_files().filter(|file| !file.is_empty_slot()) {
                let hash = file.filename_hash();
                let payload = package.unpack_file_by_hash(hash).unwrap().unwrap();
                let kind = classify_content(&payload).format;
                let name = names.resolve(hash);
                let extension = name.and_then(name_extension);
                let label = name.map_or_else(|| format!("{hash:016X}"), str::to_owned);
                let entry = counts.entry(package_name.clone()).or_default();
                if extension.as_deref() == Some(LUA_EXTENSION) {
                    entry.lua += 1;
                    if let Err(error) = check_lua(&payload) {
                        failures.push(format!("{} {label} (Lua): {error:#}{}", path.display(), context(&payload, &error)));
                    }
                } else if kind == ContentFormat::Xml || (kind == ContentFormat::Utf16Text && extension.as_deref() == Some(XML_EXTENSION)) {
                    entry.xml += 1;
                    if let Err(error) = check_xml(&payload) {
                        failures.push(format!("{} {label} (XML): {error:#}{}", path.display(), context(&payload, &error)));
                    }
                } else if kind == ContentFormat::Utf16Text {
                    entry.other_utf16.push((label, check_xml(&payload).is_ok()));
                } else if kind == ContentFormat::Text {
                    match (name, check_lua(&payload).is_ok()) {
                        (None, true) => entry.unnamed_lua += 1,
                        (None, false) => entry.unnamed_other_text += 1,
                        (Some(name), true) => entry.other_named_lua.push(name.to_owned()),
                        (Some(_), false) => {}
                    }
                }
            }
        }
        for (package, counts) in counts.iter().filter(|(_, counts)| counts.lua + counts.xml + counts.unnamed_lua + counts.other_named_lua.len() + counts.other_utf16.len() > 0) {
            println!("{}/{package}: {counts:?}", dir.display());
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}
