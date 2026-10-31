# How UOSA.exe sizes classic (legacy) art next to EC art

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.
Data: the plain `_Ultima Online Enhanced` client (`tileart.uop`, `LegacyTexture.uop`, `string_dictionary.uop`)
and the plain `_Ultima Online Classic` client (`artLegacyMUL.uop`).

"Verified" means read in the disassembly or decompilation at the stated address. "Census" means counted in
the client data with the repo readers ([`../legacy_art_census.rs`](../legacy_art_census.rs)). "Inference"
means derived from the two, not read anywhere.

## 1. Constants

| Address | Value | Used at | Meaning |
|---|---|---|---|
| `0x00d05174` | `0x3f2aaaab` = 0.6666667 (2/3) | `0x00459422` (fmul) | mode 2 scale factor: `legacy_art_scale * 2/3` |
| `0x00c803d0` | `0x3fc00000` = 1.5 | `0x00459467` | mode 2 window ends ×1.5; `0x00459a01` origin `-y - 1.5` |
| `0x00d051d8` | 0.021427423 = √2 / 66 | `0x00459b23` | sprite pixel unit `k`, multiplied by the descriptor scale |
| `0x00d05154` | 0.032141134 = √2 / 44 = 1.5 `k` | `0x004598f4` | one legacy pixel (UI branch of the position setter) |
| `0x00d050a0` | 0.12856454 = 4 × 0.032141134 | `0x00459982` | world units per Z step: 4 legacy pixels |
| `0x00c95604` | 0.5 | `0x0045994a`, `0x00459f33` | origin `x + 0.5`; quad world pairs ±0.5 |
| literal | 0x2c = 44 | `BuildSpriteDescriptor` mode 1 | window end when the record has no world art |
| `0x00d0535c`, `0x00d05358`, `0x00d05354` | 0.707105, 1.4742099, 0.06 | `0x0045a0bf`, `0x0045a0c7`, `0x0045a1de` | bit-34 camera quad only (not sprites) |
| `0x00c9c390` | 16.0 | `0x0046f870`, `0x0047647b` | default zoom (viewer +0x80) |
| `0x00c9c39c`, `0x00c9c398` | 9.0, 26.0 | `0x004aa1c7`, `0x004aa1b0` | clamp of the saved zoom `0x00e3368c` |
| `0x00c9c3a4`, `0x00c9c3a8` | 8.0, 12.0 | `SetCameraFrustum` `0x004712f0` | near, far (orthographic) |

## 2. The three descriptor modes (verified, `UOTileArt::BuildSpriteDescriptor` `0x004592e0`)

The use-legacy argument comes from `g_UOWorldViewer` (`0x00e37224`) +0x54 (read at `0x004c92c8` in
`UOObjectDefinition::GetSpriteDescriptor` and at `0x0042ef33`). With it set the record goes to mode 0 directly.
Otherwise `HasFlags2(0x80000)` (UseNewArt) picks mode 1, else `UsesEnhancedTileArt` picks mode 2, else mode 0.

| Mode | Block | Scale (descriptor +0x44) | Window (descriptor +0x00..+0x14) |
|---|---|---|---|
| 1 | 0 (WorldArt) | `world_art_scale` (vt+0x2c) | enhanced window, `x_end + 1`, `y_end + 1`; ends 44 when `HasWorldArt` is 0 |
| 2 | 2 | `legacy_art_scale * 2/3` | legacy window, `x_end` and `y_end` × 1.5 truncated (cvttss2si, `0x00459492`-`0x004594ad`); start and offset unscaled |
| 0 | 1 (TileArtLegacy) | `legacy_art_scale` (vt+0x30) | legacy window unchanged (exclusive ends) |

Mode 2 is mode 0 drawn from a 1.5× upscaled image at the same on-screen size: the window grows 1.5×, the
scale shrinks to 2/3. No record uses it (`uses_enhanced_tileart` is 0 everywhere).

The sprite cache (`UOTileArt::GetCachedSpriteDescriptor` `0x00457a70`) is keyed by tile id only, not by the
legacy flag (verified). That a flag change needs a cache flush is inference.

## 3. Where the scales come from (census of the plain EC `tileart.uop`)

Fields as named in `lib/uocf/src/enhanced/tileart.rs`: `world_art_scale` (record +0x0c, f32),
`legacy_art_scale` (+0x10, f32), `ec_img_offset` (enhanced window, +0x4d, six i32) and `cc_img_offset`
(legacy window, +0x65, six i32: `x_start, y_start, x_end, y_end, x_off, y_off`), all in source pixels of the
block image. Static namespace (36611 records):

