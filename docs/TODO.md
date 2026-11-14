# UODynamapper Canonical TODO

This file is the single source of truth for remaining work.

It intentionally excludes work that is already landed or already reduced to completed investigation notes.

## Targets And Constraints

Primary targets:
- 400+ FPS in military orthographic gameplay mode.
- 144+ FPS minimum in advanced perspective features.
- 60+ FPS at fullscreen on integrated GPUs, in every preset. Not met yet: `docs/agent_notes/renderer_release_performance.md`.
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

- [ ] Close remaining EC material parity and runtime cleanup gaps.
- [ ] Finish the production static-art pipeline and explicit UO-style static depth follow-up.
- [ ] Harden UDDP packaging, streaming, and validation.
- [ ] Move from transitional terrain scaling toward clipmap-based terrain.
- [ ] Expand mobiles, paperdoll, and export tooling.

## Now

### Current Renderer Focus: Visual Fidelity

The KR-only artwork fallback and current package handoff are integrated; see
[the source/native acceptance](reviews/2026-11-01-renderer-art-handoff.md).
Static inspection now follows composed material coverage and rendered depth;
see [the visibility-query acceptance](reviews/2026-11-01-visible-static-picking.md).
Terrain cursor/click teleport now intersects admitted displaced terrain and keeps
original signed Z; see [the terrain-query acceptance](reviews/2026-11-01-terrain-cursor.md).
Continue individual geometry, animation and material-parity work.
Older captures apply to their recorded package revisions.

- Validate remaining individual static anchors/new-art scales and effect visibility against matched fixtures. Common legacy pixel/height calibration, the perspective bridge stair/deck tie-bias defect and stale hover quad construction are repaired.
- Validate remaining individual/degenerate sprite windows and anchors against [the descriptor contracts](agent_notes/ec_sprite_descriptor_geometry.md). Required per-owner source scales, positive ordinary world-window edges and matching art/tilemeta rebuilding are implemented; retain the visible legacy-crane and scaled-plant controls.
- Effect reachability is now checked for grunge, KR boundary grime/color wash and projected shadows. Keep remaining source/strength parity distinct from inactive controls; [the effect audit](agent_notes/effect_visibility_validation.md) records normal-layer eligibility and the morning lighting cap's suppressed snow response.
- Validate broader camera/facet/large-art/multi coverage of terrain and static queries. Terrain cursor/teleport follows displaced triangles and original signed Z; static inspection uses shared GPU material coverage and rendered depth, with a stated exact-coincident-opaque tie limit. Use original logical surfaces and movement flags for future walking. See [terrain query contracts](agent_notes/terrain_cursor_surface.md) and [hover geometry contracts](agent_notes/static_hover_geometry.md).
- Investigate thin gold lines and translucent triangular artifacts over water with animated MSAA4x. The phase-2 bridge matrix and a separate pre-change binary/original-shader run both reproduce them; see [the picking review](reviews/2026-11-01-visible-static-picking.md).
- Validate wider equal-priority overlaps while keeping the repaired town and bridge histories as regression controls. Transparent batch sorting fixes the tree-edge defect; visible-set sprite tie assignment fixes the separate opaque bridge-edge tie across chunk boundaries. See [the depth notes](agent_notes/static_depth_ordering.md).
- Legacy Translucent source weights and blended-pass admission are repaired from original executable evidence; the isolated coast mark is verified original artwork. Keep [the matching-pair controls](agent_notes/classic_static_translucency.md); exact 5-bit/gamma composition remains parity work.
- Mandatory-trim art/tilemeta integration and the earlier 20-mask recovery have recorded validation. The subsequent source audit traced package differences to the fp/plain client split; plain Enhanced is canonical. Missing-source aliases now carry the client's `missing.tga`; KR-only art uses the actual Classic/missing-image fallback. The complete Classic atlas and matching current EC set are installed. Keep remaining material/reflection parity separate from this accepted source integration; see [the corrected coverage note](agent_notes/tex_aux_ec_coverage.md).
- Use the focused contracts and acceptance reports linked above. Historical repaired items must reproduce before being reopened.

### Baseline And Instrumentation

