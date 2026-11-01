# UODynamapper Canonical TODO

This file is the single source of truth for remaining work.

It intentionally excludes work that is already landed or already reduced to completed investigation notes.

## Targets And Constraints

Primary targets:
- 400+ FPS in military orthographic gameplay mode.
- 144+ FPS minimum in advanced perspective features.
- Low RAM/VRAM footprint with strict batching and cache discipline.
- Baseline hardware: GTX 1060 class GPU.

Hard constraints:
- No Mesh Shaders or vendor-only rendering features in the core runtime path.
- Partial texture uploads must be batched at least once per frame, never one upload per object.
- Metadata used for gameplay logic must remain lossless end-to-end.
- Runtime work must stay staged; do not merge base correctness, repetition, authored blends, liquid support, and extra passes into one change slice.

Current execution decisions:
- Keep `32x32` map chunks for now.
- Keep `32x32` static chunking for now.
- Keep `64x64` as the preferred UDDP transport and decompression unit for terrain payloads unless telemetry proves otherwise.
- Keep transport granularity separate from render granularity.

## Architecture Principles

- GPU-driven rendering: CPU submits compact state, shader code performs reconstruction and compositing.
- Data indirection: immutable geometry plus mutable metadata textures or buffers.
- Bounded memory: LRU with hysteresis for streamed content.
- Streaming-first I/O: `mmap` plus async tasks plus upload scheduler for `uddp` files streaming.
- Format-aware compression: codec chosen by payload type and payload size.
- EC evidence discipline: original UOP contents are primary, extracted original client shader text is secondary when present, third-party reconstructions are design references only.

## How To Read This File

- `Now`: active or near-term work that should drive implementation order.
- `Next`: important follow-up that depends on `Now` being stable first.
- `Later`: still planned, but not the best immediate use of time.
- `Validation`: tests and regression work needed to keep the pipeline from drifting.
- `Deferred`: still tracked, but intentionally not on the immediate path.

## Canonical Milestones

- [ ] Close remaining EC chosen-base, support-texture, and surface-like routing gaps.
- [ ] Finish the production static-art pipeline and explicit UO-style static depth follow-up.
- [ ] Harden UDDP packaging, streaming, and validation.
- [ ] Move from transitional terrain scaling toward clipmap-based terrain.
- [ ] Expand mobiles, paperdoll, and export tooling.

## Now

### Current Renderer Focus: Visual Fidelity

Proposed execution order and the latest asset-session corrections are in
[the 2026-10-31 renderer plan](reviews/2026-10-31-renderer-next-work-plan.md).
The KR-only artwork fallback and current package handoff are integrated; see
[the source/native acceptance](reviews/2026-11-01-renderer-art-handoff.md).
Static inspection now follows composed material coverage and rendered depth;
see [the visibility-query acceptance](reviews/2026-11-01-visible-static-picking.md).
Next correct the separate ground-plane cursor-cell/teleport approximation on
slopes, then continue individual geometry and animation work.
Older captures apply to their recorded package revisions.

