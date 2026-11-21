// Prototype of the BC7 RDO's fixed-offset search on the GPU, to measure whether it pays (rdo_gpu_proto.rs).
// One workgroup per row chunk walks the chunk's blocks in order, as a CPU thread does; for each mode 6 or mode 4
// block its threads share the candidates (earlier block of the same mode in the look-back, run length 16 to 3,
// offset), each keeping its own best, and the lowest cost wins, the earliest candidate on a tie.
// Left out against the CPU: the hash of tried segments, the match-continuation and REP0 prices, the ultrasmooth
// scales and the error bound shared across threads.

const WG_SIZE: u32 = 64u;
const BLOCK_TEXELS: u32 = 16u;
const CANDIDATES_PER_BLOCK: u32 = 105u;
const NO_CANDIDATE: u32 = 0xFFFFFFFFu;
const LITERAL_BITS: f32 = 13.0;
const MS_ERROR_SCALE: f32 = 64.0;
const MAX_ERROR: u32 = 0xFFFFFFFFu;

struct Params {
    lambda: f32,
    lookback_blocks: u32,
    thresh_ratio_squared: f32,
    chunk_count: u32,
    // This submission walks blocks block_begin to block_begin + block_count of each chunk.
    block_begin: u32,
    block_count: u32,
}

@group(0) @binding(0) var<storage, read_write> blocks: array<vec4<u32>>;
@group(0) @binding(1) var<storage, read> source: array<u32>;
@group(0) @binding(2) var<storage, read> chunks: array<vec2<u32>>;
@group(0) @binding(3) var<storage, read> error_scales: array<f32>;
@group(0) @binding(4) var<storage, read> distance_bits: array<f32>;
@group(0) @binding(5) var<uniform> params: Params;

var<workgroup> shared_source: array<u32, 16>;
var<workgroup> best_cost_bits: atomic<u32>;
var<workgroup> best_candidate: atomic<u32>;
var<workgroup> shared_current_error: u32;

const WEIGHTS2 = array<u32, 4>(0u, 21u, 43u, 64u);
const WEIGHTS3 = array<u32, 8>(0u, 9u, 18u, 27u, 37u, 46u, 55u, 64u);
const WEIGHTS4 = array<u32, 16>(0u, 4u, 9u, 13u, 17u, 21u, 26u, 30u, 34u, 38u, 43u, 47u, 51u, 55u, 60u, 64u);

fn field(b: vec4<u32>, start: u32, count: u32) -> u32 {
    let word = start >> 5u;
    let shift = start & 31u;
    var value = b[word] >> shift;
    if (shift != 0u && word < 3u) {
        value |= b[word + 1u] << (32u - shift);
    }
    return value & ((1u << count) - 1u);
}

fn block_mode(b: vec4<u32>) -> u32 {
    let low = b.x & 0xFFu;
    if (low == 0u) { return 8u; }
    return firstTrailingBit(low);
}

fn interpolate(e0: u32, e1: u32, w: u32) -> u32 {
    return ((64u - w) * e0 + w * e1 + 32u) >> 6u;
}

fn texel_error(decoded: vec4<u32>, texel: u32) -> u32 {
    let s = unpack4xU8(shared_source[texel]);
    let d = vec4<i32>(decoded) - vec4<i32>(s);
    return u32(dot(d, d));
}

