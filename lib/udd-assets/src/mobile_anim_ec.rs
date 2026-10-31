use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};
use knuffel::Decode;
use udd_container::UddpReader;

use crate::common::{
    decode_atlas_page_rgba, ensure_manifest_version, extract_atlas_subrect_rgba, read_path_entry, read_path_entry_cow,
    AtlasCacheOptions, AtlasPageCache,
};
use crate::tex_art_cc::{AtlasPackingMode, PagePixelFormat};
pub use uocf::enhanced::hues::ShadowPaletteStats;

pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const ANIMATION_MANIFEST_ENTRY_PATH: &str = "metadata/animations.bin";
pub const FRAME_MANIFEST_ENTRY_PATH: &str = "metadata/frames.bin";
pub const ITEM_MANIFEST_ENTRY_PATH: &str = "metadata/items.bin";
pub const SOURCE_HINT_MANIFEST_ENTRY_PATH: &str = "metadata/source_hints.bin";
pub const BODY_MANIFEST_ENTRY_PATH: &str = "metadata/bodies.bin";

pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_FRAME_INDEX: u16 = u16::MAX;
/// `MobileAnimEcFrameRecord::source_frame_index` of a placeholder for a frame number missing
/// from the source `AnimationFrame` entry (a gap); the record has no pixels.
pub const MISSING_SOURCE_FRAME_INDEX: u16 = u16::MAX;
/// `MobileAnimEcAnimationRecord::alias_action_id` of an action with its own frames.
pub const NO_ALIAS_ACTION: u16 = u16::MAX;
/// Number of frames `MobileAnimEcAnimationRecord::skipped_frame_mask` can describe.
pub const SKIPPED_FRAME_MASK_BITS: u32 = u32::BITS;

/// The record shows the frames of direction `MIRROR_DIRECTION_SUM - direction` flipped
/// horizontally about the frame anchor (the flipped anchor x is `width - center_x`).
pub const ANIMATION_FLAG_MIRRORED: u16 = 1 << 0;
/// The body has no `AnimationSequence` entry: actions, directions and fps are derived from the
/// `AnimationFrame` entries (see the crate README). The client cannot animate these bodies.
pub const ANIMATION_FLAG_NO_SEQUENCE: u16 = 1 << 1;
/// No-sequence action whose frame numbers do not split into directions: every direction record
/// shows all frames of the action, in frame number order, unmirrored.
pub const ANIMATION_FLAG_UNDIRECTED: u16 = 1 << 2;
/// The resolved action is the stand action of a sequence group (`IsStandAction`): the client
/// shows only its first frame when the `idleAnimation` setting is off.
pub const ANIMATION_FLAG_STAND_ACTION: u16 = 1 << 3;

/// `MobileAnimEcFrameRecord::hue_mode`: no pixel of the frame takes the hue.
pub const FRAME_HUE_NONE: u8 = 0;
/// Every coded pixel (alpha > 0) of the frame takes the hue.
pub const FRAME_HUE_ALL: u8 = 1;
/// Some pixels take the hue: the hue mask page of the frame's page marks them.
pub const FRAME_HUE_MASKED: u8 = 2;

/// `MobileAnimEcPageRecord::flags`: the page has a hue mask page (`hue_mask_entry_path`).
pub const PAGE_FLAG_HUE_MASK: u8 = 1 << 0;
/// Hue mask byte of a pixel that takes the hue (`uocf` `HUE_MASK_HUEABLE`); other pixels are 0.
pub const HUE_MASK_HUEABLE: u8 = uocf::enhanced::animationframe::HUE_MASK_HUEABLE;

/// `MobileAnimEcBodyRecord::flags`: the body is a ghost and is drawn with `forced_hue`.
pub const BODY_FLAG_GHOST: u16 = 1 << 0;
/// The appearance has no frames; the client falls back to the frames of body 0.
pub const BODY_FLAG_FRAMES_FALLBACK: u16 = 1 << 1;
/// No `tileart.uop` creature placeholder: the body uses its own sequence and frames (the client
/// cannot show it).
pub const BODY_FLAG_NO_PLACEHOLDER: u16 = 1 << 2;
/// The sequence has no `AnimationSequence` entry: the records carry `ANIMATION_FLAG_NO_SEQUENCE`.
pub const BODY_FLAG_NO_SEQUENCE: u16 = 1 << 3;
/// The placeholder has a race (props1 key 9): `MobileAnimEcBodyRecord::race` is meaningful.
pub const BODY_FLAG_HAS_RACE: u16 = 1 << 4;
/// The placeholder has a gender (props1 key 10): `MobileAnimEcBodyRecord::gender` is meaningful.
pub const BODY_FLAG_HAS_GENDER: u16 = 1 << 5;
/// `MobileAnimEcBodyRecord::forced_hue` of a body drawn with its own hue.
pub const NO_FORCED_HUE: u16 = 0;

