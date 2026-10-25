# Repository health and EC rendering review — 2026-10-25

## Scope and evidence

Workspace-wide tests and compilation covered all 17 crates, all features, and all targets available on this Linux host. Source review concentrated on rendering, asset conversion/readers, runtime switches, unfinished feature paths, extraction, and the release/test setup. This is not native Windows/macOS validation or proof that every interactive path is correct. No formatter or linter was run, as required by ENGINEERING.md.

The pre-existing `.cargo/config.toml` edit, untracked `todos dynamapper.txt`, and user-supplied `docs/images/` references were preserved.

## Confirmed defects repaired

| Area | Defect and consequence | Repair |
| --- | --- | --- |
| EC map textures | The packer omitted `GameData.uop/data/gamedata/legacyterrainmap.csv`. The active manual KDL covered only part of the source routing graph. Missing tiles became black despite their material images already being packed. | Parse and embed the original CSV; load it as the source routing base and retain its complete subtype data. Explicit embedded and loose KDL remain separate policy overlays. |
| EC material layers | Reviewed liquid materials inherited positional detail/mask slots from source layouts containing ripple/reflection inputs. This could blend over the reviewed visible base with support images. | Keep the source-layer API, and resolve reviewed liquid shader roles separately as explicit base/normal inputs. Avoid applying the auxiliary solid blend twice. |
| EC support textures | RGB normal/mask/displacement data shared an sRGB atlas with color images and was interpreted after GPU color decoding. Neutral encoded normal channels consequently shifted away from 0.5. | Recover encoded RGB values before interpreting data maps; leave alpha and albedo color handling intact. Filtering still occurs through the shared sRGB view; a dedicated linear data view would be the precise future filtering solution. |
| EC art/statics | Tileart-owned material slots could be replaced with terrain-owned copies or rendered as upright billboards when terrain redirection failed. | Preserve tileart color ownership and sampling windows; draw those atlas slots as axis-aligned ground quads with authored repetition. Known support primaries use the visible material route. Retain the existing Classic shoreline path. |
| EC ground depth | Flat material quads used a sprite's constant priority depth, cutting triangular gaps against the terrain. | Use rasterized surface depth with a small geometric separation from terrain, also avoiding coplanar overlap flicker. |
| Walkable EC sprites | Bridge planks, stairs and furniture carrying `Surface` flags could select unrelated terrain through item/land ID collisions. | Existing sprite slots take precedence; the bridge fixture now uses wooden planks and stone stairs. |
| Liquid boundary blending | Underwater sand was averaged into water across terrain neighbors, producing pale sawtooth patches at static/terrain ownership changes. | Keep solid and liquid material classes separate in cardinal and diagonal transition weights. |
| BC7 solid blocks | The encoder's constant-block endpoint table reconstructed 246 of 256 channel values incorrectly, corrupting reflection texture 1000017. | Generate exact Mode 5 endpoint pairs at compile time; exhaustively round-trip all channel values. Rebuild affected packages from original images. |
| Original liquid source roles | The runtime lacked the original material graph and used fallback water as its ordinary visible surface. | Preserve source definitions; separate fallback, bump and animated-surface roles. Match item water by source sampler identities and share sampling, flat lighting, grunge and wash across land/static owners. Retain source coordinate technique and bump scroll velocity. |
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

Material 5 uses source 1000013 for bump displacement, source 1000017 for the bump/environment surface, and source 2000051 for its hardware fallback. Its animated surface is a 2D image; the name `cube3` does not establish a cubemap. Original Technique parameters distinguish water reflection coordinates from lava/acid explicit coordinates. The terrain graph, including the original parameters, now survives conversion as `metadata/source_materials.json`. Item water is matched to this graph by its first three WorldArt sampler identities, never by treating an item graphic as a land ID.

