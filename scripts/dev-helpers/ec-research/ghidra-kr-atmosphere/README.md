# ghidra-kr-atmosphere

Traces how Kingdom Reborn (`UOKR.exe`) produces its dark, gloomy scene. The research had three passes.

The first pass covered:

- the light level from the server, the night and night-sight maps, the light sprites and the gamma option (the
  lighting post-process);
- the projected sprite shadows;
- the baked half-Lambert shade of the terrain mesh;
- what the client does **not** have: sun and ambient lights, fog, tonemapping, bloom, colour grading and
  vignette.

The second pass covered what makes KR look painterly, muddy and unevenly lit by day:

- the sprite shader (`UOSprite.vsh` / `UOSprite.psh`) and its world-anchored grunge map;
- texture filtering, mip levels and scale;
- the KR and EC terrain materials against Dynamapper's land shaders;
- weather, the circle of transparency, the sky library and the light-sprite batch;
- where either client uses normals;
- the reference screenshots, and side-by-side Dynamapper captures.

The third pass (Ghidra and data only) covered what the packages and a renderer need:

- the Enhanced Client against KR: shadows, mobiles, sprite shaders and EC-only effects;
- where each package item can come from: the grunge texture, the per-item grunge reference, the night maps, the
  light textures and the KR terrain textures, compared byte for byte between the two clients;
- the light sprites: texture, material, colour, scale and offsets;
- the exact geometry of the sprite and mobile shadows, the shadow buffer and the effect order.

The same pieces are checked in the Enhanced Client (`UOSA.exe`).

Findings: [results/uokr_atmosphere.md](results/uokr_atmosphere.md). It has:

- the summary and a list of what makes the KR look, with an estimated share for each ingredient;
- the frame order;
- the formula, defaults and contribution of each ingredient;
- KR against EC;
- one section per second-pass item (P2.1 to P2.7), with the corrections to the first pass marked;
- the third pass (P3.1 to P3.5): EC against KR, the data sources with a "what to pack from where" table, the light
  sprites, the shadow geometry with its formulas, and two tileart decoder notes;
- an item-by-item comparison with Dynamapper's KR-like preset;
- proposed changes ranked by visual impact.

Every result is marked **confirmed**, **probable** or **unknown**, with addresses. Research only: no rendering
code was changed.

## Programs

Both are in the Ghidra project `UOSA`. They were imported read only, never executed, and auto-analysed; both
were named with `--apply`. `UOSA.exe` is saved. The third-pass rename in `UOKR.exe` (`0x004387d0`) is not saved
yet: the GUI's MCP bridge stopped answering after the run.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/UOKR.exe` | `UO - Old KR/UOKR.exe` (2.53.0.2, changelist 108899) | `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43` |
| `/UOSA.exe` | `_Ultima Online Enhanced/UOSA.exe` | `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50` |

- The client folders are under `/mnt/dati/_proj_local/_uo_clients/`.
- Do not commit the binaries or anything extracted from them (UOP entries, decoded textures, UI files, shader
  sources).
- Ghidra projects stay outside the repository.

## NameAtmosphereCode.java

A GhidraScript, category `ECResearch`, built like `ghidra-paperdoll/NamePaperdollCode.java`.

```
NameAtmosphereCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="UOKR.exe", script_name="<abs path>/NameAtmosphereCode.java",
                  args="<scratch>/uokr_atmosphere_names.md --apply")
```

- It names fixed addresses, with one entry set per program above, selected by SHA-256. Any other program is
  refused.
- Every address is checked first. The checks are:
  - `calls:`: a call target or a global the function reads;
  - `calledby:`;
  - `ref:`: a vtable slot;
  - `string:`;
  - `const:`: an operand such as 0x1f = 31 light levels.

  A failed check is reported and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW.
- Without `--apply` the script only reports.
- With `--apply` it:
  - renames DEFAULT functions (source ANALYSIS) into `UOAtmosphere`, `UOSettings`,
    `UOLightingPostProcessEffect`, `UOShadowPostProcessEffect`, `UODeathPostProcessEffect`, the
    `UO*Shader` classes, `UOTerrain` and `PostProcessEffectsManager`;
  - second pass: also into `UOWorldViewer`, `UOSprite`, `UOSpriteShaderHLSL`, `UODefaultTerrainLayer`,
    `UOStaticTerrainShader`, `UOObjectComponentLighting`, `UORainEffect`, `UOSnowEffect`, `UOStencilEffect`,
    `figleaf` and (UOSA) `UOShaderLibrary`;
  - third pass: also into `UOShaderFilenameLight` and (UOSA) `UOAtmosphere`, `UOSettings`,
    `UOShadowPostProcessEffect`, `UOShadowShader`, `UOSpriteShaderShadow` and `UOFadePostProcessEffect`;
  - adds the tag `UO_ATMOSPHERE`;
  - sets a plate comment `NameAtmosphereCode: UO_ATMOSPHERE <name> [(probable)]: <evidence>`, and refreshes it
    when the evidence text changes.

  Names of another source are kept, except where an entry renames its own earlier name.

Results of the 2026-11-07 runs ([UOKR](results/uokr_atmosphere_names.md), [UOSA](results/uosa_atmosphere_names.md)):

| Program | Functions named and tagged |
|---|---|
| UOKR.exe | 63: 37 in the first pass, 25 in the second, 1 in the third (the light material) |
| UOSA.exe | 15: the composite, its resource loader, the lighting shader stages and the terrain normal; second pass, the shader library registration; third pass, 10 (the effect setup, the shadow effect and its shaders, the video defaults, the fade effect) |

There were no mismatches or conflicts. In the second pass, `0x005477d0` was renamed from the script's own
`SelectPermutation` to `BindSamplers`, the name UOSA already used. After the third-pass `--apply`, a report-only
run found every UOKR row applied. The UOKR save is still pending (see above).

