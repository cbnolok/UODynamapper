# ghidra-kr-atmosphere

Traces how Kingdom Reborn (`UOKR.exe`) produces its dark, gloomy scene. The research covers:

- the light level from the server, the night and night-sight maps, the light sprites and the gamma option (the
  lighting post-process);
- the projected sprite shadows;
- the baked half-Lambert shade of the terrain mesh;
- what the client does **not** have: sun and ambient lights, fog, tonemapping, bloom, colour grading and
  vignette.

The same pieces are checked in the Enhanced Client (`UOSA.exe`).

Findings: [results/uokr_atmosphere.md](results/uokr_atmosphere.md). It has:

- the frame order;
- the formula, defaults and contribution of each ingredient;
- KR against EC;
- an item-by-item comparison with Dynamapper's KR-like preset;
- proposed changes ranked by visual impact.

Every result is marked **confirmed**, **probable** or **unknown**, with addresses. Research only: no rendering
code was changed.

## Programs

Both are in the Ghidra project `UOSA`. They were imported read only, never executed, and auto-analysed; both
were named with `--apply` and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/UOKR.exe` | `UO - Old KR/UOKR.exe` (2.53.0.2, changelist 108899) | `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43` |
| `/UOSA.exe` | `_Ultima Online Enhanced/UOSA.exe` | `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50` |

- The client folders are under `/mnt/dati/_proj_local/_uo_clients/`.
- Do not commit the binaries or anything extracted from them (UOP entries, decoded textures, UI files).
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
  - adds the tag `UO_ATMOSPHERE`;
  - sets a plate comment `NameAtmosphereCode: UO_ATMOSPHERE <name> [(probable)]: <evidence>`.

  Names of another source are kept.

Results of the 2026-11-07 runs ([UOKR](results/uokr_atmosphere_names.md), [UOSA](results/uosa_atmosphere_names.md)):

| Program | Functions named and tagged |
|---|---|
| UOKR.exe | 37 |
| UOSA.exe | 4 (the composite, its resource loader, the lighting shader stages and the terrain normal) |

There were no mismatches, kept names or conflicts. The second `--apply` reported `changes: none` on both
programs, and both were saved.

## Probes

All three are read only. They are kept here because GhidraMCP deletes inline scripts; the copies Ghidra runs
are in `~/ghidra_scripts/`.

| Script | Args | Use |
|---|---|---|
| `KrAtmoRtti20261107.java` | `OUTPUT_FILE NAME_REGEX MAX_SLOTS` | lists the MSVC RTTI classes matching a regex, with their vtables, slots and the functions that reference each vtable (the post-process effects, the shaders, the Gamebryo light, fog and material classes) |
| `KrAtmoFloats20261107.java` | `OUTPUT_FILE ADDRESS...` | prints 32-bit values as hex, float and int (the composite constants, the terrain light vector, the defaults) |
| `KrAtmoDecompile20261107.java` | `OUTPUT_FILE ENTRY...` | decompiles the functions at the given entries into a file; an address without a function is reported, never created |

The rest of the research used the GhidraMCP decompiler, disassembly, cross-references and byte search.

## kr_composite_table.py

```
python3 -I kr_composite_table.py NIGHTMAP_PNG NIGHTSIGHTMAP_PNG
```

- The inputs are previews of `TerrainTexture.uop` entries 21 and 22, decoded outside the repository.
- It prints Markdown tables:
  - light-buffer statistics;
  - the scene multiplier per server light level;
  - the gamma slider;
  - the baked terrain shade per slope;
  - Dynamapper's KR tonemap against the client's identity.
- It writes nothing.
- Its output is the source of the numeric tables in the results document.

## Scratch

Run outputs, decompiles, extracted UI files and texture previews are under
`/home/claudio/rust-target/uodynamapper-scratch/kr-atmosphere/`. No captures were taken.
