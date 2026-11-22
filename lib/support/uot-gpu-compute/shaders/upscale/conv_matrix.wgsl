// The SPAN convolutions of conv.wgsl on the GPU's matrix units: the same matrix product (output pixels times output
// channels, summed over input channels times kernel taps), with the multiply-adds done by 16x16x16 cooperative matrix
// products of fp16 operands into f32 sums (wgpu's cooperative matrix extension; AMD's WMMA on RDNA 3). The patches
// are rounded to fp16 as they are staged, the weights are fp16 already; the sums, the epilogue and the arena stay f32.
//
// A workgroup computes TILE_M pixels by all TILE_N channels, staging TILE_K rows of the sum (input channel and tap) of
// the patches and weights in shared memory at a time, each step's read from memory while the previous step is being
// summed. Each subgroup sums one strip of 16 pixels by TILE_N channels into three matrices. How many subgroups a
// workgroup has is the driver's choice (RADV runs compute in wave64, others in wave32): the host makes STRIPS strips,
// one per subgroup at the largest subgroup size; with smaller subgroups, the ones past the strips sum a strip again
// and store it in a spare strip nobody reads (the cooperative operations need control flow the whole workgroup takes,
// so no subgroup can skip them).
//
// Measured on a Radeon 880M (docs/agent_notes/mobile_animation_motion.md): staging the patches with one pixel's rows
// contiguous (loaded transposed) and the rows padded by four halves (no bank conflicts, 8-byte aligned) makes the
// matrix loads 64-bit; 16 rows a step beats 32 to 96.
//
// upscale.rs puts the f16 and wgpu_cooperative_matrix extensions' enable directives and these constants in front of
// this source, one set per pipeline:
//   THREADS                      threads per workgroup, a multiple of TILE_M
//   STRIPS                       strips of 16 pixels per workgroup
//   TILE_K                       rows of the sum staged at a time, a multiple of 16 and of THREADS / TILE_M
//   KERNEL_SIDE                  3 or 1
//   EPILOGUE                     EPILOGUE_LINEAR, EPILOGUE_SILU or EPILOGUE_ATTENTION

const EPILOGUE_LINEAR: u32 = 0u;
const EPILOGUE_SILU: u32 = 1u;
// SPAB's parameter-free attention: (conv(x) + x) * (sigmoid(conv(x)) - 0.5), x being the block's input.
const EPILOGUE_ATTENTION: u32 = 2u;
const ATTENTION_CENTER: f32 = 0.5;
const TAPS: u32 = KERNEL_SIDE * KERNEL_SIDE;
const KERNEL_PAD: i32 = i32(KERNEL_SIDE / 2u);
// The side of a cooperative matrix.
const SIDE: u32 = 16u;
const TILE_M: u32 = STRIPS * SIDE;
// Every SPAN layer writes 48 channels: three matrices.
const TILE_N: u32 = 48u;
const K_MATRICES: u32 = TILE_K / SIDE;
// The staged rows' stride in halves, padded so that the 16 rows a matrix load reads fall in different banks.
const ROW_PADDING: u32 = 4u;
const ROW_STRIDE: u32 = TILE_K + ROW_PADDING;
// Each thread stages one pixel of the tile, A_ROWS rows of the sum A_K_STEP apart.
const A_K_STEP: u32 = THREADS / TILE_M;
const A_ROWS: u32 = TILE_K / A_K_STEP;
// The weights are read two halves to a word.
const B_PAIRS: u32 = TILE_K * TILE_N / 2u;
const B_LOADS: u32 = (B_PAIRS + THREADS - 1u) / THREADS;
// The sums' stride: the strips and the spare one.
const C_STRIDE: u32 = TILE_M + SIDE;

