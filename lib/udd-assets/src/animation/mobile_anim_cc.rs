use std::collections::{BTreeSet, HashMap};
use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};
use udd_container::UddpReader;

use crate::common::{
    decode_atlas_page_rgba, ensure_manifest_version, extract_atlas_subrect_rgba, read_path_entry,
    read_path_entry_cow, AtlasCacheOptions, AtlasPageCache,
};
use crate::animation::mobile_pages::{ClipPages, PageLayout, PageSet, StoredPage, StoredPageCache};
use crate::animation::mobile_strides::{
    read_stride_records, ClipFrameSource, MobileAnimClips, MobileClip, MobileClipFrame, MobileClipFrameExtent,
    MobileClipStoredFrame, MobileStrideRecord, PageMemo, StoredFramePixels, StrideSource, STRIDE_FACINGS,
    STRIDE_FACING_COUNT,
};
use crate::animation::mobile_upscaled::{record_index, UpscaledFrameTable};
use crate::tex_art_cc::{AtlasPackingMode, PagePixelFormat};
pub use crate::animation::move_actions::{
    GROUP_MOUNTED, GROUP_PEACE, GROUP_WAR, MOVE_ACTION_GROUPS, MOVE_ACTION_GROUP_COUNT, MOVE_ACTION_MOVEMENTS,
    MOVE_ACTION_MOVEMENT_COUNT, MOVE_RUN, MOVE_STAND, MOVE_WALK, NO_MOVE_ACTION,
};

pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const ANIMATION_MANIFEST_ENTRY_PATH: &str = "metadata/animations.bin";
pub const FRAME_MANIFEST_ENTRY_PATH: &str = "metadata/frames.bin";
/// The server body table ([`MobileAnimCcBodyRecord`]), with the move action sets its rows refer to.
pub const BODY_TABLE_ENTRY_PATH: &str = "metadata/bodies.bin";

pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_FRAME_INDEX: u16 = u16::MAX;

/// Animation record flag: the frames came from `AnimationFrame*.uop`, not from `anim*.mul`.
pub const MOBILE_ANIM_CC_FLAG_UOP_SOURCE: u16 = 1 << 14;
/// `mobtypes.txt` flag: the client draws this body from UOP (`BodyUsesUopFrames`). The packer reads it for ids in
/// `1..=0x7FF` only; any other body, or one with no row, takes the MUL path.
pub const MOB_TYPE_FLAG_USES_UOP_FRAMES: u32 = 0x10000;
/// `mobtypes.txt` flags read by the movement action (ClassicUO `AnimationFlags`; the client's tests are named, Classic
/// 7.0.102.3, confirmed): a monster idles on action 8 when it has it (`MobTypes_IdleAt8` 0x00597870).
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

/// `MobileAnimCcBodyRecord::group_type`: the `mobtypes.txt` type name (`MobTypes_GroupType` 0x005976b0, which
/// gives `MOB_TYPE_GROUP_EQUIPMENT` for a body without a row; confirmed).
pub const MOB_TYPE_GROUP_MONSTER: u8 = 0;
pub const MOB_TYPE_GROUP_SEA_MONSTER: u8 = 1;
pub const MOB_TYPE_GROUP_ANIMAL: u8 = 2;
pub const MOB_TYPE_GROUP_HUMAN: u8 = 3;
pub const MOB_TYPE_GROUP_EQUIPMENT: u8 = 4;

/// Directions of a mobile, in UO order (0 north .. 7 north-west).
pub const DIRECTION_COUNT: u8 = 8;
/// Classic stores world direction `FIRST_STORED_DIRECTION + k` at stored direction `k`.
pub const FIRST_STORED_DIRECTION: u8 = uocf::animation_sequence::FIRST_STORED_DIRECTION;
/// A world direction below `FIRST_STORED_DIRECTION` shows world direction `MIRROR_DIRECTION_SUM - d` mirrored.
pub const MIRROR_DIRECTION_SUM: u8 = uocf::animation_sequence::MIRROR_DIRECTION_SUM;

pub const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"MAPG";
pub const ANIMATION_MANIFEST_MAGIC: [u8; 4] = *b"MAAN";
pub const FRAME_MANIFEST_MAGIC: [u8; 4] = *b"MAFR";
pub const BODY_TABLE_MAGIC: [u8; 4] = *b"MABD";
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
    /// 7.0.102.3, confirmed). `NO_SEQUENCE_FPS` for the others: MUL frames, and UOP bodies without a sequence
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