- Add frame-time breakdown telemetry for CPU update, extraction, upload, and render.
- Add upload statistics for queued writes, bytes per frame, and batch count.
- Add cache diagnostics for LRU hit rate, eviction reason, hysteresis counters, and queue backlog depth.
- Define golden benchmark scenes and reproducible camera paths.
- Make metrics visible in the debug overlay and persist them in logs for A/B runs.
- BC7 pre-RDO audit: solid blocks (the empty and gutter area of a page) already take the encoder's solid fast path in `pack_bc7_rgb`/`pack_bc7_rgba`; the remaining encode time is content blocks. Reusing repeated non-solid blocks is the open option.
- Re-measure BC7 RDO before relying on the older results in [proposal (d)](agent_notes/proposals/2026-11-01-data-track-open-items.md): they predate the mode 4 and alpha-plane fixes.

### EC Runtime: Open Correctness Gaps

- Surface-like routing has no open failures (census 2026-11-01, installed packages: 9222 Enhanced surface-like items, none left without an image). An item with a `tex_art_ec` slot draws its own image (`EcSurface`/`EcRegular`), which wins over any `ec_land` match, and one without falls back to Classic art first, so `ec_land` decides only the 261 water-shader liquid payloads. The former samples `1345` (cave floor, `UOStaticTerrainShader`) and `1395..1399` (stone roof, `UOSpriteShader` with `1000034`) draw their own tileart textures, as the client does; their support maps are all in `tex_aux_ec`. The 61 unmatched `UOWaterShader` items now use direct authored stages and native primary extents; see [item-water acceptance](reviews/2026-11-01-item-water-materials.md). The dead `unresolved_surface_like` counter and unreached non-liquid `TexLandEc` branch remain separate cleanup work.
  - Remove the unreached paths (user decision 2026-11-02, renderer phase): everything in `resolve_surface_like_tex_land_ec_slot_id` after the liquid match (texture-ref and legacy `ec_texture_id` lookup, provenance canonical/alias search, wet-water fallback, `cc_texture_id` slot), the non-liquid `TexLandEc` branch of `resolve_ec_static_visual_kind`, and the tests that cover them. Items draw their own maps, as in the client; check that no installed-package item changes.
- Keep the safe static runtime precedence:
  - a present tileart-owned EC image wins over an `ec_land` match
  - otherwise try Classic art before the terrain-liquid fallback
  - never route normal art through unsafe `selected_texture_id -> canonical_slot_id` shortcuts
- Preserve the safe `ec_land` runtime rule:
  - direct populated slot wins
  - only then fall back through provenance or canonical mapping
- Revisit runtime terrain-id override coverage, including the documented forest-family `196` case in `runtime_material_id_overrides`.
- Draw the rest of the bump terrain layer (`UOBumpMapTerrainLayer`, material 8): item 3 (luminance bump, du/dv) and item 4 (environment map). The land shader now draws only items 0..2 (colour A/B by the item 2 alpha); see `scripts/dev-helpers/ec-research/ghidra-terrain-layers/results/uosa_terrain_layers.md` for the stage setup.
  - Meanwhile `sample_ec_material_normal_world` (`sampling.wesl`) reads item 3 as a tangent-space normal for the lighting, with a repo-side `normal_sample.b < 0.35` guard; the client has no such normal. The guard never fires on the current packages (blue is 255 in every normal-role texel, lossless and BC7). Replace both with the client's du/dv perturbation of item 4.

### Static Art Pipeline And Residency

