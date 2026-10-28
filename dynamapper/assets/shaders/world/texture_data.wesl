// Recover encoded RGB from the sRGB art atlas when an authored material stage
// operates in the original fixed-function color domain.
// Alpha is not sRGB-decoded by the GPU and needs no conversion.
fn texture_data_rgb(sampled_rgb: vec3<f32>) -> vec3<f32> {
    let rgb = max(sampled_rgb, vec3<f32>(0.0));
    return select(rgb * 12.92, 1.055 * pow(rgb, vec3<f32>(1.0 / 2.4)) - 0.055, rgb > vec3<f32>(0.0031308));
}

// EC page/auxiliary atlases retain encoded RGB in UNORM. Convert a completed
// material color once before Bevy's linear lighting/post-processing pipeline.
fn texture_color_rgb(encoded_rgb: vec3<f32>) -> vec3<f32> {
    let rgb = max(encoded_rgb, vec3<f32>(0.0));
    return select(rgb / 12.92, pow((rgb + 0.055) / 1.055, vec3<f32>(2.4)), rgb > vec3<f32>(0.04045));
}
