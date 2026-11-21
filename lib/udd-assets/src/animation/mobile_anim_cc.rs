use std::collections::{BTreeSet, HashMap};
use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};
use udd_container::UddpReader;

use crate::common::{
    decode_atlas_page_rgba, ensure_manifest_version, extract_atlas_subrect_rgba, read_path_entry, read_path_entry_cow,
    AtlasCacheOptions, AtlasPageCache,
};
use crate::animation::mobile_strides::{
    read_stride_records, MobileAnimClips, MobileClip, MobileClipFrame, MobileStrideRecord, PageMemo, StrideSource,
    STRIDE_FACINGS, STRIDE_FACING_COUNT,
};
use crate::tex_art_cc::{AtlasPackingMode, PagePixelFormat};
pub use crate::animation::move_actions::{
    GROUP_MOUNTED, GROUP_PEACE, GROUP_WAR, MOVE_ACTION_GROUPS, MOVE_ACTION_GROUP_COUNT, MOVE_ACTION_MOVEMENTS,
    MOVE_ACTION_MOVEMENT_COUNT, MOVE_RUN, MOVE_STAND, MOVE_WALK, NO_MOVE_ACTION,
};

pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const ANIMATION_MANIFEST_ENTRY_PATH: &str = "metadata/animations.bin";
pub const FRAME_MANIFEST_ENTRY_PATH: &str = "metadata/frames.bin";
pub const BODY_RESOLVE_MANIFEST_ENTRY_PATH: &str = "metadata/body_resolve.bin";
pub const BODY_TYPE_MANIFEST_ENTRY_PATH: &str = "metadata/body_types.bin";

pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_FRAME_INDEX: u16 = u16::MAX;

pub const BODY_RESOLVE_FLAG_BODY_DEF: u16 = 1 << 0;
pub const BODY_RESOLVE_FLAG_BODYCONV_DEF: u16 = 1 << 1;
/// Animation record flag: the frames came from `AnimationFrame*.uop`, not from `anim*.mul`.
pub const MOBILE_ANIM_CC_FLAG_UOP_SOURCE: u16 = 1 << 14;
/// `mobtypes.txt` flag: the client draws this body from UOP (`BodyUsesUopFrames`). Its table holds ids in this range;
/// any other body, or one with no row, takes the MUL path.
pub const MOB_TYPE_FLAG_USES_UOP_FRAMES: u32 = 0x10000;
pub const MOB_TYPE_BODY_IDS: std::ops::RangeInclusive<u16> = 1..=0x7FF;
/// `mobtypes.txt` flags read by the movement action (ClassicUO `AnimationFlags`; the client's tests are named, Classic
/// 7.0.103.0, confirmed): a monster idles on action 8 when it has it (`MobTypes_IdleAt8` 0x00597870).
pub const MOB_TYPE_FLAG_IDLE_AT_8_FRAME: u32 = 0x4;
/// A monster runs with action 19 when it has it (`MobTypes_CanFly` 0x00597960, which also needs the client's flying
/// gate `DAT_00742cf8`).
pub const MOB_TYPE_FLAG_CAN_FLY: u32 = 0x8;
/// An animal takes the monster movement actions (`MobTypes_IsLowGroupExtended` 0x005979d0).
pub const MOB_TYPE_FLAG_LOW_GROUP_EXTENDED: u32 = 0x20;
/// A monster takes the animal movement actions and layout (`MobTypes_IsLowGroup` 0x00597a00).
pub const MOB_TYPE_FLAG_LOW_GROUP: u32 = 0x40;
/// A monster with the people action layout: every movement plays action 0 (`MobTypes_IsPeopleGroup` 0x00597ae0).
pub const MOB_TYPE_FLAG_PEOPLE_GROUP: u32 = 0x400;

/// `MobileAnimCcBodyTypeRecord::group_type`: the `mobtypes.txt` type name (`MobTypes_GroupType` 0x005976b0, which
/// gives `MOB_TYPE_GROUP_EQUIPMENT` for a body without a row; confirmed).
pub const MOB_TYPE_GROUP_MONSTER: u8 = 0;
pub const MOB_TYPE_GROUP_SEA_MONSTER: u8 = 1;
pub const MOB_TYPE_GROUP_ANIMAL: u8 = 2;
pub const MOB_TYPE_GROUP_HUMAN: u8 = 3;
pub const MOB_TYPE_GROUP_EQUIPMENT: u8 = 4;

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

/// Directions of a mobile, in UO order (0 north .. 7 north-west).
pub const DIRECTION_COUNT: u8 = 8;
/// Classic stores world direction `FIRST_STORED_DIRECTION + k` at stored direction `k`.
pub const FIRST_STORED_DIRECTION: u8 = uocf::animation_sequence::FIRST_STORED_DIRECTION;
/// A world direction below `FIRST_STORED_DIRECTION` shows world direction `MIRROR_DIRECTION_SUM - d` mirrored.
pub const MIRROR_DIRECTION_SUM: u8 = uocf::animation_sequence::MIRROR_DIRECTION_SUM;

/// Body ids below this one take the low (monster) layout when `mobtypes.txt` has no row (ClassicUO
/// `CalculateTypeByGraphic`); ids from it to `PEOPLE_LAYOUT_FIRST_BODY` the animal layout, the rest the people
/// layout. `anim3.mul` moves the animal/monster split to `ANIM3_MONSTER_FIRST_BODY`. The original client has no id
/// ranges: a body without a row is `MOB_TYPE_GROUP_EQUIPMENT` whatever its id (`MobTypes_GroupType` 0x005976b0).
const ANIMAL_LAYOUT_FIRST_BODY: u16 = 200;
const ANIM3_MONSTER_FIRST_BODY: u16 = 300;
const PEOPLE_LAYOUT_FIRST_BODY: u16 = 400;
const ANIM2_FILE_INDEX: u8 = 1;
const ANIM3_FILE_INDEX: u8 = 2;

