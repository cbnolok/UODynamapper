//! The server body table of `mobile_anim_cc.uddp` (`metadata/bodies.bin`): the Classic client's routing of a server
//! body to its frames, action layout and movement actions, resolved at pack time from `Body.def`, `Bodyconv.def`,
//! `mobtypes.txt` and `AnimationSequence.uop`, so the reader only looks a body up.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre;
use udd_assets::mobile_anim_cc::{
    MobileAnimCcAnimationRecord, MobileAnimCcBodyRecord, MoveActionSet, BODY_FLAG_OWN_UOP_FRAMES,
    BODY_FLAG_UOP_FRAMES, BODY_RECORD_SIZE, BODY_TABLE_MAGIC, GROUP_MOUNTED, GROUP_PEACE, MOBILE_ANIM_CC_METADATA_VERSION,
    MOB_TYPE_FLAG_CAN_FLY, MOB_TYPE_FLAG_IDLE_AT_8_FRAME, MOB_TYPE_FLAG_LOW_GROUP, MOB_TYPE_FLAG_LOW_GROUP_EXTENDED,
    MOB_TYPE_FLAG_PEOPLE_GROUP, MOB_TYPE_FLAG_USES_UOP_FRAMES, MOB_TYPE_GROUP_ANIMAL, MOB_TYPE_GROUP_EQUIPMENT,
    MOB_TYPE_GROUP_HUMAN, MOB_TYPE_GROUP_MONSTER, MOB_TYPE_GROUP_SEA_MONSTER, MOVE_ACTION_MOVEMENTS,
    MOVE_ACTION_MOVEMENT_COUNT, MOVE_ACTION_SET_SIZE, MOVE_RUN, MOVE_STAND, MOVE_WALK, MOVEMENT_ACTION_MOUNTINGS,
    NO_MOVE_ACTION, NO_MOVE_ACTION_SET, MOBILE_ANIM_CC_FLAG_UOP_SOURCE,
};

pub(crate) const BODY_RESOLVE_FLAG_BODY_DEF: u16 = 1 << 0;
pub(crate) const BODY_RESOLVE_FLAG_BODYCONV_DEF: u16 = 1 << 1;
/// The `mobtypes.txt` ids the client reads its UOP flag for; any other body, or one with no row, takes the MUL path.
const MOB_TYPE_BODY_IDS: std::ops::RangeInclusive<u16> = 1..=0x7FF;

/// Movement actions (ClassicUO `GetGroupForAnimation`, peace, nothing in the hands, not flying); the `UOP` ones are
/// the fallback of a UOP-flagged body without a sequence entry.
const CLASSIC_ACTION_WALK: u16 = 0;
const CLASSIC_ACTION_ANIMAL_RUN: u16 = 1;
const CLASSIC_ACTION_MONSTER_STAND: u16 = 1;
const CLASSIC_ACTION_ANIMAL_STAND: u16 = 2;
const CLASSIC_ACTION_PEOPLE_RUN: u16 = 2;
const CLASSIC_ACTION_EQUIPMENT_UOP_RUN: u16 = 3;
const CLASSIC_ACTION_EQUIPMENT_UOP_WALK: u16 = 1;
const CLASSIC_ACTION_PEOPLE_STAND: u16 = 4;
const CLASSIC_ACTION_MONSTER_IDLE: u16 = 8;
const CLASSIC_ACTION_FLY: u16 = 19;
const CLASSIC_ACTION_UOP_WALK: u16 = 22;
const CLASSIC_ACTION_MOUNTED_WALK: u16 = 23;
const CLASSIC_ACTION_UOP_RUN: u16 = 24;
const CLASSIC_ACTION_MOUNTED_RUN: u16 = 24;
const CLASSIC_ACTION_UOP_STAND: u16 = 25;
const CLASSIC_ACTION_MOUNTED_STAND: u16 = 25;
const CLASSIC_ACTION_EQUIPMENT_UOP_STAND: u16 = 37;

/// Body ids below this one take the low (monster) layout when `mobtypes.txt` has no row (ClassicUO
/// `CalculateTypeByGraphic`); ids from it to `PEOPLE_LAYOUT_FIRST_BODY` the animal layout, the rest the people
/// layout. `anim3.mul` moves the animal/monster split to `ANIM3_MONSTER_FIRST_BODY`. The original client has no id
/// ranges: a body without a row is `MOB_TYPE_GROUP_EQUIPMENT` whatever its id (`MobTypes_GroupType` 0x005976b0).
const ANIMAL_LAYOUT_FIRST_BODY: u16 = 200;
const ANIM3_MONSTER_FIRST_BODY: u16 = 300;
const PEOPLE_LAYOUT_FIRST_BODY: u16 = 400;
const ANIM2_FILE_INDEX: u8 = 1;
const ANIM3_FILE_INDEX: u8 = 2;
/// The stored direction "has it" tests an action on.
const HAS_ACTION_DIRECTION: u8 = 0;
/// Body table header: magic, version, body count, move action set count.
const BODY_TABLE_HEADER_SIZE: usize = 16;

