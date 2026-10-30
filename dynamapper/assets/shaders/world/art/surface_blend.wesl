// Source-eligible, ground-level static surfaces blend into the map only at the
// patch boundary. Neighbor bits: NW,N,NE,W,E,SW,S,SE, independent of graphic IDs.
fn static_surface_coverage(uv: vec2<f32>, neighbors: u32) -> f32 {
    if (neighbors == 255u) { return 1.0; }
    // Match the terrain transition's 0.82-cell reach on exposed patch edges.
    // A one-cell-wide path has two exposed sides: finish each fade at its
    // center instead of multiplying two incomplete ramps into translucent art.
    let width = vec2<f32>(select(0.82, 0.5, (neighbors & 24u) == 0u),
        select(0.82, 0.5, (neighbors & 66u) == 0u));
    let low = smoothstep(vec2<f32>(0.0), width, uv);
    let high = smoothstep(vec2<f32>(0.0), width, vec2<f32>(1.0) - uv);
    var alpha = select(low.x, 1.0, (neighbors & 8u) != 0u)
        * select(high.x, 1.0, (neighbors & 16u) != 0u)
        * select(low.y, 1.0, (neighbors & 2u) != 0u)
        * select(high.y, 1.0, (neighbors & 64u) != 0u);
    // A missing diagonal rounds a concave corner even when its two cardinal
    // neighbors are present. Filled neighborhoods never acquire a tile grid.
    // Keep corner falloff within the half-cell so a missing diagonal cannot
    // fade the owning cell's center or reveal a seam between filled neighbors.
    let corner_low = smoothstep(vec2<f32>(0.0), vec2<f32>(0.5), uv);
    let corner_high = smoothstep(vec2<f32>(0.0), vec2<f32>(0.5), vec2<f32>(1.0) - uv);
    if ((neighbors & 1u) == 0u) { alpha *= 1.0 - (1.0 - corner_low.x) * (1.0 - corner_low.y); }
    if ((neighbors & 4u) == 0u) { alpha *= 1.0 - (1.0 - corner_high.x) * (1.0 - corner_low.y); }
    if ((neighbors & 32u) == 0u) { alpha *= 1.0 - (1.0 - corner_low.x) * (1.0 - corner_high.y); }
    if ((neighbors & 128u) == 0u) { alpha *= 1.0 - (1.0 - corner_high.x) * (1.0 - corner_high.y); }
    return alpha;
}
