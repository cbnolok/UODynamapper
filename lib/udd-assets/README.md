# udd-assets

`udd-assets` provides runtime readers and package access helpers for converted
`.uddp` assets. It is used by `dynamapper` at runtime and by tools that need to
read already-converted packages (e.g. `uddp-inspector-gui`, `udd-conv` during
validation).

## Responsibilities

- Typed package wrappers for each runtime asset type: `MobileAnimCcPackage`,
  `MobileAnimEcPackage`, `EffectsEcPackage`, `GumpsPackage`, `AtlasCacheOptions`, and the tilemeta
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
entry starts with a 4-byte magic and the u32 version; all values are little-endian. The reader
accepts only version 5 and asks for a repack with the current `udd-cli` otherwise.

| Entry | Magic | Content |
|---|---|---|
| `metadata/pages.bin` | `MEPG` | atlas pages; each page record ends with `u8 flags` |
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

Pixels are decoded like the client (palette byte 3 is a keep-colour flag, not alpha: solid pixels
are opaque, edge pixels keep their colour with coverage alpha, straight alpha), the anchor is
`(-x0, -y0)` of the frame box, pages carry hue modes and masks, and server bodies are mapped
through `tileart.uop` (`metadata/bodies.bin`). Frames carry the shadow palette statistics and
bodies the sequence stored id, race and gender.

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

## `effects_ec.uddp` (metadata version 1)

Built by `udd-conv` (`udd_conv::effects_ec`, CLI `udd-cli pack pack-ec-effects`) from the EC
`EffectDefinitionCollection.uop`, `tileart.uop`, `AnimationSequence.uop`, `string_dictionary.uop` and
`EffectTexture.uop`, following the client research in
`scripts/dev-helpers/ec-research/ghidra-effects/results/uosa_effects.md`. Read it with `EffectsEcPackage`
(`effects_ec.rs`). The package is shaped for the runtime and is not meant to be turned back into NIF or
`.ems` files. It holds:

- the effect definition trees, flattened into index tables;
- every Chaos and NIF system the trees reach, compiled with `particle-sim` and stored as codec blobs that
  go straight to `ChaosSystem::prepare` / `NifEffect::prepare`;
- the textures of those systems, decoded and ready for upload.

Packing is deterministic: the same client always gives the same bytes. All values are little-endian.

### Manifests

Each `metadata/*.bin` entry starts with a 16-byte `ManifestHeader`: `[u8; 4] magic, u32 version (1),
u32 record_count, u32 record_size`. The tightly packed records follow. The reader opens a package only
when every magic, version and record size matches its own, and every cross-index is in range.

| Entry | Magic | Records |
|---|---|---|
| `metadata/package.bin` | `EFPK` | one `PackageRecord`: `u32 chaos_desc_version, u32 nif_desc_version, u32 desc_codec_version` |
| `metadata/strings.bin` | `EFST` | `StringRecord { u32 offset, u32 len }`, then the UTF-8 bytes they point into |
| `metadata/collection.bin` | `EFCO` | `CollectionRecord { u32 collection_id, u32 list_first, u32 list_count }`, sorted by id |
| `metadata/tileart.bin` | `EFTA` | `TileArtRecord { u32 tile_id, u32 list_first, u32 list_count }`, sorted by tile |
| `metadata/sequence_frames.bin` | `EFSF` | `SequenceFrameRecord { u32 sequence_id, u16 action_id, u16 frame, u32 list_first, u32 list_count }`, sorted by key |
| `metadata/def_nodes.bin` | `EFDN` | `DefNodeRecord { u8 type_key, u8 flags, u16 reserved, u32 param, u32 first, u32 count }` |
| `metadata/def_lists.bin` | `EFDL` | `u32` node indices; every list is a contiguous run |
| `metadata/def_branches.bin` | `EFDB` | `DefBranchRecord { u32 key, u32 list_first, u32 list_count }` |
| `metadata/def_params.bin` | `EFDP` | `u32` parameter words |
| `metadata/systems.bin` | `EFSY` | `SystemRecord { u32 name, u8 kind, u8 flags, u16 dep_count, u32 dep_first }`, sorted by name |
| `metadata/deps.bin` | `EFDE` | `u32` dependencies: `DEP_SYSTEM_BIT` (bit 31) and a system index, or a texture index |
| `metadata/textures.bin` | `EFTX` | `TextureRecord { u32 name, u16 width, u16 height, u8 format, u8 mip_count, u8 flags, u8 source }`, sorted by name |
| `systems/NNNNN.chaos`, `systems/NNNNN.nif` | `PSCH` / `PSNF` | the codec blob of system `NNNNN` (`DataType::Effect`) |
| `textures/NNNNN.tex` | | the levels of texture `NNNNN`, level 0 first (`DataType::Texture`) |

`NONE` (`u32::MAX`) marks a missing index. Payload numbers are indices into `systems.bin` and
`textures.bin`. A record without a payload has no entry.

