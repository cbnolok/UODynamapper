## Plan: EC Material Evidence, Metadata, And Runtime Staging

Build a UOP-grounded EC material pipeline that preserves every potentially useful linked resource, fixes current base-texture selection without discarding future blend or liquid-support inputs, and stages runtime work from base-correctness to world-space repetition to authored solid-terrain blending to liquid support and only then to extra passes. The key correction to the previous draft is evidence discipline: original EC UOP contents are primary, extracted original Shaders.uop text is secondary when present, third-party reconstructed FX files are design references only, and current Rust or WGSL code is proof of current engine behavior, not proof of original EC intent.

**Evidence Tiers**
- Original EC package evidence: tileart.uop, TerrainDefinition.uop, Texture.uop, LegacyTexture.uop, TerrainTexture.uop, EffectTexture.uop, string_dictionary.uop, and any additional directly referenced package families discovered by full scans.
- Extracted original shader evidence: the currently extracted Shaders.uop texts, which so far confirm the simple sprite, hue, UI, death, and bloom or postprocess family only.
- Third-party proof-of-concept renderer evidence: single-terrain.fx, solid-terrain.fx, liquid-terrain.fx, and statics.fx from the external C# renderer. These are useful reconstruction clues, not original source.
- Current UODynamapper behavior: Rust parsers, packers, metadata sidecars, and WGSL shaders show what the current engine does, not necessarily what the EC originally did.
- Visual target evidence: supplied EC screenshots show expected end results for marsh water, desert cliffs, snow and waterfall fields, lava, roads, plazas, and terrain transitions.

**UOP Crosswalk And What Each Source Can Prove**
- tileart.uop
Why it matters: authoritative owner of art and static records, including flat surface-like art and liquid-like art.
What it can prove: art ownership, shader names such as UOWaterShader and UOStaticTerrainShader, flags, selected EC and CC art textures, full linked texture blocks, item-local source windows, and texture_stretch values.
What it cannot prove by itself: exact runtime consumption of every support texture.

- TerrainDefinition.uop
Why it matters: authoritative owner of terrain materials.
What it can prove: layered terrain materials, alias land ids, shader names such as UOWaterTerrainLayer, linked texture families, texture_repetition values, and terrain-owned material relationships.
What it cannot prove by itself: the final terrain or liquid composite math.

- Texture.uop
Why it matters: named build/worldart image pool.
What it can prove: visible image resources exist in a stable UOP-backed pool.
What it cannot prove by itself: whether a resource is a base, overlay, or support layer.

- LegacyTexture.uop
Why it matters: named build/tileartlegacy image pool.
What it can prove: visible image resources and Classic-like fallback content exist in a stable UOP-backed pool.
What it cannot prove by itself: semantic role.

- TerrainTexture.uop
Why it matters: support-resource pool, not just map terrain.
What it can prove: build/terraintexture images exist as first-class resources, including lighting-like and liquid-support-like assets.
What it cannot prove by itself: whether a resource is terrain-owned, art-liquid-owned, or shared.

- EffectTexture.uop
Why it matters: mixed effect-resource pool.
What it can prove: image assets, NIF meshes, EMS particle scripts, and text-like resources exist in the same package; some may matter for future liquid or support rendering and many matter for provenance.
What it cannot prove by itself: direct ownership by tileart or TerrainDefinition until the owner scans are done.

- string_dictionary.uop
Why it matters: authoritative path and shader-name resolution for tileart.uop and TerrainDefinition.uop.
What it can prove: exact string-level linkage for paths and shader names.

- Shaders.uop
Why it matters: authoritative client shader package, but only partially extracted in currently available evidence.
What it can prove so far: our postprocess_ec base sprite family is grounded in real extracted client shader text.
What it cannot yet prove: solid terrain blending or liquid material composition.

- Third-party proof-of-concept FX set
Why it matters: useful architectural clue set, explicitly not original EC source.
What it suggests:
1. single-terrain.fx suggests a simple one-layer world-space terrain sampling path.
2. solid-terrain.fx suggests two base textures plus one alpha mask, all sampled in world space with independent stretch values.
3. liquid-terrain.fx suggests a base liquid texture plus a normal-like perturbation texture, world-space sampling, wind-driven scrolling, WaveHeight, and a FollowCenter variant.
4. statics.fx suggests statics are architecturally separate from terrain shaders, which supports keeping art-owned wet entries in the art domain.
What it cannot prove: that the original EC used exactly the same shader math, parameters, or package mappings.

**Current Baseline To Freeze Before Changes**
1. Current item main-texture chooser.
Source of truth today: /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs.
Current order: prefer non-aux worldart ref matching tile id, then non-aux primary-selected worldart, then any non-aux worldart, then any non-aux primary-selected ref, then any non-aux ref, then legacy fallback to item.ec_texture_id.
Why freeze it: later changes need explicit before versus after reasoning.

2. Current terrain primary-layer chooser.
Source of truth today: /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs.
Current order: rank by support-layer status, then preferred repetition range, then unk ordering.
Why freeze it: it already encodes a guess about base versus support that may need to be replaced by a role-aware model.

3. Current packaging exclusions.
Source of truth today: /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs.
Current behavior: TileType::Liquid is excluded wholesale from tex_art_ec.
Why freeze it: this is currently one of the most consequential incorrect simplifications.

4. Current runtime routing.
Source of truth today: /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/scene/world/art/statics_collect.rs and /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs.
Current behavior: surface-like art can route to tex_land_ec via a sidecar-derived main_ec_texture_id and land provenance matching.
Why freeze it: this is the current insertion point for art-land correctness and later support-aware material routing.

5. Current shader behavior.
Source of truth today: /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/main.wgsl, /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl, /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/noise.wgsl, /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/normals.wgsl, and /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/surface_effects.wgsl.
Current behavior: one base albedo, world-space sampling for EC terrain, procedural blur and sharpen, geometric or bicubic heightfield normals, lighting and grading, and a generic wet UV animation fallback.
Why freeze it: it already partially matches the third-party world-space UV model, but not the authored multi-texture or liquid support model.

**Current Cross-Verified Facts**
1. tileart.uop contains liquid-like art ownership. Art-owned wet or liquid visuals must remain in the art domain even if they later consume TerrainTexture.uop or EffectTexture.uop support assets.
2. TerrainDefinition.uop contains layered terrain materials and explicit repetition values. This aligns conceptually with a Stretch-driven world-space sampling model.
3. TerrainTexture.uop is a support-resource pool. It can legitimately hold resources used by art-owned liquid entries as well as terrain-owned materials.
4. EffectTexture.uop must be preserved as metadata now. It contains image and non-image resources that may contribute future implementation clues even when not rendered directly.
5. UODynamapper already partially matches the third-party world-space UV model. /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl explicitly uses world_position.xz divided by stretch and repeat wrapping.
6. UODynamapper does not yet match the proof-of-concept authored multi-texture terrain blend. There is no current authored Texture0 plus Texture1 plus AlphaMask terrain material path.
7. UODynamapper does not yet consume authored normal-like maps for terrain or liquid. Current normals come from heights, not texture-space normal or ripple maps.
8. Current wet handling is only a generic IsWet UV animation fallback.
9. Current tex_art_ec drops TileType::Liquid entirely.
10. Current tileart classification still lacks Textures-style refs, which blocks correct art-owned liquid support preservation.

**Visual Target Families**
- Marsh and swamp water: broad green liquid, white highlights, dark breakup, shoreline variation.
- Desert canyons and sand transitions: broad-scale repetition, soft edge blending, low visible tiling.
- Snow and waterfall fields: broad white and blue materials, vertical streaking, bright watery highlights.
- Lava fields: emissive detail, strong breakup, likely animated support inputs.
- Urban roads and plazas: repeated materials with reduced tiling visibility.
- Grass to sand or dirt transitions: soft multi-material boundaries.
Use: these are validation targets for later runtime stages, not proof of exact source algorithms.

**Role Model**
- Stable first-pass roles
Use for immediate selection, packaging, reporting, and overrides.
Roles: Base, SecondaryBase, AlphaMask, GenericMask, Noise, Detail, Overlay, NormalLike, ImageSupport, EffectOnlyMetadata, UnknownSupport.

- Speculative second-pass roles
Use for preserved evidence, diagnostics, and later shader planning only.
Roles: LiquidRipple, LiquidReflectionSupport, LiquidEnvProbe, FoamHighlight, FlowMapLike, RefractionDistortionLike, WaterfallSupport, LavaBubbleSupport, PostBlendSupport.
Design choice: speculative roles must never silently control visibility or routing before stronger evidence exists.

**Steps**
1. Phase 0: Full owner-reference census from tileart.uop and TerrainDefinition.uop.
Why needed: this resolves the current uncertainty around which physical packages are directly referenced by owner records.
Substeps:
1. Scan every tileart.uop entry and every linked texture item path resolved through string_dictionary.uop.
2. Scan every TerrainDefinition.uop entry and every linked layer path resolved through string_dictionary.uop.
3. Normalize and bucket every discovered path by physical package family: Texture.uop, LegacyTexture.uop, TerrainTexture.uop, EffectTexture.uop, TerrainShaders-like or other shader-resource families, SystemTextures, and Unknown.
4. Emit a direct-reference table keyed by owner record and referenced package family.
5. Separate direct textual references from indirect NIF or EMS relationships.
Outcome: after this phase, it should no longer remain open which packages are directly referenced by tileart.uop and TerrainDefinition.uop; only indirect usage remains open.
Design choice: use a full scan rather than anecdotal examples so package membership becomes a measured fact.

2. Phase 1: Dedicated TerrainTexture.uop and EffectTexture.uop inventory pass.
Why needed: these are still the least-understood resource pools and the most likely future auxiliary sources.
Substeps:
1. Build one inventory tool for TerrainTexture.uop and one for EffectTexture.uop.
2. Output columns: physical package name, internal UOP path, normalized basename, inferred file kind, decodable image yes or no, image dimensions when decodable, logical family guess, stable role guess, speculative role guess, owner references discovered in phase 0, and notes.
3. Add dedicated report buckets for names such as ripple, water_alpha, cube, env, detail, noise, blur, splash, bubble, waterfall, lava, flare, glow, and mask.
4. Produce an anomaly summary counting unknown and unsupported entries.
Design choice: inventory the physical packages separately from owner scans so unresolved resources and owner-linked resources can be compared.
Depends on step 1.

3. Phase 2: Freeze current heuristics and expose them in diagnostics.
Why needed: later changes must be evaluated against a documented baseline.
Substeps:
1. Document the current item main-texture chooser from /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs.
2. Document the current terrain primary-layer chooser from /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs.
3. Document the current tex_art_ec Liquid exclusion from /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs.
4. Add a dedicated inspection output that shows every preserved ref, every rejected ref, the rejection reason, the final chosen base, and the heuristic reason.
5. Add a direct-routing debug surface for art-owned surface-like entries and terrain-owned materials.
Design choice: keep diagnostics explicit and queryable, not hidden inside logs.
Parallel with step 1 after step 0.

4. Phase 3: Split physical package, logical family, stable role, and speculative role.
Why needed: package, family, and role are separate axes and must remain separate.
Substeps:
1. Add a physical source package enum with values for Texture.uop, LegacyTexture.uop, TerrainTexture.uop, EffectTexture.uop, TerrainShaders-like or shader-resource families when found, and Unknown.
2. Add a logical family enum with values such as WorldArt, TileArtLegacy, TileArtEnhanced, Textures, Effects, SystemTextures, ShaderResources, and Unknown.
3. Add stable role and speculative role fields.
4. Add role reason and confidence fields.
5. Preserve all these dimensions on every ref.
Design choice: this prevents later confusion such as treating every TerrainTexture image as land-owned or every EffectTexture image as particle-only.
Depends on steps 0 and 1.

5. Phase 4: Extend tileart parsing to stop dropping Textures-style refs and preserve stretch clues.
Why needed: current tileart parsing still collapses valid support refs into Undefined.
Substeps:
1. Add tileart-side classification for Textures-style references parallel to TerrainDefinition’s current family detection.
2. Preserve raw normalized path and source string for every tileart ref.
3. Preserve texture_stretch as a first-class field in the later selection and material model, not only as a tile-type hint.
4. Add tests for art-owned liquid entries whose linked support textures live outside WorldArt.
Design choice: ownership comes from tileart.uop; support sources must not be reclassified as terrain ownership simply because they live in TerrainTexture.uop.
Depends on steps 0 through 3.

6. Phase 5: Promote multi-reference metadata to first-class runtime data.
Why needed: the renderer, inspectors, and generated artifacts must all see the full resource graph.
Substeps:
1. Extend the existing tilemeta sidecars to store physical package, logical family, stable role, speculative role, role reason, confidence, normalized name or path, raw path, decodable-image flag, metadata-only flag, block and item order, stretch or repetition, unknown fields, and owner linkage.
2. Add equivalent terrain-material sidecars for TerrainDefinition.uop layers.
3. Add metadata-only manifests for non-image EffectTexture.uop entries such as NIF, EMS, and text assets, preserving names and any discoverable related image references.
4. Preserve owner kind and owner id explicitly for every ref.
5. Version the sidecars and keep backward-compatible loading.
Design choice: keep hot-path dense records compact and move graph richness into sidecars.
Depends on steps 0 through 4.

7. Phase 6: Redesign package boundaries around ownership and role.
Why needed: base packages and support packages must not be conflated.
Substeps:
1. Keep tex_art_ec.uddp as the base image package for tileart-owned visible bases.
2. Keep tex_land_ec.uddp as the base image package for TerrainDefinition-owned visible bases.
3. Add ec_tex_aux.uddp as the image-bearing support package for non-base refs coming from Texture.uop, LegacyTexture.uop, TerrainTexture.uop, and EffectTexture.uop.
4. Store non-image EffectTexture.uop resources as metadata manifests associated with ec_tex_aux.uddp rather than bindable image slots.
5. Allow duplicated source images across packages when semantics differ.
Design choice: one auxiliary support package plus metadata manifests is simpler and safer than overloading base packages.
Depends on steps 1 through 5.

8. Phase 7: Replace the blanket tileart Liquid exclusion with role-aware visibility rules.
Why needed: art-owned wet or liquid entries can still have visible bases.
Substeps:
1. Remove the unconditional Liquid exclusion from tex_art_ec.
2. Replace it with a base-eligibility chooser that decides whether the entry has a visible base image.
3. Keep effect-only or no-base liquid entries out of tex_art_ec while preserving them in metadata and auxiliary manifests.
4. Add abnormality codes for liquid entry with no base, liquid entry with only support refs, liquid entry whose base came from a non-worldart family, and liquid entry with unresolved support-rich stack.
Design choice: visibility must depend on base eligibility, not only on tile_type.
Depends on steps 3 through 6.

9. Phase 8: Replace current base selection with stable-role selection plus heuristic reason logging.
Why needed: the current non-aux-first chooser is not sufficient.
Substeps:
1. Use stable roles only for first-pass base selection.
2. Make Base and SecondaryBase eligible by default.
3. Make AlphaMask, GenericMask, Noise, NormalLike, ImageSupport, EffectOnlyMetadata, and UnknownSupport ineligible by default.
4. Allow fallback only when it records a specific heuristic reason and emits a low-confidence abnormality.
5. Preserve speculative roles for later but never let them silently control visibility.
6. Add override hooks for base choice, role correction, and metadata-only tagging.
Design choice: safe first-pass selection must be explainable and overridable.
Depends on steps 5 through 7.

10. Phase 9: Add abnormality reporting, audit outputs, and manual overrides.
Why needed: this pipeline is exception-heavy and cross-package.
Substeps:
1. Add CSV or JSON abnormality reports with owner kind, owner id, candidate refs, chosen base, heuristic reason, confidence, abnormality code, severity, source package, logical family, and notes.
2. Cover at least missing base, only support refs, ambiguous multiple bases, unsupported effect-only references, unresolved package family, indirect-only support linkage, and runtime slot mismatch.
3. Add dedicated owner-reference audit outputs for tileart.uop and TerrainDefinition.uop.
4. Add manual override files modeled after existing KDL override patterns, but split into base-choice overrides, role overrides, metadata-only overrides, and ignore lists.
5. Generate a compact human-readable summary in addition to machine-readable outputs.
Design choice: keep audits machine-readable and overrides source-controlled.
Depends on steps 0 through 8.

11. Phase 10: Generate editable review artifacts from UOP-grounded evidence.
Why needed: human review must be anchored to original package evidence, not only to current code.
Substeps:
1. Extend the TerrainDefinition generator so it emits material ids, aliases, all linked layers, repetition or stretch, physical package origin, logical family, stable role, speculative role, confidence, and uncertainty notes.
2. Generate a companion tileart liquid and surface-like report listing art id, tile type, flags, base choice, support refs, source package, and heuristic reason.
3. Generate dedicated TerrainTexture.uop and EffectTexture.uop reference reports listing internal resources and discovered direct consumers.
4. Support curated overrides for base choice, role correction, and explicit metadata-only tagging.
Design choice: generated review artifacts reduce hidden heuristics and make drift visible.
Depends on steps 0 through 9.

12. Phase 10 Sub-Plan: Runtime staging from world-space repetition to blends and extra passes.
Why needed: runtime work should be decomposed into independently verifiable stages.
Substeps:
1. Phase 10A: Base-correctness stage.
Goal: render the correct chosen base only, using the richer metadata, with no authored blending yet.
Scope: keep current land and art shaders mostly intact; switch only the chosen base source and inspection outputs.
Why first: it isolates selection errors from later shading complexity.

2. Phase 10B: World-space repetition stage.
Goal: make all repeatable terrain-like materials respect world-space repetition and stretch consistently.
Scope: preserve current /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl path, compare derived stretch from texture extent against decoded repetition or stretch metadata, and decide how explicit per-material stretch should override inferred size-based stretch.
Why second: it is the minimum stage needed to stop squeezing large textures into single tiles.

3. Phase 10C: Single-terrain stage.
Reference clue: third-party single-terrain.fx.
Goal: support a simple one-base world-space material path where appropriate.
Scope: useful as a clean baseline for materials that do not actually need multi-texture blending.
Why included: not every terrain material needs the full two-base blend path.

4. Phase 10D: Solid-terrain authored blend stage.
Reference clue: third-party solid-terrain.fx.
Goal: support two base textures plus one alpha mask, each with independent stretch, all sampled in world space.
Scope: TerrainDefinition-owned solid materials first.
Data requirements: stable roles must identify Base, SecondaryBase, and AlphaMask candidates; overrides must exist for ambiguous materials.
Why after 10B: correct world-space repetition must be solved before authored multi-texture blend can be debugged.

5. Phase 10E: Liquid-terrain support stage.
Reference clue: third-party liquid-terrain.fx.
Goal: support one liquid base texture plus one normal-like or ripple-like perturbation texture, world-space sampling, and animated scrolling or centered variants as needed.
Scope: TerrainDefinition-owned liquid materials first.
Data requirements: preserve base, normal-like or ripple-like support, wind or motion parameters when inferable, and explicit uncertainty when parameters are unknown.
Why after 10D: liquid support is more speculative and needs the base material pipeline stabilized first.

6. Phase 10F: Art-owned wet or liquid stage.
Goal: add analogous support-aware handling in worldmap art shaders for tileart-owned wet or liquid entries.
Scope: worldmap art main path for regular art and worldmap art ground path for surface-like art.
Reference clue: tileart ownership plus third-party statics.fx separation.
Why separate from terrain liquid: ownership and geometry paths differ, even if some support resources overlap.

7. Phase 10G: Extra-pass and post-lighting exploration stage.
Goal: evaluate whether preserved resources imply additional shading passes for highlights, reflection-like overlays, waterfalls, foam, splash, lava bubbles, or post-lighting enhancement.
Scope: only after previous stages validate base correctness, repetition, and authored blends.
Why last: this is the highest-uncertainty and highest-complexity stage.
Design choice: stage runtime changes so each stage has a falsifiable visual target and does not confound the others.
Depends on steps 2 through 10.

13. Phase 11: Clarify how the third-party world-space UV report maps into our data model.
Why needed: the third-party report is useful but reconstructed, not original.
Substeps:
1. Explicitly record that the proof-of-concept renderer is a third-party best-guess reconstruction, not original EC source.
2. Map its concepts to our evidence: Stretch aligns conceptually with TerrainDefinition.uop texture_repetition and tileart.uop texture_stretch, but is not yet proven to map one-to-one.
3. Record that UODynamapper currently derives stretch from texture extent in /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl, while the proof-of-concept uses explicit per-material stretch parameters.
4. Preserve both decoded repetition values and actual texture extents so later comparison can decide the correct precedence rule.
5. Record that UODynamapper already uses 1.0 x 1.0 tile footprints and carries 7.5-derived pixel-to-world scaling clues for art, but terrain vertical scaling still needs explicit review against the external renderer and client visuals.
Design choice: use the proof-of-concept as a design reference, not as already-proven source truth.
Depends on steps 0 through 10.

14. Phase 12: Preserve future implementation clues for blending, normal mapping, and extra passes.
Why needed: this work must keep data that later stages will need.
Substeps:
1. Preserve all support candidates needed for future Texture0 plus Texture1 plus AlphaMask blending.
2. Preserve all support candidates needed for future liquid base plus ripple or normal-like perturbation plus reflection or env-like highlights.
3. Preserve NormalLike, distortion-like, and flow-like candidates separately because the current runtime cannot yet consume them.
4. Preserve extra-pass candidates such as waterfall, splash, foam, flare, glow, and lava-bubble supports from EffectTexture.uop and TerrainTexture.uop.
5. Add an implementation-readiness field for every preserved support ref: base-safe, blend-ready, liquid-ready, normal-like experimental, extra-pass only.
Design choice: future rendering richness should be enabled by preserved evidence, not by another archaeology pass later.
Depends on steps 0 through 11.

15. Phase 13: Validation, fixtures, and screenshot-family regression buckets.
Why needed: the pipeline and runtime will otherwise drift silently.
Substeps:
1. Add parser tests for Textures-family classification in tileart, stable-role classification, speculative-role tagging, and owner-reference census correctness.
2. Add packer tests for sidecar integrity, auxiliary-package owner linkage, and liquid base retention.
3. Extend current inspectors to print owner kind, source package, logical family, stable role, speculative role, heuristic reason, confidence, and runtime routing.
4. Add regression fixtures for marble floors, cave floors, marsh water, lava, blood stains, waterfall or snow scenes, roads and plazas, and grass-to-sand transitions.
5. Validate by screenshot family rather than isolated ids only.
Design choice: content-family validation matches the actual visual goals better than only tile-by-tile micro-tests.
Parallel with later runtime stages once steps 8 through 12 are in place.

**Relevant Files**
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs — tileart ownership parsing, texture_stretch capture, and current family-classification gap.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs — TerrainDefinition layering and texture_repetition capture.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/textures.rs — reusable named UOP image loader.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/classic/light.rs — proof that TerrainTexture.uop already acts as a support-resource pool.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs — existing item texture-ref sidecar emission.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs — current runtime-side item texture ref loading and main chooser.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs — current art packer and Liquid exclusion.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_land_ec.rs — terrain packer and provenance storage.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/src/tool_cli.rs — inventory, report, export, and audit extension seam.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/src/pack_cli.rs — new inventory, report, and generator command entry points.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs — existing tile-level inspection seam.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_terrain_candidates.rs — terrain-layer inspection seam.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl — current world-space UV land sampling implementation.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/main.wgsl — current single-base terrain shading pipeline.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/noise.wgsl — current procedural noise helpers, not authored terrain masks.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/normals.wgsl — current geometric and bicubic terrain normals, not texture normal maps.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/lighting.wgsl — current lighting, grading, and tonemap structure.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/shading.wgsl — current shading modes and post-lighting structure.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/surface_effects.wgsl — current generic wet fallback.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/art/main.wgsl — future art-owned wet base and support seam.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/art/ground.wgsl — future surface-like art-owned wet seam.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/scene/world/art/statics_collect.rs — current art routing, wet flags, and pixel-to-world conversion clues.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs — future live inspection surface.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/cc_ec_convtables/TerrainDefinition.kdl — curated terrain material review surface and future merge target.
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl — override precedent.

