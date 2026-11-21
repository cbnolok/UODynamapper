// The SPAN convolutions, as one tiled matrix product: output pixels times output channels, summed over input
// channels times kernel taps. A workgroup computes TILE_M pixels by TILE_N channels, staging TILE_K rows of the input
// patches and of the weights in shared memory; each thread keeps MICRO_M by MICRO_N sums.
//
// This is the tiled kernel of shaders/rife/conv.wgsl, copied rather than shared because SPAN needs what that file
// does not do: 1x1 kernels, the SiLU and attention epilogues, and every tensor in one buffer. Every SPAN layer keeps
// the frame's size (stride 1, zero padding of half the kernel), so the matrix rows are the frame's pixels. All
// activations live in one `arena` buffer at plane offsets, so a layer can read one part of it and write another, and
// the attention epilogue can overwrite its residual in place: each output is read and written by the one thread
// that finishes it. RIFE's split sums are left out: every SPAN layer runs at the full canvas size, and splitting
// measured slower even on the smallest frames.
//
// render/mobiles/upscale.rs puts these constants in front of this source, one set per pipeline:
//   THREADS                      threads per workgroup
//   TILE_M, TILE_N, TILE_K       the workgroup's tile
//   MICRO_M, MICRO_N             one thread's sums, for consecutive pixels and channels; MICRO_N is a multiple of 4
//   KERNEL_SIDE                  3 or 1
//   EPILOGUE                     EPILOGUE_LINEAR, EPILOGUE_SILU or EPILOGUE_ATTENTION

const EPILOGUE_LINEAR: u32 = 0u;
const EPILOGUE_SILU: u32 = 1u;
// SPAB's parameter-free attention: (conv(x) + x) * (sigmoid(conv(x)) - 0.5), x being the block's input.
const EPILOGUE_ATTENTION: u32 = 2u;
const ATTENTION_CENTER: f32 = 0.5;
const TAPS: u32 = KERNEL_SIDE * KERNEL_SIDE;
const KERNEL_PAD: i32 = i32(KERNEL_SIDE / 2u);
// A thread stages A_SLOTS pixels of the tile (TILE_M / THREADS when the tile is wider than the workgroup), and for
// each A_ROWS rows of the step, A_K_STEP apart (when the workgroup is wider than the tile).
const A_SLOTS: u32 = max(TILE_M / THREADS, 1u);
const A_K_STEP: u32 = max(THREADS / TILE_M, 1u);
const A_ROWS: u32 = TILE_K / A_K_STEP;
const A_LOADS: u32 = A_SLOTS * A_ROWS;
const B_PAIRS: u32 = TILE_K * TILE_N / 2u;
const B_LOADS: u32 = (B_PAIRS + THREADS - 1u) / THREADS;
const THREADS_M: u32 = TILE_M / MICRO_M;
const VECTOR: u32 = 4u;

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

// The staged patches as [row][pixel] and weights as [row][channel], read four at a time.
var<workgroup> a_tile: array<vec4<f32>, TILE_K * TILE_M / VECTOR>;
var<workgroup> b_tile: array<vec4<f32>, TILE_K * TILE_N / VECTOR>;

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