/// A `Body.def` and `Bodyconv.def` row of a body, merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClassicBodyResolveRow {
    pub body_id: u16,
    pub resolved_body_id: u16,
    pub hue: u16,
    pub file_index: u8,
    pub mount_height: i8,
    pub flags: u16,
}

/// A `mobtypes.txt` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClassicMobTypeRow {
    pub body_id: u16,
    pub group_type: u8,
    pub flags: u32,
    /// Movement actions of the body's `AnimationSequence.uop` entry; every entry `NO_MOVE_ACTION` for a body without
    /// `MOB_TYPE_FLAG_USES_UOP_FRAMES` or without a sequence entry.
    pub move_actions: MoveActionSet,
}

impl ClassicMobTypeRow {
    /// Action of the sequence for `movement` (`MOVE_STAND`, `MOVE_WALK`, `MOVE_RUN`) in group `group`
    /// (`GROUP_WAR`, `GROUP_PEACE`, `GROUP_MOUNTED`); `None` when it has no such group or the arguments are outside
    /// those sets.
    pub fn move_action(&self, group: i8, movement: u8) -> Option<u16> {
        udd_assets::animation::move_actions::move_action(&self.move_actions, group, movement)
    }
}

/// Where a server body's frames are: the body id within its source and the `anim*.mul` file, or UOP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClassicFramesBody {
    /// Body id of the animation records.
    pub body_id: u16,
    /// `anim.mul` 0, `anim2.mul` 1, ...; meaningless with `uop`.
    pub file_index: u8,
    /// The client draws the body from `AnimationFrame*.uop`.
    pub uop: bool,
    /// The body whose `mobtypes.txt` row gives the action layout and flags: the server body, or its
    /// `Body.def` graphic.
    pub type_body_id: u16,
}

/// The body rows the Classic frame resolution reads, each sorted by body id: the `Body.def` and `Bodyconv.def`
/// resolves and the `mobtypes.txt` rows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClassicBodyTables<'a> {
    pub body_resolve: &'a [ClassicBodyResolveRow],
    pub body_types: &'a [ClassicMobTypeRow],
}

