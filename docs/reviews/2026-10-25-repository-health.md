# Repository health and EC rendering review — 2026-10-25

## Scope and evidence

Workspace-wide tests and compilation covered all 17 crates, all features, and all targets available on this Linux host. Source review concentrated on rendering, asset conversion/readers, runtime switches, unfinished feature paths, extraction, and the release/test setup. This is not native Windows/macOS validation or proof that every interactive path is correct. No formatter or linter was run, as required by ENGINEERING.md.

The pre-existing `.cargo/config.toml` edit, untracked `todos dynamapper.txt`, and user-supplied `docs/images/` references were preserved.

## Confirmed defects repaired

| Area | Defect and consequence | Repair |
| --- | --- | --- |
| EC map textures | The packer omitted `GameData.uop/data/gamedata/legacyterrainmap.csv`. The active manual KDL covered only part of the source routing graph. Missing tiles became black despite their material images already being packed. | Parse and embed the original CSV; load it as the source routing base and retain its complete subtype data. Explicit embedded and loose KDL remain separate policy overlays. |
| EC material layers | Reviewed liquid materials inherited positional detail/mask slots from source layouts containing ripple/reflection inputs. This could blend over the reviewed visible base with support images. | Keep the source-layer API, and resolve reviewed liquid shader roles separately as explicit base/normal inputs. Avoid applying the auxiliary solid blend twice. |
| EC support textures | RGB normal/mask/displacement data shared an sRGB atlas with color images and was interpreted after GPU color decoding. Neutral encoded normal channels consequently shifted away from 0.5. | Recover encoded RGB values before interpreting data maps; leave alpha and albedo color handling intact. The liquid sampler now recovers individual bump texels before interpolation; other data-map paths still need equivalent filtering or a linear data view. |
| EC art/statics | Tileart-owned material slots could be replaced with terrain-owned copies or rendered as upright billboards when terrain redirection failed. | Preserve tileart color ownership and sampling windows; draw those atlas slots as axis-aligned ground quads with authored repetition. Known support primaries use the visible material route. Retain the existing Classic shoreline path. |
| EC ground depth | Flat material quads used a sprite's constant priority depth, cutting triangular gaps against the terrain. | Use rasterized surface depth with a small geometric separation from terrain, also avoiding coplanar overlap flicker. |
| Walkable EC sprites | Bridge planks, stairs and furniture carrying `Surface` flags could select unrelated terrain through item/land ID collisions. | Existing sprite slots take precedence; the bridge fixture now uses wooden planks and stone stairs. |
| Liquid boundary blending | Underwater sand was averaged into water across terrain neighbors, producing pale sawtooth patches at static/terrain ownership changes. | Keep solid and liquid material classes separate in cardinal and diagonal transition weights. |
| EC terrain coverage | Solid transitions used a neighbor-color approximation, while liquid terrain bypassed it and stopped at hard ownership boundaries. | Compose shaded material layers with the recovered ownership bytes, chunk-edge/filter rules, original sorting keys, full-chunk base and 49-pass cap. |
| BC7 solid blocks | The encoder's constant-block endpoint table reconstructed 246 of 256 channel values incorrectly, corrupting reflection texture 1000017. | Generate exact Mode 5 endpoint pairs at compile time; exhaustively round-trip all channel values. Rebuild affected packages from original images. |
| Original liquid source roles | The runtime lacked the original material graph and used fallback water as its ordinary visible surface. | Preserve source definitions; separate fallback, bump and animated-surface roles. Match item water by source sampler identities and share sampling, flat lighting, grunge and wash across land/static owners. Retain source coordinate technique and bump scroll velocity. |
| Original liquid UV domains | The shared sampler passed positive map Y into an affine V reversal and used world coordinates for both owners' explicit techniques. Original stream 0 is world-indexed on terrain but local on items. | Convert world input to signed `(x,-y)` before the original transform. Supply each owner's stream 0 separately for Technique 2, keeping bump/world inputs, reflection generation, local masks and terrain coverage distinct. |
| Shared liquid filtering | Terrain used a linear sampler while item liquid inherited a nearest sampler, so the same toggle produced different output. Slot-center clamping broke wrapped-edge interpolation, and recovering encoded bump data after sRGB filtering changed offsets. | Use source-bounded explicit taps with the same point/bilinear policy for both owners. Wrap tiled footprints, clamp unit masks, and recover encoded bump RGB before interpolation. Preserve existing preset choices. |
| Item shoreline composition | Different shoreline items shared only the first three liquid images, discarding their fourth-source coverage and blend parameters. Applying the affine V flip without the original mesh flip also reversed masks. | Preserve item identity alongside the shared base, reuse packed auxiliary pixels with original item Technique/Tweaks and local UVs, and blend authored alpha. The coast now has curved coverage instead of whole-cell steps. |
| Acid border material | Local fallback masks differ from the terrain fallback image. Requiring all three images to agree rejected valid acid graphs and selected blue water. | Also match the active bump/surface pair with original repetitions, coordinate technique and scroll parameters. Preserve the item's fourth-stage coverage. |
| NoDraw statics | The art renderer drew diagnostic black tiles with green "no draw" lettering, obscuring the acid/rock boundary. | Honor the retained source NoDraw flag in art emission and visible-object picking; retain dot diagnostics and full cell inspection. |
| Static visibility | Disabling statics returned before clearing cached draw batches; existing opaque/transparent/shadow draw entities remained alive. | Clear instance/batch outputs and invalidate output-list reuse. Keep decoded chunks reusable when enabled again. |
| Optional KR encoder feature | `tile_mappings_builder` imported a module that existed but was not declared, breaking all-features compilation. | Declare the module under its feature gate. |
| UOP extraction | Lexical traversal checks allowed an existing symlink parent or destination to redirect writes outside the output root. | Canonicalize the output root and check existing destination ancestry before creating directories or writing; reject destination symlinks, including dangling links. This guards existing symlink escapes, not concurrent hostile directory replacement. |

