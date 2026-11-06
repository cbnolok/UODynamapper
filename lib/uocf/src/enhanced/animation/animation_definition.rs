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
//!
//! Besides decoding, the module serves an editor:
//!
//! - [`AnimationDefinition::to_bytes`] is the exact inverse of the parser (frame sets in ascending key order, as in
//!   every client entry); it refuses what the client rejects or what the layout cannot hold
//!   ([`AnimationDefinition::validate`] lists the issues, with the effect checks);
//! - [`AnimationDefinitionCollection`] reads every entry of a package, edits entries and writes a copy of the
//!   package with only the changed entries replaced (`collection.rs`);
//! - a strict KDL text form, one `animation` node per entry (`kdl_text.rs`), embeds the effect definitions in the
//!   `EffectDefinitionCollection` KDL grammar.
//!
//! ## Kingdom Reborn (`ClientDialect::KingdomReborn`)
//!
//! The KR `AnimationDefinition.uop` (`UOKR.exe`, 2008) has the same entry layout as far as its data shows: every KR
//! entry parses and re-encodes byte for byte with it. The KR reader was not traced. No KR entry has frame effects; a
//! frame's effects are read and written in the KR effect grammar (`effect_definition`, shifted type keys), assumed
//! like in KR `AnimationSequence.uop`.

mod collection;
mod kdl_text;
mod validation;

pub use collection::{
    animation_definition_kdl_file_name, AnimationDefinitionChanges, AnimationDefinitionCollection,
    AnimationDefinitionImport, ANIMATION_DEFINITION_ID_RANGES, ANIMATION_DEFINITION_KDL_EXTENSION,
    ANIMATION_DEFINITION_UOP_NAME, MOBILE_ANIMATION_DEFINITION_BASE,
};
pub use kdl_text::{animation_definitions_from_kdl, animation_definitions_to_kdl, load_animation_definitions_kdl};
pub use validation::{AnimationDefinitionIssue, AnimationDefinitionIssueKind, AnimationIssueSeverity};

use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::effect_definition::{read_effect_definitions_dialect, write_effect_definition_dialect, EffectDefinition};
use crate::uop_container::hash::hash_file_name_single;
use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{bail, ensure, eyre, Result};
use std::collections::BTreeMap;
use std::io::Cursor;

