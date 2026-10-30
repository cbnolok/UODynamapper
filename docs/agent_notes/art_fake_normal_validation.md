# Non-KR art fake-normal validation

Classic and Enhanced keep `enable_art_fake_normals = 0` in all four periods
in factory presets, bundled shader settings and the saved shader settings
inspected on 2026-10-30. The art control remains an explicit opt-in in every
render style; land shading mode does not prohibit it.

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
