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
terrain-specific extra-layer composition, and final original-client reflection appearance still
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

The first technique and first two floating-point values drive the shared terrain
base sampler. Later terrain values remain retained for terrain overlay work;
item fourth stages use their own retained Technique[1] and Tweaks[2..4].

`tex_land_ec` now retains the original material entries in
`metadata/source_materials.json`. Runtime roles are
`[fallback/base, detail, mask, bump/normal, animated surface]`; source indices
remain a separate inspection API. Explicit overrides remain explicit policy.
Item water joins a terrain material through the source graph, using the full
three-image match or the parameter-checked active pair described below. A numeric item/land-ID collision is not
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
`0x58dfc0` passes those bits to `0x74d4e0`. The initialization at
`0x756b38`–`0x756b72` resolves the former unknown enum lookup: mode 0 maps
both axes to D3DTADDRESS_CLAMP (3), mode 3 maps both to D3DTADDRESS_WRAP (1).
The reflected source therefore repeats. Item masks with unit repetition clamp;
the material loader chooses wrapping for other repetition values or its explicit
geometry flag bit 3.

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

## Fourth-source operations

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
before sharing this implementation blindly. Item fourth-stage composition is
implemented below; exact camera geometry, terrain transitions and other families'
packing/UV windows remain separate validation work.

## Item composition retention and runtime selection

The former `TileArtEntry::process` kept image refs while dropping each block's
shader name, Technique and Tweaks. `ArtData.materials` now retains these as
`ArtMaterial` records with original block indices and the additional raw `unk1`
byte. The tilemeta converter writes `metadata/item_materials.json` separately
from the existing dense image-ref table. `TileMetaPackage::item_materials` and
`item_material` expose the retained source data. Missing or malformed material
metadata fails loading with an explicit rebuild message; rebuild packages with
the matching original bundle. Terrain parameters remain distinct from item data.

The reusable `ec_research items 6038 6057` command reads the raw source blocks:
both block-0 shaders are `UOWaterShader`, both have all-zero Technique/Tweaks,
and both share source images 1000013/2000051/1000017. Their fourth images differ:
6038 uses image 6038 and 6057 uses 2100014. The first three refs have raw `unk7=1`,
while the fourth has `unk7=0`; retain that distinction while reconstructing UVs.
Each item also has separate legacy and enhanced sprite blocks; a matching numeric
image ID in those blocks is a different physical source.

`resolve_surface_like_tex_land_ec_slot_id` joins both items to the same terrain
payload using their first three image identities. `GroundTileInstance` now also
carries the original item graphic, so that base sharing no longer loses the fourth
source. A separate two-record-per-item GPU lookup retains its atlas layer, true
image extent, original Technique[1], repetition, Tweaks[2..4], and UV-set index.
It covers the 16-bit item space independently of the 14-bit land lookup.

The fourth images already exist in `tex_aux_ec`: the installed 64×64 RGBA pixels
for 6038 and 2100014 match decoded original DDSs byte for byte. There was no
conversion repair to make for these fixtures. The shared auxiliary atlas now
collects item sources even without `terrain_meta.bin`; the art ground material
binds those same GPU image layers rather than allocating a duplicate atlas.
Only block-0 `UOWaterShader`, sampler 3, WorldArt/Texture references qualify.
Packed family mismatches are rejected instead of accepting a colliding sprite ID.

`ec_item_liquid.wesl` performs the traced RGB/alpha stage over the lit shared
liquid. Unit-repetition masks clamp to their actual image extent. Partial coverage
uses straight-alpha blending, without the non-water sprite path's alpha premultiply
or its 0.5 opaque threshold. An absent fourth source preserves the base surface.
Terrain's own extra input is deliberately separate and remains unfinished.

## Local UV construction is separate from the texture transform

`0x45e1e4`–`0x45e1f8` copies the raw source descriptor's UV-set field (`unk7`)
into the shader map's low flag byte. `0x58dfc0` passes that byte to the texture
coordinate-index state: cache field `+0xe8` corresponds to
`0xbc + D3DTSS_TEXCOORDINDEX(11) * 4`. This establishes a coordinate-set index,
not a generic Boolean or a filename-derived role.