**Verification**
1. Run the full owner-reference census before changing heuristics so direct package references are measured, not guessed.
2. Produce full TerrainTexture.uop and EffectTexture.uop inventories with direct-owner linkage columns.
3. Add parser tests for tileart-side Textures classification and liquid-support preservation.
4. Add regression tests proving that liquid tileart entries are preserved and audited even when no visible base exists.
5. Rebuild tilemeta.uddp and inspect chosen base, source package, logical family, stable role, speculative role, heuristic reason, and confidence for representative water, lava, blood, snow, road, and floor samples.
6. Rebuild tex_art_ec.uddp and confirm that visible art-owned liquid entries can survive base packing while support refs move into ec_tex_aux metadata or pages.
7. Build ec_tex_aux.uddp and verify that image-bearing support resources are packed while NIF and EMS resources remain metadata-only.
8. Keep runtime validation incremental: base correctness first, world-space repetition second, authored solid blend third, liquid support fourth, art-owned wet support fifth, and extra passes last.

**Decisions**
- Treat original UOP package contents and extracted asset names as stronger evidence than current Rust heuristics.
- Treat the third-party FX files as best-guess reconstruction evidence only, not as original EC proof.
- Resolve direct package references with a full tileart.uop and TerrainDefinition.uop scan rather than leaving them as open questions.
- Bring back explicit diagnostics, overrides, generated review artifacts, and validation work as first-class phases, not optional follow-up.
- Keep one auxiliary image package, ec_tex_aux.uddp, plus metadata-only manifests for non-image support resources.
- Preserve clues for blending, normal mapping, and extra passes now, even when runtime support is deferred.

**Open Points**
1. The extracted original Shaders.uop evidence still lacks the original solid-terrain or liquid-terrain shader texts.
2. The exact mapping from TerrainDefinition.uop texture_repetition and tileart.uop texture_stretch to a final Stretch parameter remains open.
3. The exact role of ripple, cube-like, env-like, and sphere-like resources remains open.
4. After the full owner-reference census, what should remain open is only indirect usage through NIF, EMS, shader-resource packages, or extra passes.

## Phase 1 Agent Handoff

Assumption: “Phase 1” refers to the plan phase named “Dedicated TerrainTexture.uop and EffectTexture.uop inventory pass”. This handoff assumes the agent is responsible only for that phase and for consuming the already-agreed evidence hierarchy, not for redesigning later phases.

### Objective
Produce a complete, machine-readable and human-reviewable inventory of TerrainTexture.uop and EffectTexture.uop resources, cross-linked to direct owner references discovered in tileart.uop and TerrainDefinition.uop, so later phases can stop guessing about auxiliary-resource coverage.

### Scope
Included:
- inventory TerrainTexture.uop contents
- inventory EffectTexture.uop contents
- classify file kind at least as image, nif, ems, text, shader-resource-like, unknown-binary
- record decodable image dimensions when possible
- record normalized basename and internal path
- cross-link each resource to direct textual references from tileart.uop and TerrainDefinition.uop
- emit anomaly buckets for important names such as ripple, water_alpha, cube, env, detail, noise, blur, splash, bubble, waterfall, lava, flare, glow, mask
- produce both machine-readable output and a concise human summary
Excluded:
- no runtime rendering changes
- no shader implementation changes
- no role-system redesign beyond provisional inventory labeling
- no packing into ec_tex_aux.uddp yet
- no consumption of NIF or EMS by the particle system

### Evidence Rules
Use this precedence when reasoning:
1. original UOP package contents and string_dictionary.uop-resolved paths
2. extracted original asset names from TerrainTexture.uop, EffectTexture.uop, and Shaders.uop
3. current Rust parser and loader behavior only as evidence of present engine behavior
4. third-party proof-of-concept FX files only as reconstruction clues, never as proof of original EC intent
If evidence conflicts, preserve the conflict in output notes instead of forcing one interpretation.

### Required Inputs
Workspace code and data:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/textures.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/classic/light.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/src/tool_cli.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/src/pack_cli.rs
External evidence directories:
- /mnt/dati/__cloud/mega/_proj/__nonmio/Mythic-Package-Editor-EC-/Mythic Package Editor bin/Output/effecttexture_ec
- /mnt/dati/__cloud/mega/_proj/__nonmio/Mythic-Package-Editor-EC-/Mythic Package Editor bin/Output/shaders_ec
Third-party reconstruction reference, for naming clues only:
- /mnt/dati/__cloud/mega/_proj/__nonmio/UOClient-stefanomerotta/UOClient/Content/shaders

### Mandatory Cross-Linking Work
The agent must do a full direct-reference census from owner records before finalizing the inventory:
- scan every tileart.uop-linked texture path resolved through string_dictionary.uop
- scan every TerrainDefinition.uop-linked layer path resolved through string_dictionary.uop
- normalize discovered paths and bucket them by physical source package family
- link each TerrainTexture.uop or EffectTexture.uop resource to the owner records that reference it directly, if any
- distinguish direct textual owner references from merely suggestive name similarity and from indirect NIF or EMS relationships
This phase should reduce the open question from “which resources are referenced by owners?” to “which resources are only indirectly used?”

### Expected Outputs
The agent should produce at least these artifacts:
1. one machine-readable TerrainTexture inventory file
2. one machine-readable EffectTexture inventory file
3. one machine-readable direct owner-reference census file covering tileart.uop and TerrainDefinition.uop
4. one concise human summary of key findings and unresolved items
Preferred formats:
- CSV or JSON are both acceptable for machine-readable outputs
- markdown summary is acceptable for the human report
If choosing CSV, keep one row per resource. If choosing JSON, keep one object per resource and a stable top-level schema.

### Required Columns Or Fields
For each inventoried resource include:
- physical_package
- internal_uop_path
- normalized_basename
- inferred_file_kind
- decodable_image
- image_width
- image_height
- logical_family_guess
- provisional_stable_role_guess
- provisional_speculative_role_guess
- direct_tileart_refs_count
- direct_tileart_ref_ids
- direct_terrain_definition_refs_count
- direct_terrain_definition_ref_ids
- reference_confidence
- notes
For owner-reference census output include:
- owner_kind
- owner_id
- shader_name if present
- source_path_raw
- source_path_normalized
- inferred_physical_package
- inferred_logical_family
- linked_resource_basename
- direct_reference_confidence
- notes

### Provisional File Kind Rules
Minimum required categories:
- image
- nif
- ems
- text
- shader_resource_like
- unknown_binary
Use container path, extension, and decodability as evidence. Do not overfit speculative semantics into file kind.

### Provisional Role Rules
Keep these provisional only. This phase is not allowed to harden them into runtime policy.
Stable-role guesses may include:
- Base
- SecondaryBase
- AlphaMask
- GenericMask
- Noise
- Detail
- Overlay
- NormalLike
- ImageSupport
- EffectOnlyMetadata
- UnknownSupport
Speculative-role guesses may include:
- LiquidRipple
- LiquidReflectionSupport
- LiquidEnvProbe
- FoamHighlight
- WaterfallSupport
- LavaBubbleSupport
- PostBlendSupport
If evidence is weak, prefer UnknownSupport plus a note over a confident but fragile guess.

### Required Special Cases To Call Out
The human summary must explicitly discuss:
- whether ripple-like names occur in TerrainTexture.uop, EffectTexture.uop, both, or neither
- whether cube-like, env-like, sphere-like, or reflection-like names occur and where
- whether water_alpha-like and mask-like names occur and whether they are directly owner-referenced
- whether EffectTexture.uop images appear to be directly referenced by tileart.uop or TerrainDefinition.uop
- which EffectTexture.uop resources are only indirect or metadata-like because they are reachable only through NIF or EMS content
- whether any TerrainShaders-like or shader-resource package families appear in direct owner references

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. every TerrainTexture.uop and EffectTexture.uop resource visible through available tooling is inventoried
2. every direct textual owner reference from tileart.uop and TerrainDefinition.uop is counted and linked
3. direct references are separated from indirect or merely suggestive relationships
4. machine-readable outputs exist and are stable enough to diff later
5. the summary clearly states what is now proven, what remains open, and what later phases can rely on
6. the summary explicitly avoids claiming original-EC shader semantics from the third-party proof-of-concept FX files

### Nice-To-Have Additions
If cheap and low-risk, also include:
- frequency tables by basename token such as water, lava, ripple, cube, env, blur, splash
- a list of directly owner-referenced resources with no decodable image payload
- a list of decodable images with no direct owner references
- a confidence histogram for inferred package family matching

### Suggested Work Order
1. inspect current loader and parser seams in tileart.rs, terrain_definition.rs, textures.rs, and light.rs
2. build the owner-reference census first
3. inventory TerrainTexture.uop and EffectTexture.uop second
4. join census results onto the inventories
5. generate anomaly buckets and summary last
This ordering matters because inventory without owner linkage is less useful than a fully cross-linked result.

### Final Instruction To The Agent
Do not redesign later phases while doing this work. Finish Phase 1 as a clean evidence artifact producer. Preserve ambiguity explicitly, prefer measured outputs over narrative claims, and leave later policy choices to later phases.

5. It remains open how much of the screenshot richness is due to authored blend layers, liquid perturbation, reflection-like support, extra passes, or post-lighting.
6. It remains open which preserved support resources are truly required for first useful liquid rendering versus later polish.

**Further Considerations**
1. Add confidence scores to every inferred role and every owner-resource link.
2. Add a small heuristic-reason enum for chosen bases so inspectors can explain whether the base came from owner match, repetition-ranked selection, explicit override, or fallback.
3. Add machine-readable and human-readable summaries for all audit outputs.
4. Add screenshot-family tags to future overrides and validation fixtures so decisions can be traced back to visual targets.

## Phase 2 Agent Handoff

Assumption: “Phase 2” refers to the plan phase named “Freeze current heuristics and expose them in diagnostics”. This phase starts after the Phase 1 evidence artifacts exist or at minimum after the agent can reproduce the same owner-reference facts locally. The agent for this phase is not redesigning the pipeline yet. The mission is to document, expose, and instrument the current behavior so later phases can change it safely.

### Objective
Freeze the current selection and routing heuristics in explicit, inspectable form, and add diagnostic surfaces that reveal how a chosen base texture or runtime route was determined. The phase must improve observability without silently changing policy.

### Scope
Included:
- document the current item main-texture chooser
- document the current terrain primary-layer chooser
- document the current tex_art_ec Liquid exclusion rule
- document the current art-land runtime routing logic
- document the current land and wet shader baseline behavior
- add explicit diagnostic outputs showing candidates, rejections, chosen result, heuristic reason, and confidence or fallback status where derivable
- extend existing CLI and overlay inspection surfaces where appropriate
Excluded:
- no replacement of the current chooser logic yet
- no semantic-role redesign yet
- no ec_tex_aux packing or runtime consumption
- no authored terrain blending implementation
- no liquid support shader implementation
- no changes whose primary effect is visual behavior rather than observability

### Phase Dependency Context
This phase depends conceptually on Phase 1 outputs, because diagnostics should refer to measured package ownership and direct-reference facts where available. However, if those outputs are not yet persisted in-tree, the agent may still complete this phase by documenting current code-path behavior and making it inspectable. Do not block on later phases.

### Evidence Rules
Use this precedence when explaining behavior:
1. current code paths are authoritative for what UODynamapper does today
2. original UOP package evidence is authoritative for what data exists, but not for current runtime behavior unless the runtime actually uses it
3. third-party FX files are never proof of current runtime behavior and should only appear in notes when contrasting current behavior with future planned behavior
If you find a mismatch between what the data could support and what the current code actually does, record the mismatch explicitly as a finding in diagnostics or docs.

### Required Inputs
Current behavior files:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_land_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/scene/world/art/statics_collect.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/noise.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/normals.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/surface_effects.wgsl
Supporting reference files:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/docs/LAND_TEXTURES_AND_TRANSITIONS.md
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/cc_ec_convtables/TerrainDefinition.kdl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl
Optional evidence from Phase 1:
- TerrainTexture.uop and EffectTexture.uop inventory outputs
- owner-reference census outputs

### Mandatory Heuristics To Freeze
The agent must capture, in explicit text and diagnostics, at least these current behaviors.

1. Item main-texture chooser.
Source of truth: lib/udd-assets/src/tilemeta.rs.
Current behavior to freeze:
- prefer non-aux WorldArt ref whose texture_id matches tile_id
- then non-aux primary-selected WorldArt ref
- then any non-aux WorldArt ref
- then any non-aux primary-selected ref
- then any non-aux ref
- then fallback to legacy item.ec_texture_id when needed
Deliverable: one concise explanation in docs or summary and one diagnostic output that shows which branch was taken for a given tile.

2. Terrain primary-layer chooser.
Source of truth: lib/uocf/src/enhanced/terrain_definition.rs.
Current behavior to freeze:
- rank by support-layer status
- then by preferred repetition range
- then by unknown ordering fields
Deliverable: one concise explanation and one inspection path that can show the ordered terrain candidates for a material or alias tile.

3. tex_art_ec Liquid exclusion.
Source of truth: lib/udd-conv/src/tex_art_ec.rs.
Current behavior to freeze:
- TileType::Liquid is excluded wholesale from tex_art_ec packing
Deliverable: one explicit note in summary or docs and one diagnostic or report surface that can identify affected art ids.

4. Surface-like art routing to tex_land_ec.
Source of truth: dynamapper/src/core/render/scene/world/art/statics_collect.rs and dynamapper/src/core/render/overlays/cursor_behavior.rs.
Current behavior to freeze:
- surface-like art can route via sidecar-derived main_ec_texture_id and land provenance matching
Deliverable: diagnostics that show the runtime slot resolution attempt, provenance match, and final routing decision.

5. Current land shader baseline.
Source of truth: the current worldmap land shader modules.
Current behavior to freeze:
- one base albedo
- EC world-space sampling for terrain
- derived stretch from texture extent
- geometric or bicubic heightfield normals
- procedural blur and sharpening
- grading, gloom, and tonemap
- generic IsWet UV animation fallback
Deliverable: short technical summary of what the current land shader does and does not do.

### Required Diagnostic Surfaces
At least one machine-readable and one human-facing diagnostic path should be improved or created.

1. CLI diagnostic surface.
Preferred seam: tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs or a nearby example or tool command.
Minimum output requirements for a queried tile or terrain material:
- owner kind if known
- chosen base texture id
- heuristic reason or exact branch taken
- all preserved refs currently visible to the runtime
- which refs were rejected as support or otherwise ineligible
- whether tex_art_ec packing would include or exclude the tile
- whether tex_land_ec runtime routing succeeded and why

2. Overlay or live runtime diagnostic surface.
Preferred seam: dynamapper/src/core/render/overlays/cursor_behavior.rs.
Minimum output requirements for hovered or inspected tiles when practical:
- chosen base texture id
- item main-texture branch taken
- surface-like or regular-art classification
- tex_art_ec slot presence
- tex_land_ec direct slot presence
- tex_land_ec runtime slot presence
- routing outcome
This can be abbreviated compared to the CLI output, but it must expose enough to debug a wrong-floor or wrong-water case live.

3. Optional machine-readable audit output.
If cheap, emit JSON or CSV describing current chooser outcomes for a targeted sample set. This is useful but not mandatory if the CLI and overlay outputs are strong enough.

### Required Documentation Or Summary Output
The phase should leave behind a short, explicit freeze document or summary section that a later agent can cite. It must state:
- what the chooser does today
- what the terrain primary-layer ranking does today
- what tex_art_ec excludes today
- what the runtime routes today
- what the land shader currently implements today
- what the current system definitely does not implement yet
Examples of missing features that should be called out explicitly:
- no authored Terrain Texture0 plus Texture1 plus AlphaMask blend path
- no authored liquid base plus ripple or normal-like perturbation path
- no consumption of EffectTexture.uop resources in runtime shading

### Non-Goals And Guardrails
- Do not “improve” the chooser in this phase.
- Do not alter visible rendering behavior unless the change is a pure observability hook with no policy effect.
- Do not retrofit stable or speculative roles yet beyond using existing auxiliary flags or existing metadata.
- Do not turn third-party FX expectations into current-engine claims.
- If you find a likely bug, record it, but do not fix it unless the only change is to expose it diagnostically.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. the current item chooser is documented and inspectable
2. the current terrain primary-layer chooser is documented and inspectable
3. the current tex_art_ec Liquid exclusion is documented and queryable
4. the current runtime art to land routing path is documented and inspectable
5. the current land shader baseline is summarized with clear implemented versus missing capabilities
6. a later agent can reproduce a wrong-floor or wrong-water diagnostic without re-reading the entire codebase
7. no silent policy changes were introduced under the guise of diagnostics

### Suggested Work Order
1. read the current chooser and routing code first
2. write the freeze summary for yourself before editing any diagnostic surface
3. extend the CLI inspection path second, because it can be richer and cheaper to validate
4. extend the live overlay third if needed
5. finish with a narrow validation run or targeted inspection examples proving the new diagnostics expose the expected branches
This ordering matters because the hardest part of this phase is precision, not volume.

### Final Instruction To The Agent
Treat this as a baseline-capture phase. Your output should make later changes easier to reason about, not harder. Prefer explicit branch reporting over interpretation, preserve current behavior, and leave policy changes to Phase 3 and later.

## Phase 3 Agent Handoff

Assumption: “Phase 3” refers to the plan phase named “Split physical package, logical family, stable role, and speculative role”. This phase comes after the baseline heuristics are frozen and inspectable. It is a data-model phase, not a policy-rewrite phase. The agent must separate concepts that are currently conflated, but must not yet redesign visibility, routing, or shader behavior.

### Objective
Introduce explicit, durable metadata axes for every preserved EC texture or support reference so later phases can reason about source package, logical family, stable first-pass role, and speculative second-pass role independently. The goal is to eliminate hidden conflation such as “TerrainTexture means terrain-owned” or “EffectTexture means particle-only”.

### Scope
Included:
- add an explicit physical source package enum
- add an explicit logical family enum
- add explicit stable-role and speculative-role fields
- add role reason and confidence fields
- preserve these dimensions on every currently preserved reference surface that Phase 4 and later will build on
- thread the new fields through sidecar-writing or intermediate metadata structures where that is required to avoid lossy collapse
- add tests proving the axes remain independent
Excluded:
- no new selection policy based on the new roles yet
- no replacement of the current chooser yet
- no tileart Textures-family parser expansion yet, except what is strictly required to avoid naming dead ends in the type model
- no ec_tex_aux package creation yet
- no liquid-visibility policy change yet
- no runtime shader or material behavior changes

### Phase Dependency Context
This phase depends on the outcome of the owner-reference and inventory work conceptually, because those phases clarify which physical packages and logical families actually exist. It also depends on Phase 2 because the current heuristic baseline must already be frozen before introducing richer dimensions. However, this phase is still allowed to proceed with provisional enum members for package families already known from current evidence.

### Evidence Rules
Use this precedence when deciding what dimensions exist and what they mean:
1. original UOP ownership and package evidence for physical source package values
2. current parser semantics for logical-family meaning where already implemented
3. measured inventory and owner-reference outputs from earlier phases for validating enum membership and edge cases
4. third-party FX files only for speculative-role naming inspiration, never for hard classification rules
If evidence is weak, preserve Unknown or a low-confidence role with a reason instead of collapsing dimensions together.

### Required Inputs
Core metadata and parser surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_land_ec.rs
Likely supporting seams:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/tests/terrain_definition.rs
- any tileart or tilemeta tests already covering preserved refs
Optional previous-phase artifacts:
- Phase 0 owner-reference census output
- Phase 1 TerrainTexture and EffectTexture inventory output
- Phase 2 freeze summary and diagnostic terminology

### Dimensions To Introduce
You must keep these axes separate in the model.

1. Physical source package
Meaning: where the referenced resource physically comes from.
Required values at minimum:
- TextureUop
- LegacyTextureUop
- TerrainTextureUop
- EffectTextureUop
- TerrainShaderResourceLike or ShaderResourceLike
- SystemTextures
- Unknown
Requirement: this field must never imply ownership by itself.

2. Logical family
Meaning: the semantic family already implied by path shape or parser classification.
Required values should include at minimum:
- WorldArt
- TileArtLegacy
- TileArtEnhanced
- Textures
- Effects
- SystemTextures
- ShaderResources
- Unknown
Requirement: logical family may differ from physical source package and must stay distinct.

3. Stable role
Meaning: safe first-pass role used later for packaging, base selection, reporting, and overrides.
Required values at minimum:
- Base
- SecondaryBase
- AlphaMask
- GenericMask
- Noise
- Detail
- Overlay
- NormalLike
- ImageSupport
- EffectOnlyMetadata
- UnknownSupport
Requirement: this field will matter later for selection, but this phase must not yet change selection behavior.

4. Speculative role
Meaning: preserved hypothesis for future liquid and advanced-material work only.
Suggested values at minimum:
- LiquidRipple
- LiquidReflectionSupport
- LiquidEnvProbe
- FoamHighlight
- FlowMapLike
- RefractionDistortionLike
- WaterfallSupport
- LavaBubbleSupport
- PostBlendSupport
Requirement: speculative roles must never silently control current visibility or routing.

5. Role reason and confidence
Meaning: explain why a role was assigned and how trustworthy that assignment is.
Requirement:
- preserve a reason string or reason enum
- preserve a confidence value or coarse confidence bucket
- low confidence must be normal and acceptable for ambiguous refs

### Mandatory Design Constraints
- Package, family, stable role, and speculative role are independent axes.
- Unknown is a valid state on any axis.
- A TerrainTexture-sourced image may still be art-owned support, not land-owned base.
- An EffectTexture-sourced image may still be image-bearing support, not necessarily particle-only.
- A WorldArt logical family ref may still end up as ImageSupport rather than Base.
- No axis is allowed to be inferred from another by default unless explicitly justified and documented.
- If existing types or serialization formats cannot express independence, refactor them now rather than encoding temporary shortcuts.

### Expected Code Changes
The exact files may vary, but the implementation should likely touch these surfaces.

1. Shared metadata structs and enums
Likely surfaces:
- lib/udd-assets/src/tilemeta.rs
- lib/udd-conv/src/tilemeta.rs
Potentially parser-side structs in:
- lib/uocf/src/enhanced/tileart.rs
- lib/uocf/src/enhanced/terrain_definition.rs
Goal: preserve the new axes in a way that later phases can consume without lossy translation.

2. Serialization and sidecar compatibility
If current sidecars already encode texture refs, extend them with versioned fields rather than replacing them ambiguously.
Requirement:
- keep backward-compatible loading where practical
- if backward compatibility is not practical, make the version break explicit and documented
- do not silently repurpose an old field to mean something broader

3. Inspection surfaces
If the Phase 2 diagnostics already exist, extend them only enough to expose the new axes. Do not redesign output shape unnecessarily, but the new metadata must become inspectable.

### Required Tests
Add focused tests proving the axes remain independent.
Minimum coverage:
1. one case where physical package and logical family differ meaningfully
2. one case where logical family and stable role differ meaningfully
3. one case where stable role is UnknownSupport but speculative role is populated
4. one case where confidence is intentionally low and preserved
5. one round-trip or serialization test proving the new fields survive sidecar writing and loading
Do not rely only on doc assertions. Add executable checks.

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- which new enums or fields were introduced
- where they are stored
- which old assumptions are now invalid because the axes are separated
- which policy decisions are still deferred to later phases
It must explicitly state that this phase does not yet change chooser behavior or visibility rules.

### Non-Goals And Guardrails
- Do not use the new roles to alter tex_art_ec inclusion yet.
- Do not use the new roles to alter main_ec_texture_id selection yet.
- Do not reclassify ownership based on source package alone.
- Do not force speculative roles when stable-role evidence is weak.
- Do not add convenience fallbacks that collapse family into package or role into family “for now”.
- Do not drift into Phase 4 parser-expansion work except for tiny type-shape adjustments that are unavoidable.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. every preserved ref can carry physical package, logical family, stable role, speculative role, role reason, and confidence
2. those axes are demonstrably independent in tests or round-trip validation
3. diagnostics or inspection output can show the new axes for representative refs
4. no current chooser or visibility policy changed as an accidental side effect
5. later phases can build role-aware selection on top of these fields without redefining the schema again

### Suggested Work Order
1. map the current ref structs and identify exactly where conflation exists
2. define the new enums and fields before editing any chooser code
3. thread the new fields through parser-side and sidecar-side structs with the smallest viable schema changes
4. extend diagnostics just enough to expose the new axes
5. validate with narrow tests and one representative inspection output
This ordering matters because the value of this phase is structural clarity, not visible behavior.

