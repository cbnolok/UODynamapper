# EC terrain filter footprints

Terrain material selection and coverage loops vary between neighboring fragments.
Computing `fwidth`/`dpdx`/`dpdy` inside their shading calls therefore depends on
which invocations reach a particular call. WGSL requires derivatives to execute
in uniform control flow; see the [uniformity rules](https://gpuweb.github.io/gpuweb/wgsl/#uniformity).
Successful shader compilation does not establish that those results are portable.

This became observable while streaming the existing ownership cache directly
instead of expanding its records into private fragment arrays. The Vulkan
regression retained bit-identical ownership layers and alpha, yet native filtered
captures changed by up to 35 channel bytes. With both linear filtering and blur
disabled, all six fixtures stayed within one byte. Separate lava comparisons
reproduced differences with linear filtering alone (26 bytes) and blur alone
(14 bytes). These controls isolate the filtering dependency; record equivalence
alone cannot validate a fragment optimization.

## Continuous footprint

`land/main.wesl::fragment` captures derivatives of interpolated world XZ before
ownership, liquid-bank and material branches. `LandGradients` carries them through
the shading calls. Each EC base/detail/mask/normal slot divides that continuous
footprint by its own authored repetition, using the same extent fallback as its
UV calculation. Differentiating wrapped UVs would also treat a texture repetition
as a large footprint rather than a continuous surface.

`pixel_art_uv_iq_grad` uses the supplied UV derivatives for its texel seam width.
EC atlas samples use explicit texture gradients. Blur uses the same center
footprint for its radius and all nine taps, instead of differentiating their
rotated/jittered offsets. Sharpening uses that footprint too. The existing CC
filter remains in its previous path.

This intentionally corrects filtered EC pixels around discontinuities; it does
not promise byte identity with the old derivative-dependent output. Texture
selection, coverage weights, composition order, authored UV repetition, geometry,
hues and lighting rules remain in their established paths. In matched native KR
orthographic fixtures, the correction affects about 0.3–1.1% of pixels by more
than one byte outside swamp, where only three pixels exceed one byte. Visual
inspection still matters when evaluating the resulting filtering.

## Cache validation

With the continuous footprint shared by both implementations, expanded cache
records and direct texture reads agree within one byte in KR, both orthographic
and perspective. Enhanced/Classic EC and the Enhanced CC control are byte-identical.
The full uncached reconstruction versus direct cache reads also stays within one
byte in KR. Each comparison covers transition (2518,561), bridge (2834,987),
lava (5744,2928), coast (1445,1790), swamp (1872,2416) and town (1647,1682),
with paused water, isolated settings, complete metadata submission and native
Desktop-4 captures. The existing Vulkan test separately checks every material
record and alpha bit at 9,216 positions, plus cache-hit and poisoned-guard cases.

Temporary comparison data uses `stream-stable-*` and `stream-filter-*` in
`/tmp/uodynamapper-ec-followup`. Earlier `single-shade-*` and the first
`streamed-stack-first-*` captures predate this correction and are not accepted
image-equivalence baselines. Future fragment changes should retain both filtered
native comparisons and the independent ownership regression.

Water pause does not freeze fullscreen fog: `fog_pass.wesl` has a separate
`time_seconds` drift. In the KR perspective fixture, changing the initial
capture warm-up from eight to twenty seconds changed up to five color bytes
over broad areas. Recapturing the expanded-stack control with the same twenty
seconds restored all six comparisons to within one byte. Match the animation
timeline as well as camera/settings, or freeze the relevant effect explicitly
in isolated comparison assets. Do not attribute that unmatched fog difference
to material selection or normal reconstruction. The same requirement applies
to cached/uncached KR comparisons in orthographic view.
