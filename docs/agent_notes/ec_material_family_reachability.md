# Reachable EC item shader families

Use the original-package helper in `scripts/dev-helpers/ec-research/README.md`:
`ec_research --client <matching bundle> items --summary`. Keep source blocks
separate. The processed material parser includes `has_texture=1`; a raw nonzero
block marker is not sufficient. Blank raw blocks can otherwise inflate the
apparent number of unnamed EC shaders.

The inspected FP bundle has 40,426 tileart entries. Its EC block 0 contains:

| Shader | Material owners | Sampler-reference counts |
|---|---:|---|
| `UOSpriteShader` | 14,626 | 0: 14,625; 1: 11,563; 2: 347 |
| `UOStaticTerrainShader` | 341 | 0/1/2: 341 each; 3: 311 |
| `UOWaterShader` | 322 | 0/1/2: 322 each; 3: 190 |

Every processed block-0 material has texture references. None names
`UOEffectsShader` or an unknown family. This establishes the reachable family
set for that bundle, not complete composition parity. The 347 sprite source-2
references (300 distinct original paths) remain a separate authored-mask audit:
examples include item 2301's `20002301_axe_dual_short_mask.tga` and item 3336's
lily-pad mask family. Do not infer their operation from the filenames or apply
the static-terrain stage formula to them. The ordinary sprite path currently
does not bind these original extra maps; its procedural shading and hue rules
are not proof of source-mask parity.

Non-EC blocks remain distinct: the census finds 37 unnamed block-1 materials
with references, and sprite materials in blocks 1, 2 and 3. They must not be
counted as extra EC composition layers. These counts are bundle-specific;
rerun the census rather than freezing them as runtime constants.

## Executable navigation

The already analyzed UOSA.exe is the executable identified in
`ec_original_water_code.md`; addresses below are build-specific. The current
Gamebryo labels improve navigation, but register arguments still need assembly
cross-checks.

- `UOSpriteShaderHLSL` constructor `0x593ba0` installs vtable `0xcb425c`.
  Its setup `0x593fd0` calls `0x593b30` to load `UOSprite.psh` and `0x593ad0`
  to load `UOSprite.vsh`. `Shaders.uop` is present in the source bundle.
- Vtable slot `+0x1c` is `0x5940e0`. The bounded disassembly reads shader-map
  elements 0 and 2 and the base map through separate property offsets, binds
  them at object offsets `+0x120`, `+0x140`, `+0x160`, and sets presence values
  at `+0x180` and `+0x1a0`. It also clears render state 194 (SRGBWRITEENABLE).
  This proves separate optional inputs, not their pixel-composition formula.
- The effects constructors are separate: FFP `0x58e350`, HLSL `0x58ec40`.
  `0x41df40` is one procedural quad builder that selects `UOEffectsShader` and
  attaches texturing, alpha and vertex-color properties. Its decompilation has
  unresolved register/stack arguments; it does not establish a shoreline owner.

The effect archive inventory and NIF/EMS ownership investigation therefore
remain distinct from item sampler routing. No filename-driven foam/bubble
substitution or guessed effect-to-terrain route was added. Next source work
should follow the actual effect-definition/spawn graph or decode the named
sprite shader programs, preserving their optional-input semantics.