## Source-derived terrain reconstruction

The standard original EC GameData package contains **2,705** unique routes. The FP source contains **4,162**. Each row names `legacyId,newId,newSubType`; the CSV is retained verbatim in `metadata/legacy_terrain_map.csv`, so subtype information and the original source remain inspectable.

For the configured standard runtime map 0:

| Check | Before | After original source routes plus existing explicit overlays |
| --- | ---: | ---: |
| Map cells examined | 29,360,128 | 29,360,128 |
| Missing material cells | 671,602 | 0 |
| Distinct missing land IDs | 779 | 0 |

Examples from the original source include `203 -> material 4, subtype 0`, `217 -> material 1, subtype 0`, and `15853 -> material 69, subtype 1`.

The reconstruction does **not** apply the proposed KR compatibility map. The original CSV differs from **232** entries in the existing manual routing table. Those explicit policies were preserved, and the coverage diagnostic reports conflicts against the retained source table. Replacing these policies with source choices requires a separate visual/content review.

`LegacyTerrain.uop` was also inspected: its 2,723 standard-client XML definitions expose land ID, name, and `texMap`. A matching low numeric ID in `Texture.uop/WorldArt` can be an unrelated static image: original land ID 203 describes forest, while WorldArt image 203 is wall artwork. Neither numeric proximity nor that XML alone establishes a usable image-family route.

The initial routing-only package added the original CSV while preserving **all 10 existing decoded package payloads byte for byte**. That phase required no image recompression. The subsequent liquid investigation found stored BC7 corruption and rebuilt the package from original images, as described below.

Installed package: `/home/claudio/test/uddp/tex_land_ec.uddp`.
Original backup: `/home/claudio/test/uddp/tex_land_ec.before-source-routing-20261025T140110.uddp`.
The initial routing-only SHA-256 was `ffe7c86cfe7dc51d1bc7be1e556802e9f46e00ae329cc18308d5aa3b5c27fb08`. The current rebuilt candidate is documented below.

Shoreline item 6038 demonstrates a separate source-selection distinction: the packer stores primary-selected image 1000013 (alpha/support), while the runtime's role-aware chooser selects visible image 2000051. Inspecting only the latter incorrectly labels the already-packed support pixels as color. The renderer now checks the packer's recorded selection before assigning tileart-owned color quads.

Reproduce the source/override coverage inspection with:

```sh
cargo run -p udd-conv-cli --example analyze_tex_land_ec_runtime_coverage -- \
  /path/to/tex_land_ec.uddp /path/to/ec-client 0 25 \
  dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl
```

Future `pack-ec-textures` conversions read the original table from a source `GameData.uop` automatically.

## Original-material follow-up

The user supplied original-client reference screenshots and map-0 fixture `[2834, 987, 0]`. The follow-up corrected sprite ownership and liquid transition boundaries, then traced the original `UOWaterShader` and terrain counterpart in `UOSA.exe`. The reproducible string/PE-section/xref/vtable procedure, executable hash, addresses, observed bindings, and proof boundary are in [ec_original_water_code.md](../agent_notes/ec_original_water_code.md).

