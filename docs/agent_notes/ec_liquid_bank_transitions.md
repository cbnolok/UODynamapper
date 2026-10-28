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

The height fade ends at the liquid/crust plane, starting 0.05 world units below
it. On the liquid side, distance to the solid cell supplies a smooth half-cell
blend under the crust. A small rising-surface guard removes liquid completely
by 0.025 world units above its plane. This avoids both a hard cutoff and the red
outline left when the blend extends up the exposed mountain face. Fully covered
flat liquid stays unchanged and skips the neighborhood search.
Lower shelves and submerged sand are not pulled into a liquid-owned surface.
Solid owners at or below liquid level keep their original coverage until the
interpolated mesh actually rises. Removing liquid just because a level substrate
shares its height exposed dry, angular patches in an otherwise flat swamp.
The neighborhood covers the bilinear ownership mask's 4×4 footprint, including
across chunks. A 3×3 owner-centered search is insufficient: filtered ownership
can carry a liquid from two cells away. Leaving those cells unchecked produced
isolated lava patches on higher rock after the main bank was removed. The
footprint is offset by `floor(world - 0.5)`, matching the coverage sample centers.

## Preserve the underlying solid composition

The bank weight attenuates only the matching liquid material's pass alpha.
The existing solid pass stack, authored coverage and source ordering then show
through. Replacing the result with one neighboring texture everywhere was
rejected: it produced obvious gray material triangles along the bank.

An all-liquid chunk can meet solid terrain across its edge without retaining a
local solid base pass. Only that case uses the selected neighboring solid as a
base. Authored flowing static overlays remain a separate rendering path.

## Ease the bank outline

Raw integer-height grid corners produce a stepped outline along diagonal banks;
normal filtering alone cannot change it. The vertex shader eases solid heights
next to lower liquid using 35% of a local binomial average, limited to 0.15 world
units in either direction and never below the liquid plane. Non-bank terrain and
CC geometry retain their original positions. Liquid heights remain fixed.

A liquid vertex adjoining raised solid only diagonally can also relax sideways
by up to a quarter cell. This bevels the inset corner of a stepped diagonal
bank. The view basis restricts that relaxation to banks facing away from the
camera, with a smooth angular ramp. Moving the visible mountain face's corners
opens a gap beside the authored crust, so that contact stays fixed. Every shared
vertex uses global cell coordinates and the same view; chunk edges agree.
Neither topology nor vertex count changes. These are presentation adjustments;
the original map heights and material ownership metadata remain intact.

## Cost and verification

Fully covered flat owning liquid returns before reading neighboring metadata. Bank selection
returns a cell coordinate and scalar weight; a whole `TileUniform` is not kept
alive across fragment shading. The coverage stack remains read-only. The opaque
pass optimization runs with the effective alpha, skipping fully hidden liquid
shading. No texture, atlas, uniform, draw pass or package format was added.
Bank geometry adds one eight-neighbor metadata search per EC vertex, sharing
those reads between the solid-height and diagonal-corner checks.

The actual GPU regression executes 29 bank cases alongside the 43 established
sampling/material/geometry cases. Synthetic metadata isolates higher banks,
partial coverage, submerged sand, payload aliases, other liquid identities and
missing sources, plus the two-cell reach in both directions and diagonals;
it also checks the liquid-side blend and bounded vertex changes. The real
bank-selection and geometry WESL run unchanged. The ordinary
composer test additionally loads its production atlas and coverage dependencies.

Desktop-4 captures cover paused lava in Enhanced/KR, animated lava, Classic EC,
bridge/stairs, coast, acid, swamp, town and CC controls. Accepted artifacts use
`bank-verified-` in `/tmp/uodynamapper-ec-followup`. Earlier `bank-stack-` and
`bank-performance-after-` captures retain the insufficient neighborhood and
overly wide height fade; their isolated mountain patches are terrain coverage,
not authored flowing statics. A temporary solid-color terrain capture proved
that distinction. `bank-after-` retains the rejected single-texture replacement.
`lava-bank-final-comparison.png` compares the previous and corrected result.
The intermediate quarter-height cutoff and height-only smoothing did not fully
address the visible crust outline and diagonal stair-step respectively.

An uncapped, unfocused-throttling-disabled ABBA smoke check uses the same paused
KR lava fixture before and after the final coverage/geometry changes.
The eight samples ranged from 33 to 36 FPS, with means of 35 FPS before and
34.5 FPS after. This small difference is within the run's observed variation.
Those eight settled-frame samples check for a material regression; they are not
a sustained benchmark or evidence of a whole-frame speedup.
