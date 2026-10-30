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
| Next | Static height/anchors and non-KR fake-normal wall artifacts | General height parity remains unproven. The specific bridge/stair depth bug is repaired, so avoid a global height/bias change without a matched reference. Fake normals default off in Classic/Enhanced, but remain manually usable; wall-side artifacts need a bounded visual fixture. |
| Next | Effects and light decals reported as invisible | Check each effect with the correct source, active preset, relevant strength and receiver. Normal maps only consume EC land in fragment modes; grime and color wash have KR gates. Trace decal collection, receiver and visible composition rather than declaring the switches broken from default output. |
| Completed 2026-10-30 | Free perspective zoom | Input now applies the actual clamped zoom delta as a dolly along the free camera's current view direction. Native scroll/text checks cover all three styles, rotated and unrotated views, both limits, and follow/orthographic controls. Object/terrain positions and the lens remain unchanged; see the perspective notes. |
| Open UI work | Shader-dialog organization | Scope groups exist, but standard/KR art separation, water-animation placement and the mixed-scope advanced Effects section remain unfinished. Review the effective layout as well as source grouping. |
| Later parity work | Liquid reflection interpolation, clocks, per-stage filtering, projected geometry and color/lighting | Default original reflection-camera coordinates, source selection, signed UVs, owner-specific streams and liquid filtering have repairs. Full pixel parity across different triangulations, item anchors and original per-stage sampler choices remains unproven. Do not reopen the repaired camera formula without new evidence. |
| Unimplemented | Foam, waterfall, lava-bubble and refraction effects | `land/main.wesl` still marks these extra passes TODO. Recover actual effect-definition/spawn and NIF/EMS ownership before assigning support images to materials. |
| Revalidation | Classic partial-water transparency and a stray authored wave | Recorded as separate follow-ups in the water-resolution notes. Reproduce first; the accepted terrain/static lighting and fog ownership are regression controls. |
| Broader coverage | Manual route overrides and additional facets/content families | Source-backed map-0 route coverage was repaired. Remaining explicit source-versus-policy conflicts need content review; they are not proof of missing routes. Extend family/facet reference coverage after a concrete defect is selected. |

Primary references:
[static ground transitions](../agent_notes/ec_static_ground_transitions.md),
[source-specific transition controls](../agent_notes/terrain_transition_controls.md),
[liquid source evidence](../agent_notes/ec_original_water_code.md),
[water resolution](../agent_notes/water_terrain_static_resolution.md), and
[material reachability](../agent_notes/ec_material_family_reachability.md).

The bridge/stair volume classification, perspective roof pitch, source hue-mask
composition and lava-to-bank/crust fixes have recorded corrections. Keep them as
regression fixtures. Historical repository-health entries that predate these
fixes are not sufficient to classify them as currently broken.

## Reconciliation with the user's unstaged text file

Read `todos dynamapper.txt` after collecting the rendering issues above. Preserve
that original file unchanged and untracked. Its own warning says findings may
be stale. The following classification is based on current source and notes.

| Text-file item | Current classification |
| --- | --- |
| Part 1.1: less crowded art controls / separate standard and KR columns | Partially addressed by collapsible Terrain, Objects and Everything groups. The requested standard/KR art columns are not present; usability still needs review. |
| Part 1.2: categorize effect toggles | Basic Controls now groups them by scope. Water Animation remains in the whole-scene group and advanced Effects mixes scopes, so the requested organization is not fully settled. |
| Part 1.3: effects have no visible impact | Keep as a targeted visual audit, not a blanket confirmed defect. Source/profile gates and effective cached settings must be recorded. Projected shadows have a prior visibility repair. |
| Part 1.4: cursor inspection strings are too long | A recorded repair splits fields across lines; current `cursor_behavior.rs` retains that formatting. Recheck only if a specific long value still overflows. |
| Part 2.1: statics seem too high | Open reference-validation item. Prior CC double-offset and bridge-priority repairs do not prove every EC/CC anchor or height is correct. |
| Part 2.2: independent transition toggles and CC support | Completed with separate Graphics checkboxes, persistence and shader paths. Production GPU cases cover shared boundaries, source/wet/missing exclusions and neighbor UVs; native paired captures cover rendering and inactive-source controls. The app-only input follow-up validates direct checkbox interaction, all four saved combinations, reopening and an exact match to the accepted CC-enabled frame. |
| Part 3.1a: Render Static Items does not hide existing items | Recorded repair is present: disabling statics clears instance/batch outputs and invalidates reuse, with a regression in `statics_collect.rs`. |
| Part 3.1b: static light decals are invisible | Open visual verification; confirm a loaded light source, collector output, receiver and intensity. |
| Part 3.2: framerate measurement accuracy | Deferred with performance work. Existing FPS is explicitly smoothed; compare raw CPU/GPU/presentation data on the workstation without disabling pipelining. |
| Part 3.3: fake normals produce non-KR wall-edge artifacts | Defaults are off for Classic/Enhanced in factory, bundled and currently saved presets. There is no universal style prohibition; manual activation and remaining artifacts need review. |
| Part 3.4: two light settings might be duplicates | They have different consumers. `enable_static_lights` enables light collection/decals; `enable_static_art_local_lights` additionally applies collected illumination to art. They are not stale duplicates. |
| Part 3.5: KDL files might be obsolete | Active KDL routing/override loaders remain in `uo_files_loader.rs`; other KDLs provide material policy and runtime rules. Classify individual files, including the legacy subfolder, before removing any. No blanket cleanup is justified. |
| Extra: perspective zoom changes only the overlay | Repaired for free perspective with native input validation. Follow perspective and orthographic remain separate tested controls; isometric pitch was repaired earlier. |

All new visual checks should use the supplied original-client images and isolated
settings. The KDE preview remains on Desktop 4 with exact process ownership.
Retain current baselines and reusable tools; retire superseded temporary images,
fixtures and logs at completed task boundaries.
