# UOKR.exe: AnimationFrame*.uop as the client reads it

Program `UOKR.exe` (Kingdom Reborn), SHA-256 `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`.
Nothing animation-related was named before this folder. RTTI: `.?AVUOMobFrameSet@@`,
`.?AVUOAnimationFrameSetFactory@@`, `.?AVUOAnimationFrameSet@@`; the path string `client\AnimationFrame`.
**confirmed** = read at the instruction level; **probable** = from the decompiled code.

## Functions (named by NameAnimationFrameCode.java, 2026-10-31)

| Address | Name | Same as UOSA.exe? |
|---|---|---|
| 0x004a1330 | `UOAnimationFrameSetFactory::Create` | yes (`RET 0x14`, does not read ECX) |
| 0x004a11d0 | `UOMobFrameSet::UOMobFrameSet` | logic yes; the object is in ESI, stack: blob shared_ptr (2 slots), ok (`RET 0xc`) |
| 0x00424120 | `UOAnimations::FindFrameEntry` | yes, instruction for instruction |
| 0x00420c90 | `UOAnimations::BuildFramePalette` | no: hue key (below) |
| 0x0041d750 | `UOHues::GetHueTable` | yes (0x4001 -> 0x9c3, 0x8000 masked, 0x4000 alone none) |
| 0x00420e50 | `UOAnimations::DecodeFrameRle` | yes |
| 0x00421040 | `UOAnimations::DecodeFrameRleMirrored` | yes |
| 0x00421250 | `UOAnimations::DecodeFrameMask` (probable) | yes |
| 0x00420b90 | `UOAnimations::GetLayerBounds` (probable) | yes |
| 0x00424ab0 | `UOAnimations::BuildFrameAtlas` | atlas sizing yes; no layers -> 64x64 filled with 0x335555ff (UOSA.exe: zero) |

## Differences from UOSA.exe

- **Hue key.** For a hueable colour (flag byte 3 == 0) the hue table entry is
  `table[(b0 + b1 + b2) / 3]` (integer average), where UOSA.exe uses `table[b0]`. The table entry's top
  byte becomes the word's alpha. **confirmed** (0x00420c90). The shipped hueable colours are mostly not grey
  (AnimationFrame1.uop: EC 1,398,115 non-grey vs 33,258 grey hueable entries; KR 1,139,821 vs 11,356), so
  the same file hues differently in the two clients. The format is the same; the hue key is a client
  property, and an encoder has nothing to choose here.
- The builder has no 0x4001 (shadow) special case; the special lives in the hue table getter's key only.
- The constructor convention (ESI object) differs; the stream reads (signature, version, total size, no
  checks) are the same. **confirmed**.

Everything else an encoder depends on (header fields read, table order, offsets relative to the entry,
index bounds, RLE rules and row limits, atlas limits) is the same as in
[uosa_animation_frames.md](uosa_animation_frames.md).