The local coordinate set is already vertically reversed by the mesh builder.
The flagged planar/sprite quad in `0x45a080` writes `(0,1), (1,1), (0,0), (1,0)`
at `0x45a364`–`0x45a3c6`. The image-window builder `0x459b00` likewise writes
`1 - image_y / image_height`. The collector `0x45cd80` copies these four local
UV pairs, but adds item position to the separate world-plane pairs at
`0x45d7b2`–`0x45d8b0`. Consequently local V must reach the affine transform as
`1-v`; its second reversal returns the authored mask orientation. Passing the
world convention directly into the local path flips shoreline coverage.

This was cross-checked independently using the packed map's neighboring masks,
without a compatibility mapping or external reconstruction. For the 29×29-cell
coast fixture centered at map 0 `(1445,1790)`, mean neighboring alpha-edge
difference is 0.005164075 in the original image orientation versus 0.022726467
with vertical reversal. All eight rotations/reflections were compared; the
original orientation has the lowest discontinuity. This measures source coverage
continuity, not original-client pixel parity. The original-UV fix also removes the
repeating exposed wedges visible in the reversed-mask capture.

Reusable scene and mask inspectors plus `compare_mask_edges.py` reproduce this
check; see the research helper README. Original images remain intact.
The bridge and coast now show authored curved item coverage. Exact original
camera geometry, the default alpha-property threshold waveform, terrain blend
inputs, unusual item stacks and all effect-family appearance remain separate
proof boundaries.

## Following the analyzed texture loaders

The configured ghidra-mcp GUI project was queried through its read-only HTTP
endpoints with `program=UOSA.exe`, without changing program state or selection.
Metadata identifies the same standard-client path and x86 image base as above.
Auto-analysis and the Direct3D type archive improve navigation, but register-based
engine arguments still require assembly checks.

The Ex import thunk is `0xbd65f6`, through IAT slot `0xc13504`. Its actual call
site is `0xaecd2e` in `0xaecc3d`: a cached scratch-texture loader using explicit
width/height/format, one mip, zero usage and D3DPOOL_SCRATCH. Xrefs to the external
symbol also include the IAT data reference and thunk jump; those are not two
independent loader call sites.

The ordinary `D3DXCreateTextureFromFileInMemory` thunk is `0xb63ba8`, called at
`0x771c8d` in `0x771af0`. Its caller `0x770b90` belongs to the NiSourceTexture
path, and `0x774a80` handles NiSourceCubeMap. The helper reads the DDS info and
chooses ordinary, cube or volume D3DX loading by resource type. If pixel preference
4 is requested, it rejects inputs outside the signed bump-format list; it does
not silently reinterpret a DXT1 file as signed DuDv. The engine also has a
NiPixelData conversion/upload fallback when direct source loading is unavailable.
These are distinct paths; locating the Ex import alone does not establish the
world water texture's upload format or prove which runtime branch a resource took.

`retain_ec_item_materials` is a research example for preserving existing packed
metadata while adding material records from a freshly rebuilt source package.
It requires every source item's sampler identities, block/item indices,
repetition, raw sampler fields and logical/physical package axes to agree.
It refuses mismatches or an existing output file. The installed test tilemeta
matches the FP bundle; standard source disagrees for 30 items including candle
2600, hat 5907 and garland 8965. Both shoreline fixtures agree. Do not infer a
whole bundle match from those two fixtures or overwrite other packed tables.

## Active liquid sources can agree while fallback masks differ

The acid fixture at map 0 `(6512,464)` contains liquid border items including
11778, 11786, 11809 and 11816. Their block-0 `UOWaterShader` sources are:

| Sampler | Source | Repetition | UV set |
| --- | --- | ---: | ---: |
| 0 | 1000012, lava normal | 12 | 1 |
| 1 | item-specific image 1011778 / 1011786 / 1011809 / 1011816 | 1 | 0 |
| 2 | 2000490, acid surface | 8 | 1 |
| 3 | item-specific coverage image 11778 / 11786 / 11809 / 11816 | 1 | 0 |

The normal-capability branch at `0x59be80`–`0x59c2a3` uses samplers 0 and 2;
sampler 1 belongs to the fallback branch at `0x59c6b7`. Requiring sampler 1 to
equal terrain material 49's fallback 2000490 rejected these valid acid graphs.
The generic wet-item route then supplied ordinary blue water instead. This is a
runtime graph-selection defect, despite every needed source image being packed.

