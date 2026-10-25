// Color and support maps share sRGB atlas views. Recover the authored RGB
// values before interpreting a sample as a normal, displacement, or mask.
// Alpha is not sRGB-decoded by the GPU and needs no conversion.
fn texture_data_rgb(sampled_rgb: vec3<f32>) -> vec3<f32> {
    let rgb = max(sampled_rgb, vec3<f32>(0.0));
    return select(rgb * 12.92, 1.055 * pow(rgb, vec3<f32>(1.0 / 2.4)) - 0.055, rgb > vec3<f32>(0.0031308));
}