Material 5 uses source 1000013 for bump displacement, source 1000017 for the bump/environment surface, and source 2000051 for its hardware fallback. Its animated surface is a 2D image; the name `cube3` does not establish a cubemap. Original Technique parameters distinguish water reflection coordinates from lava/acid explicit coordinates. The terrain graph, including the original parameters, now survives conversion as `metadata/source_materials.json`. Item water is matched to this graph through its original samplers and parameters, including the active-pair route for local fallback masks, never by treating an item graphic as a land ID.

The rebuilt candidate contains 201 source materials, 207 unique source textures and five BC7 atlas pages. It uses the corrected codec with RDO disabled for this fidelity check. For source 1000017, packed-image RGB RMSE is 0.604 on the 0–255 channel scale; an independent Pillow DDS decode agrees. The old package had 13,968 severe pixel errors in this image; the rebuild has zero under the same squared-RGB-error threshold of 500. Existing packages need a source rebuild; changing the decoder cannot recover ambiguous corrupted endpoint pairs. See [bc7_solid_block_integrity.md](../agent_notes/bc7_solid_block_integrity.md).

Rebuilt candidate SHA-256: `02a331ff5e9b199022b5226923f6fbbd05ae3ab1c9aa268e3ff43a87c7d7fe4a`. A new census still finds **zero missing material cells across all 29,360,128 map-0 cells**, with the existing explicit KDL overlays retained. No KR compatibility reconstruction was substituted.

This candidate is now installed at `/home/claudio/test/uddp/tex_land_ec.uddp`.
The preceding package is preserved as
`/home/claudio/test/uddp/tex_land_ec.before-source-materials-20261025T190636.uddp`.

At the bridge and previous shoreline fixtures, land/static water now has a continuous material response, and the pale sand/water blending patches are gone. Motion and pause were checked with two settled captures per run, rather than inferred from a single still. Original reflection appearance and authored shoreline/foam coverage remain incomplete; these changes establish shared inputs and rendering, not complete original-client parity.

The subsequent reflection fix replaces the guessed vector remap with the original affine texture transform (including repetition, vertical reversal and scrolling), and preserves unsigned bump samples from the source DDS. Bright environment-image structure is now visible in the bridge and coast captures. The exact camera/clock response and shoreline composition remain incomplete. Original item fourth-stage RGB/alpha operations are documented in `ec_original_water_code.md`; their authored alpha must be evaluated together with the underlying terrain rather than applied as a blanket water mask.

Original item shader parameters now survive `TileArtEntry` processing and tilemeta conversion in `metadata/item_materials.json`. Missing/malformed source material metadata fails with a rebuild message. The installed test package was updated from the matching FP source after all 39,062 items with material blocks passed sampler identity checks. Its four existing metadata payloads are byte-identical. The renderer can now query the original item parameters; selecting and composing each item's fourth texture remains unfinished.

## Remaining broken or incomplete paths

| Priority | Finding | Evidence and practical limit |
| --- | --- | --- |
| P2 | Free perspective camera zoom has no geometric effect. | `sys_camera_zoom` changes RenderZoom, perspective projection keeps a constant FOV, player-follow dolly is disabled in free-camera mode, and free-camera movement does not consume zoom. Follow-camera perspective uses dolly zoom and is a separate path. |
| P2 | Switching between map packages at runtime is unfinished. | Startup loads only the selected map. WorldGeoData contains only loaded maps, so the teleport dialog rejects other facets even if corresponding package files exist. It does not lazily load them. |
| P2 | EC liquid/effect parity is incomplete. | Original bump/environment inputs and the traced affine transform now have a shared path. Exact camera/clock equivalence, complete sampler/filter parity, terrain-specific extra inputs, foam, waterfalls, lava bubbles and refraction remain unverified or unimplemented. Item fourth-source stages and unit-mask clamp/reflection-wrap choices are now reconstructed. The recovered highlights do not establish complete original-client reflection parity. |
| P2 | EC shoreline/effect parity remains incomplete. | Original per-item fourth images, blend rules and local UVs now smooth the coast at `[1445, 1790, 0]` and the bridge boundary. Exact original camera geometry and alpha-property timing are still unproven; lava/acid/swamp expose separate terrain coverage and transition defects. |
| P2 | Complete terrain visual parity remains unfinished. | Source-backed EC terrain now composes ordered 32×32 ownership coverage, using original exponent bytes, chunk-edge/filter rules, material sorting keys and the 49-pass cap. Signed world UVs and owner-specific stream selection follow the recovered construction. Exact per-stage material filtering/lighting and the visual correctness of every explicit override remain unproven. |
| P2 | Full-source EC art/material composition is incomplete. | The existing packer still crops art windows. Other multi-layer item shaders, authored geometry/windows, unusual liquid/stain stacks, and support-role interpretation need their own family checks. The new ground route fixes geometry/ownership without claiming full shader parity. |
| P3 | Cross-facet and visual regression coverage remains incomplete. | The audited payload is map 0; content-family fixtures contain TBD transition IDs and do not constitute automated image golden tests. |
| P3 | Several roadmap features remain planned rather than implemented. | Clipmaps, complete mobile world rendering, world export, projected light decals, and network/editor workflows remain tracked in docs/TODO.md. Converter/reader or inspector support alone does not establish world-renderer support. |