`source_liquid_payload_for_item` now accepts the active pair when both image IDs,
both repetitions, Technique[0] and Tweaks[0..2] agree with a source terrain
material. Acid uses `[2,1,0,0]` and `[-0.012,0.0012,0,0]`. Materials 168/169 have
the same images but different scroll `[-0.1,0.01]`, so image identities alone
would be insufficient. Only block-0 `UOWaterShader`, WorldArt, physical Texture
references qualify. The earlier full-three-image sharing path remains available
for ordinary water's zero local Tweaks; its clock equivalence remains unproven.
The item fourth stage still uses its own identity, coverage and parameters.

The same acid census found 54 instances of graphic 8612, whose source flags
include `NoDraw` (`0x200000000`). Its WorldArt image is 8600 `NO_DRAW`, with a
65×65 window. These are the literal black wedges with green "no draw" lettering
in the former capture. The converter already retains that high flag bit; runtime
art emission and visible-object picking now honor it. Dot-mode diagnostics and
the cell's full static inspection remain available. No graphic-number blacklist,
terrain fill or texture replacement was added.

The matching supplied references are `20261026_181306.png` (coast),
`20261026_181421.png` (lava), `20261026_181520.png` (acid) and
`20261026_181634.png` (bridge) under `docs/images/ec_reference/`. Their framing,
resolution and animation phase differ from native captures, so comparisons here
are visual content checks rather than registered pixel differences. The corrected
acid capture removes blue-water substitutions and diagnostic black wedges;
terrain blending, texture scale, lighting and exact geometry still differ.

## Terrain coverage is generated from neighboring material ownership

The analyzed terrain loader `0x461800` constructs the layer using `0x4616e0`,
then calls `0x461060` at `0x461845` before iterating serialized material sources.
It appends this generated source through layer-vtable `+0x10` at `0x461909`;
only afterward does the source loop call `0x4613b0` and append each source at
`0x461965`. Thus terrain vector index 0 is generated coverage, followed by
serialized sources 0/1/2 at vector indices 1/2/3. This explains the vector shift
in `0x5988a0` without treating a material image as the terrain blend mask.

`0x461060` builds a 32×32 ownership field by comparing each cell's material
identity with the requested layer. For positive blend parameters it also checks
the neighborhood at the chunk edges. It creates 32×32 pixel data in its later
allocation path and writes the same coverage byte to all four channels.
Interior membership yields 255; cardinal and diagonal neighbors can receive
separate intermediate bytes. The recovered byte algorithm and sampling state
are detailed below; renderer integration remains unfinished.

In the normal draw branch, `0x59925b`–`0x5993c2` binds vector index 0 after
bump/environment stages. The cached stage values are COLOROP=SELECTARG1,
COLORARG1=CURRENT, ALPHAOP=SELECTARG1, ALPHAARG1=TEXTURE. It preserves current RGB
and selects the generated coverage alpha. The zero/pass flag branch at
`0x59919c` instead disables that extra stage. This is terrain-layer composition,
separate from the item fourth-source rules; the current neighbor-color heuristic
does not implement this ownership/coverage mechanism.

The read-only Ghidra evidence can be reproduced with `ghidra_query.py` in the
research helpers. Decompiled register arguments are imperfect here; the append
order and stage constants above were checked against x86 disassembly. The
original executable and Ghidra program were not modified.

### Coverage exponent, edge tests and byte conversion

Definition reader `0x845c25` reads name, material ID, then three float fields.
The stores at `0x845cde`–`0x845cf5` put the ID at object `+0x30` and the floats at
`+0x34/+0x38/+0x3c`. Loader `0x461800` passes `+0x38` to `0x461060`. Therefore
`TerrainDefinitionEntry.unk2`, already retained in `source_materials.json`, is
the terrain coverage exponent. It is independent of texture repetition and
the shader's `Tweaks` vector.

For positive exponent `b`, the coverage bytes are:

| Membership of the requested material | Byte |
| --- | --- |
| Current ownership cell | 255 |
| At least one cardinal ownership neighbor | trunc(255 × 0.949999988079071^b) |
| Otherwise, at least one diagonal ownership neighbor | trunc(255 × 0.6000000238418579^b) |
| Otherwise | 0 |

