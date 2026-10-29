# Renderer performance investigation — deferred

Renderer performance optimization is deferred at the user's request. Resume on
the desktop workstation with RenderDoc and rocprofv3 available. Visual fidelity
is the current priority. Keep the accepted changes; do not start another local
optimization or benchmark campaign from these hypotheses alone.

## Established evidence

- Accepted implementation: `06d3e288`, exact terrain transition-grime masks and
  the one-time cache-image resize rebind. No extra GPU texture allocation.
- Accepted executable SHA256:
  `44f02d3398d16a88b2d84404b506776346739acf1245d50c3c57ad841f287256`.
- On the previous AMD workstation configuration, the shader comparison increased
  KR orthographic sampled FPS about 15–19%. The 400 FPS orthographic and 144 FPS
  perspective targets remain open, including raw minima and movement.
- Earlier KR diagnostics bypassing fragment bicubic **and** bent reconstruction
  reduced opaque GPU time about 17–24%. Bent normals alone were not isolated.
  Those diagnostics predate the grime cache and cannot be added to its gain.
- Classic/Gouraud presets are substantially cheaper than KR, but they also change
  lighting and postprocessing. They do not isolate normal reconstruction.
- A cache-only diagnostic greatly reduced static shader code/spill statistics
  while changing opaque GPU time by less than 1%. Static instruction counts do
  not identify the dominant executed work.
- Shader A/B runs used identical new CPU preparation. A complete before/after
  executable comparison of the Rust changes was not run. Classic's absolute FPS
  also varied independently of opaque GPU timing; its cause is unestablished.

Detailed measurements are in
[renderer_release_performance.md](../agent_notes/renderer_release_performance.md)
and [ec_terrain_shader_footprint.md](../agent_notes/ec_terrain_shader_footprint.md).
The source audit is
[terrain_shader_work_placement.md](../agent_notes/terrain_shader_work_placement.md).
Temporary structured indexes are `grime-final-performance-index.json` and
`grime-accepted-native-index.json` in `/tmp/uodynamapper-ec-followup/`; portability
must not depend on that temporary directory surviving the workstation move.

## Hypotheses and discriminating tests

| Hypothesis | Test before choosing an implementation | Possible follow-up if supported |
| --- | --- | --- |
| Fragment bent normals are expensive | Bypass only fragment bent evaluation, retaining vertex work, bicubic reconstruction and all other KR settings. Compare executed work and GPU duration. | Precompute cell-owned occlusion or move terrain-scale evaluation to vertices. |
| Bicubic reconstruction is a larger cost | Bypass only fragment bicubic evaluation; retain bent evaluation. Then test the combined bypass. Do not sum individual timing differences. | Denser vertex normal evaluation, cached height neighborhoods or analytic coefficients. |
| Vertex and fragment normal work duplicates useful results | Inspect actual compiled stages and consumers, then compare a selected representation with unchanged geometry. | A finer normal-evaluation grid must preserve original piecewise planar terrain and bank positions; current displacement rounds cell coordinates. |
| Height/relief and bank queries repeat expensive reads | Attribute the executed normal, contact, cliff and liquid-bank paths and their shared reads. Source call counts alone are insufficient. | Cache cell min/max/occlusion or bank inputs with explicit residency and invalidation. Preserve material-specific normals and bank center heights. |
| Opaque ground statics hide costly terrain shading | Inspect draw order, pixel history, depth writes, early/late rejection and terrain-only fragment work. The combined opaque fragment count cannot quantify this. | Reorder eligible draws, conservative coverage or a measured depth prepass. Fractional masks, raised art and shoreline holes must remain visible. |
| Repeated layer shading dominates | Establish executed material counts and the cost of normal maps, filtering, weathering and lighting per layer. | Share material-independent work; consider a different composition architecture only if it preserves source ordering and material response. |
| Register pressure, texture traffic or pipeline latency limits throughput | Collect supported counters for the actual Vulkan workload and correlate them with RenderDoc events and GPU timings. Confirm tool coverage first. | Choose the change matching the measured limit; do not split pipelines solely to shrink static shader code. |
| Classic is limited outside terrain fragment shading | Compare CPU update/extraction/submission, presentation and GPU durations. If needed, build matched full before/after executables from the same source revision. | Address the measured CPU or presentation limit; keep pipelined rendering enabled. |

## Efficient restart procedure

1. Record the new GPU/driver, build revision, effective cached preset, packages,
   camera, viewport, zoom and frame-limit state. Rebaseline on that machine.
2. Capture representative bridge `(2834,987)` and lava `(5744,2928)` frames on
   map 0; use town `(1647,1682)` only when another material mix is needed.
3. Inspect the frame and rank actual costs before running the smallest ablation
   that can distinguish the leading hypotheses. Keep full-preset comparisons
   separate from single-effect tests; Classic with KR shadow weathering can still
   consume a fragment normal.
4. Select one implementation with a credible frame-budget benefit. Preserve
   authored material normals, coverage, source ownership and missing-texture
   policy. Compare against original-client references where lighting changes.
5. Use a small visual pilot first. Expand regression coverage after the candidate
   shows a useful measured gain. Avoid repeated full screenshot matrices for
   rejected ideas. Distinguish raw frame times, smoothed FPS and timed GPU spans.

The current liquid-only-normal, shared-weathering and cold-fallback-removal
experiments were not accepted optimizations. Their limitations and rejected
results remain in the measurement notes; do not restart them without new evidence.
