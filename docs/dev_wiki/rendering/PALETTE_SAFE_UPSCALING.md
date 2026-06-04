# Palette-Safe Upscaling

Palette-safe upscaling is the sprite enlargement path used when preserving the
authored Ultima Online color vocabulary matters more than producing smooth RGB
interpolation. It is intended for small palette-constrained art such as Classic
items, humanoid animation frames, gumps, terrain texmaps, and EC-derived art
that is visually authored around small color ramps even after decoding to RGBA.

The goal is not just making images bigger. The goal is to keep silhouettes,
transparent edges, dither patterns, skin/cloth/metal ramps, and shadow colors
readable without letting filters invent arbitrary intermediate colors.

## Core Model

The pipeline treats these concepts separately:

- Source colors: canonical opaque colors collected from the decoded source image.
- Transparency: topology, not a color. Fully transparent pixels do not define
  palette entries and should not bleed hidden RGB into borders.
- Working colors: temporary colors produced by an upscale filter.
- Output colors: the final pixels after optional palette restriction.

Palette restriction can happen in two ways:

- As a separate pass, for example `bilinear2x` followed by
  `palette-snap-strict`.
- As a paired safe upscale, where the converter detects a filter immediately
  followed by a palette snap pass and runs the filter through the palette-safe
  wrapper.

The source palette is taken from the original decoded input for that asset, not
from later filtered output. This prevents repeated resampling from drifting the
effective palette over a chain of passes.

## Snap Modes

The converter exposes palette restriction as upscale pass values:

| Pass | Purpose |
| ---- | ------- |
| `palette-snap-strict` | Snap final opaque output pixels back to the original source colors. Use this when palette preservation is a hard requirement. |
| `palette-snap-ramp-aware` | Prefer snapping inside inferred/known color ramps when possible. Use this for sprites with coherent ramps such as skin, cloth, metal, and shadow groups. |
| `palette-snap-expanded-8` | Allow up to 8 deterministic derived colors in addition to the source palette. |
| `palette-snap-expanded-16` | Allow up to 16 deterministic derived colors. |
| `palette-snap-expanded-32` | Allow up to 32 deterministic derived colors. |

No snap pass is the comparison/debug mode. It lets the scaler produce ordinary
RGBA output and is useful for judging what the palette restriction is removing.

## Transparency Rules

Transparent pixels are not palette colors. The default transparency policy is
designed to avoid halos around sprite borders:

- Fully transparent pixels do not contribute their hidden RGB to the source
  palette.
- Palette snapping is applied to opaque output pixels.
- A snap pass by itself has scale factor 1 and should only change colors, not
  geometry.
- Border readability should come from opaque source colors, not blended hidden
  transparent RGB.

If a filter still creates unwanted fringe pixels, prefer `palette-snap-strict`
or `palette-snap-ramp-aware` immediately after that filter instead of snapping
only at the end of a long pass chain.

## Pass Ordering

Use palette snap as a normal repeated upscale pass. The order matters.

Strict preservation:

```text
--upscale-pass bilinear2x
--upscale-pass palette-snap-strict
```

Ramp-preserving edge cleanup:

```text
--upscale-pass xbr2x
--upscale-pass palette-snap-ramp-aware
```

More permissive terrain or UI output:

```text
--upscale-pass fsr-easu2x
--upscale-pass palette-snap-expanded-16
```

Debug comparison:

```text
--upscale-pass fsr-easu2x
```

When a normal filter is immediately followed by a palette snap pass, the
converter treats that pair as one palette-safe upscale operation. A standalone
palette snap pass is also valid and uses the current image dimensions unchanged.

## CLI Usage

The same pass names are accepted by the UOCF inspector preview copy action and
by `udd-pack` converter commands.

Classic art:

```bash
udd-pack pack-art \
  --ccdir /path/to/classic \
  --upscale-pass bilinear2x \
  --upscale-pass palette-snap-strict
```

Classic terrain texmaps:

```bash
udd-pack pack-texmaps \
  --ccdir /path/to/classic \
  --upscale-64-pass xbr2x \
  --upscale-64-pass palette-snap-ramp-aware \
  --upscale-128-pass fsr-easu2x \
  --upscale-128-pass palette-snap-expanded-16
```

EC art and land:

```bash
udd-pack pack-ec-textures \
  --ecdir /path/to/ec \
  --art-upscale-pass bilinear2x \
  --art-upscale-pass palette-snap-strict \
  --upscale-64-pass fsr-easu2x \
  --upscale-64-pass palette-snap-expanded-16
```