fn mode6_error(b: vec4<u32>, bound: u32) -> u32 {
    let p0 = field(b, 63u, 1u);
    let p1 = field(b, 64u, 1u);
    let e0 = vec4<u32>(field(b, 7u, 7u), field(b, 21u, 7u), field(b, 35u, 7u), field(b, 49u, 7u)) * 2u + p0;
    let e1 = vec4<u32>(field(b, 14u, 7u), field(b, 28u, 7u), field(b, 42u, 7u), field(b, 56u, 7u)) * 2u + p1;
    var error = 0u;
    for (var t = 0u; t < BLOCK_TEXELS; t++) {
        var index: u32;
        if (t == 0u) { index = field(b, 65u, 3u); } else { index = field(b, 64u + 4u * t, 4u); }
        let w = WEIGHTS4[index];
        let decoded = vec4<u32>(interpolate(e0.x, e1.x, w), interpolate(e0.y, e1.y, w),
                                interpolate(e0.z, e1.z, w), interpolate(e0.w, e1.w, w));
        error += texel_error(decoded, t);
        if (error >= bound) { return error; }
    }
    return error;
}

fn mode4_error(b: vec4<u32>, bound: u32) -> u32 {
    let rotation = field(b, 5u, 2u);
    let index_mode = field(b, 7u, 1u);
    let c0 = vec3<u32>(field(b, 8u, 5u), field(b, 18u, 5u), field(b, 28u, 5u));
    let c1 = vec3<u32>(field(b, 13u, 5u), field(b, 23u, 5u), field(b, 33u, 5u));
    let e0 = vec4<u32>((c0 << vec3<u32>(3u)) | (c0 >> vec3<u32>(2u)), 0u);
    let e1 = vec4<u32>((c1 << vec3<u32>(3u)) | (c1 >> vec3<u32>(2u)), 0u);
    let a0r = field(b, 38u, 6u);
    let a1r = field(b, 44u, 6u);
    let a0 = (a0r << 2u) | (a0r >> 4u);
    let a1 = (a1r << 2u) | (a1r >> 4u);
    var error = 0u;
    for (var t = 0u; t < BLOCK_TEXELS; t++) {
        var i2: u32;
        var i3: u32;
        if (t == 0u) { i2 = field(b, 50u, 1u); i3 = field(b, 81u, 2u); }
        else { i2 = field(b, 49u + 2u * t, 2u); i3 = field(b, 80u + 3u * t, 3u); }
        var wc = WEIGHTS2[i2];
        var wa = WEIGHTS3[i3];
        if (index_mode == 1u) { wc = WEIGHTS3[i3]; wa = WEIGHTS2[i2]; }
        var d = vec4<u32>(interpolate(e0.x, e1.x, wc), interpolate(e0.y, e1.y, wc),
                          interpolate(e0.z, e1.z, wc), interpolate(a0, a1, wa));
        if (rotation == 1u) { d = d.wyzx; } else if (rotation == 2u) { d = d.xwzy; } else if (rotation == 3u) { d = d.xywz; }
        error += texel_error(d, t);
        if (error >= bound) { return error; }
    }
    return error;
}

fn block_error(b: vec4<u32>, mode: u32, bound: u32) -> u32 {
    if (mode == 6u) { return mode6_error(b, bound); }
    return mode4_error(b, bound);
}

// The candidate's run length and offset, in the CPU's order: length 16 down to 3, offsets up.
fn length_and_offset(candidate: u32) -> vec2<u32> {
    var remaining = candidate;
    var len = 16u;
    for (var step = 0u; step < 14u; step++) {
        let offsets = 17u - len;
        if (remaining < offsets) { break; }
        remaining -= offsets;
        len -= 1u;
    }
    return vec2<u32>(len, remaining);
}

fn byte_mask(start_byte: u32, len: u32, word: u32) -> u32 {
    var mask = 0u;
    for (var k = 0u; k < 4u; k++) {
        let byte = word * 4u + k;
        if (byte >= start_byte && byte < start_byte + len) { mask |= 0xFFu << (8u * k); }
    }
    return mask;
}

fn copy_segment(dst: vec4<u32>, src: vec4<u32>, ofs: u32, len: u32) -> vec4<u32> {
    let mask = vec4<u32>(byte_mask(ofs, len, 0u), byte_mask(ofs, len, 1u), byte_mask(ofs, len, 2u), byte_mask(ofs, len, 3u));
    return (dst & ~mask) | (src & mask);
}

