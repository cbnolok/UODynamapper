# Non-KR art fake-normal validation

Classic and Enhanced keep `enable_art_fake_normals = 0` in all four periods
in factory presets, bundled shader settings and the saved shader settings
inspected on 2026-10-30. The art control remains an explicit opt-in in every
render style; land shading mode does not prohibit it.

Fake normals now apply to KR world art only. Classic sprites have their shading
painted in, so a per-sprite normal repeats on every tile of a roof or wall and
draws bright and dark bands at the seams. In a difference image of the Classic
road fixture this shows as a pillow pattern per roof tile. The collector marks
legacy sprites with `STATIC_INSTANCE_FLAG_LEGACY_ART` from the same
`ResolvedArtSprite.legacy_art` that selects their source, and `art/main.wesl`
turns the effect off for them. This covers legacy sprites in KR view too. Ground
instances are EC surfaces and are unaffected. The dialog grays the toggle when
the art source is not EC.

With the toggle checked against unchecked over road, town, swamp, snow, lava and
bridge, Classic and Enhanced are byte-identical in all six. KR is unchanged from
before the gate: mean 1.87, 1.03, 0.33, 0, 0.14 and 0.73 levels. Before the gate
the Classic road fixture differed by 4.08 levels with 57 % of pixels past 2 levels,
and Enhanced by 1.41 with 20 %. The checks below predate this gate and describe
the old non-KR behaviour.

The shared `art/shading.wesl` helper already removes rectangular edge cues from
upright Regular/Background sprites. `ART_UPRIGHT_BILLBOARD_EDGE_PROFILE` and
`ART_UPRIGHT_BILLBOARD_SIDE_FALLOFF_PROFILE` are both zero. Upright surface-like
sprites also suppress the rectangular edge cue. Roof/foliage profiles and true
ground bevels remain distinct. Directional Lambert shading and vertical shading
remain when the effect is enabled; suppressing the old frame does not flatten
all optional shading.

## Explicit activation checks

Six native off/on pairs cover:

- Classic with CC objects/terrain and Enhanced with CC objects/EC terrain.
- Stone walls at map 0 `(2507,560)` in morning and night presets.
- Timber walls at `(2518,561)` in morning presets.

Runtime status confirms each active preset and the fake-normal flag. Static
light collection, local lighting and mask glow are disabled so local-light
receiving cannot explain the result. The light-control isolation correction is
present in both halves. The effect changes visible shading in every pair; it is
not an inactive control. The old bright rectangular wall-edge frame was not
reproduced in these stone/timber views. Enlarged stone-wall comparisons show
that visible light-colored brick/pillar details also exist with fake normals off.
No additional wall-normal shader change was justified by these observations.

These checks validate the reported families and the faithful off defaults. They
do not establish original-client parity for optional fake shading, arbitrary
user strengths, every art family or arbitrary camera angles. Reopen a concrete
remaining frame artifact with its tile family, active preset and source settings.
Keep effects from the underlying authored image separate from added shading.

Evidence in the temporary renderer workspace is `art-normals-accepted-evidence.json`
and twelve full native frames (two off baselines reused from the current
local-light checks). `scripts/validate_art_normals.py` reproduces explicit
activation by period/mode/fixture; all previews use exact-PID Desktop-4 placement.
Source settings are restored after each run, and shader/pipeline logs are clean.
