# Classic / Enhanced Client shared UOP formats

Checked on the 2024 and 2026 ("fp") Classic and Enhanced Client folders.

## Same payloads in both clients

- `AnimationSequence.uop`, `MultiCollection.uop` (multis and `housing.bin`), `tileart.uop` and
  `string_dictionary.uop`: every entry payload of the Classic Client is byte-identical to the Enhanced Client
  one of the same release. The `.uop` files themselves differ (container layout), so compare decompressed
  entries, not file hashes.
- One parser serves both clients: `uocf::animation_sequence`, `uocf::enhanced::{multis, housing, tileart}`.
- Only the Enhanced Client ships `AnimationDefinition.uop` and `EffectDefinitionCollection.uop`.
- The ignored real-file tests in `lib/uocf/tests/` read `UO_EC_CLIENT_DIR` and/or `UO_CC_CLIENT_DIR`; their
  exact counts are asserted only for the 2024 release.

## AnimationFrame*.uop: same container, different pixels

- Classic entries are named `build/animationlegacyframe/%06d/%02d.bin`, Enhanced ones
  `build/animationframe/%06d/%02d.bin`.
- Both use the same `AMOU` version 1 header and the same 16-byte `FrameEntry` table: action id = the
  name's action, 1-based frame numbers `(direction - 3) * frame_count + frame + 1` with gaps, header animation
  id = body (the sequence stored id). The `5 * frame_count` relation with `AnimationSequence` holds in both,
  with the same kind of exceptions.
- Frame pixels differ. Classic frame: 256 RGB555 palette, center x/y, width, height, RLE chunks ending with
  `0x7FFF7FFF` (`uocf::classic::animationframe_cc`). Enhanced frame: `uocf::enhanced::animationframe`
  decoder with the header palette. `AnimationFrame::load_metadata` accepts Classic entries (same header),
  so the EC converter must not be pointed at Classic `AnimationFrame*.uop`.
- Classic entry bounds in `FrameEntry` do not match the frame's own width/height/center; use the frame data.
- Classic UOP frames hold only part of the bodies (277 in 2024, 311 in 2026); the others are in `anim*.mul`.
  `mobile_anim_cc` reads both.
