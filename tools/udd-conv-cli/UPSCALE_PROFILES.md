# Upscale Profiles

Upscale profiles let `udd-pack` choose different ordered upscale/filter pass
chains for different asset classes during repacking. They are intended for cases
where one global `--upscale` or `--upscale-pass` chain is too coarse: art items,
land art, gumps, mobile frames, and land textures often need different
separation, color, palette, and local-contrast treatment.

Use a profile when you need repeatable repacks with targeted tuning, for example:

- increasing vibrance on desaturated mobile frames without changing land;
- adding local contrast to body frames whose limbs merge into the torso;
- keeping one conservative default for a whole image type;
- overriding a single problematic asset or animation frame by id.

## Supported Commands

The image packers that already support upscale passes also accept:

```text
--upscale-profile <file.toml|file.kdl>
```

This includes the Classic and Enhanced art, land texture, gump, and mobile
animation packers. The profile acts as a typed replacement or override for the
command-line pass chain only for targets it matches. If a target has no profile
entry, the command's normal `--upscale` and `--upscale-pass` settings remain the
fallback.

## Target Types

Profiles group defaults by image type. The supported type names are:

| Type | Meaning | Override id |
| ---- | ------- | ----------- |
| `art_land` | land entries from art packages | art tile id |
| `art_items` | item/static entries from art packages | art tile id |
| `gumps_equip` | paperdoll/equipment gumps | gump id |
| `gumps_non_equip` | other gumps | gump id |
| `cc_mobile_animation_frames` | Classic mobile animation frames | source frame index |
| `ec_mobile_animation_frames` | Enhanced mobile animation frames | source frame index |
| `cc_land_textures` | Classic texmap land textures | texmap id |
| `ec_land_textures` | Enhanced land textures | EC texture id |

Mobile animation overrides may also set `family`, which is the body id. For
non-mobile targets, omit `family`.

Type names are normalized when parsed, so `art_items`, `art-items`, and
`artItems` all resolve to the same type. The canonical names above should be
used in committed profile files.

## Resolution Order

For each image target, the packer chooses passes in this order:

1. The last matching override for `image_type`, `id`, and optional `family`.
2. The default pass list for the target's image type.
3. The command-line fallback passes from `--upscale` and `--upscale-pass`.

An explicit empty `passes` list is meaningful: it disables the fallback for that
matched target or type.

## TOML Format

Each image type can be a table with a `passes` array. Overrides are entries in
`[[overrides]]`.

```toml
[art_items]
passes = [
  { filter = "bilinear2x" },
  { filter = "vibrance30", factor = 0.35 },
  { filter = "local-laplacian-clarity25", radius = 3, amount = 0.25 },
]

[gumps_equip]
passes = [
  { filter = "nearest2x" },
  {
    filter = "unity-contrast-enhance35",
    intensity = 0.35,
    threshold = 0.08,
    blur_spread = 2.5,
  },
]

[[overrides]]
image_type = "art_items"
id = 4000
passes = [
  { filter = "nearest2x" },
  { filter = "saturation115", factor = 1.15 },
]

[[overrides]]
image_type = "cc_mobile_animation_frames"
family = 42
id = 3
passes = [
  { filter = "xbr2x" },
  { filter = "adaptive-log-contrast80", radius = 3, gamma = 0.8 },
]
```

`algorithm` is accepted as an alias for `filter`.

## KDL Format

In KDL, image type nodes contain `pass` children. Override nodes use properties
for the target selector.

```kdl
art_items {
    pass "bilinear2x"
    pass "vibrance30" factor=0.35
    pass "local-laplacian-clarity25" radius=3 amount=0.25
}

gumps_equip {
    pass "nearest2x"
    pass "unity-contrast-enhance35" intensity=0.35 threshold=0.08 blur_spread=2.5
}

override type="art_items" id=4000 {
    pass "nearest2x"
    pass "saturation115" factor=1.15
}

override type="cc_mobile_animation_frames" family=42 id=3 {
    pass "xbr2x"
    pass "adaptive-log-contrast80" radius=3 gamma=0.8
}
```

A KDL pass may also use `filter="..."` or `algorithm="..."` instead of the
first positional argument.

## Pass Ordering

