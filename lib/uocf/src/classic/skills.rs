//! `skills.mul` with `Skills.idx`, and `skillgrp.mul`, written back byte for byte.
//!
//! - **`skills.mul`** ([`SkillsFile`], an [`IndexedMul`]): one payload per skill, a `u8` 1 for a skill with a
//!   button (an action), else 0, then the name and a NUL. `Skills.idx` has 256 records; the absent ones are
//!   `(-1, 0, 0)`, the extras of the present ones are kept as read. From 7.0.20 on the last three payloads are
//!   Throwing, Imbuing, Mysticism (ids 57, 56, 55): the data order read is kept. UO Gold has 52 skills (Gold's
//!   `skills.mul` 54), Mondain's Legacy 55, 7.0.20 and later 58.
//! - **`skillgrp.mul`** ([`SkillGroupsFile`]): an `i32` group count, group 0 ("Miscellaneous", not written)
//!   included, the other groups' names in [`GROUP_NAME_BYTES`]-byte fields (NUL-padded), then the group of each
//!   skill as an `i32`, to the end of the file. ClassicUO also reads a Unicode form: an `i32` -1, the count, and
//!   UTF-16 names of twice the bytes; no shipped file has it. UO Gold's name fields hold old text after the NUL
//!   (`Actions\0es\0kills\0`), so the fields are kept as read and only a renamed group is written anew.

crate::eyre_imports!();
use std::path::Path;

use crate::classic::indexed_mul::{IndexedEntry, IndexedMul};
use crate::utils::output_file::write_through_temp;

pub const SKILLS_DATA_FILE: &str = "skills.mul";
pub const SKILLS_INDEX_FILE: &str = "skills.idx";
pub const SKILL_GROUPS_FILE: &str = "skillgrp.mul";
/// Bytes of a group name field, its NUL included.
pub const GROUP_NAME_BYTES: usize = 17;
/// The first `i32` of the Unicode `skillgrp.mul`.
const UNICODE_MARK: i32 = -1;
/// Bytes of a UTF-16 code unit.
const UTF16_UNIT_BYTES: usize = 2;
const INT_BYTES: usize = 4;
const ACTION: u8 = 1;
const NO_ACTION: u8 = 0;
const NUL: u8 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    /// The skill has a button in the skills gump.
    pub action: bool,
    /// Latin-1.
    pub name: String,
}

impl Skill {
    pub fn decode(payload: &[u8]) -> eyre::Result<Self> {
        let (&flag, rest) = payload.split_first().ok_or_else(|| eyre!("an empty skill record"))?;
        let action = match flag {
            ACTION => true,
            NO_ACTION => false,
            other => eyre::bail!("action flag {other}, not {NO_ACTION} or {ACTION}"),
        };
        let name = rest.strip_suffix(&[NUL]).ok_or_else(|| eyre!("the name has no NUL at its end"))?;
        if name.contains(&NUL) {
            eyre::bail!("a NUL inside the name");
        }
        Ok(Self { action, name: name.iter().map(|&byte| char::from(byte)).collect() })
    }

    pub fn encode(&self) -> eyre::Result<Vec<u8>> {
        let mut payload = vec![if self.action { ACTION } else { NO_ACTION }];
        for c in self.name.chars() {
            let byte = u8::try_from(u32::from(c)).ok().filter(|&byte| byte != NUL);
            payload.push(byte.ok_or_else(|| eyre!("the name {:?} has {c:?}, which is no Latin-1 letter", self.name))?);
        }
        payload.push(NUL);
        Ok(payload)
    }
}

pub struct SkillsFile {
    pub container: IndexedMul,
}

impl SkillsFile {
    pub fn from_bytes(index: &[u8], data: &[u8]) -> eyre::Result<Self> {
        let file = Self { container: IndexedMul::from_bytes(index, data)? };
        for id in 0..file.len() {
            file.skill(id).wrap_err_with(|| format!("skill {id}"))?;
        }
        Ok(file)
    }

    pub fn load(client_dir: &Path) -> eyre::Result<Self> {
        let find = |name: &str| {
            crate::utils::path::find_file_ignoring_case(client_dir, name).ok_or_else(|| eyre!("{name} not in {}", client_dir.display()))
        };
        let (index, data) = (find(SKILLS_INDEX_FILE)?, find(SKILLS_DATA_FILE)?);
        Self::from_bytes(&std::fs::read(&index)?, &std::fs::read(&data)?).wrap_err_with(|| format!("{}", data.display()))
    }

