// Copyright (c) 2025, Nils Hasenbanck
// Copyright (c) 2016-2024, Intel Corporation
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to
// permit persons to whom the Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or substantial portions of
// the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
// THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
// TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

// uot-gpu-compute's copy of block_compression 0.11.0's BC1 kernel (src/shader/bc1_to_5.wgsl,
// https://github.com/hasenbanck/block_compression, MIT, the notice above), for the frame encoder of block_encode.rs,
// with BC1's one bit of alpha added (from `punch_through_block` on). The crate's kernel encodes colour only, in the
// four-colour mode, and from the texture's first row; here a frame starts at row `texture_y_offset` as in the crate's
// BC7 kernel, and the alpha is ours, as uot-texture-codecs' CPU BC1 (the dds crate) keeps it: a texel is opaque from
// half alpha up, otherwise transparent, which only the three-colour mode's fourth index (transparent black) gives.
// An opaque block keeps the crate's encoding, unless its endpoints came out equal: that is the three-colour mode too,
// whose index 3 would be transparent, so its texels then all take index 0, the block's one colour. A block with a
// transparent texel is encoded in the three-colour mode from its opaque texels, and an empty block is transparent.
// The crate's BC2 to BC5 kernels are left out.
//
// block_encode.rs compiles this kernel without bounds checks, so every index must stay in range by itself
// (docs/agent_notes/atlas_page_compression.md, "Bounds checks off"): the arrays and vectors are indexed in loops of
// their own lengths, and the texture's and the output's, whose range depends on the inputs, through texel_coordinates
// and store_data_2.

struct Uniforms {
    width: u32,
    height: u32,
    texture_y_offset: u32,
    blocks_offset: u32,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> block_buffer: array<u32>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

fn sq(x: f32) -> f32 {
    return x * x;
}

fn rsqrt(x: f32) -> f32 {
    return 1.0 / sqrt(x);
}

fn rcp(x: f32) -> f32 {
    return 1.0 / x;
}

// The texel at `x`, `y` of the source, its last row or column for one past its edges (none in range: a frame's texture
// covers its blocks).
fn texel_coordinates(x: u32, y: u32) -> vec2<u32> {
    return min(vec2<u32>(x, y), textureDimensions(source_texture, 0) - vec2<u32>(1u));
}

fn load_block_interleaved_rgba(block: ptr<function, array<f32, 64>>, xx: u32, yy: u32) {
    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let pixel_x = xx * 4u + x;
            let pixel_y = yy * 4u + y + uniforms.texture_y_offset;
            let rgba = textureLoad(source_texture, texel_coordinates(pixel_x, pixel_y), 0);

            (*block)[16u * 0u + y * 4u + x] = rgba.r * 255.0;
            (*block)[16u * 1u + y * 4u + x] = rgba.g * 255.0;
            (*block)[16u * 2u + y * 4u + x] = rgba.b * 255.0;
            (*block)[16u * 3u + y * 4u + x] = rgba.a * 255.0;
        }
    }
}

fn store_data_2(block_width: u32, xx: u32, yy: u32, data: vec2<u32>) {
    let offset = uniforms.blocks_offset + (yy * block_width * 2u + xx * 2u);
    // In range: block_encode.rs asserts that a frame's blocks fit the buffer.
    if (offset + 1u >= arrayLength(&block_buffer)) {
        return;
    }

    block_buffer[offset + 0] = data[0];
    block_buffer[offset + 1] = data[1];
}

fn compute_covar_dc(
    covar: ptr<function, array<f32, 6>>,
    dc: ptr<function, vec3<f32>>,
    block: ptr<function, array<f32, 64>>,
) {
    for (var p = 0u; p < 3u; p++) {
        var acc = 0.0;
        for (var k = 0u; k < 16u; k++) {
            acc += (*block)[k + p * 16u];
        }
        (*dc)[p] = acc / 16.0;
    }

    var covar0 = 0.0;
    var covar1 = 0.0;
    var covar2 = 0.0;
    var covar3 = 0.0;
    var covar4 = 0.0;
    var covar5 = 0.0;

    for (var k = 0u; k < 16u; k++) {
        let rgb0 = (*block)[k + 0u * 16u] - (*dc)[0];
        let rgb1 = (*block)[k + 1u * 16u] - (*dc)[1];
        let rgb2 = (*block)[k + 2u * 16u] - (*dc)[2];

        covar0 += rgb0 * rgb0;
        covar1 += rgb0 * rgb1;
        covar2 += rgb0 * rgb2;
        covar3 += rgb1 * rgb1;
        covar4 += rgb1 * rgb2;
        covar5 += rgb2 * rgb2;
    }

    (*covar)[0] = covar0;
    (*covar)[1] = covar1;
    (*covar)[2] = covar2;
    (*covar)[3] = covar3;
    (*covar)[4] = covar4;
    (*covar)[5] = covar5;
}

