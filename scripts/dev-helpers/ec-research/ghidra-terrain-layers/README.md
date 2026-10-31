# ghidra-terrain-layers

How does `UOSA.exe` (Enhanced Client) bind the texture items of a `TerrainDefinition.uop` record to its
terrain texture stages? Does any role, "primary" layer, repetition rule or "smooth" flag come from the data
or from file names, and what happens to a material with no definition? The repo's
`lib/uocf/src/enhanced/terrain_definition.rs` guessed layer roles from path substrings and a 4..8 stretch
range; this folder replaces those guesses with the client's rules.

Findings: [results/uosa_terrain_layers.md](results/uosa_terrain_layers.md) has the layer factory, the
per-class item → stage → role tables (default, bump, water), the verdicts on each heuristic, the primary and
stretch rules, the smooth question, the missing-definition path and the replacement rule.
[results/uosa_terrain_layers_names.md](results/uosa_terrain_layers_names.md) is the script report of the first
`--apply` (26 functions renamed, tagged and commented); a second `--apply` reports `changes: none`.

In short: the layer class comes from the shader name only; item i is map i + 1 (map 0 is the generated
coverage) and its role is fixed by position per class. Default and bump: item 0 and item 1 are blended by the
alpha of item 2 (item 0 alone on < 4-stage hardware); bump adds item 3 (luminance du/dv) and item 4
(reflection map). Water: item 0 du/dv, item 1 fallback colour, item 2 main colour. Stretch is a `1 / stretch`
UV scale. There is no smooth flag (softness is `coverage_exponent`), and a material without a definition makes
the mesh builder fault; classic land 774/777 go to material 64 in the client's csv.

Earlier work used here: `ghidra-terrain` (record format, `LoadTerrainLayer`, `BuildTerrainMesh`),
`ghidra-texture-roles` (`BindShaderMapToTextureStage`), `docs/agent_notes/ec_original_water_code.md` and
`docs/agent_notes/effect_visibility_validation.md`.

## NameTerrainLayerCode.java

GhidraScript, category `ECResearch`. Built on `ghidra-legacy-art-scale/NameLegacyArtScaleCode.java`, same rules:

```
NameTerrainLayerCode.java OUTPUT_FILE [--apply]
```

- Refuses any program whose SHA-256 is not the tested `UOSA.exe` (`5a5e9dc5...ebad50`).
- Checks each address first (vftable slots, callers, callees, strings, constants); a failed check is a
  mismatch and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW; pass spaces as `%20`.
- With `--apply`: renames DEFAULT functions (source ANALYSIS), adds the tag `UO_TERRAINLAYER`, sets the plate
  comment `NameTerrainLayerCode: UO_TERRAINLAYER: <evidence>` when empty or refreshes its own.

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameTerrainLayerCode.java",
                  args="<scratch>/terrain_layers_names.md --apply")
save_program(program="UOSA.exe")
```

## terrain_layers_census.rs

The data census (shader classes, items per position with stretch, which items the repo heuristics mark, which
item the repo's primary selection picks). It is a `src/bin` of a scratch crate outside the workspace, with
path dependencies on `lib/uocf` and `color-eyre`:

```
terrain_layers <EC client dir>
```

Probe scripts used for the tracing, kept outside the repo in `~/ghidra_scripts/`:
`ProbeTerrainLayers2026Vt.java` (dump labelled vftables), `ProbeTerrainLayers2026Names.java` (current names
and plate comments), `ProbeLegacyScale2026Decomp.java` (decompile to a file).