pub const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"MAPG";
pub const ANIMATION_MANIFEST_MAGIC: [u8; 4] = *b"MAAN";
pub const FRAME_MANIFEST_MAGIC: [u8; 4] = *b"MAFR";
pub const BODY_RESOLVE_MANIFEST_MAGIC: [u8; 4] = *b"MABR";
pub const BODY_TYPE_MANIFEST_MAGIC: [u8; 4] = *b"MABT";
pub const MOBILE_ANIM_CC_METADATA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimCcPageRecord {
    pub page_index: u32,
    pub frame_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub used_width: u32,
    pub used_height: u32,
    pub pixel_format: PagePixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobileAnimCcAnimationRecord {
    pub body_id: u16,
    pub action_id: u16,
    pub direction: u8,
    pub file_index: u8,
    pub source_index: u32,
    pub frame_start: u32,
    pub frame_count: u16,
    pub flags: u16,
    /// Frames per second the client plays this animation at: for a `MOB_TYPE_FLAG_USES_UOP_FRAMES` body's UOP
    /// record, the fps of `action_id` in the body's `AnimationSequence.uop` entry (`GetActionFps`, aliases resolved,
    /// flying animations on; `CMobile_SetUopAction` 0x005b6580 sets the frame time to 1000 / fps, Classic
    /// 7.0.103.0, confirmed). `NO_SEQUENCE_FPS` for the others: MUL frames, and UOP bodies without a sequence
    /// entry, whose frame time the client doesn't set there.
    pub fps: f32,
}

/// [`MobileAnimCcAnimationRecord::fps`] of an animation no sequence times: the player keeps its own frame time
/// (ClassicUO's `CHARACTER_ANIMATION_DELAY` for MUL bodies).
pub const NO_SEQUENCE_FPS: f32 = 0.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimCcFrameRecord {
    pub animation_index: u32,
    pub frame_index: u16,
    pub page_index: u32,
    pub page_frame_index: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub center_x: i16,
    pub center_y: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimCcBodyResolveRecord {
    pub body_id: u16,
    pub resolved_body_id: u16,
    pub hue: u16,
    pub file_index: u8,
    pub mount_height: i8,
    pub flags: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimCcBodyTypeRecord {
    pub body_id: u16,
    pub group_type: u8,
    pub flags: u32,
    /// Movement actions of the body's `AnimationSequence.uop` entry by group (`MOVE_ACTION_GROUPS`) and movement
    /// (`MOVE_ACTION_MOVEMENTS`), as `FrameSetLookup_GetMoveAction` 0x00408114 gives them with the alternate
    /// variant off; `NO_MOVE_ACTION` for a group the sequence does not have, and for every entry of a body without
    /// `MOB_TYPE_FLAG_USES_UOP_FRAMES` or without a sequence entry.
    pub move_actions: [[u16; MOVE_ACTION_MOVEMENT_COUNT]; MOVE_ACTION_GROUP_COUNT],
}

impl MobileAnimCcBodyTypeRecord {
    /// Action of the sequence for `movement` (`MOVE_STAND`, `MOVE_WALK`, `MOVE_RUN`) in group `group`
    /// (`GROUP_WAR`, `GROUP_PEACE`, `GROUP_MOUNTED`); `None` when it has no such group or the arguments are outside
    /// those sets.
    pub fn move_action(&self, group: i8, movement: u8) -> Option<u16> {
        crate::animation::move_actions::move_action(&self.move_actions, group, movement)
    }
}

/// Where a server body's frames are: the body id within its source and the `anim*.mul` file, or UOP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicFramesBody {
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

/// Stored direction (0..=4) and mirroring of world direction `direction` (0 north .. 7
/// north-west): world `FIRST_STORED_DIRECTION + k` is stored at `k`; world 0, 1 and 2 show world 6, 5 and 4
/// mirrored horizontally about the frame anchor (ClassicUO `GetAnimDirection`, and the client's stored direction
/// table at 0x007087dc, [3, 2, 1, 0, 1, 2, 3, 4], with its mirror table at 0x007087fc, [1, 1, 1, 0, 0, 0, 0, 0]:
/// `GetObjectLayerFrame` 0x005441e0, `GetMobileMulFrame` 0x0044e8e0, Classic 7.0.103.0; confirmed). `None` outside
/// 0..`DIRECTION_COUNT`.
pub fn classic_stored_direction(direction: u8) -> Option<(u8, bool)> {
    if direction >= DIRECTION_COUNT {
        return None;
    }
    let mirrored = direction < FIRST_STORED_DIRECTION;
    let world = if mirrored { MIRROR_DIRECTION_SUM - direction } else { direction };
    Some((world - FIRST_STORED_DIRECTION, mirrored))
}

pub struct MobileAnimCcPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    packing_mode: AtlasPackingMode,
    pages: Vec<MobileAnimCcPageRecord>,
    animations: Vec<MobileAnimCcAnimationRecord>,
    frames: Vec<MobileAnimCcFrameRecord>,
    body_resolve: Vec<MobileAnimCcBodyResolveRecord>,
    body_types: Vec<MobileAnimCcBodyTypeRecord>,
    strides: Vec<MobileStrideRecord>,
    /// Indices of the animation records of each (body, action, direction).
    animation_lookup: HashMap<(u16, u16, u8), Vec<usize>>,
    page_cache: AtlasPageCache,
}

impl MobileAnimCcPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        Self::load_with_options(path, AtlasCacheOptions::disabled())
    }

    pub fn load_with_options(path: impl AsRef<Path>, options: AtlasCacheOptions) -> eyre::Result<Self> {
        let package = UddpReader::load(path.as_ref())
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package_with_options(package, options)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        Self::from_uddp_package_with_options(package, AtlasCacheOptions::disabled())
    }

    pub fn from_uddp_package_with_options(
        package: UddpReader,
        options: AtlasCacheOptions,
    ) -> eyre::Result<Self> {
        let page_manifest = read_path_entry_cow(&package, PAGE_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_cc.uddp missing metadata/pages.bin")?;
        let animation_manifest = read_path_entry_cow(&package, ANIMATION_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_cc.uddp missing metadata/animations.bin")?;
        let frame_manifest = read_path_entry_cow(&package, FRAME_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_cc.uddp missing metadata/frames.bin")?;
        let body_resolve_manifest = read_path_entry_cow(&package, BODY_RESOLVE_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_cc.uddp missing metadata/body_resolve.bin")?;
        let body_type_manifest = read_path_entry_cow(&package, BODY_TYPE_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_cc.uddp missing metadata/body_types.bin")?;

        let (page_width, page_height, page_gutter, page_packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let animations = parse_animation_manifest(&animation_manifest)?;
        let frames = parse_frame_manifest(&frame_manifest)?;
        let body_resolve = parse_body_resolve_manifest(&body_resolve_manifest)?;
        let body_types = parse_body_type_manifest(&body_type_manifest)?;
        let strides = read_stride_records(&package)?;
        let mut animation_lookup = HashMap::<(u16, u16, u8), Vec<usize>>::new();
        for (index, animation) in animations.iter().enumerate() {
            animation_lookup
                .entry((animation.body_id, animation.action_id, animation.direction))
                .or_default()
                .push(index);
        }

        Ok(Self {
            package,
            atlas_width: page_width,
            atlas_height: page_height,
            gutter: page_gutter,
            packing_mode: page_packing_mode,
            pages,
            animations,
            frames,
            body_resolve,
            body_types,
            strides,
            animation_lookup,
            page_cache: AtlasPageCache::new(options),
        })
    }

    pub fn atlas_width(&self) -> u32 {
        self.atlas_width
    }

    pub fn atlas_height(&self) -> u32 {
        self.atlas_height
    }

    pub fn gutter(&self) -> u16 {
        self.gutter
    }

    pub fn packing_mode(&self) -> AtlasPackingMode {
        self.packing_mode
    }

    pub fn pages(&self) -> &[MobileAnimCcPageRecord] {
        &self.pages
    }

    pub fn animations(&self) -> &[MobileAnimCcAnimationRecord] {
        &self.animations
    }

    pub fn frames(&self) -> &[MobileAnimCcFrameRecord] {
        &self.frames
    }

    pub fn body_resolve(&self) -> &[MobileAnimCcBodyResolveRecord] {
        &self.body_resolve
    }

    pub fn body_types(&self) -> &[MobileAnimCcBodyTypeRecord] {
        &self.body_types
    }

    pub fn body_resolve_record(&self, body_id: u16) -> Option<&MobileAnimCcBodyResolveRecord> {
        self.body_resolve
            .binary_search_by_key(&body_id, |record| record.body_id)
            .ok()
            .map(|index| &self.body_resolve[index])
    }

    pub fn body_type_record(&self, body_id: u16) -> Option<&MobileAnimCcBodyTypeRecord> {
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

    /// The animation of record body `body_id` in any animation file, from the source the client would use. A package
    /// can hold both a MUL and a UOP entry for one action, as when a shard replaces a body in the MUL; the other source
    /// only fills in when that one has none. Layer items and mounts don't follow this rule: the client tests the
    /// wearer, not the requested id (`docs/client_research/ghidra-classic-frames/results/uop_reachability.md`). For a
    /// server body, whose MUL frames can sit under another id in another file, use [`Self::server_body_animation`].
    pub fn animation(&self, body_id: u16, action_id: u16, direction: u8) -> Option<&MobileAnimCcAnimationRecord> {
        animation_from_source(self.matching_animations(body_id, action_id, direction), self.body_uses_uop_frames(body_id))
    }

    fn matching_animations(
        &self,
        body_id: u16,
        action_id: u16,
        direction: u8,
    ) -> impl Iterator<Item = &MobileAnimCcAnimationRecord> {
        self.animation_lookup
            .get(&(body_id, action_id, direction))
            .into_iter()
            .flatten()
            .map(|&index| &self.animations[index])
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

    /// The animation the client draws for server body `body_id`, `action_id` and stored direction `direction`
    /// (see [`classic_stored_direction`]): the records of [`Self::frames_body`], from its `anim*.mul` file on the MUL
    /// side. The other source fills in when the client's has none, as in [`Self::animation`].
    pub fn server_body_animation(
        &self,
        body_id: u16,
        action_id: u16,
        direction: u8,
    ) -> Option<&MobileAnimCcAnimationRecord> {
        let frames = self.frames_body(body_id);
        let matching = self.matching_animations(frames.body_id, action_id, direction);
        if frames.uop {
            return animation_from_source(matching, true);
        }
        animation_from_source(matching.filter(|animation| animation.file_index == frames.file_index), false)
    }

    /// Action layout of server body `body_id` (`MOB_TYPE_GROUP_*`) and its `mobtypes.txt` flags: those of the
    /// `Body.def` graphic when it has one (`MobTypes_GroupType` 0x005976b0 on the remapped body; confirmed). Without
    /// a row, the layout follows the id and file of its frames (ClassicUO `CalculateTypeByGraphic`, by choice) and
    /// the flags are 0. The client takes `MOB_TYPE_GROUP_EQUIPMENT`, the people layout without mounted actions, for
    /// any id (`docs/client_research/ghidra-classic-frames/results/classic_frames.md`); the shipped `anim.mul`
    /// bodies with frames and no row (492, 573, 634, 640, 641, 642, 970) all get the people layout either way.
    pub fn body_layout(&self, body_id: u16) -> (u8, u32) {
        let frames = self.frames_body(body_id);
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

    /// Action the Classic client plays for server body `body_id` standing, walking or running (`MOVE_STAND`,
    /// `MOVE_WALK`, `MOVE_RUN`) in peace mode, `mounted` when the body rides a mount (a mount's own body takes its
    /// layout's actions), with no server animation, nothing in the hands and no flight.
    ///
    /// A UOP-flagged body (`MOB_TYPE_FLAG_USES_UOP_FRAMES`) plays the move action of its sequence, as the client does
    /// (`CMobile_UpdateUopMoveAction` 0x005b6e40 through `FrameSetLookup_GetMoveAction` 0x00408114, Classic
    /// 7.0.103.0; confirmed): group `GROUP_MOUNTED` when `mounted` and the sequence has it, else `GROUP_PEACE`
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
    /// "Has it" tests stored direction 0 of [`Self::server_body_animation`]. `None` for another movement.
    pub fn movement_action(&self, body_id: u16, movement: u8, mounted: bool) -> Option<u16> {
        if ![MOVE_STAND, MOVE_WALK, MOVE_RUN].contains(&movement) {
            return None;
        }
        let (group_type, flags) = self.body_layout(body_id);
        let uop = flags & MOB_TYPE_FLAG_USES_UOP_FRAMES != 0;
        if uop {
            let record = self.body_type_record(self.frames_body(body_id).type_body_id);
            let sequence_action = record.and_then(|record| {
                let mounted_action = record.move_action(GROUP_MOUNTED, movement).filter(|_| mounted);
                mounted_action.or_else(|| record.move_action(GROUP_PEACE, movement))
            });
            if let Some(action) = sequence_action {
                return Some(action);
            }
        }
        let has = |action: u16| self.server_body_animation(body_id, action, 0).is_some();
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
        let action = match group_type {
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
        };
        Some(action)
    }

    pub fn animation_frames(&self, animation: &MobileAnimCcAnimationRecord) -> &[MobileAnimCcFrameRecord] {
        let start = animation.frame_start as usize;
        let end = start + animation.frame_count as usize;
        self.frames.get(start..end).unwrap_or(&[])
    }

    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .iter()
            .find(|page| page.page_index == page_index)
            .map(|page| page.pixel_format)
            .ok_or_else(|| eyre::eyre!("missing mobile animation atlas page metadata for {page_index}"))?;
        self.page_cache.read_page_bytes(page_index, || {
            read_path_entry(&self.package, &page_entry_path(page_index, fmt))
                .wrap_err_with(|| format!("unpack mobile animation atlas page {page_index}"))
        })
    }

    pub fn read_page_rgba(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let page = self
            .pages
            .iter()
            .find(|page| page.page_index == page_index)
            .ok_or_else(|| eyre::eyre!("missing mobile animation atlas page metadata for {page_index}"))?;
        let page_bytes = self.read_page_bytes(page_index)?;
        decode_atlas_page_rgba(
            &page_bytes,
            page.pixel_format,
            page.atlas_width,
            page.atlas_height,
            page.used_width,
            page.used_height,
        )
    }

    pub fn read_frame_rgba(&self, frame: &MobileAnimCcFrameRecord) -> eyre::Result<Vec<u8>> {
        self.read_frame_rgba_in(frame, &mut PageMemo::default())
    }

    /// [`Self::read_frame_rgba`], the atlas page read through `pages`: the frames of a clip, which mostly share a
    /// page, read it once.
    fn read_frame_rgba_in(&self, frame: &MobileAnimCcFrameRecord, pages: &mut PageMemo) -> eyre::Result<Vec<u8>> {
        if frame.page_index == MISSING_PAGE_INDEX || frame.width == 0 || frame.height == 0 {
            return Ok(Vec::new());
        }
        let page = self
            .pages
            .iter()
            .find(|page| page.page_index == frame.page_index)
            .ok_or_else(|| {
                eyre::eyre!(
                    "missing mobile animation atlas page metadata for {}",
                    frame.page_index
                )
            })?;
        let page_bytes = pages.page(frame.page_index, |page_index| self.read_page_bytes(page_index))?;
        extract_atlas_subrect_rgba(
            page_bytes,
            page.pixel_format,
            page.atlas_width,
            page.atlas_height,
            page.used_width,
            page.used_height,
            frame.x,
            frame.y,
            frame.width,
            frame.height,
        )
    }
}

impl MobileAnimClips for MobileAnimCcPackage {
    fn stride_source(&self) -> StrideSource {
        StrideSource::Classic
    }

    fn stride_records(&self) -> &[MobileStrideRecord] {
        &self.strides
    }

    fn stride_clips(&self) -> Vec<(u32, u16)> {
        let bodies: BTreeSet<u16> = self.animations.iter().map(|record| record.body_id)
            .chain(self.body_resolve.iter().map(|record| record.body_id))
            .chain(self.body_types.iter().map(|record| record.body_id))
            .collect();
        let mut clips = BTreeSet::new();
        for body in bodies {
            // A body with UOP frames has the animation sequence's war moves too.
            let sequence = self.body_type_record(self.frames_body(body).type_body_id);
            for movement in [MOVE_WALK, MOVE_RUN] {
                let war = sequence.and_then(|record| record.move_action(GROUP_WAR, movement));
                for action in [self.movement_action(body, movement, false), war].into_iter().flatten() {
                    clips.insert((u32::from(body), action));
                }
            }
        }
        clips.into_iter().collect()
    }

    /// The client stores five directions and mirrors the other three ([`classic_stored_direction`]). A frame's anchor
    /// is its centre x from the left and its centre y from the bottom (ClassicUO: the frame's top at the anchor minus
    /// its height and centre y).
    fn clip(&self, body: u32, action: u16, direction: u8) -> eyre::Result<Option<MobileClip>> {
        self.clip_in(body, action, direction, &mut PageMemo::default())
    }

    fn side_clips(&self, body: u32, action: u16) -> eyre::Result<[Option<MobileClip>; STRIDE_FACING_COUNT]> {
        let mut pages = PageMemo::default();
        let mut clips: [Option<MobileClip>; STRIDE_FACING_COUNT] = Default::default();
        for (clip, direction) in clips.iter_mut().zip(STRIDE_FACINGS) {
            *clip = self.clip_in(body, action, direction, &mut pages)?;
        }
        Ok(clips)
    }
}

impl MobileAnimCcPackage {
    /// [`MobileAnimClips::clip`], the atlas pages read through `pages`.
    fn clip_in(&self, body: u32, action: u16, direction: u8, pages: &mut PageMemo)
        -> eyre::Result<Option<MobileClip>> {
        let Ok(body) = u16::try_from(body) else { return Ok(None) };
        let Some((stored_direction, mirrored)) = classic_stored_direction(direction) else { return Ok(None) };
        let Some(animation) = self.server_body_animation(body, action, stored_direction) else { return Ok(None) };
        let frames = self.animation_frames(animation).iter().filter(|frame| frame.width > 0 && frame.height > 0)
            .map(|frame| Ok(MobileClipFrame {
                width: frame.width,
                height: frame.height,
                anchor_x: i32::from(frame.center_x),
                anchor_y: i32::from(frame.height) + i32::from(frame.center_y),
                rgba: self.read_frame_rgba_in(frame, pages)?,
            }))
            .collect::<eyre::Result<Vec<_>>>()?;
        Ok((!frames.is_empty()).then_some(MobileClip { frames, mirrored, fps: animation.fps,
            stored: u32::from(stored_direction) }))
    }
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<MobileAnimCcPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC { eyre::bail!("invalid mobile animation page magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("mobile animation page manifest", version, MOBILE_ANIM_CC_METADATA_VERSION)?;
    let width = cursor.read_u32::<LittleEndian>()?;
    let height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid mobile animation page pixel format"))?;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid mobile animation atlas packing mode"))?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(count);
    for _ in 0..count {
        pages.push(MobileAnimCcPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            frame_count: cursor.read_u32::<LittleEndian>()?,
            atlas_width: cursor.read_u32::<LittleEndian>()?,
            atlas_height: cursor.read_u32::<LittleEndian>()?,
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
            pixel_format,
        });
    }
    Ok((width, height, gutter, packing_mode, pages))
}

fn parse_animation_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimCcAnimationRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != ANIMATION_MANIFEST_MAGIC { eyre::bail!("invalid mobile animation manifest magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("mobile animation manifest", version, MOBILE_ANIM_CC_METADATA_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut animations = Vec::with_capacity(count);
    for _ in 0..count {
        animations.push(MobileAnimCcAnimationRecord {
            body_id: cursor.read_u16::<LittleEndian>()?,
            action_id: cursor.read_u16::<LittleEndian>()?,
            direction: cursor.read_u8()?,
            file_index: cursor.read_u8()?,
            source_index: cursor.read_u32::<LittleEndian>()?,
            frame_start: cursor.read_u32::<LittleEndian>()?,
            frame_count: cursor.read_u16::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
            fps: cursor.read_f32::<LittleEndian>()?,
        });
    }
    Ok(animations)
}

fn parse_frame_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimCcFrameRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != FRAME_MANIFEST_MAGIC { eyre::bail!("invalid mobile animation frame magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("mobile animation frame manifest", version, MOBILE_ANIM_CC_METADATA_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        frames.push(MobileAnimCcFrameRecord {
            animation_index: cursor.read_u32::<LittleEndian>()?,
            frame_index: cursor.read_u16::<LittleEndian>()?,
            page_index: cursor.read_u32::<LittleEndian>()?,
            page_frame_index: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
            center_x: cursor.read_i16::<LittleEndian>()?,
            center_y: cursor.read_i16::<LittleEndian>()?,
        });
    }
    Ok(frames)
}

fn parse_body_resolve_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimCcBodyResolveRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != BODY_RESOLVE_MANIFEST_MAGIC { eyre::bail!("invalid mobile animation body resolve magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("mobile animation body resolve manifest", version, MOBILE_ANIM_CC_METADATA_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        records.push(MobileAnimCcBodyResolveRecord {
            body_id: cursor.read_u16::<LittleEndian>()?,
            resolved_body_id: cursor.read_u16::<LittleEndian>()?,
            hue: cursor.read_u16::<LittleEndian>()?,
            file_index: cursor.read_u8()?,
            mount_height: cursor.read_i8()?,
            flags: cursor.read_u16::<LittleEndian>()?,
        });
    }
    Ok(records)
}

fn animation_from_source<'a>(
    matching: impl Iterator<Item = &'a MobileAnimCcAnimationRecord>,
    wants_uop: bool,
) -> Option<&'a MobileAnimCcAnimationRecord> {
    let mut other_source = None;
    for animation in matching {
        if (animation.flags & MOBILE_ANIM_CC_FLAG_UOP_SOURCE != 0) == wants_uop {
            return Some(animation);
        }
        other_source.get_or_insert(animation);
    }
    other_source
}

fn parse_body_type_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimCcBodyTypeRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != BODY_TYPE_MANIFEST_MAGIC { eyre::bail!("invalid mobile animation body type magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("mobile animation body type manifest", version, MOBILE_ANIM_CC_METADATA_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        records.push(MobileAnimCcBodyTypeRecord {
            body_id: cursor.read_u16::<LittleEndian>()?,
            group_type: cursor.read_u8()?,
            flags: cursor.read_u32::<LittleEndian>()?,
            move_actions: {
                let mut move_actions = [[NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT]; MOVE_ACTION_GROUP_COUNT];
                for action in move_actions.iter_mut().flatten() {
                    *action = cursor.read_u16::<LittleEndian>()?;
                }
                move_actions
            },
        });
    }
    Ok(records)
}