### Final Instruction To The Agent
Treat this phase as schema hardening. Your job is to make later policy work possible without another metadata rewrite. Prefer explicit Unknown plus a reason over premature certainty, keep the axes independent, and do not let the richer schema accidentally start controlling rendering yet.

## Phase 4 Agent Handoff

Assumption: “Phase 4” refers to the plan phase named “Extend tileart parsing to stop dropping Textures-style refs and preserve stretch clues”. This phase comes after the baseline freeze and after the metadata model can represent package, family, and role as separate axes. This is primarily a parser-preservation phase. The agent must increase fidelity of tileart-derived references without yet redesigning chooser policy, runtime routing, or shader behavior.

### Objective
Extend tileart parsing so tileart-owned entries stop losing valid support references that belong to Textures-like families, and preserve stretch-related clues as first-class metadata rather than incidental parsing leftovers. The goal is to keep tileart ownership intact while preserving enough evidence for later liquid, support-texture, and authored-material phases.

### Scope
Included:
- add tileart-side classification for Textures-style references parallel to TerrainDefinition-side family detection
- preserve raw source path and normalized path for every tileart reference
- preserve texture_stretch as an explicit field that survives parsing and later metadata transfer
- ensure tileart-owned liquid or wet entries can preserve linked support refs even when those refs are not WorldArt
- add tests covering art-owned liquid or support-heavy entries whose linked resources live outside WorldArt
Excluded:
- no role-aware visibility redesign yet
- no replacement of current base-selection heuristics yet
- no ec_tex_aux packaging yet
- no runtime shader consumption of the new refs yet
- no ownership reassignment from tileart to terrain merely because a ref points into TerrainTexture.uop
- no broad sidecar redesign beyond what is strictly necessary to carry the newly preserved parser fields forward

### Phase Dependency Context
This phase depends on earlier schema work because package, family, role, reason, and confidence should already be representable independently. It also depends on the owner-reference evidence from earlier phases so the agent has concrete examples of Textures-style refs and mixed-package ownership. However, this phase should remain implementable even if those artifacts are not committed in-tree, as long as the current parser seams and known evidence are available.

### Evidence Rules
Use this precedence when deciding how to preserve tileart refs:
1. tileart.uop ownership is authoritative for ownership of the entry
2. string_dictionary.uop-resolved paths are authoritative for the referenced path text
3. current TerrainDefinition-side family detection is a precedent for Textures-style classification, not a justification to transfer ownership
4. current runtime behavior is not proof that a ref is unimportant; parser preservation should err toward keeping evidence rather than dropping it
5. third-party FX files are not evidence for parsing rules
If unsure whether a ref is a visible base, support map, or effect-only resource, preserve it with low confidence instead of collapsing it to Undefined or discarding it.

### Required Inputs
Primary parser and metadata surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
Likely validation or inspection surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
- existing tileart and terrain_definition tests under lib/uocf/tests
Optional earlier-phase evidence:
- owner-reference census output showing tileart references outside WorldArt
- TerrainTexture.uop and EffectTexture.uop inventory outputs
- Phase 2 diagnostic outputs describing currently dropped or rejected refs

### Mandatory Parser Improvements
The implementation must address all of the following.

1. Textures-style family recognition on the tileart side
Current problem:
- tileart parsing still collapses valid non-WorldArt support refs into Undefined or an equivalent lossy bucket
Required outcome:
- tileart-side classification must recognize Textures-style references in a way parallel to TerrainDefinition’s existing family detection where appropriate
- the result must preserve package and family information without changing owner identity
Guardrail:
- recognizing a Textures-style ref must not imply the tile becomes terrain-owned

2. Raw path preservation
Current problem:
- some downstream reasoning currently depends on normalized or simplified values, which loses forensic detail
Required outcome:
- preserve the raw source string and a normalized path for every tileart ref
- preserve enough information to reconstruct what the original resolved path looked like
Guardrail:
- normalization must not overwrite or replace the original raw value

3. Stretch preservation
Current problem:
- texture_stretch currently exists more as a clue than as a preserved first-class field across the later model
Required outcome:
- preserve texture_stretch explicitly on tileart-derived refs or owner-linked metadata
- ensure the field survives far enough that later phases can use it for selection and material planning
Guardrail:
- do not reinterpret stretch semantics beyond preserving the parsed value and its provenance

4. Liquid and wet art support preservation
Current problem:
- art-owned liquid or wet entries may have linked support refs that do not belong to WorldArt and therefore get lost or underclassified
Required outcome:
- tileart-owned liquid or wet entries must preserve linked support refs even when they come from Textures-style or other non-WorldArt families
- later phases can decide visibility; this phase must decide preservation
Guardrail:
- do not use this phase to change tex_art_ec inclusion policy yet

### Required Data Guarantees
By the end of this phase, each preserved tileart ref should be able to carry at minimum:
- owner identity
- raw source path
- normalized path
- physical package when inferable
- logical family when inferable
- stable role and speculative role fields, even if still Unknown
- reason and confidence, even if low confidence
- texture_stretch when present
If any of these cannot yet be filled confidently, preserve Unknown plus notes rather than dropping the ref.

### Required Tests
Add executable tests covering at least these cases.

1. Tileart Textures-style classification
- one test proving a tileart ref pointing into a Textures-like path is preserved as such instead of collapsing to Undefined

2. Raw plus normalized path preservation
- one test proving both raw and normalized forms survive parsing for a representative tileart ref

3. Stretch preservation
- one test proving texture_stretch is preserved as an explicit field and survives the immediate parser-to-metadata handoff relevant to this phase

4. Art-owned liquid support case
- one test covering an art-owned liquid or wet entry whose linked support texture lives outside WorldArt
- the assertion must prove the ref remains tileart-owned and preserved

5. Regression against lossy fallback
- one test proving the new parsing path does not silently discard previously preserved WorldArt refs while adding Textures-style support

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- what tileart-side family detection was added or changed
- what fields are now preserved that were previously dropped or under-specified
- how raw path versus normalized path are stored
- how texture_stretch now survives parsing
- which policy decisions are still deferred to later phases
It must explicitly state that ownership still comes from tileart.uop and is not reassigned based on referenced package.

### Non-Goals And Guardrails
- Do not redesign base eligibility yet.
- Do not change tex_art_ec Liquid handling yet.
- Do not change tex_land_ec routing yet.
- Do not infer terrain ownership from TerrainTexture.uop or Textures-family membership.
- Do not over-interpret texture_stretch into final shader math yet.
- Do not discard an ambiguous ref just because role inference is weak.
- Do not drift into the full sidecar-promotion work of the next phase beyond what is strictly needed to avoid immediate data loss.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. tileart-side Textures-style refs are preserved instead of collapsing into lossy fallback buckets
2. raw and normalized tileart paths are both preserved and inspectable
3. texture_stretch is preserved as explicit metadata rather than an incidental parser detail
4. art-owned liquid or wet entries can retain support refs from non-WorldArt families without changing ownership
5. executable tests cover the new preservation behavior and a regression against older lossy behavior
6. no chooser, visibility, routing, or shader policy changed as an accidental side effect

### Suggested Work Order
1. inspect the current tileart classification path and identify exactly where non-WorldArt refs are being collapsed
2. mirror the minimum necessary Textures-style recognition from TerrainDefinition-side logic without importing ownership assumptions
3. add raw and normalized path preservation alongside the classification change
4. thread texture_stretch through the smallest viable metadata surface needed to avoid immediate loss
5. add focused tests for liquid-support and non-WorldArt preservation cases
6. validate with one inspection surface or debug output if available
This ordering matters because the core risk in this phase is silent data loss, not visible runtime breakage.

### Final Instruction To The Agent
Treat this phase as parser fidelity recovery. Preserve evidence first, interpretation later. Keep tileart ownership authoritative, keep ambiguous refs alive with explicit uncertainty, and leave base-selection, routing, and rendering decisions to the later phases that are supposed to own them.

## Phase 5 Agent Handoff

Assumption: “Phase 5” refers to the plan phase named “Promote multi-reference metadata to first-class runtime data”. This phase comes after parser fidelity and metadata-axis separation work. It is the phase where the preserved graph must become durable, loadable, and inspectable across conversion, runtime, and tooling. This is still not a rendering-policy phase. The agent must promote data richness into sidecars and manifests without yet redesigning package boundaries, base eligibility, or shader behavior.

### Objective
Extend the existing metadata pipeline so the renderer, inspectors, generated artifacts, and later policy phases can all see the full resource graph rather than a collapsed single-texture view. The goal is to preserve multi-reference ownership, package, family, role, path, and support-resource information in versioned runtime-facing metadata surfaces.

### Scope
Included:
- extend existing tilemeta sidecars to carry the richer multi-reference metadata model
- add equivalent terrain-material sidecars for TerrainDefinition-owned layers or materials
- add metadata-only manifests for non-image EffectTexture.uop resources such as NIF, EMS, and text assets
- preserve owner kind and owner id explicitly for every ref
- version the sidecars or manifests and keep loading behavior backward-compatible where practical
- expose the richer metadata to runtime or CLI inspection paths enough to prove it survives conversion and loading
Excluded:
- no package-boundary redesign yet
- no ec_tex_aux package creation yet
- no change to base-selection policy yet
- no change to tex_art_ec Liquid inclusion policy yet
- no change to tex_land_ec routing policy yet
- no direct runtime consumption of non-image EffectTexture resources yet
- no authored terrain or liquid shader implementation yet

### Phase Dependency Context
This phase depends on earlier phases having established: owner-reference evidence, inventory evidence, independent metadata axes, and parser preservation for tileart-side Textures-style refs and stretch. If any of those are incomplete, do not compensate by collapsing data again. Preserve unknowns explicitly and version the schema so later phases can fill gaps without breaking compatibility.

### Evidence Rules
Use this precedence when deciding what the sidecars and manifests must preserve:
1. currently parsed owner-linked refs and their authoritative ownership
2. physical package and logical family evidence from earlier phases
3. stable and speculative role fields as structured metadata, even when confidence is low
4. non-image EffectTexture resources as metadata-bearing evidence, not bindable runtime assets yet
If a field cannot be populated confidently, store Unknown or an equivalent nullable state plus notes or reason. Do not omit the field merely because the value is uncertain.

### Required Inputs
Primary metadata and conversion surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_land_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
Likely runtime or inspection consumers:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/scene/world/art/statics_collect.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
Optional earlier-phase artifacts:
- owner-reference census outputs
- TerrainTexture and EffectTexture inventory outputs
- Phase 2 diagnostic terminology or freeze summary

### Mandatory Metadata Promotion Work
The implementation must cover all of the following.

1. Tilemeta sidecar expansion
Current problem:
- existing runtime-facing metadata still exposes only a partial view of the item resource graph
Required outcome:
- extend the tilemeta sidecars so each item ref can preserve at minimum:
  - owner kind
  - owner id
  - physical package
  - logical family
  - stable role
  - speculative role
  - role reason
  - confidence
  - raw path
  - normalized path or normalized basename as appropriate
  - decodable-image flag
  - metadata-only flag
  - block order and item order when applicable
  - texture_stretch when applicable
  - relevant unknown fields already captured by parsers
Guardrail:
- do not repurpose a legacy single-value field to mean the full graph

2. Terrain-material sidecars
Current problem:
- terrain-owned layered materials are not yet represented with equivalent graph richness
Required outcome:
- add equivalent sidecar records for TerrainDefinition-owned layers or materials so later phases can reason about terrain-owned base and support refs using the same conceptual model
- preserve owner kind and owner id explicitly for terrain refs as well
Guardrail:
- do not force terrain data into item-shaped records if the ownership or layer model differs materially; equivalent does not have to mean identical

3. Metadata-only manifests for non-image EffectTexture resources
Current problem:
- non-image EffectTexture resources are important evidence but do not fit current bindable-image assumptions
Required outcome:
- add manifest surfaces that preserve names, paths, file kind, owner linkage when known, and any discoverable related image references for non-image resources such as NIF, EMS, and text assets
- make these manifests associated with the broader metadata pipeline even though runtime will not bind them yet
Guardrail:
- do not pretend non-image manifests are texture slots

4. Explicit owner identity on every ref
Current problem:
- some downstream consumers still infer ownership indirectly from the package or the caller path
Required outcome:
- every serialized ref record must carry explicit owner kind and owner id
- ownership must remain recoverable without re-running parser logic
Guardrail:
- ownership must not be inferred from physical package in any new structure

5. Versioning and compatibility
Current problem:
- richer metadata can easily break existing loaders or blur old semantics
Required outcome:
- version the sidecars and manifests explicitly
- keep backward-compatible loading where practical
- if a compatibility break is necessary, make it deliberate, documented, and detectable
Guardrail:
- do not silently reinterpret older sidecar bytes as the new richer model

### Required Data Guarantees
By the end of this phase, runtime-facing metadata should make it possible to inspect the full preserved graph for representative item-owned and terrain-owned samples.
Minimum guarantees:
- multiple refs can be preserved per owner
- image-bearing refs and metadata-only refs are distinguishable
- item-owned refs and terrain-owned refs are distinguishable
- source package, logical family, and roles survive serialization and loading
- raw path and normalized path survive where available
- stretch or repetition clues survive where applicable
- owner identity survives without recomputing parser logic
- unknown or low-confidence states survive without being collapsed away

### Expected Outputs
The phase should leave behind at least these concrete outputs.

1. Expanded runtime-loadable item sidecars
- versioned and loadable
- rich enough for later role-aware and package-aware phases

2. Terrain-material sidecars or equivalent metadata files
- versioned and loadable
- structurally suitable for layered terrain ownership

3. Metadata-only manifests for non-image support resources
- especially non-image EffectTexture entries
- associated with owner linkage when known

4. Updated inspection surface
- CLI or runtime inspection should be able to show the richer multi-reference graph for at least a few representative samples

### Required Tests
Add executable tests covering at least these cases.

1. Item sidecar round-trip
- one test proving an item with multiple refs survives write and load without losing package, family, role, path, owner identity, and stretch data

2. Terrain sidecar round-trip
- one test proving a terrain-owned layered material survives write and load with equivalent richness

3. Metadata-only manifest persistence
- one test proving a non-image EffectTexture-like resource can be serialized and loaded as metadata-only without pretending to be an image slot

4. Backward-compatibility or explicit-version test
- one test proving older metadata is either still loadable correctly or rejected clearly by version rather than silently misread

5. Inspection regression
- one test or targeted validation proving an inspection surface can display multiple refs for one owner after load

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- which new sidecars or manifests were added or expanded
- which fields are now preserved end-to-end across conversion and load
- how item-owned and terrain-owned metadata are represented
- how metadata-only resources are represented without becoming bindable textures
- what compatibility strategy was used
It must explicitly state that this phase promotes data availability, not rendering policy.

### Non-Goals And Guardrails
- Do not redesign package boundaries yet.
- Do not choose which refs are visible bases yet.
- Do not change runtime routing policy yet.
- Do not bind NIF, EMS, or text manifests into rendering yet.
- Do not collapse terrain and item ownership into one fake record shape if that hides important differences.
- Do not skip versioning just because the first consumer is local.
- Do not drop low-confidence or metadata-only refs from serialization just because current runtime does not use them yet.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. item-owned refs can survive conversion and load as a multi-reference graph
2. terrain-owned refs can survive conversion and load with equivalent richness
3. non-image EffectTexture resources can survive as metadata-only manifests
4. owner identity is explicit on every serialized ref
5. versioning is explicit and older data is either supported or clearly rejected
6. an inspection surface can prove the richer graph is available after load
7. no rendering, routing, or base-selection policy changed as an accidental side effect

### Suggested Work Order
1. map the current sidecar schema and identify exactly where richness is still being collapsed
2. define the expanded item and terrain metadata record shapes before editing writer logic
3. implement versioned serialization changes with the smallest deliberate schema break possible
4. add metadata-only manifests for non-image support resources
5. update loaders and one inspection surface to prove end-to-end availability
6. finish with narrow round-trip and compatibility validation
This ordering matters because the main risk in this phase is creating rich metadata that still cannot survive load boundaries.

### Final Instruction To The Agent
Treat this phase as graph promotion and durability work. Your job is to make the preserved resource graph actually usable by later phases without forcing policy decisions early. Prefer explicit metadata and explicit versioning over convenience shortcuts, and make sure low-confidence and metadata-only evidence survive intact.

## Phase 6 Agent Handoff

Assumption: “Phase 6” refers to the plan phase named “Redesign package boundaries around ownership and role”. This phase comes after the metadata graph is durable and loadable. It is a packaging-architecture phase: the agent must reorganize package responsibilities so visible bases and support resources are no longer conflated. This is still not the phase for changing rendering policy, authored blending, or liquid shader behavior.

### Objective
Redesign the conversion output boundaries so base-image packages remain ownership-driven while support resources move into a dedicated auxiliary package and metadata-only manifests. The goal is to make package semantics explicit:
- tex_art_ec.uddp holds tileart-owned visible base images
- tex_land_ec.uddp holds TerrainDefinition-owned visible base images
- ec_tex_aux.uddp holds non-base image-bearing support resources
- metadata manifests carry non-image support resources
This phase must make later role-aware selection and material consumption possible without overloading the meaning of the existing base packages.

### Scope
Included:
- keep tex_art_ec.uddp as the base image package for tileart-owned visible bases
- keep tex_land_ec.uddp as the base image package for TerrainDefinition-owned visible bases
- add ec_tex_aux.uddp as the image-bearing support package for non-base refs coming from Texture.uop, LegacyTexture.uop, TerrainTexture.uop, and EffectTexture.uop
- associate non-image EffectTexture resources with metadata manifests rather than bindable texture pages
- allow duplicated source images across packages when semantics differ
- make package assignment inspectable and auditable
Excluded:
- no authored terrain blending yet
- no authored liquid shader support yet
- no new visibility policy beyond what is strictly required to classify base versus non-base for packaging
- no runtime sampling of ec_tex_aux yet unless an existing inspection path requires a lightweight lookup
- no attempt to eliminate all duplication across packages
- no attempt to collapse item-owned and terrain-owned outputs into one combined package

### Phase Dependency Context
This phase depends on earlier phases having already established:
- owner-reference evidence
- physical package and logical family separation
- stable-role and speculative-role metadata
- durable multi-reference sidecars and manifests
Without those, package assignment will regress back into heuristics with no audit trail. If any ref remains ambiguous, preserve it with an explicit abnormality or low-confidence note rather than forcing it into a base package.

### Evidence Rules
Use this precedence when assigning package boundaries:
1. ownership determines which base package a visible base belongs to
2. stable role determines whether an image is base-eligible or support-only for packaging purposes
3. physical source package describes origin, not final output destination
4. speculative roles may inform notes or future planning but must not silently override stable-role package assignment
5. non-image resources belong in metadata manifests, not image page packs
If a source image is used in two distinct semantics, duplication across output packages is acceptable and should be recorded rather than treated as an error.

### Required Inputs
Primary conversion and metadata surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_land_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
Likely supporting inspection or packing seams:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/src/pack_cli.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
- any UDDP manifest or package-writing helpers already used by tex_art_ec or tex_land_ec
Optional earlier-phase artifacts:
- Phase 1 inventories and owner-reference census
- Phase 2 diagnostics
- Phase 5 sidecar and manifest schema outputs

### Mandatory Package Boundary Changes
The implementation must address all of the following.

1. Base package responsibility must become explicit
Required outcome:
- tex_art_ec.uddp remains the package for tileart-owned visible base images
- tex_land_ec.uddp remains the package for TerrainDefinition-owned visible base images
- package membership must be explainable from owner identity and base eligibility, not merely from source package origin
Guardrail:
- do not treat TerrainTexture-sourced or EffectTexture-sourced images as automatically ineligible for base packages if later policy would deem them visible bases; this phase only defines boundaries, not final eligibility policy

2. Auxiliary image package introduction
Required outcome:
- add ec_tex_aux.uddp as the dedicated package for image-bearing support refs that are not selected as visible bases
- it must be able to carry support refs originating from Texture.uop, LegacyTexture.uop, TerrainTexture.uop, and EffectTexture.uop
- membership must be inspectable by owner and ref metadata
Guardrail:
- ec_tex_aux is not a dumping ground for unresolved ownership; ownership must still be tracked explicitly in manifests or sidecars

3. Metadata-only support resources
Required outcome:
- non-image EffectTexture resources such as NIF, EMS, and text-like entries must remain as metadata manifests associated with the auxiliary-support pipeline, not fake image slots
- keep discoverable related-image linkage where available
Guardrail:
- do not assign bindable page slots to non-image resources just to fit an existing format

4. Duplication policy
Required outcome:
- allow the same source image to appear in more than one output package when semantics differ
- duplication must be intentional and auditable, not an accidental side effect
Guardrail:
- do not spend this phase trying to deduplicate away semantic differences

5. Auditability of package assignment
Required outcome:
- for any representative ref, tooling should be able to answer:
  - which output package it landed in
  - why it landed there
  - whether it was treated as visible base, support image, or metadata-only resource
  - which owner it belongs to
Guardrail:
- do not hide package decisions inside opaque packing code with no inspection path

### Required Classification Rules For Packaging
This phase may rely on stable roles for package assignment but must stay conservative.
Minimum packaging rules:
- Base and SecondaryBase are base-eligible candidates for owner-driven base packages
- AlphaMask, GenericMask, Noise, Detail, Overlay, NormalLike, ImageSupport, EffectOnlyMetadata, and UnknownSupport are auxiliary-package or metadata candidates by default
- non-image resources are metadata-only
- low-confidence or abnormal cases should be preserved with audit notes instead of forced into a visible-base package
If existing stable-role coverage is incomplete, preserve unresolved refs in auxiliary or metadata outputs with explicit abnormality notes rather than inventing aggressive base assignment logic.

### Required Outputs
The phase should leave behind at least these concrete outputs.

1. tex_art_ec.uddp
- explicitly representing tileart-owned visible bases only

2. tex_land_ec.uddp
- explicitly representing terrain-owned visible bases only

3. ec_tex_aux.uddp
- explicitly representing non-base, image-bearing support refs

4. Metadata manifests associated with auxiliary support
- especially for non-image EffectTexture resources

5. Inspection or report output showing package assignment
- at least for representative art-owned, terrain-owned, support-image, and metadata-only cases

### Required Tests
Add executable tests or focused validation covering at least these cases.

1. Tileart-owned visible base packaging
- one test proving a tileart-owned visible base remains in tex_art_ec and not ec_tex_aux

2. Terrain-owned visible base packaging
- one test proving a TerrainDefinition-owned visible base remains in tex_land_ec and not ec_tex_aux

3. Support-image packaging
- one test proving a non-base support image is routed into ec_tex_aux rather than a base package

4. Metadata-only support persistence
- one test proving a non-image EffectTexture-like resource becomes metadata-only support, not a bindable image slot

5. Duplication-allowed case
- one test or targeted validation proving that a source image can appear in more than one output package when semantics differ, without being treated as a packing bug

6. Package-assignment inspection
- one test or targeted validation proving tooling can explain which package a ref landed in and why

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- what each output package now means
- how owner identity and stable role influence package assignment
- how metadata-only resources are represented
- when duplication across output packages is acceptable
- which policy decisions are still deferred to later phases
It must explicitly state that this phase clarifies package semantics but does not yet implement the final renderer-side use of ec_tex_aux.

### Non-Goals And Guardrails
- Do not implement authored terrain or liquid shading yet.
- Do not rewrite runtime sampling to consume ec_tex_aux yet.
- Do not use speculative roles to override stable-role package placement.
- Do not erase ownership just because refs move into a shared auxiliary package.
- Do not force all ambiguous refs into base packages for convenience.
- Do not treat duplication as a failure when semantics differ.
- Do not collapse metadata-only manifests into image packs.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. tex_art_ec and tex_land_ec have explicit, ownership-driven base-package semantics
2. ec_tex_aux exists as a dedicated image-bearing support package
3. non-image support resources survive as metadata-only manifests associated with the auxiliary pipeline
4. package assignment is inspectable and explainable for representative refs
5. duplication across packages is allowed and auditable when semantics differ
6. no renderer behavior was silently changed just by reorganizing package outputs
7. later phases can consume ec_tex_aux without redefining the package model again