- Validate remaining individual static anchors/new-art scales and effect visibility against matched fixtures. Common legacy pixel/height calibration, the perspective bridge stair/deck tie-bias defect and stale hover quad construction are repaired.
- Validate remaining individual/degenerate sprite windows and anchors against [the descriptor contracts](agent_notes/ec_sprite_descriptor_geometry.md). Required per-owner source scales, positive ordinary world-window edges and matching art/tilemeta rebuilding are implemented; retain the visible legacy-crane and scaled-plant controls.
- Effect reachability is now checked for grunge, KR boundary grime/color wash and projected shadows. Keep remaining source/strength parity distinct from inactive controls; [the effect audit](agent_notes/effect_visibility_validation.md) records normal-layer eligibility and the morning lighting cap's suppressed snow response.
- Correct the separate visible-terrain cursor-cell/teleport query on slopes and validate broader camera/facet/large-art/multi coverage. Static inspection now uses shared GPU material coverage and rendered depth, with a stated exact-coincident-opaque tie limit. Use original logical surfaces and movement flags for future walking. See [hover geometry contracts](agent_notes/static_hover_geometry.md).
- Investigate thin gold lines and translucent triangular artifacts over water with animated MSAA4x. The phase-2 bridge matrix and a separate pre-change binary/original-shader run both reproduce them; see [the picking review](reviews/2026-11-01-visible-static-picking.md).
- Validate wider equal-priority overlaps while keeping the repaired town and bridge histories as regression controls. Transparent batch sorting fixes the tree-edge defect; visible-set sprite tie assignment fixes the separate opaque bridge-edge tie across chunk boundaries. See [the depth notes](agent_notes/static_depth_ordering.md).
- Legacy Translucent source weights and blended-pass admission are repaired from original executable evidence; the isolated coast mark is verified original artwork. Keep [the matching-pair controls](agent_notes/classic_static_translucency.md); exact 5-bit/gamma composition remains parity work.
- Mandatory-trim art/tilemeta integration and the earlier 20-mask recovery have recorded validation. The subsequent source audit traced package differences to the fp/plain client split; plain Enhanced is canonical. Missing-source aliases now carry the client's `missing.tga`; KR-only art uses the actual Classic/missing-image fallback. The complete Classic atlas and matching current EC set are installed. Keep remaining material/reflection parity separate from this accepted source integration; see [the corrected coverage note](agent_notes/tex_aux_ec_coverage.md) and the renderer plan above.
- Use [the visual follow-up review](reviews/2026-10-29-visual-fidelity-followup.md) for current evidence and reconciliation of the unstaged user TODO. Historical repaired items must reproduce before being reopened.

### Baseline And Instrumentation

- Add frame-time breakdown telemetry for CPU update, extraction, upload, and render.
- Add upload statistics for queued writes, bytes per frame, and batch count.
- Add cache diagnostics for LRU hit rate, eviction reason, hysteresis counters, and queue backlog depth.
- Define golden benchmark scenes and reproducible camera paths.
- Make metrics visible in the debug overlay and persist them in logs for A/B runs.
- Mobile conversion timing: done. Both mobile packers log a per-stage summary at the end (`CC mobile animation stage timing` / `EC mobile animation stage timing`: load/metadata, plan, trim, upscale, bucket selection, source decode, page build/blit, BC7 input/encode/RDO/flatten, page registration, manifests, package write), and the UDDP builders log compression apart from assembly and write. Measured with the `profiling` profile on 2026-11-01 (CC fp, BC7 + Zstd: 51.1s to 32.2s; EC, BC7: 167.8s to 78.2s; package sha unchanged, peak RSS lower): the lazy, speculatively parallel page prefix search (`udd_conv::prefix_search`), parallel trim and per-page source decode, row-wise gutter extrusion and the parallel EC hue scan. Largest stages left: bucket selection (CC 8.7s, EC 11.5s; guillotiere fit tests), EC page build (14.7s), BC7 encode (CC 5.6s, EC 12.0s) and package write (CC zstd 5s; EC 10s, the payload spill copied into the package).
- BC7 pre-RDO audit: solid blocks (the empty and gutter area of a page) already take the encoder's solid fast path in `pack_bc7_rgb`/`pack_bc7_rgba`; the remaining encode time is content blocks. Reusing repeated non-solid blocks is the open option.
- BC7 RDO: closed 2026-11-01, kept as the opt-in `--bc7-rdo` (default λ 0.05; the mobile packers and `tex_aux_ec` default to λ 0). At λ 0.05 the packages get larger or barely smaller: `tex_land_cc` 10.92 -> 11.12 MB (+1.8 %, 0.3 s -> 13.3 s), `tex_art_cc` 38.71 -> 38.84 MB (+0.4 %, 1.2 s -> 22.6 s), `mobile_anim_ec` 1.988 -> 1.965 GB (-1.1 %, 81 s -> 1580 s). Bytes come back only at a λ that shows (λ 2.0: -13.8 % on `tex_land_cc`, 33..41 dB PSNR against plain BC7; λ 0.05 is 43..53 dB). Plain BC7 encode is 10.6 s of the 81 s EC mobile pack, so the encoder is not the bottleneck. [Proposal (d)](agent_notes/proposals/2026-11-01-data-track-open-items.md).

### EC Material And Texture Ownership Pipeline