| | Records |
|---|---:|
| mode 0, `legacy_art_scale` 1.5 | 35814 |
| mode 0, `legacy_art_scale` 1.0 | 2 (6588, 6589) |
| mode 1, `legacy_art_scale` 1.5 | 795 |
| mode 1 `world_art_scale` 1.0 / 5.0 / 4.0 | 670 / 124 / 1 |

The scale is a pixel-size multiplier, not a UV stretch: `BuildImageWindowQuad` multiplies window pixels by
`scale * k` (section 4). So the unit of `legacy_art_scale` is "sprite pixel units of k", and 1.5 makes one
source pixel exactly one classic pixel (section 5).

## 4. From descriptor to quad (verified)

`UOTileArt::BuildSpriteGeometry` `0x0045a080` (called at the end of `BuildSpriteDescriptor`): bit 34 builds
the camera quad; otherwise it loads the block's primary texture and calls
`UOTileArt::BuildImageWindowQuad` `0x00459b00` with `s` = descriptor +0x44 and window `(x0, y0, x1, y1, ox, oy)`:

```
k      = 0.021427423
width  = floorf(|x1 - x0| * s) * k            (floorf 0x004440b0 -> CRT floor 0x00bd6d70)
height = floorf(|y1 - y0| * s) * k
left   =  ox * s * k                          along the camera right axis (camera rotation column 2)
bottom = -oy * s * k                          along the camera up axis (column 1)
right  = left + width,  top = bottom + height
u      = x0 / texW .. x1 / texW,  v = 1 - y0 / texH (top) .. 1 - y1 / texH (bottom edge)
```

The quad vertices are the camera rotation (camera +0x2c..+0x4c) applied to `(0, up, right)`, so the quad
always lies in the camera plane (a billboard). The edge at `-oy` carries image row `y1` (the bottom of the
window); the 1 - v form means the client's texture v runs from the image bottom (inference about the loader's
row order; the bottom anchoring itself is confirmed by the data in section 5).

`SetSpriteMapPosition` `0x004598d0` (world branch, verified):

```
origin = (x + 0.5, 0, -y - 1.5) - z * 0.12856454 * (viewer +0x24 column)
```

i.e. one map step past the tile centre along map +y, raised by 4 legacy pixels per Z.

Camera (verified in `UOWorldViewer::CreateCamera` `0x00471580`): rotations -π/2, 0.001 - π/4 and -0.05,
camera distance -10; orthographic frustum in `UOWorldViewer::SetCameraFrustum` `0x004712f0`, half-height
`zoom (viewer +0x80) * height fraction (0x00e33590)`, width from the ratio `0x00db9754` computed in
`UOWorldViewer::UpdateViewportFrustum` `0x00471390`. Numerically (inference from those rotations), a ground
tile projects to a diamond √2 wide and √2·cos 0.05 (0.99875) tall in camera units, and map +x / +y run to
screen right-down / left-down.

## 5. Mode-0 placement in classic pixels

With `s = 1.5`: one source pixel is `1.5 k = 0.032141134 = √2 / 44` camera units, so the 44-pixel classic
tile width is exactly the √2 diamond. One mode-1 world-art pixel at `world_art_scale = w` is `w k`, i.e.
`w / 1.5` classic pixels (66 KR pixels across a tile at w = 1). Classic pixels are `1.5 / w` times KR pixels.

Origin + 22 legacy pixels to the right is the tile's bottom (south) vertex (inference from the rotations: the
origin is the centre + one map +y step = centre + (-√2/2, -√2/2), the bottom vertex is centre + (0, -√2/2)).
So, in classic screen pixels relative to the tile's bottom vertex `B` (screen y up), raised by `4 z`:

```
quad left   = B.x - 22 + x_off            (scale s: x_off * s / 1.5)
quad bottom = B.y + 4 z - y_off           (scale s: y_off * s / 1.5)
quad size   = floor(W * s) / 1.5  by  floor(H * s) / 1.5  classic pixels, W = x1 - x0, H = y1 - y0
```

