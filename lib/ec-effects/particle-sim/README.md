# particle-sim

CPU simulation and preview rendering of the Ultima Online Enhanced Client particle effects, independent
of any engine or GUI. The behaviour follows UOSA.exe (Gamebryo 2.3) as traced in
[`uosa_chaos.md`](../../../scripts/dev-helpers/ec-research/ghidra-effects/results/uosa_chaos.md) and
[`uosa_effects.md`](../../../scripts/dev-helpers/ec-research/ghidra-effects/results/uosa_effects.md). Doc
comments cite the client addresses and the Gamebryo 2.6 SDK sources they rely on.

## Modules

- **`chaos`**: the Chaos engine (`.ems` text scripts). `compile` applies the client's compile rules to a
  uocf syntax tree and reports what the client rejects (`ChaosIssue`). `ChaosSystem` is a script prepared
  once, with its event times drawn. `ChaosEmitter` is the client's `Emitter` with its per-sequence bounds
  and MESH clones. `ChaosEffectInstance` is `UOParticleEffectChaos`.
- **`nif`**: the NIF runtime for Gamebryo `.nif` effect scenes. `compile` turns a `gamebryo-nif` file into
  a `NifEffectDesc`: nodes, meshes, `NiPSys*` particle systems, materials, textures, lights, controllers
  and keys. `NifEffectInstance` runs the scene, its controllers (`ComputeScaledTime`) and its particle
  systems as the client does.
- **`effect`**: the effect layer. It places a system by a `ParticleEffect` definition: tile offsets, the
  camera-aligned node, duration and loop.
- **`render`**: the engine-neutral render output: sprite batches, mesh draws, sort keys and the effect
  lights. It also computes the `NiStandardMaterial` vertex colour (`light_vertex`) and pixel combine
  (`combine_textures`).
- **`raster`**: a small software rasterizer for previews. It draws a frame with the client's D3D9 state
  (addressing, filtering, depth, alpha test, blending) in the client's draw order (`Canvas::draw_group`).
- **`hued`**: hued NIF effects (`UOHuedParticleEffectNif`): the half-speed clock, the offscreen target and
  the hue quad with the `uoeffects.psh` formula.
- **`codec`**: the binary form of `ChaosSystemDesc` and `NifEffectDesc`. It is stored in `effects_ec.uddp`
  (see the `udd-assets` README). Each blob has a magic, a desc version and a codec version.
- **`mips`**: the mip levels the client's textures have, shared by the packer and the previews. It covers
  stored DDS levels, generated chains and the `MIP_DEFAULT` rule.
- **`rng`**: the client's MT19937 (Chaos) and the MSVC `rand()` (NIF particle systems).
- **`math`**, **`system`**: vectors and matrices, and the `ParticleSystemInstance` trait every system kind
  implements.

Features: `uocf` (Chaos compiler from uocf syntax trees, effect definitions), `nif` (NIF compiler from
`gamebryo-nif`), `dds` (DDS level decoding). All three are on by default; a runtime that only loads
compiled descriptions can turn them off.

Examples: `chaos_preview` and `nif_preview` write contact sheets of real client effects. Tests:
`real_files` and `nif_real_files` compile and run every effect of a client (`UO_EC_CLIENT_DIR`,
`--ignored`).

## Known differences from the client

These are the places where the simulation, the preview renderer or the packer knowingly differ from
UOSA.exe, or where they rest on a reading that has not been confirmed. Everything not listed here is meant
to match the client, and the research docs mark it **confirmed**. Each item gives:

- **We:** what this crate (or the preview and packer around it) does.
- **Client:** what the client does, or what is unknown.
- **Evidence:** the UOSA.exe address or the SDK file and line.
- **Impact:** what would look different.
- **Verify:** whether it can be checked, and how.

### Draw order

**1. Equal sort depths keep scene order.**

- **We:** `Canvas::draw_group` sorts the blended drawables far to near with a stable sort, so ties keep
  scene order.
- **Client:** `NiBackToFrontAccumulator::SortObjectsByDepth` uses an unstable quicksort. The order of equal
  depths is unspecified and can change from frame to frame.
- **Evidence:** `NiBackToFrontAccumulator.cpp:111`; `NiAlphaAccumulator::Sort` (`NiAlphaAccumulator.cpp:68-110`);
  `FinishAccumulating` 0x006d6990.