### Suggested Work Order
1. map current package-writing logic and identify where base and support semantics are still conflated
2. define explicit package-assignment rules before editing pack writers
3. introduce ec_tex_aux and metadata-only auxiliary manifests with the smallest deliberate extension to current packaging flow
4. wire audit or inspection output so assignments are explainable
5. validate with focused packaging tests for one art base, one terrain base, one support image, and one metadata-only support resource
This ordering matters because the main risk in this phase is moving data around without making the new package semantics explicit.

### Final Instruction To The Agent
Treat this phase as output-boundary clarification. Your job is to make the package model match ownership and role semantics without sneaking in renderer policy. Keep base packages narrow, keep support resources explicit, keep ownership visible everywhere, and preserve ambiguous cases with auditability rather than forcing premature certainty.

## Phase 7 Agent Handoff

Assumption: “Phase 7” refers to the plan phase named “Replace the blanket tileart Liquid exclusion with role-aware visibility rules”. This phase comes after ownership-aware packaging boundaries and rich metadata are already in place. It is the first phase that deliberately changes packaging visibility policy for art-owned liquid or wet entries. The agent must replace the current blanket exclusion with a base-eligibility decision that is explicit, conservative, auditable, and ownership-aware.

### Objective
Remove the unconditional tileart Liquid exclusion from tex_art_ec packaging and replace it with a role-aware base-eligibility chooser. The goal is to allow art-owned wet or liquid entries that truly have a visible base image to survive into the art base package, while keeping effect-only, support-only, or unresolved liquid stacks out of tex_art_ec but preserved in metadata and auxiliary outputs.

### Scope
Included:
- remove the unconditional Liquid exclusion from tex_art_ec
- add a base-eligibility chooser for tileart-owned liquid or wet entries
- keep no-base, support-only, effect-only, or unresolved liquid stacks out of tex_art_ec
- preserve all non-base liquid refs in metadata and auxiliary outputs
- add abnormality reporting for important liquid-edge cases
- make the decision path inspectable and explainable
Excluded:
- no authored liquid shader implementation yet
- no runtime consumption of ripple, reflection, cube, env, sphere, or similar support inputs yet
- no attempt to make every liquid tile visible by default
- no speculative-role-driven visibility overrides without stable-role support
- no terrain-owned material policy change in this phase

### Phase Dependency Context
This phase depends on earlier phases having already established:
- rich ref metadata with package, family, stable role, speculative role, reason, confidence, ownership, and path information
- auxiliary packaging for support images and metadata-only resources
- diagnostic surfaces capable of explaining current and new decisions
Without those prerequisites, changing the Liquid rule would just replace one opaque heuristic with another. If evidence is weak for a given liquid stack, preserve it as metadata and auxiliary support rather than forcing visibility.

### Evidence Rules
Use this precedence when deciding whether a tileart-owned liquid or wet entry is base-eligible:
1. ownership remains tileart-owned because the owner record comes from tileart.uop
2. stable role governs first-pass visibility eligibility
3. physical package origin does not by itself disqualify a visible base
4. speculative roles may contribute notes or abnormality labels but must not by themselves make a ref visible
5. unresolved or low-confidence stacks should bias toward preservation without visibility, not toward forced omission from metadata
If a liquid entry has one plausible visible base and several support refs, keep the plausible base eligible and preserve the rest as support. If no plausible visible base exists, keep the whole stack out of tex_art_ec but preserve it fully in metadata.

### Required Inputs
Primary packaging and metadata surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
Likely diagnostic or runtime-consumer seams:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/scene/world/art/statics_collect.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
Relevant supporting outputs from earlier phases if present:
- package-assignment audit outputs
- owner-reference census
- TerrainTexture and EffectTexture inventories
- current abnormality or diagnostic terminology

### Mandatory Policy Change
The implementation must replace the current blanket rule:
- old behavior: TileType::Liquid is excluded wholesale from tex_art_ec
with a new behavior:
- new behavior: tileart-owned liquid or wet entries are evaluated for visible-base eligibility using preserved metadata
This policy change must be explicit in code, documented in diagnostics, and verifiable in tests.

### Required Base-Eligibility Rules
Use stable-role-aware logic for the first pass. At minimum:
- Base and SecondaryBase are base-eligible candidates
- AlphaMask, GenericMask, Noise, Detail, Overlay, NormalLike, ImageSupport, EffectOnlyMetadata, and UnknownSupport are ineligible by default
- low-confidence fallback to a visible base is allowed only if the heuristic reason is explicitly recorded and the result is marked abnormal or low confidence
- speculative-role hints such as LiquidRipple, LiquidReflectionSupport, LiquidEnvProbe, FoamHighlight, FlowMapLike, or RefractionDistortionLike are never enough by themselves to make a ref visible
The chooser must be conservative and auditable. If a tile has no credible visible base candidate, it must remain out of tex_art_ec but fully preserved elsewhere.

### Required Abnormality Coverage
Add explicit abnormality codes or equivalent structured labels for at least these cases:
- liquid entry with no base
- liquid entry with only support refs
- liquid entry whose chosen base came from a non-WorldArt family
- liquid entry with unresolved support-rich stack
- liquid entry with low-confidence base fallback
These abnormalities must be inspectable via CLI, report output, or equivalent tooling.

### Required Preservation Rules
For liquid or wet entries that are not admitted into tex_art_ec as visible bases:
- preserve the owner record in metadata
- preserve all linked refs and their roles
- preserve support-image refs in the auxiliary image package where applicable
- preserve metadata-only support resources in manifests
Do not let “not visible in tex_art_ec” become “dropped from the pipeline”.

### Required Outputs
The phase should leave behind at least these concrete outcomes.

1. tex_art_ec policy change
- liquid or wet tileart entries may now be admitted if they have a credible visible base

2. Inspectable eligibility reasoning
- tooling should be able to explain why a liquid entry was admitted or rejected

3. Structured abnormality reporting
- at least for the required abnormality cases above

4. Preserved support graph
- rejected liquid entries must still have their support refs available in metadata and auxiliary outputs

### Required Tests
Add executable tests or focused validation covering at least these cases.

1. Visible liquid-art base admitted
- one test proving a tileart-owned liquid or wet entry with a credible visible base is no longer excluded wholesale and is admitted into tex_art_ec

2. Support-only liquid remains non-visible
- one test proving a tileart-owned liquid entry with only support refs remains out of tex_art_ec but is still preserved in metadata or auxiliary outputs

3. Non-WorldArt base candidate case
- one test proving a liquid entry whose chosen visible base comes from a non-WorldArt family is handled explicitly and flagged appropriately rather than being silently dropped

4. Low-confidence fallback case
- one test proving any fallback admission path records an explicit heuristic reason and abnormality or low-confidence marker

5. Regression against blanket exclusion
- one test proving the old unconditional Liquid exclusion no longer governs all liquid entries

6. Regression against accidental over-admission
- one test proving obviously support-only or effect-only liquid stacks are not mistakenly admitted as visible bases

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- that the blanket Liquid exclusion was removed
- what the new base-eligibility rule is at a high level
- which liquid entries are still kept out of tex_art_ec and why
- which abnormalities are now surfaced
- what remains deferred to later liquid-rendering phases
It must explicitly state that this phase changes packaging visibility policy for tileart-owned liquid entries, but does not yet implement liquid shading.

### Non-Goals And Guardrails
- Do not implement authored ripple, reflection, or refraction behavior yet.
- Do not let speculative roles silently grant visibility.
- Do not make package origin the deciding factor for visibility.
- Do not drop support-rich liquid stacks merely because they are not visible yet.
- Do not broaden this phase into terrain-owned visibility policy changes.
- Do not accept low-confidence bases silently; record the reason and abnormality.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. the unconditional TileType::Liquid exclusion is gone from tex_art_ec policy
2. at least some tileart-owned liquid or wet entries with credible visible bases can now enter tex_art_ec
3. support-only or no-base liquid stacks remain non-visible but preserved
4. abnormality reporting exists for key liquid-edge cases
5. tooling can explain why a liquid entry was admitted or rejected
6. no authored liquid shading was accidentally introduced under the guise of visibility work
7. later phases can build liquid-material support on top of this without revisiting the blanket-exclusion mistake

### Suggested Work Order
1. locate the exact current Liquid exclusion seam in tex_art_ec and freeze its current behavior in a local note
2. define the conservative base-eligibility chooser using existing stable-role metadata
3. wire the chooser into tex_art_ec without widening scope into runtime shader behavior
4. add abnormality labels and inspection output for admitted versus rejected liquid entries
5. validate with one visible-base liquid case, one support-only liquid case, and one ambiguous low-confidence case
This ordering matters because the main risk in this phase is swapping one opaque rule for another opaque rule.

### Final Instruction To The Agent
Treat this phase as the first controlled correction to liquid visibility. Preserve tileart ownership, admit only what has a credible visible base, keep support-rich and unresolved stacks alive in metadata, and make every admission or rejection explainable. Do not drift into liquid shading; that belongs to later phases.

## Phase 8 Agent Handoff

Assumption: “Phase 8” refers to the plan phase named “Replace current base selection with stable-role selection plus heuristic reason logging”. This phase comes after rich metadata, auxiliary packaging, and the first liquid-visibility correction are already in place. It is the phase where base selection policy is finally rewritten around stable roles instead of ad hoc non-aux-first heuristics. The agent must make selection explainable, conservative, and overridable.

### Objective
Replace the current base-selection behavior with a stable-role-driven chooser that selects visible bases using explicit eligibility rules, records the exact heuristic reason for every chosen base, and emits low-confidence abnormalities whenever fallback logic is needed. The goal is to stop relying on opaque non-aux or path-shape guesses as the primary selector while preserving enough override points to handle EC’s exceptions.

### Scope
Included:
- replace current base-selection logic with stable-role-first selection
- make Base and SecondaryBase eligible by default
- make AlphaMask, GenericMask, Noise, NormalLike, ImageSupport, EffectOnlyMetadata, and UnknownSupport ineligible by default
- allow fallback only when a specific heuristic reason is recorded and the result is marked low-confidence or abnormal
- preserve speculative roles for later but keep them non-authoritative for first-pass selection
- add override hooks for base choice, role correction, and metadata-only tagging
- make selection outcomes inspectable in tooling or reports
Excluded:
- no authored terrain or liquid shader implementation yet
- no speculative-role-driven visibility decisions
- no silent expansion of eligibility based on source package alone
- no replacement of the broader ownership or package-boundary model
- no attempt to solve every visual exception with heuristics instead of overrides

### Phase Dependency Context
This phase depends on earlier phases having already established:
- stable roles and speculative roles as separate metadata axes
- durable multi-reference metadata across conversion and load
- package boundaries separating visible bases from support resources
- liquid-art visibility no longer being blocked by a blanket exclusion
Without those prerequisites, stable-role selection would either lack the required inputs or would silently rebuild the same old heuristic behavior under a new name.

### Evidence Rules
Use this precedence when selecting a base:
1. ownership determines which package family the chosen visible base should belong to
2. stable role determines eligibility for first-pass selection
3. explicit override data outranks heuristics when present
4. heuristic fallbacks are allowed only when they record a precise reason and reduced confidence
5. speculative roles may inform notes, diagnostics, or future planning but must not silently change first-pass selection
If a candidate stack remains ambiguous after stable-role filtering, prefer a logged low-confidence fallback or explicit abnormality over pretending certainty.

### Required Inputs
Primary selection and metadata surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_land_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
Likely inspection or override surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/cc_ec_convtables/TerrainDefinition.kdl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl
- any current or new override file used for base-choice or role correction
Optional earlier-phase outputs:
- abnormality reports
- package-assignment audits
- Phase 2 baseline chooser documentation for before-versus-after comparison

### Mandatory Policy Change
The implementation must explicitly replace the old chooser baseline with a new stable-role-first chooser.

Old behavior to retire as primary logic:
- prefer non-aux WorldArt ref matching tile id
- then non-aux primary-selected WorldArt ref
- then any non-aux WorldArt ref
- then any non-aux primary-selected ref
- then any non-aux ref
- then fallback to legacy item.ec_texture_id

New behavior to introduce:
- evaluate candidate refs by stable-role eligibility first
- choose among eligible visible-base candidates using explicit, logged tie-break rules
- fall back only with a named heuristic reason and low-confidence abnormality
This change must be explicit in code and reflected in diagnostics.

### Required Eligibility Rules
At minimum, the new first-pass chooser must implement:
- Base and SecondaryBase are eligible by default
- AlphaMask, GenericMask, Noise, NormalLike, ImageSupport, EffectOnlyMetadata, and UnknownSupport are ineligible by default
- speculative roles never make a ref eligible by themselves
- metadata-only refs are never base-eligible
- low-confidence fallback is allowed only when no eligible candidate exists or when eligible candidates remain ambiguous and the fallback reason is recorded
If Detail or Overlay require project-specific treatment, preserve them conservatively and document the choice. Do not silently broaden eligibility without a recorded reason.

### Required Heuristic Reason Logging
Every chosen base must be explainable by an explicit reason. At minimum, support reason labels or equivalent structured values for:
- selected_stable_base
- selected_stable_secondary_base
- selected_via_override
- selected_after_tie_break
- selected_low_confidence_fallback
- selected_legacy_fallback
- rejected_ineligible_role
- rejected_metadata_only
- rejected_support_only
The exact naming may vary, but the information content must be preserved in a structured, inspectable way.

### Required Override Hooks
Add explicit hooks for at least these override categories:
- base choice override
- role correction override
- metadata-only tagging override
The override mechanism may live in an existing KDL or equivalent review artifact if that is already the project pattern. Requirements:
- overrides must be explicit and reviewable
- overrides must outrank heuristic fallback
- override usage should be visible in diagnostics and reason logging
Do not bury overrides in ad hoc code exceptions.

### Required Abnormality Coverage
Selection must surface abnormalities for at least these cases:
- no eligible base candidates
- multiple eligible base candidates with unresolved ambiguity
- fallback chosen despite no stable eligible candidate
- chosen base from non-default family or unexpected package origin
- role data missing or low confidence for all candidates
These abnormalities must be available through CLI, reports, or equivalent inspection output.

### Required Outputs
The phase should leave behind at least these outcomes.

1. Stable-role-driven base chooser
- used for first-pass base selection instead of the old non-aux-first heuristic

2. Structured reason logging
- every selection path and rejection path can be explained

3. Override mechanism or explicit override extension
- suitable for future manual correction work

4. Inspection output
- tooling can show candidates, eligibility, chosen base, reason, confidence, and override involvement

### Required Tests
Add executable tests or focused validation covering at least these cases.

1. Stable Base preferred over old heuristic bias
- one test proving a Base-role candidate is chosen because of stable-role eligibility, not merely because it matches older non-aux ordering

2. SecondaryBase eligibility
- one test proving SecondaryBase can be selected when appropriate and explained by reason logging

3. Ineligible support roles rejected
- one test proving AlphaMask, GenericMask, Noise, NormalLike, ImageSupport, EffectOnlyMetadata, or UnknownSupport do not silently become chosen visible bases

4. Logged fallback case
- one test proving fallback selection records an explicit reason and low-confidence or abnormality marker

5. Override precedence
- one test proving an explicit override outranks heuristic fallback and is visible in diagnostics

6. Regression against opaque chooser behavior
- one test or targeted validation proving the chosen base can now be explained through structured reasons rather than inferred from legacy branch order alone

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- what the new stable-role-first chooser does
- which roles are eligible or ineligible by default
- what fallback paths still exist and how they are logged
- how overrides are expressed and applied
- what remains deferred to later phases
It must explicitly state that speculative roles are preserved for later use but do not control first-pass base selection in this phase.

### Non-Goals And Guardrails
- Do not let speculative roles silently grant base eligibility.
- Do not make source package origin a stand-in for stable role.
- Do not hide fallback selection behind normal success paths.
- Do not replace reviewable overrides with hardcoded one-off exceptions.
- Do not widen this phase into full renderer-side material logic.
- Do not erase low-confidence or abnormal cases; surface them.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. first-pass base selection is driven by stable-role eligibility rather than the old non-aux-first chooser
2. every chosen base has a structured reason or override explanation
3. ineligible support roles are rejected by default for visible-base selection
4. fallback paths exist only with explicit low-confidence or abnormality reporting
5. override hooks exist for base choice, role correction, and metadata-only tagging
6. tooling can explain why a base was selected or rejected for representative cases
7. speculative roles remain non-authoritative for first-pass visibility

### Suggested Work Order
1. freeze the old chooser behavior locally so before-versus-after validation stays explicit
2. implement the stable-role eligibility filter before changing tie-break logic
3. add structured reason logging and abnormality surfacing alongside the new chooser
4. add override hooks and make override usage visible in diagnostics
5. validate with one clean Base case, one SecondaryBase case, one support-only rejection case, one fallback case, and one override case
This ordering matters because the main risk in this phase is replacing one opaque chooser with a differently opaque chooser.

### Final Instruction To The Agent
Treat this phase as the point where selection policy becomes explicit. Use stable roles as the default authority, use overrides for the exceptions, log every fallback honestly, and keep speculative roles out of first-pass control. The end result should be conservative, inspectable, and easy to correct when EC data turns out to be exceptional.

## Phase 9 Agent Handoff

Assumption: “Phase 9” refers to the plan phase named “Add abnormality reporting, audit outputs, and manual overrides”. This phase comes after the selector, packaging model, and metadata graph are already explicit enough to inspect. It is the phase where the pipeline becomes reviewable at scale: every important anomaly should become reportable, every owner-reference relationship should be auditable, and every necessary exception should move out of hidden heuristics into source-controlled overrides.

### Objective
Add machine-readable abnormality reporting, owner-reference audit outputs, compact human-readable summaries, and explicit manual override files for the EC conversion pipeline. The goal is to make the exception-heavy cross-package behavior observable and correctable without burying special cases in code.

### Scope
Included:
- add CSV or JSON abnormality reports
- cover the key abnormality families already identified by the plan
- add dedicated owner-reference audit outputs for tileart.uop and TerrainDefinition.uop
- add manual override files following existing KDL-style review patterns where appropriate
- split overrides into base-choice overrides, role overrides, metadata-only overrides, and ignore lists
- add compact human-readable summaries alongside machine-readable outputs
- make override application visible in diagnostics and reports
Excluded:
- no new renderer behavior
- no new shader implementation
- no expansion of selection heuristics as a substitute for overrides
- no hiding of anomalies by auto-fixing them silently during conversion
- no ad hoc hardcoded exceptions when a source-controlled override is more appropriate

### Phase Dependency Context
This phase depends on the earlier phases having already established:
- owner-reference evidence
- package-boundary clarity
- stable-role-based selection
- structured reason logging
- auxiliary and metadata-only preservation
Without those, audit outputs would be shallow or misleading. If a field is still uncertain, report that uncertainty explicitly rather than omitting it.

### Evidence Rules
Use this precedence when reporting and overriding:
1. current conversion outputs and structured metadata are authoritative for what the pipeline currently does
2. original owner records and direct-reference evidence are authoritative for what the source data actually links
3. explicit override files outrank heuristic fallbacks, but should not erase the underlying anomaly from audit visibility
4. human-readable summaries must be derived from machine-readable outputs, not the other way around
If an override resolves a case operationally, the original anomaly should still be traceable in diagnostics or audit history.

### Required Inputs
Primary conversion, metadata, and inspection surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_land_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
Existing review-pattern references:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/cc_ec_convtables/TerrainDefinition.kdl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl
Optional earlier-phase artifacts if already produced:
- owner-reference census outputs
- package-assignment audits
- abnormality labels and reason enums from prior phases
- inventory outputs for TerrainTexture.uop and EffectTexture.uop

### Mandatory Audit Outputs
The implementation must add at least these machine-readable outputs.

1. Abnormality report
Format:
- CSV or JSON
Minimum fields per record:
- owner_kind
- owner_id
- candidate_refs
- chosen_base
- heuristic_reason
- confidence
- abnormality_code
- severity
- physical_package or source_package
- logical_family
- notes
Purpose:
- capture conversion outcomes that require review or explicit acknowledgment

2. Owner-reference audit output
Add dedicated audit surfaces for at least:
- tileart.uop owner references
- TerrainDefinition.uop owner references
Minimum fields should let a reviewer answer:
- what the owner referenced
- which package family it resolved to
- whether the ref became base, support image, metadata-only, or unresolved
- whether any override affected the outcome
Purpose:
- preserve a diffable, source-grounded audit trail between source ownership and converted output treatment

3. Compact human-readable summary
Produce a concise markdown or similar summary that aggregates the machine-readable outputs into reviewer-friendly counts and notable buckets.
It should highlight:
- top abnormality categories
- counts by severity
- counts by owner kind
- counts of override use
- notable unresolved cases that still need human review
The summary must remain a derivative artifact, not the sole source of truth.

### Required Abnormality Coverage
At minimum, abnormality reporting must cover these cases:
- missing base
- only support refs
- ambiguous multiple bases
- unsupported effect-only references
- unresolved package family
- indirect-only support linkage
- runtime slot mismatch
- low-confidence base fallback
- liquid entry with no base
- liquid entry with only support refs
- liquid entry with unresolved support-rich stack
You may add more, but do not omit the core set above.

### Required Override Model
Add manual override files modeled after existing KDL or equivalent source-controlled review patterns. The override model must be split into separate concerns:
- base-choice overrides
- role overrides
- metadata-only overrides
- ignore lists
Requirements:
- overrides must be explicit and reviewable in source control
- overrides must be visible in diagnostics and audit outputs
- overrides must not silently remove the historical anomaly context
- ignore lists must be narrowly scoped and reviewable, not generic suppression buckets
If existing KDL conventions are a good fit, reuse them. If a different format is necessary, document why clearly and keep it human-editable.

### Required Override Semantics
At minimum, overrides must support:
- force a particular ref as the chosen visible base for a given owner
- correct a role classification for a given ref
- mark a ref as metadata-only even if it is image-bearing
- suppress or downgrade a known benign abnormality via an explicit ignore entry
Override precedence rules must be documented. At a minimum:
- explicit override outranks heuristic selection
- explicit ignore affects reporting severity or visibility, not source data preservation
- overrides must not mutate source ownership

### Required Inspection Integration
Tooling or diagnostics must show when overrides were involved. At minimum, an inspection path should be able to reveal:
- whether an owner had abnormalities
- whether any override applied
- which override file and entry affected the result, if applicable
- what the original heuristic outcome would have been, when that can be shown cheaply
This can be CLI-first; live runtime overlay support is optional if already available.

### Required Tests
Add executable tests or focused validation covering at least these cases.

1. Abnormality report emission
- one test proving a representative abnormal case produces a machine-readable abnormality record with the required core fields

2. Owner-reference audit emission
- one test proving a representative owner produces an audit record linking source reference to converted treatment

3. Base-choice override precedence
- one test proving a base-choice override outranks the heuristic chooser and is visible in audit output

4. Role override effect
- one test proving a role override changes classification in a controlled, inspectable way without erasing ownership or provenance

5. Metadata-only override effect
- one test proving a ref can be forced to metadata-only treatment via override and the result is auditable

6. Ignore-list behavior
- one test proving a known benign abnormality can be suppressed or downgraded explicitly without disappearing from the underlying traceability model

7. Summary generation
- one test or targeted validation proving the human-readable summary reflects machine-readable abnormality or override data rather than separate ad hoc logic

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- what audit files are now generated
- what abnormality codes are covered
- what override files or sections exist
- how override precedence works
- how ignored cases remain traceable
- what still requires future manual review or future runtime work
It must explicitly state that this phase moves exceptional cases into auditable outputs and source-controlled overrides rather than hiding them in conversion code.

### Non-Goals And Guardrails
- Do not use overrides as a substitute for preserving source evidence.
- Do not let ignore lists become a blanket suppression mechanism.
- Do not silently auto-fix abnormal cases without recording them.
- Do not make human-readable summaries the only output.
- Do not broaden this phase into new rendering or shader behavior.
- Do not bury overrides in ad hoc code paths when a declarative file can express them.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. machine-readable abnormality reports exist with the required core fields
2. owner-reference audit outputs exist for tileart.uop and TerrainDefinition.uop
3. human-readable summaries are generated from those machine-readable outputs
4. explicit source-controlled override files exist for base choice, role correction, metadata-only tagging, and ignore cases
5. override application is visible in diagnostics or audit outputs
6. ignored cases remain traceable rather than disappearing completely
7. no renderer or shader behavior changed as a side effect of adding audits and overrides