Passes run sequentially in the order listed. Scalers change dimensions; 1x
filters keep dimensions and modify pixels in place.

Common patterns:

```text
xbr2x -> palette snap -> vibrance
nearest2x -> local laplacian clarity
bilinear2x -> contrast enhance -> adaptive log contrast
```

Keep aggressive local contrast and saturation late in the chain so they act on
the final sprite detail rather than being magnified by a later scaler.

## Tunable Parameters

If a pass has no custom parameters, it uses the built-in preset for that filter.
If any parameter is provided, the parser creates a parameterized pass. Custom
parameters are accepted only for filters that support them; adding `radius` or
`factor` to a pure scaler such as `xbr2x` is rejected.

| Filter family | Supported parameters | Notes |
| ------------- | -------------------- | ----- |
| `vibrance20`, `vibrance30`, `vibrance40` | `factor` | Prefer for desaturated sprites; typical range `0.2` to `0.4`. |
| `saturation115`, `saturation125`, `saturation130` | `factor` | Luma-based saturation multiplier; use conservatively. |
| `selective-warm20`, `selective-warm30`, `selective-warm40` | `factor` | Boosts warm hues, useful for skin/body separation. |
| `selective-green20`, `selective-green30`, `selective-green40` | `factor` | Boosts green hue ranges, useful for armor/foliage. |
| `local-laplacian-clarity15`, `local-laplacian-clarity25`, `local-laplacian-clarity30` | `radius`, `amount` | Good first choice for separating merged body parts. Radius is rounded to an integer. |
| `unity-contrast-enhance20`, `unity-contrast-enhance35`, `unity-contrast-enhance50` | `intensity`, `threshold`, `blur_spread` | Threshold prevents enhancement in flat regions. |
| `adaptive-log-contrast75`, `adaptive-log-contrast80`, `adaptive-log-contrast90` | `radius`, `gamma` | Useful for uneven lighting; lower gamma is stronger. |
| `unsharp-mask-small` | `radius`, `amount` | Simple sharpening, more direct than local clarity. |
| `high-pass-sharpen` | `radius`, `strength` | `amount` is accepted as a fallback alias for `strength`. |
| `scale-fx-smart-deblur` | `deblur_offset`, `deblur_strength`, `smart_deblur` | Deblur-oriented 1x pass for ScaleFX-like sources. |

The preset names encode conservative defaults. For example,
`local-laplacian-clarity25` defaults to radius `3` and amount `0.25`, and
`unity-contrast-enhance35` defaults to intensity `0.35`, threshold `0.08`, and
blur spread `2.5`.

## Palette Snap Passes

Palette snap passes are pass-chain entries, but they do not take numeric
parameters:

```text
palette-snap-strict
palette-snap-ramp-aware
palette-snap-expanded-8
palette-snap-expanded-16
palette-snap-expanded-32
```

They are useful after a scaler when a sprite should stay close to its source
palette. Because they are not numeric filters, do not attach fields such as
`factor` or `radius` to them.

## Filter Names

Profile filter names use the same value names as `--upscale-pass`. Use:

```text
udd-pack <command> --help
```

to see the complete accepted list for the current binary. The most relevant
profile-oriented filters are the color, local contrast, sharpening, and palette
passes listed above.

## Practical Starting Points

For body-part separation in low-contrast mobile sprites:

```toml
[cc_mobile_animation_frames]
passes = [
  { filter = "xbr2x" },
  { filter = "local-laplacian-clarity25", radius = 3, amount = 0.25 },
  { filter = "vibrance30", factor = 0.3 },
]
```

For conservative gump equipment enhancement:

```toml
[gumps_equip]
passes = [
  { filter = "nearest2x" },
  {
    filter = "unity-contrast-enhance20",
    intensity = 0.25,
    threshold = 0.08,
    blur_spread = 2.0,
  },
]
```

For a single problematic body/frame combination:

```toml
[[overrides]]
image_type = "cc_mobile_animation_frames"
family = 400
id = 12
passes = [
  { filter = "xbr2x" },
  { filter = "adaptive-log-contrast80", radius = 3, gamma = 0.8 },
  { filter = "selective-warm30", factor = 0.3 },
]
```
