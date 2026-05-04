# EC Art Trimming

This note documents the cropped Enhanced Client static-art workflow and the matching tilemeta update rules.

The current packer trims each art entry after applying its sampling window. The active design direction is to keep the original large source texture intact and apply sampling windows at runtime, so this document should be treated as the description of current behavior, not the final target.

The EC source package split matters here:
- `Texture.uop` and `LegacyTexture.uop` are the shared decode pools.
- `tileart.uop` owns the per-entry art semantics and the sampling window boundaries used by this packer.
- `TerrainDefinition.uop` is the reason some source textures must be excluded from static art even when they look visually similar.

## Rules

- Cropping is per art entry, not per shared decoded source texture.
- The effective crop order is:
  1. apply the tileart sampling window from `start_x/start_y/end_x/end_y`
  2. alpha-trim inside that window
- Two art ids may share the same `texture_id` while sampling different windows from it, so trimming one window must not rewrite or constrain another window.
- That shared-source rule is why the current crop path exists, but it is also why the future full-source packing approach is attractive: it avoids scattering one large source across multiple atlas pages.
- In the current architecture, the crop path is a transitional way to preserve item ownership while still respecting shared raw EC texture pools; the target architecture is to keep those source textures intact and classify them once for all EC outputs.

## Why Data Is Not Lost

- Pixels outside an art entry's tileart sampling window were never reachable for that entry.
- After the window is applied, only fully transparent border rows and columns are trimmed.
- No opaque pixel inside the effective art image is discarded.

## Tilemeta Contract

- Raw `tilemeta.uddp` keeps the original EC `start_x/start_y/end_x/end_y` semantics from `tileart.uop`.
- Cropped EC art needs a matching cropped tilemeta package whose `ec_start_x/ec_start_y` are shifted by the recorded top/left crop delta.
- `ec_offset_x/ec_offset_y` stay unchanged. They are draw offsets, not crop offsets.

## CLI Flow

- `uddpack pack-ec-art-cropped --uddp-dir /path/to/packages` looks for a raw `tilemeta.uddp` in that directory.
- It writes a derived cropped tilemeta package by replacing only `metadata/items.bin` with values regenerated from raw source assets plus the EC crop adjustments.
- The derived output file name is the raw package stem plus `_ec_art_cropped`, for example `tilemeta_ec_art_cropped.uddp`.

## Repeated Test Conversions

- The raw tilemeta package is never edited in place.
- Every cropped conversion regenerates updated item metadata from raw client files and the raw tilemeta package, then rewrites the derived cropped package.
- Because the command always starts from the raw package and raw source metadata, repeated cropped-art conversions do not stack offset edits.

## Progress And Output

- Cropped EC art packing reports the number of cropped art slots.
- When `--uddp-dir` is used, the command also shows an `updating tilemeta` progress bar while rebuilding the derived `metadata/items.bin` payload.
- After rewriting the derived cropped tilemeta package, the command prints how many item offsets were updated and reminds the user that the raw tilemeta package was left untouched.

If a future branch removes cropped-source packing entirely, this file should be rewritten to describe the runtime sampling-window model instead of the current build-time crop path.