### Suggested Work Order
1. define the abnormality schema and owner-reference audit schema before wiring outputs
2. emit machine-readable outputs first, because they are the authoritative audit surface
3. add the human-readable summary as a derivative artifact second
4. introduce override files and precedence rules third
5. wire inspection output so override use and abnormality context are visible
6. validate with one abnormal case, one override case, one ignored benign case, and one owner-reference audit case
This ordering matters because the main risk in this phase is creating human-friendly review surfaces that are not grounded in stable machine-readable data.

### Final Instruction To The Agent
Treat this phase as the audit and control surface for the whole pipeline. Surface anomalies honestly, keep the outputs diffable, move exceptions into reviewable override files, and make sure nothing disappears just because it became inconvenient. The result should give later agents and human reviewers a reliable way to understand, justify, and selectively correct exceptional EC cases.

## Phase 10 Agent Handoff

Assumption: “Phase 10” refers to the plan phase named “Generate editable review artifacts from UOP-grounded evidence”. This phase comes after the pipeline can already preserve rich metadata, emit audits, and accept source-controlled overrides. It is the phase where that internal richness is turned into editable, reviewer-friendly artifacts that are still anchored to original package evidence rather than to incidental implementation details.

### Objective
Generate human-editable review artifacts from UOP-grounded evidence so package relationships, base choices, support refs, stretch or repetition clues, role guesses, and uncertainty are visible in a curated surface instead of being trapped inside machine-only diagnostics. The goal is to make human review, correction, and drift detection practical without weakening traceability back to source data.

### Scope
Included:
- extend the TerrainDefinition generator to emit richer material review artifacts
- generate a companion tileart liquid and surface-like review artifact
- generate dedicated TerrainTexture.uop and EffectTexture.uop reference reports listing internal resources and discovered direct consumers
- support curated overrides for base choice, role correction, and explicit metadata-only tagging through these review surfaces or closely paired override files
- keep the generated artifacts traceable back to owner records, source paths, and package evidence
Excluded:
- no new renderer behavior
- no new shader implementation
- no replacement of machine-readable audits from Phase 9
- no use of generated review artifacts as the sole source of truth without underlying machine-readable evidence
- no freeform hand-edited artifact format that cannot be regenerated deterministically

### Phase Dependency Context
This phase depends on earlier phases having already established:
- owner-reference evidence and inventories
- rich multi-reference metadata
- stable-role-based selection with structured reasons
- audit outputs and reviewable override files
Without those inputs, the generated review artifacts would either be too shallow to be useful or too detached from the real pipeline state. If evidence is uncertain, the artifact must show that uncertainty explicitly instead of flattening it away.

### Evidence Rules
Use this precedence when generating editable review artifacts:
1. original owner records and resolved UOP evidence are authoritative for what the source data contains
2. current machine-readable metadata and audit outputs are authoritative for how the pipeline currently interprets that source data
3. generated review artifacts must reflect both source evidence and current interpretation, clearly separated when necessary
4. curated overrides may appear in the artifact surface, but should remain identifiable as human corrections rather than source facts
If there is disagreement between source evidence and current interpretation, encode the disagreement as explicit fields or notes rather than choosing one silently.

### Required Inputs
Primary generator and review surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/cc_ec_convtables/TerrainDefinition.kdl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_land_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
Likely supporting artifacts from earlier phases:
- owner-reference census outputs
- TerrainTexture.uop inventory outputs
- EffectTexture.uop inventory outputs
- abnormality reports
- override files
Optional inspection helpers:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs

### Mandatory Generated Artifacts
The implementation must generate at least these review surfaces.

1. Extended TerrainDefinition review artifact
Current problem:
- the current terrain review surface does not expose the full material graph and uncertainty clearly enough for human review
Required outcome:
- extend the TerrainDefinition generator so it emits, for each relevant terrain material or alias group:
  - material id
  - aliases
  - all linked layers or refs
  - repetition or stretch values
  - physical package origin
  - logical family
  - stable role
  - speculative role
  - confidence
  - uncertainty notes
  - chosen visible base if one exists
  - heuristic reason or override source if relevant
Guardrail:
- do not reduce layered terrain materials to a single flattened texture field if that hides important review information

2. Tileart liquid and surface-like review artifact
Current problem:
- art-owned wet or surface-like cases are scattered across diagnostics and not presented in one review-friendly surface
Required outcome:
- generate a dedicated tileart review artifact covering at least liquid and surface-like entries
- include fields such as:
  - art id
  - tile type
  - relevant flags
  - chosen base choice
  - support refs
  - source package
  - logical family
  - stable role and speculative role where available
  - heuristic reason or override source
  - abnormality notes
Guardrail:
- keep tileart ownership explicit even when support refs come from TerrainTexture.uop or EffectTexture.uop

3. TerrainTexture.uop reference report
Required outcome:
- generate a dedicated report listing TerrainTexture.uop internal resources and discovered direct consumers
- show which owners reference each resource directly when known
- show whether the resource is currently interpreted as base-capable, support image, or unresolved
Guardrail:
- do not present name similarity or speculation as if it were a direct consumer relationship

4. EffectTexture.uop reference report
Required outcome:
- generate a dedicated report listing EffectTexture.uop internal resources and discovered direct consumers
- distinguish image-bearing resources from NIF, EMS, text, or other metadata-only resources
- show whether each resource has direct consumers, indirect-only linkage, or no discovered linkage
Guardrail:
- do not flatten non-image resources into fake texture entries just for review convenience

### Required Editability Model
These artifacts must be review-friendly and, where appropriate, pair cleanly with curated overrides.
Requirements:
- generated artifacts should be deterministic and regenerable
- fields intended for human correction should map cleanly to override files or editable sections
- manual edits must not be expected directly in unstable generated blobs unless the project already uses that pattern intentionally
- if artifacts are generated read-only and paired with editable override files, make that relationship explicit in the summary and format
The agent may choose either:
- editable generated artifacts with preserved hand-edit sections, or
- deterministic generated artifacts paired with explicit override files
But whichever model is chosen must stay source-controlled, reviewable, and regenerable.

### Required Traceability Fields
Across the generated review surfaces, ensure reviewers can trace entries back to source evidence. At minimum, preserve enough information to answer:
- which owner record this row came from
- which raw or normalized source path was involved
- which physical package the ref came from
- whether the current interpretation came from stable role, heuristic fallback, or override
- whether uncertainty remains and why
If the same source resource appears in multiple semantic contexts, the artifact should make that visible rather than deduplicating it away blindly.

### Required Outputs
The phase should leave behind at least these concrete outputs.

1. Extended TerrainDefinition review artifact
- richer than the current material view
- suitable for terrain material review and later correction

2. Tileart liquid and surface-like review artifact
- dedicated review surface for art-owned wet or liquid cases

3. TerrainTexture.uop reference report
- source resources plus discovered direct consumers

4. EffectTexture.uop reference report
- source resources plus discovered direct or indirect consumers

5. Clear linkage to override surfaces
- either inline editable sections or explicit companion override files

### Required Tests Or Validation
Add focused validation covering at least these cases.

1. TerrainDefinition artifact richness
- one test or targeted validation proving the terrain review artifact includes all linked layers or refs plus role, confidence, and uncertainty information

2. Tileart liquid review coverage
- one test or targeted validation proving a representative liquid or surface-like art entry appears with chosen base, support refs, and reason information

3. TerrainTexture direct-consumer traceability
- one test or targeted validation proving a TerrainTexture resource can be traced to discovered direct consumers in the generated report

4. EffectTexture mixed-resource reporting
- one test or targeted validation proving image-bearing and metadata-only EffectTexture resources are distinguished in the generated report

5. Override linkage clarity
- one test or targeted validation proving a reviewer can see when an entry’s current interpretation came from an override rather than raw heuristic selection

6. Regeneration stability
- one test or targeted validation proving the artifacts are deterministic enough to diff meaningfully across runs when inputs do not change

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- which review artifacts are now generated
- which fields they expose that were previously hidden or scattered
- how they stay grounded in UOP evidence and machine-readable metadata
- how review edits or overrides are meant to flow back into the pipeline
- what remains deferred to later runtime phases
It must explicitly state that generated review artifacts complement machine-readable audits; they do not replace them.

### Non-Goals And Guardrails
- Do not make the generated review artifacts the only authoritative data source.
- Do not hide uncertainty for the sake of a cleaner review surface.
- Do not turn indirect or speculative relationships into direct-consumer claims.
- Do not bury override provenance in regenerated output.
- Do not widen this phase into renderer-side behavior changes.
- Do not create a format that cannot be regenerated or meaningfully diffed.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. an extended TerrainDefinition review artifact is generated with richer source-grounded fields
2. a dedicated tileart liquid and surface-like review artifact is generated
3. TerrainTexture.uop and EffectTexture.uop reference reports are generated with discovered consumer linkage
4. the artifacts expose confidence, uncertainty, and override influence rather than hiding them
5. the artifacts are deterministic enough to review and diff meaningfully
6. review corrections can flow through explicit override surfaces or controlled editable sections
7. no renderer or shader behavior changed as a side effect of adding review artifacts

### Suggested Work Order
1. define the artifact schemas before editing generators so field coverage stays deliberate
2. extend the TerrainDefinition review artifact first, because it already exists as a precedent surface
3. add the tileart liquid and surface-like artifact second
4. add TerrainTexture and EffectTexture reference reports third
5. wire override provenance and regeneration stability validation last
This ordering matters because the main risk in this phase is producing attractive review files that are not actually traceable back to source evidence.

### Final Instruction To The Agent
Treat this phase as the human-review bridge between raw audits and later runtime work. Keep every artifact source-grounded, diffable, and honest about uncertainty. Make it easy for a reviewer to see what the data says, what the pipeline inferred, what was overridden, and where confidence is still weak.

## Phase 10A Agent Handoff

Assumption: “Phase 10A” refers to the first runtime stage under the Phase 10 runtime sub-plan: the base-correctness stage. This stage starts after the metadata, packaging, audits, and review artifacts are already in place. It is intentionally narrow: render the correct chosen base only, with no authored multi-texture blending, no liquid perturbation support, and no extra shading passes yet.

### Objective
Make runtime rendering use the correct chosen base texture consistently for terrain-owned and art-owned surfaces using the richer metadata and package outputs already established by earlier phases. The goal is to isolate base-selection correctness from later shading complexity so wrong-floor and wrong-water bugs can be debugged without confounding blend, repetition, ripple, or extra-pass issues.

### Why This Stage Must Be Separate
This stage is worth isolating because it tests a single falsifiable claim:
- if the runtime consumes the correct chosen base from the new metadata and package model, visible base mismatches should shrink even before any authored blend or liquid-support work exists
If this stage is skipped or merged with later stages, a wrong visual result could come from any combination of:
- wrong base selection
- wrong package lookup
- wrong runtime routing
- wrong stretch or repetition
- wrong blend logic
- wrong liquid support interpretation
That would make debugging materially slower and much less trustworthy.

### Scope
Included:
- switch runtime consumers to use the richer chosen-base metadata as the authoritative source for visible base lookup
- keep current land and art shaders mostly intact
- update runtime inspection outputs so they report the chosen base actually used for rendering
- validate both terrain-owned and art-owned paths against representative samples
Excluded:
- no authored terrain blend path yet
- no liquid base plus ripple or normal-like perturbation support yet
- no extra reflection, foam, waterfall, splash, glow, or lava-bubble passes yet
- no world-space repetition redesign yet beyond preserving current behavior
- no shader architecture rewrite

### Phase Dependency Context
This stage assumes earlier phases have already provided:
- explicit chosen-base metadata with stable-role-driven selection
- package boundaries where visible bases live in tex_art_ec or tex_land_ec and support images live elsewhere
- inspection or audit surfaces that can explain chosen base, package origin, and reason
If those prerequisites are incomplete, the agent may need to bridge the smallest missing runtime-loading seam, but must not reopen broader metadata or packaging redesign.

### Evidence Rules
Use this precedence when deciding what base the runtime should render:
1. explicit chosen-base metadata or sidecar data is authoritative
2. explicit override data outranks heuristic fallback
3. package membership and owner identity determine where the chosen base should be loaded from
4. current shader code is only the rendering carrier for the chosen base at this stage, not the place to reinvent selection policy
If a runtime path cannot resolve a chosen base cleanly, surface that as a runtime-routing or slot-resolution abnormality rather than silently substituting an unrelated texture.

### Required Inputs
Primary runtime and metadata-consumer surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/scene/world/art/statics_collect.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/art/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/art/ground.wgsl
Likely supporting inputs:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_land_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
- any Phase 9 abnormality or override outputs relevant to chosen-base inspection

### Mandatory Runtime Goal
The runtime must render the chosen visible base and only the chosen visible base for this stage.
That means:
- if a terrain material has a chosen base, render that base with the current terrain shader path
- if an art-owned wet or surface-like entry has a chosen visible base, render that base through the appropriate art or art-ground path
- if support refs exist, preserve them in metadata but do not attempt to sample them yet
- if no chosen visible base exists, fail visibly in diagnostics rather than opportunistically substituting a support map

### Required Runtime Behavior Changes
The implementation must address all of the following.

1. Terrain-owned base correctness
Required outcome:
- the terrain runtime path uses the chosen terrain base from the richer metadata rather than an older collapsed or provenance-only shortcut when those disagree
- the shader still samples one base albedo only for this stage
Guardrail:
- do not introduce multi-texture blending yet

2. Art-owned base correctness
Required outcome:
- regular art and surface-like art paths use the chosen visible base consistently with the new metadata
- surface-like art should no longer depend on older partial fallbacks when the richer chosen-base data is available
Guardrail:
- do not consume auxiliary support textures yet

3. Routing transparency
Required outcome:
- runtime diagnostics must show which chosen base was actually resolved, from which package, and through which runtime path
- if routing fails, the failure reason must be inspectable
Guardrail:
- do not hide slot mismatch or lookup failure behind unrelated texture fallback

4. Shader minimalism
Required outcome:
- keep existing shaders as close as possible to current behavior while updating only the visible base source they sample
- preserve current grading, lighting, and generic wet fallback behavior unless a tiny change is required to wire the correct base texture through
Guardrail:
- do not use this stage to smuggle in repetition, blending, or liquid-effect experimentation

### Required Inspection Or Debug Output
At minimum, one inspection path must show for representative hovered or queried entries:
- owner kind and owner id
- chosen base ref or texture id
- package source used for runtime lookup
- runtime slot or bindable handle resolution result
- whether the object rendered through terrain, art, or art-ground path
- whether any override influenced the chosen base
- whether any abnormality or slot mismatch remains
This can be CLI-first or overlay-first, but it must reflect the actual runtime base used rather than only the offline chosen-base metadata.

### Required Validation Targets
Validation for this stage should focus on base correctness only. Use representative samples from at least these families when available:
- marble or cave floors
- marsh or swamp water with a visible base
- lava or blood-like wet surfaces with visible base ownership
- roads or plazas
- grass-to-dirt or sand-like terrain materials that currently show obvious wrong-base symptoms
The point is not visual richness yet; the point is that the base image is the right one.

### Required Tests Or Validation
Add focused executable tests or targeted validation covering at least these cases.

1. Terrain chosen-base consumption
- one test or targeted validation proving the runtime terrain path uses the chosen base from rich metadata rather than an older collapsed fallback when the two differ

2. Art chosen-base consumption
- one test or targeted validation proving an art-owned entry with a chosen visible base resolves and renders through the correct runtime path

3. Surface-like routing case
- one test or targeted validation proving a surface-like art entry uses the correct chosen base and does not silently fall back to an unrelated land texture

4. No-base rejection visibility
- one test or targeted validation proving an entry with no chosen visible base does not silently substitute a support texture as if it were the base

5. Inspection fidelity
- one test or targeted validation proving the inspection surface reports the same chosen base and runtime path that rendering actually used

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- which runtime paths now consume the richer chosen-base metadata
- how terrain-owned and art-owned base resolution differ at this stage
- what diagnostics were added for runtime base resolution
- what visual mismatches this stage is expected to fix
- what remains deferred to later runtime stages such as repetition, authored blend, liquid support, and extra passes
It must explicitly state that this stage is about base correctness only, not material richness.

### Non-Goals And Guardrails
- Do not add world-space repetition overrides yet beyond current behavior.
- Do not add two-base blend or alpha-mask support yet.
- Do not add ripple, normal-like, reflection-like, or foam support yet.
- Do not let runtime lookup silently choose a support texture because it happened to resolve.
- Do not widen this stage into shader redesign.
- Do not redefine selection policy in runtime code; consume the already chosen base.

### Acceptance Criteria
This stage is complete only if all of the following are true:
1. terrain-owned materials render the chosen visible base from the richer metadata path
2. art-owned entries render the chosen visible base from the richer metadata path
3. surface-like art routing is inspectable and base-correct for representative cases
4. support textures are preserved but not yet sampled as visible material inputs
5. runtime diagnostics can explain which base was actually used and why
6. obvious wrong-base regressions can be distinguished from later repetition or blend issues
7. no authored blend, liquid perturbation, or extra-pass behavior was introduced under the guise of base correctness

### Suggested Work Order
1. trace the current runtime base-resolution paths for terrain, art, and surface-like art
2. identify the narrow seam where offline chosen-base metadata should become the runtime authority
3. wire that seam without widening shader scope
4. extend inspection output so the runtime-used base is observable
5. validate with one terrain case, one regular art case, and one surface-like art case before moving on
This ordering matters because the main risk in this stage is confusing selection bugs with later shading bugs.

### Final Instruction To The Agent
Treat this stage as runtime consumption of already-decided truth. Do not invent new material behavior. Render the correct chosen base, keep the path inspectable, surface routing failures honestly, and leave repetition, authored blends, liquid support, and extra passes to the later runtime stages that are supposed to own them.

## Phase 10B Agent Handoff

Assumption: “Phase 10B” refers to the second runtime stage under the Phase 10 runtime sub-plan: the world-space repetition stage. This stage comes after base-correctness is already working. It is intentionally narrower than authored blending or liquid support: make repeatable terrain-like materials respect world-space repetition and stretch consistently, without yet adding new material layers or extra support textures.

### Objective
Make repeatable terrain-like materials use correct world-space repetition and stretch so large materials are no longer squeezed into a single tile-sized sample footprint. The goal is to stabilize sampling scale independently from base selection and independently from later authored blend or liquid-support work.

### Why This Stage Must Be Separate
This stage is worth isolating because it tests a different falsifiable claim from Phase 10A:
- if the runtime uses the right repetition or stretch rule for each material, large-scale terrain-like surfaces should stop showing obviously over-dense tiling or single-tile squeezing even while still rendering only one base layer
If this stage is merged with authored blend or liquid support, a bad visual result could come from:
- wrong chosen base
- wrong world-space scale
- wrong per-material repetition precedence
- wrong blend mask behavior
- wrong liquid motion or perturbation
Keeping repetition separate makes sampling-scale bugs directly measurable.

### Scope
Included:
- preserve the current world-space land-sampling path as the basis for the change
- compare derived stretch from texture extent against decoded repetition or stretch metadata
- decide and implement how explicit per-material repetition or stretch overrides inferred size-based stretch
- apply the corrected repetition logic to repeatable terrain-like materials first
- update inspection output so the effective runtime stretch or repetition can be observed
Excluded:
- no two-base or alpha-mask authored blending yet
- no liquid ripple, normal-like, reflection-like, or flow support yet
- no extra passes yet
- no broad shader rewrite beyond the narrow sampling-scale changes needed for correct repetition
- no new base-selection policy changes

### Phase Dependency Context
This stage assumes Phase 10A already made the runtime sample the correct chosen base. That matters because repetition bugs are much easier to evaluate once the base itself is correct. It also assumes earlier metadata phases preserved repetition or stretch clues from TerrainDefinition.uop and tileart.uop, plus enough diagnostics to inspect the final runtime choice.
If repetition metadata is incomplete or ambiguous, the stage must preserve that ambiguity explicitly in diagnostics rather than silently pretending that texture extent alone is always authoritative.

### Evidence Rules
Use this precedence when choosing the effective repetition or stretch rule:
1. explicit decoded repetition or stretch metadata from source-grounded material data is stronger evidence than pure texture-dimension inference
2. current world-space sampling implementation is the runtime carrier, not the authority for which scale is semantically correct
3. texture extent may remain a fallback or comparison signal when explicit metadata is missing or low-confidence
4. override data, if present, outranks heuristic inference
If explicit metadata and inferred size-based stretch disagree, surface that disagreement in diagnostics and make the precedence rule explicit in code and summary output.

### Required Inputs
Primary runtime and shader surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/land_bindings.wgsl
Likely metadata and data-model inputs:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
Likely diagnostic or inspection seams:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_terrain_candidates.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
Relevant design references:
- the saved plan notes about third-party world-space UV reconstruction as reference only, not proof

### Mandatory Runtime Goal
The runtime must continue rendering only the chosen visible base, but now at the correct world-space repetition or stretch for repeatable terrain-like materials.
That means:
- keep one-base rendering at this stage
- change how UVs or world-space sample coordinates are scaled when explicit repetition or stretch metadata exists
- make the precedence between explicit material metadata and inferred texture-extent stretch inspectable
- do not sample support textures yet

### Required Runtime Behavior Changes
The implementation must address all of the following.

1. Explicit repetition versus inferred stretch precedence
Required outcome:
- define a clear rule for when decoded repetition or stretch metadata overrides size-derived stretch
- implement that rule in the runtime sampling path or the data provided to it
Guardrail:
- do not leave the precedence implicit or distributed across unrelated call sites

2. Terrain-like world-space scaling consistency
Required outcome:
- terrain-like materials that should repeat broadly now sample consistently in world space rather than appearing squeezed into a tiny footprint
- materials lacking explicit metadata may still use fallback inference, but that fallback must be diagnosable
Guardrail:
- do not mix repetition fixes with multi-layer blending yet

3. Data flow clarity
Required outcome:
- the effective runtime stretch or repetition value should be traceable from source metadata through runtime bindings to shader sampling
- if the runtime still derives a value from texture size, that derivation must be explicit and inspectable
Guardrail:
- do not hide final scale behind opaque shader constants with no inspection path

4. Art-path scope control
Required outcome:
- if art-owned surface-like or ground-like paths already share the same terrain-like world-space repetition problem and can be fixed cheaply with the same mechanism, the agent may include them
- otherwise keep Phase 10B focused on the terrain path first and document art-path follow-up explicitly
Guardrail:
- do not widen scope into full art-side liquid behavior unless the same repetition seam obviously applies and is cheap to validate

### Required Inspection Or Debug Output
At minimum, one inspection path must show for representative terrain-like samples:
- chosen base identity
- raw decoded repetition or stretch metadata if present
- inferred texture-extent-based stretch if computed
- effective runtime stretch or repetition actually used
- whether an override affected the final scale
- which precedence rule path was taken
This inspection output must let a reviewer see whether a material is using explicit metadata or fallback inference.

### Required Validation Targets
Validation for this stage should focus on sampling scale and visible repetition only. Use representative samples from at least these families when available:
- broad marble or cave floor materials
- roads and plazas
- grass-to-sand or dirt transitions where scale mismatch is obvious even without blend support
- large repeated desert or canyon-like surfaces
- broad snow or pale ground materials where over-dense tiling is easy to notice
The point is not authored blend realism yet; the point is that the chosen base repeats at the right scale.

### Required Tests Or Validation
Add focused executable tests or targeted validation covering at least these cases.

1. Explicit repetition precedence case
- one test or targeted validation proving explicit repetition or stretch metadata can override texture-extent-based inference when the two disagree

2. Fallback inference case
- one test or targeted validation proving the runtime still has a defined fallback path when explicit repetition or stretch metadata is absent

3. Effective runtime-scale inspection
- one test or targeted validation proving inspection output reports the effective runtime stretch or repetition actually used

4. Representative visual regression case
- one targeted validation showing a previously squeezed or over-dense terrain-like material now repeats at the intended larger scale without introducing blend logic