Gumps:

```bash
udd-pack pack-gumps \
  --ccdir /path/to/classic \
  --paperdoll-upscale-pass xbr2x \
  --paperdoll-upscale-pass palette-snap-ramp-aware \
  --single-upscale-pass bilinear2x \
  --single-upscale-pass palette-snap-strict
```

Mobile animations:

```bash
udd-pack pack-mobile-anims \
  --ccdir /path/to/classic \
  --upscale-pass xbr2x \
  --upscale-pass palette-snap-ramp-aware
```

## Upscale Profiles

Commands that support repeated upscale passes also support `--upscale-profile`
for per-asset-class defaults and per-id overrides.

Supported image types:

- `art_land`
- `art_items`
- `gumps_equip`
- `gumps_non_equip`
- `cc_mobile_animation_frames`
- `ec_mobile_animation_frames`
- `cc_land_textures`
- `ec_land_textures`

For mobile animation overrides, `family` is the body id and `id` is the source
frame index. Other overrides use the asset id as `id`.

TOML example:

```toml
[art_items]
passes = [
  { filter = "bilinear2x" },
  { filter = "palette-snap-strict" },
]

[gumps_equip]
passes = [
  { filter = "xbr2x" },
  { filter = "palette-snap-ramp-aware" },
]

[[overrides]]
image_type = "art_items"
id = 4000
passes = [
  { filter = "nearest2x" },
  { filter = "palette-snap-strict" },
]
```

KDL example:

```kdl
art_items {
    pass "bilinear2x"
    pass "palette-snap-strict"
}

cc_mobile_animation_frames {
    pass "xbr2x"
    pass "palette-snap-ramp-aware"
}

override type="cc_mobile_animation_frames" family=42 id=3 {
    pass "nearest2x"
    pass "palette-snap-strict"
}
```

Profiles may also use supported parameterized filter fields for non-palette
passes. Palette snap passes ignore filter parameter fields because their behavior
is defined by the snap mode itself.

## Recommended Defaults

Tiny humanoid sprites:

- Start with `xbr2x` or `bilinear2x`.
- Follow immediately with `palette-snap-ramp-aware`.
- Use `palette-snap-strict` for assets with important outlines or strong
  color-key borders.

Terrain and ground tiles:

- Start with `fsr-easu2x` or `xbr2x`.
- Use `palette-snap-expanded-16` when strict snapping makes gradients too blocky.
- Use strict snapping only for tiles with hard symbolic shapes or very limited
  palettes.

UI art and icons:

- Use `bilinear2x` or `xbr2x` followed by `palette-snap-strict`.
- Prefer strict output for icons with intentional flat fills and crisp edges.
- Use expanded snap only when the source already has soft antialiasing.

Heavily dithered assets:

- Do not treat checkerboards as noise by default.
- Prefer `palette-snap-strict` or `palette-snap-ramp-aware` after an edge-aware
  scaler.
- Avoid unrestricted smooth filters unless the output is only for comparison.

## Implementation Notes

The main implementation points are:

- `image-postprocess::palette` owns palette modeling, transparency policy, and
  snapping.
- `udd-conv::upscale_pipeline::UpscalePass` wraps raw `UpscaleFilter` values
  plus palette snap passes.
- `apply_upscale_passes` and `apply_upscale_passes_owned` are the converter entry
  points for mixed pass chains.
- Converter option structs store `Vec<UpscalePass>` for repeated passes.
- `udd-conv-cli` maps CLI values to converter passes and loads optional
  per-asset profiles.

The palette-safe path is deterministic. Avoid adding random dithering or
asset-dependent hidden constants to this path. Any future heuristic for color
similarity should stay palette-aware and should expose its tunables explicitly.

## Validation Checklist

For changes to this feature, keep the checks narrow unless the touched surface is
bigger:

```bash
cargo test -p image-postprocess palette::
cargo test -p udd-conv upscale_pipeline
cargo test -p udd-conv-cli palette_snap
cargo test -p udd-conv-cli upscale_profile_config
cargo check -p udd-conv-cli
```

Useful invariants:

- A strict snap output should contain no opaque colors outside the source
  palette.
- Transparent pixels should not introduce hidden RGB into opaque borders.
- A standalone palette snap pass should have scale factor 1.
- Repeating a strict snap pass should be idempotent for already-snapped output.
- CLI pass names copied from the inspector should parse in `udd-pack`.
