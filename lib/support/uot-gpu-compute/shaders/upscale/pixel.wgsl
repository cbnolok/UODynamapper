// The SPAN steps around the convolutions, one thread per pixel: `prepare` turns a straight-alpha frame of the
// mobile atlas into the network's input, and `finish` turns the network's 4x colour into the 2x frame.
//
// The network sees colour premultiplied over 50% grey, and a transparent canvas around the frame (grey to it), so
// outlines can grow past the frame's edge. Alpha does not go through the network: `finish` upscales it bicubically,
// removes the grey matte from the colour with it, and averages 2x2 of the 4x samples (premultiplied, in linear light,
// as the GPU filters an sRGB texture) into each 2x texel.

const THREADS: u32 = 64u;
const COLOR_CHANNELS: u32 = 3u;
const MATTE: f32 = 0.5;
// The network's scale, and the 4x samples per side of a 2x texel.
const NETWORK_SCALE: u32 = 4u;
const BOX_SIDE: u32 = 2u;
// Catmull-Rom (a = -0.5), PIL's BICUBIC, over 4 taps per axis.
const CUBIC_A: f32 = -0.5;
const CUBIC_TAPS: u32 = 4u;
// Below this alpha a 4x sample's colour is meaningless (the matte removal divides by alpha) and is left out.
const MIN_ALPHA: f32 = 1.0 / 512.0;
// The atlas's 8-bit steps: the load's sRGB decode, encoded again, rounds back to the stored byte.
const BYTE_MAX: f32 = 255.0;
// Where a snapped alpha turns opaque.
const SNAP_ALPHA: f32 = 0.5;

struct Pixel {
    // The canvas the network runs on.
    width: u32,
    height: u32,
    pixels: u32,
    // The frame in the source texture, and where it starts on the canvas (both sides).
    layer: u32,
    source_x: u32,
    source_y: u32,
    frame_width: u32,
    frame_height: u32,
    border: u32,
    // Arena elements: the alpha plane, and the upsampler's first output channel.
    alpha_base: u32,
    network_base: u32,
    // The output's row pitch, in texels.
    row_texels: u32,
    // 1: the output's alpha snaps to 0 or 1 at SNAP_ALPHA (art with one bit of alpha).
    snap_alpha: u32,
}

@group(0) @binding(0) var<uniform> pixel: Pixel;
// The mobile frame atlas: RGBA8 sRGB, straight alpha, so a load returns linear colour.
@group(0) @binding(1) var source: texture_2d_array<f32>;
// The canvas's colour over grey (3 planes) and alpha, then the network's activations.
@group(0) @binding(2) var<storage, read_write> arena: array<f32>;
// The 2x frame: RGBA8, sRGB colour, straight alpha, packed as pack4x8unorm, rows `row_texels` apart.
@group(0) @binding(3) var<storage, read_write> output: array<u32>;

fn srgb_encode(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * 12.92;
    let high = 1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
}

fn srgb_decode(encoded: vec3<f32>) -> vec3<f32> {
    let low = encoded / 12.92;
    let high = pow((encoded + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, encoded <= vec3<f32>(0.04045));
}

@compute @workgroup_size(THREADS)
fn prepare(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= pixel.pixels) {
        return;
    }
    let x = i32(id.x % pixel.width) - i32(pixel.border);
    let y = i32(id.x / pixel.width) - i32(pixel.border);
    var texel = vec4<f32>(0.0);
    if (x >= 0 && y >= 0 && x < i32(pixel.frame_width) && y < i32(pixel.frame_height)) {
        let at = vec2<i32>(i32(pixel.source_x) + x, i32(pixel.source_y) + y);
        texel = textureLoad(source, at, pixel.layer, 0);
    }
    let color = round(srgb_encode(texel.rgb) * BYTE_MAX) / BYTE_MAX;
    let over_grey = color * texel.a + MATTE * (1.0 - texel.a);
    for (var channel = 0u; channel < COLOR_CHANNELS; channel++) {
        arena[channel * pixel.pixels + id.x] = over_grey[channel];
    }
    arena[pixel.alpha_base + id.x] = texel.a;
}