- The 2026 Classic `AnimationFrame6.uop` has entries zero-padded after the size stored in the header.
- Container writer (`uocf::enhanced::animationframe::AmouFile`): parses and rewrites both kinds byte for byte
  (every EC, KR and Classic entry; bytes after the header's total size are kept as `tail`). Enhanced/KR frame
  data: `uocf::enhanced::animationframe::encode`; Classic frame data: `uocf::classic::anim_frames`
  ([Classic frames](#classic-frames-anim-mul-and-animationlegacyframe)).
- Kingdom Reborn frame files are the Enhanced format, but UOKR.exe keys the hue table with the integer mean of
  the three colour bytes where UOSA.exe uses byte 0. Layout, encoder rules and verification marks:
  `lib/uocf/src/enhanced/ENHANCED_CLIENT_FILES.md` (`AnimationFrame*.uop` pixels) and
  `scripts/dev-helpers/ec-research/ghidra-animation-frames`.
- The EC and KR `Paperdoll.uop` entries `build/paperdoll/000001/00.bin` (male) and `01.bin` (female) are the
  same Enhanced AMOU format: the paperdoll images, action = gender, frame number = the body or worn item id
  (fallback 400 in the male set). The container and the encoder write both back byte for byte
  (`paperdoll_frame_sets` real-file test); research in `scripts/dev-helpers/ec-research/ghidra-paperdoll`.
  The Classic client has no `Paperdoll.uop`.

## Classic frames: `anim*.mul` and `animationlegacyframe`

The same frame codec in both, read by the 7.0.103.0 client in `UOClassicAnim::DrawFrameRle`, `GetMulFrame`,
`GetUopFrame` and `FindFrameEntry` (`scripts/dev-helpers/ec-research/ghidra-classic-frames/`). **V** = read in
the client code; **S** = holds on every shipped file checked; **I** = inferred.

- **Palette**: 256 `u16` RGB555 words (red bits 10..14, green 5..9, blue 0..4). The client draws
  `palette[index]` for every pixel of a run, bit 15 and a word of 0 included (0 draws black) (V): there is no
  transparent index or colour; transparency is "not in a run". The hued draws, the shadow and the hit test also
  take a colour-0 pixel as opaque (V, per path in the research results). uocf's decoders draw it opaque black
  (ClassicUO draws it transparent); shipped `AnimationFrame2` and `3.uop` frames have such pixels (4,081 frames in the 7.0.103
  `AnimationFrame2.uop`, 9,660 in CC fp, 10,362 in 7.0.20; also 44 in the 7.0.20 `animationframe4.uop`), no MUL
  frame does (S). Index 0 is never used by a pixel (S).
- **Frame**: `i16` anchor x, `i16` anchor y, `u16` width, `u16` height (V); runs; the `u32` end marker
  0x7FFF7FFF (V). Run header `u32`: bits 0..11 the length, 12..21 the signed y offset, 22..31 the signed x
  offset (V: `x = (int)hdr >> 22`, `y = (hdr << 10) >> 22`), then `length` index bytes. Image pixel `(x, y)` has
  offsets `(x - anchor x, y - anchor y - height)` (S, I for the meaning), so a frame must fit offsets
  -512..511. The client stops at the first run below its clip box (V): runs go top to bottom. Mirroring reads
  the runs backwards from the marker (V).
- **Shipped encoder** (S, every frame): runs row by row, left to right, each a maximal span of opaque pixels of
  one row (split at 0xFFF, which the offset range never reaches); no bytes after the marker.
- **MUL entry** (`anim*.idx` 12-byte entries: lookup, length, extra 0): the palette, a `u32` frame count, `u32`
  offsets relative to the end of the palette (0 = missing frame, skipped by the client (V); 365 in the 7.0.103
  `anim.mul`), the frames. Every shipped entry has ascending contiguous offsets from `4 + 4 * count`, no zero-size
  frame and no bytes after its last frame (S). The `.mul` files are not in index order and have gaps (S), so the
  writer appends changed entries at the end and points the index there. An entry is one direction of one action;
  the body/action/direction to index layout is `classic::anim::classic_animation_source_index_from_identity`.
- **UOP entry** (`build/animationlegacyframe/%06d/%02d.bin`): the AMOU container, every frame's data being the
  palette (the same in all frames of a shipped entry, S) and a Classic frame; an empty frame (width or height 0)
  is the palette and an 8-byte zero header, no marker (S; the client returns no frame for it, V). The client
  reads only the frame count (+0x20), the table offset (+0x24) and each entry's key and data offset, the table
  being binary-searched by (action, frame number) (V). The header box, the colour table (EC-like leftovers) and
  the entry boxes are not read on this path (I: `UOMobFrameSet::Construct` reads three unidentified `u32`); a
  rewrite keeps them from the entry it replaces (a new entry: no colour table, entry boxes around each frame).
- **From RGBA** (`index_rgba_frames`): alpha 0 is transparent, 255 opaque, others refused (the gump writer's
  rule); channels round to the nearest 5-bit value. With the entry's palette each colour takes its lowest index
  (bit 15 ignored, index 0 last; black takes a word of 0 when the palette has one, as the shipped frames do);
  without one, the palette is every distinct colour (at most 255, index 0 unused) or a deterministic median cut,
  and black is stored as 0x0001 (the near black the shipped frames use most): ClassicUO draws a 0 transparent,
  and the client's partial hue (0x8000) hues a 0 as a grey but draws 0x0001 as it is.
- **Round trips** (`classic_animation_frame_encoding` real-file tests): indexed re-encode and RGBA with the
  entry's palette byte-identical on 100% of the entries, and the uocf decoder agrees on every frame:
  7.0.103 CC `AnimationFrame1-4.uop` (9,610 entries, 470,415 frames), CC fp `AnimationFrame1-4, 6.uop` (10,724
  entries, 525,916 frames), 7.0.20 `animationframe1-4.uop` (7,307 entries, 357,566 frames); `anim*.mul` of Gold
  (`anim`-`anim4`), Mondain's Legacy (`anim`-`anim5`), 7.0.20 (`anim`-`anim5`), CC (`anim`-`anim5`) and CC fp
  (`anim`-`anim6`): every entry, 100%. A rebuilt palette keeps every pixel (no shipped entry has more than 255
  colours) but its palette order differs from the shipped one, so those bytes differ.
- **Edit**: `uocf-cli classicanim export|import` and the inspector's Animations view (Export frames, Replace
  frames) write a folder of PNGs plus `classicanimation.kdl` (strict KDL: body, action, source, palettes,
  anchors, frame numbers) and encode it back into copies of the files. A UOP entry whose frames per direction
  change also needs its `AnimationSequence.uop` frame count changed (the Classic Client's sequences are the EC
  format). `uop replace-entry` and the inspector's entry editor check a replaced or added
  `animationlegacyframe` entry, by name or by hash, with the Classic reader (one palette, frames that end at
  their marker), and the Enhanced Client's AMOU entries with the Enhanced one. `verdata.mul` patches of animations are not read or written (uocf's `AnimMap` applies them when it is
  given one).

## Multi ids

The 2026 `MultiCollection.uop` has multi 9000; ids go up to `0xFFFF` (`MAX_MULTI_COLLECTION_ID`).

## `.tga` entries that are not TGAs

`artLegacyMUL.uop` (`build/artlegacymul/%08d.tga`) and `gumpartLegacyMUL.uop` (`build/gumpartlegacymul/%08d.tga`) keep
the Classic encoding under a `.tga` name (the Enhanced Client ships the gump package too): art is `u32 flags, u16 w, u16 h, lookup...` or a land diamond
of 2048 bytes, a gump is `u32 w, u32 h, u32 lookup[h], RLE`. None of the 39,386 Classic art entries has a TGA footer.
Decide by the package, never by the extension.

`uocf::enhanced::textures::decode_tga_rgba8` gives an empty image only to a payload of exactly 18 bytes with a zero u16
at the TGA width or height offset: the real header-only entries, 34 in `Interface.uop` and the last one of the Kingdom
Reborn `TileArtLegacy.uop`. A blind read of the classic packages does not always fail, though. Measured on the canonical
clients, the zero at those offsets makes the TGA reader refuse 4,413 of the 4,448 Classic gumps, 4,341 of the 4,376
Enhanced ones and 1,068 of the Classic art entries, but others still come out as tiny garbage images (Classic art
0x4821, of 0xF82 bytes, reads as 2x2).

An empty image handed to egui is a wgpu validation error, and wgpu's default handler panics. The inspector's textures
go through `tools/uocf-inspector-gui/src/ui/textures.rs`, which turns that into a message.