The rebuilt candidate contains 201 source materials, 207 unique source textures and five BC7 atlas pages. It uses the corrected codec with RDO disabled for this fidelity check. For source 1000017, packed-image RGB RMSE is 0.604 on the 0–255 channel scale; an independent Pillow DDS decode agrees. The old package had 13,968 severe pixel errors in this image; the rebuild has zero under the same squared-RGB-error threshold of 500. Existing packages need a source rebuild; changing the decoder cannot recover ambiguous corrupted endpoint pairs. See [bc7_solid_block_integrity.md](../agent_notes/bc7_solid_block_integrity.md).

Rebuilt candidate SHA-256: `02a331ff5e9b199022b5226923f6fbbd05ae3ab1c9aa268e3ff43a87c7d7fe4a`. A new census still finds **zero missing material cells across all 29,360,128 map-0 cells**, with the existing explicit KDL overlays retained. No KR compatibility reconstruction was substituted.

This candidate is now installed at `/home/claudio/test/uddp/tex_land_ec.uddp`.
The preceding package is preserved as
`/home/claudio/test/uddp/tex_land_ec.before-source-materials-20261025T190636.uddp`.

At the bridge and previous shoreline fixtures, land/static water now has a continuous material response, and the pale sand/water blending patches are gone. Motion and pause were checked with two settled captures per run, rather than inferred from a single still. Original reflection appearance and authored shoreline/foam coverage remain incomplete; these changes establish shared inputs and rendering, not complete original-client parity.

The subsequent reflection fix replaces the guessed vector remap with the original affine texture transform (including repetition, vertical reversal and scrolling), and preserves unsigned bump samples from the source DDS. Bright environment-image structure is now visible in the bridge and coast captures. The exact camera/clock response and shoreline composition remain incomplete. Original item fourth-stage RGB/alpha operations are documented in `ec_original_water_code.md`; their authored alpha must be evaluated together with the underlying terrain rather than applied as a blanket water mask.

## Remaining broken or incomplete paths

| Priority | Finding | Evidence and practical limit |
| --- | --- | --- |
| P2 | Free perspective camera zoom has no geometric effect. | `sys_camera_zoom` changes RenderZoom, perspective projection keeps a constant FOV, player-follow dolly is disabled in free-camera mode, and free-camera movement does not consume zoom. Follow-camera perspective uses dolly zoom and is a separate path. |
| P2 | Switching between map packages at runtime is unfinished. | Startup loads only the selected map. WorldGeoData contains only loaded maps, so the teleport dialog rejects other facets even if corresponding package files exist. It does not lazily load them. |
| P2 | EC liquid/effect parity is incomplete. | Original bump/environment inputs and the traced affine transform now have a shared path. Exact camera/clock equivalence, sampler-state parity, fourth-layer overlays, foam, waterfalls, lava bubbles and refraction remain unverified or unimplemented. The recovered highlights do not establish complete original-client reflection parity. |
| P2 | EC shoreline composition still has visible defects. | The shared liquid path and solid/liquid transition gate remove the prior pale patches and ownership color step. The coast at `[1445, 1790, 0]` still has angular cell coverage; authored fourth-image shoreline/foam composition needs implementation. Filling a material quad does not reproduce every original coverage mask. |
| P2 | Terrain transitions remain a reconstruction. | The shader blends neighboring materials heuristically. Material-slot coverage is complete for the audited map; original EC transition-alpha parity and the visual correctness of every explicit override remain unproven. |
| P2 | Full-source EC art/material composition is incomplete. | The existing packer still crops art windows. Multi-layer item materials, authored alpha coverage, unusual liquid/stain stacks, and support-role interpretation need their own family checks. The new ground route fixes geometry/ownership without claiming full shader parity. |
| P3 | Cross-facet and visual regression coverage remains incomplete. | The audited payload is map 0; content-family fixtures contain TBD transition IDs and do not constitute automated image golden tests. |
| P3 | Several roadmap features remain planned rather than implemented. | Clipmaps, complete mobile world rendering, world export, projected light decals, and network/editor workflows remain tracked in docs/TODO.md. Converter/reader or inspector support alone does not establish world-renderer support. |

