# Height-aware EC liquid banks

This is an intentional renderer improvement over the recovered ownership-mask
composition, not an additional claim about UOSA's implementation.

## Problem and policy

At map 0 (5744,2928), lava cells at raw height 30 share sloping terrain quads
with rock vertices at heights 37–45. Applying a liquid material's ordinary
ownership coverage to the entire rising quad makes lava coat the mountain.

The exception class comes from the original `UOWaterTerrainLayer` shader:
`EcTerrainParams` class 2 and the retained reviewed-liquid flag. It applies to
liquid materials without a list of graphic IDs, filenames or color guesses.
Other material classes retain their existing coverage rules.

`land/liquid_bank.wesl` finds an eligible higher solid neighbor for a liquid-owned
cell. On a solid-owned cell carrying liquid coverage, it finds the nearby level
of that original liquid material. Material IDs from `ec_coverage_params` match
payload aliases. Missing EC textures remain ineligible; there is no CC fallback.

Bank coverage fades in between 0.05 and 0.15 world units above the liquid level
(half to one-and-a-half native height units). Level liquid stays unchanged.
Lower shelves and submerged sand are not pulled into a liquid-owned surface.
The neighborhood covers the eight adjoining cells, including across chunks.

## Preserve the underlying solid composition

The bank weight attenuates only the matching liquid material's pass alpha.
The existing solid pass stack, authored coverage and source ordering then show
through. Replacing the result with one neighboring texture everywhere was
rejected: it produced obvious gray material triangles along the bank.

An all-liquid chunk can meet solid terrain across its edge without retaining a
local solid base pass. Only that case uses the selected neighboring solid as a
base. Authored flowing static overlays remain a separate rendering path.

## Cost and verification

Flat owning liquid returns before reading neighboring metadata. Bank selection
returns a cell coordinate and scalar weight; a whole `TileUniform` is not kept
alive across fragment shading. The coverage stack remains read-only. The opaque
pass optimization runs with the effective alpha, skipping fully hidden liquid
shading. No texture, atlas, uniform, draw pass or package format was added.

The actual GPU regression executes nine bank cases alongside the 43 established
sampling/material/geometry cases. Synthetic metadata isolates higher banks,
partial coverage, submerged sand, payload aliases, other liquid identities and
missing sources; the real bank-selection WESL runs unchanged. The ordinary
composer test additionally loads its production atlas and coverage dependencies.

Desktop-4 captures cover paused lava in Enhanced/KR, animated lava, Classic EC,
bridge/stairs, coast, acid, swamp, town and CC controls. Accepted artifacts use
`bank-stack-` and `bank-performance-after-` in `/tmp/uodynamapper-ec-followup`.
The earlier `bank-after-` captures retain the rejected single-texture replacement.
`lava-bank-before-after.png` compares the requested lower half of the scene.

An uncapped, unfocused-throttling-disabled ABBA smoke check at the same paused
KR lava fixture sampled 37–39 FPS in both variants (mean 38 FPS in each).
Those eight settled-frame samples check for a material regression; they are not
a sustained benchmark or evidence of a whole-frame speedup.