The constants are doubles at `0xd05278` and `0xd05270`, promoted from float
values; multiplier `0xd04ef0` is float 255. `0xbd6860` dispatches the power
operation through its x87 or SSE implementation. Its x87 path at `0xbd690a`
uses `fyl2x` followed by exponentiation. Both integer conversions in `0x461060`
explicitly set x87 rounding control to truncation (`OR 0xc00`, `fistp`), rather
than the nearest rounding suggested by the decompiler's `ROUND` label.
Nonpositive exponents produce zero neighbor bytes, retaining binary ownership.
The generator replicates each result into all four RGBA channels.

There are two distinct boundary rules. For positive `b`, a cell on a chunk edge
counts as owned only if its immediately adjacent out-of-chunk neighbor is also
owned; at a corner all three exterior neighbors must match. The subsequent
neighbor expansion reads the resulting 32×32 boolean field. Horizontal checks
require `0 < x < 31`, vertical checks require `0 < y < 31`, and diagonal checks
require both. The implementation does not independently clamp missing neighbors
or perform ordinary image dilation across chunk edges. A positive cardinal byte
takes precedence over a diagonal one.

Observed original definitions give water material 5 `b=0`, acid 49 `b=1`, lava
10 `b=5`, and swamp 70 `b=0.15`. Their cardinal/diagonal bytes are respectively
0/0, 242/153, 197/19 and 253/236. The sparse acid variants 168/169 use `b=5`.
Matching liquid images alone does not make two terrain layers equivalent.

### Layer order and coverage coordinates

Terrain builder `0x461ae0` appends material layers, then sorts their records
using comparator `0x5996d0` (call at `0x462993`). The comparator calls each layer's
vtable `+0x08` and performs a signed ascending comparison. Getter `0x4600d0`
returns layer object `+0x04`. The normal/default/bump setter `0x4600e0` stores
the material ID directly; the water setter `0x4601e0` multiplies it by 1000.
Loader `0x46181f`–`0x46182f` supplies the definition's actual material ID to that
virtual setter. Sorting by raw ID, water flags or texture identity is incorrect.

The shader RTTI identifies `UOTerrainShader`'s vtable at `0xcb4534`.
The navigation is reproducible without guessing instruction addresses: find the
MSVC type-descriptor names `.?AVUOTerrainDefinitionBinaryFactory@@` and
`.?AVUOTerrainShader@@`, then follow data references through their complete-object
locators to the vtable pointers. The factory type descriptor at `0xde6d3c` leads
to vtable `0xca5e14`; its `+0x04` method `0x845b04` calls reader `0x845c25`.
The shader descriptor at `0xdeeea8` leads to `0xcb4534`; its `+0x1c` and `+0xa0`
methods identify the pass-count and per-pass layer drivers. Cross-check the
decompiler's containing function entry before disassembling an interior address.

`0x596c80` derives its pass count from the sorted property vector, capped at 49.
`0x596ce0` selects the current vector entry using shader object `+0x38` and passes
that same pass index as the second stack argument to the layer draw function.
This resolves the earlier zero/pass ambiguity: `0x59919c` tests the actual pass
index. Water at pass zero bypasses coverage; later water passes select generated
coverage alpha. Default high-capability body `0x5974f0` tests the forwarded pass
at `0x5979f0`: its final ALPHAOP is SELECTARG1, with ALPHAARG1=DIFFUSE (0) for
pass zero and TEXTURE (2) otherwise. Low-capability body `0x597b50` uses the same
selection. Their caller `0x597210` forwards the pass index through vtable `+0x40`
or `+0x44`. This establishes an unmasked first base for default and water layers;
the complete DOT3/bump material response remains a separate fidelity question.

