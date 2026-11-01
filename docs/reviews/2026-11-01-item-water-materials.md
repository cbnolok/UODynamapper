# Item-owned EC water materials — 2026-11-01

The canonical package set contains 61 surface-art `UOWaterShader` owners without
a matching terrain material. They previously drew only their primary fallback
image. This phase implements their authored bump, reflection and optional fourth
stages through the existing item-surface lookup and auxiliary atlas. Terrain-linked
shorelines retain their accepted shared terrain/static path.

## Source and implementation

The original `UOSA.exe` method `UOWaterShader::SetupTextureStages` at `0x59bbd0`
binds sources 0/2/3; source 1 is the hardware fallback and defines the image window.
The bump stage selects bump alpha, and the reflection stage preserves it.
Technique[0] selects vertex-generated reflection, source-2 UVs or forced local UV0.
Technique[1] selects the fourth-stage multiply, alpha blend or add operation.
Tweaks retain independent bump/fourth scroll velocities. See the addresses and
proof limits in [the executable investigation](../agent_notes/ec_original_water_code.md).

The shader preserves unsigned RG displacement, source repetition and UV sets,
wrapped reflection sampling, encoded-color diffuse/fourth-stage composition and
one RGB conversion to linear. Its fractional alpha is shared with GPU picking.
There is no additional binding, pass or instance stride. Logical graphic, hue,
XYZ, signed Z, heights and movement flags remain unchanged.

Native comparison caught an additional UV error: the art atlas is trimmed, while
the original image-window builder divides source pixels by the full primary
image dimensions. Sampling the fourth map over quad-local 0..1 shrank blood 4651.
For ordinary image-window geometry, the runtime reconstructs UVs from `ec_start`,
the native slot extent and the original primary extent. The separate `Rotated`
camera-plane constructor at `0x45a080` instead writes literal unit UVs. Item 7379
has a 64x28 fallback crop and a 64x64 primary, but its camera quad must sample the
full fourth-stage image. A final source cross-check corrected the first crop fix,
which had been applied too broadly. The lookup now distinguishes both constructors;
the primary image's dimensions are never inferred from a different support map.

`tex_aux_ec` now requires `metadata/water_primary_extents.bin`, containing sorted
native `u32 [id,width,height]` rows. The converter reads these dimensions before
optional upscaling. It retains existing image selection and does not add primary
pixel storage or GPU uploads. Old auxiliary packages need rebuilding; the reader
reports the missing required metadata explicitly. No tilemeta layout change is
needed.

## Validation and installed package

The focused checks pass: seven item-surface CPU checks, the candidate graph check
covering all unmatched water owners, 111 production GPU sampler cases, the shader
composer, two auxiliary-reader checks and three packer checks. GPU cases distinguish
stage/alpha operations, explicit UV modes, unsigned displacement, paused/animated
translation and a trimmed native window. A normal renderer build was used for
native acceptance.

Seven isolated previews retain 27 accepted frames and 34 completed GPU picks,
excluding three superseded camera-quad fixtures from the first matrix. All verified
exact-process Desktop 4 placement, cleanup and unchanged OS focus/pointer, with no
shader validation errors. They cover:

- KR blood 4651/5701, dungeon variants and primary-only 7379, with visible and
  roof-occluded controls; the recovered 4651 crop again selects at `(655,420)`.
- Classic/CC, Enhanced/CC-art+EC-land and KR/EC sources; orthographic and KR
  perspective; accepted bridge, lava, coast and town scenes.
- Animation enabled: all 61 direct materials have zero authored Tweaks, so the
  blood remains still (at most one-byte channel noise), while bridge water moves.

Bridge and town controls differ from the prior accepted path by at most two bytes
per channel outside the inspector overlay. The earlier shrunken-mask pilot and
interrupted/reused-destination Classic runs are not acceptance evidence.
The final `camera-uv-final` and `camera-uv-perspective` checks cover the corrected
7379/7442 domain, an ordinary cropped blood control and unchanged bridge controls.

The canonical-source auxiliary rebuild has exactly one added metadata entry,
zero removed entries and zero changed decoded payloads among all 652 previous
entries. It is installed at `/home/claudio/test/uddp/tex_aux_ec.uddp`, SHA-256
`ffe89a80f1476280b2782f2ff2262b4fc05353a36b1359259df044583b8f740b`.
The previous package has a checksum-verified backup under
`/home/claudio/test/uddp-backups/item-water-native-extents-20261101-191729/`.
Map/statics and mobile candidates from other agents were not installed.

Evidence and installation manifests live under
`/tmp/uodynamapper-ec-followup/phase4-item-water/`, with `accepted-evidence.json`
as the capture index. Proprietary source images remain outside the repository.

## Remaining limits

No matching original-client blood screenshot was supplied, so source-stage and
UV agreement is not a claim of complete original-client pixel parity. The current
camera adapter, diffuse policy and generic material hue path remain in use.
Other texgen modes and the original alpha-property threshold waveform still need
their own evidence. The pre-existing animated MSAA edge artifact is separate.
Performance profiling, static frame animation and world effects remain later work.
