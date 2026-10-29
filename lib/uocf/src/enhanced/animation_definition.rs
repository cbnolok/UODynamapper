//! Enhanced Client `AnimationDefinition.uop`: item (tile) animations.
//!
//! One entry per animated item id, named `build/animationdefinition/%08d.bin` (see
//! [`AnimationDefinition::entry_path`]). An entry holds frame sets of tile ids with a frame
//! interval. It has nothing to do with mobile bodies: there is no body redirect in this file
//! (mobile frames are keyed by the `AnimationSequence` stored id).
//!
//! Layout, field meanings and confidence levels follow
//! `scripts/dev-helpers/ec-research/ghidra-animation/results/uosa_animation.md`, verified against
//! `UOSA.exe` (`UOAnimationDefinition::Parse` 0x004b8d80, `ParseFrameSets` 0x004b8ec0). All values
//! are little-endian and unaligned:
//!
//! ```text
//! u32 stored_id, u32 start_set, u32 second_set, f32 value
//! u32 set_count
//! set_count x {
//!     u32 key, i32 next, f32 frame_interval
//!     u32 frame_count
//!     frame_count x { u32 tile_id, u8 effect_count, effect_count x EffectDefinition }
//! }
//! ```
//!
//! Consumers: `UOObjectComponentAnimationTile::LoadAnimationDefinition` (animated items) and the
//! projectile and tile effect loaders; an item without an entry is drawn as a static tile.

use crate::enhanced::effect_definition::{read_effect_definitions, EffectDefinition};
use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{ensure, eyre, Result};
use std::collections::BTreeMap;
use std::io::Cursor;

/// File value of a frame set `next` for a set without a successor.
const NO_NEXT_SET: i32 = -1;

/// One `AnimationDefinition.uop` entry (`UOAnimationDefinition`).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationDefinition {
    /// Id stored in the entry; 3 entries differ from their name (7905 -> 7897, 7923 -> 7916,
    /// 18153 -> 18150). Confirmed in the data, use unknown.
    pub stored_id: u32,
    /// Key of the frame set the item starts with (confirmed); must exist.
    pub start_set: u32,
    /// Key of a second frame set (confirmed: must exist); its use is unknown. Equal to
    /// `start_set` in all data.
    pub second_set: u32,
    /// Must be >= 0 (confirmed); meaning unknown, 0.0 in all data.
    pub value: f32,
    /// Frame sets by key (confirmed). One set, key 0, in all data.
    pub sets: BTreeMap<u32, FrameSet>,
}

/// One frame set of an item animation.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameSet {
    /// Key of the set played after this one (`SetNext`, confirmed); `None` in all data.
    pub next: Option<u32>,
    /// Seconds per frame (confirmed); 0.0 to 0.9375 in the data.
    pub frame_interval: f32,
    /// Frames in play order (confirmed).
    pub frames: Vec<DefinitionFrame>,
}

/// One frame of an item animation (`UOAnimationDefinitionBinaryFactory::ReadFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct DefinitionFrame {
    /// Item graphic drawn for the frame (confirmed).
    pub tile_id: u32,
    /// Effects started when the frame is shown (confirmed in the code); none in the data.
    pub effects: Vec<EffectDefinition>,
}

impl AnimationDefinition {
    /// Entry name of the animation of an item id: `build/animationdefinition/%08d.bin`.
    pub fn entry_path(item_id: u32) -> String {
        format!("build/animationdefinition/{item_id:08}.bin")
    }

    /// Parses one entry, which must end exactly after its frame sets. Like the client, rejects a
    /// negative `value` and start or second set keys that name no set. Repeated set keys are
    /// errors: the client's handling was not traced and no client entry has one.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut reader = Cursor::new(data);
        let stored_id = reader.read_u32::<LittleEndian>()?;
        let start_set = reader.read_u32::<LittleEndian>()?;
        let second_set = reader.read_u32::<LittleEndian>()?;
        let value = reader.read_f32::<LittleEndian>()?;
        ensure!(value >= 0.0, "animation definition value {value} is negative");

