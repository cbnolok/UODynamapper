// The GPU BC7 RDO (bc7_rdo.rs): the entropy reduction transform of bc7enc_rdo (ert.cpp) with the CPU RDO's price
// model and error scales, and a search of its own. One workgroup walks a chunk of a page's block rows in order; for
// each block its threads share the candidates (an earlier block of the same mode within the look-back, a run length
// 16 to 3, an offset). The block takes the candidate of lowest cost, the lowest candidate number on a tie (nearest
// block, longest run, lowest offset), if it costs less than the block as it is.
//
// The choice is that minimum over all the candidates, whatever thread tries which: a thread drops a candidate only
// when its cost cannot be below the thread's best (the rate alone reaches it, or the error passes a bound kept above
// the exact one), and that best came from a lower candidate number. So the blocks depend on the page, λ and look-back
// only, not on the workgroup size, the chunks walked at once or the blocks walked per submission.
//
// A match that continues the previous block's run (the block after its source, from offset 0, when that run reached
// its block's end) or reuses the previous match's distance (REP0) pays the cheaper prices of ERT_FAVOR_CONT_AND_REP0;
// the state is the previous matched block's, as in ert.cpp and the BC1 RDO, kept across submissions in chunk_states.
// Every loop ends: they run to constants, to uniform values or to a chunk's end read once, counting up.

const WG_SIZE: u32 = 64u;
const BLOCK_TEXELS: u32 = 16u;
const BLOCK_BYTES: u32 = 16u;
const CANDIDATES_PER_BLOCK: u32 = 105u;
/// The rates per block distance: one per run length 0 to 16.
const RATE_ROW: u32 = 17u;
const NO_CANDIDATE: u32 = 0xFFFFFFFFu;
const NO_BLOCK: u32 = 0xFFFFFFFFu;
/// The current error of a block the walk leaves alone (no valid mode, or no error with skip_zero_error).
const SKIP_BLOCK: u32 = 0xFFFFFFFFu;
const INVALID_MODE: u32 = 8u;
const MS_ERROR_SCALE: f32 = 64.0;
const MAX_ERROR: u32 = 0xFFFFFFFFu;
/// The largest f32 below 2^32, so that the bound converts to u32.
const MAX_BOUND: f32 = 4294967040.0;
/// How far above the exact error bound the trial bound stays, relative to the costs: far more than the rounding of
/// the cost, so a dropped candidate never costs less than the best.
const BOUND_MARGIN: f32 = 1.0 / 1048576.0;
/// The largest positive f32, as bits: the empty best cost.
const MAX_COST_BITS: u32 = 0x7F7FFFFFu;
const OPAQUE: u32 = 255u;

// The tables buffer (bc7_rdo.rs `tables`): the 2-subset partitions (a bit per texel), the 3-subset ones (two bits per
// texel), the anchors (second subset | third subset's first << 8 | its second << 16), each candidate's run length and
// offset (length | offset << 8).
const PARTITION2_TABLE: u32 = 0u;
const PARTITION3_TABLE: u32 = 64u;
const ANCHOR_TABLE: u32 = 128u;
const CANDIDATE_TABLE: u32 = 192u;
const NO_ANCHOR: u32 = 16u;

struct Params {
    lookback_blocks: u32,
    chunk_count: u32,
    // This submission walks blocks block_begin to block_begin + block_count of each chunk.
    block_begin: u32,
    block_count: u32,
    thresh_ratio_squared: f32,
    // The block as it is: 16 literal bytes, times λ.
    literal_rate: f32,
    skip_zero_error: u32,
    pad: u32,
}

@group(0) @binding(0) var<storage, read_write> blocks: array<vec4<u32>>;
@group(0) @binding(1) var<storage, read> texels: array<u32>;
@group(0) @binding(2) var<storage, read> error_scales: array<f32>;
// Each chunk's first block and the block after its last.
@group(0) @binding(3) var<storage, read> chunks: array<vec2<u32>>;
// Each chunk's REP0 distance in blocks (0 for none) and the block whose run from offset 0 continues the last one.
@group(0) @binding(4) var<storage, read_write> chunk_states: array<vec2<u32>>;
// λ times the bits of a match: per block distance 0 to the look-back and run length, then a continuing run's and a
// REP0 run's per run length (bc7_rdo.rs `rates`).
@group(0) @binding(5) var<storage, read> rates: array<f32>;
@group(0) @binding(6) var<storage, read> tables: array<u32>;
@group(0) @binding(7) var<uniform> params: Params;