World texture coordinates and coverage coordinates are separate. The geometry
builder writes world `(world_x, -world_y)` to one stream, and positive chunk-local
`(x/32, y/32)` to the other at `0x461fde`–`0x4620f3`. The generated map's low flag
byte is 1, consumed by `0x58dfc0` as `D3DTSS_TEXCOORDINDEX`; it has no authored
texture transform. Its address-mode bits are zero, selecting CLAMP on both axes.
Loader `0x4618d7`–`0x4618fd` sets filter index zero for `b==0` and one otherwise.
Binder `0x58e021` forwards this index to `0x74d530`; initializer `0x756990` writes
filter triples at `0xe16648` / `0xe16654`: point/point/none and linear/linear/none.
State cache `0x74c390` uses zero-based slots; the flush at `0x74c6df` maps them
through `0xda8658` before `IDirect3DDevice9::SetSamplerState` (vtable `+0x114`).
The table maps slots 4/5/6 to D3DSAMP_MAGFILTER/MINFILTER/MIPFILTER (5/6/7),
avoiding an off-by-one interpretation of the internal indices.
Thus positive-exponent coverage uses bilinear filtering of the 32×32 mask, not
an arbitrary per-edge smoothstep. No half-texel offset is added to the chunk-local
mesh coordinates. This is coverage sampling evidence, not proof of all camera,
rasterization or color-space equivalence.

### Reproduce the coverage model independently of the renderer

`inspect_ec_terrain_coverage` exports a 34×34 material-ownership grid (the chunk
plus its border), land IDs and the preserved source definitions. It uses the
packed source/embedded routing; `--routing` applies an explicitly selected loose
`TerrainTranscode.kdl`. It refuses an unresolved cell, missing source definition,
facet-edge fixture without a complete border, or existing output. This provenance
is not a claim that every manual routing override is original-client truth.

`scripts/dev-helpers/ec-research/terrain_coverage.py` reconstructs the generator's
raw bytes, writes one RGBA dump and 32×32 PNG per material, and reports sort keys,
weights and byte histograms. It does not composite a fabricated client screenshot
or feed a heuristic mask back into the renderer. Its fixtures cover cardinal vs
diagonal weights, truncation, binary ownership, exterior corner requirements,
the both-neighbors-in-bounds rule, and water's distinct sorting key.

The installed map-0 chunks containing lava `[5744,2928]`, acid `[6512,464]`, swamp
`[1872,2416]` and the bridge `[2834,987]` exported successfully: 5, 3, 15 and 9
interior terrain layers respectively. In particular, swamp cannot be represented
by a compositor that silently keeps only a handful of neighboring materials.
The four liquid masks were inspected. Those diagnostic exports are reconstructed
coverage evidence, separate from the runtime integration and native captures below.

### Runtime ordered terrain composition

Source-backed EC terrain now uses the recovered coverage model. GPU land lookup
role 6 stores material ID, signed sorting key, cardinal/diagonal bytes and flags
for source class, positive exponent, reviewed terrain policies and filter mode.
The six existing image/liquid roles keep their indices. A small Rg16Uint image
stores the first and last retained material payload for each 32×32 chunk. CPU
preparation uses the full cached chunk, collapses aliases by material identity,
and retains the first 49 sorted passes, matching `0x596c80`. Source switches and
map changes retain the ordinary texture path; the base image clears on a map
change. It is updated only when its contents change.

`land/coverage.wesl` reconstructs the ownership bytes from the paged map metadata.
The bilinear mask footprint plus its one-cell ownership expansion touches at most
4×4 source cells. Every distinct material there is retained, together with the
chunk's first base even when that base owns no nearby cell. The resulting bound
of 17 local records is derived from the sampling footprint; it does not keep
only the first few chunk materials. The CPU's last retained key also excludes
materials beyond the original 49-pass cap.

Each contributing layer uses the existing material sampler and its own wet/solid
lighting response. Sorted straight-alpha composition operates on those shaded
colors; global lighting, grading and tone mapping run on the composed result.
An opaque later pass can skip all earlier shading. Coverage stays unperturbed by
water animation and uses the original point/bilinear selection independently of
the albedo filtering control. Missing EC textures remain explicit. The previous
neighbor-color transition is used only while source coverage/base metadata is
unavailable; it is bypassed for the prepared source-backed composition path,
including liquid terrain.

Renderer tests cover weight truncation, chunk sorting/alias selection, the 49-pass
cap, map-base invalidation and WESL composition. Native checks cover the bridge
in all three presets, paused water, EC coast/town, Classic coast and lava/acid/swamp
on Desktop 4. Lava banks now receive terrain coverage instead of stopping at the
former hard ownership boundary. Reflection/camera/clock parity, source albedo
filtering and several static geometry/effect differences remain unfinished;
ordered masks do not establish
complete original-client visual parity.