- **Impact:** Only drawables with exactly the same depth, such as overlapping blended layers of one node.
  With additive blending the result is the same. With alpha blending the client may flicker where we are
  steady.
- **Verify:** No: it depends on the quicksort's input order. A frame capture can only show that both
  orders occur.

**2. Chaos sort centre before the first bound refresh.**

- **We:** A quad sequence sorts by its bound centre, refreshed every `BOUND_INTERVAL_S` (0.033 s). Before
  the first refresh we use the node position (LOCAL sequences) or the origin.
- **Client:** Before the first refresh the bound comes from the template quads of
  `ParticleTriShape::InitBuffers`. It is computed with `NiBound::ComputeFromData` in the shape's frame, and
  we have not reproduced it.
- **Evidence:** `ParticleSet::Update` 0x008b2de0 (cases 1 and 5); `InitBuffers` 0x008c5c50; `FUN_0068e750`.
  See `chaos::runtime` (`render`, "Before the first refresh").
- **Impact:** At most the first 33 ms of a sequence. During that window it can sort differently against
  other effects.
- **Verify:** Yes: read the template vertices in `InitBuffers`, or log the shape bound in a debugger right
  after `Start`.

**3. The `NiPSysBoundUpdateModifier` skip of 0.**

- **We:** We sample every `max(skip, 1)`-th particle, so a skip of 0 samples all of them.
- **Client:** `NiPSysBoundUpdateModifier::Update` handles 0 somehow, but the SDK has no body for it, so the
  client's handling of 0 is unknown.
- **Evidence:** UOSA.exe 0x00815930 (vtable 0x00cdc610 +0x48); `nif::instance` `system_bound_center`.
- **Impact:** The sort centre of NIF particle systems with a skip of 0 or 1. A different sampling moves the
  AABB centre only slightly.
- **Verify:** Yes: decompile 0x00815930 for the skip branch.

### Chaos simulation

**4. The per-particle step of the MESH sub-steps.**

- **We:** A MESH clone's controllers are sub-stepped at `MESH_SUBSTEP_S` (0.0333 s) over the particle's
  last update step (`ChaosParticle::step`), then posed at the particle age.
- **Client:** The same loop, but it reads the step from an array at +0x12 in `ParticleSet::Update`. Taking
  that array as the per-particle step is a probable reading.
- **Evidence:** `ParticleSet::Update` 0x008b2de0, case 2; `chaos::mesh` `MeshClones::update`.
- **Impact:** Only MESH sequences (9 in the data) whose clones run controllers that depend on intermediate
  steps. Pure key animation ends at the same pose.
- **Verify:** Yes: trace the writer of the +0x12 array in Ghidra.

**5. The rotation order of MESH clones.**

- **We:** The clone rotation is `Euler(rotation) · node rotation` (row-major, `M · v`), with `euler_xyz` =
  `X · (Y · Z)`.
- **Client:** The same order, read from the decompile. It has not been compared on screen.
- **Evidence:** `ParticleSet::Update` 0x008b2de0, case 2; `NiMatrix3::FromEulerAnglesXYZ` 0x0066b640
  (`NiMatrix3.cpp:1076-1084`).
- **Impact:** Rotated MESH particles would turn about the wrong axes if the order were reversed.
- **Verify:** Yes: compare a rotating MESH effect with the client from the same view.

**6. Random number generators.**

- **We:** Each `ChaosEffectInstance` owns its MT19937, and each `NifEffectInstance` its MSVC `rand()`
  state, so a seed replays an effect. Each MESH clone owns a `rand()` seeded with `MESH_CLONE_SEED` (1), so
  clones of one sequence draw the same numbers.
- **Client:** One process-wide MT19937 (`MersenneTwister::NextU32` 0x0099aa85), seeded from the clock, and
  one shared C runtime `rand()`. Every effect and every game system draws from them, so client sequences are
  not reproducible.
- **Evidence:** `rng` module docs; `chaos::mesh::MESH_CLONE_SEED`.
- **Impact:** The statistics are the same, but the individual particles differ. MESH clones with their own
  particle systems emit identical patterns where the client's would differ.
- **Verify:** No for exact sequences, by design. Distributions can be compared.

