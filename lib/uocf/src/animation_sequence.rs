//! # Enhanced Client `AnimationSequence.uop` parser
//!
//! One entry per mobile body type, named `build/animationsequence/%08d.bin` (see
//! [`AnimationSequence::entry_path`]). An entry holds the action table (frame count, fps,
//! alias), per-direction pixel offsets, per-frame effects and skipped frames, and the action
//! groups that map movement, packet animation types and state transitions to actions. The pixels
//! live in `AnimationFrame1..6.uop`, one entry per (stored id, action).
//!
//! Layout, field meanings, run-time semantics and confidence levels follow
//! `scripts/dev-helpers/ec-research/ghidra-animation/results/uosa_animation.md`, verified against
//! `UOSA.exe` (`UOAnimationSequence::Parse` 0x0083c0ba). Confidence: **confirmed** means read in
//! the client code and checked against the data, **probable** inferred from how the value is used,
//! **unknown** no reader found. All values are little-endian and unaligned:
//!
//! ```text
//! u32   stored_id
//! 8 x { i8 x, i8 y }                         sequence offsets, per direction
//! 8 x { i16 x, i16 y }                       sequence extra offsets, per direction
//! u32   action_count
//! action_count x {
//!     u32 action, u32 frame_count, i32 alias, f32 fps
//!     8 x { i8 x, i8 y }, 8 x { i16 x, i16 y }
//!     u32 n; n x { u32 frame, effect list }  (effect list as in EffectDefinitionCollection)
//!     u32 n; n x u32 skipped frame
//! }
//! u32   group_count
//! group_count x {
//!     i8 key, i8 parent, u32 default_action
//!     u32 n; n x { i8 key, u32 action }                   transitions
//!     u32 n; n x { u8 move, u32 action, u32 alternate }   moves
//!     u32 n; n x { i8 type, u8 sub, u32 m, m x u32 }      typed (0xE2) actions
//! }
//! ```
//!
//! The same format serves the Classic and the Enhanced Client: the client accepts nothing else.