fn ssymv(result: ptr<function, vec3<f32>>, covar: ptr<function, array<f32, 6>>, a_vector: ptr<function, vec3<f32>>) {
    (*result)[0] = (*covar)[0] * (*a_vector)[0] + (*covar)[1] * (*a_vector)[1] + (*covar)[2] * (*a_vector)[2];
    (*result)[1] = (*covar)[1] * (*a_vector)[0] + (*covar)[3] * (*a_vector)[1] + (*covar)[4] * (*a_vector)[2];
    (*result)[2] = (*covar)[2] * (*a_vector)[0] + (*covar)[4] * (*a_vector)[1] + (*covar)[5] * (*a_vector)[2];
}

fn compute_axis3(axis: ptr<function, vec3<f32>>, covar: ptr<function, array<f32, 6>>, powerIterations: i32) {
    var a_vector = vec3<f32>(1.0, 1.0, 1.0);

    for (var i = 0; i < powerIterations; i++) {
        ssymv(axis, covar, &a_vector);

        for (var p = 0u; p < 3u; p++) {
            a_vector[p] = (*axis)[p];
        }

        if (i % 2 == 1) {
            var norm_sq = 0.0;
            for (var p = 0u; p < 3u; p++) {
                norm_sq += (*axis)[p] * (*axis)[p];
            }

            let rnorm = rsqrt(norm_sq);
            for (var p = 0u; p < 3u; p++) {
                a_vector[p] *= rnorm;
            }
        }
    }

    for (var p = 0u; p < 3u; p++) {
        (*axis)[p] = a_vector[p];
    }
}

fn pick_endpoints(
    c0: ptr<function, vec3<f32>>,
    c1: ptr<function, vec3<f32>>,
    block: ptr<function, array<f32, 64>>,
    axis: ptr<function, vec3<f32>>,
    dc: ptr<function, vec3<f32>>
) {
    var min_dot = 256.0 * 256.0;
    var max_dot = 0.0;

    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            var dot = 0.0;
            for (var p = 0u; p < 3u; p++) {
                dot += ((*block)[p * 16u + y * 4u + x] - (*dc)[p]) * (*axis)[p];
            }

            min_dot = min(min_dot, dot);
            max_dot = max(max_dot, dot);
        }
    }

    if (max_dot - min_dot < 1.0) {
        min_dot -= 0.5;
        max_dot += 0.5;
    }

    var norm_sq = 0.0;
    for (var p = 0u; p < 3u; p++) {
        norm_sq += (*axis)[p] * (*axis)[p];
    }

    let rnorm_sq = rcp(norm_sq);
    for (var p = 0u; p < 3u; p++) {
        (*c0)[p] = clamp((*dc)[p] + min_dot * rnorm_sq * (*axis)[p], 0.0, 255.0);
        (*c1)[p] = clamp((*dc)[p] + max_dot * rnorm_sq * (*axis)[p], 0.0, 255.0);
    }
}

fn dec_rgb565(c: ptr<function, vec3<f32>>, p: i32) {
    let b5 = (p >> 0) & 31;
    let g6 = (p >> 5) & 63;
    let r5 = (p >> 11) & 31;

    (*c)[0] = f32((r5 << 3) + (r5 >> 2));
    (*c)[1] = f32((g6 << 2) + (g6 >> 4));
    (*c)[2] = f32((b5 << 3) + (b5 >> 2));
}

fn enc_rgb565(c: ptr<function, vec3<f32>>) -> i32 {
    let r = i32((*c)[0]);
    let g = i32((*c)[1]);
    let b = i32((*c)[2]);

    let r5 = (r * 31 + 128 + ((r * 31) >> 8)) >> 8;
    let g6 = (g * 63 + 128 + ((g * 63) >> 8)) >> 8;
    let b5 = (b * 31 + 128 + ((b * 31) >> 8)) >> 8;

    return (r5 << 11) + (g6 << 5) + b5;
}

