// The RIFE convolutions of conv.wgsl on the GPU's matrix units: the same matrix product (grid pixels times output
// channels, summed over input channels times kernel taps, per transposed phase), with the multiply-adds done by
// 16x16x16 cooperative matrix products of fp16 operands into f32 sums. The kernel is SPAN's
// (shaders/upscale/conv_matrix.wgsl, which says why it is laid out as it is), with what RIFE's layers add: TILE_N
// output channels a workgroup (a multiple of 16, the layer's channels past its last tile left out), the strides and
// transposed phases of conv.wgsl, and the split sums. The patches are rounded to fp16 as they are staged, the weights
// are fp16 already; the sums, the epilogue and the buffers stay f32.
//
// A workgroup computes TILE_M grid pixels by TILE_N channels, staging TILE_K rows of the sum of the patches and
// weights in shared memory at a time, each step's read from memory while the previous step is being summed. Each
// subgroup sums one strip of 16 pixels by TILE_N channels into N_MATRICES matrices; the host makes STRIPS strips, one
// per subgroup at the largest subgroup size, and smaller subgroups past the strips sum a strip again into a spare
// strip nobody reads (the cooperative operations need control flow the whole workgroup takes).
//
// rife.rs puts the f16 and wgpu_cooperative_matrix extensions' enable directives and these constants in front of
// this source, then conv_layer.wgsl, one set per pipeline:
//   THREADS                      threads per workgroup, a multiple of TILE_M
//   STRIPS                       strips of 16 pixels per workgroup
//   TILE_K                       rows of the sum staged at a time, a multiple of 16 and of THREADS / TILE_M
//   N_MATRICES                   matrices of 16 output channels per workgroup, 1 to 4
//   TRANSPOSED, TAPS, PHASES, STRIDE, PIXEL_SHUFFLE, EPILOGUE    as in conv.wgsl
// The split reduction is conv.wgsl's `reduce`.

// The side of a cooperative matrix.
const SIDE: u32 = 16u;
const TILE_M: u32 = STRIPS * SIDE;
const TILE_N: u32 = N_MATRICES * SIDE;
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

// The staged patches as [pixel][row of the sum] and weights as [channel][row of the sum], rows ROW_STRIDE apart.
var<workgroup> a_tile: array<f16, ROW_STRIDE * TILE_M>;
var<workgroup> b_tile: array<f16, ROW_STRIDE * TILE_N>;
// The sums as [channel][pixel], C_STRIDE apart, for the epilogue.
var<workgroup> c_tile: array<f32, TILE_N * C_STRIDE>;

// Where a step's reads come from: the layer's rows of the sum this workgroup adds, its weights and output channels,
// and the pixel this thread stages with the taps of it inside the input.
struct Loads {
    k_end: u32,
    weights: u32,
    n0: u32,
    pixel_y: i32,
    pixel_x: i32,
    phase_y: i32,
    phase_x: i32,
    inside: u32,
}

// Reads step k0's patches of this thread's pixel and its share of the weights into registers, zero past the sum or
// the channels. The loads are unconditional (a skipped one reads element 0 and is replaced by zero), so the compiler
// can issue them together instead of branching around each.
fn load_step(k0: u32, thread: u32, loads: Loads, a_next: ptr<function, array<f32, A_ROWS>>,
             b_next: ptr<function, array<vec2<f32>, B_LOADS>>) {
    let in_plane = conv.in_width * conv.in_height;
    let load_k = thread / TILE_M;
    for (var row = 0u; row < A_ROWS; row++) {
        let k = k0 + load_k + row * A_K_STEP;
        let channel = k / TAPS;
        let tap = k % TAPS;
        let y = tap_y(loads.pixel_y, loads.phase_y, tap);
        let x = tap_x(loads.pixel_x, loads.phase_x, tap);
        let valid = k < loads.k_end && (loads.inside & (1u << tap)) != 0u;
        let index = conv.in_base + channel * in_plane + u32(y) * conv.in_width + u32(x);
        (*a_next)[row] = select(0.0, src[select(0u, index, valid)], valid);
    }
    for (var load = 0u; load < B_LOADS; load++) {
        let pair = thread + load * THREADS;
        let k = k0 + pair / (TILE_N / 2u);
        let n = loads.n0 + (pair % (TILE_N / 2u)) * 2u;
        let valid = pair < B_PAIRS && k < loads.k_end && n < conv.out_channels;
        let index = select(0u, (loads.weights + k * conv.out_channels + n) >> 1u, valid);
        (*b_next)[load] = select(vec2<f32>(0.0), unpack2x16float(params[index]), valid);
    }
}

@compute @workgroup_size(THREADS)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) thread: u32,
        @builtin(subgroup_id) subgroup: u32) {
    let m0 = group.x * TILE_M;
    let phase = group.z % PHASES;
    let split = group.z / PHASES;
    let k_total = conv.in_channels * TAPS;
    let k_begin = split * conv.split_steps * TILE_K;
    let load_m = thread % TILE_M;
    let load_k = thread / TILE_M;
    let load_pixel = m0 + load_m;
    var loads: Loads;
    loads.k_end = min(k_begin + conv.split_steps * TILE_K, k_total);
    loads.weights = conv.weights + phase * k_total * conv.out_channels;
    loads.n0 = group.y * TILE_N;
    loads.pixel_y = i32(load_pixel / conv.grid_width);
    loads.pixel_x = i32(load_pixel % conv.grid_width);
    loads.phase_y = i32(phase / PHASE_SIDE);
    loads.phase_x = i32(phase % PHASE_SIDE);
    loads.inside = inside_taps(load_pixel, loads.phase_y, loads.phase_x);

    var sums: array<coop_mat16x16<f32, C>, N_MATRICES>;
    for (var matrix = 0u; matrix < N_MATRICES; matrix++) {
        sums[matrix] = coop_mat16x16<f32, C>();
    }
    let strip_offset = (subgroup % STRIPS) * SIDE;
    var a_next: array<f32, A_ROWS>;
    var b_next: array<vec2<f32>, B_LOADS>;
    load_step(k_begin, thread, loads, &a_next, &b_next);
    for (var k0 = k_begin; k0 < loads.k_end; k0 += TILE_K) {
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
        load_step(k0 + TILE_K, thread, loads, &a_next, &b_next);
        for (var kk = 0u; kk < K_MATRICES; kk++) {
            let a = coopLoadT<coop_mat16x16<f16, A>>(&a_tile[strip_offset * ROW_STRIDE + kk * SIDE], ROW_STRIDE);
            for (var matrix = 0u; matrix < N_MATRICES; matrix++) {
                let b = coopLoad<coop_mat16x16<f16, B>>(&b_tile[matrix * SIDE * ROW_STRIDE + kk * SIDE], ROW_STRIDE);
                sums[matrix] = coopMultiplyAdd(a, b, sums[matrix]);
            }
        }
        workgroupBarrier();
    }
    let store_offset = select(TILE_M, strip_offset, subgroup < STRIPS);
    for (var matrix = 0u; matrix < N_MATRICES; matrix++) {
        coopStore(sums[matrix], &c_tile[matrix * SIDE * C_STRIDE + store_offset], C_STRIDE);
    }
    workgroupBarrier();

    for (var index = thread; index < TILE_M * TILE_N; index += THREADS) {
        let pixel = m0 + index % TILE_M;
        let channel = loads.n0 + index / TILE_M;
        if (pixel < conv.grid_pixels && channel < conv.out_channels) {
            store_sum(c_tile[(index / TILE_M) * C_STRIDE + index % TILE_M], split, phase, channel, pixel);
        }
    }
}
