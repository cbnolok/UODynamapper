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
references (300 distinct original paths) are authored hue masks:
examples include item 2301's `20002301_axe_dual_short_mask.tga` and item 3336's
lily-pad mask family. Do not infer their operation from the filenames or apply
the static-terrain stage formula to them. Their operation is established by the
original HLSL below, rather than the names.

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
should follow the actual effect-definition/spawn graph, preserving its optional
inputs rather than assigning effect images to unrelated terrain materials.

## Original sprite HLSL and hue coverage

`Shaders.uop` contains plaintext `data/shaders/uosprite.psh` and `.vsh`, not
only compiled programs. The production UOP name hash for the pixel source is
`0xa2e9963e6d9a7e50`. The unpacked EC source is also available under
`/mnt/dati/_proj_local/_uo_clients/ec_kr_extract/shaders_ec/`. The inspected
standard EC and FP shader archives have identical SHA-256
`11ec3cdb361812a321835dd898b3b042476149e0a5823be914db4cbe6b029920`.

The world sprite shader samples source 0 at the transformed sprite UV. When
the hue offset is nonzero, its **encoded red channel** selects a hue-strip
position over `255/1024` of the 1024-square hue atlas. The comment says
"luminance", but the expression reads red, without a luminance dot product.
Hue alpha multiplies sprite alpha. With `HAS_HUEMASK_TEX`, source-2 **alpha**
mixes the original sprite with that hue result; without it, the hue replaces
the whole sprite. The mask does not add terrain coverage, normal detail or foam.
It has no effect on an unhued sprite. Vertex RGB modulation is a separate
shader operation; our renderer's white instance color must not be mistaken
for proof of the original client's diffuse-color inputs.

The runtime art atlas is sRGB, while the original lookup index is encoded red.
The hue atlas already contains the original 256-pixel EC strips; it is not a
32-entry Classic palette. A faithful implementation must recover encoded red
before lookup and keep mask alpha linear. `UOSpriteUI.psh` has a different
256/1024 factor and must not replace the world shader's formula.

The census resolves 295 distinct numeric mask IDs. Of these, 272 are packed;
all 23 absent packed IDs are also absent from the matching original
`Texture.uop`. This does **not** demonstrate a converter omission. Across 347
owners, 312 original base/mask pairs are available and every pair has matching
native extents; 26 owners lack their original base and nine more lack the mask.
Do not invent KR substitutions for missing or malformed original references.
`inspect_ec_texture_extents` checks availability/extents without exporting pixels
and continues across missing IDs. Temporary inventory and source-pair CSVs live
under `/tmp/uodynamapper-ec-followup/` with `sprite-mask-` prefixes.

## Runtime sprite hue coverage

`texture_cache/ec_item_surface/hue_mask.rs` registers block-0 local-UV,
unit-repetition `UOSpriteShader` graphs only when their selected primary matches
the resident static art slot. Header flags distinguish this sprite graph from
the existing static-terrain graph. The runtime bundle registers 11,569 sprite
owners, 287 distinct native mask crops and nine unavailable declared masks.

Masks use the art window's `ec_start_x/y` and logical dimensions, undoing atlas
upscale. A shelf allocator packs those crops into additional layers of the
existing 256-square UNORM auxiliary array. Crops larger than a layer split into
blocks, with bounded explicit filtering across block boundaries. Source images
are decoded once during setup and shared crops reuse descriptors. No package
schema, GPU binding, draw pass or instance stride is added. This is startup
construction, not a per-frame `Assets::get_mut` operation.

Only an EC sprite instance with a nonzero enabled hue consults this graph; CC
and other material families retain their previous hue path. The sprite uses its
existing art filter, reconstructs encoded red for the palette index, then mixes
the hue result by native mask alpha. Mask alpha zero skips palette work. Missing
declared inputs retain the original sprite color and are counted at startup;
this is a conservative missing-resource policy, not a claim about the original
client's missing-texture fallback. An actually absent source-2 declaration hues
the full sprite as specified by the original shader.

`find_hued_statics` scans native statics without changing map data. Map 0 has
masked flowers 3211/3213 near (7038,391), with hues 40/30, and vines 3308/3309
near (7064,726), hue 1175. These are useful runtime fixtures. CPU coverage tests
check crop offsets, upscale and block placement; the production-shader GPU test
checks encoded-red indexing, fractional alpha, filtering across the 255/256 seam,
nearest sampling, full-sprite hues and declared-but-missing masks. These checks
establish the recovered hue operation, not complete original-client pixel parity.

The final native captures use `hue-gradient-final-` under the temporary evidence
directory, with a matching EC palette loaded, all three styles and CC controls.
The bridge and road controls differ from the prior hue path by at most one byte;
flower/vine changes remain on hued art. Earlier `sprite-hue-native-` captures had
no palette, and `sprite-hue-palette-settled-` used the defective solid swatch row;
neither is acceptance evidence. See [EC hue palette conversion](ec_hue_palette_conversion.md)
for the recovered two-row bitmap contract and independent 3,000-strip comparison.