fn fast_quant(block: ptr<function, array<f32, 64>>, p0: i32, p1: i32) -> u32 {
    var c0: vec3<f32>;
    var c1: vec3<f32>;
    dec_rgb565(&c0, p0);
    dec_rgb565(&c1, p1);

    var dir: vec3<f32>;
    for (var p = 0u; p < 3u; p++) {
        dir[p] = c1[p] - c0[p];
    }

    var sq_norm = 0.0;
    for (var p = 0u; p < 3u; p++) {
        sq_norm += sq(dir[p]);
    }

    let rsq_norm = rcp(sq_norm);

    for (var p = 0u; p < 3u; p++) {
        dir[p] *= rsq_norm * 3.0;
    }

    var bias = 0.5;
    for (var p = 0u; p < 3u; p++) {
        bias -= c0[p] * dir[p];
    }

    var bits = 0u;
    var scaler = 1u;
    for (var k = 0u; k < 16u; k++) {
        var dot = 0.0;
        for (var p = 0u; p < 3u; p++) {
            dot += (*block)[k + p * 16u] * dir[p];
        }

        let q = clamp(i32(dot + bias), 0, 3);
        bits += u32(q) * scaler;
        scaler *= 4u;
    }

    return bits;
}

fn bc1_refine(pe: ptr<function, vec2<i32>>, block: ptr<function, array<f32, 64>>, bits: u32, dc: ptr<function, vec3<f32>>) {
    var c0: vec3<f32>;
    var c1: vec3<f32>;

    if ((bits ^ (bits * 4u)) < 4u) {
        for (var p = 0u; p < 3u; p++) {
            c0[p] = (*dc)[p];
            c1[p] = (*dc)[p];
        }
    } else {
        var atb1: vec3<f32>;
        var sum_q = 0.0;
        var sum_qq = 0.0;
        var shifted_bits = bits;

        for (var k = 0u; k < 16u; k++) {
            let q = f32(shifted_bits & 3u);
            shifted_bits = shifted_bits >> 2u;

            let x = 3.0 - q;

            sum_q += q;
            sum_qq += q * q;

            for (var p = 0u; p < 3u; p++) {
                atb1[p] += x * (*block)[k + p * 16u];
            }
        }

        var sum: vec3<f32>;
        var atb2: vec3<f32>;

        for (var p = 0u; p < 3u; p++) {
            sum[p] = (*dc)[p] * 16.0;
            atb2[p] = 3.0 * sum[p] - atb1[p];
        }

        let cxx = 16.0 * sq(3.0) - 2.0 * 3.0 * sum_q + sum_qq;
        let cyy = sum_qq;
        let cxy = 3.0 * sum_q - sum_qq;
        let scale = 3.0 * rcp(cxx * cyy - cxy * cxy);

        for (var p = 0u; p < 3u; p++) {
            c0[p] = (atb1[p] * cyy - atb2[p] * cxy) * scale;
            c1[p] = (atb2[p] * cxx - atb1[p] * cxy) * scale;

            c0[p] = clamp(c0[p], 0.0, 255.0);
            c1[p] = clamp(c1[p], 0.0, 255.0);
        }
    }

    (*pe)[0] = enc_rgb565(&c0);
    (*pe)[1] = enc_rgb565(&c1);
}

fn fix_qbits(qbits: u32) -> u32 {
    const MASK_01B: u32 = 0x55555555u;
    const MASK_10B: u32 = 0xAAAAAAAAu;

    let qbits0 = qbits & MASK_01B;
    let qbits1 = qbits & MASK_10B;
    return (qbits1 >> 1u) + (qbits1 ^ (qbits0 << 1u));
}

fn compress_block_bc1_core(block: ptr<function, array<f32, 64>>) -> vec2<u32> {
    let power_iterations = 4;
    let refine_iterations = 1;

    var covar: array<f32, 6>;
    var dc: vec3<f32>;
    compute_covar_dc(&covar, &dc, block);

    const eps = 0.001;
    covar[0] += eps;
    covar[3] += eps;
    covar[5] += eps;

    var axis: vec3<f32>;
    compute_axis3(&axis, &covar, power_iterations);

    var c0: vec3<f32>;
    var c1: vec3<f32>;
    pick_endpoints(&c0, &c1, block, &axis, &dc);

    var p: vec2<i32>;
    p[0] = enc_rgb565(&c0);
    p[1] = enc_rgb565(&c1);
    if (p[0] < p[1]) {
        let temp = p[0];
        p[0] = p[1];
        p[1] = temp;
    }

    var data: vec2<u32>;
    data[0] = (u32(p[1]) << 16u) | u32(p[0]);
    data[1] = fast_quant(block, p[0], p[1]);

    for (var i = 0; i < refine_iterations; i++) {
        bc1_refine(&p, block, data[1], &dc);
        if (p[0] < p[1]) {
            let temp = p[0];
            p[0] = p[1];
            p[1] = temp;
        }
        data[0] = (u32(p[1]) << 16u) | u32(p[0]);
        data[1] = fast_quant(block, p[0], p[1]);
    }

    data[1] = fix_qbits(data[1]);
    return data;
}


