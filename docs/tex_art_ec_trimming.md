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
- Historical cropped EC art used a matching tilemeta package whose `ec_start_x/ec_start_y` were shifted by the recorded source top/left crop delta.
- Atlas-packed EC art that is alpha-trimmed after the tileart sampling window should keep `ec_start_x/ec_start_y` unchanged and instead adjust the draw offsets by the visual trim delta.
- The renderer treats EC billboard Y placement as bottom-anchored, so the placement-preserving draw-offset correction uses the left trim for `ec_offset_x` and the bottom trim for `ec_offset_y`.
- `ec_offset_x/ec_offset_y` are draw offsets, so they are the correct place to preserve on-screen placement when the atlas stores only the non-empty visual bounds.

## CLI Status

- The standalone cropped EC-art command has been removed.
- EC package creation now goes through the shared `udd-pack pack-ec-textures` pass.
- `udd-pack pack-ec-textures --art-crop-transparent-bounds` enables atlas alpha trimming for EC art. When that command also writes `--tilemeta-output`, it applies the matching draw-offset adjustment automatically.
- If `tilemeta.uddp` is built separately for a trimmed EC-art atlas, use `udd-pack pack-tilemeta --ec-art-trimmed-draw-offsets`.
- The legacy `--ec-art-cropped`/`--tilemeta-ec-art-cropped` path is for historical source-cropped metadata that shifts EC sampling starts.
