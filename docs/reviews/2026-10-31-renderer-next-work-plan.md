# Renderer remaining work and proposed order — 2026-10-31

Draft for the next implementation phases. This reconciles current source,
`agents_comm.txt`, the visual follow-up review and the user's unchanged
`todos dynamapper.txt`. It is a source/document review; no new native rendering
acceptance is claimed here. Performance work remains deferred to the profiling
workstation.

## Phase 1 completion

The artwork fallback, shared source-qualified atlas, picking/inspector agreement
and canonical package integration are implemented. Native/source acceptance also
found and repaired missing Classic atlas coverage through a complete original
Classic UOP rebuild. Six coherent runtime files are installed with verified
backups. See [the acceptance and its limits](2026-11-01-renderer-art-handoff.md).
The next implementation phase is composed-material and visible-depth picking
(phase 2 below). The rest of this document retains the original handoff rationale.

## Updated handoff and scope

The auxiliary-package investigation changes the earlier diagnosis. The old
installed package came from the newer `Enhanced fp` client, while the requested
rebuild used the plain Enhanced client. The 3,072 omitted legacy-family images
are explained by that source difference and have no current Dynamapper auxiliary
consumer. Preserving every historical package entry is therefore not the
clean-build acceptance rule. The plain Enhanced client is now canonical.

A separate real bug marked repeated support references as primary images. Its
fix restores the inputs of 51 static-terrain material owners, including roofs,
carpets, wood floors and a sunken log. Converter fixes and package generation
stay with the asset agent. Renderer integration and native acceptance remain
our work. See [the corrected coverage investigation](../agent_notes/tex_aux_ec_coverage.md).

The newer handoff also changes the runtime contract:

- Tilemeta format 3 assigns texture roles by block shader and item position.
- EC art now packs KR WorldArt only. About 21,305 formerly legacy-sourced slots
  disappear; those objects need the actual Classic-art fallback.
- The selected artwork chain is available KR art, then Classic art, then the
  client's missing-texture image. `UseNewArt` remains informational, with no
  new setting or renderer gate, as decided by the user.
- Absent auxiliary sources alias the original magenta `missing.tga`; the earlier
  policy of leaving missing-mask hues unchanged is superseded.
- Terrain provenance is version 4, with roles determined by layer class and
  source position. Terrain blend direction and 22 materials' selected base change.
- EC rendering wetness follows the authored water shader; raw IsWet remains
  gameplay metadata. Surface routing must agree with that render decision.

Committed handoff changes include `267302eb`, `55647354`, `938dc6e7`,
`2cd734f2`, `633a60ad`, `59e77b3e` and the surface-routing follow-up
`55e29d41`. Further edits were in progress in
`statics_collect.rs`, `static_geometry.rs` and `cursor_behavior.rs` during this
review; coordinate through the communication file and preserve those edits.
Installed packages were left unchanged by the asset session. Earlier accepted
captures describe their recorded revisions, rather than accepting the new
candidate packages automatically.

Role/position migration, shader-based rendering wetness and its surface-routing
follow-up already have source changes. Phase 1 should complete the artwork
fallback and validate the combined behavior, retaining those existing changes.

## Proposed implementation phases

| Order | Work | Current gap | Acceptance |
| --- | --- | --- | --- |
| 1 | Finish the current renderer/asset handoff | KR-only art needs complete Classic and missing-image fallback. Recent wetness, material-role, source-position and terrain-composition changes need native acceptance. | No disappeared legacy objects; draw and picking share the actual chosen source, Classic scale/anchor, hue and opacity. Validate new surface graphs and missing-source aliases. Check all three styles before switching the installed set. |
| 2 | Select the object or terrain actually visible under the pointer | Ordinary sprite alpha works, but multi-image material quads still accept geometric hits without their composed coverage. CPU priority does not fully account for terrain occlusion or final fragment depth. | Click through faded/transparent road and shoreline regions; reject terrain-hidden objects; select the visible winner at bridge, crane and town overlaps in both projections. Preserve original logical identity and XYZ. |
| 3 | Close concrete geometry, sampling and overlap mismatches | Individual anchors, degenerate windows, extent rounding, trim-sensitive shading and authored quads outside the supported isometric view remain unproven. Wider depth ordering has limited fixture coverage. | Establish a failing original-client fixture before changing policy. Check scaled plants, legacy cranes, large art, expanded multis, roofs and equal-priority overlaps. Separate art bounds, texture filtering and optional lighting causes. |
| 4 | Verify and implement animated static artwork | The Classic AnimData reader exists, but no AnimData-driven frame-selection consumer was found in Dynamapper. UV water motion is a different operation. | Find an original-client animated prop/wave with an active source record, then reproduce its frame sequence and cadence. Pause/resume deterministically; update sprite bounds, picking and residency with the frame while keeping logical XYZ fixed. |
| 5 | Refine liquid and material appearance against original references | Reflection interpolation, clocks, stage filtering, normal orientation and land/static material parity retain limits. `follow-center` has no established original-client counterpart. | Use matched shoreline/bridge, lava, swamp and snow controls. Separate source selection, UVs, filtering and lighting; validate motion as well as still frames. Keep accepted bank/crust and transition behavior. |
| 6 | Integrate missing world effects | Foam, waterfalls, lava bubbles and refraction lack world-renderer integration. Effects parsing, packing, simulation and previews already exist in other crates/tools. | Follow actual effect definitions and spawn ownership to one bounded fixture, then reuse that pipeline in the scene. Add any extra rendering pass only when the recovered material requires it. Check effect inspector/glTF export and real-client overlap/hue cases separately. |
| 7 | Finish optional lighting and support-image fidelity | Grunge uses a generated texture; projected/contact shadows, local-light colors and optional KR shading are visibly active but do not establish original-client parity. | Trace original support resources and compare suitable receivers under matched presets. Keep Classic/legacy reference colors and opt-in enhancement settings explicit. Do not increase every strength to compensate for an unsuitable receiver. |
| 8 | Broaden content and camera coverage | Most acceptance is concentrated on map 0 and a few families. Historical route lists, overrides and general perspective/baked-art parity need current evidence. | Add representative facets, rare material families, dense interiors, large multis and camera histories. Reproduce old route failures with the canonical package set before reopening them. |