**7. The gravity axis.**

- **We:** `GRAVITY_AXIS` is local +Z, which is camera up in the world.
- **Client:** Gravity is projected onto `DAT_00de2810`, copied from column 2 of the effect rotation at
  `DAT_00e37224+0x30`. Whether it is camera up in every view was not checked in a running client.
- **Evidence:** `FUN_004216b0`; `uosa_chaos.md`, "Open questions".
- **Impact:** Every Chaos effect with gravity. A different axis would make particles fall sideways or along
  the view direction.
- **Verify:** Yes: read `DAT_00de2810` in a debugger with the camera at its default and at another zoom.

**8. The rotation sign of rotated POINT quads.**

- **We:** Rotated POINT quads use `MakeRotation(-rot.y, D) × camera rotate`, with `-rot.y` as the angle.
- **Client:** The same expression, but the sign convention is probable.
- **Evidence:** `ParticleTriShape::BuildVertices` 0x008c4c30; `uosa_chaos.md`, "Quad construction".
- **Impact:** Rotated sprites would spin the other way. Visible only with asymmetric textures.
- **Verify:** Yes: compare a spinning asymmetric sprite with the client.

**9. EVENTTIMER rewinds and `collisionCheck`.**

- **We:** A particle stops running events after `MAX_EVENT_TIMER_REWINDS` (64) rewinds in one update. The
  compiler rejects `collisionCheck` (`CollisionCheckHangs`).
- **Client:** There is no limit: an EVENTTIMER event that rewinds to its own time hangs the client.
  `collisionCheck` compiles, then loops forever in the sequence update.
- **Evidence:** `uosa_chaos.md`, "Quirks" 9; `chaos::runtime::MAX_EVENT_TIMER_REWINDS`.
- **Impact:** None on shipped data, which uses neither. We show something where the client freezes.
- **Verify:** Known from the code. Do not try it in the client.

**10. EMITTER particles.**

- **We:** EMITTER sequences are neither simulated nor rendered. The packer reports them and does not pack
  them.
- **Client:** It starts one sub-effect per particle (`EmitterParticle`, 0x008c66c0).
- **Evidence:** `uosa_chaos.md`, "MESH and EMITTER particles".
- **Impact:** None: no shipped script uses EMITTER.
- **Verify:** Not needed until data uses it.

### Rendering state

**11. Blend factor mapping.**

- **We:** `NiAlphaProperty::AlphaFunction` values map to the matching `D3DBLEND` (`ONE` → `D3DBLEND_ONE`,
  and so on).
- **Client:** This is the standard Gamebryo DX9 mapping, but the client's DX9 renderer was not traced.
- **Evidence:** `ParseBlendMode` 0x008b1430 (table 0x00c983c8); `render` (`BlendFactor`); `uosa_chaos.md`,
  "Blending".
- **Impact:** Every blended effect, if the mapping were different. It is very unlikely to be.
- **Verify:** Yes: trace `NiDX9RenderState` alpha setup, or capture a frame with PIX.

**12. Chaos quad addressing and filter (now settled).**

- **We:** Chaos sprites use WRAP_S_WRAP_T and FILTER_TRILERP (`SPRITE_ADDRESSING`, `SPRITE_FILTER`).
- **Client:** Traced: the base map is created with flags 0x3100 (`FUN_004169a0`), then the filter is set
  to TRILERP. Only the step from `NiTexturingProperty::Map` flags to D3D sampler states is Gamebryo's and
  was not traced. It is the same assumption as item 11.
- **Evidence:** `ParticleSet::CreateSequenceRenderer` 0x008b3c52..0x008b3c5d; `uosa_chaos.md`, "Render
  state of quads".
- **Impact:** None expected.
- **Verify:** Yes: a frame capture of the sampler state.

**13. Anisotropic filtering is drawn trilinear.**

- **We:** `FILTER_ANISOTROPIC` maps to `TextureFilter::TRILERP`.
- **Client:** It sets D3D anisotropic filtering where the device supports it. Gamebryo falls back to
  trilinear where it does not.
- **Evidence:** `nif::desc` `TextureFilter::from_raw`.
- **Impact:** Slightly sharper textures in the client on surfaces seen at a grazing angle. Effects are
  camera-facing, so the difference is small.
