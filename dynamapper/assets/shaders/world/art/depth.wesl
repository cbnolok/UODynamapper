import bevy_pbr::render::mesh_view_bindings::view;

// Equal-priority sprites retain their established draw-order separation.
// A fixed NDC offset is safe only for the orthographic lens: in perspective
// it grows quadratically in world distance and can reorder neighboring cells.
const STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON: f32 = 0.000001;
// The startup orthographic depth range is 2000 world units, so its existing
// epsilon corresponds to this small displacement away from the camera.
const STATIC_DEPTH_TIE_BREAK_VIEW_STEP: f32 = 0.002;

fn static_priority_depth(clip_position: vec4<f32>, clip_from_view: mat4x4<f32>, ordinal: u32) -> f32 {
    if (clip_from_view[3][3] == 1.0) {
        let depth = clamp(clip_position.z / clip_position.w, 0.0, 1.0);
        return clamp(depth - f32(ordinal) * STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON, 0.0, 1.0);
    }
    // Apply only to the priority point, never to sprite placement or logical XYZ.
    let offset = clip_from_view * vec4<f32>(0.0, 0.0,
        -f32(ordinal) * STATIC_DEPTH_TIE_BREAK_VIEW_STEP, 0.0);
    let biased = clip_position + offset;
    return clamp(biased.z / biased.w, 0.0, 1.0);
}

// Below this, the view direction is too close to the horizontal plane to solve for its depth.
const HORIZONTAL_PLANE_MIN_VIEW_Y: f32 = 0.000001;

// Depth of the horizontal world plane y = plane_y at a fragment, solved from the fragment's pixel
// centre under the orthographic camera. Coplanar quads then write bit-identical depth at a pixel
// they share, so their one-pixel overlaps go to draw order. Rasterised depth differs between the two
// quads by rounding that depends on where the camera is. Perspective keeps the rasterised depth.
fn horizontal_plane_frag_depth(frag_position: vec4<f32>, plane_y: f32) -> f32 {
    let world_from_view = view.world_from_view;
    if (view.clip_from_view[3][3] != 1.0 || abs(world_from_view[2].y) < HORIZONTAL_PLANE_MIN_VIEW_Y) {
        return frag_position.z;
    }
    let ndc = vec2<f32>(
        (frag_position.x - view.viewport.x) / view.viewport.z * 2.0 - 1.0,
        1.0 - (frag_position.y - view.viewport.y) / view.viewport.w * 2.0,
    );
    let view_xy = (ndc - view.clip_from_view[3].xy) / vec2<f32>(view.clip_from_view[0].x, view.clip_from_view[1].y);
    let view_z = (plane_y - world_from_view[3].y - world_from_view[0].y * view_xy.x - world_from_view[1].y * view_xy.y)
        / world_from_view[2].y;
    return clamp(view.clip_from_view[2].z * view_z + view.clip_from_view[3].z, 0.0, 1.0);
}

// The depth step that decides a shared pixel: one unit in the last place, the smallest change of the depth.
const PAINTER_ORDER_DEPTH_STEP_BITS: u32 = 1u;

// `horizontal_plane_frag_depth` with the clients' painter order on the pixels that coplanar quads of neighbouring
// cells share. The clients paint the cells in map order, a larger x + y later, so the cell nearer the viewer (down
// the screen, toward +x and +z) covers the shared pixel. Draw order alone decided it only inside one chunk batch:
// between two batches the winner followed their submission order. A fragment in the near half of its cell, toward
// +x + z from the cell centre, sits one depth step farther (reverse Z, so lower). On an edge shared by two cells the
// far cell's fragment is in that half and the near cell's is not, so the near cell wins in either draw order. Two
// quads of one cell take the same step at a pixel and keep their draw order. Perspective keeps the rasterised depth.
fn horizontal_plane_painter_depth(frag_position: vec4<f32>, plane_y: f32, frag_world_xz: vec2<f32>,
    cell_center_xz: vec2<f32>) -> f32 {
    let depth = horizontal_plane_frag_depth(frag_position, plane_y);
    let toward_viewer = (frag_world_xz.x - cell_center_xz.x) + (frag_world_xz.y - cell_center_xz.y);
    if (view.clip_from_view[3][3] != 1.0 || toward_viewer <= 0.0 || depth <= 0.0) {
        return depth;
    }
    return bitcast<f32>(bitcast<u32>(depth) - PAINTER_ORDER_DEPTH_STEP_BITS);
}
