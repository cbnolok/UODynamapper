//! Parser for classic `Bodyconv.def` animation-file redirects.

crate::eyre_imports!();

use std::collections::HashMap;
use std::path::Path;

use crate::classic::generic_def::DefReader;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BodyConvEntry {
    /// Animation source index used by `AnimMap`: 0 = anim.mul, 1 = anim2.mul, etc.
    pub file_index: u8,
    pub graphic: u16,
    pub mount_height: i8,
}

#[derive(Debug, Clone, Default)]
pub struct BodyConvDef {
    entries: HashMap<u16, BodyConvEntry>,
}

impl BodyConvDef {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let mut reader = DefReader::new(path.as_ref())?;
        let mut entries = HashMap::new();

        while reader.next() {
            let index = reader.read_int();
            if index < 0 || index > u16::MAX as i32 {
                continue;
            }
            let index = index as u16;

            for part_index in 1..reader.parts_count() {
                let graphic = reader.read_int();
                if graphic < 0 || graphic > u16::MAX as i32 {
                    continue;
                }

                entries.insert(index, BodyConvEntry {
                    file_index: part_index as u8,
                    graphic: graphic as u16,
                    mount_height: mounted_height_offset(index, part_index),
                });
            }
        }

        Ok(Self { entries })
    }

    pub fn get(&self, body: u16) -> Option<&BodyConvEntry> {
        self.entries.get(&body)
    }

    pub fn resolve(&self, body: u16) -> Option<BodyConvEntry> {
        self.entries.get(&body).copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u16, &BodyConvEntry)> {
        self.entries.iter()
    }
}

fn mounted_height_offset(body: u16, part_index: usize) -> i8 {
    match part_index {
        1 if body == 0x00C0 || body == 793 => -9,
        2 if body == 0x0579 => 9,
        4 if body == 0x0115 || body == 0x00C0 => 0,
        4 if body == 0x042D => 3,
        4 => -9,
        _ => 0,
    }
}