use crate::enhanced::effect_definition::{read_effect_list, EffectDefinition};
use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{bail, ensure, eyre, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::ops::RangeInclusive;

/// Directions of a mobile, in UO order (0 north, 1 north-east, 2 east, 3 south-east, 4 south,
/// 5 south-west, 6 west, 7 north-west). Offsets are stored for all of them.
pub const DIRECTION_COUNT: usize = 8;
/// Directions that have their own frames: `FIRST_STORED_DIRECTION..DIRECTION_COUNT`.
pub const STORED_DIRECTION_COUNT: u32 = 5;
/// First direction with its own frames; lower directions are mirrors.
pub const FIRST_STORED_DIRECTION: u8 = 3;
/// A mirrored direction `d < FIRST_STORED_DIRECTION` shows direction `MIRROR_DIRECTION_SUM - d`
/// flipped horizontally (0 -> 6, 1 -> 5, 2 -> 4).
pub const MIRROR_DIRECTION_SUM: u8 = 6;
/// Global frame number of the first frame of the first plain action (`NextBaseFrame`).
pub const FIRST_BASE_FRAME: u32 = 1;
/// First local frame number of an `AnimationFrame` entry (`FrameEntry::frame_number`).
pub const FIRST_FRAME_NUMBER: u32 = 1;

/// Action i8 offset value meaning "unset"; the client uses 0 instead.
pub const UNSET_BYTE_OFFSET: i8 = -128;
/// Action i16 offset value meaning "unset"; the client uses the sequence extra offset instead.
pub const UNSET_SHORT_OFFSET: i16 = -0x8000;
/// Typed action sub type matching any sub type; also the sub type forced by
/// `FRAME_SET_RESTRICTION_NO_SUB_TYPES`.
pub const ANY_SUB_TYPE: u8 = 0xff;
/// Fps the client uses for an action the sequence does not have (`GetActionFps`).
pub const MISSING_ACTION_FPS: f32 = 16.0;

/// File value of `alias` for an action with its own frames.
const NO_ALIAS: i32 = -1;
/// File value of a group `parent` for a group without a parent.
const NO_PARENT: i8 = -1;

/// Group key: war mode.
pub const GROUP_WAR: i8 = 0;
/// Group key: peace (the default group).
pub const GROUP_PEACE: i8 = 1;
/// Group key: mounted, when the sequence has the group.
pub const GROUP_MOUNTED: i8 = 2;
/// Group key: flying (setting `flying_animations` and the mobile flying state).
pub const GROUP_FLYING: i8 = 3;
/// Group key selected by a mobile state flag (predicate not decoded); only body 666 has it.
pub const GROUP_STATE_4: i8 = 4;
/// Group key selected by a mobile state flag (predicate not decoded); only body 666 has it.
pub const GROUP_STATE_5: i8 = 5;
/// Group key: seated.
pub const GROUP_SEATED: i8 = 6;

/// Move key of the stand action.
pub const MOVE_STAND: u8 = 0;
/// Move key of the walk action.
pub const MOVE_WALK: u8 = 1;
/// Move key of the run action.
pub const MOVE_RUN: u8 = 2;

/// Setting `frameSetRestriction`: typed lookups ignore the sub type and the variant.
pub const FRAME_SET_RESTRICTION_NO_SUB_TYPES: u32 = 0;
/// Setting `frameSetRestriction`: typed lookups ignore the variant.
pub const FRAME_SET_RESTRICTION_NO_VARIANTS: u32 = 1;
/// Setting `frameSetRestriction`: typed lookups honour sub type and variant.
pub const FRAME_SET_RESTRICTION_NONE: u32 = 2;

/// Action that resolves to `FLYING_ACTION_FALLBACK` when flying animations are disabled.
pub const FLYING_ACTION: u32 = 19;
/// Action used instead of `FLYING_ACTION` when flying animations are disabled.
pub const FLYING_ACTION_FALLBACK: u32 = 0;
/// Actions that take the fps of action - `FLYING_FPS_ACTION_OFFSET` when flying animations are
/// disabled.
pub const FLYING_FPS_ACTIONS: RangeInclusive<u32> = 61..=63;
/// See `FLYING_FPS_ACTIONS`.
pub const FLYING_FPS_ACTION_OFFSET: u32 = 60;

/// 0x6E actions mapped through the typed lookup (`MapCharacterAnimationAction`).
pub const CHARACTER_ANIMATION_TYPED_ACTIONS: RangeInclusive<u32> = 200..=269;
/// Typed animation type used for `CHARACTER_ANIMATION_TYPED_ACTIONS`, in group `GROUP_PEACE`
/// with sub type `ANY_SUB_TYPE`.
pub const CHARACTER_ANIMATION_TYPE: i8 = 11;
/// 0x6E actions mapped to `CHARACTER_ANIMATION_LOW_FALLBACK` when the sequence has no groups;
/// the rest of `CHARACTER_ANIMATION_TYPED_ACTIONS` maps to `CHARACTER_ANIMATION_HIGH_FALLBACK`.
pub const CHARACTER_ANIMATION_LOW_ACTIONS: RangeInclusive<u32> = 200..=209;
/// See `CHARACTER_ANIMATION_LOW_ACTIONS`.
pub const CHARACTER_ANIMATION_LOW_FALLBACK: u32 = 16;
/// See `CHARACTER_ANIMATION_LOW_ACTIONS`.
pub const CHARACTER_ANIMATION_HIGH_FALLBACK: u32 = 17;

/// Per-direction i8 pixel offsets.
pub type ByteOffsets = [(i8, i8); DIRECTION_COUNT];
/// Per-direction i16 pixel offsets.
pub type ShortOffsets = [(i16, i16); DIRECTION_COUNT];

/// One `AnimationSequence.uop` entry (`UOAnimationSequence`).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationSequence {
    /// Body id under which the frames are stored: `AnimationFrame` entries of this sequence are
    /// keyed by it, not by the entry name (confirmed by the data: 146 -> 704, 1308 -> 29,
    /// 1309 -> 1254; probable in the code). This is the only body redirect of the EC data path.
    pub stored_id: u32,
    /// Sequence i8 offsets; all zero in the data, no reader found besides the copy (unknown).
    pub offsets: ByteOffsets,
    /// Sequence i16 offsets: default of an action i16 offset equal to `UNSET_SHORT_OFFSET`
    /// (probable); all zero in the data.
    pub extra_offsets: ShortOffsets,
    /// Actions by id (confirmed).
    pub actions: BTreeMap<u32, SequenceAction>,
    /// Action groups (`FrameSetLookup`) by key, see the `GROUP_*` constants (confirmed).
    pub groups: BTreeMap<i8, ActionGroup>,
}

/// One action record (`UOAnimationSequence` action map value).
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceAction {
    /// Frames per direction (confirmed); 0 in every alias record.
    pub frame_count: u32,
    /// Action of the same body whose frames this action borrows; `None` when the action has its
    /// own frames (confirmed). See [`AnimationSequence::resolve_action`].
    pub alias: Option<u32>,
    /// Frames per second; the frame interval is `1 / fps` (confirmed); 0.0 in every alias record.
    pub fps: f32,
    /// Per-direction offset of what rides on the body (a rider on a mount); `UNSET_BYTE_OFFSET`
    /// means 0 (probable).
    pub offsets: ByteOffsets,
    /// Extra per-direction offset added to `offsets`; `UNSET_SHORT_OFFSET` means the sequence
    /// `extra_offsets` value (probable).
    pub extra_offsets: ShortOffsets,
    /// Effects started when the frame is shown, keyed by the frame index within the direction
    /// (confirmed); sounds (footsteps) in the data.
    pub frame_effects: BTreeMap<u32, Vec<EffectDefinition>>,
    /// Frames the player passes over, by 0-based frame index within the direction (confirmed in
    /// the code, index base probable); always the single frame 0 in the data.
    pub skipped_frames: BTreeSet<u32>,
    /// Global number of the first frame (not in the file): plain actions own
    /// `STORED_DIRECTION_COUNT * frame_count` consecutive numbers from `FIRST_BASE_FRAME`, in
    /// file order (`NextBaseFrame`, confirmed). 0 for alias records, which own no frames.
    pub base_frame: u32,
}

