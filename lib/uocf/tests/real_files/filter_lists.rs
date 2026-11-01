//! `Uobscene.lst` and `Obscene.lst` (Classic) and `badwords.lst` (KR, EC) of real clients read strictly and
//! write back byte for byte; an added word reads back and its removal restores the file. The schemes are checked
//! against each other: every `Obscene.lst` word is a `Uobscene.lst` word, and the KR `badwords.lst` holds the
//! `Uobscene.lst` words. The words are never printed.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a file a client lacks is
//! skipped. Run with `--ignored`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use uocf::classic::filter_lists::{BADWORDS_FILE, FilterList, FilterListScheme, OBSCENE_FILE, UOBSCENE_FILE};
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// The word the edit adds: printable ASCII, so every scheme takes it.
const ADDED_WORD: &str = "uocf test word";

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Reads `name` of `root`, checks the round trip and an add/remove edit; its words, or `None` when the client
/// lacks the file.
fn check_list(root: &Path, name: &str) -> Option<Vec<String>> {
    let client = label(root);
    let path = find_file_ignoring_case(root, name)?;
    let bytes = std::fs::read(&path).unwrap();
    let mut list = FilterList::load(&path).unwrap_or_else(|error| panic!("{client} {name}: {error:#}"));
    assert_eq!(list.to_bytes(), bytes, "{client} {name}: written back byte for byte");
    let words = list.words().to_vec();
    assert!(!words.is_empty(), "{client} {name}: no word");
    list.push(ADDED_WORD).unwrap();
    let edited = list.to_bytes();
    let back = FilterList::from_bytes(list.scheme(), &edited).unwrap();
    assert_eq!(back.words().last().map(String::as_str), Some(ADDED_WORD), "{client} {name}: the added word");
    assert_eq!(back.words().len(), words.len() + 1, "{client} {name}");
    list.remove(words.len()).unwrap();
    assert_eq!(list.to_bytes(), bytes, "{client} {name}: removing the word restores the file");
    println!("{client}: {name} ({} words, {} bytes) written back byte for byte", words.len(), bytes.len());
    Some(words)
}

#[test]
#[ignore = "needs real clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn filter_lists_write_back_byte_for_byte() {
    let (mut uobscene, mut obscene, mut badwords) = (Vec::new(), Vec::new(), Vec::new());
    for root in client_roots() {
        uobscene.extend(check_list(&root, UOBSCENE_FILE));
        obscene.extend(check_list(&root, OBSCENE_FILE));
        badwords.extend(check_list(&root, BADWORDS_FILE));
    }
    assert!(!uobscene.is_empty() || !obscene.is_empty() || !badwords.is_empty(), "no client ships a filter list");
    assert_eq!(FilterListScheme::for_file_name(OBSCENE_FILE), Some(FilterListScheme::Rotated));
    if let Some(reference) = uobscene.first() {
        let reference: BTreeSet<_> = reference.iter().collect();
        for words in &obscene {
            assert!(words.iter().all(|word| reference.contains(word)), "every Obscene.lst word is a Uobscene.lst word");
        }
        // The KR list is Uobscene.lst unshifted; the EC one differs from it in one line.
        let same = badwords.iter().filter(|words| words.iter().collect::<BTreeSet<_>>() == reference).count();
        assert!(badwords.is_empty() || same > 0, "a badwords.lst holds the Uobscene.lst words");
        println!("badwords.lst lists equal to Uobscene.lst: {same} of {}", badwords.len());
    }
}