var<workgroup> shared_source: array<u32, 16>;
var<workgroup> texel_errors: array<u32, 16>;
var<workgroup> current_error: u32;
var<workgroup> best_cost_bits: atomic<u32>;
var<workgroup> best_candidate: atomic<u32>;
var<workgroup> rep0_delta: u32;
var<workgroup> continue_block: u32;

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
    if (low == 0u) { return INVALID_MODE; }
    return firstTrailingBit(low);
}

/// BC7's weight of index `i` of `bits`-bit indices: round(i * 64 / (2^bits - 1)), which the spec's tables list.
fn weight(i: u32, bits: u32) -> u32 {
    let top = (1u << bits) - 1u;
    return (i * 128u + top) / (2u * top);
}

/// A `bits`-bit endpoint widened to 8 bits by repeating its top bits.
fn expand(v: u32, bits: u32) -> u32 {
    if (bits >= 8u) { return v; }
    return (v << (8u - bits)) | (v >> (2u * bits - 8u));
}

fn interpolate(e0: u32, e1: u32, w: u32) -> u32 {
    return ((64u - w) * e0 + w * e1 + 32u) >> 6u;
}

fn mix4(e0: vec4<u32>, e1: vec4<u32>, w: u32) -> vec4<u32> {
    return vec4<u32>(interpolate(e0.x, e1.x, w), interpolate(e0.y, e1.y, w), interpolate(e0.z, e1.z, w),
        interpolate(e0.w, e1.w, w));
}

fn texel_error(decoded: vec4<u32>, t: u32) -> u32 {
    let d = vec4<i32>(decoded) - vec4<i32>(unpack4xU8(shared_source[t]));
    return u32(dot(d, d));
}

fn rotate(d: vec4<u32>, rotation: u32) -> vec4<u32> {
    if (rotation == 1u) { return d.wyzx; }
    if (rotation == 2u) { return d.xwzy; }
    if (rotation == 3u) { return d.xywz; }
    return d;
}

// ─── Mode 6: one subset, RGBA 7.7.7.7 with a p-bit per endpoint, 4-bit indices ───

struct Mode6 { e0: vec4<u32>, e1: vec4<u32> }

fn mode6(b: vec4<u32>) -> Mode6 {
    let p0 = field(b, 63u, 1u);
    let p1 = field(b, 64u, 1u);
    let e0 = vec4<u32>(field(b, 7u, 7u), field(b, 21u, 7u), field(b, 35u, 7u), field(b, 49u, 7u)) * 2u + p0;
    let e1 = vec4<u32>(field(b, 14u, 7u), field(b, 28u, 7u), field(b, 42u, 7u), field(b, 56u, 7u)) * 2u + p1;
    return Mode6(e0, e1);
}

fn mode6_texel(b: vec4<u32>, m: Mode6, t: u32) -> vec4<u32> {
    var index: u32;
    if (t == 0u) { index = field(b, 65u, 3u); } else { index = field(b, 64u + 4u * t, 4u); }
    return mix4(m.e0, m.e1, weight(index, 4u));
}

// ─── Modes 4 and 5: one subset, colour and alpha with indices of their own, a rotation ───

struct SeparateAlpha {
    e0: vec4<u32>,
    e1: vec4<u32>,
    rotation: u32,
    // Mode 4's index selection: colour takes the 3-bit indices when set.
    swap_indices: u32,
}

fn mode4(b: vec4<u32>) -> SeparateAlpha {
    let c0 = vec3<u32>(field(b, 8u, 5u), field(b, 18u, 5u), field(b, 28u, 5u));
    let c1 = vec3<u32>(field(b, 13u, 5u), field(b, 23u, 5u), field(b, 33u, 5u));
    let e0 = vec4<u32>(expand(c0.x, 5u), expand(c0.y, 5u), expand(c0.z, 5u), expand(field(b, 38u, 6u), 6u));
    let e1 = vec4<u32>(expand(c1.x, 5u), expand(c1.y, 5u), expand(c1.z, 5u), expand(field(b, 44u, 6u), 6u));
    return SeparateAlpha(e0, e1, field(b, 5u, 2u), field(b, 7u, 1u));
}

fn mode4_texel(b: vec4<u32>, m: SeparateAlpha, t: u32) -> vec4<u32> {
    var i2: u32;
    var i3: u32;
    if (t == 0u) { i2 = field(b, 50u, 1u); i3 = field(b, 81u, 2u); }
    else { i2 = field(b, 49u + 2u * t, 2u); i3 = field(b, 80u + 3u * t, 3u); }
    var wc = weight(i2, 2u);
    var wa = weight(i3, 3u);
    if (m.swap_indices == 1u) { wc = weight(i3, 3u); wa = weight(i2, 2u); }
    let d = vec4<u32>(interpolate(m.e0.x, m.e1.x, wc), interpolate(m.e0.y, m.e1.y, wc),
        interpolate(m.e0.z, m.e1.z, wc), interpolate(m.e0.w, m.e1.w, wa));
    return rotate(d, m.rotation);
}

