// The RIFE convolutions, as one tiled matrix product: output pixels times output channels, summed over input
// channels times kernel taps. A workgroup computes TILE_M pixels by TILE_N channels, staging TILE_K rows of the input
// patches and of the weights in shared memory; each thread keeps MICRO_M by MICRO_N sums.
//
// A layer with few pixels and a long sum (the coarse blocks) would leave most of the GPU idle while a few workgroups
// walk the whole sum, so its sum can be split: each workgroup then adds up one range of rows into `partial`, and
// `reduce` adds the ranges and finishes the output.
//
// rife.rs puts these constants in front of this source, then conv_layer.wgsl (the parameters, buffers, taps and
// epilogue this kernel shares with conv_matrix.wgsl), one set per pipeline:
//   THREADS                      threads per workgroup
//   TILE_M, TILE_N, TILE_K       the workgroup's tile
//   MICRO_M, MICRO_N             one thread's sums, for consecutive pixels and channels; MICRO_N is a multiple of 4
//   TRANSPOSED                   a 4x4 stride 2 transposed convolution, computed as four 2x2 phases; otherwise a 3x3
//                                convolution with padding 1
//   TAPS                         kernel taps per input channel: 4 for a transposed phase, 9 otherwise
//   PHASES                       4 for a transposed convolution, 1 otherwise
//   STRIDE                       the 3x3 convolution's stride
//   PIXEL_SHUFFLE                the transposed output goes through PixelShuffle(2)
//   EPILOGUE                     EPILOGUE_LEAKY, EPILOGUE_LINEAR or EPILOGUE_RESIDUAL

const A_LOADS: u32 = TILE_K * TILE_M / THREADS;
const A_K_STEP: u32 = THREADS / TILE_M;
const B_PAIRS: u32 = TILE_K * TILE_N / 2u;
const B_LOADS: u32 = (B_PAIRS + THREADS - 1u) / THREADS;
const THREADS_M: u32 = TILE_M / MICRO_M;
const VECTOR: u32 = 4u;

// The staged patches as [row][pixel] and weights as [row][channel], read four at a time.
var<workgroup> a_tile: array<vec4<f32>, TILE_K * TILE_M / VECTOR>;
var<workgroup> b_tile: array<vec4<f32>, TILE_K * TILE_N / VECTOR>;