/// Directions of a mobile, in UO order (0 north .. 7 north-west); each has a record.
pub const DIRECTION_COUNT: u8 = uocf::animation_sequence::DIRECTION_COUNT as u8;
/// Directions below this one are mirrored (`ANIMATION_FLAG_MIRRORED`).
pub const FIRST_STORED_DIRECTION: u8 = uocf::animation_sequence::FIRST_STORED_DIRECTION;
/// A mirrored direction `d` shows direction `MIRROR_DIRECTION_SUM - d` flipped horizontally.
pub const MIRROR_DIRECTION_SUM: u8 = uocf::animation_sequence::MIRROR_DIRECTION_SUM;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"MEPG";
const ANIMATION_MANIFEST_MAGIC: [u8; 4] = *b"MEAN";
const FRAME_MANIFEST_MAGIC: [u8; 4] = *b"MEFR";
const ITEM_MANIFEST_MAGIC: [u8; 4] = *b"MEIT";
const SOURCE_HINT_MANIFEST_MAGIC: [u8; 4] = *b"MESH";
const BODY_MANIFEST_MAGIC: [u8; 4] = *b"MEBD";
/// Metadata version written by the converter, in every manifest.
pub const MOBILE_ANIM_EC_METADATA_VERSION: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimEcPageRecord {
    pub page_index: u32,
    pub frame_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub used_width: u32,
    pub used_height: u32,
    pub pixel_format: PagePixelFormat,
    /// `PAGE_FLAG_*`.
    pub flags: u8,
}

impl MobileAnimEcPageRecord {
    pub fn has_hue_mask(&self) -> bool {
        self.flags & PAGE_FLAG_HUE_MASK != 0
    }
}

/// How the client draws one server body, from its `tileart.uop` creature placeholder
/// `10000000 + body_id`. Records are sorted by `body_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimEcBodyRecord {
    pub body_id: u32,
    /// `AnimationSequence` entry giving actions, directions and fps (the placeholder body type).
    pub sequence_id: u32,
    /// `AnimationFrame` body holding the pixels (`MobileAnimEcAnimationRecord::source_body_id`).
    pub frames_body_id: u32,
    /// Frame body the placeholder asks for (props1 key 8 appearance); differs from
    /// `frames_body_id` with `BODY_FLAG_FRAMES_FALLBACK`.
    pub appearance_body_id: u32,
    /// Hue the client draws the body with regardless of the mobile hue (`GHOST_HUE` for
    /// ghosts); `NO_FORCED_HUE` otherwise.
    pub forced_hue: u16,
    /// `BODY_FLAG_*`.
    pub flags: u16,
    /// Id stored inside the `AnimationSequence` entry `sequence_id` (sequence +0x10); equals
    /// `sequence_id` except for a few entries (146 -> 704, 1308 -> 29, 1309 -> 1254) and for
    /// `BODY_FLAG_NO_SEQUENCE`. The client draws an equipment item on the body only when the item
    /// appearance's body type equals it (`BuildLayers`, "Not letting a %u(sq %u) wear a %u(sq %u)").
    pub sequence_stored_id: u32,
    /// props1 key 9 with `BODY_FLAG_HAS_RACE` (see [`Self::race`]).
    pub race: u8,
    /// props1 key 10 with `BODY_FLAG_HAS_GENDER` (see [`Self::gender`]).
    pub gender: u8,
}

impl MobileAnimEcBodyRecord {
    pub fn is_ghost(&self) -> bool {
        self.flags & BODY_FLAG_GHOST != 0
    }

    /// Race of the placeholder (props1 key 9; the client's `GetRace` gives
    /// `uocf::enhanced::mobile_body::DEFAULT_RACE` without it). Read by the client for the
    /// equipment layer order (`FUN_00468810`).
    pub fn race(&self) -> Option<u8> {
        (self.flags & BODY_FLAG_HAS_RACE != 0).then_some(self.race)
    }

    /// Gender of the placeholder (props1 key 10, 0 male, 1 female; the client's `GetGender`
    /// gives `uocf::enhanced::mobile_body::DEFAULT_GENDER` without it). Read by the client for
    /// equipment appearance filters by owner gender and the equipment layer order.
    pub fn gender(&self) -> Option<u8> {
        (self.flags & BODY_FLAG_HAS_GENDER != 0).then_some(self.gender)
    }
}

