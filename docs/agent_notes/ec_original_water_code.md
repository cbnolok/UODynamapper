# Locating the original EC water drawing code

## Reproducible search

Reusable PE string/reference extraction and bounded Ghidra decompilation helpers
live in [scripts/dev-helpers/ec-research](../../scripts/dev-helpers/ec-research/README.md).
They accept input/output paths and keep generated evidence outside the repository.

Start with material identities from `TerrainDefinition.uop` and `tileart.uop`,
not a guessed function address. Material 5 names `UOWaterTerrainLayer`; shoreline
item 6038 names `UOWaterShader`. Their first three image identities agree:

| Source index | Image | Original role |
| --- | --- | --- |
| 0 | 1000013, `water_alpha` | Bump/environment displacement |
| 1 | 2000051, `water` | Hardware fallback surface |
| 2 | 1000017, `cube3` | Surface sampled after bump displacement |

`cube3` is an ordinary 256-by-256 2D DDS with no cubemap caps. Its filename alone
does not establish a cubemap or a scene reflection pass. Static water can add a
fourth item-specific image; this is a separate composition question.

The inspected file is the standard client's `UOSA.exe`, SHA-256
`5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.
Addresses below belong to that executable, not every EC build. Analysis is static;
the executable was neither run nor patched.

1. `strings -t x UOSA.exe` finds `UOWaterShader` at file offset `0x8a0538`,
   and `UOWaterTerrainLayer` at `0x8a08c4`.
2. `objdump -h UOSA.exe` gives PE section locations. `.rdata` starts at file
   offset `0x813000`, virtual address `0xc13000`. Convert an offset with
   `VA = section_VA + file_offset - section_file_offset`. The water string is
   therefore at `0xca0538`. The `+0x400000` shortcut happens to work for these
   sections in this file; it is not a general PE rule.
3. Search the executable's `.text` bytes for the little-endian four-byte address
   `38 05 ca 00`. Each match is a candidate reference, not yet a function entry.
   Convert its file offset using the `.text` section mapping, then disassemble
   a bounded surrounding region. Follow instruction boundaries from a known
   entry point: starting at an arbitrary byte can produce plausible nonsense.
4. A reference at `0x59b8e3` belongs to the constructor at `0x59b8d0`. It supplies
   the name to its base constructor and installs vtable `0xcb46bc`. The vtable
   slot at `+0xa0` points to `0x59bbd0`, a substantial drawing method.
5. Inspect that method's material texture loads, renderer-state setters and
   branches. Confirm the interpretation by following the shared texture binding
   function at `0x58dfc0` and decoding the Direct3D state constants.

For example, after establishing the entry point:

```sh
objdump -d -Mintel --start-address=0x59bbd0 --stop-address=0x59d004 UOSA.exe
```

The terrain-name reference at `0x4616e9` instead leads through the factory to
constructor `0x460190`, vtable `0xca0a5c`, and drawing method `0x5988a0` at
vtable `+4`. It sets up the same bump/environment operation. Terrain's internal
texture vector includes its terrain blend input, so its vector positions must
not be substituted blindly for the serialized material's source indices.

## What the item drawing method establishes

- Material-to-engine construction names its integer array `Technique` at
  `0x45db3b` and its floating-point array `Tweaks` at `0x45dbe6`, attaching
  them as extra data. The drawing method looks up those same names at
  `0x59bd0c` and `0x59bd76`. These are the semantic names for the reader's
  retained `unk8` and `unk9` arrays; preserve their raw values as well.
- Material source 0 is bound at `0x59be80`–`0x59be8d`. The color operation is
  `0x16` (22, `D3DTOP_BUMPENVMAP`) at `0x59bead`. Its bump matrix diagonal is
  1.0, with zero off-diagonal terms.
- Material source 2 is bound at `0x59c289`–`0x59c2a3`, after the bump stage.
  The default coordinate-generation state is `0x30000` at `0x59c3b9`,
  `D3DTSS_TCI_CAMERASPACEREFLECTIONVECTOR`. Technique parameters can select
  other coordinate modes. The item path modulates the sampled surface by
  diffuse lighting.
- The capability branch at `0x59be2a`/`0x59be48` sends unsupported hardware to
  `0x59c6b7`, which uses source 1. A fallback image is not evidence that the
  original normal rendering path blended sources 1 and 2 together.
- The optional fourth texture has its own stages beginning at `0x59c43c`.
  It must not be interpreted as generic solid-material detail.

State names were cross-checked against Microsoft's
[texture operations](https://learn.microsoft.com/en-us/windows/win32/direct3d9/d3dtextureop)
and [texture-coordinate generation](https://learn.microsoft.com/en-us/windows/win32/direct3d9/automatically-generated-texture-coordinates).

## Proof boundary and runtime contract

The source roles, bindings, fallback branch, and coordinate-generation state are
observed in code. They do not by themselves prove a complete modern shader
reimplementation. The texture transform and unsigned sampling follow-up below
corrects the former reflection-vector-to-2D guess. Exact camera/clock equivalence,
fourth-layer composition, and final original-client reflection appearance still
require separate verification. Source Technique[0]
selects explicit coordinates for lava/acid (1/2), or reflection coordinates for
water (0). Tweaks[0..2] provides the bump UV scroll velocity; material 5 contains
`[-0.01, 0.0]`, rather than a universal guessed water speed. The shared runtime
uses the terrain material velocity on both geometry owners; the original item
6038 has zero local Tweaks and does not establish the terrain clock behavior.

Representative original terrain parameters:

| Material | Technique[0] | Technique[1] | Tweaks[0..2], bump UV velocity |
| --- | ---: | ---: | --- |
| 5 water | 0 | 1 | -0.01, 0.0 |
| 10 lava | 1 | 2 | -0.0075, 0.0 |
| 49 acid | 2 | 1 | -0.012, 0.0012 |
| 53 stars | 0 | 2 | -0.02, -0.015 |
| 70 swamp | 0 | 3 | -0.005, 0.0025 |

Only the first technique and first two floating-point values currently drive
the shared sampler. Later values remain retained evidence for overlay work.

`tex_land_ec` now retains the original material entries in
`metadata/source_materials.json`. Runtime roles are
`[fallback/base, detail, mask, bump/normal, animated surface]`; source indices
remain a separate inspection API. Explicit overrides remain explicit policy.
Item water joins a terrain material only when its first three WorldArt sampler
identities match the original graph. A numeric item/land-ID collision is not
evidence of shared material ownership. Both geometry owners then call the same
liquid sampling/shading path, avoiding a visible boundary caused solely by
different land and static renderers.

## Texture transforms and unsigned bump sampling

The item material loader creates each extra source's transform at `0x45e527`–
`0x45e59f`. It sets zero translation/rotation, scale `1 / texture_repetition`,
pivot `(0.5, 0.5)` and transform mode 2. The first source has the same setup at
`0x45dfe2`–`0x45e099`. The texture binder calls the transform builder at
`0x692760`, then copies its affine rows through `0x58e0c0` into a D3D texture
matrix with `D3DTTFF_COUNT2`. The mode-2 branch starts at `0x69279a`.
For zero rotation, the resulting two coordinates are:

```text
u_out = u_in / repetition + translation_x
v_out = 1 - v_in / repetition - translation_y
```

There is no `reflection.xy * 0.5 + 0.5` remapping in this chain. The incoming
reflection vector must reach the authored affine transform directly; explicit
lava/acid coordinates use their own repetition through the same transform.
The bump source's scroll changes translation before this vertical reversal.
The source-2 draw path forces the engine address-mode bits to 3 at `0x59c28d`;
`0x58dfc0` passes those bits to `0x74d4e0`. This identifies the addressing path,
but the runtime-initialized enum lookup at `0xe16690` is not a static proof of
each final sampler value. Current shader sampling repeats the source image.

The original images 1000013 and 1000017 are unsigned DXT1 DDSs, not signed DuDv
formats; image 1000013's decoded red/green ranges are 107–181 and 109–178.
The inspected load/bind path supplies those texture resources without a signed
format request or a normal-decoding operation. With identity `BUMPENVMAT`, the
reimplementation therefore adds sampled unsigned RG directly. A normal-style
`RG * 2 - 1` expansion changes that displacement range. The earlier shader
incorrectly applied this expansion to all materials.

Microsoft's [bump formulas](https://learn.microsoft.com/en-us/windows/win32/direct3d9/bump-mapping-formulas)
specify that the sampled deltas pass through the matrix into the next stage;
the [signed bump pixel formats](https://learn.microsoft.com/en-us/windows/win32/direct3d9/bump-map-pixel-formats)
describe different resource formats. As a secondary API-semantics cross-check,
DXVK's [fixed-function fragment implementation](https://github.com/doitsujin/dxvk/blob/master/src/d3d9/shaders/d3d9_fixed_function_frag.glsl)
adds the sampled previous-stage XY through that matrix, without a blanket
unsigned-to-signed expansion. Its [vertex implementation](https://github.com/doitsujin/dxvk/blob/master/src/d3d9/shaders/d3d9_fixed_function_vert.vert)
also distinguishes raw reflection-vector generation from sphere-map remapping.
These corroborate API behavior; they do not supply EC material selection or
replace the original executable/asset evidence.

The shared shader now applies this transform and preserves unsigned bump samples
when original material metadata is present. Reviewed materials without that
source graph retain their existing normal decoding. The bridge capture recovers
bright environment-image structure that was absent from the earlier output;
this is a visible improvement, not a claim of pixel-exact reflection parity.

## Fourth-source operations traced, not yet implemented

The item draw branches on `Technique[1]` at `0x59c479`:

| Technique[1] | Fourth-stage RGB | Fourth-stage alpha | Code |
| --- | --- | --- | --- |
| 1 | current × texture | texture alpha | `0x59c59f`–`0x59c623` |
| 2 | texture × texture alpha + current × (1 − texture alpha) | current alpha | `0x59c506`–`0x59c58f` |
| other | current + texture | texture alpha | `0x59c489`–`0x59c623` |

The default also sets property-flags bit 0 and writes a time-dependent value to
the alpha property's byte at `+0x1a`, at `0x59c48f`–`0x59c4e3`. That write is
separate from texture-stage alpha; its property semantics and timing still need
to be traced rather than assumed to multiply the sampled color.
Tweaks[2..4] supplies fourth-source translation, through `0x59c44f`–`0x59c474`.
The low-capability fallback has corresponding branches starting at `0x59c8ec`.

The state setter at `0x74c260` preserves ECX/EDX, which matters when following
the final LEAs: `0x59c609` sets ALPHAOP (4) to SELECTARG1 (2), then `0x59c61c`–
`0x59c623` sets ALPHAARG1 (5) to TEXTURE (2). Do not infer alpha behavior from
the RGB operation or from untyped decompiler output.

Item 6038 has all-zero Technique/Tweaks, so its sparse-alpha fourth image is
added to RGB and supplies the static layer's alpha. This refines the earlier
unverified concern that simply masking it would create holes: alpha coverage is
indeed authored for this item path, but compositing also depends on the terrain
or other surfaces underneath. It is not established as blanket water coverage.
Terrain has an additional blend input and requires its own composition trace
before sharing this implementation blindly. Shoreline geometry/transition
coverage and fourth-image packing/UV windows remain separate unresolved work.

## Item composition information lost before runtime

`TileArtEntry::process` retains processed image refs, but `ArtData` has no field
for each block's shader name, Technique or Tweaks. `udd-conv::tilemeta` flattens
those processed image refs into `TileMetaItemTextureRef`; the original per-block
shader parameters consequently never reach `TileMetaPackage`. Source terrain
material metadata does not restore item-specific parameters.

The reusable `ec_research items 6038 6057` command reads the raw source blocks:
both block-0 shaders are `UOWaterShader`, both have all-zero Technique/Tweaks,
and both share source images 1000013/2000051/1000017. Their fourth images differ:
6038 uses image 6038 and 6057 uses 2100014. The first three refs have raw `unk7=1`,
while the fourth has `unk7=0`; retain that distinction while reconstructing UVs.
Each item also has separate legacy and enhanced sprite blocks; a matching numeric
image ID in those blocks is a different physical source.

`resolve_surface_like_tex_land_ec_slot_id` currently joins both items to the
same terrain payload using only their first three image identities. The resulting
`GroundTileInstance.material_payload` cannot distinguish their fourth inputs.
`create_aux_atlas_images` builds a terrain-oriented lookup from `TerrainMetaPackage`;
the art ground material does not bind that auxiliary atlas. Having an image in
`tex_aux_ec` is therefore insufficient to render its item-specific composition.

A complete repair needs original item material parameters retained through
conversion, a per-item fourth-source lookup with the correct physical package
and UV domain, and the original RGB/alpha operation in the ground path. Validate
the underlying terrain together with that alpha coverage. Do not replace these
missing inputs with filename-only role guesses or reuse the terrain's Technique
as the item's Technique.
