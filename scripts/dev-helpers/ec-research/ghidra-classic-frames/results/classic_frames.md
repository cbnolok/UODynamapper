# classic_7.0.103.0_client.exe: Classic animation frames

Program `classic_7.0.103.0_client.exe` (SHA-256 `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a`).
**confirmed** = read in the code; **probable** = inferred from the code and the shipped data.

## Functions

| Address | Name | Notes |
|---|---|---|
| 0x00449c90 | `UOClassicAnim::DrawFrameRle` | draws a frame's runs; hue modes; mirror |
| 0x00449aa0 | `UOClassicAnim::DrawFrameShadowRle` | the same runs as a shadow (0x3DEF blend) |
| 0x0044a4c0 | `UOClassicAnim::FrameRleHitTest` | pixel under the cursor |
| 0x005d4dc0 | `UOClassicAnim::DrawFrameRleToBuffer` | probable name: a 64x128 buffer, anchor (32, 80), same hue modes |
| 0x00543500 | `UOClassicAnim::GetMulFrame` | `anim*.mul` entry → frame |
| 0x00543e50 | `UOClassicAnim::GetUopFrame` | `animationlegacyframe` entry → frame |
| 0x00542c40 | `UOClassicAnim::FindFrameEntry` | binary search of the AMOU table |
| 0x005bc880 | `UOClassicAnim::GetMobileFrame` | MUL or UOP by body |
| 0x00452ad0 | `UOClassicAnim::DrawMobile` | probable name: calls the two above |
| 0x0056c170 | `UOClassicFiles::ReadMulIndexEntry` | probable name: idx lookup / length |
| 0x00543890 | `UOClassicAnim::GetUopFrameSet` | probable name: requests the frame set asset |
| 0x00413752 | `UOMobFrameSet::Construct` | the asset object (vtable 0x006db10c) |
| 0x0041385a | `UOAnimationFrameSetFactory::CreateAsset` | "Failed to get animation frame set" |

## The frame

1. **Palette.** 256 `u16` words before the frames (MUL: at the entry start; UOP: at the start of every frame's
   data). The client draws `palette[index]` for every pixel of a run, bit 15 and a word of 0 included (0 draws
   black): there is no transparent index or colour. **confirmed** (`DrawFrameRle`).
2. **Header.** `i16` anchor x, `i16` anchor y, `u16` width, `u16` height. **confirmed**.
3. **Runs.** A `u32` header: `x = (int)hdr >> 22`, `y = (hdr << 10) >> 22` (both signed, 10 bits),
   length `hdr & 0xFFF`; then `length` palette indices. The pixel drawn is anchor-relative: image pixel
   `(x, y)` has offsets `(x - anchor x, y - anchor y - height)`. **confirmed** for the decoding, **probable** for
   the image mapping (it matches every shipped frame and uocf's decoder).
4. **End.** The `u32` 0x7FFF7FFF. Drawing stops at the first run below the clip box, so runs go top to bottom.
   Mirroring reads the runs backwards from the marker. **confirmed**.
5. **Hue.** A hue of 0x3FFF bits takes hue table entry hue - 1 (`DAT_00a6b810`, stride 0x2C); bit 0x8000 hues
   only the greys (`(c & 0x3FF) == c >> 5`). **confirmed**.

## The sources

1. **MUL.** `GetMulFrame` reads the idx entry, then at the entry start the palette, a `u32` frame count and
   `u32` offsets relative to the end of the palette; an offset of 0 is a missing frame (skipped). **confirmed**.
2. **UOP.** `GetMobileFrame` takes the MUL path when `0x005978a0(body)` is false, else `GetUopFrame`, which
   requests the frame set of (body, action), searches its table with `FindFrameEntry` by (action, frame number)
   and decodes at the entry's data offset; a width or height of 0 gives no frame. The container fields read are
   the frame count (+0x20), the table offset (+0x24) and each entry's key and data offset. **confirmed**.
3. **Unread fields.** The header box, the colour table and the entry boxes of the AMOU container are not read
   on this path. `UOMobFrameSet::Construct` reads three `u32` from its stream, not identified. **probable** that
   nothing else reads them.

## For a writer

- The shipped encoder: runs row by row, left to right, each a maximal span of opaque pixels split at 0xFFF; no
  bytes after the marker; MUL frames contiguous after the offset table. The uocf writer reproduces every shipped
  frame byte for byte (`classic_animation_frame_encoding` real-file tests).
- Transparency is only "not in a run": an RGBA pixel of alpha 0 is left out, every other pixel is opaque.
- uocf's decoders and ClassicUO draw a colour of 0 transparent, the client black: new palettes store black as
  0x0001.