/// What the client shows for "body, action, direction" (one record per sequence action and
/// direction 0..`DIRECTION_COUNT`). Records are sorted by `(body_id, action_id, direction)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobileAnimEcAnimationRecord {
    /// Server body (see `MobileAnimEcBodyRecord`).
    pub body_id: u32,
    /// Requested action.
    pub action_id: u16,
    pub direction: u8,
    /// Frames shown, `frame_start..frame_start + frame_count` in the frame records. Mirrored and
    /// alias records share the frames of the record they show. For sequence actions every
    /// direction has exactly the sequence frame count of frames (placeholders fill gaps), so the
    /// position is the frame index within the direction; 0 frames when the action has no
    /// `AnimationFrame` entry.
    pub frame_start: u32,
    pub frame_count: u16,
    /// `ANIMATION_FLAG_*`.
    pub flags: u16,
    /// `AnimationFrame` body holding the frames: the tileart appearance of the body (401 -> 401,
    /// 402 -> 400, 146 -> 124), body 0 when it has no frames. Bodies sharing it share the pixels.
    pub source_body_id: u32,
    /// Action whose frames are shown (`ResolveAction` with flying animations enabled).
    pub resolved_action_id: u16,
    /// Alias stored in the sequence for `action_id`; `NO_ALIAS_ACTION` for an action with its
    /// own frames.
    pub alias_action_id: u16,
    /// `ResolveAction` with flying animations disabled; the frames are those of the record of
    /// this action and the same direction.
    pub resolved_action_id_no_flying: u16,
    /// Frames per second (`GetActionFps`) with flying animations enabled.
    pub fps: f32,
    /// Frames per second with flying animations disabled.
    pub fps_no_flying: f32,
    /// Bit `i` set: the player passes over frame `i` of the direction (the sequence skipped
    /// frames of `action_id`).
    pub skipped_frame_mask: u32,
}

impl MobileAnimEcAnimationRecord {
    pub fn is_mirrored(&self) -> bool {
        self.flags & ANIMATION_FLAG_MIRRORED != 0
    }

    pub fn is_alias(&self) -> bool {
        self.alias_action_id != NO_ALIAS_ACTION
    }

    pub fn is_frame_skipped(&self, frame_index: u32) -> bool {
        frame_index < SKIPPED_FRAME_MASK_BITS && self.skipped_frame_mask & (1 << frame_index) != 0
    }
}