fn mode5(b: vec4<u32>) -> SeparateAlpha {
    let e0 = vec4<u32>(expand(field(b, 8u, 7u), 7u), expand(field(b, 22u, 7u), 7u), expand(field(b, 36u, 7u), 7u),
        field(b, 50u, 8u));
    let e1 = vec4<u32>(expand(field(b, 15u, 7u), 7u), expand(field(b, 29u, 7u), 7u), expand(field(b, 43u, 7u), 7u),
        field(b, 58u, 8u));
    return SeparateAlpha(e0, e1, field(b, 6u, 2u), 0u);
}

fn mode5_texel(b: vec4<u32>, m: SeparateAlpha, t: u32) -> vec4<u32> {
    var ic: u32;
    var ia: u32;
    if (t == 0u) { ic = field(b, 66u, 1u); ia = field(b, 97u, 1u); }
    else { ic = field(b, 65u + 2u * t, 2u); ia = field(b, 96u + 2u * t, 2u); }
    let wc = weight(ic, 2u);
    let wa = weight(ia, 2u);
    let d = vec4<u32>(interpolate(m.e0.x, m.e1.x, wc), interpolate(m.e0.y, m.e1.y, wc),
        interpolate(m.e0.z, m.e1.z, wc), interpolate(m.e0.w, m.e1.w, wa));
    return rotate(d, m.rotation);
}

// ─── Modes 0, 1, 2, 3 and 7: two or three subsets, one set of indices ───

struct Partitioned {
    subsets: u32,
    // The partition (its shape).
    shape: u32,
    index_bits: u32,
    index_base: u32,
    // The anchor texels of the second and third subsets (NO_ANCHOR when there is none).
    anchor_a: u32,
    anchor_b: u32,
    lo0: vec4<u32>,
    hi0: vec4<u32>,
    lo1: vec4<u32>,
    hi1: vec4<u32>,
    lo2: vec4<u32>,
    hi2: vec4<u32>,
}

const PBITS_NONE: u32 = 0u;
const PBITS_PER_ENDPOINT: u32 = 1u;
const PBITS_SHARED: u32 = 2u;
const MAX_ENDPOINTS: u32 = 6u;

fn partitioned(b: vec4<u32>, mode: u32) -> Partitioned {
    // Subsets, partition bits, colour bits, alpha bits, p-bits, index bits.
    var subsets = 2u;
    var partition_bits = 6u;
    var colour_bits = 5u;
    var alpha_bits = 0u;
    var pbits = PBITS_PER_ENDPOINT;
    var index_bits = 2u;
    switch mode {
        case 0u: { subsets = 3u; partition_bits = 4u; colour_bits = 4u; index_bits = 3u; }
        case 1u: { colour_bits = 6u; pbits = PBITS_SHARED; index_bits = 3u; }
        case 2u: { subsets = 3u; pbits = PBITS_NONE; }
        case 3u: { colour_bits = 7u; }
        default: { alpha_bits = 5u; }
    }
    var out: Partitioned;
    out.subsets = subsets;
    out.shape = field(b, mode + 1u, partition_bits);
    out.index_bits = index_bits;
    let base = mode + 1u + partition_bits;
    let endpoints = 2u * subsets;
    let alpha_base = base + 3u * endpoints * colour_bits;
    let pbit_base = alpha_base + endpoints * alpha_bits;
    var pbit_count = 0u;
    var widened = 0u;
    if (pbits == PBITS_PER_ENDPOINT) { pbit_count = endpoints; widened = 1u; }
    if (pbits == PBITS_SHARED) { pbit_count = subsets; widened = 1u; }
    out.index_base = pbit_base + pbit_count;
    for (var e = 0u; e < MAX_ENDPOINTS; e++) {
        if (e >= endpoints) { break; }
        var p = 0u;
        if (pbits == PBITS_PER_ENDPOINT) { p = field(b, pbit_base + e, 1u); }
        if (pbits == PBITS_SHARED) { p = field(b, pbit_base + e / 2u, 1u); }
        var v: vec4<u32>;
        for (var c = 0u; c < 3u; c++) {
            let raw = field(b, base + (c * endpoints + e) * colour_bits, colour_bits);
            v[c] = expand((raw << widened) | p, colour_bits + widened);
        }
        v.w = OPAQUE;
        if (alpha_bits > 0u) {
            v.w = expand((field(b, alpha_base + e * alpha_bits, alpha_bits) << widened) | p, alpha_bits + widened);
        }
        switch e {
            case 0u: { out.lo0 = v; }
            case 1u: { out.hi0 = v; }
            case 2u: { out.lo1 = v; }
            case 3u: { out.hi1 = v; }
            case 4u: { out.lo2 = v; }
            default: { out.hi2 = v; }
        }
    }
    let anchors = tables[ANCHOR_TABLE + out.shape];
    if (subsets == 2u) {
        out.anchor_a = anchors & 0xFFu;
        out.anchor_b = NO_ANCHOR;
    } else {
        out.anchor_a = (anchors >> 8u) & 0xFFu;
        out.anchor_b = (anchors >> 16u) & 0xFFu;
    }
    return out;
}

