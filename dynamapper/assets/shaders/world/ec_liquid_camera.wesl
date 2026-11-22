import bevy_pbr::render::{mesh_view_bindings::view, view_transformations};
import package::shaders::world::ec_liquid::{ec_liquid_original_view, ec_liquid_reflection_uv};

const LIQUID_REFLECTION_FADE_START_ZOOM: f32 = 10.0;
const LIQUID_REFLECTION_FADE_END_ZOOM: f32 = 20.0;

fn ec_liquid_camera_reflection_uv(world_position: vec3<f32>, camera_distance: f32, zoom: f32) -> vec2<f32> {
    // The fixed-distance reflection stretches into spokes across a facet. At that
    // scale use a constant reflection coordinate; the liquid still samples its
    // world-space bump texture, and techniques with explicit UVs bypass this input.
    if (zoom >= LIQUID_REFLECTION_FADE_END_ZOOM) {
        return vec2<f32>(0.0);
    }
    let weight = 1.0 - smoothstep(LIQUID_REFLECTION_FADE_START_ZOOM, LIQUID_REFLECTION_FADE_END_ZOOM, zoom);
    let backward = view.world_from_view[2].xyz;
    // The original viewing basis belongs to the default isometric view. Keep
    // perspective and deliberately rotated free cameras responsive to their view.
    if (view.clip_from_view[3][3] != 1.0 ||
        dot(backward, vec3<f32>(0.577350269)) < 0.999999 || camera_distance <= 0.0) {
        return ec_liquid_reflection_uv(view_transformations::position_world_to_view(world_position)) * weight;
    }
    // The camera already publishes its per-frame transform. Recover its focus
    // without updating/re-extracting shared material assets on every movement.
    let focus = view.world_position - backward * camera_distance;
    return ec_liquid_reflection_uv(ec_liquid_original_view(world_position - focus)) * weight;
}
