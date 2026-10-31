# Original EC hue palette conversion

`pack-hues --ecdir <matching-client> --output <package-dir>/hues.uddp`
must receive the EC directory without a CC directory when recovering EC colors.
The existing mixed-source converter prefers `hues.mul` if one is supplied.
The renderer enables hue lookup only when `hues.uddp` exists in its configured
package directory; a preview without that package cannot validate hue behavior.

Three conversion defects were reproduced against the inspected FP bundle:

- Linux source discovery checked only `hues.uop`, missing the official `Hues.uop`.
- Lazy UOP indexing tried to unpack `huenames.csv` before loading its payload.
- Palette conversion sampled bitmap row `height/2`. Original EC bitmaps are
  256×2: row 0 contains the gradient, row 1 contains a solid preview swatch.
  Selecting row 1 produces flat recoloring and removes authored sprite detail.

The converter now recognizes the official filename, loads names before indexing
and reads the first bitmap row. Classic 32-color strips now ramp linearly
between colours by default, exactly as EA's atlas (`HueRampMode`, shared with
`uocf::enhanced::hues_mul_uop`); `--hue-ramp flat` keeps 8-texel blocks.
A synthetic compressed `Hues.uop` with a gradient top row and distinct solid
bottom row exercises discovery, lazy names, decoding and UDDP round-trip colors.

`inspect_ec_hues` exports original selected bitmaps and the native atlas through
the production UOP/DDS readers. In the inspected FP source, all 3,000 converted
256-pixel strips match the corresponding `build/hues/hues.dds` rows **exactly in
RGBA bytes**. This independently validates the row and coordinate mapping;
sample filenames or a plausible recolored scene alone do not establish it.
Temporary raw evidence is under `/tmp/uodynamapper-ec-followup/original-hues/`
and `hue-gradient-package/`. Proprietary palette pixels remain outside the repo.

## Sprite mask crops and auxiliary coverage

**Historical installation below:** the preservation-only extension used an
auxiliary package from the newer Enhanced fp client. The plain Enhanced client
is now canonical. Its KR-only art and tilemeta format-3 handoff restores missing
support roles and uses the original `missing.tga` for absent source images.
The earlier advice that a full repack is unsafe and missing-mask hues stay
unchanged is superseded by [the corrected source/role investigation](tex_aux_ec_coverage.md).
Compare packages built from the same client; omitted unused legacy-family
images do not imply lost renderer inputs. Runtime auxiliary/hue caches share
aliases by payload ID while keeping each requested image number for diagnostics.


Palette correctness is separate from source-2 mask availability. Ordinary
UOSprite masks use the matching art slot's native extent and tilemeta's trimmed
source origin. The mandatory-trim integration independently checks original
pixels, trim bounds and placement for three shifted-mask owners, in addition
to four geometry controls. Ten of the eleven shifted entries admitted by the
runtime's source gates have valid decoded mask crops. The eleventh, owner
41256, references original-missing texture 2003211.

`inspect_ec_sprite_hue_crops` reproduces those binding gates and reports why
a declared mask is unavailable. The installed set initially had 29 unavailable
bindings: 20 owners referenced 18 images present in original `Texture.uop` but
absent from `tex_aux_ec`; nine owners referenced five images absent from that
original archive. This is not a negative-origin or crop-bounds failure.

A full auxiliary repack is unsafe as a replacement for the configured set:
it omits existing support images, including 2600, and adds unrelated content.
`extend_ec_aux_package` instead requires explicit image IDs and adds only missing
entries to a new package. The accepted extension retains all 40,829 existing
image descriptors and decoded RGBA bytes exactly, and adds the 18 source-backed
mask images. Runtime coverage becomes 11,673 admitted sprites, 331 available
mask bindings, 307 distinct native crops and nine unavailable bindings.

At map 0 `(6404,2600)`, authored chairs with hue 1157 now recolor through the
recovered mask: 12,212 pixels change by more than one channel unit. Matching
bridge, coast and road frames are byte-identical. The `(1131,2221)` hue-2 scene
is unchanged and is not positive evidence for visible recovery there.
The source palette, original image references and decoded mask bounds justify
the repair; a plausible chair color alone does not establish original rendered
pixel parity.

The remaining source IDs are 2003211, 2003212, 20003979, 20004038 and 20004039.
No guessed ID spelling, synthesized mask or full-art hue fallback is applied.
The accepted matching art/tilemeta and preserved auxiliary extension are
installed together under `/home/claudio/test/uddp`, with checksum-verified
backups. Evidence is `water-trim-accepted-evidence.json` and
`trim-asset-installation.json` under `/tmp/uodynamapper-ec-followup/`.
