# Visual fidelity follow-up — 2026-10-29

This review supplies evidence and scope for the active items in `docs/TODO.md`.
Performance optimization is deferred to the workstation profiling session in
[the performance handoff](2026-10-29-renderer-performance-handoff.md).
The initial inventory was a source/document review. The 2026-10-30 static-ground
follow-up below has native rendering validation; the other open rows still need
their own runtime acceptance checks.

## Current visual priorities

| Priority | Issue | Evidence and next acceptance check |
| --- | --- | --- |
| Completed 2026-10-30 | Static/map blending at map 0 `(2518,561)` | Runtime probes verified eligibility and alpha composition. Widening the old 0.3-cell fade to 0.82 cell gives a visible soft edge; half-cell limits preserve isolated centers and filled shared edges. Orthographic/perspective and style/source controls are recorded in the ground-transition notes. No geometry or artwork substitution was needed. |
| Completed 2026-10-30 | Independent EC/CC transition controls and CC terrain smoothing | CC dry texmaps use cell-center blending with adjacent-edge sampling; EC controls map blending and the added static-ground fade. Settings/preset tests and paired native captures verify source independence and retained water/bank behavior. App-only input now exercises F2, both checkboxes through all four combinations, saved TOML and dialog reopening. The UI-driven CC frame exactly matches the accepted configured frame; both controls are visible at 1280x800. |
| Completed 2026-10-30 | Common static height and pixel calibration | Two independent matched legacy sprites confirmed the old 1.096 vertical stretch. The exact legacy grid now projects square sprite pixels and four pixels per raw Z. CPU/GPU units, inverse Z conversion, picking and light placement agree; logical XYZ and movement metadata remain unchanged. Native three-style/source/camera controls and reference fits are recorded in the height-calibration note. |
| Completed 2026-10-30 | Perspective bridge stair/deck overlap | A fixed NDC tie bias grew too large at perspective distances and reordered neighboring sprites. An isolated zero-bias control removes the blocks; a small projected view-space tie displacement retains separation without changing geometry or logical XYZ. Production GPU, native three-style/source, orthographic and closer/farther perspective controls are recorded in the depth notes. |
| Completed 2026-10-30 | Static hover quad construction and route labels | Picking shares effective source routing, CC/EC anchors, slot bounds and resident flat-surface admission with collection. It follows horizontal and authored camera-quad transforms and tests projected mesh triangles. The old inspector's guessed EC land route is replaced with the actual resolved source route. Native before/after controls and scope limits are recorded in the hover notes. |
| Completed 2026-10-30 | History-sensitive transparent static edges | Dummy batch mesh centers made Bevy's equal-distance ordering depend on retained insertion history. A stable ground/upright and chunk tie key preserves stock distance/material bias and fixes the KR town's 234 differing tree-edge pixels. Three-style and perspective native histories, bridge and enabled-shadow controls are recorded in the depth notes. A separate pre-existing 50-pixel KR orthographic bridge-edge difference remains isolated. |
| Open | Individual anchors/new-art scales and exact visible picking | Common calibration does not prove every CC/EC window or EC new-art source scale. Hover quad construction is repaired; texture-alpha, full visible-depth selection, very large art and expanded-multi coverage remain unproven. Compare actual descriptor/window branches and individual matched artwork. Arbitrary-angle baked-art fidelity and general original-client depth parity remain unproven. |
| Reviewed 2026-10-30 | Non-KR fake-normal wall report | Factory/bundled/saved Classic and Enhanced presets disable it in every period. Six explicit off/on pairs on stone/timber walls show the control working without reproducing the old rectangular edge frame. Existing upright edge profiles are zero. No further shader change is justified by these fixtures; optional shading parity and arbitrary configurations are not claimed. |
| Reviewed 2026-10-30 | Remaining effects reported as invisible | Three-style native controls show grunge and projected shadows working on suitable receivers; KR boundary grime and color wash also change actual pixels. Normal maps require authored inputs: common dry materials have none, liquid bump composition is separate, and snow's default lighting response is weak. Seventeen ordinary fourth-image bindings were incorrectly labeled normal maps and are repaired. See the effect-visibility note for gates, preset precedence, receiver controls and remaining parity work. Static masks/local-light intensity have their separate completed checks. |
| Completed 2026-10-30 | Free perspective zoom | Input now applies the actual clamped zoom delta as a dolly along the free camera's current view direction. Native scroll/text checks cover all three styles, rotated and unrotated views, both limits, and follow/orthographic controls. Object/terrain positions and the lens remain unchanged; see the perspective notes. |
| Completed 2026-10-30 | Shader-dialog organization | Standard/KR-style art columns stack at narrower widths. Water has its own shared terrain/static group; advanced Effects/Graphics separate scopes. Native three-style, resize, save and side-window checks pass; see the shader-control layout notes. Effect fidelity remains separate. |
| Later parity work | Liquid reflection interpolation, clocks, per-stage filtering, projected geometry and color/lighting | Default original reflection-camera coordinates, source selection, signed UVs, owner-specific streams and liquid filtering have repairs. Full pixel parity across different triangulations, item anchors and original per-stage sampler choices remains unproven. Do not reopen the repaired camera formula without new evidence. |
| Unimplemented | Foam, waterfall, lava-bubble and refraction effects | `land/main.wesl` still marks these extra passes TODO. Recover actual effect-definition/spawn and NIF/EMS ownership before assigning support images to materials. |
| Revalidation | Classic partial-water transparency and a stray authored wave | Recorded as separate follow-ups in the water-resolution notes. Reproduce first; the accepted terrain/static lighting and fog ownership are regression controls. |
| Broader coverage | Manual route overrides and additional facets/content families | Source-backed map-0 route coverage was repaired. Remaining explicit source-versus-policy conflicts need content review; they are not proof of missing routes. Extend family/facet reference coverage after a concrete defect is selected. |