    pub fn len(&self) -> usize {
        self.container.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.container.entries.is_empty()
    }

    pub fn skill(&self, id: usize) -> eyre::Result<Option<Skill>> {
        self.container.entries.get(id).and_then(IndexedEntry::payload).map(Skill::decode).transpose()
    }

    /// Replaces skill `id` (a new one gets extra 0); `None` clears it.
    pub fn set_skill(&mut self, id: usize, skill: Option<&Skill>) -> eyre::Result<()> {
        match skill {
            Some(skill) => {
                let extra = self.container.entries.get(id).and_then(IndexedEntry::extra).unwrap_or(0);
                self.container.set(id, IndexedEntry::Present { extra, payload: skill.encode()? });
            }
            None => self.container.clear(id),
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> eyre::Result<(Vec<u8>, Vec<u8>)> {
        self.container.to_bytes()
    }

    pub fn save(&self, index_path: &Path, data_path: &Path) -> eyre::Result<()> {
        self.container.save(index_path, data_path)
    }
}

/// How `skillgrp.mul` stores the group names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupNameEncoding {
    /// [`GROUP_NAME_BYTES`] bytes of Latin-1 (every shipped file).
    Latin1,
    /// Twice the bytes of UTF-16LE, after a leading `i32` -1 (ClassicUO).
    Unicode,
}

impl GroupNameEncoding {
    fn field_bytes(self) -> usize {
        match self {
            Self::Latin1 => GROUP_NAME_BYTES,
            Self::Unicode => GROUP_NAME_BYTES * UTF16_UNIT_BYTES,
        }
    }

    fn decode(self, field: &[u8]) -> String {
        match self {
            Self::Latin1 => field.iter().take_while(|&&byte| byte != NUL).map(|&byte| char::from(byte)).collect(),
            Self::Unicode => {
                let units: Vec<u16> = field.chunks_exact(UTF16_UNIT_BYTES).map(|unit| u16::from_le_bytes([unit[0], unit[1]])).collect();
                let len = units.iter().position(|&unit| unit == 0).unwrap_or(units.len());
                String::from_utf16_lossy(&units[..len])
            }
        }
    }

    fn encode(self, name: &str) -> eyre::Result<Vec<u8>> {
        let mut field = match self {
            Self::Latin1 => name
                .chars()
                .map(|c| u8::try_from(u32::from(c)).ok().filter(|&byte| byte != NUL))
                .collect::<Option<Vec<u8>>>()
                .ok_or_else(|| eyre!("the group name {name:?} is not Latin-1"))?,
            Self::Unicode => name.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        };
        // The name keeps a NUL after it.
        let room = self.field_bytes() - self.field_bytes() / GROUP_NAME_BYTES;
        if field.len() > room {
            eyre::bail!("the group name {name:?} takes {} bytes; a field holds {room} and a NUL", field.len());
        }
        field.resize(self.field_bytes(), NUL);
        Ok(field)
    }
}

/// A group name field, kept as read until the name changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillGroupName {
    pub name: String,
    field: Option<Vec<u8>>,
}

impl SkillGroupName {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), field: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillGroupsFile {
    pub encoding: GroupNameEncoding,
    /// The groups from 1 on (group 0 has no name in the file).
    pub groups: Vec<SkillGroupName>,
    /// The group of each skill, by skill id.
    pub skill_groups: Vec<i32>,
}

fn read_i32(bytes: &[u8], at: usize) -> eyre::Result<i32> {
    let field = bytes.get(at..at + INT_BYTES).ok_or_else(|| eyre!("the file ends at {}, inside an integer at {at}", bytes.len()))?;
    Ok(i32::from_le_bytes(field.try_into().expect("four bytes")))
}

