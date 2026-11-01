import bevy_pbr::render::mesh_view_bindings::view;

// Keep the world anchor separate from the small mesh/sprite offset
// until after subtracting the camera. A precombined world-to-clip matrix loses
// low bits when it cancels large map translations, perturbing interpolation.
// The geometry, camera projection (including jitter) and logical XYZ are unchanged.
fn camera_relative_world_to_clip(origin: vec3<f32>, offset: vec3<f32>) -> vec4<f32> {
    let relative = (origin - view.world_from_view[3].xyz) + offset;
    let rotation = mat3x3<f32>(view.view_from_world[0].xyz, view.view_from_world[1].xyz, view.view_from_world[2].xyz);
    return view.clip_from_view * vec4<f32>(rotation * relative, 1.0);
}