Primary references:
[static ground transitions](../agent_notes/ec_static_ground_transitions.md),
[source-specific transition controls](../agent_notes/terrain_transition_controls.md),
[shader-control layout](../agent_notes/shader_control_layout.md),
[static light visibility](../agent_notes/static_light_visibility.md),
[effect visibility and source roles](../agent_notes/effect_visibility_validation.md),
[non-KR fake-normal validation](../agent_notes/art_fake_normal_validation.md),
[static height calibration](../agent_notes/static_height_calibration.md),
[static depth and perspective ties](../agent_notes/static_depth_ordering.md),
[static hover geometry](../agent_notes/static_hover_geometry.md),
[liquid source evidence](../agent_notes/ec_original_water_code.md),
[water resolution](../agent_notes/water_terrain_static_resolution.md), and
[material reachability](../agent_notes/ec_material_family_reachability.md).

The bridge/stair volume classification, perspective roof pitch, source hue-mask
composition and lava-to-bank/crust fixes have recorded corrections. The perspective
stair/deck tie-bias repair above is distinct from the earlier orthographic volume
classification. Keep them as regression fixtures. Historical repository-health entries that predate these
fixes are not sufficient to classify them as currently broken.

## Reconciliation with the user's unstaged text file

Read `todos dynamapper.txt` after collecting the rendering issues above. Preserve
that original file unchanged and untracked. Its own warning says findings may
be stale. The following classification is based on current source and notes.

| Text-file item | Current classification |
| --- | --- |
| Part 1.1: less crowded art controls / separate standard and KR columns | Completed with responsive Standard art / KR-style art columns and a stacked narrow layout. Native checks cover all three styles and actual resize drags. |
| Part 1.2: categorize effect toggles | Completed: Basic Controls separates Terrain, Objects, Water and Everything; advanced Effects/Graphics label their scopes. Water animation explicitly covers map and static water. Moved toggles save through the existing preset path. |
| Part 1.3: effects have no visible impact | Source/profile/receiver audit completed for grunge, KR boundary grime, color wash and projected shadows. Common dry materials lack normals; snow supplies a bump input but morning lighting strongly limits its visible effect. Unsupported normal interpretation of seventeen fourth-layer art images is repaired. Original-client parity and support-texture provenance remain separate. |
| Part 1.4: cursor inspection strings are too long | A recorded repair splits fields across lines; current `cursor_behavior.rs` retains that formatting. Recheck only if a specific long value still overflows. |
| Part 2.1: statics seem too high | Common projection/height mismatch repaired with matched legacy-sprite evidence, exact camera tests and signed-Z preservation. Perspective deck overlap is separately repaired by correcting the tie bias. Individual new-art scales/anchors remain open; see the height-calibration and depth notes. |
| Part 2.2: independent transition toggles and CC support | Completed with separate Graphics checkboxes, persistence and shader paths. Production GPU cases cover shared boundaries, source/wet/missing exclusions and neighbor UVs; native paired captures cover rendering and inactive-source controls. The app-only input follow-up validates direct checkbox interaction, all four saved combinations, reopening and an exact match to the accepted CC-enabled frame. |
| Part 3.1a: Render Static Items does not hide existing items | Recorded repair is present: disabling statics clears instance/batch outputs and invalidates reuse, with a regression in `statics_collect.rs`. |
| Part 3.1b: static light decals are invisible | Missing mask-package cause repaired in the configured data set. Original-client masks, actual collector counts and paired Classic/Enhanced/KR night pixels verify visible mask composition. Direct Options interaction changes collector counts 115 to 0 and back, with a visible off/on change. Dedicated projected decals and original-client lighting parity remain separate follow-ups. |
| Part 3.2: framerate measurement accuracy | Deferred with performance work. Existing FPS is explicitly smoothed; compare raw CPU/GPU/presentation data on the workstation without disabling pipelining. |
| Part 3.3: fake normals produce non-KR wall-edge artifacts | Off defaults verified in factory/bundled/saved presets. Six native explicit-activation pairs on stone/timber walls do not reproduce the old rectangular frame; the existing shader structurally suppresses that cue. Keep the effect opt-in; further concrete artifacts need matched reproduction. |
| Part 3.4: two light settings might be duplicates | They have different consumers. `enable_static_lights` enables light collection/decals; `enable_static_art_local_lights` additionally applies collected illumination to art. They are not stale duplicates. |
| Part 3.5: KDL files might be obsolete | Active KDL routing/override loaders remain in `uo_files_loader.rs`; other KDLs provide material policy and runtime rules. Classify individual files, including the legacy subfolder, before removing any. No blanket cleanup is justified. |
| Extra: perspective zoom changes only the overlay | Repaired for free perspective with native input validation. Follow perspective and orthographic remain separate tested controls; isometric pitch was repaired earlier. |

All new visual checks should use the supplied original-client images and isolated
settings. The KDE preview remains on Desktop 4 with exact process ownership.
Retain current baselines and reusable tools; retire superseded temporary images,
fixtures and logs at completed task boundaries.
