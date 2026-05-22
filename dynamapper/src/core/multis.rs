use bevy::prelude::*;

pub const MULTI_STATIC_GRAPHIC_OFFSET: u16 = 0x4000;
pub const MULTI_PART_VISIBLE_FLAG: u32 = 0x1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MultiPart {
    pub item_id: u16,
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

#[derive(Resource, Default)]
pub struct MultiDefinitionsRes {
    definitions: Vec<Vec<MultiPart>>,
}

impl MultiDefinitionsRes {
    pub fn from_classic_parts(parts: Vec<Vec<uocf::classic::multi::MultiPart>>) -> Self {
        Self {
            definitions: parts
                .into_iter()
                .map(|parts| {
                    parts
                        .into_iter()
                        .filter(|part| multi_part_is_visible(part.flags))
                        .map(|part| MultiPart {
                            item_id: part.item_id,
                            x: part.x,
                            y: part.y,
                            z: part.z,
                        })
                        .collect()
                })
                .collect(),
        }
    }

    pub fn from_ec_collection(collection: &uocf::enhanced::multis::MultiCollection) -> Self {
        let max_id = collection
            .items
            .iter()
            .map(|item| item.id)
            .max()
            .unwrap_or(0);
        let mut definitions = vec![Vec::new(); max_id as usize + 1];

        for item in &collection.items {
            definitions[item.id as usize] = item
                .parts
                .iter()
                .filter(|part| multi_part_is_visible(part.flags as u32))
                .map(|part| MultiPart {
                    item_id: part.item_id,
                    x: part.x,
                    y: part.y,
                    z: part.z,
                })
                .collect();
        }

        Self { definitions }
    }

    pub fn parts(&self, multi_id: u32) -> Option<&[MultiPart]> {
        self.definitions
            .get(multi_id as usize)
            .map(Vec::as_slice)
            .filter(|parts| !parts.is_empty())
    }

    pub fn definition_count(&self) -> usize {
        self.definitions
            .iter()
            .filter(|parts| !parts.is_empty())
            .count()
    }
}

pub fn multi_id_from_static_graphic(graphic: u16) -> Option<u32> {
    graphic
        .checked_sub(MULTI_STATIC_GRAPHIC_OFFSET)
        .map(u32::from)
}

fn multi_part_is_visible(flags: u32) -> bool {
    flags & MULTI_PART_VISIBLE_FLAG != 0
}

pub fn expanded_multi_part_z(base_z: i8, part_z: i16) -> i8 {
    (base_z as i16 + part_z).clamp(i8::MIN as i16, i8::MAX as i16) as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_id_only_uses_static_graphic_range() {
        assert_eq!(multi_id_from_static_graphic(0x3fff), None);
        assert_eq!(multi_id_from_static_graphic(0x4000), Some(0));
        assert_eq!(multi_id_from_static_graphic(0x4007), Some(7));
    }

    #[test]
    fn classic_conversion_keeps_only_visible_parts() {
        let definitions = MultiDefinitionsRes::from_classic_parts(vec![vec![
            uocf::classic::multi::MultiPart {
                item_id: 1,
                x: 0,
                y: 0,
                z: 0,
                flags: MULTI_PART_VISIBLE_FLAG,
            },
            uocf::classic::multi::MultiPart {
                item_id: 2,
                x: 0,
                y: 0,
                z: 0,
                flags: 0,
            },
        ]]);

        let parts = definitions.parts(0).expect("visible multi definition");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].item_id, 1);
    }

    #[test]
    fn expanded_z_clamps_to_static_tile_range() {
        assert_eq!(expanded_multi_part_z(120, 20), i8::MAX);
        assert_eq!(expanded_multi_part_z(-120, -20), i8::MIN);
        assert_eq!(expanded_multi_part_z(10, -3), 7);
    }
}