/// Stand / walk / run actions of a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveActions {
    pub action: u32,
    /// Action of the alternate variant; equal to `action` in 2483 of 2491 data entries.
    pub alternate: u32,
}

/// One action group (`FrameSetLookup`). Every lookup searches the group, then its parent chain.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionGroup {
    /// Key of the group searched next (confirmed). Always a group parsed before this one.
    pub parent: Option<i8>,
    /// Fallback of the move and typed lookups (confirmed).
    pub default_action: u32,
    /// Action played when the mobile changes state, by transition key (probable).
    pub transitions: BTreeMap<i8, u32>,
    /// Movement actions by `MOVE_*` key (confirmed).
    pub moves: BTreeMap<u8, MoveActions>,
    /// Action choices of a 0xE2 (type, sub type); `ANY_SUB_TYPE` matches any sub type
    /// (confirmed). Lists can be empty.
    pub typed: BTreeMap<(i8, u8), Vec<u32>>,
}

/// Frames of one direction of an action (`GetDirectionFrames`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectionFrames {
    /// Global number of the first frame (`base_frame` based).
    pub first_frame: u32,
    /// 1-based local frame number of the first frame in the action's `AnimationFrame` entry
    /// (`FrameEntry::frame_number`).
    pub first_frame_number: u32,
    /// Frames of the direction (the action frame count).
    pub count: u32,
    /// The frames are those of direction `MIRROR_DIRECTION_SUM - direction`, drawn flipped
    /// horizontally.
    pub mirrored: bool,
}

impl AnimationSequence {
    /// Entry name of the sequence of a body type: `build/animationsequence/%08d.bin`.
    pub fn entry_path(body_type: u32) -> String {
        format!("build/animationsequence/{body_type:08}.bin")
    }

    /// Parses one entry, which must end exactly after the group table.
    ///
    /// Repeated action ids, frame keys and group or map keys, and a parent group not parsed
    /// before its child, are errors: the client's handling of them was not traced and no client
    /// entry has one.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut reader = Cursor::new(data);
        let stored_id = reader.read_u32::<LittleEndian>()?;
        let offsets = read_byte_offsets(&mut reader)?;
        let extra_offsets = read_short_offsets(&mut reader)?;

        let mut actions = BTreeMap::new();
        let mut next_base_frame = FIRST_BASE_FRAME;
        for _ in 0..reader.read_u32::<LittleEndian>()? {
            let action_id = reader.read_u32::<LittleEndian>()?;
            let frame_count = reader.read_u32::<LittleEndian>()?;
            let alias = match reader.read_i32::<LittleEndian>()? {
                NO_ALIAS => None,
                alias => Some(
                    u32::try_from(alias).map_err(|_| eyre!("action {action_id} has negative alias {alias}"))?,
                ),
            };
            let fps = reader.read_f32::<LittleEndian>()?;
            let offsets = read_byte_offsets(&mut reader)?;
            let extra_offsets = read_short_offsets(&mut reader)?;
            let mut frame_effects = BTreeMap::new();
            for _ in 0..reader.read_u32::<LittleEndian>()? {
                let frame = reader.read_u32::<LittleEndian>()?;
                let effects = read_effect_list(&mut reader)?;
                insert_unique(&mut frame_effects, frame, effects, "action frame effect frame")?;
            }
            let mut skipped_frames = BTreeSet::new();
            for _ in 0..reader.read_u32::<LittleEndian>()? {
                let frame = reader.read_u32::<LittleEndian>()?;
                ensure!(skipped_frames.insert(frame), "action {action_id} repeats skipped frame {frame}");
            }
            let base_frame = if alias.is_none() {
                let base_frame = next_base_frame;
                next_base_frame = STORED_DIRECTION_COUNT
                    .checked_mul(frame_count)
                    .and_then(|frames| base_frame.checked_add(frames))
                    .ok_or_else(|| eyre!("action {action_id} frame count {frame_count} overflows the frame numbers"))?;
                base_frame
            } else {
                0
            };
            let action = SequenceAction {
                frame_count,
                alias,
                fps,
                offsets,
                extra_offsets,
                frame_effects,
                skipped_frames,
                base_frame,
            };
            insert_unique(&mut actions, action_id, action, "action id")?;
        }

