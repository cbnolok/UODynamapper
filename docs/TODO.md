# UODynamapper Canonical TODO

This file is the single source of truth for remaining work.

It supersedes the old split planning files:
- `TODO_PHASE_1_STATIC_ART.md`
- `TODO_PHASE_2_STREAMING_AND_EDITING.md`
- `TODO_PHASE_3_ADVANCED_RENDERING.md`
- `TODO_PHASE_4_MOBILES_AND_PAPERDOLL.md`
- `URGENT_TESTS.md`
- `TODO_CONSOLIDATED.md`

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
- Streaming-first I/O: `mmap` plus async tasks plus upload scheduler.
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

### Baseline And Instrumentation

- Add frame-time breakdown telemetry for CPU update, extraction, upload, and render.
- Add upload statistics for queued writes, bytes per frame, and batch count.
- Add cache diagnostics for LRU hit rate, eviction reason, hysteresis counters, and queue backlog depth.
- Define golden benchmark scenes and reproducible camera paths.
- Make metrics visible in the debug overlay and persist them in logs for A/B runs.
- Add mobile conversion timing breakdowns for atlas creation/blit, BC7 encode, RDO precompute or ultrasmooth work, RDO search, package compression, and package write.
- Optimize mobile atlas page build and blit paths before further RDO micro-tuning; atlas pages are approximately `2048x2048`, so full-page clears, crop/copy passes, and alpha scans can dominate.
- Audit BC7 encode work before RDO for uniform and gutter-heavy pages, including safe reuse of repeated transparent or solid blocks without compromising image quality.
- Continue RDO algorithmic reduction after timings isolate it, prioritizing branch predictability, bitwise operations, SIMD-friendly data layout, early backend dispatch, stack-local scratch where sensible, and cache-line-aware traversal.
- Profile package compression and write time separately from image processing so storage overhead is not misattributed to BC7/RDO.

### EC Material And Texture Ownership Pipeline

- Build a full owner-reference census from `tileart.uop` and `TerrainDefinition.uop` so direct package usage is measured instead of inferred.
- Add dedicated inventories for `TerrainTexture.uop` and `EffectTexture.uop`, including direct-consumer linkage, file-kind classification, and anomaly buckets for names like `ripple`, `water_alpha`, `cube`, `env`, `splash`, `foam`, `glow`, `lava`, and `mask`.
- Freeze the current chooser, routing, and shader baseline in explicit diagnostics before changing behavior further.
- Split and preserve these metadata axes everywhere they matter:
  - physical source package
  - logical family
  - stable role
  - speculative role
  - reason and confidence
- Extend tileart parsing so `Textures`-style refs are preserved instead of collapsing into lossy fallback buckets.
- Preserve raw path, normalized path, and `texture_stretch` for tileart-linked refs.
- Promote multi-reference metadata to first-class runtime data for both item-owned and land-owned records.
- Preserve metadata-only manifests for non-image `EffectTexture.uop` resources.

### EC Packaging Boundaries

- Keep `tex_art_ec.uddp` for tileart-owned visible bases.
- Keep `tex_land_ec.uddp` for `TerrainDefinition`-owned visible bases.
- Add `ec_tex_aux.uddp` for non-base image-bearing support refs.
- Keep non-image support resources as metadata manifests, not fake image slots.
- Make package assignment inspectable and auditable.
- Keep semantic ownership as the packaging driver; do not collapse EC assets into one flat texture-id atlas family.

### EC Base Selection And Visibility Rules

- Replace the old non-aux-first base chooser with stable-role-first selection plus explicit reason logging.
- Add override hooks for:
  - base choice
  - role correction
  - metadata-only tagging
- Replace the blanket tileart `Liquid` exclusion with a conservative base-eligibility decision.
- Keep support-only and no-base liquid stacks preserved in metadata and auxiliary outputs.
- Surface abnormalities for:
  - missing base
  - support-only stacks
  - ambiguous multiple bases
  - unresolved support-rich stacks
  - low-confidence fallback selection

### EC Runtime: Open Correctness Gaps

- Finish runtime consumption of chosen-base metadata for land-owned, art-owned, and surface-like art paths.
- Resolve remaining surface-like static routing failures where tilemeta says `SurfaceLike` but runtime `ec_land` resolution is still missing.
  - Known samples: `1345`, `1395`, `1396`, `1397`, `1398`, `1399`.
  - `1345` points at `ec_texture_id=2000130` and appears to have `TerrainDefinition` matches.
  - `1395..1399` point at `1000034` and currently show no `TerrainDefinition` matches.
- Keep the safe static runtime precedence:
  - `tilemeta.is_surface_like()` plus resolvable `ec_land` slot first
  - then existing art fallback paths
  - never route normal art through unsafe `selected_texture_id -> canonical_slot_id` shortcuts
- Preserve the safe `ec_land` runtime rule:
  - direct populated slot wins
  - only then fall back through provenance or canonical mapping
- Revisit runtime terrain-id override coverage, including the documented forest-family `196` case in `runtime_material_id_overrides`.

### Static Art Pipeline And Residency

- Finish the production-grade clustered static-art pipeline.
- Cluster tiny art records into `32..96 KB` uncompressed payload groups instead of compressing them individually.
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

### Terrain Metadata VRAM Optimization

- Implement indexed metadata atlases for terrain so metadata can drop below the current per-tile footprint when the palette cardinality supports it.
- Add per-page metadata palettes or dictionaries in a GPU-readable buffer.
- Update the terrain atlas shader path to perform indexed metadata indirection.
- Update the converter path to generate metadata palettes during packaging.
- Keep this optimization lossless for gameplay-relevant height and id data.

