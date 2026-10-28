import bevy_pbr::render::{mesh_view_bindings::view, view_transformations};
import package::shaders::world::ec_liquid::{ec_liquid_original_view, ec_liquid_reflection_uv};

fn ec_liquid_camera_reflection_uv(world_position: vec3<f32>, camera_distance: f32) -> vec2<f32> {
    let backward = view.world_from_view[2].xyz;
    // The original viewing basis belongs to the default isometric view. Keep
    // perspective and deliberately rotated free cameras responsive to their view.
    if (view.clip_from_view[3][3] != 1.0 ||
        dot(backward, vec3<f32>(0.577350269)) < 0.999999 || camera_distance <= 0.0) {
        return ec_liquid_reflection_uv(view_transformations::position_world_to_view(world_position));
    }
    // The camera already publishes its per-frame transform. Recover its focus
    // without updating/re-extracting shared material assets on every movement.
    let focus = view.world_position - backward * camera_distance;
    return ec_liquid_reflection_uv(ec_liquid_original_view(world_position - focus));
}