        let mut groups = BTreeMap::new();
        for _ in 0..reader.read_u32::<LittleEndian>()? {
            let key = reader.read_i8()?;
            let parent = match reader.read_i8()? {
                NO_PARENT => None,
                parent => {
                    // The client links `GetGroup(parent)`, which is a shared empty group when the
                    // parent is not parsed yet.
                    ensure!(groups.contains_key(&parent), "group {key} parent {parent} is not parsed before it");
                    Some(parent)
                }
            };
            let default_action = reader.read_u32::<LittleEndian>()?;
            let mut transitions = BTreeMap::new();
            for _ in 0..reader.read_u32::<LittleEndian>()? {
                let transition = reader.read_i8()?;
                let action = reader.read_u32::<LittleEndian>()?;
                insert_unique(&mut transitions, transition, action, "group transition key")?;
            }
            let mut moves = BTreeMap::new();
            for _ in 0..reader.read_u32::<LittleEndian>()? {
                let movement = reader.read_u8()?;
                let action = reader.read_u32::<LittleEndian>()?;
                let alternate = reader.read_u32::<LittleEndian>()?;
                insert_unique(&mut moves, movement, MoveActions { action, alternate }, "group move key")?;
            }
            let mut typed = BTreeMap::new();
            for _ in 0..reader.read_u32::<LittleEndian>()? {
                let anim_type = reader.read_i8()?;
                let sub_type = reader.read_u8()?;
                let mut choices = Vec::new();
                for _ in 0..reader.read_u32::<LittleEndian>()? {
                    choices.push(reader.read_u32::<LittleEndian>()?);
                }
                insert_unique(&mut typed, (anim_type, sub_type), choices, "group typed key")?;
            }
            let group = ActionGroup { parent, default_action, transitions, moves, typed };
            insert_unique(&mut groups, key, group, "group key")?;
        }

