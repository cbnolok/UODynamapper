# Effects rendering: status and approximations

Status of the EC particle effects pipeline (`lib/particle-sim`, the `effects_ec.uddp` packer in
`lib/udd-conv/src/effects_ec.rs`, and the uocf-inspector-gui Effects tab) after the October 2026 UOSA.exe
tracing. The full list of known differences, with evidence and impact, is in
[`lib/particle-sim/README.md`, "Known differences from the client"](../../lib/particle-sim/README.md#known-differences-from-the-client).
The client research is in
[`uosa_effects.md`](../../scripts/dev-helpers/ec-research/ghidra-effects/results/uosa_effects.md) and
[`uosa_chaos.md`](../../scripts/dev-helpers/ec-research/ghidra-effects/results/uosa_chaos.md). Those docs
mark each item **confirmed** or **probable**; this note does not repeat their addresses.

## Verified against UOSA.exe and implemented

- **Lighting.** No light reaches effects (`CLIENT_EFFECT_LIGHTING`). `E_A_D` materials show only their
  emissive colour, and the 33 drawables with neither emissive nor glow draw black, as in the client. The
  lights inside an effect NIF (only `00005540_shadowstrike.nif`) are compiled and applied per vertex to the
  meshes they reach (`cf721d75`, `ee6a52bb`).
- **Placement and hue.** Tile offsets are converted as `PlayEffectAtTile` converts them. Hued NIF details
  are the half-speed clock, the target cleared to transparent black, the samplers and the hue coordinate
  (`ee6a52bb`).
- **Textures.** Clamp/wrap per axis, the depth test and write of each draw, the client's mip levels (stored
  DDS levels kept, a chain generated only for `MIP YES` single-level power-of-two sources), each map's
  filter mode, and NIF path controllers with velocity and follow (`7bccbf66`).
- **Packer.** `.nif` texture files (one `NiPixelData`) are decoded. Particle files missing from the client
  are reported once each, as warnings (`e6d7e963`).
- **Draw order.** All of render group 7 is drawn in one pass. Unsorted drawables come first, in scene
  order. Then the blended ones are drawn far to near by bound centre · view direction, one sort item per
  Chaos sequence and per NIF geometry.
- **Material and blending.** The `NiStandardMaterial` combine has the decal lerp before `Kd`, no
  intermediate saturation and one glow pass. `NiTextureEffect` environment maps are added to the colour.
  DestAlpha reads 1 in the world pass and accumulates in the hued target. Chaos MESH particles are clones of
  their `.nif` with its own properties (traced in `911fd240`, implemented in `3449f8d1`).

Related package work in the same effort: UOP data block headers and their `data_block_hash` are decoded
(`b3c78e4f`), and replaced or added UOP entries get a fresh timestamp header (`622749ed`).

## Still approximated or unverified

The README has one entry per item. The ones most likely to matter on screen:

- Gravity axis and the rotation sign of rotated POINT quads: read from the code, never compared in a
  running client.
- Hue atlas width (1024) is assumed, not read from `Hues.dds`. It decides the hue colours for hues ≥ 1024.
- MESH clone details: the per-particle sub-step (a probable reading of the +0x12 array in `0x008b2de0`),
  the Euler · node rotation order (from the decompile), and one RNG per clone where the client shares one.
- Draw order edge cases: equal depths keep scene order where the client's quicksort is unstable. A Chaos
  sequence sorts by node position or origin until its first 0.033 s bound refresh.

Items with no effect on the shipped data: cube environment maps, spot light inner cone, specular, the
0.01 dimmer threshold and the 8-light cap, EMITTER particles, the `collisionCheck` hang.

## Gotchas

- Effects are camera-aligned billboards: in an effect's local frame X is camera right, Y the view
  direction and Z camera up. Previews must look along local +Y (`CameraFrame::client_view`). A world-axis
  camera makes every effect look wrong.
- Chaos effects are never hued, whatever the definition's `hued` flag. Only `.nif` particles with an object
  hue ≠ 0 are.
- Collection effects played from packets have no object, so PropertyFlag can only play mode 1's default
  children. The inspector preview cannot tell this apart from an object with hue 0.
- In the inspector, embedded NIF textures of the MESH NIFs of one Chaos part share one name map. The
  packer avoids this by naming them `<nif name>#<index>`.
