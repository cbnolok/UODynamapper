// BC1's one bit of alpha for the mobiles' frames (render/mobiles/block_encode.rs), one thread per 4x4 block, after
// block_compression's BC1 kernel has written the frame's blocks: that kernel encodes colour only, in the four-colour
// mode. A block with a transparent texel (alpha under one half) is encoded again here in the three-colour mode, whose
// fourth index decodes to transparent black; an empty block becomes transparent. An opaque block keeps the kernel's
// encoding unless its endpoints came out equal, which selects the three-colour mode too and would make index 3
// transparent: its texels then all take index 0, the one colour of the block.

struct Frame {
    // The frame's padded size, multiples of 4.
    width: u32,
    height: u32,
    // Where its blocks start in `blocks`, in u32.
    blocks_offset: u32,
    padding: u32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> blocks: array<u32>;
@group(0) @binding(2) var<uniform> frame: Frame;

const BLOCK_SIDE: u32 = 4u;
const TEXELS: u32 = 16u;
const ALL_OPAQUE: u32 = 0xffffu;
// Opaque from this alpha up: Classic art and its in-betweens are 0 or 1.
const OPAQUE_FROM: f32 = 0.5;
const CHANNEL_MAX: f32 = 255.0;
const TRANSPARENT_INDEX: u32 = 3u;
const TRANSPARENT_INDICES: u32 = 0xffffffffu;
const INDEX_BITS: u32 = 2u;
const ENDPOINT_BITS: u32 = 16u;
const ENDPOINT_MASK: u32 = 0xffffu;
// Power iterations for the principal axis of the opaque texels' colours, as the ISPC kernel does.
const POWER_ITERATIONS: u32 = 4u;
const DEGENERATE: f32 = 1e-6;
// RGB565.
const RED_SHIFT: u32 = 11u;
const GREEN_SHIFT: u32 = 5u;
const FIVE_BITS: f32 = 31.0;
const SIX_BITS: f32 = 63.0;

fn quantize565(color: vec3<f32>) -> u32 {
    let c = clamp(color, vec3<f32>(0.0), vec3<f32>(CHANNEL_MAX)) / CHANNEL_MAX;
    let r = u32(round(c.r * FIVE_BITS));
    let g = u32(round(c.g * SIX_BITS));
    let b = u32(round(c.b * FIVE_BITS));
    return (r << RED_SHIFT) | (g << GREEN_SHIFT) | b;
}

// The 8-bit colour a decoder expands an RGB565 endpoint to (bit replication).
fn expand565(packed: u32) -> vec3<f32> {
    let r = (packed >> RED_SHIFT) & 0x1fu;
    let g = (packed >> GREEN_SHIFT) & 0x3fu;
    let b = packed & 0x1fu;
    return vec3<f32>(f32((r << 3u) | (r >> 2u)), f32((g << 2u) | (g >> 4u)), f32((b << 3u) | (b >> 2u)));
}

fn distance_squared(a: vec3<f32>, b: vec3<f32>) -> f32 {
    let d = a - b;
    return dot(d, d);
}

// A three-colour encoding: the endpoint words, the indices, and the squared error over the opaque texels.
struct Encoding {
    endpoints: u32,
    indices: u32,
    error: f32,
}

fn encode_three_colour(a: vec3<f32>, b: vec3<f32>, colors: ptr<function, array<vec3<f32>, 16>>, opaque: u32)
    -> Encoding {
    var q0 = quantize565(a);
    var q1 = quantize565(b);
    // color0 <= color1 selects the three-colour mode.
    if (q0 > q1) {
        let swap = q0;
        q0 = q1;
        q1 = swap;
    }
    let p0 = expand565(q0);
    let p1 = expand565(q1);
    let p2 = (p0 + p1) * 0.5;
    var result = Encoding(q0 | (q1 << ENDPOINT_BITS), 0u, 0.0);
    for (var k = 0u; k < TEXELS; k++) {
        var index = TRANSPARENT_INDEX;
        if ((opaque & (1u << k)) != 0u) {
            let color = (*colors)[k];
            let d0 = distance_squared(color, p0);
            let d1 = distance_squared(color, p1);
            let d2 = distance_squared(color, p2);
            index = 0u;
            var best = d0;
            if (d1 < best) {
                index = 1u;
                best = d1;
            }
            if (d2 < best) {
                index = 2u;
                best = d2;
            }
            result.error += best;
        }
        result.indices |= index << (k * INDEX_BITS);
    }
    return result;
}

@compute @workgroup_size(8, 8)
fn punch_through(@builtin(global_invocation_id) id: vec3<u32>) {
    let blocks_wide = frame.width / BLOCK_SIDE;
    if (id.x >= blocks_wide || id.y >= frame.height / BLOCK_SIDE) {
        return;
    }
    var colors: array<vec3<f32>, 16>;
    var opaque = 0u;
    var count = 0u;
    var mean = vec3<f32>(0.0);
    for (var k = 0u; k < TEXELS; k++) {
        let texel = textureLoad(source, vec2<u32>(id.x * BLOCK_SIDE + k % BLOCK_SIDE, id.y * BLOCK_SIDE + k / BLOCK_SIDE), 0);
        colors[k] = texel.rgb * CHANNEL_MAX;
        if (texel.a >= OPAQUE_FROM) {
            opaque |= 1u << k;
            count += 1u;
            mean += colors[k];
        }
    }
    let at = frame.blocks_offset + (id.y * blocks_wide + id.x) * 2u;
    if (opaque == ALL_OPAQUE) {
        let endpoints = blocks[at];
        if ((endpoints & ENDPOINT_MASK) <= (endpoints >> ENDPOINT_BITS)) {
            blocks[at + 1u] = 0u;
        }
        return;
    }
    if (count == 0u) {
        blocks[at] = 0u;
        blocks[at + 1u] = TRANSPARENT_INDICES;
        return;
    }

    // The endpoints at the ends of the opaque colours along their principal axis.
    mean /= f32(count);
    var xx = 0.0; var xy = 0.0; var xz = 0.0; var yy = 0.0; var yz = 0.0; var zz = 0.0;
    for (var k = 0u; k < TEXELS; k++) {
        if ((opaque & (1u << k)) != 0u) {
            let d = colors[k] - mean;
            xx += d.x * d.x; xy += d.x * d.y; xz += d.x * d.z;
            yy += d.y * d.y; yz += d.y * d.z; zz += d.z * d.z;
        }
    }
    var axis = vec3<f32>(1.0);
    for (var i = 0u; i < POWER_ITERATIONS; i++) {
        let next = vec3<f32>(xx * axis.x + xy * axis.y + xz * axis.z, xy * axis.x + yy * axis.y + yz * axis.z,
            xz * axis.x + yz * axis.y + zz * axis.z);
        let length_squared = dot(next, next);
        if (length_squared < DEGENERATE) {
            break;
        }
        axis = next * inverseSqrt(length_squared);
    }
    axis = normalize(axis);
    var low = 0.0;
    var high = 0.0;
    for (var k = 0u; k < TEXELS; k++) {
        if ((opaque & (1u << k)) != 0u) {
            let t = dot(colors[k] - mean, axis);
            low = min(low, t);
            high = max(high, t);
        }
    }
    var best = encode_three_colour(mean + axis * low, mean + axis * high, &colors, opaque);

    // One least-squares refinement of the endpoints for the indices found: index 0 weighs (1, 0), 1 (0, 1), 2 (1/2, 1/2).
    var a = 0.0; var b = 0.0; var c = 0.0;
    var first = vec3<f32>(0.0);
    var second = vec3<f32>(0.0);
    for (var k = 0u; k < TEXELS; k++) {
        let index = (best.indices >> (k * INDEX_BITS)) & 3u;
        if (index != TRANSPARENT_INDEX) {
            let w1 = select(select(0.5, 1.0, index == 1u), 0.0, index == 0u);
            let w0 = 1.0 - w1;
            a += w0 * w0; b += w0 * w1; c += w1 * w1;
            first += w0 * colors[k];
            second += w1 * colors[k];
        }
    }
    let determinant = a * c - b * b;
    if (abs(determinant) > DEGENERATE) {
        let e0 = (c * first - b * second) / determinant;
        let e1 = (a * second - b * first) / determinant;
        let refined = encode_three_colour(e0, e1, &colors, opaque);
        if (refined.error < best.error) {
            best = refined;
        }
    }
    blocks[at] = best.endpoints;
    blocks[at + 1u] = best.indices;
}
