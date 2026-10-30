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
