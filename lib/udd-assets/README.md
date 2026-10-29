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

## `mobile_anim_ec.uddp` (metadata version 3)

Built by `udd-conv` from the EC `AnimationFrame1..6.uop` and `AnimationSequence.uop`, following the
client (`scripts/dev-helpers/ec-research/ghidra-animation/results/uosa_animation.md`). Every metadata
entry starts with a 4-byte magic and the u32 version; all values are little-endian.

| Entry | Magic | Content |
|---|---|---|
| `metadata/pages.bin` | `MEPG` | atlas pages (unchanged since version 2) |
| `metadata/animations.bin` | `MEAN` | `u32 count`, then `count` animation records (37 bytes) |
| `metadata/frames.bin` | `MEFR` | `u32 count`, then `count` frame records (30 bytes) |
| `metadata/items.bin` | `MEIT` | `EcMobileAnimations.kdl` items (unchanged) |
| `metadata/source_hints.bin` | `MESH` | `EcMobileAnimations.kdl` source hints (unchanged) |
| `pages/NNNNN.{rgba8888,bc7}` | | atlas page payloads |

Animation record: `u32 body_id, u16 action_id, u8 direction, u32 frame_start, u16 frame_count,
u16 flags, u32 source_body_id, u16 resolved_action_id, u16 alias_action_id,
u16 resolved_action_id_no_flying, f32 fps, f32 fps_no_flying, u32 skipped_frame_mask`.

Frame record: `u32 animation_index, u16 frame_index, u16 source_frame_index, u16 source_action,
u16 source_frame_number, u32 page_index, u16 page_frame_index, u16 x, u16 y, u16 width, u16 height,
i16 center_x, i16 center_y`.

Version 3 replaced the placeholder records of versions 1 and 2 (one action 0 / direction 0 record
holding every frame of a body). The reader still accepts versions 1..3 for pages, items and source
hints, but rejects animation and frame manifests older than 3: rebuild the package.

Resolving "body, action, direction" (`MobileAnimEcPackage::animation` /
`resolve_animation`):

- Records are keyed by body type (the `AnimationSequence` entry name) and sorted by
  `(body_id, action_id, direction)`. Each sequence action has a record for each of the 8
  directions. `source_body_id` is the sequence stored id under which the frames live
  (146 -> 704, 1308 -> 29, 1309 -> 1254).
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
- Bodies with frames but no sequence carry `ANIMATION_FLAG_NO_SEQUENCE` (the client cannot
  animate them). Their actions come from the `FrameEntry` action and use fps 16 (the client value
  for a missing action). When the highest frame number is a positive multiple of 5 it is split
  into directions like a sequence action of `highest / 5` frames; otherwise every direction shows
  all frames in number order (`ANIMATION_FLAG_UNDIRECTED`).

Not carried: sequence frame effects (footstep sounds), per-direction rider offsets, and the action
groups (move, typed 0xE2 and transition lookups that choose the action). A consumer that needs to
pick actions from movement or packets must read them from `AnimationSequence.uop` with
`uocf::animation_sequence` for now.