impl SkillGroupsFile {
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut at = 0;
        let mut count = read_i32(bytes, at)?;
        at += INT_BYTES;
        let encoding = if count == UNICODE_MARK {
            count = read_i32(bytes, at)?;
            at += INT_BYTES;
            GroupNameEncoding::Unicode
        } else {
            GroupNameEncoding::Latin1
        };
        let named = usize::try_from(count).ok().and_then(|count| count.checked_sub(1)).ok_or_else(|| eyre!("a group count of {count}"))?;
        let field_bytes = encoding.field_bytes();
        let names_end = named.checked_mul(field_bytes).and_then(|len| len.checked_add(at)).filter(|&end| end <= bytes.len());
        let names_end = names_end.ok_or_else(|| eyre!("{named} group names of {field_bytes} bytes do not fit the {} bytes", bytes.len()))?;
        let groups = bytes[at..names_end]
            .chunks_exact(field_bytes)
            .map(|field| SkillGroupName { name: encoding.decode(field), field: Some(field.to_vec()) })
            .collect();
        let rest = &bytes[names_end..];
        if rest.len() % INT_BYTES != 0 {
            eyre::bail!("{} bytes after the group names are no whole number of skill groups", rest.len());
        }
        let skill_groups: Vec<i32> = rest.chunks_exact(INT_BYTES).map(|int| i32::from_le_bytes(int.try_into().expect("four bytes"))).collect();
        if let Some((skill, group)) = skill_groups.iter().enumerate().find(|(_, group)| **group < 0 || **group >= count) {
            eyre::bail!("skill {skill} is in group {group}; the file has groups 0 to {}", count - 1);
        }
        Ok(Self { encoding, groups, skill_groups })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).wrap_err_with(|| format!("{}", path.display()))
    }

    /// The name of `group`; group 0 has none in the file.
    pub fn group_name(&self, group: usize) -> Option<&str> {
        group.checked_sub(1).and_then(|index| self.groups.get(index)).map(|group| group.name.as_str())
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let count = i32::try_from(self.groups.len() + 1).map_err(|_| eyre!("{} groups", self.groups.len()))?;
        let mut bytes = Vec::new();
        if self.encoding == GroupNameEncoding::Unicode {
            bytes.extend_from_slice(&UNICODE_MARK.to_le_bytes());
        }
        bytes.extend_from_slice(&count.to_le_bytes());
        for group in &self.groups {
            match &group.field {
                Some(field) if field.len() == self.encoding.field_bytes() && self.encoding.decode(field) == group.name => {
                    bytes.extend_from_slice(field);
                }
                _ => bytes.extend(self.encoding.encode(&group.name)?),
            }
        }
        for (skill, group) in self.skill_groups.iter().enumerate() {
            if *group < 0 || *group >= count {
                eyre::bail!("skill {skill} is in group {group}; the file has groups 0 to {}", count - 1);
            }
            bytes.extend_from_slice(&group.to_le_bytes());
        }
        Ok(bytes)
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let bytes = self.to_bytes()?;
        write_through_temp(path, |temp| std::fs::write(temp, &bytes).wrap_err_with(|| format!("write {}", temp.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_records_read_and_write() {
        let skill = Skill::decode(b"\x01Anatomy\0").unwrap();
        assert_eq!(skill, Skill { action: true, name: "Anatomy".into() });
        assert_eq!(skill.encode().unwrap(), b"\x01Anatomy\0");
        assert!(Skill::decode(b"\x02A\0").is_err());
        assert!(Skill::decode(b"\x00A").is_err());
        assert!(Skill::decode(b"\x00A\0B\0").is_err());
    }

    #[test]
    fn skill_groups_keep_their_fields_and_write_renames_anew() {
        let mut bytes = 3i32.to_le_bytes().to_vec();
        bytes.extend(b"Actions\0es\0kills\0");
        bytes.extend(b"Lore\0\0\0\0\0\0\0\0\0\0\0\0\0");
        for group in [0i32, 2, 1] {
            bytes.extend(group.to_le_bytes());
        }
        let mut file = SkillGroupsFile::from_bytes(&bytes).unwrap();
        assert_eq!((file.group_name(1), file.group_name(2), file.skill_groups.as_slice()), (Some("Actions"), Some("Lore"), &[0, 2, 1][..]));
        assert_eq!(file.to_bytes().unwrap(), bytes);
        file.groups[0].name = "Combat".into();
        assert_eq!(&file.to_bytes().unwrap()[INT_BYTES..INT_BYTES + GROUP_NAME_BYTES], b"Combat\0\0\0\0\0\0\0\0\0\0\0");
        file.groups.push(SkillGroupName::new("x".repeat(GROUP_NAME_BYTES)));
        assert!(file.to_bytes().is_err());

        let unicode = SkillGroupsFile { encoding: GroupNameEncoding::Unicode, groups: vec![SkillGroupName::new("Magie")], skill_groups: vec![1] };
        let back = SkillGroupsFile::from_bytes(&unicode.to_bytes().unwrap()).unwrap();
        assert_eq!((back.encoding, back.group_name(1)), (GroupNameEncoding::Unicode, Some("Magie")));
        assert!(SkillGroupsFile::from_bytes(&[2, 0, 0, 0]).is_err());
    }
}