## Probes

The first three are read only; the fourth runs headless on a read-only copy of the project. They are kept here
because GhidraMCP deletes inline scripts; the copies Ghidra runs are in `~/ghidra_scripts/`.

| Script | Args | Use |
|---|---|---|
| `KrAtmoRtti20261107.java` | `OUTPUT_FILE NAME_REGEX MAX_SLOTS` | lists the MSVC RTTI classes matching a regex, with their vtables, slots and the functions that reference each vtable (the post-process effects, the shaders, the Gamebryo light, fog and material classes) |
| `KrAtmoFloats20261107.java` | `OUTPUT_FILE ADDRESS...` | prints 32-bit values as hex, float and int (the composite constants, the terrain light vector, the defaults) |
| `KrAtmoDecompile20261107.java` | `OUTPUT_FILE ENTRY...` | decompiles the functions at the given entries into a file; an address without a function is reported, never created |
| `KrShadowHeadlessDecompile20261107.java` | `OUTPUT_FILE ADDRESS...` | third pass, while the GUI's MCP bridge was stuck: decompiles the functions at the given addresses, with their callers. It disassembles and creates a missing function in memory, so run it only with `-readOnly` on a copy of the project |

The rest of the research used the GhidraMCP decompiler, disassembly, cross-references, memory reads and byte
search. The third pass ran the headless probe like this, on a copy of the project in the scratch folder, plus
`objdump` for the assembly:

```
lowprio.sh analyzeHeadless <scratch>/gproj UOSA -process UOKR.exe -noanalysis -readOnly \
    -scriptPath ~/ghidra_scripts -postScript KrShadowHeadlessDecompile20261107.java <out.c> 440e50 ...
```

## Helper scripts

All are read only, print to standard output and write nothing, except `capture-rig/sheet.py`, which writes the
sheet into the scratch folder. Run Python that reads client data with `python3 -I`.

| Script | Usage | Use |
|---|---|---|
| `kr_composite_table.py` | `NIGHTMAP_PNG NIGHTSIGHTMAP_PNG` | the light-buffer statistics, the scene multiplier per server light level, the gamma slider, the baked terrain shade per slope, and Dynamapper's KR tonemap against the client's identity |
| `kr_reference_stats.py` | `docs/images/kr_reference` | colour statistics of the reference screenshots (mean, luma percentiles, blue/red ratio, chroma) |
| `kr_tileart_grunge_census.py` | `ITEMS_DUMP.txt` | counts which tileart sprites carry the grunge map (shader map 1), by block, shader and texture |
| `dds_info.py` | `FILE.dds|FILE.png ...` | DDS header facts (size, mip count, FourCC) and PNG luminance statistics |
| `uop_hash.py` | `PATH ...` | the UOP entry hash (Jenkins lookup3) of entry paths such as `build/worldart/01000045.dds`, to find unnamed extracted entries |
| `kr_ec_data_compare.py` | `SUBCOMMAND ...` (see its docstring) | third pass: KR against EC for `TerrainTexture.uop`, the light-texture colours, the terrain textures the TerrainDefinitions reference, the per-item grunge references, the tileart light fields and `Shaders.uop` |

- The inputs of `kr_composite_table.py` are previews of `TerrainTexture.uop` entries 21 and 22, decoded outside
  the repository. Its output is the source of the numeric tables in the results document.
- The items dump is the output of `cargo run -p uocf-devtools-cli --example ec_research -- --client <KR client>
  items`, written to the scratch folder. The `materials` command of the same example gave the KR and EC
  TerrainDefinition dumps.
- `kr_ec_data_compare.py` reads `uoppackage-cli extract --dictionary <client>/string_dictionary.uop` extractions of
  both clients in the scratch folder. Run it with `python3 -I -B`: `-I` ignores PYTHONDONTWRITEBYTECODE, and `-B`
  keeps its import of `uop_hash.py` from writing a `__pycache__` folder here.

## capture-rig

Dynamapper preview captures for the second pass, following the `renderer-reference-validation` and
`kde-x11-preview` rules.

| File | Use |
|---|---|
| `rig.py` | one isolated run: a private working directory with copies of `assets/` and `cache/`, the packages in `/home/claudio/test/uddp/`, the KR-like `kr.afternoon` preset overridden only in the run's cache copy of `land.toml`; variants `a` (as shipped), `b` (`CLIENT_TERMS_OFF`) and `c` (`CLIENT_SHADOW_STANDIN`) |
| `views.py` | `RUN VARIANT VIEW_SET`: starts a run, discards the first capture, teleports through the views and captures each one |
| `sheet.py` | the side-by-side sheet (rows are views, columns the variants) and per-capture colour statistics |

- Each run goes through `~/.codex/skills/kde-x11-preview`, which starts the binary on Desktop 4, verifies the
  exact PID and the desktop, and cleans up. The control commands go through `DYNAMAPPER_CONTROL_SOCKET`.
- Run every capture under the shared preview lock and at low priority:

  ```
  flock /home/claudio/rust-target/uodynamapper-scratch/preview.lock \
      /home/claudio/rust-target/uodynamapper-scratch/lowprio.sh python3 views.py sheet-a a sheet
  ```
- The binary is a copy of a debug build of this repository, placed by hand in the rig's `bin/` folder in the
  scratch folder. The user's saved settings are never read or written.

## Scratch

Run outputs, decompiles, extracted UI and shader files, texture previews, the tileart and material dumps, the
capture runs and the captures are under `/home/claudio/rust-target/uodynamapper-scratch/kr-atmosphere/`. The
sheet is `caps/kr_atmosphere_sheet.png` there. It is not committed because it shows game art.