fn match_len_bits(len: u32) -> f32 {
    if (len >= 12u) { return 9.0; }
    if (len >= 8u) { return 8.0; }
    if (len >= 6u) { return 7.0; }
    return 6.0;
}

@compute @workgroup_size(64)
fn rdo(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    if (group.x >= params.chunk_count) { return; }
    let chunk = chunks[group.x];
    let end = min(chunk.y, chunk.x + params.block_begin + params.block_count);
    for (var i = chunk.x + params.block_begin; i < end; i++) {
        if (lid < BLOCK_TEXELS) { shared_source[lid] = source[i * BLOCK_TEXELS + lid]; }
        if (lid == 0u) {
            atomicStore(&best_cost_bits, 0x7F7FFFFFu);
            atomicStore(&best_candidate, NO_CANDIDATE);
        }
        workgroupBarrier();
        let orig = blocks[i];
        let mode = block_mode(orig);
        // The block's own error, zero for the modes left alone, read by all as a uniform value.
        if (lid == 0u) {
            var error = 0u;
            if (mode == 6u || mode == 4u) { error = block_error(orig, mode, MAX_ERROR); }
            shared_current_error = error;
        }
        let cur_err = workgroupUniformLoad(&shared_current_error);
        if (cur_err == 0u) { continue; }
        let scale = error_scales[i];
        let cur_ms = f32(cur_err) / MS_ERROR_SCALE;
        let thresh = params.thresh_ratio_squared * max(cur_ms, 1.0);
        var best_t = cur_ms * scale + LITERAL_BITS * 16.0 * params.lambda;
        var best_k = NO_CANDIDATE;
        let first = max(chunk.x, i - min(i, params.lookback_blocks));
        let total = (i - first) * CANDIDATES_PER_BLOCK;
        for (var k = lid; k < total; k += WG_SIZE) {
            let delta = k / CANDIDATES_PER_BLOCK + 1u;
            let prev = blocks[i - delta];
            if (block_mode(prev) != mode) { continue; }
            let lo = length_and_offset(k % CANDIDATES_PER_BLOCK);
            let rate = (LITERAL_BITS * f32(16u - lo.x) + distance_bits[delta] + match_len_bits(lo.x)) * params.lambda;
            if (rate >= best_t) { continue; }
            let trial = copy_segment(orig, prev, lo.y, lo.x);
            if (all(trial == orig)) {
                let t = cur_ms * scale + rate;
                if (t < best_t) { best_t = t; best_k = k; }
                continue;
            }
            if (lo.y == 0u && block_mode(trial) != mode) { continue; }
            let bound_f = ceil(max(best_t - rate, 0.0) * MS_ERROR_SCALE / scale);
            let bound = u32(min(bound_f, 4294967040.0));
            let err = block_error(trial, mode, bound);
            if (err >= bound) { continue; }
            let ms = f32(err) / MS_ERROR_SCALE;
            if (ms < thresh) {
                let t = ms * scale + rate;
                if (t < best_t) { best_t = t; best_k = k; }
            }
        }
        if (best_k != NO_CANDIDATE) { atomicMin(&best_cost_bits, bitcast<u32>(best_t)); }
        workgroupBarrier();
        if (best_k != NO_CANDIDATE && bitcast<u32>(best_t) == atomicLoad(&best_cost_bits)) {
            atomicMin(&best_candidate, best_k);
        }
        workgroupBarrier();
        if (lid == 0u) {
            let k = atomicLoad(&best_candidate);
            if (k != NO_CANDIDATE) {
                let delta = k / CANDIDATES_PER_BLOCK + 1u;
                let lo = length_and_offset(k % CANDIDATES_PER_BLOCK);
                blocks[i] = copy_segment(orig, blocks[i - delta], lo.y, lo.x);
            }
        }
        storageBarrier();
        workgroupBarrier();
    }
}
