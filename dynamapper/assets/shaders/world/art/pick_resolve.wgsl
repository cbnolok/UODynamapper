// Integer IDs are read before any MSAA color averaging. Prefer the visible ID
// covering most samples; ties retain the first sample's deterministic order.
@group(0) @binding(0) var ids: texture_multisampled_2d<f32>;
@group(0) @binding(1) var<storage, read_write> result: u32;
fn read_id(sample_index: u32) -> u32 {
    let bytes = vec4<u32>(round(textureLoad(ids, vec2<i32>(0), i32(sample_index)) * 255.0));
    return bytes.x | (bytes.y << 8u) | (bytes.z << 16u) | (bytes.w << 24u);
}
@compute @workgroup_size(1)
fn resolve() {
    var winner = 0u;
    var best = 0u;
    for (var sample_index = 0u; sample_index < textureNumSamples(ids); sample_index += 1u) {
        let id = read_id(sample_index);
        if (id == 0u) { continue; }
        var count = 0u;
        for (var other = 0u; other < textureNumSamples(ids); other += 1u) {
            count += select(0u, 1u, read_id(other) == id);
        }
        if (count > best) { winner = id; best = count; }
    }
    result = winner;
}