impl<'a> ClassicBodyTables<'a> {
    pub fn body_resolve_record(&self, body_id: u16) -> Option<&'a ClassicBodyResolveRow> {
        self.body_resolve
            .binary_search_by_key(&body_id, |record| record.body_id)
            .ok()
            .map(|index| &self.body_resolve[index])
    }

    pub fn body_type_record(&self, body_id: u16) -> Option<&'a ClassicMobTypeRow> {
        self.body_types
            .binary_search_by_key(&body_id, |record| record.body_id)
            .ok()
            .map(|index| &self.body_types[index])
    }

    /// Whether the client draws this body from UOP. The client's frame code doesn't always test the requested id
    /// (`docs/client_research/ghidra-classic-frames/results/classic_frames.md`); this is the mobile's own body.
    pub fn body_uses_uop_frames(&self, body_id: u16) -> bool {
        MOB_TYPE_BODY_IDS.contains(&body_id)
            && self
                .body_type_record(body_id)
                .is_some_and(|record| record.flags & MOB_TYPE_FLAG_USES_UOP_FRAMES != 0)
    }

    /// Where the client takes the frames of server body `body_id`. A body `mobtypes.txt` flags for UOP draws its own
    /// id from UOP. Otherwise `Body.def` gives the body another id unless `Bodyconv.def` moves the body to another
    /// `anim*.mul`; the resulting id then goes through UOP or `Bodyconv.def` the same way (ClassicUO
    /// `ConvertBodyIfNeeded`, `GetIndices`; the client applies `body.def`, then the bodyconv table, on the MUL side
    /// only: `uop_reachability.md`).
    pub fn frames_body(&self, body_id: u16) -> ClassicFramesBody {
        let type_body_id = match self.body_resolve_record(body_id) {
            Some(record)
                if !self.body_uses_uop_frames(body_id)
                    && record.flags & BODY_RESOLVE_FLAG_BODYCONV_DEF == 0
                    && record.flags & BODY_RESOLVE_FLAG_BODY_DEF != 0 =>
            {
                record.resolved_body_id
            }
            _ => body_id,
        };
        let (frames_body_id, file_index, uop) = if self.body_uses_uop_frames(type_body_id) {
            (type_body_id, 0, true)
        } else {
            match self.body_resolve_record(type_body_id) {
                Some(record) if record.flags & BODY_RESOLVE_FLAG_BODYCONV_DEF != 0 => {
                    (record.resolved_body_id, record.file_index, false)
                }
                _ => (type_body_id, 0, false),
            }
        };
        ClassicFramesBody { body_id: frames_body_id, file_index, uop, type_body_id }
    }

    /// Action layout of server body `body_id` (`MOB_TYPE_GROUP_*`) and its `mobtypes.txt` flags: those of the
    /// `Body.def` graphic when it has one (`MobTypes_GroupType` 0x005976b0 on the remapped body; confirmed). Without
    /// a row, the layout follows the id and file of its frames (ClassicUO `CalculateTypeByGraphic`, by choice) and
    /// the flags are 0. The client takes `MOB_TYPE_GROUP_EQUIPMENT`, the people layout without mounted actions, for
    /// any id (`docs/client_research/ghidra-classic-frames/results/classic_frames.md`); the shipped `anim.mul`
    /// bodies with frames and no row (492, 573, 634, 640, 641, 642, 970) all get the people layout either way.
    fn body_layout(&self, frames: ClassicFramesBody) -> (u8, u32) {
        if let Some(record) = self.body_type_record(frames.type_body_id) {
            return (record.group_type, record.flags);
        }
        let group_type = match (frames.file_index, frames.body_id) {
            (ANIM2_FILE_INDEX, id) if id < ANIMAL_LAYOUT_FIRST_BODY => MOB_TYPE_GROUP_MONSTER,
            (ANIM2_FILE_INDEX, _) => MOB_TYPE_GROUP_ANIMAL,
            (ANIM3_FILE_INDEX, id) if id < ANIM3_MONSTER_FIRST_BODY => MOB_TYPE_GROUP_ANIMAL,
            (ANIM3_FILE_INDEX, id) if id < PEOPLE_LAYOUT_FIRST_BODY => MOB_TYPE_GROUP_MONSTER,
            (_, id) if id < ANIMAL_LAYOUT_FIRST_BODY => MOB_TYPE_GROUP_MONSTER,
            (_, id) if id < PEOPLE_LAYOUT_FIRST_BODY => MOB_TYPE_GROUP_ANIMAL,
            _ => MOB_TYPE_GROUP_HUMAN,
        };
        (group_type, 0)
    }
}

/// The animation records by body id.
struct AnimationsByBody<'a>(HashMap<u16, Vec<&'a MobileAnimCcAnimationRecord>>);

impl<'a> AnimationsByBody<'a> {
    fn new(animations: &'a [MobileAnimCcAnimationRecord]) -> Self {
        let mut by_body = HashMap::<u16, Vec<_>>::new();
        for animation in animations {
            by_body.entry(animation.body_id).or_default().push(animation);
        }
        Self(by_body)
    }

    /// The records the client draws a body with frames `frames` from: those of its frames body, from its `anim*.mul`
    /// file on the MUL side, in either source (a package can hold both for one action; the reader prefers the
    /// client's).
    fn of(&self, frames: ClassicFramesBody) -> impl Iterator<Item = &'a MobileAnimCcAnimationRecord> + '_ {
        self.0.get(&frames.body_id).into_iter().flatten().copied()
            .filter(move |animation| frames.uop || animation.file_index == frames.file_index)
    }
}

