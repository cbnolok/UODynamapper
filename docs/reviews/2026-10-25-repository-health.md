# Repository health and EC rendering review — 2026-10-25

## Scope and evidence

Workspace-wide tests and compilation covered all 17 crates, all features, and all targets available on this Linux host. Source review concentrated on rendering, asset conversion/readers, runtime switches, unfinished feature paths, extraction, and the release/test setup. This is not native Windows/macOS validation or proof that every interactive path is correct. No formatter or linter was run, as required by ENGINEERING.md.

The pre-existing `.cargo/config.toml` edit and untracked `todos dynamapper.txt` were preserved.

## Confirmed defects repaired

| Area | Defect and consequence | Repair |
| --- | --- | --- |
| EC map textures | The packer omitted `GameData.uop/data/gamedata/legacyterrainmap.csv`. The active manual KDL covered only part of the source routing graph. Missing tiles became black despite their material images already being packed. | Parse and embed the original CSV; load it as the source routing base and retain its complete subtype data. Explicit embedded and loose KDL remain separate policy overlays. |
| EC material layers | Reviewed liquid materials inherited positional detail/mask slots from source layouts containing ripple/reflection inputs. This could blend over the reviewed visible base with support images. | Keep the source-layer API, and resolve reviewed liquid shader roles separately as explicit base/normal inputs. Avoid applying the auxiliary solid blend twice. |
| EC support textures | RGB normal/mask/displacement data shared an sRGB atlas with color images and was interpreted after GPU color decoding. Neutral encoded normal channels consequently shifted away from 0.5. | Recover encoded RGB values before interpreting data maps; leave alpha and albedo color handling intact. Filtering still occurs through the shared sRGB view; a dedicated linear data view would be the precise future filtering solution. |
| EC art/statics | Tileart-owned material slots could be replaced with terrain-owned copies or rendered as upright billboards when terrain redirection failed. | Preserve tileart color ownership and sampling windows; draw those atlas slots as axis-aligned ground quads with authored repetition. Known support primaries use the visible material route. Retain the existing Classic shoreline path. |
| EC ground depth | Flat material quads used a sprite's constant priority depth, cutting triangular gaps against the terrain. | Use rasterized surface depth with a small geometric separation from terrain, also avoiding coplanar overlap flicker. |
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

The validated runtime package adds the original CSV while preserving **all 10 existing decoded package payloads byte for byte**. No texture recompression or replacement of authored image pixels was required; only container compression and the additional metadata changed.

Installed package: `/home/claudio/test/uddp/tex_land_ec.uddp`.
Original backup: `/home/claudio/test/uddp/tex_land_ec.before-source-routing-20261025T140110.uddp`.
The installed SHA-256 is `ffe7c86cfe7dc51d1bc7be1e556802e9f46e00ae329cc18308d5aa3b5c27fb08`, matching the visually checked candidate.

Shoreline item 6038 demonstrates a separate source-selection distinction: the packer stores primary-selected image 1000013 (alpha/support), while the runtime's role-aware chooser selects visible image 2000051. Inspecting only the latter incorrectly labels the already-packed support pixels as color. The renderer now checks the packer's recorded selection before assigning tileart-owned color quads.

Reproduce the source/override coverage inspection with:

```sh
cargo run -p udd-conv-cli --example analyze_tex_land_ec_runtime_coverage -- \
  /path/to/tex_land_ec.uddp /path/to/ec-client 0 25 \
  dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl
```

Future `pack-ec-textures` conversions read the original table from a source `GameData.uop` automatically.

## Remaining broken or incomplete paths

| Priority | Finding | Evidence and practical limit |
| --- | --- | --- |
| P2 | Free perspective camera zoom has no geometric effect. | `sys_camera_zoom` changes RenderZoom, perspective projection keeps a constant FOV, player-follow dolly is disabled in free-camera mode, and free-camera movement does not consume zoom. Follow-camera perspective uses dolly zoom and is a separate path. |
| P2 | Switching between map packages at runtime is unfinished. | Startup loads only the selected map. WorldGeoData contains only loaded maps, so the teleport dialog rejects other facets even if corresponding package files exist. It does not lazily load them. |
| P2 | EC liquid/effect parity is incomplete. | Foam, waterfalls, lava bubbles, reflection/refraction and other extra-pass candidates are preserved/classified but do not have their dedicated compositing passes. Generic wet animation is not equivalent to implementing these passes. |
| P2 | EC shoreline composition still has visible defects. | The final map-0 fixture at `[1445, 1790, 0]` no longer displays purple support maps, large depth-cut triangles, or coplanar flicker. Isolated pale patches and a tonal boundary between terrain and static water remain visible under all three presets. These need material/coverage and shared liquid-shading review; zero missing material slots does not resolve them. |
| P2 | Terrain transitions remain a reconstruction. | The shader blends neighboring materials heuristically. Material-slot coverage is complete for the audited map; original EC transition-alpha parity and the visual correctness of every explicit override remain unproven. |
| P2 | Full-source EC art/material composition is incomplete. | The existing packer still crops art windows. Multi-layer item materials, authored alpha coverage, unusual liquid/stain stacks, and support-role interpretation need their own family checks. The new ground route fixes geometry/ownership without claiming full shader parity. |
| P3 | Cross-facet and visual regression coverage remains incomplete. | The audited payload is map 0; content-family fixtures contain TBD transition IDs and do not constitute automated image golden tests. |
| P3 | Several roadmap features remain planned rather than implemented. | Clipmaps, complete mobile world rendering, world export, projected light decals, and network/editor workflows remain tracked in docs/TODO.md. Converter/reader or inspector support alone does not establish world-renderer support. |

The historical height-scale concern was not treated as a confirmed bug: the existing documented geometric derivation supports the current scale. Existing accepted Classic water ownership/fog behavior was preserved.

## Validation

- Final full workspace, all-features test run: **656 passed, 0 failed, 4 intentionally ignored slow brute-force tests** across 57 test groups.
- Focused extraction gate: **5 passed**, including symlink-parent and dangling-destination coverage.
- Original CSV parsing, source-UOP-to-UDDP round-trip, subtype preservation, routing precedence, liquid-role isolation, static visibility, and shader composition have regression coverage.
- `cargo check --workspace --all-targets --all-features --locked`: passed after the final code changes.
- `cargo build -p dynamapper --locked`: passed after removal of the temporary capture code.
- Render checks run with isolated settings and packages; every test window is assigned by its exact process ID to the KDE desktop named Desktop 4 before capture. Saved user rendering preferences are not changed.
- EC town and shoreline, plus the existing Classic water path, were inspected under Classic, Enhanced, and KR presets without shader/pipeline errors. The town's large black gaps and shoreline's purple support-map coloring are gone. Remaining EC shoreline defects are listed above.
- The temporary screenshot hook was removed before the final production build. No persistent desktop rule was added.

Tests and type checks do not certify perfect original-client visuals, all liquid/effect families, long-session stability, or the roadmap performance targets.

Local captures are available in `/tmp/uodynamapper-render/`: `before-source-routes.png`, `ec-{classic,enhanced,kr}-town.png`, `ec-{classic,enhanced,kr}-water-validated.png`, and `cc-{classic,enhanced,kr}-water.png`. Test logs are `/tmp/uodynamapper-final-workspace-tests.log`, `/tmp/uodynamapper-final-check.log`, and `/tmp/uodynamapper-final-build.log`.