// BC1's one bit of alpha (uot-gpu-compute), on the block as load_block_interleaved_rgba gives it.

const TEXELS: u32 = 16u;
const ALPHA_CHANNEL: u32 = 3u;
const ALL_OPAQUE: u32 = 0xffffu;
// Opaque from this alpha up, as the dds crate's BC1 (`ALPHA_THRESHOLD`): Classic art and its in-betweens are 0 or 1.
const OPAQUE_FROM: f32 = 0.5;
const CHANNEL_MAX: f32 = 255.0;
const TRANSPARENT_INDEX: u32 = 3u;
const TRANSPARENT_INDICES: u32 = 0xffffffffu;
const INDEX_BITS: u32 = 2u;
const INDEX_MASK: u32 = 3u;
const ENDPOINT_BITS: u32 = 16u;
const ENDPOINT_MASK: u32 = 0xffffu;
// Power iterations for the principal axis of the opaque texels' colours, as the ISPC kernel does.
const POWER_ITERATIONS: u32 = 4u;
const DEGENERATE: f32 = 1e-6;
// RGB565.
const RED_SHIFT: u32 = 11u;
const GREEN_SHIFT: u32 = 5u;
const FIVE_BIT_MASK: u32 = 0x1fu;
const SIX_BIT_MASK: u32 = 0x3fu;
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
    let r = (packed >> RED_SHIFT) & FIVE_BIT_MASK;
    let g = (packed >> GREEN_SHIFT) & SIX_BIT_MASK;
    let b = packed & FIVE_BIT_MASK;
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

// A block with transparent texels (`opaque`, a bit per texel, has some but not all set), in the three-colour mode.
fn punch_through_block(block: ptr<function, array<f32, 64>>, opaque: u32) -> vec2<u32> {
    var colors: array<vec3<f32>, 16>;
    var count = 0u;
    var mean = vec3<f32>(0.0);
    for (var k = 0u; k < TEXELS; k++) {
        colors[k] = vec3<f32>((*block)[k], (*block)[k + TEXELS], (*block)[k + 2u * TEXELS]);
        if ((opaque & (1u << k)) != 0u) {
            count += 1u;
            mean += colors[k];
        }
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

    // One least-squares refinement of the endpoints for the indices found: index 0 weighs (1, 0), 1 (0, 1),
    // 2 (1/2, 1/2).
    var a = 0.0; var b = 0.0; var c = 0.0;
    var first = vec3<f32>(0.0);
    var second = vec3<f32>(0.0);
    for (var k = 0u; k < TEXELS; k++) {
        let index = (best.indices >> (k * INDEX_BITS)) & INDEX_MASK;
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
    return vec2<u32>(best.endpoints, best.indices);
}

fn compress_block_bc1_alpha(block: ptr<function, array<f32, 64>>) -> vec2<u32> {
    var opaque = 0u;
    for (var k = 0u; k < TEXELS; k++) {
        if ((*block)[k + ALPHA_CHANNEL * TEXELS] >= OPAQUE_FROM * CHANNEL_MAX) {
            opaque |= 1u << k;
        }
    }
    if (opaque == ALL_OPAQUE) {
        var data = compress_block_bc1_core(block);
        if ((data[0] & ENDPOINT_MASK) <= (data[0] >> ENDPOINT_BITS)) {
            data[1] = 0u;
        }
        return data;
    }
    if (opaque == 0u) {
        return vec2<u32>(0u, TRANSPARENT_INDICES);
    }
    return punch_through_block(block, opaque);
}

@compute
@workgroup_size(8, 8)
fn compress_bc1(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let xx = global_id.x;
    let yy = global_id.y;

    let block_width = (uniforms.width + 3u) / 4u;
    let block_height = (uniforms.height + 3u) / 4u;

    if (xx >= block_width || yy >= block_height) {
        return;
    }

    var block: array<f32, 64>;
    load_block_interleaved_rgba(&block, xx, yy);
    store_data_2(block_width, xx, yy, compress_block_bc1_alpha(&block));
}