/// Action the Classic client plays for a body of frames `frames`, action layout `group_type`, `mobtypes.txt` flags
/// `flags` and `mobtypes.txt` row `record` (of `frames.type_body_id`), standing, walking or running (`MOVE_STAND`,
/// `MOVE_WALK`, `MOVE_RUN`) in peace mode, `mounted` when the body rides a mount (a mount's own body takes its
/// layout's actions), with no server animation, nothing in the hands and no flight.
///
/// A UOP-flagged body (`MOB_TYPE_FLAG_USES_UOP_FRAMES`) plays the move action of its sequence, as the client does
/// (`CMobile_UpdateUopMoveAction` 0x005b6e40 through `FrameSetLookup_GetMoveAction` 0x00408114, Classic
/// 7.0.102.3; confirmed): group `GROUP_MOUNTED` when `mounted` and the sequence has it, else `GROUP_PEACE`
/// (`CMobile_UopSequenceGroup` 0x005b6270). One without a sequence entry takes the fixed UOP actions below.
///
/// The others follow ClassicUO `GetGroupForAnimation` by choice:
///
/// - human / equipment: walk 0, run 2, stand 4; mounted walk 23, run 24, stand 25; UOP-flagged: run 24 when the
///   body has it, equipment walk 1 / run 3 without 0 / 2, equipment stand 37 when it has it.
/// - animal: walk 0, run 1 (2 without 1), stand 2; UOP-flagged walk 22, run 24, stand 25. With
///   `MOB_TYPE_FLAG_LOW_GROUP_EXTENDED`, the monster rules.
/// - monster: walk 0, run 0, stand 1; stand 8 with `MOB_TYPE_FLAG_IDLE_AT_8_FRAME` when the body has it, run 19
///   with `MOB_TYPE_FLAG_CAN_FLY` when it has it; UOP-flagged walk 22, run 24, stand 25.
///   `MOB_TYPE_FLAG_PEOPLE_GROUP`: 0 for all; `MOB_TYPE_FLAG_LOW_GROUP`: the animal actions 0, 1, 2.
/// - sea monster: walk 0, run 1, stand 2.
///
/// The client's MUL rules (`GetMobileMulFrame` 0x0044e8e0) are the same except that only a human
/// (`MobTypes_IsHuman` 0x005977e0) takes the mounted actions, a body without a row takes the people layout, and
/// it tests no action: a missing one falls back to the layout's stand action (`GetObjectLayerFrame` 0x005441e0),
/// so a flying monster without 19 runs with 1, not 0
/// (`docs/client_research/ghidra-classic-frames/results/classic_frames.md`).
///
/// "Has it" tests stored direction 0 of the body's records (`AnimationsByBody::of`).
fn movement_action(
    animations: &AnimationsByBody<'_>,
    frames: ClassicFramesBody,
    (group_type, flags): (u8, u32),
    record: Option<&ClassicMobTypeRow>,
    movement: u8,
    mounted: bool,
) -> u16 {
    let uop = flags & MOB_TYPE_FLAG_USES_UOP_FRAMES != 0;
    if uop {
        let sequence_action = record.and_then(|record| {
            let mounted_action = record.move_action(GROUP_MOUNTED, movement).filter(|_| mounted);
            mounted_action.or_else(|| record.move_action(GROUP_PEACE, movement))
        });
        if let Some(action) = sequence_action {
            return action;
        }
    }
    let has = |action: u16| animations.of(frames)
        .any(|animation| animation.action_id == action && animation.direction == HAS_ACTION_DIRECTION);
    let monster = |movement: u8| {
        if flags & MOB_TYPE_FLAG_PEOPLE_GROUP != 0 {
            return CLASSIC_ACTION_WALK;
        }
        if flags & MOB_TYPE_FLAG_LOW_GROUP != 0 {
            return match movement {
                MOVE_WALK => CLASSIC_ACTION_WALK,
                MOVE_RUN => CLASSIC_ACTION_ANIMAL_RUN,
                _ => CLASSIC_ACTION_ANIMAL_STAND,
            };
        }
        match movement {
            MOVE_WALK if uop => CLASSIC_ACTION_UOP_WALK,
            MOVE_WALK => CLASSIC_ACTION_WALK,
            MOVE_RUN if flags & MOB_TYPE_FLAG_CAN_FLY != 0 && has(CLASSIC_ACTION_FLY) => CLASSIC_ACTION_FLY,
            MOVE_RUN if uop => CLASSIC_ACTION_UOP_RUN,
            MOVE_RUN => CLASSIC_ACTION_WALK,
            _ if flags & MOB_TYPE_FLAG_IDLE_AT_8_FRAME != 0 && has(CLASSIC_ACTION_MONSTER_IDLE) => {
                CLASSIC_ACTION_MONSTER_IDLE
            }
            _ if uop => CLASSIC_ACTION_UOP_STAND,
            _ => CLASSIC_ACTION_MONSTER_STAND,
        }
    };
    match group_type {
        MOB_TYPE_GROUP_ANIMAL if flags & MOB_TYPE_FLAG_LOW_GROUP_EXTENDED != 0 => monster(movement),
        MOB_TYPE_GROUP_ANIMAL => match movement {
            MOVE_WALK if uop => CLASSIC_ACTION_UOP_WALK,
            MOVE_WALK => CLASSIC_ACTION_WALK,
            MOVE_RUN if uop => CLASSIC_ACTION_UOP_RUN,
            MOVE_RUN if has(CLASSIC_ACTION_ANIMAL_RUN) => CLASSIC_ACTION_ANIMAL_RUN,
            MOVE_RUN => CLASSIC_ACTION_ANIMAL_STAND,
            _ if uop => CLASSIC_ACTION_UOP_STAND,
            _ => CLASSIC_ACTION_ANIMAL_STAND,
        },
        MOB_TYPE_GROUP_MONSTER => monster(movement),
        MOB_TYPE_GROUP_SEA_MONSTER => match movement {
            MOVE_WALK => CLASSIC_ACTION_WALK,
            MOVE_RUN => CLASSIC_ACTION_ANIMAL_RUN,
            _ => CLASSIC_ACTION_ANIMAL_STAND,
        },
        _ if mounted => match movement {
            MOVE_WALK => CLASSIC_ACTION_MOUNTED_WALK,
            MOVE_RUN => CLASSIC_ACTION_MOUNTED_RUN,
            _ => CLASSIC_ACTION_MOUNTED_STAND,
        },
        group_type => {
            let uop_equipment = uop && group_type == MOB_TYPE_GROUP_EQUIPMENT;
            match movement {
                MOVE_WALK if uop_equipment && !has(CLASSIC_ACTION_WALK) => CLASSIC_ACTION_EQUIPMENT_UOP_WALK,
                MOVE_WALK => CLASSIC_ACTION_WALK,
                MOVE_RUN if uop && has(CLASSIC_ACTION_UOP_RUN) => CLASSIC_ACTION_UOP_RUN,
                MOVE_RUN if uop_equipment && !has(CLASSIC_ACTION_PEOPLE_RUN) => CLASSIC_ACTION_EQUIPMENT_UOP_RUN,
                MOVE_RUN => CLASSIC_ACTION_PEOPLE_RUN,
                _ if uop_equipment && has(CLASSIC_ACTION_EQUIPMENT_UOP_STAND) => CLASSIC_ACTION_EQUIPMENT_UOP_STAND,
                _ => CLASSIC_ACTION_PEOPLE_STAND,
            }
        }
    }
}