- Finish the production-grade clustered static-art pipeline.
- Validate art atlas storage choices by content, alpha fidelity and device support. RGBA8 and BC7 are used for art pages; BC1 (DXT1) is also used in other packages. Evaluate ASTC and ETC2 for mobile devices later; see [the format research](agent_notes/uddp_anim_format_requirements.md).
- Build metadata lookup per art id for atlas layer, UV rectangle, pivot or offset, and behavior flags.
- Keep metadata lookup O(1) and exact, with partial writes only when atlas slot assignment changes.
- Implement the alpha-mask plus depth-write shader path for clean overlap.
- Keep draw submission batched by material and atlas page or layer set rather than by object.
- Art packages of different page formats: the shared Enhanced/KR-like sprite atlas is then RGBA8 and every BC7 page is decoded on the CPU at upload (one startup warning names the repack). A second atlas array per page format would keep both packages as stored.

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
- Player-local building ceiling scan state (`_maxZ`, `_maxGroundZ`): done as the z-view limit below.
- Z-view limit (done as a hard cut; `scene/world/z_view.rs`, view setting `world_rendering.show_only_player_floor`, off by default). It follows ClassicUO's `UpdateMaxDrawZ` (GameSceneDrawingSorting.cs lines 57 to 212) in every style: statics and multi parts at or above `max_z` and roofs while hidden are not emitted (so not picked either), land above `max_ground_z` (set only under land, in a cave) is discarded in the land shader, and the light cover and light sprite scan take `max_z`. KR (UOKR.exe `0x0043c350`) tests the same two cells, then hides whole 20-z layers (`0x0043c950`, `0x0043c5b0`, `0x0043c7d0`): with a ceiling found, every layer above the player's own (`0x0043c9f0`; the ceiling's z is not used; traced 2026-11-14, see the lighting doc); the EC rule (UOSA.exe candidates `0x00474360`, `0x004776d0`, which test 0x20004) is not traced. Following KR's cut in KR-like is a proposal (it would replace ClassicUO's max z there). The Enhanced client's light cull (0x00449f60) reads its own limit at 0x00dba01c, set by 0x0044f090 the ClassicUO way: 0x80 (none); land over the player's cell above player z + 16 sets it to player z + 16; a static there above player z + 14, with none of 0x2020004 and not a Roof unless a Surface, sets it to its z; a Roof above player z + 14 in the next cell, with none of 0x2000204, sets it (through 0x0044eab0); it is then raised to at least player z + 16 (`CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md`, Lights under a roof or a floor). Follow-ups: fade instead of the hard cut (the alpha lerp above); tag cached static instances with z and roof so a cap change re-appends the cached chunks instead of rebuilding them; the webmap export always hides the caves; giving it the cave view needs a CLI option and a wait for the cave scan.
- Caves drawn over the map (done; `scene/world/caves.rs`, view setting `world_rendering.caves`, "Hidden Until Entered" by default, "Drawn Over the Map" as ReMapRenderer's `RENDER_UNDERGROUND`). A cell is a candidate when its highest floor static (Surface, neither Roof nor Impassable, a Bridge at half height) lies 16 z or more below its own land; the candidates flood through 8-neighbours whose floors are within a walking step (2 z), and a component shows only when one of its cells is a step from the walkable ground of an open neighbour, so basements reached by stairs and sealed floors stay hidden. Map 0 shows 12781 cells: the Minoc mine 470, north of Britain 899, the Britain basements 0, Buccaneer's Den 0 (its tunnels reach the open only by basement stairs, 5 to 7 z apart). A shown cell's land is drawn at the cave floor, its statics at or above floor + 16 are not drawn, and its statics below, with the buried statics of the cells around (the walls), are lifted toward the camera in depth; lights there take the z-view's cover for a player standing in the cave. Follow-ups: a cave whose only mouth is an item (a ladder, a teleporter) stays hidden; shadows of lifted statics are lifted with them; light sprites are not.
- Validate the result in dense town and building scenes with equal-Z walls, foliage near walls, and terrain-versus-land-static overlap.

## Next

### UDDP, UDDF, Streaming, And Upload Scheduling

- Keep UDDF as the single-object wrapper for standalone resources and tooling.
- Preserve the planned `u16` compression header layout:
  - bits `0..3`: base compression type
  - bits `4..6`: custom compression type
  - bits `7..15`: content type id
- Implement async read and decode tasks through Bevy task pools.
- Implement upload scheduling that merges writes and emits bounded GPU copy batches each frame.
- Evaluate per-content dictionaries for small payload classes where measurements justify them; map chunks use plain Zstd after comparison.
- Implement fallback decode paths per content type.
- Keep predictive prefetch rings optional until telemetry proves they improve real movement behavior.
- Keep live editing persistence and delta-merging deferred until editor and gameplay ownership is explicit.
- Keep any EC extra-pass exploration behind explicit feature gates and diagnostics.

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

### Classic Equipment And Mount Animation Sources

- Build the custom body and wear table with the animation format, and carry item-to-wearer rows (tileart filters, `Equipconv.def`, the table) in both
  animation packages. The design is in `docs/agent_notes/uddp_anim_format_requirements.md`, section 8.
- Carry the tile-to-appearance data and `IsMountBody`'s list in the Classic animation package, and add a reader call that
  takes the object drawn as well as the requested id. Traced in
  `scripts/dev-helpers/ec-research/ghidra-classic-frames/results/uop_reachability.md`. Do it with the animation format
  change, before the renderer draws Classic equipment.

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
- Continue texture-backed grunge/noise: first pass binds a generated repeatable texture to land/art/ground-art and blends it with procedural variation; remaining work is identifying the official KR/EC support texture and replacing the fallback image. The KR client's own grunge on statics (its `01000045` texture, mapped as the client does) is now the separate Client Grunge setting.
- Keep these effects modular and independently tunable in shader settings; each effect must be disableable for regression comparison.
- Preserve original UO/KR stylistic intent: no heavy bloom, screen-space outlines, modern PBR material treatment, or procedural noise pasted uniformly over every surface.

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

- Validate and complete Classic fidelity for manually placed terrain-like static transition art. Classic and Enhanced share the same map tiles, even though their storage differs.
- Finish the switch for dynamic terrain texture blending at transitions for both Classic and KR/Enhanced textures. Dynamic blending can reduce reliance on manually placed transition art.
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
- Light-source items glow on the Classic paperdoll and the old-style gump paperdoll (the user liked the idea,
  2026-11-06). The Enhanced client's own paperdoll (`Paperdoll.uop`) already has the halo painted into lit items'
  frames (lit lantern 0xA15: a 375x306 frame, mostly partly transparent), so it needs nothing. An item
  with tiledata's light-source flag (0x00800000) stores its light id in the quality byte; draw that light's texture from
  `world_lights.uddp` over the worn item, as a translucent layer that shows on any window background, with one strength
  setting in the app and no per-item numbers. Open: where on the item the light sits (the centre of its drawn area, or
  its brightest part). The lit lanterns use gump 50500 and the unlit ones 50503; the coloured lanterns 0x40FE..0x4101
  are light sources but use the unlit gump 50503, so the light would make them glow too. The only paperdoll gumps with a glow painted in, 50322
  and 60322, are used by no item, and the client draws that glow as an opaque dark disc.

### Export And Tooling

- Audit the UOCF editor’s inspection-only views and add edit and save workflows for client files it can display where feasible. Reuse existing format writers and CLI capabilities where appropriate; map painting remains deferred below.
- Finish remaining export acceptance and portability work (approved 2026-11-01: WebP method 4 q85 sharp YUV only; offscreen export mode after a control-channel pilot; Britannia first; 512 px tiles; Leaflet 1.9.4 vendored; z = 0 readout; sRGB box downsampling). [Web map export proposal](agent_notes/proposals/2026-11-01-web-map-export.md#decisions-2026-11-01).
  - Broader visual acceptance of the Classic Britannia export and CI builds on Windows, macOS and ARM64. See [export acceptance and remaining limits](reviews/2026-11-01-webmap-export-renderer.md).
  - Enhanced and KR exports (user decision 2026-11-02, renderer phase, after the format work): the export job can select the preset, with a choice between water reflections at a fixed reflection point and reflections off. Today the Enhanced preset cannot be chosen through an export job, and reflections differ between overlapping frames because they follow the camera.

## Validation

- Check remaining coverage for metadata packing, isometric depth ordering, subtile precision, and hue lookup; add behavioral regression tests where gaps remain.

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
- A sidecar for the Kingdom Reborn materials and subtypes that `uocf-cli facet convert` loses going KR → Classic (squares whose pair is not the shipped land map's for their Classic id; facet0 9,608, facet1 10,013, facet2 7, facet3 2,665, facet4 1,065), so a KR → Classic → KR round trip is exact; until then the conversion needs `--allow-lossy` (the user, 2026-11-01).
- Classic map editing in dynamapper (option C of the [map writers proposal](agent_notes/proposals/2026-11-01-classic-map-writers.md)): the writers, dif baking and `uocf-cli facet convert` are done (option B); the editor path (`.uddp` → Classic export or `MapPlane` dirty blocks) is still to decide.

## Suggested Execution Order

1. Continue visual fidelity checks and the specific EC material and routing cleanup above.
2. Address static depth and art pipeline gaps in narrow, validated slices.
3. Add instrumentation and harden streaming and bounded uploads.
4. Extend regression coverage and complete export acceptance.
5. Continue deferred and longer-term work as priorities and dependencies allow.
