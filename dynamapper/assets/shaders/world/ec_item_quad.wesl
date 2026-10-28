// UOSA 0x45a080's TileArt bit-34 quad, in original screen right/up units.
fn ec_item_quad_screen(corner: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(0.707105 + (corner.x - corner.y) * 0.7671049,
        1.4742099 - (corner.x + corner.y) * 0.7671049);
}

fn ec_item_quad_world(origin: vec3<f32>, corner: vec2<f32>, right: vec3<f32>,
                      up: vec3<f32>, pixel_aspect: f32) -> vec3<f32> {
    let screen = ec_item_quad_screen(corner);
    return origin + vec3<f32>(0.5, 0.0, 1.5) + right * screen.x + up * (screen.y * pixel_aspect);
}