/// An animation record chosen by [`MobileAnimEcPackage::resolve_animation`] and its fps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedMobileAnimEcAnimation<'a> {
    /// Record whose frames, mirroring and skipped frames are shown.
    pub record: &'a MobileAnimEcAnimationRecord,
    pub fps: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimEcFrameRecord {
    /// Record that owns the frame: the stored direction of the plain action.
    pub animation_index: u32,
    /// Position within the owner's frames.
    pub frame_index: u16,
    /// Position of the frame among all `AnimationFrame` entries of the source body (atlas key);
    /// `MISSING_SOURCE_FRAME_INDEX` for a placeholder.
    pub source_frame_index: u16,
    /// `FrameEntry` action of the source frame.
    pub source_action: u16,
    /// `FrameEntry` 1-based frame number of the source frame (the expected number for a
    /// placeholder).
    pub source_frame_number: u16,
    pub page_index: u32,
    pub page_frame_index: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// Anchor inside the frame image: `(-x0, -y0)` of the `AnimationFrame` frame box (minus the
    /// transparent trim, times the upscale). The pixels are straight-alpha RGBA: opaque solid
    /// pixels, edge pixels with coverage alpha.
    pub center_x: i16,
    pub center_y: i16,
    /// `FRAME_HUE_*`: which pixels take the mobile hue. The hue table key of a hueable pixel is
    /// its red byte.
    pub hue_mode: u8,
    /// Palette statistics of the source `AnimationFrame` file (source body and `source_action`)
    /// that the shadow hue `uocf::enhanced::hues::SHADOW_HUE` normalizes against; the default for
    /// a placeholder frame without a source file.
    pub shadow: ShadowPaletteStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileAnimEcItemRecord {
    pub item_id: i32,
    pub item_type: i16,
    pub layer: i16,
    pub flags: u8,
    pub name: String,
    pub source_hint_start: u32,
    pub source_hint_count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimEcSourceHintRecord {
    pub item_id: i32,
    pub action_id: u16,
    pub uop_index: u8,
    pub block_index: u32,
    pub file_index: u32,
}

#[derive(Decode, Debug, Clone)]
pub struct EcMobileAnimationsKdl {
    #[knuffel(children(name = "item"))]
    pub items: Vec<EcMobileAnimationItemKdl>,
}

impl EcMobileAnimationsKdl {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .wrap_err_with(|| format!("Failed to read KDL file: {:?}", path.as_ref()))?;
        Self::parse(
            path.as_ref().to_str().unwrap_or("EcMobileAnimations.kdl"),
            &content,
        )
    }

    pub fn parse(name: &str, content: &str) -> eyre::Result<Self> {
        udd_kdl::strict::decode(name, content, EC_MOBILE_ANIMATIONS_SCHEMA)
    }
}

/// The nodes and properties the decoders below accept, for error messages that name them.
const EC_MOBILE_ANIMATIONS_SCHEMA: &[udd_kdl::strict::NodeSchema] = &[udd_kdl::strict::NodeSchema {
    name: "item",
    properties: &["name", "type", "layer", "male", "female", "gargoyle"],
    children: &[udd_kdl::strict::NodeSchema::leaf("anim", &["uop", "block", "file"])],
}];

#[derive(Decode, Debug, Clone)]
pub struct EcMobileAnimationItemKdl {
    #[knuffel(argument)]
    pub id: i32,
    #[knuffel(property)]
    pub name: String,
    #[knuffel(property(name = "type"))]
    pub item_type: i16,
    #[knuffel(property)]
    pub layer: i16,
    #[knuffel(property)]
    pub male: Option<bool>,
    #[knuffel(property)]
    pub female: Option<bool>,
    #[knuffel(property)]
    pub gargoyle: Option<bool>,
    #[knuffel(children(name = "anim"))]
    pub animations: Vec<EcMobileAnimationSourceHintKdl>,
}

impl EcMobileAnimationItemKdl {
    pub fn flags(&self) -> u8 {
        u8::from(self.male.unwrap_or(false))
            | (u8::from(self.female.unwrap_or(false)) << 1)
            | (u8::from(self.gargoyle.unwrap_or(false)) << 2)
    }
}

#[derive(Decode, Debug, Clone)]
pub struct EcMobileAnimationSourceHintKdl {
    #[knuffel(argument)]
    pub action_id: u16,
    #[knuffel(property)]
    pub uop: String,
    #[knuffel(property)]
    pub block: u32,
    #[knuffel(property)]
    pub file: u32,
}

pub struct MobileAnimEcPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    packing_mode: AtlasPackingMode,
    pages: Vec<MobileAnimEcPageRecord>,
    bodies: Vec<MobileAnimEcBodyRecord>,
    animations: Vec<MobileAnimEcAnimationRecord>,
    animation_lookup: HashMap<(u32, u16, u8), usize>,
    lowest_plain_actions: HashMap<u32, u16>,
    frames: Vec<MobileAnimEcFrameRecord>,
    items: Vec<MobileAnimEcItemRecord>,
    source_hints: Vec<MobileAnimEcSourceHintRecord>,
    page_cache: AtlasPageCache,
}