fn partitioned_texel(b: vec4<u32>, m: Partitioned, t: u32) -> vec4<u32> {
    var subset: u32;
    if (m.subsets == 2u) { subset = (tables[PARTITION2_TABLE + m.shape] >> t) & 1u; }
    else { subset = (tables[PARTITION3_TABLE + m.shape] >> (2u * t)) & 3u; }
    // Each anchor texel's index has one bit fewer, so the indices after it start a bit earlier.
    var offset = m.index_base + t * m.index_bits;
    var width = m.index_bits;
    if (t == 0u) { width -= 1u; } else { offset -= 1u; }
    if (t == m.anchor_a) { width -= 1u; } else if (t > m.anchor_a) { offset -= 1u; }
    if (t == m.anchor_b) { width -= 1u; } else if (t > m.anchor_b) { offset -= 1u; }
    let w = weight(field(b, offset, width), m.index_bits);
    var lo = m.lo0;
    var hi = m.hi0;
    if (subset == 1u) { lo = m.lo1; hi = m.hi1; }
    if (subset == 2u) { lo = m.lo2; hi = m.hi2; }
    return mix4(lo, hi, w);
}

// ─── Errors ───

/// The squared RGBA error of block `b` (of `mode`) against the source, or at least `bound` once it reaches it.
fn block_error(b: vec4<u32>, mode: u32, bound: u32) -> u32 {
    var error = 0u;
    switch mode {
        case 6u: {
            let m = mode6(b);
            for (var t = 0u; t < BLOCK_TEXELS; t++) {
                error += texel_error(mode6_texel(b, m, t), t);
                if (error >= bound) { break; }
            }
        }
        case 4u: {
            let m = mode4(b);
            for (var t = 0u; t < BLOCK_TEXELS; t++) {
                error += texel_error(mode4_texel(b, m, t), t);
                if (error >= bound) { break; }
            }
        }
        case 5u: {
            let m = mode5(b);
            for (var t = 0u; t < BLOCK_TEXELS; t++) {
                error += texel_error(mode5_texel(b, m, t), t);
                if (error >= bound) { break; }
            }
        }
        default: {
            let m = partitioned(b, mode);
            for (var t = 0u; t < BLOCK_TEXELS; t++) {
                error += texel_error(partitioned_texel(b, m, t), t);
                if (error >= bound) { break; }
            }
        }
    }
    return error;
}

/// Texel `t` of block `b` (of a valid `mode`), decoded.
fn decode_texel(b: vec4<u32>, mode: u32, t: u32) -> vec4<u32> {
    switch mode {
        case 6u: { return mode6_texel(b, mode6(b), t); }
        case 4u: { return mode4_texel(b, mode4(b), t); }
        case 5u: { return mode5_texel(b, mode5(b), t); }
        default: { return partitioned_texel(b, partitioned(b, mode), t); }
    }
}

/// The trial error from which a candidate with this rate cannot cost less than `best_t`: the exact bound, raised by
/// BOUND_MARGIN of the costs, and one more.
fn error_bound(best_t: f32, rate: f32, error_per_cost: f32) -> u32 {
    let slack = best_t - rate + best_t * BOUND_MARGIN;
    return u32(min(ceil(slack * error_per_cost * (1.0 + BOUND_MARGIN)) + 1.0, MAX_BOUND));
}

// ─── Candidates ───

fn byte_mask(start_byte: u32, len: u32, word: u32) -> u32 {
    var mask = 0u;
    for (var k = 0u; k < 4u; k++) {
        let byte = word * 4u + k;
        if (byte >= start_byte && byte < start_byte + len) { mask |= 0xFFu << (8u * k); }
    }
    return mask;
}