Done; kept as a pointer. Owner-reference census, support-texture inventories and the baseline report are `udd-cli pack audit-ec-material-refs`, `inventory-ec-support-textures` and `report-ec-material-baseline` (c2e383fe); the metadata axes are on every tilemeta texture ref (`physical_package`, `logical_family()`, `stable_role` from the block shader and the item position since 55647354, `block_index`/`item_index`, and the selector's reason with `is_low_confidence`); `Textures`-family refs, raw/normalized paths and `texture_stretch` survive the tileart parser (`lib/uocf/tests/synthetic/tileart_ref_preservation.rs`); item refs are tilemeta `item_texture_refs`, land layers are `tex_land_ec` `source_materials.json` plus terrain provenance; non-image `EffectTexture.uop` resources have `write-ec-effect-meta-manifest` (33c88fb5). Tilemeta stores texture ids, not paths: a path follows from the id and its family.

### EC Packaging Boundaries

Done; kept as the rule. `tex_art_ec.uddp` holds the tileart-owned visible images, `tex_land_ec.uddp` the `TerrainDefinition`-owned ones, `tex_aux_ec.uddp` the image-bearing support refs that runtime consumers read (c61470d8, a8cf7d93), and non-image resources stay metadata manifests. Packages follow semantic ownership, not one flat texture-id atlas; `udd-cli tool info` shows each package's contents.

### EC Base Selection And Visibility Rules

Done. The main-texture selector is role first with explicit reasons (`choose_main_ec_texture_id_role_aware`, `MainEcTextureRoleReason`, `TileMetaMainEcTextureReason`), and the tex_art_ec packer takes the client's own primary (`ArtData.ec_texture`); roles come from the shader and the position, so no role-correction hook is needed. Liquid entries are kept whenever they have a base (`LiquidExclusionReason`; all 322 included), support-only stacks stay in tilemeta refs and `tex_aux_ec`, and `write-ec-abnormality-report` (d7319dc8) reports missing and low-confidence bases. The empty override hooks (`dynamapper/assets/ec_overrides/*.kdl`, `udd_conv::ec_overrides`) are removed: the base and the role are the client's.

### EC Runtime: Open Correctness Gaps

- Finish runtime consumption of chosen-base metadata for land-owned, art-owned, and surface-like art paths.
- Surface-like routing has no open failures (census 2026-11-01, installed packages: 9222 Enhanced surface-like items, none left without an image). An item with a `tex_art_ec` slot draws its own image (`EcSurface`/`EcRegular`), which wins over any `ec_land` match, and one without falls back to Classic art first, so `ec_land` decides only the 261 water-shader liquid payloads. The former samples `1345` (cave floor, `UOStaticTerrainShader`) and `1395..1399` (stone roof, `UOSpriteShader` with `1000034`) draw their own tileart textures, as the client does; their support maps are all in `tex_aux_ec`. Open renderer details are in `agents_comm.txt` (2026-11-01): the dead `unresolved_surface_like` counter, the unreached non-liquid `TexLandEc` branch, and the 61 `UOWaterShader` blood items drawn from their primary only.
- Keep the safe static runtime precedence:
  - `tilemeta.is_surface_like()` plus resolvable `ec_land` slot first
  - then existing art fallback paths
  - never route normal art through unsafe `selected_texture_id -> canonical_slot_id` shortcuts
- Preserve the safe `ec_land` runtime rule:
  - direct populated slot wins
  - only then fall back through provenance or canonical mapping
- Revisit runtime terrain-id override coverage, including the documented forest-family `196` case in `runtime_material_id_overrides`.
- Draw the rest of the bump terrain layer (`UOBumpMapTerrainLayer`, material 8): item 3 (luminance bump, du/dv) and item 4 (environment map). The land shader now draws only items 0..2 (colour A/B by the item 2 alpha); see `scripts/dev-helpers/ec-research/ghidra-terrain-layers/results/uosa_terrain_layers.md` for the stage setup.

### Static Art Pipeline And Residency

- Finish the production-grade clustered static-art pipeline.
- Implement atlas packing with `guillotiere` for static art.
- Support two runtime storage modes for art atlas pages:
  - `RGBA8` as the lossless default for tiny and alpha-sensitive art
  - `BC7` as optional for larger pages after validation
- Build metadata lookup per art id for atlas layer, UV rectangle, pivot or offset, and behavior flags.
- Keep metadata lookup O(1) and exact, with partial writes only when atlas slot assignment changes.
- Implement the alpha-mask plus depth-write shader path for clean overlap.
- Keep draw submission batched by material and atlas page or layer set rather than by object.

### Static Depth, Transparency, And Building Occlusion Follow-Up

- Replace projected clip-space-derived static ordering with an explicit UO-style logical depth key.
- Keep visual billboard placement and logical depth computation independent.
- Preserve the split between:
  - visual position and billboard offsets
  - logical ordering inputs such as `tile_x`, `tile_y`, `priority_z_units`, and class depth offset
- Freeze the explicit depth-key formula in code comments, docs, and shader or Rust contracts before further tuning.
- Add `tile_x`, `tile_y`, and `priority_z_units` as first-class logical-depth inputs for static instances.
- Stop using projected clip depth as the final ordering source for statics; use the explicit UO depth key instead.
- Reapply class offset tables on top of `priority_z_units`, not visual Y bias.
- Re-tune foliage only after the explicit depth key is live.
- Implement timer-based alpha lerp for fade-in and fade-out behavior rather than abrupt opacity changes.
- Add player-local building ceiling scan state similar to `_maxZ` or `_maxGroundZ` to support roof and upper-floor hiding or fading.
- Validate the result in dense town and building scenes with equal-Z walls, foliage near walls, and terrain-versus-land-static overlap.

## Next

### UDDP, UDDF, Streaming, And Upload Scheduling

- Finalize UDDP on-disk layout with explicit sections for header, dictionary index, dictionary blobs, entry table, and data blocks.
- Keep UDDF as the single-object wrapper for standalone resources and tooling.
- Preserve the planned `u16` compression header layout:
  - bits `0..3`: base compression type
  - bits `4..6`: custom compression type
  - bits `7..15`: content type id
- Implement async read and decode tasks through Bevy task pools.
- Implement upload scheduling that merges writes and emits bounded GPU copy batches each frame.
- Implement per-content dictionary handling for small payload classes.
- Implement fallback decode paths per content type.
- Keep predictive prefetch rings optional until telemetry proves they improve real movement behavior.
- Keep live editing persistence and delta-merging deferred until editor and gameplay ownership is explicit.

### EC Review, Audit, And Artifact Surfaces

Abnormality reports and owner audits (`write-ec-abnormality-report`, `write-tileart-owner-audit`, `write-terrain-definition-owner-audit`, d7319dc8), source-controlled override tables (`dynamapper/assets/cc_ec_convtables/`, checked by `audit-ec-terrain-overrides` and `audit-land-routing-kdl`) and editable review artifacts (`write-terrain-definition-review`, `write-tileart-liquid-surface-report`, 4a9a87de) exist.

- Done: `uddp-inspector-gui` shows each tilemeta item's texture refs (position, role, package, family, primary/aux, stretch, main-texture reason and confidence) and which of `tex_art_ec`, `tex_aux_ec` and `tex_land_ec` beside the tilemeta holds each image (`logic/tilemeta_refs.rs`).

### EC Effects: Manual Checks

These need a person at the GUI or in a running client; the code side is done.

- Open an `effects_ec.uddp` in `uddp-inspector-gui` (Effects view) and check the overview, tables, texture
  view and preview on screen; nothing has been looked at outside headless tests.
- Try the inspector Effects tab "Export glTF..." buttons (NIF editor and preview panel) and open the
  results in Blender.
- Check the "Known differences from the client" in `lib/ec-effects/particle-sim/README.md` against the real client
  (in-game look or a frame capture, e.g. RenderDoc under Wine). Most valuable first:
  - the pale halo on low-luminance edges of hued effects;
  - the effects whose unlit parts draw black (E_A_D materials without emissive or glow);
  - draw order where blended effects overlap (back-to-front by bound centre, unstable in the client).

### EC Runtime Staging

- World-space repetition and stretch precedence:
  - make explicit metadata override texture-extent inference when warranted
  - keep the rule inspectable
- Single-land material mode for materials that are truly one-base by evidence.
- Solid-land blend mode for materials with credible `Base`, `SecondaryBase`, `BlendAlpha`, and `CoverageAlpha` support.
- Land-owned liquid mode with explicit uncertainty handling for ripple-like or normal-like support inputs.
- Art-owned wet or liquid mode separate from terrain-liquid logic.
- Extra-pass exploration only behind explicit feature gates and diagnostics.

### Camera Modes, Water, And Terrain Switching

- Keep camera projection concerns in camera setup rather than content data.
- Implement or harden runtime switches for:
  - military orthographic
  - free orthographic
  - perspective
  - terrain blend mode
  - water mode
  - structural advanced path toggle
- Preserve the mandatory water pass ordering:
  - opaque terrain with water fragments excluded
  - submerged art and mobiles
  - transparent water final pass
- Keep classic water as the baseline mode and treat enhanced procedural water as benchmark-first.

### Static Light Source Follow-Up

- Upgrade the current static-light support from light-mask quads plus approximate local response into a real projected decal path.
- Keep using item metadata as the source of truth:
  - `flags & 0x00800000` marks light-source statics
  - `quality` selects the world light id
  - placed static hue can tint the light mask through the configured hue source
- Preserve the visible static-light mask rendering, but move it to a dedicated light-decal material instead of ordinary `StandardMaterial`.
- Apply `light_decal_intensity` to the decal draw path, not only to approximate art and land shading response.
- Feed additive light decals before tonemap and bloom so KR-style local lights can survive the dark material profile.
- Replace fixed warm local-light response with sampled or projected light color where practical.
- Improve land response beyond the current first-16-light uniform cap:
  - tile/chunk-local light lists
  - stable sorting to avoid flicker when visible-light order changes
- Improve art response beyond per-instance accumulated intensity by considering projected mask coverage or screen-space/local UV coverage.
- Validate against torch, window, brazier, and spell-light scenes in day, night, and cave presets.

### KR-Style Visual Enhancement Effects

- Add height-aware contact darkening for terrain, statics, tree bases, cliffs, walls, and art feet.
- Strengthen directional slope shading for land while keeping broad painterly roll-off instead of sharp modern normal-map lighting.
- Add material-specific wet and highlight response for water, lava, swamp, ice, and snow; avoid global shine on ordinary terrain.
- Continue transition dirt/grime accumulation near material boundaries: first KR pass darkens texture-payload boundaries through `enable_kr_transition_grime` and `kr_transition_grime_strength`; remaining work is metadata-backed material-family tuning for grass-to-road, snow-to-rock, cliff-to-ground, and plaza edges.
- Improve sprite grounding with soft contact shadows under statics and mobiles plus subtle side darkening for tall art.
- Continue soft local color and broad KR color-wash influence on nearby art and land: first shader pass adds bounded low-frequency warm/cool wash through `enable_kr_color_wash`, `kr_color_wash_strength`, and `kr_color_wash_scale`; remaining work is screenshot tuning and real additive dynamic light decals.
- Add restrained elevation or distance atmosphere for large terrain views without obscuring tile readability.
- Add material-class micro-contrast controls so grass, rock, snow, roads, and liquid surfaces can separate without one global sharpening curve.
- Extend normal-map source/orientation/filtering validation beyond the recorded snow receiver. Common dry grass/rock/road materials supply no authored normals; original liquid bump composition has a separate path. Seventeen false fourth-artwork normal bindings are repaired. Morning lighting caps can suppress real snow response; retain the lower-energy control in the effect audit before changing strengths.
- Continue material-family wet response beyond generic specular: first shader pass separates water, lava-like liquid, swamp-like liquid, and ice/snow-like grazing highlights; remaining work is visual validation and metadata-backed material-family classification.
- Continue light-color provenance for static lights: first pass feeds ClassicUO light-shader colors into land/art local response, with warm fallback; remaining work is package-hue and sampled-mask color provenance.
- Continue terrain shadow/compression for vertical cliffs and high banks: first KR pass darkens steep neighbor-height transitions through `kr_land_relief_shadow_strength`; remaining work is screenshot validation and metadata-aware cliff family tuning.
- Continue roof/tree/large-static side falloff: first KR shader pass adds subtle one-sided depth tint for foliage, roofs, and tall non-ground statics; remaining work is screenshot validation and family-specific tuning.
- Continue projected art shadows: first pass renders opt-in projected shadows for tall sprite statics; second pass samples softened sprite atlas alpha so trees, walls, and large statics cast silhouette-like shadows. Remaining work is screenshot validation, draw ordering review, and material/depth-family tuning.
- Continue texture-backed grunge/noise: first pass binds a generated repeatable texture to land/art/ground-art and blends it with procedural variation; remaining work is identifying the official KR/EC support texture and replacing the fallback image.
- Keep these effects modular and independently tunable in shader settings; each effect must be disableable for regression comparison.
- Preserve original UO/KR stylistic intent: no heavy bloom, screen-space outlines, modern PBR material treatment, or procedural noise pasted uniformly over every surface.

### Future-Readiness Preservation

Done or superseded. Blend inputs (`SecondaryBase`, `BlendAlpha`, `CoverageAlpha`) and the water inputs (`WaterBump`, `WaterReflection`, `WaterFourthStage`) are distinct stable roles (55647354), so the role already says how a ref is used and no separate readiness field is needed. Waterfall, splash, foam, glow and similar effects come from the effect definitions (`effects_ec.uddp`), not from texture names.

## Later

### Terrain Architecture Beyond Current EC Work

- Keep the current threshold-based terrain scale and mesh path explicitly documented as transitional.
- Design the clipmap land path as nested GPU land rings around the camera, with clear rules for ring size, update cadence, and metadata residency.
- Replace discrete zoom mesh swapping with GPU clipmap rings.
- Implement snapped world-space sampling in the vertex stage.
- Use continuous LOD so zoom no longer depends on hard render-threshold swaps.
- Add live-edit compatible terrain invalidation and update rules for the chosen clipmap path.
- Preserve deterministic export compatibility while advanced rendering modes evolve.

### Terrain Blending And Water Modes

- Keep the original transition-tile path for classic fidelity.
- Add automatic blend mode for custom maps that lack transition tiles.
- Keep source logical tile ids immutable in both modes.
- Treat stochastic or noise seam refinement as optional after the base auto-blend path works.
- Keep depth-aware tinting for enhanced water bounded and benchmark-first.

### Mobiles, Animation, Hues, And Paperdoll

- Define the animation asset normalization path for classic and KR or EC sources.
- Store a hue mask channel for tintable pixels.
- Keep hue as a runtime parameter applied in the fragment shader rather than duplicating atlases per hue.
- Reuse an existing secondary UV channel for hue and runtime payload where practical.
- Build the clustered mobile animation pipeline.
- Implement mobile streaming cache with LRU and hysteresis.
- Benchmark and choose between:
  - multi-quad layered composition
  - single-quad fragment compositing
- Preserve deterministic layering for mount, body, equipment, hair, and overlays.

### Export And Tooling

- Implement the 1:1 tiled world export pipeline (approved 2026-11-01: WebP method 4 q85 sharp YUV, no JPEG XL set; offscreen export mode after a control-channel pilot; Britannia first; 512 px tiles; Leaflet 1.9.4 vendored; z = 0 readout; sRGB box downsampling). [Web map export proposal](agent_notes/proposals/2026-11-01-web-map-export.md#decisions-2026-11-01).
  - Done: `tools/webmap-export-cli` (`plan`, `run`, `pyramid`, `viewer`, `pilot`): the frame plan, the spool consumer of the renderer contract, the lossless level-6 intermediate and the pyramid built from it, atomic writes and frame-level resume, the libwebp wrapper with `--quality`/`--method`/`--sharp-yuv` (covers the WebP profile item; lossless tiles are not needed).
  - Renderer session: the export mode (`DYNAMAPPER_EXPORT_JOB`), per the [contract](agent_notes/proposals/2026-11-01-web-map-export.md#contract-for-the-renderer-session); requested in agents_comm.txt.
  - Renderer session: the 2 x 2 control-channel pilot with `webmap-export-cli pilot`; requested in agents_comm.txt.
  - Then: the full Britannia run, a visual pass, and the CI build of the tool on Windows, macOS and ARM64.
- Done: map and statics chunks are compressed with plain Zstd at the level that decodes fastest for its size (`udd_conv::chunk_compression`: map 19, 15.96 MB for Britannia at the same 3.4 us per warm chunk as level 7's 17.76 MB; statics 7, since higher levels decode 5..10 % slower); `--zstd=LEVEL` sets the level and `--stored` stores them (0.2 us per map chunk, 118 MB). No type dictionary: it made map chunks larger and 10 % slower. The UDDP reader reuses a per-thread Zstd context and the type dictionaries prepared at open (the installed dictionary map: 7.9 -> 3.9 us per chunk). LZ4 was measured for comparison only (map 58 MB at 2.2 us). Benchmark: `lib/udd-conv/tests/integration/map_statics_retrieval.rs`. [Proposal (a)](agent_notes/proposals/2026-11-01-data-track-open-items.md).
- Done: `udd-cli tool validate <files or dirs>` decodes every entry through its key, checks the package hash, codecs and dictionaries, and runs the package's own reader, so a stale package fails before the app loads it.
- Done: the `uddp-inspector-gui` tile and land-material previews decode only the BC7 blocks covering the rect (`udd_image_codecs::bc7::decode_bc7_rect_to_rgba8888`, equal to the whole page cropped); the whole page is decoded when the Full Atlas view is shown. First selection on a 2048x2048 `tex_land_ec` BC7 page: 23.9 ms -> 0.14 ms CPU (the 4096x2048 RGBA `tex_art_ec` page: 24.4 ms -> 4 us), plus the full-page texture upload it no longer does.

## Validation

### Immediate Regression Protection

`udd-container` `uddp_layout` checks the blob layout (head, dictionaries, unpadded payloads in key order) and reads through the memory-mapped reader. The owner-reference census has unit tests for its archive and family guesses and an ignored real-file test (`udd-conv-cli` `ec_material_refs_client`: one row per tileart item and terrain layer, every tileart row equal to the packed tilemeta ref). The ignored real-file test `udd-conv` `tex_aux_ec_client` packs `tilemeta` and `tex_aux_ec` from the clients and checks that every block-0 support ref of a material item resolves in `tex_aux_ec` with readable pixels (15271 items, 13714 refs, 23 on the missing-texture image). Parser tests for `Textures`-family classification, stable roles and position roles exist (`lib/uocf/tests/synthetic/tileart_ref_preservation.rs`; tilemeta `roles_come_from_the_shader_and_the_item_position`, `roles_follow_the_block_shader_and_the_position_not_the_name`), as do liquid-base retention (`tex_art_ec` `liquid_with_*`), tilemeta material and `terrain_meta` round trips, and `meta32` packing.
 `udd-conv` `decoder_robustness` cuts and changes samples (fixed-seed xorshift) for the Classic art, gump and animation RLE, AMOU, DDS, TGA, `tilemeta` and `terrain_meta` readers; none may panic (synthetic samples always, shipped ones ignored behind `UO_CLIENT_ROOTS`). It found the `terrain_meta` readers allocating a damaged record count, a span past the layer table panicking in `udd_conv` `iter_tile_layers`, and the DDS decoder allocating the RGBA of a damaged header's size; all three fail with an error now.

### Priority Test Inventory

- `test_metadata_packing_roundtrip`
- `test_iso_depth_sorting`
- `test_subtile_precision_bias`
- `test_hue_lookup_logic`

### Screenshot-Family Regression Buckets

- Build fixture families instead of validating only isolated ids.
- Keep at least these families covered:
  - marble floors
  - cave floors
  - marsh water
  - lava
  - blood stains
  - waterfall or snow scenes
  - roads and plazas
  - grass-to-sand transitions
- Combine structural tests and screenshot-family checks; do not rely on only one of them.

### Acceptance Gates

- Static art in military mode uses one shared material path and shows no visible sorting artifacts on masked sprites in the baseline map.
- Equal-Z walls, roof edges, and terrain-versus-land-backed statics remain stable after the explicit depth-key migration.
- No main-thread stalls during normal camera motion.
- Upload bursts remain bounded under stress camera sweeps.
- No zoom-threshold redraw hitches once the clipmap path replaces the transitional mesh swap path.
- Terrain metadata VRAM optimization must not lose height precision or tile-id precision.
- Military mode maintains the target performance profile on the benchmark scene.
- Perspective and advanced modes remain deterministic under runtime switching.
- Multiple hue variants share the same base atlas allocation.

## Documentation And Evidence Hygiene

- Add one durable documentation artifact that explains how the third-party world-space UV proof-of-concept maps into the current data model.
- Keep it explicit that the third-party renderer is reconstruction-only, not original EC source.
- Preserve both decoded repetition values and texture extents as separate signals for future comparison.
- Keep unresolved scaling questions explicit rather than flattening them into false certainty.

## Known Open Constraints And Rules

- `tileart.uop` ownership remains authoritative for art-owned wet and liquid entries even when support inputs come from `TerrainTexture.uop` or `EffectTexture.uop`.
- `TerrainDefinition.uop` ownership remains authoritative for land materials.
- `Texture.uop` is a mixed pool; package origin alone is not sufficient classification.
- `ec_land` and `ec_art` may legitimately duplicate the same source image when semantics differ.
- `tex_art_ec` must keep `TileType::Solid` admission; only the old liquid blanket exclusion was wrong.
- Surface-like statics need dedicated runtime handling and must not be forced through ordinary billboard-art assumptions.
- Shader specialization is benchmark-first, not architecture-first.
- Water pass ordering correctness has priority over cosmetic enhanced-water work.

## Risks And Mitigations

- Risk: over-fragmented atlas allocation under long sessions.
  - Mitigation: periodic defrag or copy planning plus migration-map updates.
- Risk: too many small `queue.write_texture` calls.
  - Mitigation: per-frame upload queue coalescing and hard caps.
- Risk: over-specialized compression policy complexity.
  - Mitigation: keep the policy table centralized by content id.

## Deferred

- Terrain metadata VRAM optimization (indexed metadata atlases, per-page palettes in a GPU-readable buffer, shader indirection, converter-generated palettes), deferred to the renderer performance work (the user, 2026-11-01). The `Rg16Uint` atlas is 4 B per tile, 16 MiB per 2048² page, 128 MiB for Britannia (8 pages). On `map0` a page holds 318..859 tile ids and 1,817..21,143 (id, Z) pairs (the whole map 1,496 ids, 38,770 pairs); a 32x32 chunk at most 117 ids, an 8x8 block at most 41. So an 8-bit per-page id index does not fit, Z needs its own 8 bits, and the smallest lossless layout is 2 B per tile: -64 MiB for Britannia, for an indirection on the land shader's 63 metadata reads. The converter side is about a day and not needed until the runtime part is chosen. Keep it lossless (see Validation). [Proposal (c)](agent_notes/proposals/2026-11-01-data-track-open-items.md).
- Renderer FPS optimization is deferred until the desktop workstation session with RenderDoc and rocprofv3. Preserve the accepted cache changes; resume from [the hypotheses and test plan](reviews/2026-10-29-renderer-performance-handoff.md) before choosing another implementation.
- Full lighting-model parity with original client time-of-day rules.
- Networking protocol integration.
- Editor UX for live map sculpting and tile painting.
- Classic map, statics and dif writers (`mapN.mul`, `mapNLegacyMUL.uop`, `staidxN`/`staticsN`, the `x` editions, dif baking): deferred until a dynamapper editor or a `.uddp` → Classic export needs them (the user, 2026-11-01). The measured layout and the options are in the [proposal](agent_notes/proposals/2026-11-01-classic-map-writers.md); option B is recommended.

## Suggested Execution Order

1. Finish baseline instrumentation and EC evidence, metadata, and packaging cleanup.
2. Close the remaining EC runtime routing and chosen-base correctness gaps.
3. Finish the static art pipeline and bounded upload discipline.
4. Lock in audit, override, and review surfaces.
5. Continue runtime staging in narrow, falsifiable slices only.
6. Harden regression coverage with parser, packer, inspector, and screenshot-family validation.
7. Resume broader roadmap items such as clipmaps, streaming hardening, mobiles, export, and advanced camera modes once the EC material pipeline is stable.