The historical height-scale concern was not treated as a confirmed bug: the existing documented geometric derivation supports the current scale. Existing accepted Classic water ownership/fog behavior was preserved.

## Validation

- Follow-up full workspace, all-features test run: **660 passed, 0 failed, 4 intentionally ignored slow brute-force tests** across 57 test groups. The subsequent source-parameter changes also passed all **105** renderer tests, including liquid WESL composition.
- Focused extraction gate: **5 passed**, including symlink-parent and dangling-destination coverage.
- Original CSV parsing, source-UOP-to-UDDP round-trip, subtype preservation, routing precedence, source material retention, liquid-role isolation, sprite ownership, shared liquid payloads, BC7 channel integrity, static visibility, and shader composition have regression coverage.
- `cargo check --workspace --all-targets --all-features --locked`: passed for the source-parameter implementation.
- `cargo build -p dynamapper --locked`: passed again after removal of the temporary capture code.
- Render checks run with isolated settings and packages; every test window is assigned by its exact process ID to the KDE desktop named Desktop 4 before capture. Saved user rendering preferences are not changed.
- EC town and shoreline, plus the existing Classic water path, were inspected under Classic, Enhanced, and KR presets without shader/pipeline errors. The town's large black gaps and shoreline's purple support-map coloring are gone. Remaining EC shoreline defects are listed above.
- The temporary screenshot hook was removed before the final production build. No persistent desktop rule was added.
- Reflection follow-up: focused WESL composition passed; all three EC bridge presets, pause/motion, EC coast/town, Classic coast, lava, acid and swamp reached InGame without shader errors. The paused water crop matched exactly across two frames; the animated crop changed. Reusable PE helpers passed three mapping tests and reproduced the known executable references; the Ghidra helper and source-material/image extractor were exercised against the original inputs.

Tests and type checks do not certify perfect original-client visuals, all liquid/effect families, long-session stability, or the roadmap performance targets.

Local captures are available in `/tmp/uodynamapper-render/`: `before-source-routes.png`, `ec-{classic,enhanced,kr}-town.png`, `ec-{classic,enhanced,kr}-water-validated.png`, and `cc-{classic,enhanced,kr}-water.png`. Test logs are `/tmp/uodynamapper-final-workspace-tests.log`, `/tmp/uodynamapper-final-check.log`, and `/tmp/uodynamapper-final-build.log`.

Follow-up captures are in `/tmp/uodynamapper-ec-followup/`: `baseline-enhanced.png`, `ec-{classic,enhanced,kr}-bridge.png`, `ec-paused-bridge.png`, `ec-enhanced-coast.png`, `ec-enhanced-town.png`, `cc-{classic,enhanced,kr}-coast.png`, and `ec-enhanced-{lava,acid,swamp}.png`. The corresponding `.early.png` images retain the earlier settled frame. Motion changed water-only crop pixels; the paused pair matched exactly in that crop. Lava `[5744,2928]`, acid `[6512,464]`, and swamp `[1872,2416]` checks established successful source-family rendering without shader errors, not full overlay/coverage parity. The lava edges and swamp coverage also expose remaining composition work.

Follow-up logs: `/tmp/uodynamapper-water-followup-tests.log`, `/tmp/uodynamapper-water-params-tests.log`, `/tmp/uodynamapper-water-followup-check.log`, `/tmp/uodynamapper-water-followup-final-build.log`, and `/tmp/uodynamapper-water-followup-coverage.log`. Desktop placement and cleanup evidence is retained in `/tmp/uodynamapper-scenes-params-validation.log` and `/tmp/uodynamapper-liquid-families-validation.log`. Captures and logs are temporary local artifacts, not checked-in reference images.

Reflection follow-up captures use the `reflection-` prefix in the same temporary directory. Logs: `/tmp/uodynamapper-reflection-{tests,scenes,families,build}.log`; all captured previews verified Desktop 4 placement and cleanup. This phase does not rebuild asset packages or change their previous validation counts.