/// File value of a frame set `next` for a set without a successor.
const NO_NEXT_SET: i32 = -1;
/// Largest effect count of a frame, whose count is a u8.
pub const FRAME_EFFECT_COUNT_MAX: usize = u8::MAX as usize;

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

    /// The name hash of [`Self::entry_path`].
    pub fn entry_hash(item_id: u32) -> u64 {
        hash_file_name_single(&Self::entry_path(item_id))
    }

    /// Parses one entry, which must end exactly after its frame sets. Like the client, rejects a
    /// negative `value` and start or second set keys that name no set. Repeated set keys are
    /// errors: the client's handling was not traced and no client entry has one.
    pub fn parse(data: &[u8]) -> Result<Self> {
        Self::parse_dialect(data, ClientDialect::Enhanced)
    }

    /// [`Self::parse`] with frame effects in the effect grammar of `dialect` (the entry layout is the same).
    pub fn parse_dialect(data: &[u8], dialect: ClientDialect) -> Result<Self> {
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
                let effects = read_effect_definitions_dialect(&mut reader, effect_count, dialect)?;
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

    /// Writes the entry in the Enhanced Client effect grammar ([`Self::to_bytes_dialect`]).
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.to_bytes_dialect(ClientDialect::Enhanced)
    }

    /// Writes the entry, the inverse of [`Self::parse_dialect`]: frame sets in ascending key order (every client
    /// entry is). Fails on the first error [`Self::validate`] finds in the entry's own fields (a missing start,
    /// second or next set, a NaN, infinite or negative frame interval, a NaN or negative `value`, more effects than
    /// a frame holds) and on effects `dialect` cannot hold; the effects themselves are not checked here.
    pub fn to_bytes_dialect(&self, dialect: ClientDialect) -> Result<Vec<u8>> {
        if let Some(issue) = self.field_issues().into_iter().find(|issue| issue.severity() == AnimationIssueSeverity::Error) {
            bail!("{issue}");
        }
        let mut out = Vec::new();
        for value in [self.stored_id, self.start_set, self.second_set] {
            put_u32(&mut out, value);
        }
        out.extend_from_slice(&self.value.to_le_bytes());
        put_count(&mut out, self.sets.len(), "frame sets")?;
        for (&key, set) in &self.sets {
            put_u32(&mut out, key);
            let next = match set.next {
                None => NO_NEXT_SET,
                Some(next) => i32::try_from(next).map_err(|_| eyre!("frame set {key}: next {next} exceeds i32"))?,
            };
            out.extend_from_slice(&next.to_le_bytes());
            out.extend_from_slice(&set.frame_interval.to_le_bytes());
            put_count(&mut out, set.frames.len(), "frames")?;
            for frame in &set.frames {
                put_u32(&mut out, frame.tile_id);
                out.push(u8::try_from(frame.effects.len()).map_err(|_| eyre!("frame set {key}: too many effects"))?);
                for effect in &frame.effects {
                    write_effect_definition_dialect(&mut out, effect, dialect)?;
                }
            }
        }
        Ok(out)
    }

    /// Keys of the sets the item can play: the start set, the second set and every set a played set's `next`
    /// names.
    pub fn reachable_sets(&self) -> std::collections::BTreeSet<u32> {
        let mut reachable = std::collections::BTreeSet::new();
        let mut pending = vec![self.start_set, self.second_set];
        while let Some(key) = pending.pop() {
            if self.sets.contains_key(&key) && reachable.insert(key) {
                pending.extend(self.sets[&key].next);
            }
        }
        reachable
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_count(out: &mut Vec<u8>, count: usize, what: &str) -> Result<()> {
    put_u32(out, u32::try_from(count).map_err(|_| eyre!("{count} {what} exceed u32"))?);
    Ok(())
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

    #[test]
    fn to_bytes_is_the_inverse_of_parse() {
        for data in [sample(0, 0.0), sample(1, 2.5)] {
            let definition = AnimationDefinition::parse(&data).unwrap();
            assert_eq!(definition.to_bytes().unwrap(), data);
        }
        // Negative zero keeps its bits; the client accepts it (`value >= 0`).
        let data = sample(0, -0.0);
        assert_eq!(AnimationDefinition::parse(&data).unwrap().to_bytes().unwrap(), data);
    }

    #[test]
    fn kingdom_reborn_effects_use_the_kr_type_keys() {
        const KR: ClientDialect = ClientDialect::KingdomReborn;
        let mut definition = AnimationDefinition::parse(&sample(0, 0.0)).unwrap();
        let wait = EffectDefinition::Wait(crate::enhanced::effect_definition::WaitEffect {
            children: Vec::new(),
            min_delay_ms: 1,
            max_delay_ms: 2,
            aligned: false,
        });
        definition.sets.get_mut(&1).unwrap().frames[0].effects.push(wait);
        let ec = definition.to_bytes().unwrap();
        let kr = definition.to_bytes_dialect(KR).unwrap();
        assert_ne!(ec, kr, "the wait type key is one lower in KR");
        assert_eq!(AnimationDefinition::parse_dialect(&kr, KR).unwrap(), definition);
        assert_eq!(AnimationDefinition::parse(&ec).unwrap(), definition);
    }

    #[test]
    fn to_bytes_refuses_what_the_client_rejects() {
        let valid = AnimationDefinition::parse(&sample(0, 0.0)).unwrap();
        let error = |edit: fn(&mut AnimationDefinition)| {
            let mut definition = valid.clone();
            edit(&mut definition);
            definition.to_bytes().unwrap_err().to_string()
        };
        assert!(error(|definition| definition.start_set = 7).contains("start frame set 7"));
        assert!(error(|definition| definition.second_set = 7).contains("second frame set 7"));
        assert!(error(|definition| definition.value = -1.0).contains("value"));
        assert!(error(|definition| definition.value = f32::NAN).contains("value"));
        assert!(error(|definition| definition.sets.get_mut(&0).unwrap().next = Some(9)).contains("next 9"));
        let too_big = u32::try_from(i32::MAX).unwrap() + 1;
        let mut far = valid.clone();
        far.sets.insert(too_big, far.sets[&1].clone());
        far.sets.get_mut(&0).unwrap().next = Some(too_big);
        assert!(far.to_bytes().unwrap_err().to_string().contains("i32"));
        for interval in [f32::NAN, f32::INFINITY, -0.5] {
            let mut definition = valid.clone();
            definition.sets.get_mut(&0).unwrap().frame_interval = interval;
            assert!(definition.to_bytes().unwrap_err().to_string().contains("interval"), "{interval}");
        }
        let mut crowded = valid.clone();
        let sound = crowded.sets[&0].frames[1].effects[0].clone();
        crowded.sets.get_mut(&0).unwrap().frames[1].effects = vec![sound; FRAME_EFFECT_COUNT_MAX + 1];
        assert!(crowded.to_bytes().unwrap_err().to_string().contains("effects"));
        // Infinity is a valid value (the client only rejects negatives) and round trips.
        let mut infinite = valid.clone();
        infinite.value = f32::INFINITY;
        assert_eq!(AnimationDefinition::parse(&infinite.to_bytes().unwrap()).unwrap(), infinite);
    }

    #[test]
    fn reachable_sets_follow_next_links() {
        let mut definition = AnimationDefinition::parse(&sample(0, 0.0)).unwrap();
        assert_eq!(definition.reachable_sets().into_iter().collect::<Vec<_>>(), [0, 1]);
        definition.sets.get_mut(&0).unwrap().next = None;
        definition.second_set = 0;
        assert_eq!(definition.reachable_sets().into_iter().collect::<Vec<_>>(), [0]);
    }
}