5. No unintended shading-scope expansion
- one test or targeted validation proving this stage did not accidentally introduce multi-texture blend, liquid perturbation, or support-texture sampling

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- what precedence rule now governs explicit repetition or stretch versus inferred texture-size stretch
- which runtime path consumes that rule
- what kinds of visual issues this stage is expected to fix
- which materials still rely on fallback inference
- what remains deferred to later stages such as single-terrain cleanup, authored solid blending, and liquid support
It must explicitly state that this stage is about world-space scale correctness, not multi-layer material richness.

### Non-Goals And Guardrails
- Do not add two-base blending yet.
- Do not add alpha-mask support yet.
- Do not add ripple or normal-like liquid perturbation yet.
- Do not redefine chosen-base selection here.
- Do not silently let texture extent override explicit repetition metadata when the plan calls for the opposite.
- Do not widen into a general shader rewrite.

### Acceptance Criteria
This stage is complete only if all of the following are true:
1. repeatable terrain-like materials use an explicit and inspectable world-space repetition or stretch rule
2. explicit repetition or stretch metadata can override inferred texture-size stretch where appropriate
3. fallback inference remains available and diagnosable when explicit metadata is absent
4. inspection output shows the effective runtime scale actually used
5. obvious squeezed or over-dense repetition regressions can be distinguished from later blend or liquid issues
6. no multi-texture blend, liquid perturbation, or extra-pass behavior was introduced under the guise of repetition work

### Suggested Work Order
1. freeze the current scale-derivation behavior in the land sampling path
2. trace how repetition or stretch metadata could reach runtime with the smallest possible change surface
3. implement the precedence rule between explicit metadata and inferred texture-size stretch
4. expose the effective runtime scale in inspection output
5. validate with one explicit-metadata case and one fallback-inference case before moving on
This ordering matters because the main risk in this stage is mixing scale-correction work with later material-composition work.

### Final Instruction To The Agent
Treat this stage as runtime scale correction only. Keep the chosen base fixed, make repetition or stretch precedence explicit, surface the final runtime value in diagnostics, and leave blending, liquid support, and extra passes to later stages. The result should make it obvious whether a remaining visual problem is now about material composition rather than about UV scale.

## Phase 10C Agent Handoff

Assumption: “Phase 10C” refers to the third runtime stage under the Phase 10 runtime sub-plan: the single-terrain stage. This stage comes after base correctness and world-space repetition are already working. It is not the authored blend stage yet. The purpose here is to formalize and validate a simple one-base world-space terrain material path for materials that do not actually require multi-texture blending.

### Objective
Introduce an explicit single-terrain material path for terrain-owned materials whose correct rendering is adequately represented by one chosen base sampled in world space at the correct repetition or stretch. The goal is to stop treating all terrain materials as either a generic baseline or an unfinished blend case: some materials should be explicitly recognized as complete under a single-base path.

### Why This Stage Must Be Separate
This stage is worth isolating because it establishes a clean baseline material mode between “base-correct one texture” and “full authored multi-texture blend”. It tests this falsifiable claim:
- some terrain materials should look correct enough with one chosen base at the right world-space scale, and forcing them through future multi-layer logic would only add complexity without improving correctness
If this stage is skipped, later blend work risks swallowing simple materials into unnecessary complexity, making it harder to tell whether a visual issue comes from missing blend support or from misclassifying a material that should have remained single-terrain.

### Scope
Included:
- introduce an explicit single-terrain runtime material mode or equivalent explicit classification
- keep sampling one chosen base only, using the world-space repetition logic established in Phase 10B
- identify which terrain-owned materials are eligible for this simpler path based on current metadata and roles
- make the single-terrain decision inspectable in diagnostics
- validate representative materials that should be considered complete without authored multi-texture blending
Excluded:
- no two-base plus alpha-mask authored solid blending yet
- no liquid perturbation or ripple support yet
- no extra passes yet
- no attempt to force all terrain materials into the single-terrain mode
- no broad shader rewrite beyond what is needed to make the single-terrain path explicit and inspectable

### Phase Dependency Context
This stage assumes:
- Phase 10A already made the runtime use the correct chosen base
- Phase 10B already made the runtime use the correct world-space repetition or stretch
Those two prerequisites matter because only after base identity and base scale are correct can the agent evaluate whether one base is actually sufficient for a given material. This stage also assumes earlier metadata phases provide enough role and ownership information to distinguish likely single-base materials from materials that are clearly blend candidates.

### Evidence Rules
Use this precedence when deciding whether a terrain-owned material should use the single-terrain path:
1. current source-grounded material metadata and role data are authoritative for whether only one credible visible base exists
2. explicit override or review-artifact correction outranks heuristic classification
3. the third-party single-terrain.fx is a design clue for architecture, not proof that any specific material should be single-terrain
4. if a material shows credible Base plus SecondaryBase plus AlphaMask evidence, it should not be prematurely treated as definitively single-terrain
If the evidence is ambiguous, prefer classifying the material as “not yet single-terrain-safe” rather than overcommitting and hiding a future blend requirement.

### Required Inputs
Primary runtime and shader surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/land_bindings.wgsl
Likely metadata and classification inputs:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
Likely inspection or review surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_terrain_candidates.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
- generated review artifacts and override surfaces from Phase 10 and Phase 9

### Mandatory Runtime Goal
The runtime must support an explicit single-terrain material path for terrain-owned materials that only need one chosen base.
That means:
- the material is intentionally recognized as complete under one-base rendering for the current stage
- the runtime path remains world-space sampled and uses the effective repetition or stretch from Phase 10B
- the material is not treated as an unfinished blend case by default if the evidence says one base is sufficient
- support refs may remain preserved in metadata without being sampled

### Required Runtime Behavior Changes
The implementation must address all of the following.

1. Explicit single-terrain classification
Required outcome:
- introduce a runtime-facing notion of “single-terrain” or equivalent explicit material mode for terrain-owned materials
- the classification decision must be inspectable and grounded in metadata, not hidden in shader-side assumptions
Guardrail:
- do not classify obviously blend-capable materials as single-terrain merely because only one input is currently rendered

2. Single-terrain runtime path
Required outcome:
- the runtime uses a stable, explicit one-base terrain path for eligible materials
- this path reuses the world-space repetition or stretch logic already stabilized in Phase 10B
Guardrail:
- do not add multi-layer logic yet

3. Material-mode transparency
Required outcome:
- diagnostics or inspection output must reveal whether a material is being treated as single-terrain, why, and what evidence or override led to that decision
- if a material is excluded from the single-terrain path because it appears blend-capable, that exclusion should be inspectable too
Guardrail:
- do not collapse “single-terrain by evidence” and “single-terrain because fallback” into the same opaque outcome

4. Conservative classification boundaries
Required outcome:
- materials with credible Base plus SecondaryBase plus AlphaMask evidence should remain staged for future blend work rather than being declared complete under single-terrain
- materials with one clear visible base and no strong blend evidence should be allowed to stabilize under single-terrain
Guardrail:
- prefer false negatives over false positives; it is safer to defer a material to the blend stage than to incorrectly flatten a true blend material into a single-terrain path

### Required Inspection Or Debug Output
At minimum, one inspection path must show for representative terrain materials:
- chosen base identity
- effective repetition or stretch used
- whether the material is classified as single-terrain
- which evidence or override led to that classification
- whether the material was excluded from single-terrain because of blend-capable evidence
- whether any abnormality or low-confidence note remains
This must let a reviewer distinguish “looks simple because it is simple” from “looks simple only because blend support is not implemented yet”.

### Required Validation Targets
Validation for this stage should focus on terrain materials that plausibly do not need authored multi-texture blending. Use representative samples from at least these families when available:
- broad stone or marble floor materials that appear visually coherent as one base
- simple dirt or packed-earth terrain
- roads or plazas with no strong transition-mask evidence
- plain snow or pale terrain surfaces where one broad base may be sufficient
The point is to identify and validate materials that should already be considered correct before the blend stage begins.

### Required Tests Or Validation
Add focused executable tests or targeted validation covering at least these cases.

1. Single-terrain eligible material
- one test or targeted validation proving a terrain material with one credible visible base and no strong blend evidence is classified and rendered as single-terrain

2. Blend-capable material excluded from single-terrain
- one test or targeted validation proving a material with credible Base plus SecondaryBase plus AlphaMask evidence is not silently flattened into the single-terrain path

3. Inspection fidelity for material mode
- one test or targeted validation proving inspection output reports the single-terrain classification decision and the reason behind it

4. Repetition continuity
- one test or targeted validation proving a material classified as single-terrain still uses the world-space repetition or stretch rule established in Phase 10B

5. No unintended blend support
- one test or targeted validation proving this stage does not accidentally introduce two-base blending, mask sampling, or liquid-support behavior

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- what the explicit single-terrain material path is
- how materials are classified into or out of that path
- what kinds of materials are expected to be correct under single-terrain
- which materials are deliberately deferred to the authored blend stage
- what remains deferred to later stages such as solid-terrain blend, liquid support, and extra passes
It must explicitly state that single-terrain is not a fallback for everything; it is a deliberate material mode for materials whose source evidence does not justify multi-layer support.

### Non-Goals And Guardrails
- Do not add two-base blending yet.
- Do not add alpha-mask sampling yet.
- Do not add liquid perturbation or support-texture behavior yet.
- Do not redefine world-space repetition here; consume the stabilized rule from Phase 10B.
- Do not classify materials as single-terrain just because the current renderer only knows one texture.
- Do not widen into a generic shader redesign.

### Acceptance Criteria
This stage is complete only if all of the following are true:
1. an explicit single-terrain material path exists for eligible terrain-owned materials
2. eligible materials can be identified and inspected as single-terrain by evidence or override
3. clearly blend-capable materials are not silently flattened into the single-terrain path
4. single-terrain materials retain correct world-space repetition or stretch behavior
5. inspection output can explain why a material is or is not treated as single-terrain
6. no authored blend, liquid perturbation, or extra-pass behavior was introduced under the guise of single-terrain work

### Suggested Work Order
1. identify the minimal runtime seam where terrain materials can gain an explicit material-mode distinction
2. define conservative eligibility rules for single-terrain classification
3. wire the explicit single-terrain path while reusing the base and repetition logic from earlier stages
4. extend inspection output so material-mode decisions are visible
5. validate with one clearly simple material and one clearly blend-capable material before moving on
This ordering matters because the main risk in this stage is mistaking “currently rendered as one texture” for “semantically a one-texture material”.

### Final Instruction To The Agent
Treat this stage as material-mode clarification, not material enrichment. Make it explicit which terrain materials are genuinely well-served by one base, keep the path inspectable, and defer real blend-capable materials to the next stage instead of flattening them prematurely.

## Phase 10D Agent Handoff

Assumption: “Phase 10D” refers to the fourth runtime stage under the Phase 10 runtime sub-plan: the solid-terrain authored blend stage. This stage comes after base correctness, world-space repetition, and explicit single-terrain classification are already working. It is the first stage that intentionally adds authored multi-texture terrain composition. The agent must support two base textures plus one alpha mask, each with independent stretch, all sampled in world space, starting with TerrainDefinition-owned solid materials.

### Objective
Implement an authored solid-terrain blend path that supports:
- one primary base texture
- one secondary base texture
- one alpha mask
- independent world-space repetition or stretch for each input when the data supports it
The goal is to render terrain materials whose source evidence clearly indicates multi-layer composition, without conflating them with single-terrain or liquid behavior.

### Why This Stage Must Be Separate
This stage is worth isolating because it introduces the first real material-composition jump beyond one-base rendering. It tests this falsifiable claim:
- when a terrain material has credible Base, SecondaryBase, and AlphaMask evidence, a world-space two-base-plus-mask path should improve correctness relative to single-base rendering
If this stage is merged with liquid support or extra passes, a bad result could come from:
- wrong base selection
- wrong repetition precedence
- wrong mask interpretation
- wrong blend ordering
- wrong liquid-like perturbation support
Keeping the solid blend stage separate lets the team verify authored blend correctness without confounding it with fluid or post-lighting behavior.

### Scope
Included:
- add a solid-terrain runtime material path for TerrainDefinition-owned solid materials
- sample Base, SecondaryBase, and AlphaMask candidates in world space
- allow independent stretch or repetition for each sampled input when the metadata supports it
- use overrides for ambiguous or exceptional materials
- make blend participation and input selection inspectable in diagnostics
Excluded:
- no liquid ripple or normal-like perturbation yet
- no art-owned wet or liquid support yet
- no reflection, foam, waterfall, splash, or extra-pass work yet
- no attempt to force all terrain materials into the solid-blend path
- no widening into general-purpose N-layer terrain shading

### Phase Dependency Context
This stage assumes:
- Phase 10A already established correct chosen-base consumption
- Phase 10B already established correct world-space repetition or stretch handling
- Phase 10C already separated truly single-terrain materials from blend-capable materials
- earlier metadata phases already provide stable-role and override coverage for Base, SecondaryBase, and AlphaMask candidates
If those prerequisites are weak for a given material, the agent should use explicit abnormalities or override hooks rather than guessing silently.

### Evidence Rules
Use this precedence when deciding whether a terrain material should use the solid-terrain blend path and which inputs it should use:
1. stable-role evidence for Base, SecondaryBase, and AlphaMask is authoritative for first-pass input selection
2. explicit overrides outrank heuristic ambiguity
3. source package origin does not by itself decide whether an input is usable in the blend path
4. speculative roles must not silently substitute for missing AlphaMask or SecondaryBase evidence
5. if AlphaMask evidence is absent or low-confidence, do not silently invent a fake authored blend just because two plausible base textures exist
If the material is ambiguous, prefer keeping it out of the authored solid-blend path until overrides or better evidence exist.

### Required Inputs
Primary runtime and shader surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/land_bindings.wgsl
- any neighboring land shader modules used for shading or compositing
Likely metadata and classification inputs:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- override surfaces introduced in earlier phases
Likely inspection seams:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_terrain_candidates.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
Relevant reference note:
- the third-party solid-terrain.fx is a design clue only, not proof of exact original EC equations

### Mandatory Runtime Goal
The runtime must support an explicit authored solid-terrain material mode for appropriate TerrainDefinition-owned materials.
That means:
- one base remains insufficient for these materials once strong blend evidence exists
- Base and SecondaryBase are both sampled when the material qualifies for solid-terrain
- AlphaMask modulates the blend rather than being treated as a visible base
- each sampled input may use its own world-space repetition or stretch when supported by the metadata
- materials lacking sufficient evidence remain single-terrain or unresolved rather than being forced into this path

### Required Runtime Behavior Changes
The implementation must address all of the following.

1. Explicit solid-terrain classification
Required outcome:
- introduce a runtime-facing solid-terrain material mode or equivalent explicit distinction
- the classification decision must be inspectable and grounded in stable roles and overrides
Guardrail:
- do not classify a material as authored solid blend simply because multiple candidate refs exist without credible AlphaMask support

2. Base plus SecondaryBase plus AlphaMask input wiring
Required outcome:
- wire the chosen Base, SecondaryBase, and AlphaMask inputs into the terrain shader path
- AlphaMask must modulate the composition rather than being treated as a third color layer
Guardrail:
- do not use support-only or metadata-only refs as visible blend inputs

3. Independent world-space scale handling
Required outcome:
- Base, SecondaryBase, and AlphaMask may each use their own effective repetition or stretch if the metadata supports it
- where metadata is incomplete, fallback behavior must be explicit and inspectable
Guardrail:
- do not silently force all three inputs to share one scale when the source evidence says otherwise unless a documented fallback rule requires it

4. Conservative ambiguity handling
Required outcome:
- materials with unresolved or conflicting candidates must remain inspectable and overridable
- low-confidence or unresolved cases should surface abnormalities instead of silently producing misleading blend output
Guardrail:
- prefer false negatives over false positives; do not over-admit materials into authored solid blending

### Required Inspection Or Debug Output
At minimum, one inspection path must show for representative solid-terrain candidates:
- whether the material is classified as solid-terrain
- chosen Base, SecondaryBase, and AlphaMask inputs
- effective repetition or stretch used for each input
- whether any override influenced the chosen inputs or classification
- whether any abnormality or low-confidence state remains
This must let a reviewer tell whether a material is genuinely using an authored blend path versus a single-terrain fallback.

### Required Validation Targets
Validation for this stage should focus on solid terrain materials where soft transitions or layered composition are expected. Use representative samples from at least these families when available:
- grass-to-sand transitions
- dirt-to-stone or earth-to-road boundaries
- plaza or roadway materials with clear layered structure
- desert or canyon terrain where one base plus a second layer appears plausible from source evidence
The point is not yet liquid realism; the point is that terrain materials with strong blend evidence stop looking artificially flattened into one base.

### Required Tests Or Validation
Add focused executable tests or targeted validation covering at least these cases.

1. Solid-terrain eligible material
- one test or targeted validation proving a material with credible Base, SecondaryBase, and AlphaMask evidence is classified and rendered through the solid-terrain blend path

2. AlphaMask semantics case
- one test or targeted validation proving the AlphaMask is used as a blend control and not misinterpreted as a visible albedo layer

3. Independent scale case
- one test or targeted validation proving Base, SecondaryBase, and AlphaMask can use distinct effective repetition or stretch values when required

4. Ambiguous material exclusion
- one test or targeted validation proving a material with unresolved blend evidence is not silently forced into the authored solid-terrain path

5. Inspection fidelity for blend mode
- one test or targeted validation proving inspection output reports the chosen blend inputs, scales, and override involvement accurately

6. No unintended liquid or extra-pass behavior
- one test or targeted validation proving this stage does not accidentally introduce ripple, liquid support, or post-lighting extra passes

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- what the explicit solid-terrain material path is
- how materials qualify for it
- how Base, SecondaryBase, and AlphaMask are chosen
- how independent repetition or stretch is handled
- which materials remain deferred to liquid or extra-pass stages
It must explicitly state that this stage covers authored solid terrain blending only, not liquid-terrain support and not extra passes.

### Non-Goals And Guardrails
- Do not add liquid perturbation or ripple support yet.
- Do not add art-owned wet or liquid behavior yet.
- Do not interpret AlphaMask as a visible base texture.
- Do not flatten blend-capable materials back into single-terrain merely for convenience once the evidence is strong.
- Do not widen into generic N-layer terrain compositing.
- Do not silently invent missing mask or secondary inputs.

### Acceptance Criteria
This stage is complete only if all of the following are true:
1. an explicit authored solid-terrain blend path exists for appropriate TerrainDefinition-owned materials
2. Base, SecondaryBase, and AlphaMask inputs are chosen and inspected explicitly
3. independent world-space repetition or stretch can be honored for the blend inputs when supported
4. ambiguous materials remain inspectable and overridable rather than silently forced into the blend path
5. representative solid-terrain materials show improved correctness relative to single-terrain flattening
6. no liquid support, art-owned wet behavior, or extra-pass logic was introduced under the guise of solid blending

### Suggested Work Order
1. identify the minimal runtime seam where terrain materials can gain an authored solid-blend material mode
2. define conservative eligibility and input-selection rules using stable roles and overrides
3. wire Base, SecondaryBase, and AlphaMask through the terrain path while reusing the scale logic from Phase 10B
4. extend inspection output so solid-terrain participation and input scales are visible
5. validate with one clearly blend-capable material and one intentionally excluded ambiguous material before moving on
This ordering matters because the main risk in this stage is turning uncertain multi-ref materials into seemingly-correct blends without enough evidence.

### Final Instruction To The Agent
Treat this stage as authored solid-material composition only. Keep the mode explicit, choose inputs from stable evidence and overrides, respect per-input world-space scale, and surface unresolved cases honestly. Leave liquid behavior and extra passes for the later runtime stages that are meant to handle them.

## Phase 10E Agent Handoff

Assumption: “Phase 10E” refers to the fifth runtime stage under the Phase 10 runtime sub-plan: the liquid-terrain support stage. This stage comes after base correctness, world-space repetition, explicit single-terrain handling, and authored solid-terrain blending are already working. It is the first runtime stage that intentionally adds liquid-specific support inputs and motion semantics for TerrainDefinition-owned liquid materials.

### Objective
Implement a liquid-terrain material path that supports:
- one liquid base texture
- one normal-like, ripple-like, or distortion-like support texture when credible evidence exists
- world-space sampling
- animated scrolling or centered variants where source evidence or preserved parameters justify them
The goal is to support TerrainDefinition-owned liquid materials without conflating them with solid-terrain blend rules or with art-owned wet and liquid paths.

### Why This Stage Must Be Separate
This stage is worth isolating because liquid materials introduce a different class of behavior from solid materials. It tests this falsifiable claim:
- when a TerrainDefinition-owned liquid material has a credible visible base plus a credible ripple-like or normal-like support texture, a dedicated liquid path should improve correctness beyond single-base or solid-blend rendering
If this stage is merged with art-owned wet support or extra passes, a bad result could come from:
- wrong liquid base selection
- wrong support-texture interpretation
- wrong world-space scale
- wrong scrolling or centered-motion semantics
- wrong ownership routing between terrain and art
Keeping liquid-terrain support separate allows the team to validate terrain-owned liquid behavior before tackling geometry-specific art paths or speculative extra-pass work.

### Scope
Included:
- add a dedicated liquid-terrain runtime material path for TerrainDefinition-owned liquid materials
- support one liquid base plus one normal-like, ripple-like, or distortion-like support input when credible evidence exists
- sample inputs in world space
- support animated scrolling or centered variants when parameters or source clues justify them
- preserve explicit uncertainty when motion or support semantics are unknown
- make liquid participation, chosen inputs, and motion-mode decisions inspectable in diagnostics
Excluded:
- no art-owned wet or liquid shader behavior yet
- no reflection-like overlay, foam, waterfall, splash, or lava-bubble extra passes yet unless they are required merely to preserve diagnostics
- no generic N-layer liquid compositor
- no assumption that every liquid material has all parameters fully known
- no silent borrowing of solid-terrain AlphaMask logic as a substitute for liquid support semantics

### Phase Dependency Context
This stage assumes:
- Phase 10A already established correct chosen-base runtime consumption
- Phase 10B already established world-space repetition or stretch precedence
- Phase 10C already separated simple one-base terrain materials from richer ones
- Phase 10D already handled authored solid terrain blending separately
- earlier metadata phases preserved liquid base candidates, ripple-like or normal-like support candidates, and any inferable motion parameters or uncertainty notes
If those prerequisites are missing for a given material, the agent should preserve unresolved status and diagnostics rather than guessing aggressively.

### Evidence Rules
Use this precedence when deciding whether a TerrainDefinition-owned material should use the liquid-terrain path and which inputs it should use:
1. ownership and material typing from TerrainDefinition-side evidence are authoritative for whether the terrain material is liquid-owned
2. stable-role evidence for a visible liquid base is authoritative for first-pass base choice
3. stable-role or preserved-support evidence for NormalLike, ripple-like, or distortion-like support is stronger than name-only speculation
4. explicit overrides outrank heuristic ambiguity
5. speculative roles may inform notes and motion hypotheses but must not silently invent a support input or motion mode without traceable evidence
If a material has a credible liquid base but unclear support semantics, it may still use the liquid-terrain path in a reduced form with explicit uncertainty. If even the base is unclear, do not force it into the liquid path.

### Required Inputs
Primary runtime and shader surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/land_bindings.wgsl
- neighboring land shader modules used for normals, shading, or lighting
Likely metadata and classification inputs:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- earlier override and review-artifact surfaces
Likely inspection seams:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_terrain_candidates.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
Reference note:
- the third-party liquid-terrain.fx is a design clue only, not proof of exact original EC equations or parameter names

### Mandatory Runtime Goal
The runtime must support an explicit liquid-terrain material mode for appropriate TerrainDefinition-owned liquid materials.
That means:
- the material path is intentionally distinct from single-terrain and authored solid-terrain
- one chosen liquid base remains visible
- one credible ripple-like, normal-like, or distortion-like support input may modulate the material when evidence supports it
- world-space sampling remains in use
- motion or centered-variant behavior is explicit and inspectable when present
- unresolved support or motion semantics remain diagnosable rather than silently fabricated

### Required Runtime Behavior Changes
The implementation must address all of the following.

1. Explicit liquid-terrain classification
Required outcome:
- introduce a runtime-facing liquid-terrain material mode or equivalent explicit distinction for TerrainDefinition-owned liquid materials
- the classification decision must be inspectable and grounded in source-grounded material evidence, roles, and overrides
Guardrail:
- do not classify a material as liquid-terrain merely because it is visually wet-looking if ownership and material evidence do not support that mode