@compute @workgroup_size(THREADS)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) thread: u32) {
    let m0 = group.x * TILE_M;
    let n0 = group.y * TILE_N;
    let k_end = conv.in_channels * TAPS;

    // The pixels whose patches this thread stages, and which of their taps fall inside the frame (one bit each).
    let load_k = thread / TILE_M;
    var load_pixels: array<u32, A_SLOTS>;
    var inside_taps: array<u32, A_SLOTS>;
    for (var slot = 0u; slot < A_SLOTS; slot++) {
        let load_pixel = m0 + thread % TILE_M + slot * THREADS;
        let load_y = i32(load_pixel / conv.width);
        let load_x = i32(load_pixel % conv.width);
        var inside = 0u;
        for (var tap = 0u; tap < TAPS; tap++) {
            let y = load_y + i32(tap / KERNEL_SIDE) - KERNEL_PAD;
            let x = load_x + i32(tap % KERNEL_SIDE) - KERNEL_PAD;
            let tap_inside = load_pixel < conv.pixels && y >= 0 && x >= 0 && y < i32(conv.height)
                && x < i32(conv.width);
            inside |= select(0u, 1u << tap, tap_inside);
        }
        load_pixels[slot] = load_pixel;
        inside_taps[slot] = inside;
    }

    let thread_m = thread % THREADS_M;
    let thread_n = thread / THREADS_M;
    var sums: array<array<f32, MICRO_N>, MICRO_M>;

    // Each step's patches and weights are read from memory while the previous step is being summed.
    var a_next: array<f32, A_LOADS>;
    var b_next: array<vec2<f32>, B_LOADS>;
    for (var k0 = 0u; k0 < k_end + TILE_K; k0 += TILE_K) {
        if (k0 > 0u) {
            for (var slot = 0u; slot < A_SLOTS; slot++) {
                for (var row = 0u; row < A_ROWS; row++) {
                    let index = (load_k + row * A_K_STEP) * TILE_M + thread % TILE_M + slot * THREADS;
                    a_tile[index / VECTOR][index % VECTOR] = a_next[slot * A_ROWS + row];
                }
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
        // The loads are unconditional (a skipped one reads element 0 and is replaced by zero), so the compiler can
        // issue them together instead of branching around each.
        if (k0 < k_end) {
            for (var row = 0u; row < A_ROWS; row++) {
                let k = k0 + load_k + row * A_K_STEP;
                let channel = k / TAPS;
                let tap = k % TAPS;
                let offset = (i32(tap / KERNEL_SIDE) - KERNEL_PAD) * i32(conv.width) + i32(tap % KERNEL_SIDE)
                    - KERNEL_PAD;
                for (var slot = 0u; slot < A_SLOTS; slot++) {
                    let valid = k < k_end && (inside_taps[slot] & (1u << tap)) != 0u;
                    let index = conv.in_base + channel * conv.pixels + u32(i32(load_pixels[slot]) + offset);
                    a_next[slot * A_ROWS + row] = select(0.0, arena[select(0u, index, valid)], valid);
                }
            }
            for (var load = 0u; load < B_LOADS; load++) {
                let pair = thread + load * THREADS;
                let k = k0 + pair / (TILE_N / 2u);
                let n = n0 + (pair % (TILE_N / 2u)) * 2u;
                let valid = pair < B_PAIRS && k < k_end && n < conv.out_channels;
                let index = select(0u, (conv.weights + k * conv.out_channels + n) >> 1u, valid);
                b_next[load] = select(vec2<f32>(0.0), unpack2x16float(params[index]), valid);
            }
        }
        if (k0 > 0u) {
            // Shared memory indices in vectors where the tile allows, so that per row they differ from the thread's
            // first by constants the compiler folds into the loads.
            for (var kk = 0u; kk < TILE_K; kk++) {
                var a: array<f32, MICRO_M>;
                var b: array<f32, MICRO_N>;
                if (MICRO_M % VECTOR == 0u) {
                    for (var i = 0u; i < MICRO_M; i += VECTOR) {
                        let packed = a_tile[kk * (TILE_M / VECTOR) + thread_m * (MICRO_M / VECTOR) + i / VECTOR];
                        for (var v = 0u; v < VECTOR; v++) {
                            a[i + v] = packed[v];
                        }
                    }
                } else {
                    let a_index = kk * TILE_M + thread_m * MICRO_M;
                    for (var i = 0u; i < MICRO_M; i++) {
                        a[i] = a_tile[(a_index + i) / VECTOR][(a_index + i) % VECTOR];
                    }
                }
                for (var j = 0u; j < MICRO_N; j += VECTOR) {
                    let packed = b_tile[kk * (TILE_N / VECTOR) + thread_n * (MICRO_N / VECTOR) + j / VECTOR];
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

    // The residuals are all read before the first output is written: the arena's stores could alias them, so a read
    // after a store would wait for it.
    var residuals: array<array<f32, MICRO_N>, MICRO_M>;
    if (EPILOGUE == EPILOGUE_ATTENTION) {
        for (var i = 0u; i < MICRO_M; i++) {
            let pixel = m0 + thread_m * MICRO_M + i;
            for (var j = 0u; j < MICRO_N; j++) {
                let channel = n0 + thread_n * MICRO_N + j;
                let index = conv.residual_base + channel * conv.pixels + pixel;
                let valid = pixel < conv.pixels && channel < conv.out_channels;
                residuals[i][j] = select(0.0, arena[select(0u, index, valid)], valid);
            }
        }
    }
    for (var i = 0u; i < MICRO_M; i++) {
        let pixel = m0 + thread_m * MICRO_M + i;
        if (pixel >= conv.pixels) {
            continue;
        }
        for (var j = 0u; j < MICRO_N; j++) {
            let channel = n0 + thread_n * MICRO_N + j;
            if (channel >= conv.out_channels) {
                continue;
            }
            arena[conv.out_base + channel * conv.pixels + pixel] = finish(sums[i][j], channel, residuals[i][j]);
        }
    }
}

