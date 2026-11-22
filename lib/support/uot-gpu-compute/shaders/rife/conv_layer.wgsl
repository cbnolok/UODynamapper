// What the two kernels of the RIFE convolutions share, conv.wgsl on the shader cores and conv_matrix.wgsl on the
// matrix units: the layer's parameters and buffers, the taps a grid pixel reads, and the epilogue that stores an
// output. rife.rs puts this source after the kernel's constants and before the kernel's own source.
//
// The constants it reads, set per pipeline (see conv.wgsl):
//   TRANSPOSED, TAPS, PHASES, STRIDE, PIXEL_SHUFFLE, EPILOGUE

const EPILOGUE_LEAKY: u32 = 0u;
const EPILOGUE_LINEAR: u32 = 1u;
// leaky(conv(x) * beta + x), the ResConv of IFNet_HDv3.py.
const EPILOGUE_RESIDUAL: u32 = 2u;
const LEAKY_SLOPE: f32 = 0.2;
const KERNEL_SIDE: u32 = 3u;
const PHASE_SIDE: u32 = 2u;
const SHUFFLE_SIDE: u32 = 2u;

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

// One bit for each tap of a grid pixel that falls inside the input.
fn inside_taps(pixel: u32, phase_y: i32, phase_x: i32) -> u32 {
    let grid_y = i32(pixel / conv.grid_width);
    let grid_x = i32(pixel % conv.grid_width);
    var inside = 0u;
    for (var tap = 0u; tap < TAPS; tap++) {
        let y = tap_y(grid_y, phase_y, tap);
        let x = tap_x(grid_x, phase_x, tap);
        let tap_inside = pixel < conv.grid_pixels && y >= 0 && x >= 0 && y < i32(conv.in_height)
            && x < i32(conv.in_width);
        inside |= select(0u, 1u << tap, tap_inside);
    }
    return inside;
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

// Where a sum ends up: finished, or as a range's partial sum when the layer's sum is split.
fn store_sum(sum: f32, split: u32, phase: u32, channel: u32, pixel: u32) {
    if (conv.splits > 1u) {
        partial[partial_index(split, phase, channel, pixel)] = sum;
    } else {
        finish(sum, phase, channel, pixel);
    }
}