- **Verify:** Yes: a frame capture. It is not worth reproducing in a CPU rasterizer.

**14. The depth EQUAL test uses exact float equality.**

- **We:** `CompareFunction::Equal` compares the f32 depths exactly.
- **Client:** D3D compares the values quantized in the depth buffer (D24), so nearly coplanar surfaces
  can pass in the client and fail here, or the other way round.
- **Evidence:** `nif::desc` `CompareFunction::passes`, used by the depth test in `raster`;
  `NiDX9RenderState::ApplyZBuffer`.
- **Impact:** Only drawables whose `NiZBufferProperty` uses EQUAL. The shipped data was not surveyed for
  it.
- **Verify:** Yes: survey the z-buffer test functions of the effect NIFs. Quantize the depth if any use
  EQUAL.

**15. Cube environment maps are not drawn.**

- **We:** `SpecularCubeMap` and `DiffuseCubeMap` texture coordinates give no sample.
- **Client:** It looks the cube map up by the reflection vector or the normal.
- **Evidence:** `render` `EnvironmentMap::uv`; `nif::instance` (coordinate generation).
- **Impact:** None: none of the environment maps in the client data (three NIFs carry an
  `NiTextureEffect`) is a cube map.
- **Verify:** Not needed.

### Lighting

**16. Specular light, the dimmer threshold and the light cap.**

- **We:** We sum the ambient and diffuse terms of every light that is on, scaled by its dimmer. We add no
  specular term. We skip no light for a near-zero dimmer, and we do not cap the number of lights at 8.
- **Client:** Specular needs an `NiSpecularProperty`, and no lit effect has one. The Gamebryo light
  handling (a 0.01 dimmer cut-off and at most 8 lights per draw) was not traced in UOSA.exe.
- **Evidence:** `render` `EffectLight`, `SceneLighting`; `uosa_effects.md`, "Lights inside effect NIFs".
- **Impact:** None on shipped data: only `00005540_shadowstrike.nif` has lights, two white
  `NiDirectionalLight`s with dimmer 1.
- **Verify:** Yes, if new data adds lights: trace `NiDX9LightManager` / the `NiStandardMaterial` light
  count.

**17. Particles get ambient light only.**

- **We:** NIF particles have no normals, so only ambient lights reach them. Meshes are lit per vertex.
- **Client:** How the client lights geometry without normals was not traced.
- **Evidence:** `render` `light_vertex`; `nif::instance` (lights on draws).
- **Impact:** Only the particle systems of `00005540_shadowstrike.nif`. Its two lights are directional,
  with ambient 0, so in our preview they add nothing to its particles.
- **Verify:** Yes: compare shadowstrike in the client, or trace the shader generated for a lit
  `NiParticleSystem`.

**18. The spot light inner cone.**

- **We:** `SPOT_INNER_COS` = 1 (inner angle 0°).
- **Client:** NIF 20.0.0.4 stores no inner angle (it is streamed only since 20.2.0.5), so the angle is the
  constructor default. We take that default to be 0°.
- **Evidence:** `NiSpotLight::SetInnerSpotAngle`; `render::SPOT_INNER_COS`.
- **Impact:** None: no effect NIF has a spot light.
- **Verify:** Yes: read the `NiSpotLight` constructor in UOSA.exe.

### Hued effects

**19. Square texels in the hued offscreen target.**

- **We:** The offscreen target has square texels: `round(512 × aspect)` × 512 pixels, with the same world
  mapping.
- **Client:** The target is 512 × 512 and covers 8·w/h × 8 world units, so its texels are stretched by the
  screen aspect.
- **Evidence:** `UOHuedParticleEffectNif::CreateOffscreenCamera` 0x00472160; `NiRenderedTexture::Create`
  0x0067f480; `hued::offscreen_view`.
- **Impact:** The horizontal resolution of hued effects is finer here. Their shape, size and clipping are
  the same.
- **Verify:** Yes: a frame capture of a hued effect on a wide screen.

**20. The hue atlas width.**

- **We:** We assume a 1024-texel-wide atlas: the size the `Hues.uop` loader in `lib/uocf` produces, and the
  width the shader's 1/1024 scale implies. The 255-texel column stride and the 50/50 texel-edge blend
  follow from that width.
