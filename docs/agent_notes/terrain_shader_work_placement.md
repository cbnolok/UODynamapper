# Terrain shader work placement

The close-view terrain mesh has one vertex per tile: the high-detail 8×8 mesh
has 81 vertices. Its spacing grows with LOD/zoom in `land/draw_mesh.rs`.
The vertex entry in `land/main.wesl` performs bank displacement, original liquid
reflection-coordinate generation, geometric normals, optional bicubic normals,
optional bent normals and Classic's Lambert term.

Enhanced/KR still reconstruct bicubic and bent normals in `land_surface_normal`.
The geometric result is shared across the material stack, but the fragment
normal computation has not been moved entirely to vertices. Bicubic uses a
4×4 height neighborhood; bent normals read the center and four cardinal heights.
These are source-query counts, not demonstrated hardware transactions: compiler
sharing and register allocation can change their actual cost.

`normals.wesl` evaluates analytic Catmull–Rom derivatives at the fragment's
fractional world position. Interpolating normalized vertex normals on the
current tile grid does not reproduce that field. Bent-normal strength uses
`floor(world_position.xz)`, so it has cell ownership semantics as well. Reusing
the current vertex normal is therefore a deliberate lighting change. The
vertex-normal attribution diagnostic in `ec_terrain_shader_footprint.md`
establishes a useful performance opportunity, not visual equivalence.

## Where work can move

- Terrain-scale normal/occlusion evaluation can move to a denser vertex grid
  if reference comparisons accept the interpolation. Extra vertices must retain
  the original piecewise planar geometry and bank boundaries. The current
  vertex displacement rounds coordinates to integer cells; subdividing its
  input grid without changing displacement would create stepped geometry.
- An alternative is a bounded cached height/normal field. Cached Catmull–Rom
  coefficients retain analytic evaluation; sampled normals introduce a filtering
  approximation. Account for both memory traffic and invalidation, rather than
  treating either as a free texture read.
- Bent-normal occlusion and contact/cliff relief share cardinal height statistics.
  The latter two each read four heights inside `shade_land`. Their min/max and
  neighbor decisions are cell data suitable for precomputation. Keep current
  cell ownership and the passed material height: an extra bank material can
  have a different center height. Cliff response also depends on the final
  material normal and light, which remain dynamic.
- Authored texture normals, liquid normal/reflection sampling, filtered albedo,
  coverage and view-dependent highlights retain pixel detail. Diffuse/fill work
  can move only where its normal and material dependencies permit it. A textured
  normal can replace the terrain normal separately for each layer.
- Whole-frame or uniform-only expressions belong on the CPU or outside material
  loops. Moving a cheap affine expression into another varying may increase
  interpolation traffic; helper-call counts alone do not establish a gain.

The adaptive cheap-albedo branch in `shade_land` executes after
`land_surface_normal` at the fragment entry. It does not itself avoid that normal
work. Explicit zoom guards inside `land_surface_normal` do disable bicubic/bent
work at their existing thresholds; Classic has a separate unused-normal guard.

## Larger pixel-work opportunity

Expensive map shading beneath opaque ground statics is another potential large
cost. The opaque-pass fragment count includes terrain and statics, so it cannot
by itself quantify hidden terrain fragments. Inspect depth/order and measure
terrain invocations before implementing an occlusion or depth-prepass policy.
Only proven opaque coverage may occlude: shoreline masks, transparent crust,
overlapping camera quads and raised statics cannot use a blanket tile-level cull.
This needs separate attribution from normal reconstruction. See
`renderer_release_performance.md` for the current local measurements and limits.