impl MobileAnimEcPackage {
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
            .context("mobile_anim_ec.uddp missing metadata/pages.bin")?;
        let animation_manifest = read_path_entry_cow(&package, ANIMATION_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/animations.bin")?;
        let frame_manifest = read_path_entry_cow(&package, FRAME_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/frames.bin")?;
        let item_manifest = read_path_entry_cow(&package, ITEM_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/items.bin")?;
        let source_hint_manifest = read_path_entry_cow(&package, SOURCE_HINT_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/source_hints.bin")?;
        let body_manifest = read_path_entry_cow(&package, BODY_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/bodies.bin; rebuild mobile_anim_ec.uddp")?;

        let (atlas_width, atlas_height, gutter, packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let animations = parse_animation_manifest(&animation_manifest)?;
        let frames = parse_frame_manifest(&frame_manifest)?;
        let items = parse_item_manifest(&item_manifest)?;
        let source_hints = parse_source_hint_manifest(&source_hint_manifest)?;
        let bodies = parse_body_manifest(&body_manifest)?;
        let mut animation_lookup = HashMap::with_capacity(animations.len());
        let mut lowest_plain_actions = HashMap::<u32, u16>::new();
        for (index, animation) in animations.iter().enumerate() {
            animation_lookup.insert((animation.body_id, animation.action_id, animation.direction), index);
            if !animation.is_alias() {
                let lowest = lowest_plain_actions.entry(animation.body_id).or_insert(animation.action_id);
                *lowest = (*lowest).min(animation.action_id);
            }
        }

        Ok(Self {
            package,
            atlas_width,
            atlas_height,
            gutter,
            packing_mode,
            pages,
            bodies,
            animations,
            animation_lookup,
            lowest_plain_actions,
            frames,
            items,
            source_hints,
            page_cache: AtlasPageCache::new(options),
        })
    }

    pub fn atlas_width(&self) -> u32 { self.atlas_width }
    pub fn atlas_height(&self) -> u32 { self.atlas_height }
    pub fn gutter(&self) -> u16 { self.gutter }
    pub fn packing_mode(&self) -> AtlasPackingMode { self.packing_mode }
    pub fn pages(&self) -> &[MobileAnimEcPageRecord] { &self.pages }
    pub fn bodies(&self) -> &[MobileAnimEcBodyRecord] { &self.bodies }
    pub fn animations(&self) -> &[MobileAnimEcAnimationRecord] { &self.animations }
    pub fn frames(&self) -> &[MobileAnimEcFrameRecord] { &self.frames }
    pub fn items(&self) -> &[MobileAnimEcItemRecord] { &self.items }
    pub fn source_hints(&self) -> &[MobileAnimEcSourceHintRecord] { &self.source_hints }

    /// Body mapping of a server body.
    pub fn body(&self, body_id: u32) -> Option<&MobileAnimEcBodyRecord> {
        self.bodies
            .binary_search_by_key(&body_id, |body| body.body_id)
            .ok()
            .map(|index| &self.bodies[index])
    }

    /// Record of exactly (body, action, direction); `None` when the sequence has no such action.
    pub fn animation(&self, body_id: u32, action_id: u16, direction: u8) -> Option<&MobileAnimEcAnimationRecord> {
        self.animation_lookup
            .get(&(body_id, action_id, direction))
            .map(|&index| &self.animations[index])
    }

    /// What the client shows for (body, action, direction) under the `flying_animations`
    /// setting: an action the sequence does not have falls back to the lowest plain action of
    /// the body (`ResolveAction`); with flying disabled, the frames of
    /// `resolved_action_id_no_flying` and `fps_no_flying` are used.
    pub fn resolve_animation(
        &self,
        body_id: u32,
        action_id: u16,
        direction: u8,
        flying_enabled: bool,
    ) -> Option<ResolvedMobileAnimEcAnimation<'_>> {
        let requested = self.animation(body_id, action_id, direction).or_else(|| {
            let lowest = *self.lowest_plain_actions.get(&body_id)?;
            self.animation(body_id, lowest, direction)
        })?;
        if flying_enabled {
            return Some(ResolvedMobileAnimEcAnimation { record: requested, fps: requested.fps });
        }
        let record = self.animation(body_id, requested.resolved_action_id_no_flying, direction)?;
        Some(ResolvedMobileAnimEcAnimation { record, fps: requested.fps_no_flying })
    }

    pub fn animation_frames(&self, animation: &MobileAnimEcAnimationRecord) -> &[MobileAnimEcFrameRecord] {
        let start = animation.frame_start as usize;
        let end = start + animation.frame_count as usize;
        self.frames.get(start..end).unwrap_or(&[])
    }

    pub fn item(&self, item_id: i32) -> Option<&MobileAnimEcItemRecord> {
        self.items.iter().find(|item| item.item_id == item_id)
    }

    pub fn item_source_hints(&self, item: &MobileAnimEcItemRecord) -> &[MobileAnimEcSourceHintRecord] {
        let start = item.source_hint_start as usize;
        let end = start + item.source_hint_count as usize;
        self.source_hints.get(start..end).unwrap_or(&[])
    }

    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .iter()
            .find(|page| page.page_index == page_index)
            .map(|page| page.pixel_format)
            .unwrap_or(PagePixelFormat::Rgba8888);
        self.page_cache.read_page_bytes(page_index, || {
            read_path_entry(&self.package, &page_entry_path(page_index, fmt))
                .wrap_err_with(|| format!("unpack EC mobile animation atlas page {page_index}"))
        })
    }

    pub fn read_page_rgba(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let page = self
            .pages
            .iter()
            .find(|page| page.page_index == page_index)
            .ok_or_else(|| eyre::eyre!("missing EC mobile animation atlas page metadata for {page_index}"))?;
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

    pub fn read_frame_rgba(&self, frame: &MobileAnimEcFrameRecord) -> eyre::Result<Vec<u8>> {
        if frame.page_index == MISSING_PAGE_INDEX || frame.width == 0 || frame.height == 0 {
            return Ok(Vec::new());
        }
        let page = self
            .pages
            .iter()
            .find(|page| page.page_index == frame.page_index)
            .ok_or_else(|| {
                eyre::eyre!(
                    "missing EC mobile animation atlas page metadata for {}",
                    frame.page_index
                )
            })?;
        let page_bytes = self.read_page_bytes(frame.page_index)?;
        extract_atlas_subrect_rgba(
            &page_bytes,
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

impl MobileAnimEcPackage {
    /// R8 hue mask of a page (`used_width * used_height` bytes), `None` when the page has none.
    /// Only the rects of `FRAME_HUE_MASKED` frames (and their gutters) are meaningful.
    pub fn read_page_hue_mask(&self, page_index: u32) -> eyre::Result<Option<Vec<u8>>> {
        let page = self
            .pages
            .iter()
            .find(|page| page.page_index == page_index)
            .ok_or_else(|| eyre::eyre!("missing EC mobile animation atlas page metadata for {page_index}"))?;
        if !page.has_hue_mask() {
            return Ok(None);
        }
        let mask = read_path_entry(&self.package, &hue_mask_entry_path(page_index))
            .wrap_err_with(|| format!("unpack EC mobile animation hue mask page {page_index}"))?;
        let expected_len = page.used_width as usize * page.used_height as usize;
        if mask.len() != expected_len {
            eyre::bail!(
                "EC mobile animation hue mask page {page_index} has {} bytes, expected {expected_len}",
                mask.len()
            );
        }
        Ok(Some(mask))
    }

    /// Hue mask of a frame, one byte per pixel (`HUE_MASK_HUEABLE` or 0); `None` for
    /// `FRAME_HUE_NONE` or an empty frame.
    pub fn read_frame_hue_mask(&self, frame: &MobileAnimEcFrameRecord) -> eyre::Result<Option<Vec<u8>>> {
        if frame.page_index == MISSING_PAGE_INDEX || frame.width == 0 || frame.height == 0 {
            return Ok(None);
        }
        match frame.hue_mode {
            FRAME_HUE_NONE => Ok(None),
            FRAME_HUE_ALL => {
                let rgba = self.read_frame_rgba(frame)?;
                Ok(Some(
                    rgba.chunks_exact(RGBA_CHANNELS)
                        .map(|pixel| if pixel[RGBA_ALPHA_CHANNEL] != 0 { HUE_MASK_HUEABLE } else { 0 })
                        .collect(),
                ))
            }
            FRAME_HUE_MASKED => {
                let page = self
                    .pages
                    .iter()
                    .find(|page| page.page_index == frame.page_index)
                    .ok_or_else(|| eyre::eyre!("missing EC mobile animation atlas page metadata for {}", frame.page_index))?;
                let mask = self.read_page_hue_mask(frame.page_index)?.ok_or_else(|| {
                    eyre::eyre!("EC mobile animation page {} has masked frames but no hue mask", frame.page_index)
                })?;
                let stride = page.used_width as usize;
                let (x, y) = (frame.x as usize, frame.y as usize);
                let (width, height) = (frame.width as usize, frame.height as usize);
                if x + width > stride || y + height > page.used_height as usize {
                    eyre::bail!("EC mobile animation frame rect is outside hue mask page {}", frame.page_index);
                }
                let mut frame_mask = Vec::with_capacity(width * height);
                for row in y..y + height {
                    frame_mask.extend_from_slice(&mask[row * stride + x..row * stride + x + width]);
                }
                Ok(Some(frame_mask))
            }
            mode => eyre::bail!("invalid EC mobile animation frame hue mode {mode}"),
        }
    }
}

const RGBA_CHANNELS: usize = 4;
const RGBA_ALPHA_CHANNEL: usize = 3;

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<MobileAnimEcPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation page magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("EC mobile animation page manifest", version, MOBILE_ANIM_EC_METADATA_VERSION)?;
    let width = cursor.read_u32::<LittleEndian>()?;
    let height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid EC mobile animation pixel format"))?;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid EC mobile animation packing mode"))?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(count);
    for _ in 0..count {
        pages.push(MobileAnimEcPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            frame_count: cursor.read_u32::<LittleEndian>()?,
            atlas_width: cursor.read_u32::<LittleEndian>()?,
            atlas_height: cursor.read_u32::<LittleEndian>()?,
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
            pixel_format,
            flags: cursor.read_u8()?,
        });
    }
    Ok((width, height, gutter, packing_mode, pages))
}

fn parse_animation_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimEcAnimationRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != ANIMATION_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation manifest magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("EC mobile animation manifest", version, MOBILE_ANIM_EC_METADATA_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut animations = Vec::with_capacity(count);
    for _ in 0..count {
        animations.push(MobileAnimEcAnimationRecord {
            body_id: cursor.read_u32::<LittleEndian>()?,
            action_id: cursor.read_u16::<LittleEndian>()?,
            direction: cursor.read_u8()?,
            frame_start: cursor.read_u32::<LittleEndian>()?,
            frame_count: cursor.read_u16::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
            source_body_id: cursor.read_u32::<LittleEndian>()?,
            resolved_action_id: cursor.read_u16::<LittleEndian>()?,
            alias_action_id: cursor.read_u16::<LittleEndian>()?,
            resolved_action_id_no_flying: cursor.read_u16::<LittleEndian>()?,
            fps: cursor.read_f32::<LittleEndian>()?,
            fps_no_flying: cursor.read_f32::<LittleEndian>()?,
            skipped_frame_mask: cursor.read_u32::<LittleEndian>()?,
        });
    }
    Ok(animations)
}

fn parse_frame_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimEcFrameRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != FRAME_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation frame magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("EC mobile animation frame manifest", version, MOBILE_ANIM_EC_METADATA_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        frames.push(MobileAnimEcFrameRecord {
            animation_index: cursor.read_u32::<LittleEndian>()?,
            frame_index: cursor.read_u16::<LittleEndian>()?,
            source_frame_index: cursor.read_u16::<LittleEndian>()?,
            source_action: cursor.read_u16::<LittleEndian>()?,
            source_frame_number: cursor.read_u16::<LittleEndian>()?,
            page_index: cursor.read_u32::<LittleEndian>()?,
            page_frame_index: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
            center_x: cursor.read_i16::<LittleEndian>()?,
            center_y: cursor.read_i16::<LittleEndian>()?,
            hue_mode: cursor.read_u8()?,
            shadow: ShadowPaletteStats {
                min_inverse: cursor.read_u8()?,
                mean_inverse: cursor.read_u8()?,
                max_inverse: cursor.read_u8()?,
            },
        });
    }
    Ok(frames)
}

fn parse_item_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimEcItemRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != ITEM_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation item magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("EC mobile animation item manifest", version, MOBILE_ANIM_EC_METADATA_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let string_len = cursor.read_u32::<LittleEndian>()? as usize;
    let mut raw_records = Vec::with_capacity(count);
    for _ in 0..count {
        raw_records.push((
            cursor.read_i32::<LittleEndian>()?,
            cursor.read_i16::<LittleEndian>()?,
            cursor.read_i16::<LittleEndian>()?,
            cursor.read_u8()?,
            cursor.read_u32::<LittleEndian>()?,
            cursor.read_u16::<LittleEndian>()?,
            cursor.read_u32::<LittleEndian>()?,
            cursor.read_u16::<LittleEndian>()?,
        ));
    }
    let mut strings = vec![0u8; string_len];
    cursor.read_exact(&mut strings)?;
    let mut items = Vec::with_capacity(count);
    for (item_id, item_type, layer, flags, name_start, name_len, source_hint_start, source_hint_count) in raw_records {
        let start = name_start as usize;
        let end = start + name_len as usize;
        let name = std::str::from_utf8(strings.get(start..end).unwrap_or(&[]))
            .unwrap_or("")
            .to_string();
        items.push(MobileAnimEcItemRecord {
            item_id,
            item_type,
            layer,
            flags,
            name,
            source_hint_start,
            source_hint_count,
        });
    }
    Ok(items)
}

fn parse_source_hint_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimEcSourceHintRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SOURCE_HINT_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation source hint magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("EC mobile animation source hint manifest", version, MOBILE_ANIM_EC_METADATA_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut source_hints = Vec::with_capacity(count);
    for _ in 0..count {
        source_hints.push(MobileAnimEcSourceHintRecord {
            item_id: cursor.read_i32::<LittleEndian>()?,
            action_id: cursor.read_u16::<LittleEndian>()?,
            uop_index: cursor.read_u8()?,
            block_index: cursor.read_u32::<LittleEndian>()?,
            file_index: cursor.read_u32::<LittleEndian>()?,
        });
    }
    Ok(source_hints)
}

fn parse_body_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimEcBodyRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != BODY_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation body magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("EC mobile animation body manifest", version, MOBILE_ANIM_EC_METADATA_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut bodies = Vec::with_capacity(count);
    for _ in 0..count {
        bodies.push(MobileAnimEcBodyRecord {
            body_id: cursor.read_u32::<LittleEndian>()?,
            sequence_id: cursor.read_u32::<LittleEndian>()?,
            frames_body_id: cursor.read_u32::<LittleEndian>()?,
            appearance_body_id: cursor.read_u32::<LittleEndian>()?,
            forced_hue: cursor.read_u16::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
            sequence_stored_id: cursor.read_u32::<LittleEndian>()?,
            race: cursor.read_u8()?,
            gender: cursor.read_u8()?,
        });
    }
    if !bodies.windows(2).all(|pair| pair[0].body_id < pair[1].body_id) {
        eyre::bail!("EC mobile animation body records are not sorted by body id");
    }
    Ok(bodies)
}

pub fn page_entry_path(page_index: u32, fmt: PagePixelFormat) -> String {
    format!("pages/{page_index:05}.{}", fmt.extension())
}

/// R8 hue mask of a page with `PAGE_FLAG_HUE_MASK`, same size as the page's used area.
pub fn hue_mask_entry_path(page_index: u32) -> String {
    format!("masks/{page_index:05}.r8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ec_mobile_animations_kdl_asset() {
        let metadata = EcMobileAnimationsKdl::parse(
            "EcMobileAnimations.kdl",
            include_str!("../../../dynamapper/assets/cc_ec_convtables/EcMobileAnimations.kdl"),
        )
        .unwrap();

        let male = metadata.items.iter().find(|item| item.id == 400).unwrap();
        assert_eq!(male.name, "Human Male");
        assert_eq!(male.item_type, 3);
        assert_eq!(male.layer, 0);
        assert_eq!(male.male, Some(true));
        assert!(male.animations.iter().any(|animation| {
            animation.action_id == 0
                && animation.uop == "AnimationFrame1.uop"
                && animation.block == 0
                && animation.file == 1213
        }));
    }

    fn body_manifest_bytes(version: u32, records: &[(u32, u16, u32, u8, u8)]) -> Vec<u8> {
        let mut bytes = BODY_MANIFEST_MAGIC.to_vec();
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for &(body_id, flags, sequence_stored_id, race, gender) in records {
            for value in [body_id, body_id, body_id, body_id] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&NO_FORCED_HUE.to_le_bytes());
            bytes.extend_from_slice(&flags.to_le_bytes());
            bytes.extend_from_slice(&sequence_stored_id.to_le_bytes());
            bytes.extend_from_slice(&[race, gender]);
        }
        bytes
    }

    #[test]
    fn body_manifest_reads_creature_fields() {
        let human_female = 401;
        let creature = 12;
        let elf_race = 2;
        let female_gender = 1;
        let bytes = body_manifest_bytes(MOBILE_ANIM_EC_METADATA_VERSION, &[
            (creature, 0, creature, 0, 0),
            (human_female, BODY_FLAG_HAS_RACE | BODY_FLAG_HAS_GENDER, 400, elf_race, female_gender),
        ]);
        let bodies = parse_body_manifest(&bytes).unwrap();
        assert_eq!((bodies[0].race(), bodies[0].gender()), (None, None));
        assert_eq!(bodies[1].sequence_stored_id, 400);
        assert_eq!((bodies[1].race(), bodies[1].gender()), (Some(elf_race), Some(female_gender)));
    }

    #[test]
    fn older_manifests_ask_for_a_repack() {
        let older_version = MOBILE_ANIM_EC_METADATA_VERSION - 1;
        let bytes = body_manifest_bytes(older_version, &[]);
        let error = parse_body_manifest(&bytes).unwrap_err().to_string();
        assert_eq!(
            error,
            format!(
                "EC mobile animation body manifest: package version {older_version} is not supported \
                 (current is {MOBILE_ANIM_EC_METADATA_VERSION}); repack with the current udd-cli"
            )
        );

        let mut pages = PAGE_MANIFEST_MAGIC.to_vec();
        pages.extend_from_slice(&older_version.to_le_bytes());
        assert!(parse_page_manifest(&pages).unwrap_err().to_string().contains("repack"));
    }

    fn animations_error(content: &str) -> String {
        EcMobileAnimationsKdl::parse("test.kdl", content).expect_err("parse error").to_string()
    }

    #[test]
    fn mobile_animations_reject_unknown_nodes_properties_and_missing_values() {
        assert_eq!(
            animations_error("item 400 name=\"Human Male\" type=3 layer=0 {\n    frame 0\n}\n"),
            "test.kdl: line 2, column 5: unknown node `frame` in `item`: expected `anim`"
        );
        assert_eq!(
            animations_error("item 400 name=\"Human Male\" type=3 layer=0 elf=true\n"),
            "test.kdl: line 1, column 43: unknown property `elf` of `item`: expected one of `name`, `type`, \
             `layer`, `male`, `female`, `gargoyle`"
        );
        let missing = animations_error("item 400 name=\"Human Male\" type=3\n");
        assert!(missing.starts_with("test.kdl: invalid KDL: line 1, column 1:"), "{missing}");
        assert!(missing.contains("layer"), "{missing}");
    }
}