### EC Review, Audit, And Artifact Surfaces

- Emit machine-readable abnormality reports and owner-reference audits.
- Keep override files source-controlled and reviewable.
- Generate editable review artifacts grounded in original package evidence, not just current code behavior.
- Extend inspectors so they print at least:
  - owner kind
  - source package
  - logical family
  - stable role
  - speculative role
  - reason
  - confidence
  - runtime routing

### EC Runtime Staging

- World-space repetition and stretch precedence:
  - make explicit metadata override texture-extent inference when warranted
  - keep the rule inspectable
- Single-land material mode for materials that are truly one-base by evidence.
- Solid-land blend mode for materials with credible `Base`, `SecondaryBase`, and `AlphaMask` support.
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
  - nearest or strongest light selection
  - tile/chunk-local light lists
  - stable sorting to avoid flicker when visible-light order changes
- Improve art response beyond per-instance accumulated intensity by considering projected mask coverage or screen-space/local UV coverage.
- Validate against torch, window, brazier, and spell-light scenes in day, night, and cave presets.

### KR-Style Visual Enhancement Effects

- Add height-aware contact darkening for terrain, statics, tree bases, cliffs, walls, and art feet.
- Strengthen directional slope shading for land while keeping broad painterly roll-off instead of sharp modern normal-map lighting.
- Add material-specific wet and highlight response for water, lava, swamp, ice, and snow; avoid global shine on ordinary terrain.
- Add transition dirt or grime accumulation near material boundaries such as grass-to-road, snow-to-rock, cliff-to-ground, and plaza edges.
- Improve sprite grounding with soft contact shadows under statics and mobiles plus subtle side darkening for tall art.
- Replace white blob-like static light emphasis with soft local color influence on nearby art and land, keeping visible decals optional.
- Add restrained elevation or distance atmosphere for large terrain views without obscuring tile readability.
- Add material-class micro-contrast controls so grass, rock, snow, roads, and liquid surfaces can separate without one global sharpening curve.
- Add normal-map validation views for KR/EC land families: water, lava, swamp, snow, rock, grass, and road. Compare `enable_normal_maps` on/off for channel orientation, BC7/RDO damage, and useful `land_normal_map_strength` ranges.
- Add material-family wet response beyond generic specular: water gets moving normal/ripple response, lava gets warmer emissive-style color lift, swamp gets low-contrast dark sheen, ice/snow gets cool grazing highlights only.
- Add light-color provenance for static lights so local land/art response uses the same hue/source color as the visible mask instead of the current warm fallback where practical.
- Add terrain shadow/compression pass for vertical cliffs and high banks: darken near steep height transitions and cliff bases without turning ordinary rolling hills into harsh normal-mapped PBR.
- Add roof/tree/large-static side falloff: tall art should receive subtle one-sided depth tint and base contact so it reads as part of the KR scene rather than a flat pasted sprite.
- Add optional texture-backed grunge/noise once the relevant KR/EC support texture is identified; keep procedural grunge as fallback and avoid applying the same noise scale uniformly to every material.
- Keep these effects modular and independently tunable in shader settings; each effect must be disableable for regression comparison.
- Preserve original UO/KR stylistic intent: no heavy bloom, screen-space outlines, modern PBR material treatment, or procedural noise pasted uniformly over every surface.

### Future-Readiness Preservation

- Preserve future blend candidates distinctly from generic support refs.
- Preserve `NormalLike`, distortion-like, and flow-like candidates distinctly.
- Preserve extra-pass candidates such as waterfall, splash, foam, flare, glow, and lava bubbles.
- Add an implementation-readiness field for preserved support refs with categories like:
  - `base-safe`
  - `blend-ready`
  - `liquid-ready`
  - `normal-like experimental`
  - `extra-pass only`

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

- Implement the 1:1 tiled world export pipeline.
- Add WebP output profile controls for quality, lossless, and speed.
- Add CLI pack validation for UDDP consistency and dictionary coverage.
- Improve `uddp_inspector` atlas preview with selective CPU decode for BC7 sub-rects instead of full-page decode.

## Validation

### Immediate Regression Protection

- Add parser tests for:
  - tileart `Textures`-family classification
  - stable-role classification
  - speculative-role tagging
  - owner-reference census correctness
- Add packer tests for:
  - sidecar integrity
  - auxiliary-package owner linkage
  - liquid base retention
- Add metadata round-trip and bit-packing tests where format packing is critical.
- Add UDDP alignment and `mmap` safety tests for package layout.
- Keep RLE or decode fuzzing for high-risk decode paths.

### Priority Test Inventory

- `test_metadata_packing_roundtrip`
- `test_uddp_alignment_verification`
- `test_mmap_block_access`
- `test_iso_depth_sorting`
- `test_subtile_precision_bias`
- `fuzz_rle_decoder`
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

- Full lighting-model parity with original client time-of-day rules.
- Networking protocol integration.
- Editor UX for live map sculpting and tile painting.

## Suggested Execution Order

1. Finish baseline instrumentation and EC evidence, metadata, and packaging cleanup.
2. Close the remaining EC runtime routing and chosen-base correctness gaps.
3. Finish the static art pipeline and bounded upload discipline.
4. Lock in audit, override, and review surfaces.
5. Continue runtime staging in narrow, falsifiable slices only.
6. Harden regression coverage with parser, packer, inspector, and screenshot-family validation.
7. Resume broader roadmap items such as clipmaps, streaming hardening, mobiles, export, and advanced camera modes once the EC material pipeline is stable.
