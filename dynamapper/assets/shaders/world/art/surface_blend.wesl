// Source-eligible, ground-level static surfaces blend into the map only at the
// patch boundary. Neighbor bits: NW,N,NE,W,E,SW,S,SE, independent of graphic IDs.
fn static_surface_coverage(uv: vec2<f32>, neighbors: u32) -> f32 {
    if (neighbors == 255u) { return 1.0; }
    let low = smoothstep(vec2<f32>(0.0), vec2<f32>(0.3), uv);
    let high = smoothstep(vec2<f32>(0.0), vec2<f32>(0.3), vec2<f32>(1.0) - uv);
    var alpha = select(low.x, 1.0, (neighbors & 8u) != 0u)
        * select(high.x, 1.0, (neighbors & 16u) != 0u)
        * select(low.y, 1.0, (neighbors & 2u) != 0u)
        * select(high.y, 1.0, (neighbors & 64u) != 0u);
    // A missing diagonal rounds a concave corner even when its two cardinal
    // neighbors are present. Filled neighborhoods never acquire a tile grid.
    if ((neighbors & 1u) == 0u) { alpha *= 1.0 - (1.0 - low.x) * (1.0 - low.y); }
    if ((neighbors & 4u) == 0u) { alpha *= 1.0 - (1.0 - high.x) * (1.0 - low.y); }
    if ((neighbors & 32u) == 0u) { alpha *= 1.0 - (1.0 - low.x) * (1.0 - high.y); }
    if ((neighbors & 128u) == 0u) { alpha *= 1.0 - (1.0 - high.x) * (1.0 - high.y); }
    return alpha;
}
