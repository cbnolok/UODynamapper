// The RIFE convolutions, as one tiled matrix product: output pixels times output channels, summed over input
// channels times kernel taps. A workgroup computes TILE_M pixels by TILE_N channels, staging TILE_K rows of the input
// patches and of the weights in shared memory; each thread keeps MICRO_M by MICRO_N sums.
//
// A layer with few pixels and a long sum (the coarse blocks) would leave most of the GPU idle while a few workgroups
// walk the whole sum, so its sum can be split: each workgroup then adds up one range of rows into `partial`, and
// `reduce` adds the ranges and finishes the output.
//
// render/mobiles/rife.rs puts these constants in front of this source, one set per pipeline:
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

const EPILOGUE_LEAKY: u32 = 0u;
const EPILOGUE_LINEAR: u32 = 1u;
// leaky(conv(x) * beta + x), the ResConv of IFNet_HDv3.py.
const EPILOGUE_RESIDUAL: u32 = 2u;
const LEAKY_SLOPE: f32 = 0.2;
const KERNEL_SIDE: u32 = 3u;
const PHASE_SIDE: u32 = 2u;
const SHUFFLE_SIDE: u32 = 2u;
const A_LOADS: u32 = TILE_K * TILE_M / THREADS;
const A_K_STEP: u32 = THREADS / TILE_M;
const B_PAIRS: u32 = TILE_K * TILE_N / 2u;
const B_LOADS: u32 = (B_PAIRS + THREADS - 1u) / THREADS;
const THREADS_M: u32 = TILE_M / MICRO_M;
const VECTOR: u32 = 4u;

struct Conv {
    in_width: u32,
    in_height: u32,
    in_channels: u32,
    // First element of the input's first channel.
    in_base: u32,
    out_width: u32,
    out_height: u32,
    out_channels: u32,
    out_base: u32,
    // The pixels the matrix rows walk: the output for a 3x3 convolution, the input for a transposed phase.
    grid_width: u32,
    grid_pixels: u32,
    // Offsets in halves into the packed parameters.
    weights: u32,
    bias: u32,
    beta: u32,
    // Ranges the sum is split into (workgroup z is range * PHASES + phase), and TILE_K steps per range.
    splits: u32,
    split_steps: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> conv: Conv;
// fp16 parameters, two to a word; weights as [phase][input channel * TAPS + tap][output channel].
@group(0) @binding(1) var<storage, read> params: array<u32>;
@group(0) @binding(2) var<storage, read> src: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst: array<f32>;
// The split sums, as [range][phase][output channel][grid pixel].
@group(0) @binding(4) var<storage, read_write> partial: array<f32>;

// The staged patches as [row][pixel] and weights as [row][channel], read four at a time.
var<workgroup> a_tile: array<vec4<f32>, TILE_K * TILE_M / VECTOR>;
var<workgroup> b_tile: array<vec4<f32>, TILE_K * TILE_N / VECTOR>;

fn param(index: u32) -> f32 {
    let pair = unpack2x16float(params[index >> 1u]);
    return select(pair.x, pair.y, (index & 1u) == 1u);
}

fn leaky(value: f32) -> f32 {
    return select(value * LEAKY_SLOPE, value, value >= 0.0);
}

// The input row and column a tap reads for a grid pixel.
fn tap_y(grid_y: i32, phase_y: i32, tap: u32) -> i32 {
    if (TRANSPOSED) {
        return grid_y + phase_y - i32(tap / PHASE_SIDE);
    }
    return grid_y * i32(STRIDE) + i32(tap / KERNEL_SIDE) - 1;
}

fn tap_x(grid_x: i32, phase_x: i32, tap: u32) -> i32 {
    if (TRANSPOSED) {
        return grid_x + phase_x - i32(tap % PHASE_SIDE);
    }
    return grid_x * i32(STRIDE) + i32(tap % KERNEL_SIDE) - 1;
}

fn partial_index(split: u32, phase: u32, channel: u32, pixel: u32) -> u32 {
    return ((split * PHASES + phase) * conv.out_channels + channel) * conv.grid_pixels + pixel;
}

// Adds the bias, applies the epilogue and stores one output.
fn finish(sum: f32, phase: u32, channel: u32, pixel: u32) {
    var value = sum + param(conv.bias + channel);
    if (EPILOGUE == EPILOGUE_LEAKY) {
        value = leaky(value);
    } else if (EPILOGUE == EPILOGUE_RESIDUAL) {
        let residual = src[conv.in_base + channel * conv.in_width * conv.in_height + pixel];
        value = leaky(value * param(conv.beta + channel) + residual);
    }
    let out_plane = conv.out_width * conv.out_height;
    var index: u32;
    if (TRANSPOSED) {
        let y = (pixel / conv.grid_width) * PHASE_SIDE + phase / PHASE_SIDE;
        let x = (pixel % conv.grid_width) * PHASE_SIDE + phase % PHASE_SIDE;
        if (PIXEL_SHUFFLE) {
            let group_size = SHUFFLE_SIDE * SHUFFLE_SIDE;
            let sub = channel % group_size;
            let shuffled_y = y * SHUFFLE_SIDE + sub / SHUFFLE_SIDE;
            let shuffled_x = x * SHUFFLE_SIDE + sub % SHUFFLE_SIDE;
            index = (channel / group_size) * out_plane + shuffled_y * conv.out_width + shuffled_x;
        } else {
            index = channel * out_plane + y * conv.out_width + x;
        }
    } else {
        index = channel * out_plane + pixel;
    }
    dst[conv.out_base + index] = value;
}

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
    var inside_taps = 0u;
    for (var tap = 0u; tap < TAPS; tap++) {
        let y = tap_y(load_y, phase_y, tap);
        let x = tap_x(load_x, phase_x, tap);
        let inside = load_pixel < conv.grid_pixels && y >= 0 && x >= 0 && y < i32(conv.in_height)
            && x < i32(conv.in_width);
        inside_taps |= select(0u, 1u << tap, inside);
    }

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
                if (k < k_end && (inside_taps & (1u << tap)) != 0u) {
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
            if (conv.splits > 1u) {
                partial[partial_index(split, phase, channel, pixel)] = sums[i][j];
            } else {
                finish(sums[i][j], phase, channel, pixel);
            }
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