/// The server body table: the bodies sorted by id, and the move action sets they refer to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ClassicBodyTable {
    pub bodies: Vec<MobileAnimCcBodyRecord>,
    pub move_action_sets: Vec<MoveActionSet>,
}

/// The body table of `tables` over the package's `animations`: one record per body the server can send that reaches
/// frames, among the ids with animation records and the `Body.def`, `Bodyconv.def` and `mobtypes.txt` ones. A body
/// reaches frames when its frames body ([`ClassicBodyTables::frames_body`]) has records in its source (any file on
/// the UOP route, its `anim*.mul` file on the MUL one). Each holds where its frames are, its action layout
/// ([`ClassicBodyTables::body_layout`]), its own `Body.def` hue and `Bodyconv.def` mount height, its movement actions
/// ([`movement_action`]) and its action layout's sequence move actions, each distinct set stored once.
pub(crate) fn resolve_classic_bodies(
    tables: ClassicBodyTables<'_>,
    animations: &[MobileAnimCcAnimationRecord],
) -> eyre::Result<ClassicBodyTable> {
    let by_body = AnimationsByBody::new(animations);
    let candidates: BTreeSet<u16> = animations.iter().map(|record| record.body_id)
        .chain(tables.body_resolve.iter().map(|record| record.body_id))
        .chain(tables.body_types.iter().map(|record| record.body_id))
        .collect();
    let mut table = ClassicBodyTable::default();
    let mut set_indices = BTreeMap::<MoveActionSet, u16>::new();
    for body_id in candidates {
        let frames = tables.frames_body(body_id);
        if by_body.of(frames).next().is_none() {
            continue;
        }
        let layout = tables.body_layout(frames);
        let type_record = tables.body_type_record(frames.type_body_id);
        let movement_actions = [false, true].map(|mounted| MOVE_ACTION_MOVEMENTS.map(|movement|
            movement_action(&by_body, frames, layout, type_record, movement, mounted)));
        let sequence_actions = type_record.map(|record| record.move_actions)
            .filter(|actions| actions.iter().flatten().any(|&action| action != NO_MOVE_ACTION));
        let move_action_set = match sequence_actions {
            Some(actions) => {
                let next = u16::try_from(set_indices.len()).ok().filter(|&index| index != NO_MOVE_ACTION_SET)
                    .ok_or_else(|| eyre::eyre!("more move action sets than the body table indexes"))?;
                let index = *set_indices.entry(actions).or_insert(next);
                if index == next {
                    table.move_action_sets.push(actions);
                }
                index
            }
            None => NO_MOVE_ACTION_SET,
        };
        let own = tables.body_resolve_record(body_id);
        table.bodies.push(MobileAnimCcBodyRecord {
            body_id,
            frames_body_id: frames.body_id,
            file_index: if frames.uop { 0 } else { frames.file_index },
            flags: if frames.uop { BODY_FLAG_UOP_FRAMES } else { 0 }
                | if tables.body_uses_uop_frames(body_id) { BODY_FLAG_OWN_UOP_FRAMES } else { 0 },
            group_type: layout.0,
            mount_height: own.map_or(0, |record| record.mount_height),
            mob_type_flags: layout.1,
            hue: own.map_or(0, |record| record.hue),
            move_action_set,
            movement_actions,
        });
    }
    Ok(table)
}

