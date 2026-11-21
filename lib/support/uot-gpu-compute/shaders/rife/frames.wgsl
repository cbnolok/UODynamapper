// The steps between a texture of frames and a RIFE workspace (src/rife/frames.rs), one thread per pixel: two frames
// into the network's input, and a frame or an in-between into output rows of RGBA8, which a copy then writes into a
// texture (an sRGB texture takes no storage writes). The frames are a 2D array texture of straight alpha, its colour
// loaded linear (sRGB); the network works on the sRGB-encoded values, as the PyTorch model does on 8-bit images.

const THREADS: u32 = 64u;
// An image of the input: RGB over 50% grey (the only channels the network looks at), premultiplied RGB, alpha.
const IMAGE_CHANNELS: u32 = 7u;
const PREMULTIPLIED: u32 = 3u;
const ALPHA: u32 = 6u;
const BACKGROUND_GREY: f32 = 0.5;
// A snapped in-between's alpha is 0 or 1, as one-bit-alpha frames' is (the Classic client's): opaque from this
// coverage up.
const SNAP_ALPHA: f32 = 0.5;
// sRGB's encoding (IEC 61966-2-1).
const SRGB_LINEAR_LIMIT: f32 = 0.0031308;
const SRGB_LINEAR_SLOPE: f32 = 12.92;
const SRGB_SCALE: f32 = 1.055;
const SRGB_OFFSET: f32 = 0.055;
const SRGB_EXPONENT: f32 = 1.0 / 2.4;

// A step's frames and card. A frame is its rectangle on its layer of the source and its place on the card; the card
// lies at the top-left corner of the workspace's padded planes.
struct FrameStep {
    width: u32,
    height: u32,
    card_width: u32,
    card_height: u32,
    first_layer: u32,
    first_x: u32,
    first_y: u32,
    first_width: u32,
    first_height: u32,
    first_offset_x: u32,
    first_offset_y: u32,
    next_layer: u32,
    next_x: u32,
    next_y: u32,
    next_width: u32,
    next_height: u32,
    next_offset_x: u32,
    next_offset_y: u32,
    // u32 per row of the output.
    pitch: u32,
    snap_alpha: u32,
}

@group(0) @binding(0) var<uniform> frame_step: FrameStep;
@group(0) @binding(1) var source: texture_2d_array<f32>;
// The workspace's input: image 0's channels, then image 1's, planes of width by height.
@group(0) @binding(2) var<storage, read_write> images: array<f32>;
// The workspace's in-between, laid out as an image.
@group(0) @binding(3) var<storage, read> merged: array<f32>;
@group(0) @binding(4) var<storage, read_write> output: array<u32>;

fn encode_srgb(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * SRGB_LINEAR_SLOPE;
    let high = SRGB_SCALE * pow(max(linear, vec3<f32>(0.0)), vec3<f32>(SRGB_EXPONENT)) - SRGB_OFFSET;
    return select(high, low, linear <= vec3<f32>(SRGB_LINEAR_LIMIT));
}

// A frame's texel at a point of the card, sRGB-encoded with straight alpha; transparent outside the frame.
fn frame_texel(layer: u32, origin: vec2<u32>, size: vec2<u32>, offset: vec2<u32>, point: vec2<u32>) -> vec4<f32> {
    if (any(point < offset) || any(point >= offset + size)) {
        return vec4<f32>(0.0);
    }
    let texel = textureLoad(source, vec2<i32>(origin + point - offset), i32(layer), 0);
    return vec4<f32>(encode_srgb(texel.rgb), texel.a);
}

fn first_texel(point: vec2<u32>) -> vec4<f32> {
    return frame_texel(frame_step.first_layer, vec2<u32>(frame_step.first_x, frame_step.first_y),
        vec2<u32>(frame_step.first_width, frame_step.first_height),
        vec2<u32>(frame_step.first_offset_x, frame_step.first_offset_y), point);
}

fn next_texel(point: vec2<u32>) -> vec4<f32> {
    return frame_texel(frame_step.next_layer, vec2<u32>(frame_step.next_x, frame_step.next_y),
        vec2<u32>(frame_step.next_width, frame_step.next_height),
        vec2<u32>(frame_step.next_offset_x, frame_step.next_offset_y), point);
}

fn store_image(image: u32, index: u32, texel: vec4<f32>) {
    let plane = frame_step.width * frame_step.height;
    let base = image * IMAGE_CHANNELS;
    for (var c = 0u; c < PREMULTIPLIED; c++) {
        images[(base + c) * plane + index] = texel[c] * texel.a + BACKGROUND_GREY * (1.0 - texel.a);
        images[(base + PREMULTIPLIED + c) * plane + index] = texel[c] * texel.a;
    }
    images[(base + ALPHA) * plane + index] = texel.a;
}

// The first and next frames on the card, into the input; the padding is transparent too.
@compute @workgroup_size(THREADS)
fn pack(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= frame_step.width * frame_step.height) {
        return;
    }
    let point = vec2<u32>(id.x % frame_step.width, id.x / frame_step.width);
    store_image(0u, id.x, first_texel(point));
    store_image(1u, id.x, next_texel(point));
}

// The first frame as it is, card_width by card_height at its offset, into the output rows.
@compute @workgroup_size(THREADS)
fn copy_frame(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= frame_step.card_width * frame_step.card_height) {
        return;
    }
    let point = vec2<u32>(id.x % frame_step.card_width, id.x / frame_step.card_width);
    output[point.y * frame_step.pitch + point.x] = pack4x8unorm(first_texel(point));
}

// The in-between's card, straight alpha, into the output rows; with snap_alpha, its alpha 0 or 1.
@compute @workgroup_size(THREADS)
fn store(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= frame_step.card_width * frame_step.card_height) {
        return;
    }
    let point = vec2<u32>(id.x % frame_step.card_width, id.x / frame_step.card_width);
    let plane = frame_step.width * frame_step.height;
    let index = point.y * frame_step.width + point.x;
    let coverage = clamp(merged[ALPHA * plane + index], 0.0, 1.0);
    let premultiplied = vec3<f32>(merged[PREMULTIPLIED * plane + index], merged[(PREMULTIPLIED + 1u) * plane + index],
        merged[(PREMULTIPLIED + 2u) * plane + index]);
    var alpha = coverage;
    if (frame_step.snap_alpha != 0u) {
        alpha = select(0.0, 1.0, coverage >= SNAP_ALPHA);
    }
    var color = vec3<f32>(0.0);
    if (alpha > 0.0) {
        color = clamp(premultiplied / coverage, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    output[point.y * frame_step.pitch + point.x] = pack4x8unorm(vec4<f32>(color, alpha));
}