        let trailing = data.len() - reader.position() as usize;
        ensure!(trailing == 0, "{trailing} trailing bytes after the animation sequence");
        Ok(Self { stored_id, offsets, extra_offsets, actions, groups })
    }

    /// Action whose frames, frame count and fps are used for `action`
    /// (`UOAnimationSequence::ResolveAction`, confirmed):
    ///
    /// - a plain action resolves to itself;
    /// - an alias resolves to its target when the target exists and is plain (one level only);
    /// - `FLYING_ACTION` is treated as an alias of `FLYING_ACTION_FALLBACK` when
    ///   `flying_enabled` is false;
    /// - a missing action, a missing target or an alias of an alias resolves to the lowest plain
    ///   action id (the client logs it).
    ///
    /// `None` when the sequence has no plain action at all.
    pub fn resolve_action(&self, action: u32, flying_enabled: bool) -> Option<u32> {
        let resolved = self.actions.get(&action).and_then(|record| {
            let alias = if action == FLYING_ACTION && !flying_enabled {
                Some(FLYING_ACTION_FALLBACK)
            } else {
                record.alias
            };
            match alias {
                None => Some(action),
                Some(target) => self
                    .actions
                    .get(&target)
                    .filter(|target_record| target_record.alias.is_none())
                    .map(|_| target),
            }
        });
        resolved.or_else(|| self.lowest_plain_action())
    }

    /// Frames per second of `action` (`GetActionFps`, confirmed): the fps of the resolved action,
    /// or `MISSING_ACTION_FPS` when there is none. With `flying_enabled` false, a resolved action
    /// in `FLYING_FPS_ACTIONS` takes the fps of action - `FLYING_FPS_ACTION_OFFSET` when that
    /// record exists.
    pub fn action_fps(&self, action: u32, flying_enabled: bool) -> f32 {
        let Some((resolved, record)) = self
            .resolve_action(action, flying_enabled)
            .and_then(|resolved| self.actions.get(&resolved).map(|record| (resolved, record)))
        else {
            return MISSING_ACTION_FPS;
        };
        if !flying_enabled && FLYING_FPS_ACTIONS.contains(&resolved) {
            if let Some(ground) = self.actions.get(&(resolved - FLYING_FPS_ACTION_OFFSET)) {
                return ground.fps;
            }
        }
        record.fps
    }

    /// Frames of `direction` of `action`, which must be an already resolved (plain) action
    /// (`GetDirectionFrames`, confirmed). Directions below `FIRST_STORED_DIRECTION` are mirrors.
    ///
    /// The client calls it with `resolve_action(action)` and shows a single frame of a stand
    /// action ([`Self::is_stand_action`]) when the `idleAnimation` setting is off.
    ///
    /// `None` when the action is missing or an alias, or the direction is not below
    /// `DIRECTION_COUNT`.
    pub fn direction_frames(&self, action: u32, direction: u8) -> Option<DirectionFrames> {
        let record = self.actions.get(&action).filter(|record| record.alias.is_none())?;
        if usize::from(direction) >= DIRECTION_COUNT {
            return None;
        }
        let mirrored = direction < FIRST_STORED_DIRECTION;
        let stored_direction = if mirrored { MIRROR_DIRECTION_SUM - direction } else { direction };
        let frame_offset = u32::from(stored_direction - FIRST_STORED_DIRECTION) * record.frame_count;
        Some(DirectionFrames {
            first_frame: record.base_frame + frame_offset,
            first_frame_number: FIRST_FRAME_NUMBER + frame_offset,
            count: record.frame_count,
            mirrored,
        })
    }

    /// Whether `action` is the stand action (or its alternate) of any group
    /// (`FrameSetLookup::IsStandAction`, confirmed).
    pub fn is_stand_action(&self, action: u32) -> bool {
        self.groups.values().any(|group| {
            group
                .moves
                .get(&MOVE_STAND)
                .is_some_and(|stand| stand.action == action || stand.alternate == action)
        })
    }

    /// Movement action of `group` (`FrameSetLookup::GetMoveAction`, confirmed): the `movement`
    /// entry (`MOVE_*`) of the first group of the parent chain that has one, its alternate when
    /// `alternate`; else the `default_action` of the last group of the chain.
    ///
    /// `None` when the group does not exist.
    pub fn move_action(&self, group: i8, movement: u8, alternate: bool) -> Option<u32> {
        let mut default_action = None;
        for lookup in self.group_chain(group) {
            if let Some(moves) = lookup.moves.get(&movement) {
                return Some(if alternate { moves.alternate } else { moves.action });
            }
            default_action = Some(lookup.default_action);
        }
        default_action
    }

    /// Action of a 0xE2 animation (`FrameSetLookup::GetTypedAction`, confirmed). Along the
    /// parent chain of `group`, the first non-empty list at (`anim_type`, `sub_type`), or at
    /// (`anim_type`, `ANY_SUB_TYPE`) when that key is absent, gives `list[variant % len]`; else
    /// the `default_action` of the last group of the chain. `restriction` is the
    /// `frameSetRestriction` setting: below `FRAME_SET_RESTRICTION_NONE` the variant is 0, at
    /// `FRAME_SET_RESTRICTION_NO_SUB_TYPES` the sub type is `ANY_SUB_TYPE`.
    ///
    /// The client computes the index with the variant as a signed byte and the list length as a
    /// byte. `None` when the group does not exist or when that index falls outside the list
    /// (a variant of 0x80 or more, or a list of a multiple of 256 entries), where the client reads
    /// past the list.
    pub fn typed_action(&self, group: i8, anim_type: i8, sub_type: u8, variant: u8, restriction: u32) -> Option<u32> {
        let variant = if restriction < FRAME_SET_RESTRICTION_NONE { 0 } else { variant };
        let sub_type = if restriction == FRAME_SET_RESTRICTION_NO_SUB_TYPES { ANY_SUB_TYPE } else { sub_type };
        let mut default_action = None;
        for lookup in self.group_chain(group) {
            let choices = lookup
                .typed
                .get(&(anim_type, sub_type))
                .or_else(|| lookup.typed.get(&(anim_type, ANY_SUB_TYPE)));
            if let Some(choices) = choices.filter(|choices| !choices.is_empty()) {
                // Client: `movsx` variant, `movzx` length byte, `idiv`, `movzx` remainder byte.
                let length = i32::from(choices.len() as u8);
                if length == 0 {
                    return None;
                }
                let index = (i32::from(variant as i8) % length) as u8;
                return choices.get(usize::from(index)).copied();
            }
            default_action = Some(lookup.default_action);
        }
        default_action
    }

    /// Action played on a state change (`FrameSetLookup::GetTransitionAction`, probable): the
    /// `transition` entry of the first group of the parent chain of `group` that has one.
    /// The client passes (state flag == 1 ? 0 : 1) + (flying ? 4 : 0).
    pub fn transition_action(&self, group: i8, transition: i8) -> Option<u32> {
        self.group_chain(group)
            .find_map(|lookup| lookup.transitions.get(&transition).copied())
    }

    /// Action played for a 0x6E packet action (`MapCharacterAnimationAction`, confirmed):
    /// `CHARACTER_ANIMATION_TYPED_ACTIONS` become [`Self::typed_action`] of `GROUP_PEACE`,
    /// `CHARACTER_ANIMATION_TYPE`, `ANY_SUB_TYPE` with the packet action as variant (its low byte,
    /// as the client) when the sequence has groups; without groups they become
    /// `CHARACTER_ANIMATION_LOW_FALLBACK` / `CHARACTER_ANIMATION_HIGH_FALLBACK`. Other actions
    /// pass through. The result is then resolved through [`Self::resolve_action`].
    pub fn map_character_animation(&self, action: u32, restriction: u32) -> Option<u32> {
        if !CHARACTER_ANIMATION_TYPED_ACTIONS.contains(&action) {
            return Some(action);
        }
        if self.groups.is_empty() {
            return Some(if CHARACTER_ANIMATION_LOW_ACTIONS.contains(&action) {
                CHARACTER_ANIMATION_LOW_FALLBACK
            } else {
                CHARACTER_ANIMATION_HIGH_FALLBACK
            });
        }
        self.typed_action(GROUP_PEACE, CHARACTER_ANIMATION_TYPE, ANY_SUB_TYPE, action as u8, restriction)
    }

    fn lowest_plain_action(&self) -> Option<u32> {
        self.actions
            .iter()
            .find(|(_, record)| record.alias.is_none())
            .map(|(&action, _)| action)
    }

    /// `group` and its parents; bounded by the group count so a hand-built cycle ends.
    fn group_chain(&self, group: i8) -> impl Iterator<Item = &ActionGroup> {
        let mut next = self.groups.get(&group);
        std::iter::from_fn(move || {
            let current = next?;
            next = current.parent.and_then(|parent| self.groups.get(&parent));
            Some(current)
        })
        .take(self.groups.len())
    }
}

