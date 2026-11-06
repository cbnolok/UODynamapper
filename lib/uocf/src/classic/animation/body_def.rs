//! Parser for classic `Body.def` body redirects.

crate::eyre_imports!();

use std::collections::HashMap;
use std::path::Path;

use crate::classic::generic_def::DefReader;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BodyDefEntry {
    pub graphic: u16,
    pub hue: u16,
}

#[derive(Debug, Clone, Default)]
pub struct BodyDef {
    entries: HashMap<u16, BodyDefEntry>,
}

impl BodyDef {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let mut reader = DefReader::new(path.as_ref())?;
        let mut entries = HashMap::new();

        while reader.next() {
            let index = reader.read_int();
            if index < 0 || index > u16::MAX as i32 {
                continue;
            }

            let Some(group) = reader.read_group() else {
                continue;
            };
            if group.is_empty() {
                continue;
            }

            let graphic = if group.len() >= 3 { group[2] } else { group[0] };
            if graphic < 0 || graphic > u16::MAX as i32 {
                continue;
            }

            let hue = reader.read_int();
            let hue = if hue < 0 { 0 } else { hue.min(u16::MAX as i32) as u16 };

            entries.entry(index as u16).or_insert(BodyDefEntry {
                graphic: graphic as u16,
                hue,
            });
        }

        Ok(Self { entries })
    }

    pub fn get(&self, body: u16) -> Option<&BodyDefEntry> {
        self.entries.get(&body)
    }

    pub fn resolve(&self, body: u16) -> Option<BodyDefEntry> {
        self.entries.get(&body).copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u16, &BodyDefEntry)> {
        self.entries.iter()
    }
}