/// `dst` with `len` bytes from offset `ofs` taken from `src`.
fn copy_segment(dst: vec4<u32>, src: vec4<u32>, ofs: u32, len: u32) -> vec4<u32> {
    let mask = vec4<u32>(byte_mask(ofs, len, 0u), byte_mask(ofs, len, 1u), byte_mask(ofs, len, 2u),
        byte_mask(ofs, len, 3u));
    return (dst & ~mask) | (src & mask);
}

@compute @workgroup_size(64)
fn walk(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    if (group.x >= params.chunk_count) { return; }
    let chunk = chunks[group.x];
    let begin = chunk.x + params.block_begin;
    let end = min(chunk.y, begin + params.block_count);
    if (begin >= end) { return; }
    if (lid == 0u) {
        let state = chunk_states[group.x];
        rep0_delta = state.x;
        continue_block = state.y;
    }
    let continue_rates = (params.lookback_blocks + 1u) * RATE_ROW;
    let rep0_rates = continue_rates + RATE_ROW;
    for (var i = begin; i < end; i++) {
        if (lid < BLOCK_TEXELS) { shared_source[lid] = texels[i * BLOCK_TEXELS + lid]; }
        if (lid == 0u) {
            atomicStore(&best_cost_bits, MAX_COST_BITS);
            atomicStore(&best_candidate, NO_CANDIDATE);
        }
        workgroupBarrier();
        let orig = blocks[i];
        let mode = block_mode(orig);
        if (lid < BLOCK_TEXELS && mode != INVALID_MODE) {
            texel_errors[lid] = texel_error(decode_texel(orig, mode, lid), lid);
        }
        workgroupBarrier();
        if (lid == 0u) {
            var error = SKIP_BLOCK;
            if (mode != INVALID_MODE) {
                error = 0u;
                for (var t = 0u; t < BLOCK_TEXELS; t++) { error += texel_errors[t]; }
                if (error == 0u && params.skip_zero_error != 0u) { error = SKIP_BLOCK; }
            }
            current_error = error;
        }
        let cur_err = workgroupUniformLoad(&current_error);
        if (cur_err == SKIP_BLOCK) { continue; }

        let scale = error_scales[i];
        let error_per_cost = MS_ERROR_SCALE / scale;
        let cur_ms = f32(cur_err) / MS_ERROR_SCALE;
        let thresh = params.thresh_ratio_squared * max(cur_ms, 1.0);
        let rep0 = rep0_delta;
        let continues = continue_block;
        var best_t = cur_ms * scale + params.literal_rate;
        var best_k = NO_CANDIDATE;
        let total = min(params.lookback_blocks, i - chunk.x) * CANDIDATES_PER_BLOCK;
        for (var k = lid; k < total; k += WG_SIZE) {
            let delta = k / CANDIDATES_PER_BLOCK + 1u;
            let prev_index = i - delta;
            let prev = blocks[prev_index];
            if (block_mode(prev) != mode) { continue; }
            let shape = tables[CANDIDATE_TABLE + k % CANDIDATES_PER_BLOCK];
            let len = shape & 0xFFu;
            let ofs = shape >> 8u;
            var rate = rates[delta * RATE_ROW + len];
            if (delta == rep0) { rate = rates[rep0_rates + len]; }
            if (ofs == 0u && prev_index == continues) { rate = rates[continue_rates + len]; }
            if (rate >= best_t) { continue; }
            let trial = copy_segment(orig, prev, ofs, len);
            var err = cur_err;
            if (any(trial != orig)) {
                if (ofs == 0u && block_mode(trial) != mode) { continue; }
                let bound = error_bound(best_t, rate, error_per_cost);
                err = block_error(trial, mode, bound);
                if (err >= bound) { continue; }
            }
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
                let shape = tables[CANDIDATE_TABLE + k % CANDIDATES_PER_BLOCK];
                let len = shape & 0xFFu;
                let ofs = shape >> 8u;
                blocks[i] = copy_segment(orig, blocks[i - delta], ofs, len);
                rep0_delta = delta;
                continue_block = NO_BLOCK;
                if (ofs + len == BLOCK_BYTES) { continue_block = i - delta + 1u; }
            } else {
                continue_block = NO_BLOCK;
            }
        }
        storageBarrier();
    }
    workgroupBarrier();
    if (lid == 0u) { chunk_states[group.x] = vec2<u32>(rep0_delta, continue_block); }
}