Census of the legacy windows against this: `x_start = y_start = 0` in 36610 of 36611 records;
`x_off = 22 - floor(W / 2)` and `y_off = 0` in 33719 (92 %), the rest are authored shifts: `x_off` ±1..±4 off the
rule in about 3300 records, a few larger, and `y_off` -1 or -2 (raised 1 or 2 pixels) in about 170. With those defaults
the formula is the Classic client's rule: image left = tile centre x - floor(W / 2), image bottom = tile bottom
vertex, 4 pixels per Z.

The only difference from Classic pixel placement is the width/height rounding: for odd W (or H),
`floor(1.5 W) / 1.5 = W - 1/3`, so the quad is a third of a pixel narrower while the UVs still cover the full
W columns (a 1/(3W) horizontal squeeze).

## 6. The legacy setting (viewer +0x54)

Verified readers and writers: set to 0 in `0x0046f870` (also stores zoom 16) and by command case 0x47 in
`0x00476120` (`0x0047652c`); read by the descriptor path above, by `DrawWorldRenderGroups` `0x00470430` at
`0x00470830` (skips render group 7, effects/particles, and the next group when set) and by the hit test
`0x00471940` (vcall +0x3c). Not read by `SetCameraFrustum`, `CreateCamera`, `SetSpriteMapPosition`,
`BuildImageWindowQuad` or `BuildSpriteGeometry` (verified). So it changes which block (and pixel scale) a
record draws, not the camera or the tile size. No writer of a nonzero value was found (heuristic search of
byte stores after loads of the global).

Zoom: screen pixels per camera unit = viewport height / (2 · zoom · height fraction); one classic pixel maps
to one screen pixel when `zoom = 0.016070567 × window height` (about 17.4 at 1080 lines). Inference from the
frustum code; the zoom is clamped to 9..26 and defaults to 16.

## 7. TileArtLegacy images versus Classic art (census)

Block-1 item 0 of each static record against `artLegacyMUL.uop` of the Classic client, item path
`data\tileartlegacy\NNNNN.tga` -> `build/tileartlegacy/%08d.dds` in `LegacyTexture.uop`:

- Containers: DDS DXT5 36519, DXT1 1. Each image is the classic art pasted at (0, 0) on a power-of-two canvas
  (44×44 -> 64×64, 44×89 -> 64×128, 148×137 -> 256×256, 45×129 -> 64×256), and the legacy window is that
  rectangle. Same pixel grid, no upscaling.
- 36520 records with both images: window size equal to the Classic image in 33306, different in 3214 (other
  art revisions).
- Of the 33306: identical alpha mask in 28553 (colour equal at 8 bits 195, at 5 bits 32 more, otherwise DXT5
  colour error, typically 4..31 per channel); alpha mask different on at most 1 % of the pixels in 1533 and on
  more in 3220 (content revisions). Visible alpha outside the window in 856 records.
- Samples (legacy canvas / classic size / window): 1: 64×64 / 44×44 / (0,0,44,44,0,0); 2: 64×128 / 44×89;
  1993: 64×64 / 44×48; 3240: 256×256 / 148×137 / x_off -52 (= 22 - 74); 7070: 64×32 / 45×29; 18911:
  64×256 / 45×129. 6588 (scale 1.0): window 44×54 against classic 44×43, a different image.
- 3 block-1 items name world art (`data\worldart\..._catapult_a.tga`, `..._wall_torch.tga`); 91 records have
  no legacy image.

## 8. Formula for Classic art in an EC world

Given a Classic art image W×H for a static at map (x, y, z), to match the client's mode 0:

1. Pixel size: one Classic pixel = √2 / 44 camera-plane units = 1/44 of the tile diamond width. Next to KR
   art of `world_art_scale` w, a Classic pixel is `1.5 / w` KR pixels (1.5 at w = 1).
2. Quad: a camera-plane billboard `W × H` Classic pixels (exact client: `floor(1.5 W)/1.5 × floor(1.5 H)/1.5`).
3. Anchor: bottom edge on the tile's bottom vertex raised `4 z` pixels; left edge `floor(W / 2)` pixels left of
   the tile centre line. Equivalently in client terms, origin `(x + 0.5, -y - 1.5)` + `z · 4` pixels up, left
   `x_off`, bottom `-y_off`, with `x_off = 22 - floor(W / 2)`, `y_off = 0`.
4. When the EC record exists and its legacy window is `(0, 0, W, H)` of the same size, use its `x_off`,
   `y_off` and `legacy_art_scale` (pixel size `s / 1.5`) instead: 8 % of the records carry authored shifts.
   When the sizes differ (3214 records), the record's offsets belong to another image; use the defaults.
