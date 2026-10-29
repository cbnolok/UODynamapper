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
and reads the first bitmap row. Classic 32-color strip expansion is unchanged.
A synthetic compressed `Hues.uop` with a gradient top row and distinct solid
bottom row exercises discovery, lazy names, decoding and UDDP round-trip colors.

`inspect_ec_hues` exports original selected bitmaps and the native atlas through
the production UOP/DDS readers. In the inspected FP source, all 3,000 converted
256-pixel strips match the corresponding `build/hues/hues.dds` rows **exactly in
RGBA bytes**. This independently validates the row and coordinate mapping;
sample filenames or a plausible recolored scene alone do not establish it.
Temporary raw evidence is under `/tmp/uodynamapper-ec-followup/original-hues/`
and `hue-gradient-package/`. Proprietary palette pixels remain outside the repo.