The historical height-scale concern was not treated as a confirmed bug: the existing documented geometric derivation supports the current scale. Existing accepted Classic water ownership/fog behavior was preserved.

## Validation

- Latest full workspace, all-features test run: **665 passed, 0 failed, 4 intentionally ignored slow brute-force tests** across 57 test groups, including source item material retention/validation, per-item fourth-source selection, and the 16-bit GPU item lookup. That run contained **107** renderer tests; the subsequent terrain-integration renderer-only run passed **111**, including coverage sorting/alias selection, map invalidation, the 49-pass cap and WESL composition.
- Focused extraction gate: **5 passed**, including symlink-parent and dangling-destination coverage.
- Original CSV parsing, source-UOP-to-UDDP round-trip, subtype preservation, routing precedence, source material retention, liquid-role isolation, sprite ownership, shared liquid payloads, BC7 channel integrity, static visibility, and shader composition have regression coverage.
- `cargo check --workspace --all-targets --all-features --locked`: passed for the source-parameter implementation.
- `cargo build -p dynamapper --locked`: passed again after removal of the temporary capture code.
- Render checks run with isolated settings and packages; every test window is assigned by its exact process ID to the KDE desktop named Desktop 4 before capture. Saved user rendering preferences are not changed.
- EC town and shoreline, plus the existing Classic water path, were inspected under Classic, Enhanced, and KR presets without shader/pipeline errors. The town's large black gaps and shoreline's purple support-map coloring are gone. Remaining EC shoreline defects are listed above.
- The temporary screenshot hook was removed before the final production build. No persistent desktop rule was added.
- Reflection follow-up: focused WESL composition passed; all three EC bridge presets, pause/motion, EC coast/town, Classic coast, lava, acid and swamp reached InGame without shader errors. The paused water crop matched exactly across two frames; the animated crop changed. Reusable PE helpers passed three mapping tests and reproduced the known executable references; the Ghidra helper and source-material/image extractor were exercised against the original inputs.

Tests and type checks do not certify perfect original-client visuals, all liquid/effect families, long-session stability, or the roadmap performance targets.

Terrain coverage research follow-up: **8 Python checks passed** (3 PE mapping
checks and 5 coverage-model cases). The two diagnostic CLI crates passed
all-target/all-feature checking; only the existing unused import warning remains.
The exporter and offline model processed the lava, acid, swamp and bridge chunks,
including 15 material layers at swamp. Applying the preview's loose routing KDL
produced identical ownership in all four fixtures. These are diagnostic/model
checks; this phase changes no renderer shader, asset package or GUI state and
does not claim repaired terrain transitions or new screenshot parity.

The subsequent runtime coverage phase passed all **111 renderer tests** and the
normal build after temporary capture-hook removal. Twelve settled native previews
covered the bridge in all three presets, acid in all three presets, lava, swamp,
EC coast/town, paused bridge water and Classic coast. Every preview verified
Desktop 4 placement and cleanup, with no shader/pipeline errors. The paused
water crop was identical between frames; the animated crop changed. Accepted
final captures use `terrain-coverage-final-` in `/tmp/uodynamapper-ec-followup/`.
The first frame-count-only lava capture was blank and was rejected; the final
hook waited 8 and 12 seconds after InGame before capturing. This phase changes
terrain composition without rebuilding packages or applying compatibility maps.
Texture/camera/reflection and several static-geometry differences remain visible.
Logs use `/tmp/uodynamapper-terrain-integration-` with `final-tests`,
`final-families`, `final-scenes`, `normal-build` and `normal-preview` suffixes.

