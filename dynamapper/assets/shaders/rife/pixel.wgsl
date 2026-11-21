// The RIFE steps around the convolutions, one thread per pixel: an IFBlock's input (the warped frames and features,
// resized to the block's scale), the block's output resized back to the frame and added to the flow, and the final
// blend. Buffers are channel planes of f32, frame-sized unless named block-sized.

const THREADS: u32 = 64u;
// An image: RGB over 50% grey (the only channels the network looks at), premultiplied RGB, alpha.
const IMAGE_CHANNELS: u32 = 7u;
const IMAGE_COLOR_CHANNELS: u32 = 3u;
// The Head encoding of each image.
const FEATURE_CHANNELS: u32 = 4u;
// What a block hands on: flow t->0 xy and t->1 xy, the mask logit, 8 feature channels.
const FLOW_CHANNELS: u32 = 4u;
const STATE_MASK: u32 = 4u;
const STATE_FEATURES: u32 = 5u;
const STATE_FEATURE_CHANNELS: u32 = 8u;
const STATE_CHANNELS: u32 = 13u;
const CORNERS: u32 = 4u;

struct Pixel {
    width: u32,
    height: u32,
    block_width: u32,
    block_height: u32,
    scale: f32,
    timestep: f32,
    // Nonzero for block 0, which starts the flow and sees the frames unwarped.
    first_block: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> pixel: Pixel;
// Image 0's channels, then image 1's.
@group(0) @binding(1) var<storage, read> images: array<f32>;
// f0's channels, then f1's.
@group(0) @binding(2) var<storage, read> features: array<f32>;
@group(0) @binding(3) var<storage, read_write> state: array<f32>;
// Block-sized: the block's input, or its last convolution's output.
@group(0) @binding(4) var<storage, read_write> block: array<f32>;
@group(0) @binding(5) var<storage, read_write> merged: array<f32>;

// The two source pixels and the second one's weight of F.interpolate (bilinear, align_corners=False) along one axis,
// `ratio` source pixels per destination pixel.
struct Taps {
    first: u32,
    second: u32,
    weight: f32,
}

fn resize_taps(destination: u32, source_size: u32, ratio: f32) -> Taps {
    let real = max(ratio * (f32(destination) + 0.5) - 0.5, 0.0);
    let first = min(u32(real), source_size - 1u);
    let second = select(first, first + 1u, first < source_size - 1u);
    return Taps(first, second, clamp(real - f32(first), 0.0, 1.0));
}

// The four frame pixels and weights of grid_sample (bilinear, border padding, align_corners=True) at a pixel moved
// by a flow in pixels.
struct Bilinear {
    indices: vec4<u32>,
    weights: vec4<f32>,
}

fn warp_taps(x: u32, y: u32, flow: vec2<f32>) -> Bilinear {
    let size = vec2<f32>(f32(pixel.width), f32(pixel.height));
    let position = clamp(vec2<f32>(f32(x), f32(y)) + flow, vec2<f32>(0.0), size - 1.0);
    let corner = floor(position);
    let fraction = position - corner;
    let x0 = u32(corner.x);
    let y0 = u32(corner.y);
    let x1 = min(x0 + 1u, pixel.width - 1u);
    let y1 = min(y0 + 1u, pixel.height - 1u);
    return Bilinear(
        vec4<u32>(y0 * pixel.width + x0, y0 * pixel.width + x1, y1 * pixel.width + x0, y1 * pixel.width + x1),
        vec4<f32>((1.0 - fraction.x) * (1.0 - fraction.y), fraction.x * (1.0 - fraction.y),
            (1.0 - fraction.x) * fraction.y, fraction.x * fraction.y));
}

fn sample_image(channel: u32, taps: Bilinear) -> f32 {
    let base = channel * pixel.width * pixel.height;
    return dot(taps.weights, vec4<f32>(images[base + taps.indices.x], images[base + taps.indices.y],
        images[base + taps.indices.z], images[base + taps.indices.w]));
}

fn sample_feature(channel: u32, taps: Bilinear) -> f32 {
    let base = channel * pixel.width * pixel.height;
    return dot(taps.weights, vec4<f32>(features[base + taps.indices.x], features[base + taps.indices.y],
        features[base + taps.indices.z], features[base + taps.indices.w]));
}

fn flow_at(index: u32) -> vec4<f32> {
    let plane = pixel.width * pixel.height;
    return vec4<f32>(state[index], state[plane + index], state[2u * plane + index], state[3u * plane + index]);
}

// Block 0 sees img0[:3], img1[:3], f0, f1 and the timestep; later blocks see the frames and features warped by the
// flow so far, the timestep, the previous mask and features, and the flow itself (divided by the scale).
const FIRST_BLOCK_CHANNELS: u32 = 15u;
const BLOCK_CHANNELS: u32 = 28u;
const BLOCK_TIMESTEP: u32 = 14u;
const BLOCK_MASK: u32 = 15u;
const BLOCK_FEATURES: u32 = 16u;
const BLOCK_FLOW: u32 = 24u;

fn add_block_source(sums: ptr<function, array<f32, BLOCK_CHANNELS>>, x: u32, y: u32, weight: f32) {
    let plane = pixel.width * pixel.height;
    let index = y * pixel.width + x;
    if (pixel.first_block != 0u) {
        for (var c = 0u; c < IMAGE_COLOR_CHANNELS; c++) {
            (*sums)[c] += weight * images[c * plane + index];
            (*sums)[IMAGE_COLOR_CHANNELS + c] += weight * images[(IMAGE_CHANNELS + c) * plane + index];
        }
        for (var c = 0u; c < 2u * FEATURE_CHANNELS; c++) {
            (*sums)[2u * IMAGE_COLOR_CHANNELS + c] += weight * features[c * plane + index];
        }
        (*sums)[BLOCK_TIMESTEP] += weight * pixel.timestep;
        return;
    }
    let flow = flow_at(index);
    let taps0 = warp_taps(x, y, flow.xy);
    let taps1 = warp_taps(x, y, flow.zw);
    for (var c = 0u; c < IMAGE_COLOR_CHANNELS; c++) {
        (*sums)[c] += weight * sample_image(c, taps0);
        (*sums)[IMAGE_COLOR_CHANNELS + c] += weight * sample_image(IMAGE_CHANNELS + c, taps1);
    }
    for (var c = 0u; c < FEATURE_CHANNELS; c++) {
        (*sums)[2u * IMAGE_COLOR_CHANNELS + c] += weight * sample_feature(c, taps0);
        (*sums)[2u * IMAGE_COLOR_CHANNELS + FEATURE_CHANNELS + c] += weight * sample_feature(FEATURE_CHANNELS + c, taps1);
    }
    (*sums)[BLOCK_TIMESTEP] += weight * pixel.timestep;
    (*sums)[BLOCK_MASK] += weight * state[STATE_MASK * plane + index];
    for (var c = 0u; c < STATE_FEATURE_CHANNELS; c++) {
        (*sums)[BLOCK_FEATURES + c] += weight * state[(STATE_FEATURES + c) * plane + index];
    }
    for (var c = 0u; c < FLOW_CHANNELS; c++) {
        (*sums)[BLOCK_FLOW + c] += weight * flow[c] / pixel.scale;
    }
}

@compute @workgroup_size(THREADS)
fn block_input(@builtin(global_invocation_id) id: vec3<u32>) {
    let block_plane = pixel.block_width * pixel.block_height;
    if (id.x >= block_plane) {
        return;
    }
    let bx = id.x % pixel.block_width;
    let by = id.x / pixel.block_width;
    let along_x = resize_taps(bx, pixel.width, pixel.scale);
    let along_y = resize_taps(by, pixel.height, pixel.scale);
    var sums: array<f32, BLOCK_CHANNELS>;
    for (var corner = 0u; corner < CORNERS; corner++) {
        let second_x = (corner & 1u) != 0u;
        let second_y = (corner & 2u) != 0u;
        let weight = select(1.0 - along_x.weight, along_x.weight, second_x)
            * select(1.0 - along_y.weight, along_y.weight, second_y);
        if (weight == 0.0) {
            continue;
        }
        add_block_source(&sums, select(along_x.first, along_x.second, second_x),
            select(along_y.first, along_y.second, second_y), weight);
    }
    let channels = select(BLOCK_CHANNELS, FIRST_BLOCK_CHANNELS, pixel.first_block != 0u);
    for (var c = 0u; c < channels; c++) {
        block[c * block_plane + id.x] = sums[c];
    }
}

// Resizes the block's output (flow, mask logit, features, block-sized) to the frame: the flow, times the scale, is
// added to the flow so far (block 0 starts it); the mask and features replace the previous block's.
@compute @workgroup_size(THREADS)
fn upsample(@builtin(global_invocation_id) id: vec3<u32>) {
    let plane = pixel.width * pixel.height;
    if (id.x >= plane) {
        return;
    }
    let x = id.x % pixel.width;
    let y = id.x / pixel.width;
    let along_x = resize_taps(x, pixel.block_width, 1.0 / pixel.scale);
    let along_y = resize_taps(y, pixel.block_height, 1.0 / pixel.scale);
    let block_plane = pixel.block_width * pixel.block_height;
    let row0 = along_y.first * pixel.block_width;
    let row1 = along_y.second * pixel.block_width;
    let weights = vec4<f32>((1.0 - along_x.weight) * (1.0 - along_y.weight), along_x.weight * (1.0 - along_y.weight),
        (1.0 - along_x.weight) * along_y.weight, along_x.weight * along_y.weight);
    for (var c = 0u; c < STATE_CHANNELS; c++) {
        let base = c * block_plane;
        var value = dot(weights, vec4<f32>(block[base + row0 + along_x.first], block[base + row0 + along_x.second],
            block[base + row1 + along_x.first], block[base + row1 + along_x.second]));
        if (c < FLOW_CHANNELS) {
            value *= pixel.scale;
            if (pixel.first_block == 0u) {
                value += state[c * plane + id.x];
            }
        }
        state[c * plane + id.x] = value;
    }
}

// warp(img0, flow t->0) * mask + warp(img1, flow t->1) * (1 - mask), all seven channels.
@compute @workgroup_size(THREADS)
fn merge(@builtin(global_invocation_id) id: vec3<u32>) {
    let plane = pixel.width * pixel.height;
    if (id.x >= plane) {
        return;
    }
    let x = id.x % pixel.width;
    let y = id.x / pixel.width;
    let flow = flow_at(id.x);
    let taps0 = warp_taps(x, y, flow.xy);
    let taps1 = warp_taps(x, y, flow.zw);
    let mask = 1.0 / (1.0 + exp(-state[STATE_MASK * plane + id.x]));
    for (var c = 0u; c < IMAGE_CHANNELS; c++) {
        merged[c * plane + id.x] = sample_image(c, taps0) * mask + sample_image(IMAGE_CHANNELS + c, taps1) * (1.0 - mask);
    }
}
