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
  (every EC, KR and Classic entry; bytes after the header's total size are kept as `tail`). Only Enhanced/KR
  frame data has an encoder; Classic `animationlegacyframe` pixels would need a Classic frame encoder.
- Kingdom Reborn frame files are the Enhanced format, but UOKR.exe keys the hue table with the integer mean of
  the three colour bytes where UOSA.exe uses byte 0. Layout, encoder rules and verification marks:
  `lib/uocf/src/enhanced/ENHANCED_CLIENT_FILES.md` (`AnimationFrame*.uop` pixels) and
  `scripts/dev-helpers/ec-research/ghidra-animation-frames`.

## Multi ids

The 2026 `MultiCollection.uop` has multi 9000; ids go up to `0xFFFF` (`MAX_MULTI_COLLECTION_ID`).