/// Body record flag: the client draws the body's frames from `AnimationFrame*.uop`, not from an `anim*.mul` file.
pub const BODY_FLAG_UOP_FRAMES: u8 = 1 << 0;
/// Body record flag: `mobtypes.txt` flags the body id itself for UOP frames. The source an animation record of this
/// id prefers ([`MobileAnimCcPackage::animation`]); a `Body.def` graphic can take a body to UOP frames without it.
pub const BODY_FLAG_OWN_UOP_FRAMES: u8 = 1 << 1;
/// [`MobileAnimCcBodyRecord::move_action_set`] of a body whose action layout has no sequence move actions.
pub const NO_MOVE_ACTION_SET: u16 = u16::MAX;
/// [`MobileAnimCcBodyRecord::movement_actions`] rows: on foot, then riding a mount.
pub const MOVEMENT_ACTION_MOUNTINGS: usize = 2;
const MOUNTED_MOVEMENT_ACTIONS: usize = 1;
/// Serialized body record: body, frames body, file, flags, group type, mount height, `mobtypes.txt` flags, hue,
/// move action set, then the movement actions (u16 each).
pub const BODY_RECORD_SIZE: usize = 16 + MOVEMENT_ACTION_MOUNTINGS * MOVE_ACTION_MOVEMENT_COUNT * 2;
/// Serialized move action set: `MoveActionSet`, u16 each.
pub const MOVE_ACTION_SET_SIZE: usize = MOVE_ACTION_GROUP_COUNT * MOVE_ACTION_MOVEMENT_COUNT * 2;
/// Body table header: magic, version, body count, move action set count.
const BODY_TABLE_HEADER_SIZE: usize = 16;

/// Movement actions of a body's `AnimationSequence.uop` entry by group (`MOVE_ACTION_GROUPS`) and movement
/// (`MOVE_ACTION_MOVEMENTS`), as `FrameSetLookup_GetMoveAction` 0x00408114 gives them with the alternate variant off;
/// `NO_MOVE_ACTION` for a group the sequence does not have.
pub type MoveActionSet = [[u16; MOVE_ACTION_MOVEMENT_COUNT]; MOVE_ACTION_GROUP_COUNT];

/// What the Classic client draws for one server body, resolved by the packer from `Body.def`, `Bodyconv.def`,
/// `mobtypes.txt` and `AnimationSequence.uop` as the client routes them
/// (`udd_conv::animation::mobile_anim_cc::body_table`). The package has one per body id that reaches frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimCcBodyRecord {
    /// The server body id.
    pub body_id: u16,
    /// Body id of the animation records the client draws it with.
    pub frames_body_id: u16,
    /// `anim.mul` 0, `anim2.mul` 1, ...; 0 with `BODY_FLAG_UOP_FRAMES`.
    pub file_index: u8,
    /// `BODY_FLAG_*`.
    pub flags: u8,
    /// Action layout, `MOB_TYPE_GROUP_*`: the `mobtypes.txt` type of the body, or of its `Body.def` graphic.
    pub group_type: u8,
    /// The rider's height offset on this body as a mount: the body's own `Bodyconv.def` row's, else 0.
    pub mount_height: i8,
    /// `mobtypes.txt` flags of the row giving `group_type` (bit 31 set on a row), 0 without one.
    pub mob_type_flags: u32,
    /// The body's own `Body.def` hue, 0 without a row.
    pub hue: u16,
    /// Index of the sequence move actions of the action layout's body in the package's move action sets,
    /// `NO_MOVE_ACTION_SET` without them (a body without UOP frames or without a sequence entry).
    pub move_action_set: u16,
    /// The actions the client plays standing, walking and running (`MOVE_ACTION_MOVEMENTS`), on foot and riding
    /// (`MOVEMENT_ACTION_MOUNTINGS`), in peace mode with nothing in the hands and no flight.
    pub movement_actions: [[u16; MOVE_ACTION_MOVEMENT_COUNT]; MOVEMENT_ACTION_MOUNTINGS],
}

impl MobileAnimCcBodyRecord {
    /// Whether the client draws the body from `AnimationFrame*.uop`.
    pub fn uses_uop_frames(&self) -> bool {
        self.flags & BODY_FLAG_UOP_FRAMES != 0
    }

    /// See [`MobileAnimCcPackage::movement_action`].
    pub fn movement_action(&self, movement: u8, mounted: bool) -> Option<u16> {
        let movement = MOVE_ACTION_MOVEMENTS.iter().position(|&known| known == movement)?;
        let mounting = if mounted { MOUNTED_MOVEMENT_ACTIONS } else { 0 };
        Some(self.movement_actions[mounting][movement]).filter(|&action| action != NO_MOVE_ACTION)
    }
}

