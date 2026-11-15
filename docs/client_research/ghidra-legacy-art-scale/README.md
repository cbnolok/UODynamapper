# ghidra-legacy-art-scale

How does `UOSA.exe` (Enhanced Client) size and place classic-style art (tileart block 1, `TileArtLegacy`)
next to EC world art (block 0)? The renderer wants to draw Classic art (`tex_art_cc`) in place of missing KR
art in an EC-style world, with the client's mode-0 sizing.

Findings: [results/uosa_legacy_art_scale.md](results/uosa_legacy_art_scale.md) has the constants with
addresses, the three descriptor modes, the quad and position math, the legacy setting, the census of the
legacy windows, the pixel comparison of `LegacyTexture.uop` with the Classic `artLegacyMUL.uop`, and the
resulting formula. [results/uosa_legacy_art_scale_names.md](results/uosa_legacy_art_scale_names.md) is the
script report of the first `--apply` (5 functions renamed, tagged and commented); a second `--apply` reports
`changes: none`.

In short: a source pixel is `scale × 0.021427423` camera units; at the default `legacy_art_scale` 1.5 that is
√2/44, one classic pixel of the 44-pixel tile diamond. TileArtLegacy images are the Classic art at 1:1 on a
power-of-two DXT5 canvas, and the legacy window offsets reproduce the Classic anchor
(`x_off = 22 - floor(W / 2)`, `y_off = 0` in 92 % of the records). The legacy setting does not touch the
camera or the tile size.

Earlier work used here: `ghidra-tileart` (record fields, `BuildSpriteDescriptor`),
`docs/agent_notes/ec_original_water_code.md` and `docs/agent_notes/ec_sprite_descriptor_geometry.md`.

## NameLegacyArtScaleCode.java

GhidraScript, category `ECResearch`. Built on `ghidra-texture-roles/NameTextureRoleCode.java`, same rules:

```
NameLegacyArtScaleCode.java OUTPUT_FILE [--apply]
```

- Refuses any program whose SHA-256 is not the tested `UOSA.exe` (`5a5e9dc5...ebad50`).
- Checks each address first (callers, callees, referenced float constants); a failed check is a mismatch and
  nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW; pass spaces as `%20`.
- With `--apply`: renames DEFAULT functions (source ANALYSIS), adds the tag `UO_LEGACYSCALE`, sets the plate
  comment `NameLegacyArtScaleCode: UO_LEGACYSCALE: <evidence>` when empty or refreshes its own.

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameLegacyArtScaleCode.java",
                  args="<scratch>/legacy_art_scale_names.md --apply")
save_program(program="UOSA.exe")
```

## legacy_art_census.rs

The data census (scale histograms, window/offset rule, TileArtLegacy versus Classic pixels), using the repo
readers. It is a `src/bin` of a scratch crate outside the workspace, with path dependencies on `lib/uocf`,
`color-eyre` and `image`:

```
census <EC client dir> <Classic client dir> [tile ids to print...]
```

Probe scripts used for the tracing, kept outside the repo: `~/ghidra_scripts/ProbeLegacyScale2026a.java`
(uses of viewer +0x54 / +0x80), `ProbeLegacyScale2026Decomp.java` (decompile to a file) and
`ProbeLegacyScale2026Refs.java` (callers, callees and float constants of a function).