        let mut sets = BTreeMap::new();
        for _ in 0..reader.read_u32::<LittleEndian>()? {
            let key = reader.read_u32::<LittleEndian>()?;
            let next = match reader.read_i32::<LittleEndian>()? {
                NO_NEXT_SET => None,
                next => Some(u32::try_from(next).map_err(|_| eyre!("frame set {key} has negative next {next}"))?),
            };
            let frame_interval = reader.read_f32::<LittleEndian>()?;
            let mut frames = Vec::new();
            for _ in 0..reader.read_u32::<LittleEndian>()? {
                let tile_id = reader.read_u32::<LittleEndian>()?;
                let effect_count = usize::from(reader.read_u8()?);
                let effects = read_effect_definitions(&mut reader, effect_count)?;
                frames.push(DefinitionFrame { tile_id, effects });
            }
            let set = FrameSet { next, frame_interval, frames };
            ensure!(sets.insert(key, set).is_none(), "repeated frame set key {key}");
        }

        let trailing = data.len() - reader.position() as usize;
        ensure!(trailing == 0, "{trailing} trailing bytes after the animation definition");
        for (role, key) in [("start", start_set), ("second", second_set)] {
            ensure!(sets.contains_key(&key), "{role} frame set {key} does not exist");
        }
        Ok(Self { stored_id, start_set, second_set, value, sets })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::effect_definition::{SoundEffect, EFFECT_TYPE_SOUND};

    #[derive(Default)]
    struct Writer(Vec<u8>);

    impl Writer {
        fn u8(&mut self, value: u8) -> &mut Self { self.0.push(value); self }
        fn u32(&mut self, value: u32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn i32(&mut self, value: i32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn f32(&mut self, value: f32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
    }

    const STORED_ID: u32 = 7897;
    const SOUND_ID: u32 = 42;

    /// Set 0 (two frames, the second with a sound, next 1) and set 1 (one frame, no next).
    fn sample(start_set: u32, value: f32) -> Vec<u8> {
        let mut writer = Writer::default();
        writer.u32(STORED_ID).u32(start_set).u32(1).f32(value).u32(2);
        writer.u32(0).i32(1).f32(0.25).u32(2);
        writer.u32(3634).u8(0);
        writer.u32(3635).u8(1).u32(EFFECT_TYPE_SOUND).u32(SOUND_ID).f32(1.0);
        writer.u32(1).i32(NO_NEXT_SET).f32(0.0625).u32(1).u32(22).u8(0);
        writer.0
    }

    #[test]
    fn parse_reads_frame_sets_and_effects() {
        let definition = AnimationDefinition::parse(&sample(0, 0.0)).unwrap();
        assert_eq!((definition.stored_id, definition.start_set, definition.second_set), (STORED_ID, 0, 1));
        assert_eq!(definition.value, 0.0);
        let first = &definition.sets[&0];
        assert_eq!((first.next, first.frame_interval), (Some(1), 0.25));
        assert_eq!(first.frames[0], DefinitionFrame { tile_id: 3634, effects: Vec::new() });
        assert_eq!(
            first.frames[1].effects,
            vec![EffectDefinition::Sound(SoundEffect { sound_id: SOUND_ID, value_08: 1.0 })]
        );
        let second = &definition.sets[&1];
        assert_eq!((second.next, second.frame_interval, second.frames.len()), (None, 0.0625, 1));
    }

    #[test]
    fn parse_rejects_what_the_client_rejects() {
        let mut data = sample(0, 0.0);
        data.push(0);
        assert!(AnimationDefinition::parse(&data).unwrap_err().to_string().contains("trailing"));
        let data = sample(0, 0.0);
        assert!(AnimationDefinition::parse(&data[..data.len() - 1]).is_err());
        assert!(AnimationDefinition::parse(&sample(5, 0.0)).is_err());
        assert!(AnimationDefinition::parse(&sample(0, -1.0)).is_err());
    }

    #[test]
    fn entry_path_uses_eight_digits() {
        assert_eq!(AnimationDefinition::entry_path(7905), "build/animationdefinition/00007905.bin");
    }
}