fn read_byte_offsets(reader: &mut Cursor<&[u8]>) -> Result<ByteOffsets> {
    let mut offsets = [(0, 0); DIRECTION_COUNT];
    for offset in &mut offsets {
        *offset = (reader.read_i8()?, reader.read_i8()?);
    }
    Ok(offsets)
}

fn read_short_offsets(reader: &mut Cursor<&[u8]>) -> Result<ShortOffsets> {
    let mut offsets = [(0, 0); DIRECTION_COUNT];
    for offset in &mut offsets {
        *offset = (reader.read_i16::<LittleEndian>()?, reader.read_i16::<LittleEndian>()?);
    }
    Ok(offsets)
}

fn insert_unique<K: Ord + Copy + std::fmt::Debug, V>(map: &mut BTreeMap<K, V>, key: K, value: V, what: &str) -> Result<()> {
    if map.insert(key, value).is_some() {
        bail!("repeated {what} {key:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::effect_definition::{SoundEffect, EFFECT_TYPE_SOUND, EFFECT_TYPE_USER_PREFERENCE};

    /// Little-endian writer for synthetic sequence bytes.
    #[derive(Default)]
    struct Writer(Vec<u8>);

    impl Writer {
        fn u8(&mut self, value: u8) -> &mut Self { self.0.push(value); self }
        fn i8(&mut self, value: i8) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn i16(&mut self, value: i16) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn u32(&mut self, value: u32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn i32(&mut self, value: i32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn f32(&mut self, value: f32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }

        /// 8 i8 pairs, then 8 i16 pairs; direction `d` gets (`d`, -`d`) scaled by `scale`.
        fn offsets(&mut self, scale: i8) -> &mut Self {
            for direction in 0..DIRECTION_COUNT as i8 {
                self.i8(direction * scale).i8(-direction * scale);
            }
            for direction in 0..DIRECTION_COUNT as i16 {
                self.i16(direction * i16::from(scale)).i16(UNSET_SHORT_OFFSET);
            }
            self
        }

        /// Action record header and offsets; the caller writes frame effects and skipped frames.
        fn action(&mut self, action: u32, frame_count: u32, alias: i32, fps: f32) -> &mut Self {
            self.u32(action).u32(frame_count).i32(alias).f32(fps).offsets(1)
        }

        /// Action record without frame effects or skipped frames.
        fn plain(&mut self, action: u32, frame_count: u32, alias: i32, fps: f32) -> &mut Self {
            self.action(action, frame_count, alias, fps).u32(0).u32(0)
        }

        /// Group record with no transitions, moves or typed entries.
        fn empty_group(&mut self, key: i8, parent: i8, default_action: u32) -> &mut Self {
            self.i8(key).i8(parent).u32(default_action).u32(0).u32(0).u32(0)
        }
    }

    const STORED_ID: u32 = 704;
    const FOOTSTEP_SOUND: u32 = 299;
    const USER_PREFERENCE_MASK: u32 = 1;

    /// Sequence with plain actions 0 (10 frames), 4 (8 frames, frame effect, skipped frame),
    /// 19 and 61 (6 frames each); alias 1 -> 4, alias 2 -> 1 (alias of alias), alias 3 -> 9
    /// (missing target); groups 1 (peace) and 0 (war, parent 1).
    fn sample() -> Vec<u8> {
        let mut writer = Writer::default();
        writer.u32(STORED_ID).offsets(0).u32(7);
        writer.plain(0, 10, NO_ALIAS, 9.0);
        writer.action(4, 8, NO_ALIAS, 12.0);
        // One effect frame: frame 1, a UserPreference wrapping a Sound.
        writer.u32(1).u32(1).u32(1)
            .u32(EFFECT_TYPE_USER_PREFERENCE).u32(USER_PREFERENCE_MASK)
            .u32(1).u32(EFFECT_TYPE_SOUND).u32(FOOTSTEP_SOUND).f32(1.0);
        writer.u32(1).u32(0);
        writer.plain(1, 0, 4, 0.0);
        writer.plain(2, 0, 1, 0.0);
        writer.plain(3, 0, 9, 0.0);
        writer.plain(19, 6, NO_ALIAS, 9.0);
        writer.plain(61, 6, NO_ALIAS, 31.0);
        writer.u32(2);
        // Group 1 (peace): transition 0 -> 28, moves stand/walk/run, typed (11, any) and (0, 2).
        writer.i8(GROUP_PEACE).i8(NO_PARENT).u32(0);
        writer.u32(1).i8(0).u32(28);
        writer.u32(3).u8(MOVE_STAND).u32(4).u32(1).u8(MOVE_WALK).u32(0).u32(0).u8(MOVE_RUN).u32(19).u32(19);
        writer.u32(3)
            .i8(CHARACTER_ANIMATION_TYPE).u8(ANY_SUB_TYPE).u32(3).u32(30).u32(31).u32(32)
            .i8(0).u8(2).u32(2).u32(40).u32(41)
            .i8(5).u8(ANY_SUB_TYPE).u32(0);
        // Group 0 (war), parent 1: run override, typed (0, any).
        writer.i8(GROUP_WAR).i8(GROUP_PEACE).u32(7);
        writer.u32(1).i8(3).u32(20);
        writer.u32(1).u8(MOVE_RUN).u32(50).u32(51);
        writer.u32(1).i8(0).u8(ANY_SUB_TYPE).u32(1).u32(60);
        writer.0
    }

    fn parsed() -> AnimationSequence {
        AnimationSequence::parse(&sample()).expect("sample parses")
    }

    #[test]
    fn parse_reads_header_actions_and_base_frames() {
        let sequence = parsed();
        assert_eq!(sequence.stored_id, STORED_ID);
        assert_eq!(sequence.offsets, [(0, 0); DIRECTION_COUNT]);
        assert_eq!(sequence.actions.len(), 7);

        let first = &sequence.actions[&0];
        assert_eq!((first.frame_count, first.alias, first.fps, first.base_frame), (10, None, 9.0, FIRST_BASE_FRAME));
        assert_eq!(first.offsets[7], (7, -7));
        assert_eq!(first.extra_offsets[7], (7, UNSET_SHORT_OFFSET));

        // Base frames advance by 5 * frame count over plain actions in file order.
        let fourth = &sequence.actions[&4];
        assert_eq!(fourth.base_frame, FIRST_BASE_FRAME + STORED_DIRECTION_COUNT * 10);
        assert_eq!(sequence.actions[&19].base_frame, fourth.base_frame + STORED_DIRECTION_COUNT * 8);

        let alias = &sequence.actions[&1];
        assert_eq!((alias.alias, alias.frame_count, alias.base_frame), (Some(4), 0, 0));
    }

    #[test]
    fn parse_reads_frame_effects_and_skipped_frames() {
        let action = &parsed().actions[&4];
        assert_eq!(action.skipped_frames, BTreeSet::from([0]));
        let effects = &action.frame_effects[&1];
        assert_eq!(effects.len(), 1);
        let EffectDefinition::UserPreference(preference) = &effects[0] else {
            panic!("expected a user preference effect, got {:?}", effects[0]);
        };
        assert_eq!(preference.mask, USER_PREFERENCE_MASK);
        assert_eq!(
            preference.children,
            vec![EffectDefinition::Sound(SoundEffect { sound_id: FOOTSTEP_SOUND, value_08: 1.0 })]
        );
    }

    #[test]
    fn parse_reads_groups() {
        let sequence = parsed();
        let peace = &sequence.groups[&GROUP_PEACE];
        assert_eq!(peace.parent, None);
        assert_eq!(peace.transitions, BTreeMap::from([(0, 28)]));
        assert_eq!(peace.moves[&MOVE_STAND], MoveActions { action: 4, alternate: 1 });
        assert_eq!(peace.typed[&(CHARACTER_ANIMATION_TYPE, ANY_SUB_TYPE)], vec![30, 31, 32]);
        assert_eq!(peace.typed[&(5, ANY_SUB_TYPE)], Vec::<u32>::new());
        let war = &sequence.groups[&GROUP_WAR];
        assert_eq!((war.parent, war.default_action), (Some(GROUP_PEACE), 7));
    }

    #[test]
    fn parse_rejects_trailing_bytes_truncation_and_late_parents() {
        let mut data = sample();
        data.push(0);
        assert!(AnimationSequence::parse(&data).unwrap_err().to_string().contains("trailing"));
        let data = sample();
        assert!(AnimationSequence::parse(&data[..data.len() - 1]).is_err());

        let mut writer = Writer::default();
        writer.u32(1).offsets(0).u32(0).u32(1).empty_group(GROUP_WAR, GROUP_PEACE, 0);
        assert!(AnimationSequence::parse(&writer.0).is_err());
    }

    #[test]
    fn resolve_action_follows_one_alias_level_and_falls_back() {
        let sequence = parsed();
        assert_eq!(sequence.resolve_action(0, true), Some(0));
        assert_eq!(sequence.resolve_action(1, true), Some(4));
        // Alias of an alias, missing target, missing action: lowest plain action.
        assert_eq!(sequence.resolve_action(2, true), Some(0));
        assert_eq!(sequence.resolve_action(3, true), Some(0));
        assert_eq!(sequence.resolve_action(99, true), Some(0));
        // The flying action is an alias of 0 when flying animations are disabled.
        assert_eq!(sequence.resolve_action(FLYING_ACTION, true), Some(FLYING_ACTION));
        assert_eq!(sequence.resolve_action(FLYING_ACTION, false), Some(FLYING_ACTION_FALLBACK));

        let mut empty = sequence.clone();
        empty.actions.retain(|_, record| record.alias.is_some());
        assert_eq!(empty.resolve_action(0, true), None);
    }

    #[test]
    fn action_fps_uses_resolved_action_and_flying_rule() {
        let sequence = parsed();
        assert_eq!(sequence.action_fps(1, true), 12.0);
        assert_eq!(sequence.action_fps(61, true), 31.0);
        // Action 61 takes the fps of action 1, whose record is an alias (fps 0.0 in the file).
        assert_eq!(sequence.action_fps(61, false), 0.0);
        let mut empty = sequence.clone();
        empty.actions.clear();
        assert_eq!(empty.action_fps(0, true), MISSING_ACTION_FPS);
    }

    #[test]
    fn direction_frames_mirror_low_directions() {
        let sequence = parsed();
        let base = sequence.actions[&4].base_frame;
        let south_east = sequence.direction_frames(4, FIRST_STORED_DIRECTION).unwrap();
        assert_eq!(
            south_east,
            DirectionFrames { first_frame: base, first_frame_number: FIRST_FRAME_NUMBER, count: 8, mirrored: false }
        );
        let west = sequence.direction_frames(4, 6).unwrap();
        assert_eq!((west.first_frame, west.first_frame_number, west.mirrored), (base + 24, 25, false));
        // North (0) shows west (6) mirrored.
        let north = sequence.direction_frames(4, 0).unwrap();
        assert_eq!((north.first_frame, north.first_frame_number, north.mirrored), (base + 24, 25, true));
        assert!(sequence.direction_frames(4, DIRECTION_COUNT as u8).is_none());
        assert!(sequence.direction_frames(1, 4).is_none());
        assert!(sequence.is_stand_action(4) && sequence.is_stand_action(1) && !sequence.is_stand_action(0));
    }

    #[test]
    fn group_lookups_search_the_parent_chain() {
        let sequence = parsed();
        assert_eq!(sequence.move_action(GROUP_WAR, MOVE_RUN, false), Some(50));
        assert_eq!(sequence.move_action(GROUP_WAR, MOVE_RUN, true), Some(51));
        assert_eq!(sequence.move_action(GROUP_WAR, MOVE_STAND, true), Some(1));
        // No group has the key: default of the last group of the chain.
        assert_eq!(sequence.move_action(GROUP_WAR, 9, false), Some(0));
        assert_eq!(sequence.move_action(GROUP_FLYING, MOVE_RUN, false), None);
        assert_eq!(sequence.transition_action(GROUP_WAR, 3), Some(20));
        assert_eq!(sequence.transition_action(GROUP_WAR, 0), Some(28));
        assert_eq!(sequence.transition_action(GROUP_WAR, 1), None);
    }

    #[test]
    fn typed_action_applies_sub_type_variant_and_restriction() {
        let sequence = parsed();
        let full = FRAME_SET_RESTRICTION_NONE;
        // (0, 2) is in group 1 only; group 0 has (0, any), found first.
        assert_eq!(sequence.typed_action(GROUP_WAR, 0, 2, 1, full), Some(60));
        assert_eq!(sequence.typed_action(GROUP_PEACE, 0, 2, 1, full), Some(41));
        assert_eq!(sequence.typed_action(GROUP_PEACE, 0, 2, 3, full), Some(41));
        assert_eq!(sequence.typed_action(GROUP_PEACE, 0, 2, 3, FRAME_SET_RESTRICTION_NO_VARIANTS), Some(40));
        // Sub type ignored: (0, 2) is not looked up and group 1 has no (0, any).
        assert_eq!(sequence.typed_action(GROUP_PEACE, 0, 2, 3, FRAME_SET_RESTRICTION_NO_SUB_TYPES), Some(0));
        // Empty list: default action.
        assert_eq!(sequence.typed_action(GROUP_WAR, 5, 0, 0, full), Some(0));
        // Variant 0x80 and above is negative for the client: index outside the list.
        assert_eq!(sequence.typed_action(GROUP_PEACE, CHARACTER_ANIMATION_TYPE, 0, 0x80, full), None);
        assert_eq!(sequence.typed_action(GROUP_SEATED, 0, 0, 0, full), None);
    }

    #[test]
    fn map_character_animation_uses_type_11_or_fallbacks() {
        let sequence = parsed();
        assert_eq!(sequence.map_character_animation(12, FRAME_SET_RESTRICTION_NONE), Some(12));
        // Low byte of 256 + 4 is variant 4: 4 % 3 = 1.
        assert_eq!(sequence.map_character_animation(260, FRAME_SET_RESTRICTION_NONE), Some(31));
        assert_eq!(sequence.map_character_animation(260, FRAME_SET_RESTRICTION_NO_VARIANTS), Some(30));
        let mut no_groups = sequence.clone();
        no_groups.groups.clear();
        assert_eq!(no_groups.map_character_animation(205, FRAME_SET_RESTRICTION_NONE), Some(CHARACTER_ANIMATION_LOW_FALLBACK));
        assert_eq!(no_groups.map_character_animation(215, FRAME_SET_RESTRICTION_NONE), Some(CHARACTER_ANIMATION_HIGH_FALLBACK));
    }

    #[test]
    fn entry_path_uses_eight_digits() {
        assert_eq!(AnimationSequence::entry_path(400), "build/animationsequence/00000400.bin");
    }
}
