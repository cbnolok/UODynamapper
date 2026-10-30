# udd-assets

`udd-assets` provides runtime readers and package access helpers for converted
`.uddp` assets. It is used by `dynamapper` at runtime and by tools that need to
read already-converted packages (e.g. `uddp-inspector-gui`, `udd-conv` during
validation).

## Responsibilities

- Typed package wrappers for each runtime asset type: `MobileAnimCcPackage`,
  `MobileAnimEcPackage`, `GumpsPackage`, `AtlasCacheOptions`, and the tilemeta
  readers (`TileMetaLandTile`, `TileMetaItemTile`).
- Lazy/cached atlas page decoding with configurable cache options.
- BC7-aware texture decoding for atlas pages (via `udd-image-codecs`).
- KDL-based runtime asset configuration reading (`knuffel`).
- Helpers for locating and opening named entries by virtual path hash.

## Features

- `bc7-encode`: re-exports `udd-image-codecs/bc7-encode`. Needed only if the
  caller encodes BC7 data at runtime (uncommon).

## Notes

- This crate depends on `udd-container` for package I/O, `udd-image-codecs` for
  decoding, and `uocf` for tile metadata types shared with the source parsers.
- `dynamapper` is the primary consumer at runtime. Tools use it mainly for
  inspection and validation.

## `mobile_anim_ec.uddp` (metadata version 5)

Built by `udd-conv` from the EC `AnimationFrame1..6.uop`, `AnimationSequence.uop` and the
`tileart.uop` creature placeholders, following the client (`scripts/dev-helpers/ec-research/ghidra-animation/results/uosa_animation.md`). Every metadata
entry starts with a 4-byte magic and the u32 version; all values are little-endian.

| Entry | Magic | Content |
|---|---|---|
| `metadata/pages.bin` | `MEPG` | atlas pages; version 4 adds `u8 flags` to each page record |
| `metadata/animations.bin` | `MEAN` | `u32 count`, then `count` animation records (37 bytes) |
| `metadata/frames.bin` | `MEFR` | `u32 count`, then `count` frame records (34 bytes) |
| `metadata/bodies.bin` | `MEBD` | `u32 count`, then `count` body records (26 bytes), sorted by body |
| `metadata/items.bin` | `MEIT` | `EcMobileAnimations.kdl` items (unchanged) |
| `metadata/source_hints.bin` | `MESH` | `EcMobileAnimations.kdl` source hints (unchanged) |
| `pages/NNNNN.{rgba8888,bc7}` | | atlas page payloads |
| `masks/NNNNN.r8` | | hue mask of a page with `PAGE_FLAG_HUE_MASK` (zstd) |

Animation record: `u32 body_id, u16 action_id, u8 direction, u32 frame_start, u16 frame_count,
u16 flags, u32 source_body_id, u16 resolved_action_id, u16 alias_action_id,
u16 resolved_action_id_no_flying, f32 fps, f32 fps_no_flying, u32 skipped_frame_mask`.

Frame record: `u32 animation_index, u16 frame_index, u16 source_frame_index, u16 source_action,
u16 source_frame_number, u32 page_index, u16 page_frame_index, u16 x, u16 y, u16 width, u16 height,
i16 center_x, i16 center_y, u8 hue_mode, u8 shadow_min, u8 shadow_mean, u8 shadow_max`.

Body record: `u32 body_id, u32 sequence_id, u32 frames_body_id, u32 appearance_body_id,
u16 forced_hue, u16 flags, u32 sequence_stored_id, u8 race, u8 gender`.

Both records are fixed-size; nothing in them is variable-length.

Version 4 decodes the pixels like the client (palette byte 3 is a keep-colour flag, not alpha:
solid pixels are opaque, edge pixels keep their colour with coverage alpha, straight alpha), sets
the anchor to `(-x0, -y0)` of the frame box, adds the hue modes and masks, and maps server bodies
through `tileart.uop` (`metadata/bodies.bin`). Version 3 replaced the placeholder records of
versions 1 and 2. Version 5 adds the creature fields: the shadow palette statistics of each frame
and the sequence stored id, race and gender of each body. The reader still accepts versions 1..5 for
pages, items and source hints, but rejects animation, frame and body manifests older than 5: rebuild
the package.

Bodies (`MobileAnimEcPackage::bodies` / `body`): the client draws server body `S` through the
creature placeholder `10000000 + S`. `sequence_id` is its body type (the `AnimationSequence`
entry); `appearance_body_id` the frame body it asks for (props1 key 8 appearance, or a non-zero
animation appearance); `frames_body_id` the frame body used, body 0 when the appearance has no
frames (`BODY_FLAG_FRAMES_FALLBACK`). Ghost bodies (402, 403, 607, 608, 694, 695) carry
`BODY_FLAG_GHOST` and `forced_hue = 0x3e3`: draw them with that hue whatever the mobile hue.
Bodies with frames or a sequence but no placeholder use their own sequence and frames
(`BODY_FLAG_NO_PLACEHOLDER`; the client cannot show them); `BODY_FLAG_NO_SEQUENCE` marks bodies
whose records are the no-sequence fallback below.