/// `metadata/bodies.bin`: the header (magic, version, body count, move action set count), the body records, then the
/// move action sets.
pub(crate) fn serialize_body_table(table: &ClassicBodyTable) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(BODY_TABLE_HEADER_SIZE + table.bodies.len() * BODY_RECORD_SIZE
        + table.move_action_sets.len() * MOVE_ACTION_SET_SIZE);
    bytes.extend_from_slice(&BODY_TABLE_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_CC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(table.bodies.len() as u32)?;
    bytes.write_u32::<LittleEndian>(table.move_action_sets.len() as u32)?;
    for body in &table.bodies {
        bytes.write_u16::<LittleEndian>(body.body_id)?;
        bytes.write_u16::<LittleEndian>(body.frames_body_id)?;
        bytes.push(body.file_index);
        bytes.push(body.flags);
        bytes.push(body.group_type);
        bytes.write_i8(body.mount_height)?;
        bytes.write_u32::<LittleEndian>(body.mob_type_flags)?;
        bytes.write_u16::<LittleEndian>(body.hue)?;
        bytes.write_u16::<LittleEndian>(body.move_action_set)?;
        for &action in body.movement_actions.iter().flatten() {
            bytes.write_u16::<LittleEndian>(action)?;
        }
    }
    for &action in table.move_action_sets.iter().flatten().flatten() {
        bytes.write_u16::<LittleEndian>(action)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::mobile_anim_cc::{GROUP_WAR, MOVE_ACTION_GROUP_COUNT, NO_SEQUENCE_FPS};

    const _: () = assert!(MOVEMENT_ACTION_MOUNTINGS == 2 && MOVE_ACTION_MOVEMENT_COUNT == 3);

    fn animation(body_id: u16, action_id: u16, file_index: u8, flags: u16) -> MobileAnimCcAnimationRecord {
        MobileAnimCcAnimationRecord {
            body_id,
            action_id,
            direction: 0,
            file_index,
            source_index: u32::from(body_id),
            frame_start: 0,
            frame_count: 0,
            flags,
            fps: NO_SEQUENCE_FPS,
        }
    }

    fn resolve(body_id: u16, resolved_body_id: u16, file_index: u8, flags: u16) -> ClassicBodyResolveRow {
        ClassicBodyResolveRow { body_id, resolved_body_id, hue: 0, file_index, mount_height: 0, flags }
    }

    fn body_type(body_id: u16, group_type: u8, flags: u32) -> ClassicMobTypeRow {
        ClassicMobTypeRow {
            body_id,
            group_type,
            flags: MOB_TYPE_FLAG_FOUND | flags,
            move_actions: NO_MOVE_ACTIONS,
        }
    }

    const NO_MOVE_ACTIONS: MoveActionSet = [[NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT]; MOVE_ACTION_GROUP_COUNT];

    /// `mobtypes.txt` rows carry this flag (the packer's "found" bit).
    const MOB_TYPE_FLAG_FOUND: u32 = 0x8000_0000;

    /// The body table of `body_resolve` and `body_types` (sorted by body as the packer reads them) over `animations`.
    fn table_of(
        animations: &[MobileAnimCcAnimationRecord],
        body_resolve: &[ClassicBodyResolveRow],
        body_types: &[ClassicMobTypeRow],
    ) -> ClassicBodyTable {
        let mut body_resolve = body_resolve.to_vec();
        body_resolve.sort_by_key(|record| record.body_id);
        let mut body_types = body_types.to_vec();
        body_types.sort_by_key(|record| record.body_id);
        resolve_classic_bodies(ClassicBodyTables { body_resolve: &body_resolve, body_types: &body_types }, animations)
            .unwrap()
    }

    fn body(table: &ClassicBodyTable, body_id: u16) -> MobileAnimCcBodyRecord {
        *table.bodies.iter().find(|record| record.body_id == body_id).unwrap_or_else(|| panic!("body {body_id}"))
    }

    /// The stand, walk and run actions of `body_id`, mounted or not.
    fn actions(table: &ClassicBodyTable, body_id: u16, mounted: bool) -> [u16; MOVE_ACTION_MOVEMENT_COUNT] {
        body(table, body_id).movement_actions[usize::from(mounted)]
    }

    #[test]
    fn server_bodies_take_body_def_then_bodyconv_and_their_own_file() {
        const ANIM4: u8 = 3;
        let (bodyconv_body, body_def_body, chained_body, uop_body, frameless_body) = (250, 9, 8, 700, 10);
        let table = table_of(
            &[
                animation(bodyconv_body, 0, 0, 0),
                animation(17, 0, ANIM4, 0),
                animation(17, 0, 0, 0),
                animation(42, 0, 0, 0),
                animation(uop_body, 0, ANIM4, MOBILE_ANIM_CC_FLAG_UOP_SOURCE),
            ],
            &[
                resolve(chained_body, body_def_body, 0, BODY_RESOLVE_FLAG_BODY_DEF),
                resolve(body_def_body, 42, 0, BODY_RESOLVE_FLAG_BODY_DEF),
                resolve(bodyconv_body, 17, ANIM4, BODY_RESOLVE_FLAG_BODYCONV_DEF),
                resolve(frameless_body, 43, 0, BODY_RESOLVE_FLAG_BODY_DEF),
            ],
            &[body_type(uop_body, MOB_TYPE_GROUP_MONSTER, MOB_TYPE_FLAG_USES_UOP_FRAMES)],
        );

        let converted = body(&table, bodyconv_body);
        assert_eq!((converted.frames_body_id, converted.file_index, converted.uses_uop_frames()), (17, ANIM4, false));
        // Body.def applies once: 8 -> 9, and 9's own Body.def row is not followed; 9 has no frames of its own.
        assert!(table.bodies.iter().all(|record| record.body_id != chained_body));
        assert_eq!(body(&table, body_def_body).frames_body_id, 42);
        let uop = body(&table, uop_body);
        assert_eq!((uop.frames_body_id, uop.flags), (uop_body, BODY_FLAG_UOP_FRAMES | BODY_FLAG_OWN_UOP_FRAMES));
        // A body whose frames body has no records reaches no frames.
        assert!(table.bodies.iter().all(|record| record.body_id != frameless_body));
        // 17 as a server body: its anim.mul records.
        assert_eq!(body(&table, 17).file_index, 0);
        let ids = table.bodies.iter().map(|record| record.body_id).collect::<Vec<_>>();
        assert_eq!(ids, [body_def_body, 17, 42, bodyconv_body, uop_body]);
    }

    #[test]
    fn movement_actions_follow_the_body_layout() {
        let (human, horse, uop_animal, monster, idle_monster, people_monster, sea_monster, unlisted_animal) =
            (400, 204, 277, 1, 2, 3, 150, 220);
        let (equipment, unlisted_people, flying_monster, frameless_flyer) = (970, 492, 4, 5);
        let bodies = [human, horse, uop_animal, monster, idle_monster, people_monster, sea_monster, unlisted_animal,
            equipment, unlisted_people, flying_monster, frameless_flyer];
        let mut animations = bodies.map(|body| animation(body, 0, 0, 0)).to_vec();
        animations.extend([animation(horse, 1, 0, 0), animation(idle_monster, 8, 0, 0),
            animation(flying_monster, 19, 0, 0)]);
        let table = table_of(
            &animations,
            &[],
            &[
                body_type(human, MOB_TYPE_GROUP_HUMAN, 0),
                body_type(horse, MOB_TYPE_GROUP_ANIMAL, 0),
                body_type(uop_animal, MOB_TYPE_GROUP_ANIMAL, MOB_TYPE_FLAG_USES_UOP_FRAMES),
                body_type(monster, MOB_TYPE_GROUP_MONSTER, 0),
                body_type(idle_monster, MOB_TYPE_GROUP_MONSTER, MOB_TYPE_FLAG_IDLE_AT_8_FRAME),
                body_type(people_monster, MOB_TYPE_GROUP_MONSTER, MOB_TYPE_FLAG_PEOPLE_GROUP),
                body_type(sea_monster, MOB_TYPE_GROUP_SEA_MONSTER, 0),
                body_type(equipment, MOB_TYPE_GROUP_EQUIPMENT, 0),
                body_type(flying_monster, MOB_TYPE_GROUP_MONSTER, MOB_TYPE_FLAG_CAN_FLY),
                body_type(frameless_flyer, MOB_TYPE_GROUP_MONSTER, MOB_TYPE_FLAG_CAN_FLY),
            ],
        );

        assert_eq!(actions(&table, human, false), [4, 0, 2]);
        assert_eq!(actions(&table, human, true), [25, 23, 24]);
        assert_eq!(actions(&table, equipment, true), [25, 23, 24]);
        assert_eq!(actions(&table, horse, true), [2, 0, 1]);
        // A UOP body without a sequence entry: the fixed actions.
        assert_eq!(actions(&table, uop_animal, false), [25, 22, 24]);
        assert_eq!(actions(&table, monster, false), [1, 0, 0]);
        assert_eq!(actions(&table, idle_monster, false), [8, 0, 0]);
        assert_eq!(actions(&table, people_monster, false), [0, 0, 0]);
        assert_eq!(actions(&table, sea_monster, false), [2, 0, 1]);
        // A flying monster runs with 19 when it has it.
        assert_eq!(actions(&table, flying_monster, false), [1, 0, 19]);
        assert_eq!(actions(&table, frameless_flyer, false), [1, 0, 0]);
        // No row: the id range gives the animal layout; without action 1 it runs with 2.
        let unlisted = body(&table, unlisted_animal);
        assert_eq!((unlisted.group_type, unlisted.mob_type_flags), (MOB_TYPE_GROUP_ANIMAL, 0));
        assert_eq!(actions(&table, unlisted_animal, false), [2, 0, 2]);
        // No row from 400 on: the human layout.
        let unlisted = body(&table, unlisted_people);
        assert_eq!((unlisted.group_type, unlisted.mob_type_flags), (MOB_TYPE_GROUP_HUMAN, 0));
        assert_eq!(actions(&table, unlisted_people, true), [25, 23, 24]);
        assert!(table.move_action_sets.is_empty());
    }

    #[test]
    fn uop_bodies_take_the_move_actions_of_their_sequence() {
        let (monster, human, animal, same_as_monster) = (714, 666, 432, 715);
        let with_actions = |body_id: u16, group_type: u8, peace: [u16; MOVE_ACTION_MOVEMENT_COUNT], mounted| {
            let mut record = body_type(body_id, group_type, MOB_TYPE_FLAG_USES_UOP_FRAMES);
            record.move_actions = [[NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT], peace, mounted];
            record
        };
        let no_group = [NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT];
        let table = table_of(
            &[monster, human, animal, same_as_monster].map(|body| animation(body, 0, 0, MOBILE_ANIM_CC_FLAG_UOP_SOURCE)),
            &[],
            &[
                with_actions(monster, MOB_TYPE_GROUP_MONSTER, [1, 0, 0], no_group),
                with_actions(same_as_monster, MOB_TYPE_GROUP_MONSTER, [1, 0, 0], no_group),
                with_actions(human, MOB_TYPE_GROUP_HUMAN, [37, 1, 3], [25, 23, 24]),
                with_actions(animal, MOB_TYPE_GROUP_ANIMAL, [31, 22, 24], [31, 29, 30]),
            ],
        );

        assert_eq!(actions(&table, monster, false), [1, 0, 0]);
        // Without a mounted group the peace group plays.
        assert_eq!(actions(&table, monster, true), [1, 0, 0]);
        assert_eq!(actions(&table, human, false), [37, 1, 3]);
        assert_eq!(actions(&table, human, true), [25, 23, 24]);
        assert_eq!(actions(&table, animal, false), [31, 22, 24]);
        assert_eq!(actions(&table, animal, true), [31, 29, 30]);
        // Equal sequence move actions are stored once.
        assert_eq!(table.move_action_sets.len(), 3);
        assert_eq!(body(&table, monster).move_action_set, body(&table, same_as_monster).move_action_set);
        let set = table.move_action_sets[usize::from(body(&table, human).move_action_set)];
        assert_eq!(udd_assets::animation::move_actions::move_action(&set, GROUP_WAR, MOVE_WALK), None);
        assert_eq!(udd_assets::animation::move_actions::move_action(&set, GROUP_MOUNTED, MOVE_STAND), Some(25));
    }
}
