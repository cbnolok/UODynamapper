// ============================================================================
// srgb.wesl — The sRGB transfer functions, for the clients' maths on 8-bit (sRGB-encoded) values in a linear frame.
// ============================================================================

// The sRGB transfer functions (IEC 61966-2-1), as the GPU's sRGB formats apply them.
const SRGB_LINEAR_LIMIT: f32 = 0.0031308;
const SRGB_ENCODED_LIMIT: f32 = 0.04045;
const SRGB_LINEAR_SLOPE: f32 = 12.92;
const SRGB_SCALE: f32 = 1.055;
const SRGB_OFFSET: f32 = 0.055;
const SRGB_GAMMA: f32 = 2.4;

fn encode_srgb(linear: vec3<f32>) -> vec3<f32> {
    let c = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = c * SRGB_LINEAR_SLOPE;
    let high = SRGB_SCALE * pow(c, vec3<f32>(1.0 / SRGB_GAMMA)) - SRGB_OFFSET;
    return select(high, low, c <= vec3<f32>(SRGB_LINEAR_LIMIT));
}

fn decode_srgb(encoded: vec3<f32>) -> vec3<f32> {
    let low = encoded / SRGB_LINEAR_SLOPE;
    let high = pow((encoded + SRGB_OFFSET) / SRGB_SCALE, vec3<f32>(SRGB_GAMMA));
    return select(high, low, encoded <= vec3<f32>(SRGB_ENCODED_LIMIT));
}