pub fn page_entry_path(page_index: u32, fmt: PagePixelFormat) -> String {
    format!("pages/{page_index:05}.{}", fmt.extension())
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

    #[test]
    fn package_reader_loads_metadata_and_page_payload() {
        const TEST_FPS: f32 = 12.5;
        let page_manifest = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, MOBILE_ANIM_CC_METADATA_VERSION).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 8).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 8).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 4).unwrap();
            bytes.push(PagePixelFormat::Rgba8888 as u8);
            bytes.push(AtlasPackingMode::Bc7Oriented as u8);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 8).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 8).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 4).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 4).unwrap();
            bytes
        };
        let animation_manifest = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&ANIMATION_MANIFEST_MAGIC);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, MOBILE_ANIM_CC_METADATA_VERSION).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 2).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 3).unwrap();
            bytes.push(4);
            bytes.push(0);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 20).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_f32::<LittleEndian>(&mut bytes, TEST_FPS).unwrap();
            bytes
        };
        let frame_manifest = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&FRAME_MANIFEST_MAGIC);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, MOBILE_ANIM_CC_METADATA_VERSION).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 2).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 2).unwrap();
            byteorder::WriteBytesExt::write_i16::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_i16::<LittleEndian>(&mut bytes, -1).unwrap();
            bytes
        };
        let body_resolve_manifest = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&BODY_RESOLVE_MANIFEST_MAGIC);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, MOBILE_ANIM_CC_METADATA_VERSION).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 7).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 8).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 9).unwrap();
            bytes.push(2);
            byteorder::WriteBytesExt::write_i8(&mut bytes, -1).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, BODY_RESOLVE_FLAG_BODY_DEF | BODY_RESOLVE_FLAG_BODYCONV_DEF).unwrap();
            bytes
        };
        let body_type_manifest = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&BODY_TYPE_MANIFEST_MAGIC);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, MOBILE_ANIM_CC_METADATA_VERSION).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 7).unwrap();
            bytes.push(3);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 0x8000_0001).unwrap();
            for _ in 0..MOVE_ACTION_GROUP_COUNT * MOVE_ACTION_MOVEMENT_COUNT {
                byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, NO_MOVE_ACTION).unwrap();
            }
            bytes
        };
        let page = vec![255u8; 4 * 4 * 4];
        let page_path = page_entry_path(0, PagePixelFormat::Rgba8888);
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (path, data, data_type) in [
            (PAGE_MANIFEST_ENTRY_PATH, page_manifest.as_slice(), DataType::Metadata),
            (ANIMATION_MANIFEST_ENTRY_PATH, animation_manifest.as_slice(), DataType::Metadata),
            (FRAME_MANIFEST_ENTRY_PATH, frame_manifest.as_slice(), DataType::Metadata),
            (BODY_RESOLVE_MANIFEST_ENTRY_PATH, body_resolve_manifest.as_slice(), DataType::Metadata),
            (BODY_TYPE_MANIFEST_ENTRY_PATH, body_type_manifest.as_slice(), DataType::Metadata),
            (page_path.as_str(), page.as_slice(), DataType::Texture),
        ] {
            builder.add_file(AddFileRequest {
                data_type: data_type as u8,
                compression: CompressionFlag::ZstdNoDict,
                width: 0,
                height: 0,
                virtual_path: Some(path),
                path_hash64: None,
                id: None,
                data,
            }).unwrap();
        }

        let package = MobileAnimCcPackage::from_uddp_package(
            UddpReader::open(builder.build().unwrap()).unwrap(),
        ).unwrap();

        assert_eq!(package.pages().len(), 1);
        assert_eq!(package.animation(2, 3, 4).unwrap().source_index, 20);
        assert_eq!(package.animation(2, 3, 4).unwrap().fps, TEST_FPS);
        assert_eq!(package.animation_frames(package.animation(2, 3, 4).unwrap()).len(), 1);
        assert_eq!(package.body_resolve_record(7).unwrap().resolved_body_id, 8);
        assert_eq!(package.body_type_record(7).unwrap().group_type, 3);
        assert_eq!(package.body_type_record(7).unwrap().move_action(GROUP_PEACE, MOVE_WALK), None);
        assert!(!package.body_uses_uop_frames(7));
        assert_eq!(package.read_page_bytes(0).unwrap().len(), page.len());
        let animation = package.animation(2, 3, 4).unwrap();
        let frame = &package.animation_frames(animation)[0];
        assert_eq!(
            package.read_frame_rgba(frame).unwrap().len(),
            2 * 2 * 4
        );
    }

    fn manifest(magic: [u8; 4], count: usize) -> Vec<u8> {
        let mut bytes = magic.to_vec();
        bytes.extend_from_slice(&MOBILE_ANIM_CC_METADATA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(count as u32).to_le_bytes());
        bytes
    }

    /// A package of `animations` (no frames), `body_resolve` and `body_types` (sorted by body as the packer writes
    /// them), with no pages.
    fn package_from(
        animations: &[MobileAnimCcAnimationRecord],
        body_resolve: &[MobileAnimCcBodyResolveRecord],
        body_types: &[MobileAnimCcBodyTypeRecord],
    ) -> MobileAnimCcPackage {
        let mut pages = PAGE_MANIFEST_MAGIC.to_vec();
        pages.extend_from_slice(&MOBILE_ANIM_CC_METADATA_VERSION.to_le_bytes());
        for value in [8u32, 8, 4] {
            pages.extend_from_slice(&value.to_le_bytes());
        }
        pages.extend_from_slice(&[PagePixelFormat::Rgba8888 as u8, AtlasPackingMode::Bc7Oriented as u8]);
        pages.extend_from_slice(&0u32.to_le_bytes());
        let mut animation_bytes = manifest(ANIMATION_MANIFEST_MAGIC, animations.len());
        for animation in animations {
            animation_bytes.extend_from_slice(&animation.body_id.to_le_bytes());
            animation_bytes.extend_from_slice(&animation.action_id.to_le_bytes());
            animation_bytes.extend_from_slice(&[animation.direction, animation.file_index]);
            animation_bytes.extend_from_slice(&animation.source_index.to_le_bytes());
            animation_bytes.extend_from_slice(&animation.frame_start.to_le_bytes());
            animation_bytes.extend_from_slice(&animation.frame_count.to_le_bytes());
            animation_bytes.extend_from_slice(&animation.flags.to_le_bytes());
            animation_bytes.extend_from_slice(&animation.fps.to_le_bytes());
        }
        let frames = manifest(FRAME_MANIFEST_MAGIC, 0);
        let mut resolve_bytes = manifest(BODY_RESOLVE_MANIFEST_MAGIC, body_resolve.len());
        let mut body_resolve = body_resolve.to_vec();
        body_resolve.sort_by_key(|record| record.body_id);
        for record in &body_resolve {
            resolve_bytes.extend_from_slice(&record.body_id.to_le_bytes());
            resolve_bytes.extend_from_slice(&record.resolved_body_id.to_le_bytes());
            resolve_bytes.extend_from_slice(&record.hue.to_le_bytes());
            resolve_bytes.extend_from_slice(&[record.file_index, record.mount_height as u8]);
            resolve_bytes.extend_from_slice(&record.flags.to_le_bytes());
        }
        let mut type_bytes = manifest(BODY_TYPE_MANIFEST_MAGIC, body_types.len());
        let mut body_types = body_types.to_vec();
        body_types.sort_by_key(|record| record.body_id);
        for record in &body_types {
            type_bytes.extend_from_slice(&record.body_id.to_le_bytes());
            type_bytes.push(record.group_type);
            type_bytes.extend_from_slice(&record.flags.to_le_bytes());
            for action in record.move_actions.iter().flatten() {
                type_bytes.extend_from_slice(&action.to_le_bytes());
            }
        }
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (path, data) in [
            (PAGE_MANIFEST_ENTRY_PATH, pages.as_slice()),
            (ANIMATION_MANIFEST_ENTRY_PATH, animation_bytes.as_slice()),
            (FRAME_MANIFEST_ENTRY_PATH, frames.as_slice()),
            (BODY_RESOLVE_MANIFEST_ENTRY_PATH, resolve_bytes.as_slice()),
            (BODY_TYPE_MANIFEST_ENTRY_PATH, type_bytes.as_slice()),
        ] {
            builder.add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::ZstdNoDict,
                width: 0,
                height: 0,
                virtual_path: Some(path),
                path_hash64: None,
                id: None,
                data,
            }).unwrap();
        }
        MobileAnimCcPackage::from_uddp_package(UddpReader::open(builder.build().unwrap()).unwrap()).unwrap()
    }

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

    fn resolve(body_id: u16, resolved_body_id: u16, file_index: u8, flags: u16) -> MobileAnimCcBodyResolveRecord {
        MobileAnimCcBodyResolveRecord { body_id, resolved_body_id, hue: 0, file_index, mount_height: 0, flags }
    }

    fn body_type(body_id: u16, group_type: u8, flags: u32) -> MobileAnimCcBodyTypeRecord {
        MobileAnimCcBodyTypeRecord {
            body_id,
            group_type,
            flags: MOB_TYPE_FLAG_FOUND | flags,
            move_actions: NO_MOVE_ACTIONS,
        }
    }

    type MoveActions = [[u16; MOVE_ACTION_MOVEMENT_COUNT]; MOVE_ACTION_GROUP_COUNT];

    const NO_MOVE_ACTIONS: MoveActions = [[NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT]; MOVE_ACTION_GROUP_COUNT];

    /// `mobtypes.txt` rows carry this flag (the packer's "found" bit).
    const MOB_TYPE_FLAG_FOUND: u32 = 0x8000_0000;

    #[test]
    fn server_bodies_take_body_def_then_bodyconv_and_their_own_file() {
        const ANIM4: u8 = 3;
        let (bodyconv_body, body_def_body, chained_body, uop_body) = (250, 9, 8, 700);
        let package = package_from(
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
            ],
            &[body_type(uop_body, MOB_TYPE_GROUP_MONSTER, MOB_TYPE_FLAG_USES_UOP_FRAMES)],
        );

        let converted = package.frames_body(bodyconv_body);
        assert_eq!((converted.body_id, converted.file_index, converted.uop), (17, ANIM4, false));
        // The id 17 also exists in anim.mul: the lookup takes the record of the body's file.
        assert_eq!(package.server_body_animation(bodyconv_body, 0, 0).unwrap().file_index, ANIM4);
        // Body.def applies once: 8 -> 9, and 9's own Body.def row is not followed.
        let renamed = package.frames_body(chained_body);
        assert_eq!((renamed.body_id, renamed.type_body_id), (body_def_body, body_def_body));
        assert_eq!(package.frames_body(body_def_body).body_id, 42);
        assert_eq!(package.server_body_animation(body_def_body, 0, 0).unwrap().body_id, 42);
        let uop = package.frames_body(uop_body);
        assert_eq!((uop.body_id, uop.uop), (uop_body, true));
        assert!(package.server_body_animation(uop_body, 0, 0).is_some());
        assert!(package.server_body_animation(uop_body, 1, 0).is_none());
    }

    #[test]
    fn movement_actions_follow_the_body_layout() {
        let (human, horse, uop_animal, monster, idle_monster, people_monster, sea_monster, unlisted_animal) =
            (400, 204, 277, 1, 2, 3, 150, 220);
        let (equipment, unlisted_people, flying_monster, frameless_flyer) = (970, 492, 4, 5);
        let package = package_from(
            &[animation(horse, 1, 0, 0), animation(idle_monster, 8, 0, 0), animation(flying_monster, 19, 0, 0)],
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
        let actions = |body: u16, mounted: bool| {
            [MOVE_STAND, MOVE_WALK, MOVE_RUN].map(|movement| package.movement_action(body, movement, mounted).unwrap())
        };

        assert_eq!(actions(human, false), [4, 0, 2]);
        assert_eq!(actions(human, true), [25, 23, 24]);
        assert_eq!(actions(equipment, true), [25, 23, 24]);
        assert_eq!(actions(horse, true), [2, 0, 1]);
        // A UOP body without a sequence entry: the fixed actions.
        assert_eq!(actions(uop_animal, false), [25, 22, 24]);
        assert_eq!(actions(monster, false), [1, 0, 0]);
        assert_eq!(actions(idle_monster, false), [8, 0, 0]);
        assert_eq!(actions(people_monster, false), [0, 0, 0]);
        assert_eq!(actions(sea_monster, false), [2, 0, 1]);
        // A flying monster runs with 19 when it has it.
        assert_eq!(actions(flying_monster, false), [1, 0, 19]);
        assert_eq!(actions(frameless_flyer, false), [1, 0, 0]);
        // No row: the id range gives the animal layout; without action 1 it runs with 2.
        assert_eq!(package.body_layout(unlisted_animal), (MOB_TYPE_GROUP_ANIMAL, 0));
        assert_eq!(actions(unlisted_animal, false), [2, 0, 2]);
        // No row from 400 on: the human layout.
        assert_eq!(package.body_layout(unlisted_people), (MOB_TYPE_GROUP_HUMAN, 0));
        assert_eq!(actions(unlisted_people, true), [25, 23, 24]);
        let unknown_movement = 3;
        assert_eq!(package.movement_action(human, unknown_movement, false), None);
    }

    #[test]
    fn uop_bodies_take_the_move_actions_of_their_sequence() {
        let (monster, human, animal) = (714, 666, 432);
        let with_actions = |body_id: u16, group_type: u8, peace: [u16; MOVE_ACTION_MOVEMENT_COUNT], mounted| {
            let mut record = body_type(body_id, group_type, MOB_TYPE_FLAG_USES_UOP_FRAMES);
            record.move_actions = [[NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT], peace, mounted];
            record
        };
        let package = package_from(
            &[],
            &[],
            &[
                with_actions(monster, MOB_TYPE_GROUP_MONSTER, [1, 0, 0], [NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT]),
                with_actions(human, MOB_TYPE_GROUP_HUMAN, [37, 1, 3], [25, 23, 24]),
                with_actions(animal, MOB_TYPE_GROUP_ANIMAL, [31, 22, 24], [31, 29, 30]),
            ],
        );
        let actions = |body: u16, mounted: bool| {
            [MOVE_STAND, MOVE_WALK, MOVE_RUN].map(|movement| package.movement_action(body, movement, mounted).unwrap())
        };

        assert_eq!(actions(monster, false), [1, 0, 0]);
        // Without a mounted group the peace group plays.
        assert_eq!(actions(monster, true), [1, 0, 0]);
        assert_eq!(actions(human, false), [37, 1, 3]);
        assert_eq!(actions(human, true), [25, 23, 24]);
        assert_eq!(actions(animal, false), [31, 22, 24]);
        assert_eq!(actions(animal, true), [31, 29, 30]);
    }

    #[test]
    fn world_directions_map_to_five_stored_directions_with_mirrors() {
        let mapped = (0..DIRECTION_COUNT).map(|direction| classic_stored_direction(direction).unwrap()).collect::<Vec<_>>();
        assert_eq!(
            mapped,
            [(3, true), (2, true), (1, true), (0, false), (1, false), (2, false), (3, false), (4, false)]
        );
        assert_eq!(classic_stored_direction(DIRECTION_COUNT), None);
    }

    #[test]
    fn animation_takes_the_source_the_client_uses_and_the_other_only_when_alone() {
        let record = |flags| MobileAnimCcAnimationRecord {
            body_id: 1,
            action_id: 0,
            direction: 0,
            file_index: 0,
            source_index: 0,
            frame_start: 0,
            frame_count: 1,
            flags,
            fps: NO_SEQUENCE_FPS,
        };
        let (mul, uop) = (record(0), record(MOBILE_ANIM_CC_FLAG_UOP_SOURCE));

        assert_eq!(animation_from_source([mul, uop].iter(), false), Some(&mul));
        assert_eq!(animation_from_source([mul, uop].iter(), true), Some(&uop));
        assert_eq!(animation_from_source([uop, mul].iter(), false), Some(&mul));
        assert_eq!(animation_from_source([uop].iter(), false), Some(&uop));
        assert_eq!(animation_from_source([mul].iter(), true), Some(&mul));
        assert_eq!(animation_from_source([].iter(), true), None);
    }
}