`package.bin` records the desc and codec versions of the blobs. A package built with another
`particle-sim` desc or codec version is rejected with a "rebuild" error. The blob decoder checks the
same versions again in each blob header.

### Definition trees

Roots: `collection_roots(id)` (the effect ids of packets; see below), `tileart_roots(tile)`, and
`frame_roots(sequence, action, frame)`. Each returns a list, and every list value is a node index.
`node(index)` decodes a node into an `EffectNode` by its client type key (`uocf` `EFFECT_TYPE_*`):

| Type | Params (`u32` words at `param`) | `first`/`count` |
|---|---|---|
| Particle (0) | system index (or `NONE`), offset_x, offset_y, camera_offset, scale, duration (f32 bits), attach_mode, field_3c | — |
| Sound | sound_id, value_08 (f32 bits) | — |
| RoundRobin | — | list |
| PositionOffset | 16 bytes `x0 y0 .. x7 y7` (`i8`, per direction from north) | — |
| Wait | min_delay_ms, max_delay_ms | list |
| Lod | — | branches: key = level, ascending |
| PropertyFlag | mask low, mask high, property, default list first, default list count | branches: key = case, ascending |
| Earthquake | amplitude, duration, frequency (f64, low word first) | — |
| Random | — | branches in file order, key = weight (f32 bits) |
| UserPreference | mask | list |

Node flags: `NODE_FLAG_LOOPING`, `NODE_FLAG_HUED` (Particle), `NODE_FLAG_WAIT_ALIGNED`, and
`NODE_FLAG_PROPERTY_MODE` (PropertyFlag `Property` mode; clear means `FlagMask`).

- Lod levels and PropertyFlag cases are client `std::map`s. When a key repeats, the packer keeps its
  last list, as the client does, and reports the repeat. Random weights stay in file order.
- A Particle node names its system by index. The index is `NONE` when the string dictionary lacks the
  name, or when the name is neither `.ems` nor `.nif`. The client plays nothing for such a node.
- `hued`: only NIF effects are ever hued. A hued NIF runs at half speed and is drawn through an
  offscreen pass with a `Hues.dds` lookup. That is runtime behaviour, so the package keeps only the
  flag. `.ems` effects are never hued.
- Collection: only entries named by id are stored. Entries known only by hash cannot be reached by id
  and are reported. The explode field of a 0xC7 moving effect plays collection `(N - 1) & 0xffff`
  (0 is the classic item 0x36CB).
- Sequence frames are keyed by the `AnimationSequence` entry id (`MobileAnimEcBodyRecord::sequence_id`),
  by the action of the record that holds them, and by the frame index. Aliases are not resolved. The
  client looks a frame's effects up under the requested action's own record, and alias records carry
  no effects, so nothing is inherited. Frame effects play on the animated object, so `hued` applies
  to them.

### Systems

Names are normalized: the last path segment of the resource, trimmed and ASCII-lowercased
(`normalize_resource_name`). That is the client's `data/effects/<name>` lookup.
`system_by_name` and `texture_by_name` binary-search on it. `kind` is `SYSTEM_KIND_CHAOS` (`.ems`)
or `SYSTEM_KIND_NIF` (`.nif`).

- Flags: `MISSING` (no `EffectTexture.uop` entry: a client data gap, the client plays nothing),
  `COMPILE_FAILED`, and `COMPILE_WARNINGS` (the packer report lists them). `DEFINITION_ROOT` means a particle definition
  names the system. When it is clear, only a Chaos MESH sequence uses the system.
- Missing and failed systems have no payload.
- Dependencies (`system_dependencies`):
  - Chaos POINT and AXIAL sequences depend on their texture.
  - Chaos MESH sequences depend on their `.nif`, which is packed as a NIF system.
  - A NIF depends on each texture it uses, in texture-index order, including the environment maps of
    its `NiTextureEffect`s.
  - EMITTER sequences are not simulated by `particle-sim`. They are reported and not packed.
- Blobs are `particle_sim::codec` (`encode_chaos_desc` / `encode_nif_desc`). The module documentation
  holds the binary rules and tag tables. A blob decodes exactly to what `particle_sim` compiles from
  the source; the packer checks this round trip for every system.
- Embedded NIF textures (`NiPixelData`) are stored as the textures `<nif name>#<index>`. Their desc
  sources are rewritten to `External { "<nif name>#<index>" }`, so a renderer resolves every texture
  of a desc by name through `texture_by_name`. `udd_conv::effects_ec::compile_nif_for_package`
  compiles a NIF the same way.

### Textures

Formats: `RGBA8` (top row first, straight alpha), `BC7`, `BC1`, and `BC3`. All are gamma-space
`Unorm` with no sRGB, like the D3D9 fixed-function client. Level `n` is `max(1, size >> n)`, and a
payload is every level back to back (`texture_payload_len`). Flags:

- `EMBEDDED`;
- `OPAQUE` (every level-0 alpha is 255);
- `AUTHORED_MIPS` (levels 1.. come from the source);
- `MISSING`, `UNDECODED` (a `NiPixelData` pixel format the packer does not decode) and `DECODE_FAILED`.
  These three mean no payload.

`source` is `TGA`, `DDS`, `NIF_PIXEL_DATA` (embedded in a NIF system) or `NIF_FILE` (a `.nif` texture
file).

- DDS DXT1 and DXT5 sources with a size that is a multiple of 4 pass through as BC1 and BC3 blocks.
  Other DDS sources decode to RGBA8. The levels kept follow `NiDDSReader` (`NiDDSReader.cpp:313-323`):
  the file's levels when `DDSCAPS_COMPLEX | DDSCAPS_MIPMAP` is set and the count is not 1.
- TGA is decoded as Gamebryo 2.6 `NiTGAReader` decodes it (`CoreLibs/NiMain/NiTGAReader.cpp`):
  - types 1, 2, 3, 9, 10 and 11;
  - RLE runs that cross rows;
  - rows flipped unless attribute bit 0x20 is set;
  - 8-bit greyscale expanded to R = G = B with alpha 255 (`Unpack8BitSourceRowGray`, `:176-189`, into
    RGB24);
  - alpha only from 32-bit pixels or colour-map entries (`ReadHeader`, `:453-466`);
  - 15/16-bit pixels as `(hi & 0x7C) << 1, ((hi & 3) << 6) + ((lo & 0xE0) >> 2), (lo & 0x1F) << 3`,
    without bit replication;
  - colour-map indices taken relative to the first entry.

  4-bit colour-mapped pixels are not supported.
- A `.nif` texture file is read as `NiNIFImageReader::ReadFile` reads it (`NiNIFImageReader.cpp:42-70`):
  the file must hold exactly one top object, and it must be a `NiPixelData`. Its pixels then follow the
  rules of embedded `NiPixelData`: 32-bit RGBA and 24-bit RGB decode to RGBA8, other formats are
  `UNDECODED`, and the stored levels are kept when their sizes halve level by level (else level 0
  only). The client's three (`03000017_star.nif`, `03000018_circle_g.nif`, `03002014_flare.nif`) are
  128x128 RGBA with 8 levels.
- Mipmaps follow each texture's own `NiSourceTexture` format preferences, as the D3D9 renderer applies
  them (`NiDX9SourceTextureData.cpp:375-378`): a texture is mipmapped when a user says `YES`, or says
  `MIP_DEFAULT` and the default is on. The SDK default is off (`NiTexture.cpp:27`
  `ms_bMipmapByDefault = false`), and Chaos-only textures count as `MIP_DEFAULT`. When users disagree
  and one says `YES`, the texture gets mipmaps and the report notes the conflict.
- A chain is generated only as `NiDevImageConverter` generates one (`NiDevImageConverter.cpp:453-470`):
  - the source has a single level;
  - the source is uncompressed (not a block-compressed DDS);
  - width and height are powers of two.

  It is a truncating 2x2 box (`(a + b + c + d) >> 2`, or `(a + b) >> 1` once a side is 1) down to 1x1
  (`GenerateMipmapLevels`, `:1108-1290`). Authored levels are kept as they are.
- RGBA8 is the default. With `--bc7`, RGBA8 textures whose size is a multiple of 4 are encoded to BC7,
  every level included. BC1 and BC3 sources stay as they are. Sizes that are not a multiple of 4 stay
  RGBA8.

### Compression and the packer report

Manifests and textures use the CLI compression (`ZstdNoDict` by default, `--zstd=LEVEL`). System blobs
use `CompressionFlag::Auto`, which takes a trained dictionary for small blobs when one is worth training.

The packer lists every finding with a severity:

- errors: sources that do not decode, codec mismatches, definition validation errors (the client
  fails to play them, or the readers reject them);
- warnings: missing particle files (one per file, listing every definition and Chaos MESH sequence
  naming it; a client data gap, the system record keeps `MISSING`), missing textures, dropped
  hash-only entries, EMITTER sequences;
- info: compiler notes, alias records, mip conflicts, and the unreferenced `EffectTexture.uop`
  resources (`.ems`, `.nif`, `.tga`, `.dds` that nothing reaches; not packed).

`--report FILE` writes the findings, one line each. `--strict` fails on any error finding; missing
particle files never fail it.

`tests/integration/effects_ec_package_client.rs` in `udd-conv` (ignored) checks a package against the client.
Set `UO_EC_CLIENT_DIR`; set `UDD_EFFECTS_EC_UDDP` to check a prebuilt package instead of packing in
memory; run with `--ignored`. It checks that:

- every collection entry equals the client tree;
- every tile and frame resolves;
- every blob equals a fresh compile;
- textures match their sources' size, format and levels;
- decoded and freshly compiled systems simulate identical frames from the same seed.