/// Stored direction (0..=4) and mirroring of world direction `direction` (0 north .. 7
/// north-west): world `FIRST_STORED_DIRECTION + k` is stored at `k`; world 0, 1 and 2 show world 6, 5 and 4
/// mirrored horizontally about the frame anchor (ClassicUO `GetAnimDirection`, and the client's stored direction
/// table at 0x007087dc, [3, 2, 1, 0, 1, 2, 3, 4], with its mirror table at 0x007087fc, [1, 1, 1, 0, 0, 0, 0, 0]:
/// `GetObjectLayerFrame` 0x005441e0, `GetMobileMulFrame` 0x0044e8e0, Classic 7.0.102.3; confirmed). `None` outside
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
    pixel_format: PagePixelFormat,
    pages: Vec<MobileAnimCcPageRecord>,
    animations: Vec<MobileAnimCcAnimationRecord>,
    frames: Vec<MobileAnimCcFrameRecord>,
    /// Sorted by body id.
    bodies: Vec<MobileAnimCcBodyRecord>,
    move_action_sets: Vec<MoveActionSet>,
    strides: Vec<MobileStrideRecord>,
    /// The 2x frames made ahead of time, if the package has them ([`crate::animation::mobile_upscaled`]).
    upscaled: Option<UpscaledFrameTable>,
    /// Indices of the animation records of each (body, action, direction).
    animation_lookup: HashMap<(u16, u16, u8), Vec<usize>>,
    page_cache: AtlasPageCache,
    /// The pages the stored clips are cut from ([`crate::animation::mobile_pages`]).
    stored_pages: StoredPageCache,
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
        let body_table = read_path_entry_cow(&package, BODY_TABLE_ENTRY_PATH).context(
            "mobile_anim_cc.uddp has no metadata/bodies.bin (the server body table): it predates the table; repack \
             mobile_anim_cc.uddp with the current udd-cli")?;

        let (page_width, page_height, page_gutter, page_pixel_format, page_packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let animations = parse_animation_manifest(&animation_manifest)?;
        let frames = parse_frame_manifest(&frame_manifest)?;
        let (bodies, move_action_sets) = parse_body_table(&body_table)?;
        let strides = read_stride_records(&package)?;
        let upscaled = UpscaledFrameTable::read(&package)?;
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
            pixel_format: page_pixel_format,
            pages,
            animations,
            frames,
            bodies,
            move_action_sets,
            strides,
            upscaled,
            animation_lookup,
            page_cache: AtlasPageCache::new(options),
            stored_pages: StoredPageCache::default(),
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

    /// The pixel format of every page, as the page manifest names it.
    pub fn pixel_format(&self) -> PagePixelFormat {
        self.pixel_format
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

    /// The server body table, sorted by body id.
    pub fn bodies(&self) -> &[MobileAnimCcBodyRecord] {
        &self.bodies
    }

    /// The sequence move action sets [`MobileAnimCcBodyRecord::move_action_set`] refers to.
    pub fn move_action_sets(&self) -> &[MoveActionSet] {
        &self.move_action_sets
    }

    /// What the client draws for server body `body_id`; `None` for a body that reaches no frames.
    pub fn body(&self, body_id: u16) -> Option<&MobileAnimCcBodyRecord> {
        self.bodies.binary_search_by_key(&body_id, |record| record.body_id).ok().map(|index| &self.bodies[index])
    }

    /// Action of the sequence of `body`'s action layout for `movement` (`MOVE_STAND`, `MOVE_WALK`, `MOVE_RUN`) in
    /// group `group` (`GROUP_WAR`, `GROUP_PEACE`, `GROUP_MOUNTED`); `None` when it has no such group or no sequence.
    pub fn body_move_action(&self, body: &MobileAnimCcBodyRecord, group: i8, movement: u8) -> Option<u16> {
        let set = self.move_action_sets.get(usize::from(body.move_action_set))?;
        crate::animation::move_actions::move_action(set, group, movement)
    }

    /// Whether `mobtypes.txt` flags body id `body_id` itself for UOP frames ([`BODY_FLAG_OWN_UOP_FRAMES`]).
    pub fn body_uses_uop_frames(&self, body_id: u16) -> bool {
        self.body(body_id).is_some_and(|record| record.flags & BODY_FLAG_OWN_UOP_FRAMES != 0)
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

    /// The animation the client draws for server body `body_id`, `action_id` and stored direction `direction`
    /// (see [`classic_stored_direction`]): the records of its [`MobileAnimCcBodyRecord::frames_body_id`], from its
    /// `anim*.mul` file on the MUL side. The other source fills in when the client's has none, as in
    /// [`Self::animation`].
    pub fn server_body_animation(
        &self,
        body_id: u16,
        action_id: u16,
        direction: u8,
    ) -> Option<&MobileAnimCcAnimationRecord> {
        let body = self.body(body_id)?;
        let matching = self.matching_animations(body.frames_body_id, action_id, direction);
        if body.uses_uop_frames() {
            return animation_from_source(matching, true);
        }
        animation_from_source(matching.filter(|animation| animation.file_index == body.file_index), false)
    }

    /// Action the Classic client plays for server body `body_id` standing, walking or running (`MOVE_STAND`,
    /// `MOVE_WALK`, `MOVE_RUN`) in peace mode, `mounted` when the body rides a mount (a mount's own body takes its
    /// layout's actions), with no server animation, nothing in the hands and no flight: the packer's resolution
    /// (`udd_conv::animation::mobile_anim_cc::body_table`, which documents the client's rules). `None` for another
    /// movement, or a body that reaches no frames.
    pub fn movement_action(&self, body_id: u16, movement: u8, mounted: bool) -> Option<u16> {
        self.body(body_id)?.movement_action(movement, mounted)
    }

    pub fn animation_frames(&self, animation: &MobileAnimCcAnimationRecord) -> &[MobileAnimCcFrameRecord] {
        let start = animation.frame_start as usize;
        let end = start + animation.frame_count as usize;
        self.frames.get(start..end).unwrap_or(&[])
    }

    /// The 2x frames made ahead of time, if the package has them.
    pub fn upscaled_frames(&self) -> Option<&UpscaledFrameTable> {
        self.upscaled.as_ref()
    }

    /// The 2x canvas of frame record `frame` as straight-alpha RGBA8, if the package has one.
    pub fn read_upscaled_frame_rgba(&self, frame: u32) -> eyre::Result<Option<Vec<u8>>> {
        let Some(table) = &self.upscaled else { return Ok(None) };
        let Some(record) = table.frame(frame) else { return Ok(None) };
        table.read_rgba(&self.package, record, &mut PageMemo::default()).map(Some)
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

    /// A frame's pixels as its page stores them, not decoded ([`StoredFramePixels`]).
    pub fn read_frame_stored(&self, frame: &MobileAnimCcFrameRecord) -> eyre::Result<StoredFramePixels> {
        self.read_frame_stored_in(frame, &mut ClipPages::default())
    }

    /// [`Self::read_frame_rgba`], the atlas page read through `pages`: the frames of a clip, which mostly share a
    /// page, read it once.
    pub fn read_frame_rgba_in(&self, frame: &MobileAnimCcFrameRecord, pages: &mut PageMemo) -> eyre::Result<Vec<u8>> {
        if frame.page_index == MISSING_PAGE_INDEX || frame.width == 0 || frame.height == 0 {
            return Ok(Vec::new());
        }
        let page = self.frame_page(frame)?;
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

    /// [`Self::read_frame_stored`], the atlas page read through `pages` and the package's page cache: a BC7 page
    /// stored in the field split joins only the frame's blocks.
    fn read_frame_stored_in(&self, frame: &MobileAnimCcFrameRecord, pages: &mut ClipPages)
        -> eyre::Result<StoredFramePixels> {
        if frame.page_index == MISSING_PAGE_INDEX || frame.width == 0 || frame.height == 0 {
            return Ok(match self.pixel_format {
                PagePixelFormat::Rgba8888 => StoredFramePixels::Rgba8(Vec::new()),
                PagePixelFormat::Bc7 => StoredFramePixels::Bc7Blocks(Vec::new()),
            });
        }
        let page = self.frame_page(frame)?;
        let layout = PageLayout { pixel_format: page.pixel_format, atlas_width: page.atlas_width,
            atlas_height: page.atlas_height, used_width: page.used_width, used_height: page.used_height };
        let stored = pages.page(&self.stored_pages, (PageSet::Frames, frame.page_index), || {
            StoredPage::read(&self.package, &page_entry_path(frame.page_index, page.pixel_format), layout)
        })?;
        stored.frame(layout, frame.x, frame.y, frame.width, frame.height)
    }

    /// The page record of `frame`'s page.
    fn frame_page(&self, frame: &MobileAnimCcFrameRecord) -> eyre::Result<&MobileAnimCcPageRecord> {
        self
            .pages
            .iter()
            .find(|page| page.page_index == frame.page_index)
            .ok_or_else(|| {
                eyre::eyre!(
                    "missing mobile animation atlas page metadata for {}",
                    frame.page_index
                )
            })
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
        let mut clips = BTreeSet::new();
        for body in &self.bodies {
            // A body with UOP frames has the animation sequence's war moves too.
            for movement in [MOVE_WALK, MOVE_RUN] {
                let war = self.body_move_action(body, GROUP_WAR, movement);
                for action in [body.movement_action(movement, false), war].into_iter().flatten() {
                    clips.insert((u32::from(body.body_id), action));
                }
            }
        }
        clips.into_iter().collect()
    }

    /// The client stores five directions and mirrors the other three ([`classic_stored_direction`]). A frame's anchor
    /// is its centre x from the left and its centre y from the bottom (ClassicUO: the frame's top at the anchor minus
    /// its height and centre y).
    fn clip(&self, body: u32, action: u16, direction: u8) -> eyre::Result<Option<MobileClip>> {
        let mut pages = PageMemo::default();
        self.clip_in(body, action, direction, |frame, anchor_x, anchor_y| Ok(MobileClipFrame {
            width: frame.width, height: frame.height, anchor_x, anchor_y,
            rgba: self.read_frame_rgba_in(frame, &mut pages)?,
        }))
    }

    fn side_clips(&self, body: u32, action: u16) -> eyre::Result<[Option<MobileClip>; STRIDE_FACING_COUNT]> {
        let mut pages = PageMemo::default();
        let mut clips: [Option<MobileClip>; STRIDE_FACING_COUNT] = Default::default();
        for (clip, direction) in clips.iter_mut().zip(STRIDE_FACINGS) {
            *clip = self.clip_in(body, action, direction, |frame, anchor_x, anchor_y| Ok(MobileClipFrame {
                width: frame.width, height: frame.height, anchor_x, anchor_y,
                rgba: self.read_frame_rgba_in(frame, &mut pages)?,
            }))?;
        }
        Ok(clips)
    }

    fn page_pixel_format(&self) -> PagePixelFormat {
        self.pixel_format
    }

    fn stored_clip(&self, body: u32, action: u16, direction: u8)
        -> eyre::Result<Option<MobileClip<MobileClipStoredFrame>>> {
        let mut pages = ClipPages::default();
        self.clip_in(body, action, direction, |frame, anchor_x, anchor_y| Ok(MobileClipStoredFrame {
            width: frame.width, height: frame.height, anchor_x, anchor_y,
            pixels: self.read_frame_stored_in(frame, &mut pages)?,
        }))
    }

    /// The records of the body the client takes the frames from ([`MobileAnimCcBodyRecord::frames_body_id`]), in any
    /// file and source; none for a body that reaches no frames.
    fn body_actions(&self, body: u32) -> Vec<u16> {
        let Some(frames_body) = u16::try_from(body).ok().and_then(|body| self.body(body))
            .map(|body| body.frames_body_id) else { return Vec::new() };
        let actions: BTreeSet<u16> = self.animations.iter().filter(|record| record.body_id == frames_body)
            .map(|record| record.action_id).collect();
        actions.into_iter().collect()
    }

    fn clip_extents(&self, body: u32, action: u16, direction: u8) -> Option<MobileClip<MobileClipFrameExtent>> {
        // The frames are read from the records alone, so this never fails.
        self.clip_in(body, action, direction, |frame, anchor_x, anchor_y| Ok(MobileClipFrameExtent {
            width: frame.width, height: frame.height, anchor_x, anchor_y,
        })).ok().flatten()
    }

    fn clip_sources(&self, body: u32, action: u16, direction: u8) -> Option<MobileClip<ClipFrameSource>> {
        let body = u16::try_from(body).ok()?;
        let (stored_direction, mirrored) = classic_stored_direction(direction)?;
        let animation = self.server_body_animation(body, action, stored_direction)?;
        let uop = animation.flags & MOBILE_ANIM_CC_FLAG_UOP_SOURCE != 0;
        // The frames are read from the records alone, so this never fails.
        self.record_clip_in(animation, mirrored, |frame, _, _| Ok(if uop {
            ClipFrameSource::ClassicUop { file_index: animation.file_index, body: animation.body_id,
                action: animation.action_id, direction: animation.direction, frame_index: frame.frame_index }
        } else {
            ClipFrameSource::ClassicMul { file_index: animation.file_index, source_index: animation.source_index,
                frame_index: frame.frame_index }
        })).ok().flatten()
    }

    fn upscaled_stored_clip(&self, body: u32, action: u16, direction: u8)
        -> eyre::Result<Option<MobileClip<MobileClipStoredFrame>>> {
        let Some(table) = &self.upscaled else { return Ok(None) };
        if !self.has_upscaled_clip(body, action, direction) {
            return Ok(None);
        }
        let mut pages = ClipPages::default();
        self.clip_in(body, action, direction, |frame, anchor_x, anchor_y| {
            let index = record_index(&self.frames, frame);
            let record = table.frame(index).ok_or_else(|| eyre::eyre!("frame record {index} has no 2x canvas"))?;
            Ok(MobileClipStoredFrame {
                width: record.width, height: record.height,
                anchor_x: UpscaledFrameTable::canvas_anchor(anchor_x),
                anchor_y: UpscaledFrameTable::canvas_anchor(anchor_y),
                pixels: table.read_stored(&self.package, record, &self.stored_pages, &mut pages)?,
            })
        })
    }

    fn has_upscaled_clip(&self, body: u32, action: u16, direction: u8) -> bool {
        let Some(table) = &self.upscaled else { return false };
        let mut all = true;
        let clip = self.clip_in(body, action, direction, |frame, _, _| {
            all &= table.frame(record_index(&self.frames, frame)).is_some();
            Ok(())
        });
        matches!(clip, Ok(Some(_))) && all
    }
}

impl MobileAnimCcPackage {
    /// [`MobileAnimClips::clip`], each frame made by `frame` from its record and its anchor.
    fn clip_in<F>(&self, body: u32, action: u16, direction: u8,
        frame: impl FnMut(&MobileAnimCcFrameRecord, i32, i32) -> eyre::Result<F>)
        -> eyre::Result<Option<MobileClip<F>>> {
        let Ok(body) = u16::try_from(body) else { return Ok(None) };
        let Some((stored_direction, mirrored)) = classic_stored_direction(direction) else { return Ok(None) };
        let Some(animation) = self.server_body_animation(body, action, stored_direction) else { return Ok(None) };
        self.record_clip_in(animation, mirrored, frame)
    }

    /// The clip of one animation record (a stored direction), drawn mirrored when `mirrored`.
    fn record_clip_in<F>(&self, animation: &MobileAnimCcAnimationRecord, mirrored: bool,
        mut frame: impl FnMut(&MobileAnimCcFrameRecord, i32, i32) -> eyre::Result<F>)
        -> eyre::Result<Option<MobileClip<F>>> {
        let frames = self.animation_frames(animation).iter().filter(|record| record.width > 0 && record.height > 0)
            .map(|record| frame(record, i32::from(record.center_x),
                i32::from(record.height) + i32::from(record.center_y)))
            .collect::<eyre::Result<Vec<_>>>()?;
        Ok((!frames.is_empty()).then_some(MobileClip { frames, mirrored, fps: animation.fps,
            stored: u32::from(animation.direction) }))
    }

    /// The clip of one animation record as [`MobileAnimClips::clip`] gives it, its stored direction unmirrored:
    /// what an inspector shows for the record. `None` without frames.
    pub fn animation_clip(&self, animation: &MobileAnimCcAnimationRecord) -> eyre::Result<Option<MobileClip>> {
        let mut pages = PageMemo::default();
        self.record_clip_in(animation, false, |frame, anchor_x, anchor_y| Ok(MobileClipFrame {
            width: frame.width, height: frame.height, anchor_x, anchor_y,
            rgba: self.read_frame_rgba_in(frame, &mut pages)?,
        }))
    }
}

#[allow(clippy::type_complexity)]
fn parse_page_manifest(bytes: &[u8])
    -> eyre::Result<(u32, u32, u16, PagePixelFormat, AtlasPackingMode, Vec<MobileAnimCcPageRecord>)> {
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
    Ok((width, height, gutter, pixel_format, packing_mode, pages))
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

/// The body table: the body records, sorted by unique body id, then the move action sets they refer to.
fn parse_body_table(bytes: &[u8]) -> eyre::Result<(Vec<MobileAnimCcBodyRecord>, Vec<MoveActionSet>)> {
    let path = BODY_TABLE_ENTRY_PATH;
    eyre::ensure!(bytes.len() >= BODY_TABLE_HEADER_SIZE && bytes.starts_with(&BODY_TABLE_MAGIC),
        "{path}: no MABD header; repack mobile_anim_cc.uddp with the current udd-cli");
    let mut cursor = Cursor::new(&bytes[BODY_TABLE_MAGIC.len()..]);
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("mobile animation body table", version, MOBILE_ANIM_CC_METADATA_VERSION)?;
    let body_count = cursor.read_u32::<LittleEndian>()? as usize;
    let set_count = cursor.read_u32::<LittleEndian>()? as usize;
    let expected = body_count.checked_mul(BODY_RECORD_SIZE)
        .zip(set_count.checked_mul(MOVE_ACTION_SET_SIZE))
        .and_then(|(bodies, sets)| bodies.checked_add(sets)?.checked_add(BODY_TABLE_HEADER_SIZE));
    eyre::ensure!(expected == Some(bytes.len()), "{path}: {} bytes for {body_count} bodies and {set_count} move \
        action sets", bytes.len());
    let mut bodies = Vec::with_capacity(body_count);
    for _ in 0..body_count {
        bodies.push(MobileAnimCcBodyRecord {
            body_id: cursor.read_u16::<LittleEndian>()?,
            frames_body_id: cursor.read_u16::<LittleEndian>()?,
            file_index: cursor.read_u8()?,
            flags: cursor.read_u8()?,
            group_type: cursor.read_u8()?,
            mount_height: cursor.read_i8()?,
            mob_type_flags: cursor.read_u32::<LittleEndian>()?,
            hue: cursor.read_u16::<LittleEndian>()?,
            move_action_set: cursor.read_u16::<LittleEndian>()?,
            movement_actions: {
                let mut actions = [[NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT]; MOVEMENT_ACTION_MOUNTINGS];
                for action in actions.iter_mut().flatten() {
                    *action = cursor.read_u16::<LittleEndian>()?;
                }
                actions
            },
        });
    }
    let mut move_action_sets = Vec::with_capacity(set_count);
    for _ in 0..set_count {
        let mut set = [[NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT]; MOVE_ACTION_GROUP_COUNT];
        for action in set.iter_mut().flatten() {
            *action = cursor.read_u16::<LittleEndian>()?;
        }
        move_action_sets.push(set);
    }
    eyre::ensure!(bodies.windows(2).all(|pair| pair[0].body_id < pair[1].body_id),
        "{path}: the bodies are not sorted by unique id");
    if let Some(body) = bodies.iter().find(|body| body.move_action_set != NO_MOVE_ACTION_SET
        && usize::from(body.move_action_set) >= set_count) {
        eyre::bail!("{path}: body {} refers to move action set {} of {set_count}", body.body_id,
            body.move_action_set);
    }
    Ok((bodies, move_action_sets))
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
        let sequence_actions = [[NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT], [4, 0, 2], [25, 23, 24]];
        let body_table = body_table_bytes(&[body(2, 2, 0, 0), MobileAnimCcBodyRecord {
            body_id: 7,
            frames_body_id: 8,
            file_index: 2,
            flags: 0,
            group_type: MOB_TYPE_GROUP_HUMAN,
            mount_height: -1,
            mob_type_flags: 0x8000_0001,
            hue: 9,
            move_action_set: 0,
            movement_actions: [[4, 0, 2], [25, 23, 24]],
        }], &[sequence_actions]);
        let page = vec![255u8; 4 * 4 * 4];
        let page_path = page_entry_path(0, PagePixelFormat::Rgba8888);
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (path, data, data_type) in [
            (PAGE_MANIFEST_ENTRY_PATH, page_manifest.as_slice(), DataType::Metadata),
            (ANIMATION_MANIFEST_ENTRY_PATH, animation_manifest.as_slice(), DataType::Metadata),
            (FRAME_MANIFEST_ENTRY_PATH, frame_manifest.as_slice(), DataType::Metadata),
            (BODY_TABLE_ENTRY_PATH, body_table.as_slice(), DataType::Metadata),
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
        let body = package.body(7).unwrap();
        assert_eq!((body.frames_body_id, body.file_index, body.hue, body.mount_height), (8, 2, 9, -1));
        assert_eq!(package.body_move_action(body, GROUP_PEACE, MOVE_WALK), Some(0));
        assert_eq!(package.body_move_action(body, GROUP_WAR, MOVE_WALK), None);
        assert_eq!(package.movement_action(7, MOVE_RUN, true), Some(24));
        assert!(package.body(8).is_none());
        assert!(!package.body_uses_uop_frames(7));
        assert_eq!(package.read_page_bytes(0).unwrap().len(), page.len());
        let animation = package.animation(2, 3, 4).unwrap();
        let frame = &package.animation_frames(animation)[0];
        assert_eq!(
            package.read_frame_rgba(frame).unwrap().len(),
            2 * 2 * 4
        );
        assert_eq!(package.pixel_format(), PagePixelFormat::Rgba8888);
        assert_eq!(MobileAnimClips::page_pixel_format(&package), PagePixelFormat::Rgba8888);
        // The facing whose stored direction is the animation's.
        let direction = (0..8).find(|&direction| classic_stored_direction(direction) == Some((4, false))).unwrap();
        let clip = package.clip(2, 3, direction).unwrap().unwrap();
        let stored = package.stored_clip(2, 3, direction).unwrap().unwrap();
        assert_eq!(stored.frames.len(), 1);
        assert_eq!((stored.frames[0].anchor_x, stored.frames[0].anchor_y), (clip.frames[0].anchor_x,
            clip.frames[0].anchor_y));
        assert_eq!(stored.frames[0].pixels, StoredFramePixels::Rgba8(clip.frames[0].rgba.clone()));
    }

    fn manifest(magic: [u8; 4], count: usize) -> Vec<u8> {
        let mut bytes = magic.to_vec();
        bytes.extend_from_slice(&MOBILE_ANIM_CC_METADATA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(count as u32).to_le_bytes());
        bytes
    }

    /// The body table of `bodies` (sorted by body as the packer writes them) and `sets`.
    fn body_table_bytes(bodies: &[MobileAnimCcBodyRecord], sets: &[MoveActionSet]) -> Vec<u8> {
        let mut bytes = manifest(BODY_TABLE_MAGIC, bodies.len());
        bytes.extend_from_slice(&(sets.len() as u32).to_le_bytes());
        for body in bodies {
            bytes.extend_from_slice(&body.body_id.to_le_bytes());
            bytes.extend_from_slice(&body.frames_body_id.to_le_bytes());
            bytes.extend_from_slice(&[body.file_index, body.flags, body.group_type, body.mount_height as u8]);
            bytes.extend_from_slice(&body.mob_type_flags.to_le_bytes());
            bytes.extend_from_slice(&body.hue.to_le_bytes());
            bytes.extend_from_slice(&body.move_action_set.to_le_bytes());
            for action in body.movement_actions.iter().flatten() {
                bytes.extend_from_slice(&action.to_le_bytes());
            }
        }
        for action in sets.iter().flatten().flatten() {
            bytes.extend_from_slice(&action.to_le_bytes());
        }
        bytes
    }

    /// A package of `animations` (no frames), with no pages, and `body_table` when given.
    fn package_with(animations: &[MobileAnimCcAnimationRecord], body_table: Option<&[u8]>)
        -> eyre::Result<MobileAnimCcPackage> {
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
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        let entries = [
            (PAGE_MANIFEST_ENTRY_PATH, pages.as_slice()),
            (ANIMATION_MANIFEST_ENTRY_PATH, animation_bytes.as_slice()),
            (FRAME_MANIFEST_ENTRY_PATH, frames.as_slice()),
        ];
        for (path, data) in entries.into_iter().chain(body_table.map(|table| (BODY_TABLE_ENTRY_PATH, table))) {
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
        MobileAnimCcPackage::from_uddp_package(UddpReader::open(builder.build().unwrap()).unwrap())
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

    fn body(body_id: u16, frames_body_id: u16, file_index: u8, flags: u8) -> MobileAnimCcBodyRecord {
        MobileAnimCcBodyRecord {
            body_id,
            frames_body_id,
            file_index,
            flags,
            group_type: MOB_TYPE_GROUP_MONSTER,
            mount_height: 0,
            mob_type_flags: 0,
            hue: 0,
            move_action_set: NO_MOVE_ACTION_SET,
            movement_actions: [[1, 0, 0]; MOVEMENT_ACTION_MOUNTINGS],
        }
    }

    #[test]
    fn server_bodies_take_the_records_of_their_frames_body_and_file() {
        const ANIM4: u8 = 3;
        let (bodyconv_body, uop_body, frames_only_body) = (250, 700, 17);
        let animations = [
            animation(frames_only_body, 0, ANIM4, 0),
            animation(frames_only_body, 0, 0, 0),
            animation(uop_body, 0, ANIM4, MOBILE_ANIM_CC_FLAG_UOP_SOURCE),
        ];
        let table = body_table_bytes(&[
            body(frames_only_body, frames_only_body, 0, 0),
            body(bodyconv_body, frames_only_body, ANIM4, 0),
            body(uop_body, uop_body, 0, BODY_FLAG_UOP_FRAMES | BODY_FLAG_OWN_UOP_FRAMES),
        ], &[]);
        let package = package_with(&animations, Some(&table)).unwrap();

        // The id 17 also exists in anim.mul: the lookup takes the record of the body's file.
        assert_eq!(package.server_body_animation(bodyconv_body, 0, 0).unwrap().file_index, ANIM4);
        assert_eq!(package.server_body_animation(frames_only_body, 0, 0).unwrap().file_index, 0);
        assert!(package.server_body_animation(uop_body, 0, 0).is_some());
        assert!(package.server_body_animation(uop_body, 1, 0).is_none());
        assert!(package.body_uses_uop_frames(uop_body));
        // A body without a record reaches no frames.
        assert!(package.server_body_animation(bodyconv_body + 1, 0, 0).is_none());
        assert_eq!(package.movement_action(bodyconv_body + 1, MOVE_WALK, false), None);
        assert_eq!(package.body_actions(u32::from(bodyconv_body)), [0]);
        assert!(package.body_actions(u32::from(bodyconv_body + 1)).is_empty());
    }

    #[test]
    fn stride_clips_take_the_walk_and_run_and_the_sequence_war_moves() {
        let (uop_body, mul_body) = (714, 1);
        let war = [1, 20, 21];
        let table = body_table_bytes(&[
            body(mul_body, mul_body, 0, 0),
            MobileAnimCcBodyRecord { move_action_set: 0, movement_actions: [[25, 22, 24], [25, 22, 24]],
                ..body(uop_body, uop_body, 0, BODY_FLAG_UOP_FRAMES) },
        ], &[[war, [NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT], [NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT]]]);
        let package = package_with(&[], Some(&table)).unwrap();

        assert_eq!(package.movement_action(uop_body, MOVE_WALK, true), Some(22));
        let unknown_movement = 3;
        assert_eq!(package.movement_action(uop_body, unknown_movement, true), None);
        assert_eq!(package.stride_clips(), [(1, 0), (714, 20), (714, 21), (714, 22), (714, 24)]);
    }

    #[test]
    fn a_package_without_the_body_table_asks_for_a_repack() {
        let error = package_with(&[], None).err().expect("no body table");
        assert!(format!("{error:#}").contains("repack mobile_anim_cc.uddp"), "{error:#}");
        let error = package_with(&[], Some(b"MABR")).err().expect("another table");
        assert!(format!("{error:#}").contains("repack mobile_anim_cc.uddp"), "{error:#}");
        let unknown_set = body_table_bytes(&[MobileAnimCcBodyRecord { move_action_set: 0, ..body(1, 1, 0, 0) }], &[]);
        assert!(package_with(&[], Some(&unknown_set)).is_err());
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