- **Client:** It loads `Build\Hues\Hues.dds`. The size of that file was not read in the binary.
- **Evidence:** `BuildHueQuad` 0x0041df40 (scale 0x00d05158, 255.0 at 0x00d04ef0); `hued::HUE_ATLAS_UV_SCALE`;
  `uosa_effects.md`, "Hue atlas sampling".
- **Impact:** Hue colours of hued NIF effects, mostly for hues ≥ 1024.
- **Verify:** Yes: read the size of `Hues.dds` from the client, or capture a hued effect with a hue ≥ 1024.

**21. No depth bias on the hue quad.**

- **We:** The hue quad is centred on the effect position, without the constructor's depth bias.
- **Client:** It offsets the quad by 2 × a depth bias along the camera direction. The bias is built from
  two unidentified object values (vtable +0x11c × -0.001 - vtable +0x124 × 1e-5).
- **Evidence:** `UOHuedParticleEffectNif` constructor 0x0041d9c0, `BuildHueQuad` 0x0041df40.
- **Impact:** Where the hued effect sorts against the object and other effects. Its look is unchanged.
- **Verify:** Yes: identify object vtable slots +0x11c and +0x124.

### Preview only (uocf-editor-gui Effects tab, examples)

**22. MESH clones use the default camera and lighting.**

- **We:** MESH clones render with `nif::default_camera` (the client view) and the default lighting. The
  preview's orbit camera and lighting controls do not reach them.
- **Client:** It always uses the client view and no light, so this matches the client. It differs from the
  rest of the preview only after the user changes those controls.
- **Evidence:** `chaos::mesh` `MeshClones` (no `set_camera`); `nif::instance::default_camera`.
- **Impact:** After orbiting, billboards inside a MESH clone keep facing the client view.
- **Verify:** Not a client question.

**23. The preview's lighting and camera are parameters.**

- **We:** The Effects tab defaults to `CLIENT_EFFECT_LIGHTING` (no light) and the client view, but both can
  be changed.
- **Client:** No light reaches effects, and the view is fixed.
- **Evidence:** `uosa_effects.md`, "Effect camera and lighting"; `render::CLIENT_EFFECT_LIGHTING`.
- **Impact:** Only what the user picks.
- **Verify:** The defaults are confirmed.

**24. Embedded texture names collide within one Chaos part.**

- **We:** The inspector keys the embedded `NiPixelData` textures of a Chaos part's MESH NIFs by their
  lowercase render name. Two MESH NIFs in one part with the same embedded name share one texture: the
  last one loaded wins.
- **Client:** Each NIF's texture object is its own.
- **Evidence:** `tools/uocf-editor-gui/src/logic/effect_preview.rs`, `nif_textures` (`embedded` map). The
  packer has no such problem: it renames them `<nif name>#<index>` (`udd-assets` README, `effects_ec.uddp`).
- **Impact:** Wrong texture on one of the meshes, in the inspector only. No shipped Chaos part is known to
  hit it.
- **Verify:** Yes: key the map by NIF name and texture index.

**25. PropertyFlag without an object.**

- **We:** For PropertyFlag mode 1 with property 7 (object hue), the preview picks the case of the preview
  hue. Mode 0 plays nothing, and other properties play the default children.
- **Client:** Collection effects played from packets (`PlayEffectAtTile`) have no object, so only the
  default children of mode 1 play. With an object, the object's value selects the case.
- **Evidence:** `uosa_effects.md`, "12 PropertyFlag" and "Runtime semantics"; `effect_preview.rs`
  `property_flag`.
- **Impact:** The preview cannot show "played from a packet, no object" apart from "object with hue 0"
  when a case has key 0.
- **Verify:** Not a client question. It would need a "no object" preview option.

### Packer

**26. Optional BC7.**

- **We:** `pack-ec-effects --bc7` encodes uncompressed textures (sides a multiple of 4) to BC7. The default
  keeps RGBA8, and DXT1/DXT5 sources stay BC1/BC3.
- **Client:** It uploads the decoded texture in its source format.
- **Evidence:** `lib/udd-conv/src/effects_ec.rs` (`EffectsEcPackOptions::bc7`).
- **Impact:** Lossy colour only when `--bc7` is used.
- **Verify:** Compare packages built with and without `--bc7`.