2. Liquid base plus support input wiring
Required outcome:
- wire one chosen liquid base and, when credible evidence exists, one support input representing ripple-like, normal-like, or distortion-like perturbation
- keep the support input semantically distinct from a visible albedo layer
Guardrail:
- do not reuse AlphaMask semantics from the solid-terrain path as if they were liquid perturbation semantics

3. Motion-mode explicitness
Required outcome:
- support scrolling, centered, or equivalent motion variants only when the available evidence or preserved parameter data justifies them
- when the motion model is uncertain, expose a conservative default plus explicit uncertainty in diagnostics
Guardrail:
- do not hardcode speculative motion semantics as if they were proven source truth

4. World-space scale continuity
Required outcome:
- the liquid base and any support input should respect the world-space repetition or stretch framework already established in Phase 10B
- if the liquid support input needs a different effective scale, that difference must be explicit and inspectable
Guardrail:
- do not break previously stabilized world-space scaling just to get visible motion quickly

5. Conservative ambiguity handling
Required outcome:
- materials with uncertain support inputs, uncertain motion parameters, or unresolved liquid ownership remain inspectable and overridable
- reduced liquid behavior is acceptable when support evidence is partial, but the uncertainty must be visible
Guardrail:
- prefer explicit reduced behavior plus diagnostics over overfitting speculative liquid richness

### Required Inspection Or Debug Output
At minimum, one inspection path must show for representative liquid-terrain materials:
- whether the material is classified as liquid-terrain
- chosen visible liquid base
- chosen support input, if any
- effective repetition or stretch used for each input
- motion mode chosen, if any
- whether an override influenced classification, input choice, or motion semantics
- whether any abnormality or uncertainty note remains
This must let a reviewer distinguish “liquid path with known support” from “liquid path in reduced or uncertain mode”.

### Required Validation Targets
Validation for this stage should focus on TerrainDefinition-owned liquid materials. Use representative samples from at least these families when available:
- marsh or swamp water
- broad water or shoreline terrain
- lava-like liquid terrain if represented by TerrainDefinition ownership
- other liquid terrains where source data suggests ripple-like or perturbation support
The point is not yet full reflective richness. The point is that terrain-owned liquid materials stop looking like mislabeled solid terrain or flat single-base surfaces when the evidence clearly supports more.

### Required Tests Or Validation
Add focused executable tests or targeted validation covering at least these cases.

1. Liquid-terrain eligible material
- one test or targeted validation proving a TerrainDefinition-owned liquid material with a credible base and support input is classified and rendered through the liquid-terrain path

2. Reduced liquid mode case
- one test or targeted validation proving a liquid material with a credible base but uncertain support semantics can still use a reduced liquid path with explicit uncertainty instead of being silently flattened or overfit

3. Motion-mode inspection case
- one test or targeted validation proving inspection output reports the chosen motion mode or the absence of one explicitly

4. World-space continuity case
- one test or targeted validation proving liquid base and support inputs continue to obey the world-space repetition or stretch framework rather than introducing ad hoc scale logic

5. No solid-mask semantic leakage
- one test or targeted validation proving AlphaMask-style solid-terrain semantics are not accidentally reused as liquid perturbation inputs without explicit evidence

6. No art-owned or extra-pass scope creep
- one test or targeted validation proving this stage does not accidentally introduce art-owned wet handling or reflection, foam, waterfall, or splash extra-pass behavior

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- what the explicit liquid-terrain material path is
- how liquid materials qualify for it
- how the visible base and support input are chosen
- how motion modes are handled when known or uncertain
- which liquid behaviors remain deferred to later art-owned or extra-pass stages
It must explicitly state that this stage covers TerrainDefinition-owned liquid materials first and does not yet solve art-owned wet or liquid rendering.

### Non-Goals And Guardrails
- Do not add art-owned wet or liquid behavior yet.
- Do not add reflection-like, foam, waterfall, splash, glow, or lava-bubble extra passes yet.
- Do not treat speculative support names as proven liquid perturbation inputs without traceable evidence.
- Do not break world-space scale consistency while adding motion.
- Do not silently invent missing motion parameters.
- Do not widen into a generic all-effects water system.

### Acceptance Criteria
This stage is complete only if all of the following are true:
1. an explicit liquid-terrain runtime path exists for appropriate TerrainDefinition-owned liquid materials
2. visible liquid base and support inputs are chosen and inspected explicitly when evidence supports them
3. reduced or uncertain liquid behavior remains diagnosable instead of being hidden
4. motion semantics, when used, are explicit and inspectable
5. world-space repetition or stretch remains coherent for liquid inputs
6. no art-owned wet handling or extra-pass logic was introduced under the guise of liquid-terrain support

### Suggested Work Order
1. identify the minimal runtime seam where TerrainDefinition-owned liquid materials can gain a distinct material mode
2. define conservative eligibility and input-selection rules for liquid base and support inputs
3. wire the liquid-terrain path while reusing the world-space scale framework from Phase 10B
4. add explicit motion-mode handling only where evidence exists, with uncertainty surfaced otherwise
5. extend inspection output so liquid participation, inputs, and motion decisions are visible
6. validate with one clear liquid-terrain case and one reduced or uncertain liquid case before moving on
This ordering matters because the main risk in this stage is replacing honest uncertainty with overconfident liquid behavior.

### Final Instruction To The Agent
Treat this stage as TerrainDefinition-owned liquid material support only. Keep the mode explicit, keep the visible base authoritative, add support input and motion semantics only where evidence justifies them, and surface all uncertainty honestly. Leave art-owned wet or liquid behavior and extra passes for the next runtime stages.

## Phase 10F Agent Handoff

Assumption: “Phase 10F” refers to the sixth runtime stage under the Phase 10 runtime sub-plan: the art-owned wet or liquid stage. This stage comes after terrain-owned liquid support is already working. It is intentionally separate because tileart-owned wet and liquid entries follow different ownership, geometry, and runtime paths from TerrainDefinition-owned terrain materials, even when some support resources overlap.

### Objective
Add support-aware handling in the worldmap art shaders and runtime for tileart-owned wet or liquid entries. Cover both:
- the regular worldmap art path for upright or standard art geometry
- the worldmap art ground or surface-like path for art entries that behave more like ground-aligned surfaces
The goal is to render art-owned wet or liquid content using ownership-correct base and support handling rather than treating it as either terrain-owned liquid or ordinary static art.

### Why This Stage Must Be Separate
This stage is worth isolating because art-owned wet or liquid entries are not just “terrain liquid on different assets”. They differ in:
- ownership source, which comes from tileart.uop rather than TerrainDefinition.uop
- geometry path, because art and surface-like art use different runtime pipelines than terrain tiles
- routing rules, because surface-like art may traverse art-ground or land-adjacent paths
This stage tests this claim:
- tileart-owned wet or liquid entries with a visible base and credible support refs should render more correctly when handled by a dedicated art-side support-aware path rather than by terrain-liquid logic or ordinary static-art fallback
If this stage is merged with terrain liquid or extra-pass exploration, failures become hard to localize between ownership, geometry path, support interpretation, and optional post-lighting behavior.

### Scope
Included:
- add support-aware handling for tileart-owned wet or liquid entries in the worldmap art runtime
- support both regular art and surface-like art-ground paths where applicable
- preserve tileart ownership as authoritative even when support resources originate from TerrainTexture.uop or EffectTexture.uop
- allow a visible art-owned base plus optional support input when evidence justifies it
- make art path, chosen inputs, and routing decisions inspectable in diagnostics
Excluded:
- no TerrainDefinition-owned liquid work here; that belongs to Phase 10E
- no extra reflection, foam, waterfall, splash, lava-bubble, or post-lighting exploration yet
- no attempt to flatten all art-owned wet or liquid entries into one geometry path
- no assumption that every art-owned wet entry has a reliable support input or motion model
- no generic all-effects material system for worldmap art yet

### Phase Dependency Context
This stage assumes:
- Phase 7 already removed the blanket tileart Liquid exclusion and introduced role-aware visibility rules for art-owned liquid entries
- Phase 8 already stabilized chosen visible-base selection and reason logging
- Phase 10A already made runtime base consumption correct
- Phase 10E already introduced a terrain-owned liquid path, providing useful separation of concerns
- earlier phases already preserved tileart-owned support refs, including Textures-style and other non-WorldArt resources, plus ownership and routing diagnostics
If an art-owned wet or liquid entry lacks credible support or motion evidence, the stage should still preserve reduced behavior and diagnostics rather than forcing terrain-liquid semantics onto it.

### Evidence Rules
Use this precedence when deciding whether and how a tileart-owned wet or liquid entry should use the art-side support-aware path:
1. tileart ownership is authoritative for ownership and should not be reassigned based on package origin
2. chosen visible base metadata is authoritative for the visible albedo input
3. support refs with credible NormalLike, ripple-like, distortion-like, or equivalent evidence may influence the art-side wet or liquid path
4. explicit overrides outrank heuristic ambiguity
5. speculative roles may inform notes and future work but must not silently invent support inputs or motion behavior
If an entry has a visible base but uncertain support semantics, reduced art-side wet or liquid behavior is acceptable as long as the uncertainty remains inspectable.

### Required Inputs
Primary runtime and shader surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/art/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/art/ground.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/surface_effects.wgsl
- art-related bindings or neighboring shader modules used by the worldmap art pipeline
Primary runtime routing and consumer surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/scene/world/art/statics_collect.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
Likely metadata and conversion inputs:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- override and review-artifact surfaces from earlier phases
Reference clue:
- third-party statics.fx is only an architectural clue that statics or art remain separate from terrain logic, not proof of exact original EC behavior

### Mandatory Runtime Goal
The runtime must support an explicit art-owned wet or liquid handling path that remains ownership-correct.
That means:
- tileart-owned visible base remains the authoritative albedo input
- art-owned support refs may modulate the result when the evidence justifies it
- regular art and surface-like art-ground paths may differ, but both must remain inspectable
- art-owned wet or liquid entries must not be silently rerouted into terrain-owned liquid logic just because their support resources overlap
- reduced art-side behavior is acceptable when support or motion evidence is partial, as long as the uncertainty is surfaced

### Required Runtime Behavior Changes
The implementation must address all of the following.

1. Explicit art-owned wet or liquid classification
Required outcome:
- introduce a runtime-facing distinction for art-owned wet or liquid entries, separate from both ordinary static art and terrain-owned liquid materials
- classification must be grounded in tileart ownership, tile type, roles, and overrides
Guardrail:
- do not collapse art-owned liquid handling into the terrain-liquid material mode

2. Visible base plus support input wiring
Required outcome:
- wire the chosen art-owned visible base into the correct art or art-ground shader path
- where evidence supports it, wire one support input such as ripple-like, normal-like, or distortion-like data
- keep support semantics distinct from the visible base and from terrain solid-blend semantics
Guardrail:
- do not reinterpret a support input as a replacement visible base

3. Geometry-path separation
Required outcome:
- make it explicit whether a tile is rendered through regular art path or surface-like art-ground path
- ensure support-aware logic respects that distinction instead of assuming one geometry path fits both
Guardrail:
- do not hide path-specific behavior behind one opaque “wet art” branch with no inspection output

4. Conservative motion and support behavior
Required outcome:
- add motion or perturbation semantics only where evidence justifies them
- when support evidence exists but motion semantics are unclear, prefer reduced behavior plus diagnostics
Guardrail:
- do not hardcode terrain-liquid motion rules into art-owned wet entries without traceable evidence

5. Ownership and routing transparency
Required outcome:
- diagnostics must show that the entry is tileart-owned, which runtime path it used, which support input it used if any, and whether any override influenced the result
Guardrail:
- do not let support-package origin override owner identity in runtime reasoning

### Required Inspection Or Debug Output
At minimum, one inspection path must show for representative art-owned wet or liquid entries:
- owner kind and owner id
- whether the entry is classified as art-owned wet or liquid
- whether it rendered through regular art or surface-like art-ground path
- chosen visible base
- chosen support input, if any
- any motion or perturbation mode, if any
- whether an override influenced classification or input choice
- whether any abnormality or uncertainty remains
This must let a reviewer distinguish art-owned wet handling from both ordinary static art and terrain-owned liquid behavior.

### Required Validation Targets
Validation for this stage should focus on tileart-owned wet or liquid entries. Use representative samples from at least these families when available:
- art-owned marsh or water-like surfaces
- blood, slime, lava, or similar wet-looking art-owned entries with visible bases
- surface-like art entries that visually behave like ground-aligned wet surfaces
- other tileart-owned liquid or wet entries previously excluded or mishandled by the old blanket Liquid rule
The point is to show that art-owned wet and liquid content now behaves like ownership-correct special art, not like ordinary statics and not like terrain.

### Required Tests Or Validation
Add focused executable tests or targeted validation covering at least these cases.

1. Art-owned wet or liquid eligible case
- one test or targeted validation proving a tileart-owned wet or liquid entry with a credible visible base and support evidence uses the art-side wet or liquid path rather than ordinary static fallback

2. Surface-like art-ground case
- one test or targeted validation proving a surface-like art entry uses the correct art-ground routing and support-aware handling without being mistaken for terrain-owned liquid

3. Reduced behavior case
- one test or targeted validation proving an art-owned wet or liquid entry with uncertain support semantics can still render via a reduced art-side path with explicit diagnostics

4. Ownership transparency case
- one test or targeted validation proving diagnostics keep tileart ownership explicit even when support resources originate from terrain-support packages

5. No terrain-liquid semantic leakage
- one test or targeted validation proving terrain-owned liquid motion or support rules are not blindly copied onto art-owned wet entries without evidence

6. No extra-pass scope creep
- one test or targeted validation proving this stage does not accidentally introduce reflection, foam, waterfall, splash, glow, lava-bubble, or post-lighting extra-pass behavior

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- what the explicit art-owned wet or liquid runtime path is
- how regular art and surface-like art-ground cases are distinguished
- how the visible base and optional support input are chosen
- how ownership is preserved in routing and diagnostics
- which richer wet or liquid behaviors remain deferred to the extra-pass stage
It must explicitly state that this stage solves tileart-owned wet or liquid handling separately from TerrainDefinition-owned liquid materials.

### Non-Goals And Guardrails
- Do not merge art-owned wet logic into terrain-owned liquid logic.
- Do not add extra reflection, foam, waterfall, splash, glow, or lava-bubble passes yet.
- Do not let support-package origin override tileart ownership.
- Do not hardcode terrain-liquid motion behavior onto art-owned wet entries without evidence.
- Do not flatten regular art and surface-like art-ground behavior into one opaque path.
- Do not widen into a general all-effects art-material framework.

### Acceptance Criteria
This stage is complete only if all of the following are true:
1. an explicit art-owned wet or liquid runtime path exists for appropriate tileart-owned entries
2. regular art and surface-like art-ground cases are distinguished and inspectable
3. visible base and support input choice remain ownership-correct and inspectable
4. reduced or uncertain art-owned wet behavior remains diagnosable rather than hidden
5. tileart ownership remains explicit even when support resources come from terrain-support packages
6. no terrain-owned liquid behavior or extra-pass logic was introduced under the guise of art-owned wet handling

### Suggested Work Order
1. identify the minimal runtime seams where tileart-owned wet or liquid entries can gain a distinct art-side material mode
2. define conservative eligibility and input-selection rules for regular art and surface-like art-ground cases
3. wire the art-side wet or liquid path while preserving ownership and routing transparency
4. add reduced behavior for uncertain support cases instead of overfitting speculative semantics
5. extend inspection output so geometry path, ownership, and support use are visible
6. validate with one regular art case, one surface-like art-ground case, and one reduced or uncertain case before moving on
This ordering matters because the main risk in this stage is collapsing ownership-correct art behavior into terrain logic or opaque fallback.

### Final Instruction To The Agent
Treat this stage as tileart-owned wet or liquid support only. Keep ownership explicit, keep geometry-path differences visible, use support inputs only where evidence justifies them, and surface uncertainty honestly. Leave extra-pass experimentation for the final runtime stage.

## Phase 10G Agent Handoff

Assumption: “Phase 10G” refers to the seventh and final runtime stage under the Phase 10 runtime sub-plan: the extra-pass and post-lighting exploration stage. This stage comes only after earlier runtime stages have already validated base correctness, world-space repetition, single-terrain behavior, authored solid blending, terrain-owned liquid support, and art-owned wet or liquid handling. This is intentionally the highest-uncertainty stage. The agent must treat it as exploratory, incremental, and strongly diagnostics-driven.

### Objective
Evaluate whether preserved resources and earlier evidence justify additional shading passes or post-lighting layers for effects such as:
- highlights
- reflection-like overlays
- waterfalls
- foam
- splash
- lava bubbles
- glow or flare-like overlays
- other post-lighting enhancements
The goal is not to maximize spectacle blindly. The goal is to determine which preserved resources actually warrant extra runtime passes, and to introduce only those passes whose contribution can be isolated, inspected, and justified.

### Why This Stage Must Be Separate
This stage is worth isolating because it has the weakest evidence and the highest risk of aesthetic overreach. It tests this claim:
- some preserved support resources may imply additional post-base or post-lighting visual treatment, but these treatments must only be added after the core material pipeline is already correct
If this stage is attempted earlier, any incorrect visual result becomes impossible to localize between:
- wrong base
- wrong scale
- wrong solid blend
- wrong liquid support
- wrong art-owned wet routing
- wrong extra-pass logic
This stage must therefore be explicitly exploratory and last.

### Scope
Included:
- evaluate whether specific preserved resources justify extra runtime passes or post-lighting layers
- introduce extra-pass behavior only for narrowly justified cases
- keep pass participation explicit and inspectable
- support disabling or bypassing each extra-pass feature independently for validation
- surface uncertainty or speculation explicitly when a pass is heuristic rather than source-proven
Excluded:
- no silent global “make it shinier” pass across all materials
- no replacement of the established base, blend, or liquid stages
- no assumption that every glow-like, env-like, cube-like, waterfall-like, or splash-like resource should become a runtime pass
- no hard commitment to original EC parity where the evidence is still only suggestive
- no burying of exploratory behavior inside opaque shader constants with no diagnostics or feature flags

### Phase Dependency Context
This stage assumes all earlier runtime stages are already stable enough that any remaining visual mismatch can reasonably be attributed to missing secondary or post-lighting treatment rather than to first-order material errors. It also assumes earlier phases preserved:
- extra-pass candidate resources
- role or note fields that distinguish likely extra-pass-only candidates
- diagnostics and overrides capable of surfacing when an exploratory pass is in use
If those prerequisites are weak for a given effect family, the agent should prefer leaving the effect disabled and documenting the uncertainty rather than inventing global behavior.

### Evidence Rules
Use this precedence when deciding whether to implement an extra pass or post-lighting effect:
1. preserved source-grounded evidence and owner linkage are stronger than name-only speculation
2. stable and speculative role data can suggest candidate families, but must not by themselves prove the final runtime math
3. explicit overrides or feature toggles outrank heuristic enablement
4. third-party reconstruction clues may inspire architecture, but not prove exact original EC behavior
5. if an effect cannot be tied to a narrow visual target and a narrow input set, do not generalize it into the whole renderer
If an effect remains speculative, implement it only behind explicit gating and diagnostics, or leave it deferred.

### Required Inputs
Primary runtime and shader surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/art/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/art/ground.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/surface_effects.wgsl
- any neighboring shading, lighting, or postprocess shader modules implicated by earlier runtime stages
Likely metadata and evidence inputs:
- rich metadata sidecars and manifests from earlier phases
- generated review artifacts and audit outputs identifying extra-pass-only candidates
- override surfaces for selectively enabling or suppressing pass usage
Likely inspection seams:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
- CLI inspection examples or debug outputs that can surface pass participation

### Mandatory Runtime Goal
Any extra-pass or post-lighting behavior introduced in this stage must be:
- narrowly scoped
- explicitly gated
- inspectable
- independently disableable
- attributable to a specific preserved resource family or effect hypothesis
This stage is not complete when “things look shinier”; it is complete when any added pass can be justified, toggled, and distinguished from the core material stages.

### Required Runtime Behavior Changes
The implementation must address all of the following.

1. Explicit pass or overlay classification
Required outcome:
- any added extra-pass family must have an explicit runtime classification or feature gate
- diagnostics must reveal when a pass is active and why
Guardrail:
- do not hide exploratory pass logic inside generic shader tweaks with no visibility

2. Narrow effect-family introduction
Required outcome:
- implement only the minimum number of extra-pass families justified by current evidence, such as reflection-like overlays, foam-like highlights, waterfall enhancement, or lava-bubble enhancement
- each family should have a clearly delimited input set and target material set
Guardrail:
- do not lump all speculative effects into one monolithic “special FX” branch

3. Post-lighting or overlay ordering clarity
Required outcome:
- if an effect is applied after core lighting or as an overlay, make that ordering explicit in code and summary output
- if the effect modifies core shading instead, justify why it is not a separate pass
Guardrail:
- do not blur base-material correctness with exploratory post-lighting enhancement

4. Toggleable experimentation
Required outcome:
- each exploratory effect family must be easy to disable for comparison testing
- inspection output must reveal whether a visible result depends on a pass being enabled
Guardrail:
- do not make exploratory passes mandatory to make the scene look “acceptable” if the core material stages are already correct

5. Conservative unknown handling
Required outcome:
- if preserved resources suggest an effect family but the runtime math or ownership mapping remains uncertain, expose the uncertainty and consider leaving the family unimplemented or feature-gated
Guardrail:
- do not convert uncertain naming evidence into globally enabled rendering behavior

### Required Inspection Or Debug Output
At minimum, one inspection path must show for representative entries:
- whether any extra-pass or post-lighting family is active
- which effect family is active
- which preserved input or metadata evidence triggered it
- whether an override or feature flag enabled or disabled it
- whether the effect is post-lighting, overlay, or another explicitly described ordering
- whether any uncertainty note remains
This must let a reviewer tell whether a visible highlight or overlay is coming from a deliberate exploratory pass or from the underlying core material path.

### Required Validation Targets
Validation for this stage should focus on families where preserved evidence suggests additional treatment may matter. Use representative samples from at least some of these families when available:
- waterfalls or falling-water scenes
- foam or shoreline-highlight scenes
- lava with bubbling or emissive-like breakup
- reflection-like or env-like overlay candidates
- glow or flare-like highlight cases
The point is not blanket activation. The point is targeted validation that a specific exploratory pass improves a specific class of scene while remaining disableable and inspectable.

### Required Tests Or Validation
Add focused executable tests or targeted validation covering at least these cases.

1. Feature-gated extra-pass case
- one test or targeted validation proving an exploratory pass can be enabled or disabled explicitly and that the rendered difference is attributable to that pass

2. Inspection fidelity for pass participation
- one test or targeted validation proving diagnostics report which extra-pass family is active and why

3. Narrow-target effect case
- one test or targeted validation proving a chosen extra-pass affects only its intended material or scene family rather than leaking globally

4. Uncertainty-preserving case
- one test or targeted validation proving a speculative effect family can remain disabled or explicitly low-confidence rather than being globally forced on

5. No core-stage regression
- one test or targeted validation proving enabling an exploratory pass does not silently replace or break the established base, scale, solid-blend, liquid-terrain, or art-owned wet stages

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- which extra-pass or post-lighting families were introduced, if any
- which evidence justified each family
- how each pass is gated and inspected
- which scene families are expected to benefit
- which candidate effects remain too speculative and therefore deferred
It must explicitly state that this stage is exploratory and should remain subordinate to the correctness of the earlier runtime stages.

### Non-Goals And Guardrails
- Do not make exploratory passes globally mandatory.
- Do not let extra-pass work mask a regression in base, repetition, blend, or liquid-core stages.
- Do not treat speculative names alone as proof of original behavior.
- Do not merge unrelated effect families into one opaque branch.
- Do not hide pass ordering or activation behind unexplained shader constants.
- Do not widen this stage into a catch-all visual rewrite.

### Acceptance Criteria
This stage is complete only if all of the following are true:
1. any introduced extra-pass or post-lighting family is explicitly gated and inspectable
2. each active pass can be tied to a specific evidence family or override decision
3. targeted scene families can show isolated improvement without requiring the effect globally
4. speculative or uncertain effect families can remain disabled without blocking the rest of the runtime pipeline
5. enabling exploratory passes does not regress or obscure the correctness of earlier runtime stages
6. the summary clearly separates implemented exploratory passes from deferred speculative ideas