Creature fields (version 5):

- `sequence_stored_id`: the stored id of the body's `AnimationSequence` entry (the sequence id
  when the body has no sequence). The client draws an equipment item only when the item's body
  type equals the wearer's stored id.
- `race` / `gender`: props1 keys 9 and 10 of the placeholder, valid when `BODY_FLAG_HAS_RACE` /
  `BODY_FLAG_HAS_GENDER` is set (`race()` / `gender()` return `None` otherwise; the client then
  uses race 1 and gender 0). The client orders the equipment layers by them and filters item
  appearances by the owner's gender.
- `tests/mobile_anim_ec_package_client.rs` (ignored) checks a built package against the client
  files: set `UO_EC_CLIENT_DIR` and `UDD_MOBILE_ANIM_EC_UDDP` and run with `--ignored`.

Hues: `hue_mode` is `FRAME_HUE_NONE` (no pixel takes the hue), `FRAME_HUE_ALL` (every pixel with
alpha > 0 does) or `FRAME_HUE_MASKED` (the R8 page mask `masks/NNNNN.r8`, sized like the used
area of the page, is 255 on the pixels that do, inside the frame rect; gutters repeat the edge).
`read_frame_hue_mask` returns the per-pixel mask for any mode. A hued pixel takes `table[red]` of
the hue table and keeps its alpha (`uocf::enhanced::hues`).

Shadow hue 0x4001 (the client's look for a dead bonded pet): every pixel turns black, and its alpha
depends on its palette colour and on statistics of the whole palette of the source
`AnimationFrame` file. The frame record stores those statistics (`shadow`, a
`ShadowPaletteStats` of the inverse luminance: min, integer mean, max). A pixel's shadow alpha is
`shadow.alpha(rgb)` of its decoded colour, times its edge coverage; `EcHueTables::apply(..,
&frame.shadow)` does both.

Resolving "body, action, direction" (`MobileAnimEcPackage::animation` /
`resolve_animation`):

- Records are keyed by server body and sorted by `(body_id, action_id, direction)`. Each action
  of the body's sequence has a record for each of the 8 directions. `source_body_id` is the
  frame body (`frames_body_id` of the body record); bodies with the same sequence and frame body
  (402 and 400) share the frame records, and so the pixels.
- `frame_start..frame_start + frame_count` index the frame records. For a sequence action every
  direction has exactly the sequence frame count of frames, so the position is the frame index
  within the direction; a frame number missing from the source entry is a placeholder
  (`source_frame_index == MISSING_SOURCE_FRAME_INDEX`, `page_index == MISSING_PAGE_INDEX`). An
  action without an `AnimationFrame` entry has 0 frames. Source frames whose number lies beyond
  `5 * frame_count`, repeated numbers, and frame entries of actions the sequence does not use as
  plain actions are not packed.
- Directions 3..7 own their frames. Directions 0..2 carry `ANIMATION_FLAG_MIRRORED` and point at
  the frames of direction `6 - direction`: draw them flipped horizontally about the anchor (the
  flipped anchor x is `width - center_x`).
- Alias actions point at the frames of the resolved action (`ResolveAction` with flying
  animations enabled: one alias level; an alias of an alias falls back to the lowest plain
  action). `alias_action_id` keeps the raw alias (`NO_ALIAS_ACTION` for a plain action).
- `fps` follows `GetActionFps` with flying enabled. With the `flying_animations` setting off, use
  the record of `resolved_action_id_no_flying` (same direction) and `fps_no_flying`;
  `resolve_animation(.., false)` does that. An action the sequence does not have falls back to the
  lowest plain action of the body, as in the client.
- `skipped_frame_mask` bit `i`: frame `i` of the direction is passed over (the skipped frames of
  `action_id` in the sequence; frames at index 32 or above are dropped by the converter and
  counted in its summary). `ANIMATION_FLAG_STAND_ACTION` marks a resolved stand action, shown with
  one frame when the `idleAnimation` setting is off.
- Bodies whose sequence is missing carry `ANIMATION_FLAG_NO_SEQUENCE` (the client cannot
  animate them). Their actions come from the `FrameEntry` action and use fps 16 (the client value
  for a missing action). When the highest frame number is a positive multiple of 5 it is split
  into directions like a sequence action of `highest / 5` frames; otherwise every direction shows
  all frames in number order (`ANIMATION_FLAG_UNDIRECTED`).

Not carried: sequence frame effects (footstep sounds), per-direction rider offsets, and the action
groups (move, typed 0xE2 and transition lookups that choose the action). A consumer that needs to
pick actions from movement or packets must read them from `AnimationSequence.uop` with
`uocf::animation_sequence` for now.
