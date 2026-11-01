//! `speech.mul`, `skills.mul` with `Skills.idx`, and `skillgrp.mul` of real clients of every era read strictly
//! and write back byte for byte; a renamed skill and a renamed group change only themselves.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a file a client lacks is
//! skipped. Run with `--ignored`.

use std::path::{Path, PathBuf};

use uocf::classic::skills::{SKILL_GROUPS_FILE, SKILLS_DATA_FILE, SKILLS_INDEX_FILE, SkillGroupsFile, SkillsFile};
use uocf::classic::speech::{SPEECH_FILE, SpeechFile};
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// The name the edits give, longer than every shipped skill name so the later payloads move.
const EDITED_NAME: &str = "Edited Skill Name";
/// A group name that fits a field.
const EDITED_GROUP: &str = "Edited";

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn read(root: &Path, name: &str) -> Option<Vec<u8>> {
    find_file_ignoring_case(root, name).map(|path| std::fs::read(path).unwrap())
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn classic_speech_skills_and_skill_groups_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        if let Some(bytes) = read(&root, SPEECH_FILE) {
            let file = SpeechFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client} {SPEECH_FILE}: {error:#}"));
            assert_eq!(file.to_bytes().unwrap(), bytes, "{client} {SPEECH_FILE}");
            println!("{client}: {SPEECH_FILE} ({} records) written back byte for byte", file.entries.len());
            checked += 1;
        }
        if let (Some(index), Some(data)) = (read(&root, SKILLS_INDEX_FILE), read(&root, SKILLS_DATA_FILE)) {
            let mut file = SkillsFile::from_bytes(&index, &data).unwrap_or_else(|error| panic!("{client} {SKILLS_DATA_FILE}: {error:#}"));
            assert_eq!(file.to_bytes().unwrap(), (index, data), "{client} {SKILLS_DATA_FILE}");
            let present: Vec<usize> = (0..file.len()).filter(|&id| file.skill(id).unwrap().is_some()).collect();
            println!("{client}: {SKILLS_DATA_FILE} ({} skills) written back byte for byte", present.len());

            let before = file.container.clone();
            let mut skill = file.skill(0).unwrap().unwrap();
            skill.name = EDITED_NAME.to_owned();
            file.set_skill(0, Some(&skill)).unwrap();
            let (index, data) = file.to_bytes().unwrap();
            let back = SkillsFile::from_bytes(&index, &data).unwrap();
            assert_eq!(back.skill(0).unwrap().unwrap(), skill, "{client}: the renamed skill");
            for &id in &present[1..] {
                assert_eq!(back.container.entries[id], before.entries[id], "{client}: skill {id}");
            }
            assert_eq!(back.container.data_order, before.data_order, "{client}: the data order");
            checked += 1;
        }
        if let Some(bytes) = read(&root, SKILL_GROUPS_FILE) {
            let mut file = SkillGroupsFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client} {SKILL_GROUPS_FILE}: {error:#}"));
            assert_eq!(file.to_bytes().unwrap(), bytes, "{client} {SKILL_GROUPS_FILE}");
            println!(
                "{client}: {SKILL_GROUPS_FILE} ({} groups, {} skills, {:?}) written back byte for byte",
                file.groups.len() + 1,
                file.skill_groups.len(),
                file.encoding
            );
            file.groups[0].name = EDITED_GROUP.to_owned();
            let edited = file.to_bytes().unwrap();
            assert_eq!(edited.len(), bytes.len());
            let back = SkillGroupsFile::from_bytes(&edited).unwrap();
            assert_eq!(back.group_name(1), Some(EDITED_GROUP));
            assert_eq!((&back.groups[1..], &back.skill_groups), (&file.groups[1..], &file.skill_groups), "{client}");
            checked += 1;
        }
    }
    assert!(checked > 0);
}