The liquid-UV follow-up passed all **111 renderer tests**, including composition
with the separate owner-supplied stream 0. Twelve signed-world-UV previews covered
the existing preset/family matrix; five final previews after the item stream-0
correction covered acid in all three presets, lava and bridge water. All 17
verified Desktop 4 placement and cleanup, with no shader/pipeline errors. Final
paused acid pixels matched between frames; animated bridge water changed. The
paused before/after acid comparison isolates changes along static borders while
leaving the terrain interior intact. The earlier signed-world change also left
the town scene below the HUD and a bridge-plank crop byte-identical. These are
native-renderer comparisons, not registered original-client pixel parity.

Captures use `world-uv-` and `liquid-uv-final-` under the same temporary directory.
Logs are `/tmp/uodynamapper-world-uv-{families,scenes}.log` and
`/tmp/uodynamapper-liquid-uv-final-{tests,scenes,normal-build,normal-preview}.log`.
The original placement, UV construction and technique dispatch addresses are
documented in `ec_original_water_code.md`. This phase changes no asset packages,
compatibility mappings, filtering presets or geometry. Reflection/camera/filter
parity and authored static geometry/effects remain unfinished.

Local captures are available in `/tmp/uodynamapper-render/`: `before-source-routes.png`, `ec-{classic,enhanced,kr}-town.png`, `ec-{classic,enhanced,kr}-water-validated.png`, and `cc-{classic,enhanced,kr}-water.png`. Test logs are `/tmp/uodynamapper-final-workspace-tests.log`, `/tmp/uodynamapper-final-check.log`, and `/tmp/uodynamapper-final-build.log`.

Liquid filtering follow-up: **111 renderer tests passed**, with the new optional
GPU regression ignored by the ordinary suite and then run explicitly with a
passing result. Its 14 real-shader samples cover linear color/data differences,
wrap/clamp edges, negative coordinates, point sampling, alpha and a single-texel
slot surrounded by contrasting pixels in an sRGB array texture. Original sampler
construction was traced through direct Ghidra MCP tools; terrain sources use
filter 1, extra item sources clear it to 0, and the primary item source has a
flag-dependent choice. The runtime filter toggle remains explicit policy rather
than silently overriding every stage with one assumed original filter.

Thirteen final native captures cover bridge and acid in all three presets,
filtered lava/swamp/shoreline, town, Classic shoreline and an explicitly filtered
paused bridge. All verified Desktop 4 placement and cleanup, without shader
errors. Nearest-filtered paused bridge, acid and town pixels below the HUD match
the prior UV phase exactly. Both point and linear paused water crops remain
stable between frames, while animated filtered water changes. The native color
path uses hardware filtering inside a slot and explicit taps only at wrapped
edges; data filtering still requires four decoded taps. No atlas duplication or
package rebuild was introduced.

Two additional uncapped old/new bridge runs verified placement/cleanup. The
captured FPS samples were 50/55 before and 50/47 after. This short overlay check
is not a controlled benchmark or a claim of unchanged GPU cost; exact data
interpolation adds work. Earlier all-explicit-color sampling was replaced by
the native interior-color path to reduce unnecessary loads. Current captures
use `liquid-filter-final-` in `/tmp/uodynamapper-ec-followup/`; logs use
`/tmp/uodynamapper-liquid-filter-final-` with `tests`, `gpu-test`, `scenes`,
`performance`, `normal-build` and `normal-preview` suffixes. Reflection/camera,
per-stage faithful filtering and authored static geometry remain open.

Follow-up captures are in `/tmp/uodynamapper-ec-followup/`: `baseline-enhanced.png`, `ec-{classic,enhanced,kr}-bridge.png`, `ec-paused-bridge.png`, `ec-enhanced-coast.png`, `ec-enhanced-town.png`, `cc-{classic,enhanced,kr}-coast.png`, and `ec-enhanced-{lava,acid,swamp}.png`. The corresponding `.early.png` images retain the earlier settled frame. Motion changed water-only crop pixels; the paused pair matched exactly in that crop. Lava `[5744,2928]`, acid `[6512,464]`, and swamp `[1872,2416]` checks established successful source-family rendering without shader errors, not full overlay/coverage parity. The lava edges and swamp coverage also expose remaining composition work.

