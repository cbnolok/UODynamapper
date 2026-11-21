// The steps between the mobiles' frame atlas and a RIFE workspace (render/mobiles/rife_in_betweens/render.rs), one
// thread per pixel: two frames of a clip into the network's input, and a frame or an in-between into a staging
// buffer of RGBA8 rows, which a copy then writes into the atlas (the atlas is sRGB RGBA8, which no storage binding
// takes). The network works on the frames' sRGB-encoded values, as the PyTorch model does on 8-bit images.

const THREADS: u32 = 64u;
// An image of the input: RGB over 50% grey (the only channels the network looks at), premultiplied RGB, alpha.
const IMAGE_CHANNELS: u32 = 7u;
const PREMULTIPLIED: u32 = 3u;
const ALPHA: u32 = 6u;
const BACKGROUND_GREY: f32 = 0.5;
// A Classic in-between's alpha is 0 or 1, as the Classic frames' is: opaque from this coverage up.
const CLASSIC_ALPHA_SNAP: f32 = 0.5;
// sRGB's encoding (IEC 61966-2-1).
const SRGB_LINEAR_LIMIT: f32 = 0.0031308;
const SRGB_LINEAR_SLOPE: f32 = 12.92;
const SRGB_SCALE: f32 = 1.055;
const SRGB_OFFSET: f32 = 0.055;
const SRGB_EXPONENT: f32 = 1.0 / 2.4;

// A step's frames and card. A frame is its rectangle on its atlas page and its place on the card; the card lies at
// the top-left corner of the workspace's padded planes.
struct AtlasStep {
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
    // u32 per row of the staging buffer.
    pitch: u32,
    snap_alpha: u32,
}

@group(0) @binding(0) var<uniform> atlas_step: AtlasStep;
@group(0) @binding(1) var atlas: texture_2d_array<f32>;
// The workspace's input: image 0's channels, then image 1's, planes of width by height.
@group(0) @binding(2) var<storage, read_write> images: array<f32>;
// The workspace's in-between, laid out as an image.
@group(0) @binding(3) var<storage, read> merged: array<f32>;
@group(0) @binding(4) var<storage, read_write> staging: array<u32>;

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
    let texel = textureLoad(atlas, vec2<i32>(origin + point - offset), i32(layer), 0);
    return vec4<f32>(encode_srgb(texel.rgb), texel.a);
}

fn first_texel(point: vec2<u32>) -> vec4<f32> {
    return frame_texel(atlas_step.first_layer, vec2<u32>(atlas_step.first_x, atlas_step.first_y),
        vec2<u32>(atlas_step.first_width, atlas_step.first_height),
        vec2<u32>(atlas_step.first_offset_x, atlas_step.first_offset_y), point);
}

fn next_texel(point: vec2<u32>) -> vec4<f32> {
    return frame_texel(atlas_step.next_layer, vec2<u32>(atlas_step.next_x, atlas_step.next_y),
        vec2<u32>(atlas_step.next_width, atlas_step.next_height),
        vec2<u32>(atlas_step.next_offset_x, atlas_step.next_offset_y), point);
}

fn store_image(image: u32, index: u32, texel: vec4<f32>) {
    let plane = atlas_step.width * atlas_step.height;
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
    if (id.x >= atlas_step.width * atlas_step.height) {
        return;
    }
    let point = vec2<u32>(id.x % atlas_step.width, id.x / atlas_step.width);
    store_image(0u, id.x, first_texel(point));
    store_image(1u, id.x, next_texel(point));
}

// The first frame as it is, card_width by card_height at its offset, into the staging rows.
@compute @workgroup_size(THREADS)
fn copy_frame(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= atlas_step.card_width * atlas_step.card_height) {
        return;
    }
    let point = vec2<u32>(id.x % atlas_step.card_width, id.x / atlas_step.card_width);
    staging[point.y * atlas_step.pitch + point.x] = pack4x8unorm(first_texel(point));
}

// The in-between's card, straight alpha, into the staging rows; with snap_alpha, its alpha 0 or 1.
@compute @workgroup_size(THREADS)
fn store(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= atlas_step.card_width * atlas_step.card_height) {
        return;
    }
    let point = vec2<u32>(id.x % atlas_step.card_width, id.x / atlas_step.card_width);
    let plane = atlas_step.width * atlas_step.height;
    let index = point.y * atlas_step.width + point.x;
    let coverage = clamp(merged[ALPHA * plane + index], 0.0, 1.0);
    let premultiplied = vec3<f32>(merged[PREMULTIPLIED * plane + index], merged[(PREMULTIPLIED + 1u) * plane + index],
        merged[(PREMULTIPLIED + 2u) * plane + index]);
    var alpha = coverage;
    if (atlas_step.snap_alpha != 0u) {
        alpha = select(0.0, 1.0, coverage >= CLASSIC_ALPHA_SNAP);
    }
    var color = vec3<f32>(0.0);
    if (alpha > 0.0) {
        color = clamp(premultiplied / coverage, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    staging[point.y * atlas_step.pitch + point.x] = pack4x8unorm(vec4<f32>(color, alpha));
}