// The four Catmull-Rom weights of the taps before, at, after and two after a sample `t` past a source pixel.
fn cubic_weights(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(
        CUBIC_A * (t3 - 2.0 * t2 + t),
        (CUBIC_A + 2.0) * t3 - (CUBIC_A + 3.0) * t2 + 1.0,
        -(CUBIC_A + 2.0) * t3 + (2.0 * CUBIC_A + 3.0) * t2 - CUBIC_A * t,
        CUBIC_A * (t2 - t3));
}

// The canvas's alpha at a 4x sample, bicubic with pixel centres aligned; outside the canvas is transparent.
fn alpha_at(sample_x: u32, sample_y: u32) -> f32 {
    let u = (f32(sample_x) + 0.5) / f32(NETWORK_SCALE) - 0.5;
    let v = (f32(sample_y) + 0.5) / f32(NETWORK_SCALE) - 0.5;
    let first_x = i32(floor(u)) - 1;
    let first_y = i32(floor(v)) - 1;
    let weights_x = cubic_weights(u - floor(u));
    let weights_y = cubic_weights(v - floor(v));
    var sum = 0.0;
    for (var row = 0u; row < CUBIC_TAPS; row++) {
        let y = first_y + i32(row);
        if (y < 0 || y >= i32(pixel.height)) {
            continue;
        }
        for (var column = 0u; column < CUBIC_TAPS; column++) {
            let x = first_x + i32(column);
            if (x < 0 || x >= i32(pixel.width)) {
                continue;
            }
            sum += weights_y[row] * weights_x[column] * arena[pixel.alpha_base + u32(y) * pixel.width + u32(x)];
        }
    }
    return clamp(sum, 0.0, 1.0);
}

@compute @workgroup_size(THREADS)
fn finish(@builtin(global_invocation_id) id: vec3<u32>) {
    let out_width = pixel.width * NETWORK_SCALE / BOX_SIDE;
    let out_height = pixel.height * NETWORK_SCALE / BOX_SIDE;
    if (id.x >= out_width * out_height) {
        return;
    }
    let out_x = id.x % out_width;
    let out_y = id.x / out_width;
    var premultiplied = vec3<f32>(0.0);
    var coverage = 0.0;
    for (var dy = 0u; dy < BOX_SIDE; dy++) {
        for (var dx = 0u; dx < BOX_SIDE; dx++) {
            let sample_x = out_x * BOX_SIDE + dx;
            let sample_y = out_y * BOX_SIDE + dy;
            let alpha = alpha_at(sample_x, sample_y);
            if (alpha <= MIN_ALPHA) {
                continue;
            }
            // PixelShuffle(4): channel c of the 4x frame is the network's channels c * 16 + sub-row * 4 + sub-column.
            let sub = (sample_y % NETWORK_SCALE) * NETWORK_SCALE + sample_x % NETWORK_SCALE;
            let at = (sample_y / NETWORK_SCALE) * pixel.width + sample_x / NETWORK_SCALE;
            var over_grey: vec3<f32>;
            for (var channel = 0u; channel < COLOR_CHANNELS; channel++) {
                let plane = channel * NETWORK_SCALE * NETWORK_SCALE + sub;
                over_grey[channel] = arena[pixel.network_base + plane * pixel.pixels + at];
            }
            let color = clamp((over_grey - MATTE * (1.0 - alpha)) / alpha, vec3<f32>(0.0), vec3<f32>(1.0));
            premultiplied += srgb_decode(color) * alpha;
            coverage += alpha;
        }
    }
    var color = vec3<f32>(0.0);
    if (coverage > 0.0) {
        color = srgb_encode(premultiplied / coverage);
    }
    var alpha = coverage / f32(BOX_SIDE * BOX_SIDE);
    if (pixel.snap_alpha != 0u) {
        alpha = select(0.0, 1.0, alpha >= SNAP_ALPHA);
    }
    output[out_y * pixel.row_texels + out_x] = pack4x8unorm(vec4<f32>(color, alpha));
}