### Signed world coordinates

The original terrain mesh builder `0x461ae0` appends `(map_x, -map_y)` to texture
stream 0 at `0x461e80`–`0x461ed5`, then the other three cell corners through
`0x461fdd`. Stream 1 remains the positive chunk-local coverage domain above.
The sign is explicit subtraction from zero, not a DDS image flip. Terrain chunk
placement `0x462fb0` also stores negative map Y in its geometry origin at
`0x46308a`–`0x4630ac`; the origin's half-unit adjustment belongs to geometry,
separately from stream 0's integer map coordinates. The decompiler incorrectly
types these origin floats as integers; the MOVSS stores establish their types.

The item path confirms the same signed axes independently. Position setter
`0x4598d0` retains integer map X/Y at object `+0x4c/+0x50` and forms geometry
translation X/Z from positive X and negative Y at `0x45999b`–`0x4599e7`.
Its camera/height displacement and geometry-anchor offsets are additional terms.
Image-window builder `0x459b00` creates local UVs and separate world-plane pairs;
at `0x459f2d`–`0x45a058` each world pair takes local geometry X minus 0.5 and
Z plus 0.5. Collector `0x45cd80` adds the item's X/Z translation to those pairs
at `0x45d7b2`–`0x45d8b0`. The alternate flagged-quad adjustment `0x45a930`
likewise adds integer X and **negative** integer Y at `0x45a969`–`0x45a99f`.
Neither world path uses the local mask's `1-v` convention. These observations
establish the signed domain; they do not establish complete parity of item
geometry, anchors or height projection in our simpler grid quads.

Bevy surface positions use positive map Y as world Z. The shared liquid sampler
now converts that input to `(x, -z)` before applying the recovered mode-2 affine
transform to original world-indexed bump and surface sources.
Terrain supplies its signed world stream for Technique 2; items supply their
local stream as described below. Item fourth stages selecting world UV set 1
use the same signed-world conversion. Previously, passing positive map Y directly
into the affine V reversal mirrored their spatial
sampling. For repetition `r` and translation `(tx,ty)`, the corrected wrapped
world texture domain is `(x/r + tx, y/r - ty)`, since the affine's constant 1
vanishes under wrapping. Ordinary solid terrain already samples positive
map coordinates and does not need this additional reversal.

Reflection-vector XY still enters the affine transform directly, and local
shoreline masks still enter as `(u,1-v)`. Generated terrain coverage remains
positive chunk-local/32. Keep these domains distinct when investigating
remaining reflection/camera, filtering and authored geometry differences.

### Technique 2 selects an owner-specific stream

The item draw's jump table at `0x59d008`, indexed by `Technique[0]-1` at
`0x59c2a8`–`0x59c2bb`, points Technique 1 to `0x59c2c2` and Technique 2 to
`0x59c322`. The first retains the source map's low-byte UV-set index; the second
sets TEXCOORDINDEX to zero, explicitly at `0x59c334` and its cache-update paths.
Terrain's corresponding table at `0x5996b4` dispatches to `0x598f01` and
`0x598f20`, with the zero write at `0x598f2c`. This distinction is visible in
assembly even where the decompiler simplifies the state-cache branches.

| Input | Terrain mesh | Item ground mesh |
| --- | --- | --- |
| Explicit stream 0, selected by Technique 2 | Signed world `(x,-y)` | Local art `(u,1-v)` |
| World stream used by the retained liquid sources | Signed world `(x,-y)` | Signed world plane, separately constructed from local UVs |
| Generated terrain coverage | Positive chunk-local `(x/32,y/32)` | Not an item input |
| Reflection generation | Camera-space reflection-vector XY | Camera-space reflection-vector XY |

The previous shared liquid sampler lost this distinction and fed world
coordinates to both explicit techniques on both owners. The sampler now accepts
the owner's original stream 0 separately: land passes signed world coordinates,
and item ground art passes its vertically reversed local UV. Technique 1 and
the bump source continue using world coordinates; Technique 0 continues using
the reflection vector. The optional fourth stage retains its independent
source UV-set selection.