struct Conv {
    width: u32,
    height: u32,
    pixels: u32,
    in_channels: u32,
    // Elements of the arena: the input's first channel, the output's, and the residual's (attention only).
    in_base: u32,
    out_channels: u32,
    out_base: u32,
    residual_base: u32,
    // Offsets in halves into the packed parameters.
    weights: u32,
    bias: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> conv: Conv;
// fp16 parameters, two to a word; weights as [input channel * TAPS + tap][output channel].
@group(0) @binding(1) var<storage, read> params: array<u32>;
// Channel planes of f32, each `pixels` long.
@group(0) @binding(2) var<storage, read_write> arena: array<f32>;

// The staged patches as [pixel][row of the sum] and weights as [channel][row of the sum], rows ROW_STRIDE apart.
var<workgroup> a_tile: array<f16, ROW_STRIDE * TILE_M>;
var<workgroup> b_tile: array<f16, ROW_STRIDE * TILE_N>;
// The sums as [channel][pixel], C_STRIDE apart, for the epilogue.
var<workgroup> c_tile: array<f32, TILE_N * C_STRIDE>;

fn param(index: u32) -> f32 {
    let pair = unpack2x16float(params[index >> 1u]);
    return select(pair.x, pair.y, (index & 1u) == 1u);
}

fn sigmoid(value: f32) -> f32 {
    return 1.0 / (1.0 + exp(-value));
}

// Adds the bias and applies the epilogue to one output.
fn finish(sum: f32, channel: u32, residual: f32) -> f32 {
    let value = sum + param(conv.bias + channel);
    if (EPILOGUE == EPILOGUE_SILU) {
        return value * sigmoid(value);
    } else if (EPILOGUE == EPILOGUE_ATTENTION) {
        return (value + residual) * (sigmoid(value) - ATTENTION_CENTER);
    }
    return value;
}

// Reads step k0's patches of this thread's pixel (`inside` has a bit for each tap inside the frame) and its share of
// the weights into registers, zero past the sum. The loads are unconditional (a skipped one reads element 0 and is
// replaced by zero), so the compiler can issue them together instead of branching around each.
fn load_step(k0: u32, thread: u32, load_pixel: u32, inside: u32, a_next: ptr<function, array<f32, A_ROWS>>,
             b_next: ptr<function, array<vec2<f32>, B_LOADS>>) {
    let k_end = conv.in_channels * TAPS;
    let load_k = thread / TILE_M;
    for (var row = 0u; row < A_ROWS; row++) {
        let k = k0 + load_k + row * A_K_STEP;
        let channel = k / TAPS;
        let tap = k % TAPS;
        let offset = (i32(tap / KERNEL_SIDE) - KERNEL_PAD) * i32(conv.width) + i32(tap % KERNEL_SIDE) - KERNEL_PAD;
        let valid = k < k_end && (inside & (1u << tap)) != 0u;
        let index = conv.in_base + channel * conv.pixels + u32(i32(load_pixel) + offset);
        (*a_next)[row] = select(0.0, arena[select(0u, index, valid)], valid);
    }
    // The weights' rows k0.. are TILE_K * TILE_N consecutive halves.
    for (var load = 0u; load < B_LOADS; load++) {
        let pair = thread + load * THREADS;
        let k = k0 + pair / (TILE_N / 2u);
        let valid = pair < B_PAIRS && k < k_end;
        let index = select(0u, (conv.weights + k0 * TILE_N) / 2u + pair, valid);
        (*b_next)[load] = select(vec2<f32>(0.0), unpack2x16float(params[index]), valid);
    }
}

@compute @workgroup_size(THREADS)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) thread: u32,
        @builtin(subgroup_id) subgroup: u32) {
    let m0 = group.x * TILE_M;
    let k_end = conv.in_channels * TAPS;

    // The pixel whose patches this thread stages, and which of its taps fall inside the frame.
    let load_m = thread % TILE_M;
    let load_k = thread / TILE_M;
    let load_pixel = m0 + load_m;
    let load_y = i32(load_pixel / conv.width);
    let load_x = i32(load_pixel % conv.width);
    var inside = 0u;
    for (var tap = 0u; tap < TAPS; tap++) {
        let y = load_y + i32(tap / KERNEL_SIDE) - KERNEL_PAD;
        let x = load_x + i32(tap % KERNEL_SIDE) - KERNEL_PAD;
        let tap_inside = load_pixel < conv.pixels && y >= 0 && x >= 0 && y < i32(conv.height) && x < i32(conv.width);
        inside |= select(0u, 1u << tap, tap_inside);
    }

    var sums0 = coop_mat16x16<f32, C>();
    var sums1 = coop_mat16x16<f32, C>();
    var sums2 = coop_mat16x16<f32, C>();
    let strip_offset = (subgroup % STRIPS) * SIDE;
    var a_next: array<f32, A_ROWS>;
    var b_next: array<vec2<f32>, B_LOADS>;
    load_step(0u, thread, load_pixel, inside, &a_next, &b_next);
    for (var k0 = 0u; k0 < k_end; k0 += TILE_K) {
        for (var row = 0u; row < A_ROWS; row++) {
            a_tile[load_m * ROW_STRIDE + load_k + row * A_K_STEP] = f16(a_next[row]);
        }
        for (var load = 0u; load < B_LOADS; load++) {
            let pair = thread + load * THREADS;
            if (pair < B_PAIRS) {
                let kk = pair / (TILE_N / 2u);
                let n = (pair % (TILE_N / 2u)) * 2u;
                b_tile[n * ROW_STRIDE + kk] = f16(b_next[load].x);
                b_tile[(n + 1u) * ROW_STRIDE + kk] = f16(b_next[load].y);
            }
        }
        workgroupBarrier();
        load_step(k0 + TILE_K, thread, load_pixel, inside, &a_next, &b_next);
        for (var kk = 0u; kk < K_MATRICES; kk++) {
            let a = coopLoadT<coop_mat16x16<f16, A>>(&a_tile[strip_offset * ROW_STRIDE + kk * SIDE], ROW_STRIDE);
            let b0 = coopLoad<coop_mat16x16<f16, B>>(&b_tile[kk * SIDE], ROW_STRIDE);
            let b1 = coopLoad<coop_mat16x16<f16, B>>(&b_tile[SIDE * ROW_STRIDE + kk * SIDE], ROW_STRIDE);
            let b2 = coopLoad<coop_mat16x16<f16, B>>(&b_tile[2u * SIDE * ROW_STRIDE + kk * SIDE], ROW_STRIDE);
            sums0 = coopMultiplyAdd(a, b0, sums0);
            sums1 = coopMultiplyAdd(a, b1, sums1);
            sums2 = coopMultiplyAdd(a, b2, sums2);
        }
        workgroupBarrier();
    }
    let store_offset = select(TILE_M, strip_offset, subgroup < STRIPS);
    coopStore(sums0, &c_tile[store_offset], C_STRIDE);
    coopStore(sums1, &c_tile[SIDE * C_STRIDE + store_offset], C_STRIDE);
    coopStore(sums2, &c_tile[2u * SIDE * C_STRIDE + store_offset], C_STRIDE);
    workgroupBarrier();

    // Each output is read (its residual) and written by one thread, so the attention can overwrite its residual.
    for (var index = thread; index < TILE_M * TILE_N; index += THREADS) {
        let m = index % TILE_M;
        let channel = index / TILE_M;
        let pixel = m0 + m;
        if (pixel < conv.pixels) {
            var residual = 0.0;
            if (EPILOGUE == EPILOGUE_ATTENTION) {
                residual = arena[conv.residual_base + channel * conv.pixels + pixel];
            }
            let sum = c_tile[channel * C_STRIDE + m];
            arena[conv.out_base + channel * conv.pixels + pixel] = finish(sum, channel, residual);
        }
    }
}
