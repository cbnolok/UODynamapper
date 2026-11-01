import bevy_pbr::render::mesh_view_bindings::view;
// Keep the world anchor separate from the small mesh/sprite offset
// until after subtracting the camera. A precombined world-to-clip matrix loses
// low bits when it cancels large map translations, perturbing interpolation.
// The geometry, camera projection (including jitter) and logical XYZ are unchanged.

// Screen positions are rounded to this fraction of a pixel. UO geometry lands on whole
// pixels at zoom 1, and the rasteriser snaps to its own sub-pixel grid: a vertex that
// rounding left a hair below a pixel corner would flip pixel-centre ties along shared
// edges depending on where the camera is. 1/64 px is exact in f32 at any target size.
const SCREEN_SNAP_SUBPIXELS: f32 = 64.0;

fn camera_relative_world_to_clip(origin: vec3<f32>, offset: vec3<f32>) -> vec4<f32> {
    let relative = (origin - view.world_from_view[3].xyz) + offset;
    let rotation = mat3x3<f32>(view.view_from_world[0].xyz, view.view_from_world[1].xyz, view.view_from_world[2].xyz);
    let clip = view.clip_from_view * vec4<f32>(rotation * relative, 1.0);
    // Pixels from the viewport centre, which lies on a pixel corner for even target sizes.
    let half_size = view.viewport.zw * 0.5;
    let screen = clip.xy / clip.w * half_size;
    let snapped = round(screen * SCREEN_SNAP_SUBPIXELS) / SCREEN_SNAP_SUBPIXELS;
    return vec4<f32>(snapped / half_size * clip.w, clip.zw);
}
