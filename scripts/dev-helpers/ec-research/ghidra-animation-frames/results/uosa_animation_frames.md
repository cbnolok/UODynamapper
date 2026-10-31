# UOSA.exe: AnimationFrame*.uop as the client reads it

Program `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.
The decoders, palette builder and atlas were named by [`ghidra-animation`](../../ghidra-animation/README.md)
(details in its `results/uosa_animation.md`, sections "AnimationFrame", "Palette", "RLE"); this note adds
the frame entry search and lists what an encoder must respect. **confirmed** = read at the instruction level;
**probable** = from the decompiled code and the data.

## Functions

| Address | Name | Notes |
|---|---|---|
| 0x0084217e | `UOAnimationFrameSetFactory::Create` | asset type 0x10; fails (next resolver) only if a stream read fails |
| 0x00842078 | `UOMobFrameSet::UOMobFrameSet` | `__stdcall`, object on the stack |
| 0x00430f60 | `UOAnimations::FindFrameEntry` | new name (this folder) |
| 0x0042b270 | `UOAnimations::BuildFramePalette` | |
| 0x00427ad0 | `UOHues::GetHueTable` | |
| 0x0042b620 | `UOAnimations::DecodeFrameRle` | |
| 0x0042b810 | `UOAnimations::DecodeFrameRleMirrored` | |
| 0x0042ba20 | `UOAnimations::DecodeFrameMask` | probable: hit testing |
| 0x0042b170 | `UOAnimations::GetLayerBounds` | probable |
| 0x00431a10 | `UOAnimations::BuildFrameAtlas` | |

## What the client checks (encoder constraints)

1. **Header.** The constructor stream-reads the u32 signature, version and total size (+0, +4, +8) and
   checks none of them; `Create` fails only when the blob is shorter than those 12 bytes. The animation id
   (+0xc) and the header bounds (+0x10..+0x17) are never read. Only +0x18 colours count, +0x1c colours
   offset, +0x20 frames count and +0x24 frames offset are used, all relative to the entry start.
   **confirmed**. An encoder should still write `AMOU`, version 1, the entry size and the shipped bounds
   (see the format doc) so other tools keep working.
2. **Frame table order.** `FindFrameEntry` (EAX = frame file, stack: action, number; `RET 8`) is a binary
   search over the `frames_count` 16-byte entries at `frames_offset`: it compares the u16 action, then the
   u16 frame number, both unsigned, and returns the entry or 0. The table must be sorted ascending by
   (action, number) without duplicates, otherwise frames go missing. **confirmed** (0x00430f60..0x00430fbe).
   Callers: `UpdateFrame`, 0x0050b780, 0x0050b850.
3. **Frame data offset** is relative to the entry's own address (entry + offset). **confirmed** (decoder).
4. **Palette.** `BuildFramePalette` allocates `colours_count` words and fills them from the 4-byte entries
   [b0, b1, b2, flag]; every colour is opaque; flag 0 = hueable (the hue table, indexed by b0, replaces the
   colour). Pixel indices are bytes and the decoders do not bound-check them: every index must be below
   `colours_count` (at most 256 usable colours). **confirmed**.
5. **RLE.** A byte c < 0x80 skips c pixels, counted linearly over the box, so skips may cross rows. Otherwise
   a flag byte F follows: if F >> 4 is non-zero, one edge pixel with that factor; then c - 0x80 solid
   pixels; then, if F & 15 is non-zero, one edge pixel. Edge blend: per channel (src * f + dst * (16 - f)) >> 4
   over the zeroed atlas. The decoder writes a coded run as a linear span from the current position; the
   skip counter is re-based per row, but a coded run that passes the row end is written into the next row's
   memory (and past the canvas on the last row): runs must not cross rows. **confirmed**.
6. **Atlas.** `BuildFrameAtlas` sizes a power-of-2 texture from the layer bounds: sides at least 4, aspect
   clamped to 8:1; there is no explicit maximum (the D3D texture limit applies). **confirmed**.

## Data verification (shipped files)

Every shipped EC entry (see the real-file test `animation_frame_encoding` in `lib/uocf/tests/real_files`):
table sorted ascending by (action, number), offsets contiguous in table order, no index at or above the
colour count, every coded run inside its row.
