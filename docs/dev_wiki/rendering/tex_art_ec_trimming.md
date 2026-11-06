# EC Art Trimming

This note documents how the Enhanced Client static-art packer trims each art entry, and where the trim is recorded so that placement and source sampling stay right.

EC art atlases are always alpha-trimmed; there is no untrimmed mode. The active design direction is to keep the original large source texture intact and apply sampling windows at runtime, so this document should be treated as the description of current behavior, not the final target.

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

## Where the Trim Is Recorded

The trim is written in exactly two places, each once:

- **`tex_art_ec.uddp` slot draw offsets** (`draw_offset_x/draw_offset_y`) keep on-screen placement: the atlas stores only the non-empty visual bounds, and the offsets absorb the trim. The renderer treats EC billboard Y placement as bottom-anchored, so the correction uses the left trim for X and the bottom trim for Y.
- **`tilemeta.uddp` EC sampling starts** (`ec_start_x/ec_start_y`) are the trimmed image's origin in the source texture: the tileart start shifted by the top/left trim (the window start clamped to the texture, plus the trim). Consumers that crop a source-space texture at `ec_start` with the packed slot's extent get exactly the slot's pixels; the EC sprite hue masks depend on this. A tile the atlas did not pack from its `Texture.uop` texture keeps its tileart start.

`tilemeta.uddp` stores no draw offsets, so placement is never corrected twice.

The trims are recorded per art id, so an art id whose pixels are shared with another window (a rendered alias) still gets the origin of its own window.

## CLI Status

- The standalone cropped EC-art command has been removed.
- EC package creation goes through the shared `udd-cli pack pack-ec-textures` pass, which always trims.
- When `pack-ec-textures` also writes `--tilemeta-output`, tilemeta takes the trims the atlas pass recorded; nothing is decoded twice.
- A separate `udd-cli pack pack-tilemeta` recomputes the same trims from the EC sources, so it gives the same `ec_start` values. It therefore needs the sources the art packer needs: `tileart.uop`, `string_dictionary.uop`, `TerrainDefinition.uop` and `Texture.uop` or `LegacyTexture.uop`.