This path is exercised by acid border item 11778, whose original block-0
Technique is `[2,1,0,0]`: source 0 is world-indexed bump 1000012/repetition 12,
source 2 is image 2000490/repetition 8, and source 3 is the local unit-repetition
mask. Sharing its active image graph with terrain material 49 must not replace
the item's stream-0 coordinates with terrain's. This corrects sampling ownership;
full source geometry/anchor and reflection/filtering parity remain separate.

## Liquid filtering and original sampler state

`0x58e021` extracts `(map_flags >> 8) & 0xf` and passes that filter index to
`0x74d530`. The previously recovered table maps index 0 to point/point/no-mip
and index 1 to linear/linear/no-mip. Source maps need their own trace, separately
from generated terrain coverage:

- Terrain source-map loader `0x4613b0` initializes flags with `0x3100` at
  `0x4613dd`, then applies `(flags & 0xf1ff) | 0x3100` at `0x461590`–`0x46159c`.
  The resulting filter nibble is 1, with WRAP addressing. These are the maps
  appended after generated coverage by `0x461800`.
- The item's first map retains a flag-dependent filter. `0x45df69`–`0x45dfba`
  tests low-word masks `0x10000000`, `0x200`, `0x10` and high-word mask `0x20`;
  `0x45dfaf`–`0x45dfc2` writes filter 1 when any is set, otherwise 0.
- Extra item maps are initialized with `0x3100`, but the authored-source branch
  then clears the filter nibble with `AND ECX,0xf0ff` at `0x45e231`. Their
  source UV index and repetition-dependent address mode remain separate fields.
  Do not infer their final filter from the constructor alone.

These source choices are not one universal original liquid filter. The runtime's
existing user filtering toggle remains explicit policy; a future faithful
per-stage filtering mode must preserve the distinctions above rather than
silently enabling linear filtering for every original source.

### Shared runtime filter implementation

Terrain's sampler is explicitly linear in `land/texture_array.rs`; art atlas
images inherit the application-wide `ImagePlugin::default_nearest()`. The former
liquid `textureSample` branch consequently filtered terrain but kept item liquid
nearest even when `enable_linear_filtering` was enabled. Clamping the sample
center into a packed slot also prevented bilinear interpolation across a tiled
source's last/first texels. Finally, recovering encoded bump RGB **after** sRGB
interpolation changed the authored offsets between texel centers.

`ec_liquid_sample` now applies the same point/bilinear policy on both owners.
Art ground binds a linear sampler from the existing land-page image at binding
116; both real and blank page images declare that sampler explicitly. Terrain
already supplies its linear sampler. Color footprints inside a slot, and clamped
color sources, use one hardware-filtered sample at mip zero. Wrapped border
footprints use four explicit taps, wrapping each within the source extent before
adding the packed origin. Tiled sources consequently blend opposite edges while
unit item masks keep CLAMP, without sampling neighboring packed images.

Bump/data filtering uses four explicit taps to recover encoded RGB individually
**before** interpolation; alpha stays linear. Color taps retain their decoded
linear RGB. Nearest sampling still uses one load. Keeping native color filtering
for the common interior case avoids paying four color loads across every liquid
fragment. No duplicate atlas is allocated. Existing preset filtering choices
and the non-liquid art/terrain samplers are unchanged.

The headless `liquid_sampling_gpu_preserves_data_and_source_edges` regression
executes the actual composed WESL on an sRGB array texture. Contrasting source
values distinguish color/data interpolation, and magenta guard pixels plus an
unused array layer detect slot/layer leakage. It covers wrap, clamp, negative
coordinates, nearest sampling, alpha and a single-texel source. Run explicitly
with `cargo test -p dynamapper --all-features --locked liquid_sampling_gpu_preserves_data_and_source_edges -- --ignored`.
It is ignored in the ordinary suite because it requires a Vulkan adapter; it
does not open a window. Other support-map samplers still require their own data
filtering review. This repair does not establish camera equivalence, original
runtime upload-format or complete original-client pixel parity.