### Suggested Work Order
1. review preserved extra-pass candidates and rank them by evidence strength and visual payoff
2. choose at most one or a very small number of narrowly justified effect families first
3. implement each pass behind explicit gating and diagnostics
4. validate with targeted scenes and direct before-versus-after comparison while keeping earlier stages fixed
5. document deferred speculative families instead of overextending implementation
This ordering matters because the main risk in this stage is turning weak evidence into broad visual behavior that obscures the renderer’s core correctness.

### Final Instruction To The Agent
Treat this stage as controlled exploration, not as a finishing-polish free-for-all. Add only what can be justified, gated, and inspected. Keep earlier runtime stages authoritative, keep speculative families honest about uncertainty, and leave anything still too ambiguous in a documented deferred state rather than forcing it into the renderer.

## Phase 11 Agent Handoff

Assumption: “Phase 11” refers to the plan phase named “Clarify how the third-party world-space UV report maps into our data model”. This is not an implementation-heavy renderer phase. It is a documentation, evidence-mapping, and model-clarification phase that exists specifically to prevent future agents from treating the third-party reconstruction as if it were original EC source code.

### Objective
Explicitly map the concepts from the third-party world-space UV report and proof-of-concept renderer into UODynamapper’s current data model, while clearly separating:
- what is original-source-grounded evidence
- what is current UODynamapper behavior
- what is third-party reconstruction guidance
The goal is to preserve the useful architectural insights from the external report without letting them silently harden into false claims about original EC behavior.

### Scope
Included:
- explicitly record that the proof-of-concept renderer is a third-party best-guess reconstruction, not original EC source
- map its “Stretch” and related concepts onto current UODynamapper fields such as TerrainDefinition texture_repetition and tileart texture_stretch
- record where UODynamapper currently derives stretch from texture extent rather than explicit per-material parameters
- preserve both decoded repetition values and actual texture extents as separate comparable signals
- document unresolved questions such as terrain vertical scaling or one-to-one parameter mapping
Excluded:
- no new runtime behavior by default
- no new shader implementation by default
- no claim that conceptual similarity implies proven equivalence
- no retroactive reinterpretation of current fields as exact matches to the proof-of-concept unless evidence supports it

### Phase Dependency Context
This phase assumes that earlier work already identified:
- the third-party proof-of-concept shader set as reconstruction evidence only
- current UODynamapper runtime behavior in sampling.wgsl and related paths
- decoded repetition or stretch clues from TerrainDefinition.uop and tileart.uop
- the distinction between source-grounded evidence and reconstruction guidance
The purpose here is to make that distinction durable and explicit so later implementation phases do not drift into overstating certainty.

### Evidence Rules
Use this precedence when writing the mapping:
1. original UOP content and extracted names are stronger evidence than any reconstruction
2. current UODynamapper code is authoritative for current behavior, not for original EC semantics
3. the third-party report and proof-of-concept shaders are design references only
4. conceptual similarity must be labeled as conceptual unless a stronger proof exists
If any mapping remains uncertain, preserve it as “conceptually aligned but not proven one-to-one” rather than flattening it into a confident equivalence.

### Required Inputs
Primary current-behavior surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/sampling.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/worldmap/land/main.wgsl
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
Previously identified external reference surfaces:
- third-party proof-of-concept shader files such as single-terrain.fx, solid-terrain.fx, liquid-terrain.fx, and statics.fx
- any earlier notes or saved evidence classifying those files as reconstruction-only
Likely documentation target surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/docs/
- or an appropriate persisted review note or repo memory entry if the project already has a preferred place for this clarification

### Mandatory Clarifications
The implementation or documentation work must capture all of the following.

1. Reconstruction status of the third-party renderer
Required outcome:
- explicitly state that the proof-of-concept renderer is a third-party best-guess reconstruction and not original EC shader source
- ensure later agents cannot reasonably mistake it for original client proof
Guardrail:
- do not use tentative language that leaves the source status ambiguous

2. Stretch concept mapping
Required outcome:
- record that the proof-of-concept “Stretch” concept aligns conceptually with TerrainDefinition texture_repetition and tileart texture_stretch, but is not yet proven to map one-to-one
- explain which parts of the alignment are evidence-backed and which are inference
Guardrail:
- do not collapse “conceptually similar” into “same parameter with same semantics”

3. Current UODynamapper behavior
Required outcome:
- explicitly record that UODynamapper currently derives stretch from texture extent in the current world-space sampling path, whereas the proof-of-concept uses explicit per-material stretch parameters
- capture this as a present-behavior fact, not as a bug by itself
Guardrail:
- do not imply current UODynamapper behavior is already equivalent just because visuals may sometimes resemble the reconstruction

4. Dual-signal preservation rule
Required outcome:
- explicitly record that both decoded repetition values and actual texture extents should be preserved for later comparison
- make it clear that later precedence decisions depend on comparing these two signal families rather than discarding one early
Guardrail:
- do not let future agents assume texture extent alone or decoded repetition alone is already proven authoritative in every case

5. Open questions log
Required outcome:
- record unresolved issues such as terrain vertical scaling, exact precedence between explicit repetition and inferred size, and whether art scaling clues map cleanly onto the external report
- preserve these as explicit open questions for future comparison against client visuals or stronger evidence
Guardrail:
- do not present unresolved scaling questions as if later runtime phases already settled them universally

### Required Output
Leave behind one durable clarification artifact. It may be:
- a dedicated documentation note in docs/
- a clearly scoped section added to an existing technical note
- or another project-standard persisted artifact
The output must be easy for future agents to find and cite.
It should contain at minimum:
- evidence tier statement
- concept mapping table or equivalent structured comparison
- current UODynamapper behavior summary
- unresolved questions list
- implementation caution notes for future phases

### Required Content Structure
At minimum, the clarification output should include sections equivalent to:
- “What the third-party report is and is not”
- “Concept mapping into current data model”
- “Current UODynamapper behavior”
- “What remains unproven”
- “Rules for future implementation phases”
A compact table is acceptable where it improves precision. The critical requirement is clarity, not a specific markdown style.

### Required Validation
Validation for this phase is documentation-oriented but still must be concrete.
At minimum:
1. verify the clarification artifact explicitly labels the third-party renderer as reconstruction-only
2. verify it distinguishes conceptual alignment from proven equivalence
3. verify it names the current sampling.wgsl texture-extent-derived behavior explicitly
4. verify it preserves both decoded repetition and texture extent as separate signals for future comparison
5. verify it lists the remaining open questions rather than implying they are solved
This can be done by targeted review of the final artifact rather than runtime execution, but it should still be precise and checkable.

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- where the clarification artifact lives
- what key conceptual mappings it records
- what future misuse or confusion it is meant to prevent
- which open questions remain active after the clarification
It must explicitly state that this phase reduces conceptual ambiguity; it does not prove original EC shader semantics.

### Non-Goals And Guardrails
- Do not treat the proof-of-concept renderer as original source.
- Do not widen this phase into runtime rewrites.
- Do not use conceptual similarity to retroactively justify unproven parameter equivalence.
- Do not erase open questions for the sake of a cleaner narrative.
- Do not let the output read like a speculative design pitch; it must read like an evidence map.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. the third-party renderer is explicitly documented as reconstruction-only
2. its key concepts are mapped into the current data model without overstating certainty
3. current UODynamapper stretch behavior is explicitly recorded
4. decoded repetition values and texture extents are preserved as separate comparison signals in the documented model
5. unresolved scaling and precedence questions remain explicitly open
6. future agents can use the artifact to avoid overstating what is proven

### Suggested Work Order
1. collect the exact current-behavior facts from sampling.wgsl and the relevant parser fields
2. restate the third-party renderer’s status in explicit non-ambiguous language
3. write the concept mapping table or structured comparison
4. write the open-questions section and implementation caution notes
5. validate that the final artifact distinguishes “current behavior”, “conceptual alignment”, and “proven source evidence” cleanly
This ordering matters because the main risk in this phase is letting a useful design reference silently become faux-source truth.

### Final Instruction To The Agent
Treat this phase as evidence hygiene. Preserve the usefulness of the third-party report, but strip away any accidental aura of source authority. Future implementation work should be able to cite this artifact to say exactly what is known, what is merely aligned conceptually, and what remains unresolved.

## Phase 12 Agent Handoff

Assumption: “Phase 12” refers to the plan phase named “Preserve future implementation clues for blending, normal mapping, and extra passes”. This is a preservation and readiness phase. It exists to ensure that even when the current runtime cannot yet consume certain support resources, those resources are still classified, retained, and exposed in a way that makes later implementation possible without repeating archaeology work.

### Objective
Preserve all support candidates and implementation clues needed for future rendering features such as:
- Texture0 plus Texture1 plus AlphaMask blending
- liquid base plus ripple or normal-like perturbation
- reflection-like or env-like highlights
- distortion-like or flow-like support usage
- extra-pass families such as waterfall, splash, foam, flare, glow, and lava bubbles
The goal is to keep future rendering richness enabled by already-preserved evidence, rather than forcing a later agent to rediscover dropped or flattened support data.

### Scope
Included:
- preserve all support candidates needed for future solid-terrain blending
- preserve all support candidates needed for future liquid and support-aware rendering
- preserve NormalLike, distortion-like, and flow-like candidates separately rather than flattening them into generic support
- preserve extra-pass candidates from EffectTexture.uop and TerrainTexture.uop with enough metadata to reconnect them to future runtime work
- add an implementation-readiness field for every preserved support ref
Excluded:
- no requirement to implement the future render features now
- no need to treat preserved readiness classes as immediate runtime behavior
- no collapsing of distinct future-useful support types into a single “misc support” bucket for convenience
- no discarding of weak but still potentially meaningful support evidence solely because the current renderer cannot consume it yet

### Phase Dependency Context
This phase assumes earlier work already established:
- rich metadata sidecars and manifests
- package and family separation
- stable roles, speculative roles, and reason or confidence fields
- audit and review surfaces for support refs
- runtime stages that consume only a subset of preserved support information today
The purpose here is to make sure the preserved graph remains future-capable. If a support candidate is not usable today but could plausibly matter later, preserve it explicitly rather than leaving it implicit or dropping it.

### Evidence Rules
Use this precedence when deciding what to preserve and how to classify readiness:
1. source-grounded support refs and owner linkage are stronger evidence than name-only speculation
2. stable-role, speculative-role, and file-kind evidence together may justify a future-use classification even when runtime support is absent
3. current runtime inability to consume a ref is not evidence that the ref is unimportant
4. explicit overrides may correct classification or readiness tagging, but should not erase provenance
If readiness is uncertain, preserve the ref with a conservative readiness class and an uncertainty note rather than removing it from the preservation model.

### Required Inputs
Primary metadata and preservation surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
Relevant inventory, audit, and review artifacts from earlier phases:
- TerrainTexture.uop inventory outputs
- EffectTexture.uop inventory outputs
- owner-reference audits
- review artifacts and override files
Likely documentation or inspection surfaces:
- generated reports or CLI inspection outputs that can expose preserved readiness classes

### Mandatory Preservation Requirements
The implementation or data-model work must address all of the following.

1. Blend-ready support preservation
Required outcome:
- preserve all support refs that could plausibly be used for future Texture0 plus Texture1 plus AlphaMask blending
- keep Base, SecondaryBase, and AlphaMask-adjacent candidates distinguishable from more speculative support types
Guardrail:
- do not let current one-base or two-base runtime limitations collapse away future blend candidates

2. Liquid-ready support preservation
Required outcome:
- preserve all support refs that could plausibly be used for future liquid base plus ripple, normal-like, reflection-like, or env-like behavior
- keep their provenance and confidence visible even when runtime liquid support is partial or absent
Guardrail:
- do not demote liquid-support candidates into generic “unused texture” buckets

3. NormalLike, distortion-like, and flow-like separation
Required outcome:
- preserve NormalLike, distortion-like, and flow-like candidates as distinct conceptual classes or clearly distinguishable readiness states
- ensure they are not flattened into one undifferentiated “support image” category if later stages would need to tell them apart
Guardrail:
- current runtime inability to consume these inputs does not justify losing their distinction

4. Extra-pass candidate preservation
Required outcome:
- preserve candidates such as waterfall, splash, foam, flare, glow, lava-bubble, and related effect-like support inputs from EffectTexture.uop and TerrainTexture.uop
- retain enough metadata to reconnect them to future scene families, owner records, or effect families
Guardrail:
- do not require that every preserved extra-pass candidate already have a committed runtime implementation plan

5. Implementation-readiness field
Required outcome:
- add an implementation-readiness field for every preserved support ref
- minimum readiness classes should include:
  - base-safe
  - blend-ready
  - liquid-ready
  - normal-like experimental
  - extra-pass only
If an existing naming scheme is better for the codebase, it may vary, but the distinctions above must survive clearly.
Guardrail:
- readiness is a planning and preservation aid, not an automatic runtime enablement flag

### Required Data Guarantees
By the end of this phase, preserved support refs should be able to answer at least:
- what future feature family they may be useful for
- how strong the evidence is
- whether they are currently runtime-consumed or only preserved for later
- whether they are blend-oriented, liquid-oriented, normal-like experimental, or extra-pass-oriented
- whether any override influenced the readiness classification
If certainty is weak, keep the classification conservative and explicit rather than erasing the clue.

### Required Output
The phase should leave behind at least one durable preservation surface where future agents can inspect readiness classifications. This may be:
- an expanded sidecar field set
- a generated report
- an audit extension
- or a combination of those
The critical requirement is that the data be machine-readable and inspectable, not trapped only in prose.

### Required Validation
Add focused validation covering at least these cases.

1. Blend-ready preservation case
- one test or targeted validation proving a future blend candidate survives with a readiness class that distinguishes it from generic support

2. Liquid-ready preservation case
- one test or targeted validation proving a liquid-support candidate survives even when the current runtime does not fully consume it yet

3. NormalLike separation case
- one test or targeted validation proving a NormalLike or related experimental support ref remains distinguishable from other support categories

4. Extra-pass candidate preservation case
- one test or targeted validation proving an extra-pass candidate from EffectTexture.uop or TerrainTexture.uop remains preserved with enough metadata to be useful later

5. Readiness visibility case
- one test or targeted validation proving inspection or report output can show the readiness class and any uncertainty or override influence

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- what readiness classes now exist
- which future feature families they are meant to support
- how preserved support refs remain visible even when current runtime support is absent
- where future agents should look to consume these readiness classes
It must explicitly state that this phase is about preservation for future implementation, not immediate rendering behavior.

### Non-Goals And Guardrails
- Do not auto-enable runtime features just because a ref is tagged ready for a future family.
- Do not flatten all future-useful support refs into a single catch-all bucket.
- Do not discard uncertain but potentially meaningful support clues solely because they are not yet consumable.
- Do not let readiness tags override owner provenance or source evidence.
- Do not widen this phase into new renderer behavior unless a tiny inspection surface update is needed to expose the readiness data.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. future blend, liquid, normal-like, and extra-pass candidates remain explicitly preserved
2. an implementation-readiness field or equivalent survives for preserved support refs
3. readiness classes are machine-readable and inspectable
4. uncertain support clues remain preserved with conservative classification rather than being erased
5. future agents can determine which preserved refs were meant for later blend, liquid, or extra-pass work without repeating archaeology
6. no new renderer behavior was silently introduced under the guise of preservation

### Suggested Work Order
1. identify the support distinctions that later phases would otherwise lose
2. define the readiness classification scheme before editing preservation outputs
3. thread readiness classes through sidecars, reports, or audit surfaces with the smallest deliberate schema extension possible
4. extend one inspection or report surface so the readiness classes are visible
5. validate with one blend-ready, one liquid-ready, one normal-like experimental, and one extra-pass-only example
This ordering matters because the main risk in this phase is preserving data in a form that still does not communicate future usefulness clearly.

### Final Instruction To The Agent
Treat this phase as future-proofing by explicit preservation. Keep the support graph rich enough that later agents can build blend, liquid, and extra-pass features without redoing archaeology. Preserve uncertainty honestly, preserve provenance, and make readiness visible without turning it into premature runtime behavior.

## Phase 13 Agent Handoff

Assumption: “Phase 13” refers to the plan phase named “Validation, fixtures, and screenshot-family regression buckets”. This is the final consolidation phase of the roadmap. It exists to keep the pipeline and runtime from drifting silently after all of the parser, metadata, packaging, runtime, and review work introduced by earlier phases. The agent’s job is to turn the accumulated behavior into durable validation surfaces that match visual reality rather than only unit-level abstractions.

### Objective
Build durable validation coverage for the EC pipeline and runtime using a mix of:
- parser and metadata tests
- packer and sidecar integrity tests
- richer inspection output
- representative regression fixtures
- screenshot-family validation buckets
The goal is to ensure future changes can be checked against meaningful visual content families rather than only isolated ids or narrow code-path assumptions.

### Scope
Included:
- add parser tests for Textures-family classification in tileart, stable-role classification, speculative-role tagging, and owner-reference census correctness
- add packer tests for sidecar integrity, auxiliary-package owner linkage, and liquid base retention
- extend inspectors to print owner kind, source package, logical family, stable role, speculative role, heuristic reason, confidence, and runtime routing
- add regression fixtures for representative scene or content families
- validate by screenshot family rather than isolated ids only
Excluded:
- no large new feature work
- no new renderer architecture by default
- no replacing executable tests with screenshot checks alone
- no replacing screenshot-family checks with only micro-id assertions
- no treating a green unit-test suite as sufficient if visual families can still drift materially

### Phase Dependency Context
This phase assumes the earlier phases have already produced a rich enough system to validate:
- parser and metadata behavior
- sidecar and package outputs
- runtime base, scale, blend, liquid, art-owned wet, and exploratory pass behavior
- inspection and audit outputs
The purpose here is to lock those behaviors down in a way future agents can run and interpret. If a behavior is still intentionally uncertain or feature-gated, the validation surfaces should reflect that instead of pretending all outputs are final.

### Evidence Rules
Use this precedence when designing validation:
1. executable tests should validate concrete parser, packer, and sidecar invariants
2. screenshot-family buckets should validate user-visible outcomes that are not well-captured by micro-tests alone
3. inspection output should bridge machine assertions and visual interpretation by exposing the runtime state that produced a screenshot
4. a single isolated tile id is weaker validation than a representative content family when the visual target is inherently contextual
If a regression is easiest to detect visually, preserve the visual fixture. If it is easiest to detect structurally, use an executable test. Do not force everything into one validation style.

### Required Inputs
Primary parser, packer, and metadata surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/tileart.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/uocf/src/enhanced/terrain_definition.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tilemeta.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_art_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-conv/src/tex_land_ec.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/lib/udd-assets/src/tilemeta.rs
Primary inspection or debug surfaces:
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/tools/udd-conv-cli/examples/inspect_ec_tile_terrain_candidates.rs
- /mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/src/core/render/overlays/cursor_behavior.rs
Runtime surfaces implicated by screenshot-family validation:
- the worldmap land and art shader paths touched by earlier runtime stages
Likely fixture or docs surfaces:
- existing tests directories under lib/uocf, lib/udd-conv, and related crates
- any established location for fixture manifests, screenshots, or regression notes

### Mandatory Validation Areas
The implementation must address all of the following.

1. Parser and metadata classification tests
Required outcome:
- add executable tests covering Textures-family classification in tileart
- add executable tests covering stable-role classification
- add executable tests covering speculative-role tagging where applicable
- add executable tests covering owner-reference census correctness
Guardrail:
- do not rely only on manual inspection for metadata behavior that can be unit-tested directly

2. Packer and sidecar integrity tests
Required outcome:
- add executable tests covering sidecar integrity and multi-reference preservation
- add tests for auxiliary-package owner linkage
- add tests proving liquid base retention across packing and load boundaries
Guardrail:
- do not assume serialization is correct just because runtime visuals look plausible on a small sample

3. Inspection output completeness
Required outcome:
- extend current inspectors so they print at minimum:
  - owner kind
  - source package
  - logical family
  - stable role
  - speculative role
  - heuristic reason
  - confidence
  - runtime routing
- ensure this output is useful for connecting structural data to visual results
Guardrail:
- do not keep critical debugging state hidden only inside transient logs or internal branches

4. Regression fixtures by content family
Required outcome:
- add regression fixtures for representative families such as:
  - marble floors
  - cave floors
  - marsh water
  - lava
  - blood stains
  - waterfall or snow scenes
  - roads and plazas
  - grass-to-sand transitions
- fixtures may be screenshots, scene manifests, ids grouped by family, or a combination, as long as they are stable and reviewable
Guardrail:
- do not reduce each family to one arbitrary tile id if the visual target depends on scene context

5. Screenshot-family validation model
Required outcome:
- define validation buckets by screenshot family or scene family rather than only by isolated ids
- ensure those buckets can be used to detect visual drift in meaningful categories
Guardrail:
- do not treat screenshot comparison as a replacement for structural testing; the two must complement each other

### Required Validation Strategy
The phase should leave behind a clear validation strategy that future agents can follow.
At minimum it should establish:
- what is checked by parser tests
- what is checked by packer or sidecar tests
- what is checked by inspection output
- what is checked by screenshot-family regression buckets
- when to use family-level validation instead of isolated-id validation
This can be documented in a short validation note if the rules are not already obvious from test names alone.

### Required Output
The phase should leave behind at least:
1. executable parser or metadata tests
2. executable packer or sidecar integrity tests
3. richer inspection output exposing the runtime-relevant metadata fields
4. a set of regression fixtures or screenshot-family buckets
5. a short validation summary or note describing how these surfaces work together
The exact file layout may follow the project’s existing test and fixture conventions, but the outputs must be durable and reviewable.

### Required Validation Cases
Add focused executable tests or targeted validation covering at least these categories.

1. Parser classification case
- prove tileart Textures-family classification and related role tagging behave as expected

2. Sidecar integrity case
- prove rich metadata survives packing and loading without losing owner linkage or key fields

3. Liquid base retention case
- prove a liquid or wet case that should retain a visible base still does so through packing and load

4. Inspection richness case
- prove inspectors expose the minimum required metadata and routing fields

5. Screenshot-family regression case
- prove at least one representative family bucket can be compared meaningfully over time without relying only on one isolated id

6. Mixed validation complementarity case
- show that a structural test and a screenshot-family or scene-family validation together cover a regression class better than either alone

### Required Documentation Or Summary Output
Leave behind a short implementation summary stating:
- which parser and packer validations were added
- which inspection fields are now exposed
- which content families now have regression coverage
- how screenshot-family validation complements structural testing
- which areas still rely on manual review or exploratory judgment
It must explicitly state that content-family validation better matches the real visual goals than only tile-by-tile micro-tests.

### Non-Goals And Guardrails
- Do not rely on screenshots alone.
- Do not rely on unit tests alone when the user-visible regression is fundamentally scene-level.
- Do not hide runtime-routing state from inspectors if that state explains visible output.
- Do not make fixture selection arbitrary or unrepresentative of the target family.
- Do not widen this phase into feature development instead of validation hardening.

### Acceptance Criteria
This phase is complete only if all of the following are true:
1. parser and metadata tests exist for the key classification and census behaviors
2. packer or sidecar tests exist for integrity, owner linkage, and liquid base retention
3. inspectors expose the key metadata and routing fields needed to explain visual output
4. representative regression fixtures or screenshot-family buckets exist for the named content families
5. validation now combines structural and family-level visual checks rather than relying on only one style
6. future agents can use the validation surfaces to detect drift without re-deriving the testing strategy from scratch

### Suggested Work Order
1. identify the most drift-prone parser, packer, and runtime behaviors from earlier phases
2. add the structural parser and sidecar tests first
3. extend inspectors so runtime state can explain visual results
4. assemble representative fixture families and screenshot buckets second
5. write the short validation note or summary last so future agents can follow the intended validation model
This ordering matters because the main risk in this phase is building visual regression coverage that lacks enough structural context to explain failures.

### Final Instruction To The Agent
Treat this phase as validation hardening for the whole roadmap. Build checks that reflect how the renderer can actually fail: sometimes structurally, sometimes visually, often both. Keep the fixtures representative, keep the inspectors informative, and make the validation strategy durable enough that later changes cannot drift silently.