Follow-up logs: `/tmp/uodynamapper-water-followup-tests.log`, `/tmp/uodynamapper-water-params-tests.log`, `/tmp/uodynamapper-water-followup-check.log`, `/tmp/uodynamapper-water-followup-final-build.log`, and `/tmp/uodynamapper-water-followup-coverage.log`. Desktop placement and cleanup evidence is retained in `/tmp/uodynamapper-scenes-params-validation.log` and `/tmp/uodynamapper-liquid-families-validation.log`. Captures and logs are temporary local artifacts, not checked-in reference images.

Reflection follow-up captures use the `reflection-` prefix in the same temporary directory. Logs: `/tmp/uodynamapper-reflection-{tests,scenes,families,build}.log`; all captured previews verified Desktop 4 placement and cleanup. This phase does not rebuild asset packages or change their previous validation counts.

Item material retention logs: `/tmp/uodynamapper-item-material-{workspace-tests,check,build,preview,final-test,final-preserve}.log`. The normal binary reached InGame on Desktop 4 with the validated candidate. Installed `tilemeta.uddp` SHA-256 is `2ba4d5c43d4b3fc2ff75fab1fdb3a577e199d54e19f3fad3b7074dbcf8abbbf2`; its backup is `tilemeta.before-item-materials-20261026T144526Z.uddp` alongside it. This metadata phase does not change atlas pixels or claim new shoreline coverage.


Item composition follow-up: the two fixture masks were already packed correctly
and match original decoded DDSs exactly. No package rebuild or compatibility map
was applied. The shared auxiliary atlas now retains the item fourth sources even
when the optional terrain sidecar is absent; the ground shader selects them by
original graphic, with original blend parameters and corrected local UVs.
The independent original-map mask continuity comparison reports 0.005164075 mean
edge difference in the authored orientation versus 0.022726467 when vertically
reversed; this is coverage evidence, not pixel parity. New source/loader/UV findings
are in `docs/agent_notes/ec_original_water_code.md`.

Accepted captures use `item-composition-` in `/tmp/uodynamapper-ec-followup/`.
They include the bridge in all three presets, paused water, EC coast/town,
Classic coast and lava/acid/swamp. Desktop placement and cleanup logs are
`/tmp/uodynamapper-item-composition-accepted-{scenes,families}.log`.
The prior `composition-` captures deliberately retain the initial reversed-mask
failure; use the accepted prefix for the final output. Terrain-specific liquid
coverage defects remain visible in the family scenes and are not claimed fixed.

Item composition validation: workspace/all-features tests passed **665/665**
(4 intentional ignores, 57 groups); all-targets/all-features check passed with
only the pre-existing unused-import warning. The two-frame paused water crop
matched exactly; the animated crop changed. The 10 final native-capture previews
verified Desktop 4 placement and cleanup, with no shader/pipeline errors.
Logs use `/tmp/uodynamapper-item-composition-{workspace-tests,workspace-check,normal-build}.log`.

The matching original-client views added on 2026-10-26 distinguish acid-family
selection from diagnostic geometry. The 37×37-cell census around `[6512,464,0]`
contains 54 NoDraw item-8612 markers, plus acid border items with local source-1
masks. Matching active bump/surface inputs and honoring NoDraw removes the blue
water patches and black labeled wedges. No source package rebuild or compatibility
mapping was needed. Native captures use `liquid-routing-` in the same temporary
directory; the acid fixture was checked in all three presets, alongside bridge,
coast, town, Classic coast, lava and swamp. Every preview verified Desktop 4
placement and cleanup. No shader/pipeline errors were reported; paused bridge
water was identical across the two settled frames.

The original terrain coverage input is now traced through loader `0x461800` and
generator `0x461060`: a 32×32 per-layer ownership mask precedes serialized source
images and supplies alpha after bump/environment shading. The current heuristic
terrain transitions do not implement it. Exact mask weights, filtering, layer
ordering, camera geometry and reflection parity remain unfinished. Lava and swamp
still expose visible terrain/coverage differences. The detailed evidence and the
new reference filenames are recorded in `ec_original_water_code.md`.

Liquid-routing validation: **159 passed** in the affected renderer/assets suites
(107 renderer, 52 assets), plus three PE helper tests. Workspace all-targets and
all-features checking passed with only the existing unused-import warning in
`inspect_ec_tile_surface_signals`. The normal renderer was rebuilt after removing
temporary capture code, reached InGame on Desktop 4, and cleaned up successfully.
The routing inspector confirms acid items 11778/11816/11820 share payload 10049,
ordinary shoreline item 6038 retains payload 168, and item 8612 is hidden by its
NoDraw flag. Logs use `/tmp/uodynamapper-liquid-routing-` prefixes.