For later serialized geometry/property checks, the user identified
[nifly](https://github.com/ousnius/nifly) and
[NifSkope](https://github.com/niftools/nifskope). These are inspection aids;
support for this client's exact NIF version and custom blocks must be established
on actual assets. They were not required to diagnose the source selection and
NoDraw defects, and no NIF dependency was added to the renderer.

## Reflection generation and missing vertex normals

The `0x30000` coordinate-generation flag describes an operation on **vertices**,
not a requirement to recompute a reflected ray in the fragment shader. The
original water path uses the fixed-function vertex pipeline:

- `UOWaterShader` installs vtable `0xcb46bc`. Its inherited pass entry at `+0x28`
  is `0x749ea0`, which dispatches through render-pass vtable slot `+4`.
  The pass vtable at `0xcd396c` resolves that slot to `0x7453d0`.
- The pass constructor at `0x746290` clears its shader fields. The water
  constructor `0x59b8d0` acquires a pass through `0x745170` and attaches texture
  stages, without installing a vertex program. The pass's absent-program branch
  at `0x7455cf`–`0x7455f3` clears the cached vertex shader and calls device
  vtable `+0x170`, `IDirect3DDevice9::SetVertexShader(NULL)`. It then selects the
  geometry FVF/declaration through `0x58e230` or `0x58e270`.
- Terrain mesh construction calls `0x688710` at `0x4625c4` with its second
  stack argument zero (`XOR EDI,EDI` at `0x4625b4`, `PUSH EDI` at `0x4625bf`).
  That constructor stores this argument into geometry data `+0x24`, the normal
  pointer. Position/color/UV streams are separate arguments.
- Item collector `0x45cd80` creates geometry through `0x45ccc0` → `0x66c260` →
  `0x67f7e0`. The last constructor also passes zero (`PUSH 0` at `0x67f7f2`)
  as the normal argument to
  `0x688710`. Its body does not allocate a replacement normal stream.
- Buffer-format builder `0x76d1b0` adds `D3DFVF_NORMAL` (`0x10`) only when
  geometry data `+0x24` is nonzero. Buffer upload `0x76d270` likewise copies
  normals only when present. There is no fabricated upward normal in these
  traced mesh paths. Do not confuse the CPU normals used for terrain vertex
  colors with an uploaded normal stream.

This matters to the reflection formula. Microsoft's
[automatic-coordinate documentation](https://learn.microsoft.com/en-us/windows/win32/direct3d9/automatically-generated-texture-coordinates)
identifies vertex position and normal as the inputs. As an independent API
implementation check, Wine's
[fixed-function vertex shader generator](https://github.com/wine-mirror/wine/blob/master/dlls/wined3d/glsl_shader.c)
initializes an absent normal to zero and computes
`reflect(normalize(view_position), normal)` for this TCI mode. With zero normal,
the result is simply `normalize(view_position)`. DXVK's
[vertex implementation](https://github.com/doitsujin/dxvk/blob/master/src/d3d9/shaders/d3d9_fixed_function_vert.vert)
also generates reflection coordinates before rasterization. These sources
corroborate API semantics; the mesh ownership and absent streams above come
from UOSA itself. Function names can change as external recovery annotates the
shared Ghidra project; the executable hash and addresses remain the anchors.

The runtime therefore computes normalized view-position XY at vertices, then
interpolates it without renormalizing in the fragment stage. Its source affine
is linear and can remain after interpolation. The former helper instead
reflected an interpolated position about an assumed horizontal normal per
fragment, changing both the direction and interpolation. A tiny length floor
makes an exact camera-origin vertex finite. Terrain reuses its otherwise unused
UV varying; ground statics carry one extra `vec2`. This removes per-fragment
view-position/normal transforms and normalization without another texture,
lookup, allocation or draw call. Explicit-coordinate lava/acid techniques still
select their existing UV streams.

The GPU sampler regression additionally checks the recovered coordinate values
for two view directions and the camera origin. This establishes the traced
coordinate contract, not full camera equivalence: original camera orientation,
item geometry/anchors, clocks, per-stage filtering and final color response still
need separate comparison. It is unnecessary to reproduce the original
fixed-function state machinery to preserve these coordinates efficiently.
The retained terrain LOD topology and reconstructed item quads can still differ
from the original triangles, so this change does not establish interpolation
parity at every point or across all terrain/static boundaries.