@compute @workgroup_size(THREADS)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) thread: u32) {
    let m0 = group.x * TILE_M;
    let n0 = group.y * TILE_N;
    let phase = group.z % PHASES;
    let split = group.z / PHASES;
    let phase_y = i32(phase / PHASE_SIDE);
    let phase_x = i32(phase % PHASE_SIDE);
    let k_total = conv.in_channels * TAPS;
    let k_begin = split * conv.split_steps * TILE_K;
    let k_end = min(k_begin + conv.split_steps * TILE_K, k_total);
    let weights = conv.weights + phase * k_total * conv.out_channels;
    let in_plane = conv.in_width * conv.in_height;

    // The pixel whose patch this thread stages, and which of its taps fall inside the input (one bit each).
    let load_m = thread % TILE_M;
    let load_k = thread / TILE_M;
    let load_pixel = m0 + load_m;
    let load_y = i32(load_pixel / conv.grid_width);
    let load_x = i32(load_pixel % conv.grid_width);
    let inside = inside_taps(load_pixel, phase_y, phase_x);

    let thread_m = thread % THREADS_M;
    let thread_n = thread / THREADS_M;
    var sums: array<array<f32, MICRO_N>, MICRO_M>;

    // Each step's patches and weights are read from memory while the previous step is being summed.
    var a_next: array<f32, A_LOADS>;
    var b_next: array<vec2<f32>, B_LOADS>;
    for (var k0 = k_begin; k0 < k_end + TILE_K; k0 += TILE_K) {
        if (k0 > k_begin) {
            for (var load = 0u; load < A_LOADS; load++) {
                let index = (load_k + load * A_K_STEP) * TILE_M + load_m;
                a_tile[index / VECTOR][index % VECTOR] = a_next[load];
            }
            for (var load = 0u; load < B_LOADS; load++) {
                let pair = thread + load * THREADS;
                if (pair < B_PAIRS) {
                    let index = (pair / (TILE_N / 2u)) * TILE_N + (pair % (TILE_N / 2u)) * 2u;
                    b_tile[index / VECTOR][index % VECTOR] = b_next[load].x;
                    b_tile[index / VECTOR][index % VECTOR + 1u] = b_next[load].y;
                }
            }
            workgroupBarrier();
        }
        if (k0 < k_end) {
            for (var load = 0u; load < A_LOADS; load++) {
                let k = k0 + load_k + load * A_K_STEP;
                let channel = k / TAPS;
                let tap = k % TAPS;
                var value = 0.0;
                if (k < k_end && (inside & (1u << tap)) != 0u) {
                    let y = u32(tap_y(load_y, phase_y, tap));
                    let x = u32(tap_x(load_x, phase_x, tap));
                    value = src[conv.in_base + channel * in_plane + y * conv.in_width + x];
                }
                a_next[load] = value;
            }
            for (var load = 0u; load < B_LOADS; load++) {
                let pair = thread + load * THREADS;
                let k = k0 + pair / (TILE_N / 2u);
                let n = n0 + (pair % (TILE_N / 2u)) * 2u;
                var value = vec2<f32>(0.0);
                if (pair < B_PAIRS && k < k_end && n < conv.out_channels) {
                    value = unpack2x16float(params[(weights + k * conv.out_channels + n) >> 1u]);
                }
                b_next[load] = value;
            }
        }
        if (k0 > k_begin) {
            for (var kk = 0u; kk < TILE_K; kk++) {
                var a: array<f32, MICRO_M>;
                var b: array<f32, MICRO_N>;
                let a_index = kk * TILE_M + thread_m * MICRO_M;
                if (MICRO_M % VECTOR == 0u) {
                    for (var i = 0u; i < MICRO_M; i += VECTOR) {
                        let packed = a_tile[(a_index + i) / VECTOR];
                        for (var v = 0u; v < VECTOR; v++) {
                            a[i + v] = packed[v];
                        }
                    }
                } else {
                    for (var i = 0u; i < MICRO_M; i++) {
                        a[i] = a_tile[(a_index + i) / VECTOR][(a_index + i) % VECTOR];
                    }
                }
                for (var j = 0u; j < MICRO_N; j += VECTOR) {
                    let packed = b_tile[(kk * TILE_N + thread_n * MICRO_N + j) / VECTOR];
                    for (var v = 0u; v < VECTOR; v++) {
                        b[j + v] = packed[v];
                    }
                }
                for (var i = 0u; i < MICRO_M; i++) {
                    for (var j = 0u; j < MICRO_N; j++) {
                        sums[i][j] = fma(a[i], b[j], sums[i][j]);
                    }
                }
            }
            workgroupBarrier();
        }
    }

    for (var i = 0u; i < MICRO_M; i++) {
        let pixel = m0 + thread_m * MICRO_M + i;
        if (pixel >= conv.grid_pixels) {
            continue;
        }
        for (var j = 0u; j < MICRO_N; j++) {
            let channel = n0 + thread_n * MICRO_N + j;
            if (channel >= conv.out_channels) {
                continue;
            }
            store_sum(sums[i][j], split, phase, channel, pixel);
        }
    }
}

// Adds a split layer's ranges, one thread per output.
@compute @workgroup_size(THREADS)
fn reduce(@builtin(global_invocation_id) id: vec3<u32>) {
    let outputs = conv.out_channels * conv.grid_pixels;
    if (id.x >= PHASES * outputs) {
        return;
    }
    let phase = id.x / outputs;
    let channel = (id.x % outputs) / conv.grid_pixels;
    let pixel = id.x % conv.grid_pixels;
    var sum = 0.0;
    for (var split = 0u; split < conv.splits; split++) {
        sum += partial[partial_index(split, phase, channel, pixel)];
    }
    finish(sum, phase, channel, pixel);
}