Phases 2 and 4 are independent of each other once phase 1 is accepted. Phase 3
is driven by concrete failed fixtures; it is not an instruction to rewrite the
entire static depth policy. A global UO-style depth replacement remains a design
proposal until evidence justifies it.

### Phase 1 boundaries and controls

Use one explicit package manifest, built from the canonical plain Enhanced
client and the matching Classic source. Candidate directories currently include
`kr-art-only-20261031` and `terrain-layers-by-class-20261031` under
`/home/claudio/test/uddp-candidates/`. Confirm their current metadata versions and
pairing rather than mixing files by filename. The latest readers reject the old
installed tilemeta/terrain versions; a successful compile is not a usable runtime
baseline with those old files.

Follow the actual selected source through collection, geometry, inspector and
picking. Classic fallback must keep Classic pixel units and south-vertex anchor,
including its wet flat-art rules. Missing-source auxiliary aliases should share
their payload upload by `payload_texture_id()` and retain original requested IDs
for diagnostics. The source-backed magenta fallback is now an integration and
validation task, not an invitation to invent replacement masks.

Revisit the existing fixtures: bridge `(2834,987)`, road `(2518,561)`, town
`(1647,1682)`, coast `(1445,1790)`, lava `(5744,2928)`, swamp `(1872,2416)` and
hued chairs `(6404,2600)`. Add a formerly missing roof/carpet/floor graph and
representative changed grass/rock/cave terrain. The road can legitimately change
when its nearby carpet material becomes complete; do not require an old image
to remain identical when its authored inputs changed.

The supplied EC references use legacy object artwork. Compare them with the
corresponding Classic-art source and EC land. They cannot establish new KR-art
pixel parity. Preserve independent style/source controls, isolated cached presets
and exact-process Desktop-4 placement for native runs.

## Already closed or reviewed

The current plan does not reclassify these historical reports as open bugs:

- Static cobblestone/grass smoothing at `(2518,561)` and independent CC/EC
  transition controls.
- Common sprite height/pixel calibration, bridge volume classification,
  perspective tie bias and history-dependent town/bridge depth ties.
- Ordinary sprite alpha picking and collector-backed large-art/multi candidates.
- Legacy Translucent weights and the isolated water mark, verified as source art.
- Shader-dialog organization, inspector wrapping, free perspective zoom and
  app-only teleport/key/click control.
- Static rendering toggle and visible light masks; the two light switches have
  different consumers.
- Effect controls reported as inactive: suitable native receivers demonstrate
  reachability. Original-client fidelity is still phase 5/7 work.
- Non-KR fake-normal defaults and the old wall-edge report: reviewed fixtures
  did not reproduce it. The legacy TerrainDefinition KDL copies were removed;
  blanket removal of remaining active KDL configuration is not planned.

New handoff changes can require revalidation of these controls. That is separate
from discarding their recorded fixes.

## Later client milestones and deferred work

For the future playable client, separately plan walkable-surface/collision queries,
player-local roof/upper-floor hiding and fading, and world mobiles with animation
and equipment layering. Existing readers, conversion and inspector previews do
not establish that these world/client features are implemented. Preserve logical
XYZ, signed Z, heights, movement flags and multi-part identity throughout every
renderer phase; visual offsets must never become walking coordinates.

Performance/FPS telemetry, fragment/vertex work placement, renderer targets and
streaming/LOD architecture remain in
[the workstation handoff](2026-10-29-renderer-performance-handoff.md).
Clipmaps, broad streaming redesign, export/paperdoll expansion and optional
enhancements remain later roadmap work. Source-code TODOs that still refer to
the removed speculative-role enum are historical implementation ideas; they do
not prove an effect's ownership or prescribe its rendering pass.
