// ============================================================================
// art::projected_shadow — the sprite shadows cast on the ground under the sun (PASS_MODE_SHADOW in main.wesl), where
// a preset turns them on (UOD-Painterly's); KR-like draws the KR client's shadow instead (client_shadow.wesl).
//
// Every caster's shadow lies on the plane of its base and leans away from the sun: a point `height` world units
// above the base falls `height * projected_shadow_step(..)` along the ground from the point under it. The step
// follows the sun's azimuth and its elevation, scaled so that the sun the presets were tuned under
// (PROJECTED_SHADOW_REFERENCE_SUN_RUN) keeps their Length.
//
// A wall stands on its cell's south and east edges (light_scene.wesl): its card's left half shows the south face and
// its right half the east face. Its shadow is the shadow of those two faces, found per ground point: the face point
// that casts onto it, seen through the card, says whether the art covers it there (`wall_shadow_card_corner`). So the
// shadow is the faces' parallelogram along the wall, with its doors and windows, and not the art's card laid on the
// ground. The other casters lay their card on the ground (main.wesl).
// ============================================================================

import package::shaders::world::art::art_bindings::{BILLBOARD_RIGHT_XZ, FLAT_QUAD_FORWARD_XZ, DEPTH_CLASS_REGULAR,
    DEPTH_CLASS_FOLIAGE};

// The sun the presets' shadow lengths were tuned under, the default sun (BAKED_SUN_ELEVATION_DEGREES in
// terrain/mesh_material.rs, the light (-1, 2.5, -1)): sqrt(2) / 2.5 of horizontal run per unit of height.
const PROJECTED_SHADOW_REFERENCE_SUN_RUN: f32 = 0.56568542;
// Under a lower sun the shadows stop growing: at most this many times their length under the reference sun, which a
// sun about 8 degrees above the horizon reaches.
const PROJECTED_SHADOW_MAX_SUN_RUN_RATIO: f32 = 4.0;
// The least horizontal part of the light direction that gives the shadows a direction; under it the sun is overhead.
const PROJECTED_SHADOW_MIN_LIGHT_XZ: f32 = 0.001;
// The least height of the sun in the light direction the run divides by: the sun on the horizon.
const PROJECTED_SHADOW_MIN_LIGHT_Y: f32 = 0.000001;
// The least step across a face's plane that finds the face point casting onto a ground point: under it the sun shines
// along the face, whose shadow is a line.
const WALL_SHADOW_MIN_STEP_ACROSS: f32 = 0.0001;
// The least part of the camera's backward axis across the card's plane: under it the camera looks along the card.
const WALL_SHADOW_MIN_VIEW_ACROSS: f32 = 0.0001;
// A card corner no art covers: outside the card's 0..1.
const WALL_SHADOW_NO_CORNER: vec2<f32> = vec2<f32>(-1.0, -1.0);
// From a cell's corner to its far (south and east) edges, where a wall's faces stand.
const WALL_SHADOW_CELL_SIZE: f32 = 1.0;

// The EC item surfaces and camera quads (SpriteInstance flat_quad 2 and 3) cast no shadow.
const PROJECTED_SHADOW_FIRST_EC_QUAD: u32 = 2u;
// SpriteInstance client_shadow_flags: the KR client casts the static's shadow (client_shadow.wesl's
// CLIENT_SHADOW_FLAG_CASTS, static_batches.rs).
const PROJECTED_SHADOW_CLIENT_CASTS: u32 = 1u;

// Whether a static of the depth class `depth_class` (StaticDepthClass in static_placement.rs) casts a shadow: the
// statics and foliage standing up that the KR client casts a shadow for (client_shadow.wesl). So no roof, Surface,
// Rotated or NoShadow item: a roof or a flight of stairs is one surface, and the shadow of each of its tiles, on the
// plane of the tile's base, fell on the tiles beside it. Nor do the floors, the Background statics and the EC item
// surfaces, which lie on the ground, nor the overview zoom's radar dots (no client shadow flags), which the pass drew
// again where their shadow geometry lay.
fn projected_shadow_casts(depth_class: u32, flat_quad: u32, client_shadow_flags: u32) -> bool {
    return (depth_class == DEPTH_CLASS_REGULAR || depth_class == DEPTH_CLASS_FOLIAGE)
        && flat_quad < PROJECTED_SHADOW_FIRST_EC_QUAD
        && (client_shadow_flags & PROJECTED_SHADOW_CLIENT_CASTS) != 0u;
}

// The ground offset, in world units along x and z, of the shadow of a point one world unit above its base: away from
// the sun, `shadow_length` (the preset's Length) times the run of the sun's light over the reference sun's.
fn projected_shadow_step(light_direction: vec3<f32>, shadow_length: f32) -> vec2<f32> {
    let light_xz = length(light_direction.xz);
    if (light_xz < PROJECTED_SHADOW_MIN_LIGHT_XZ) {
        return vec2<f32>(0.0);
    }
    let run = light_xz / max(light_direction.y, PROJECTED_SHADOW_MIN_LIGHT_Y);
    let ratio = min(run / PROJECTED_SHADOW_REFERENCE_SUN_RUN, PROJECTED_SHADOW_MAX_SUN_RUN_RATIO);
    return -light_direction.xz / light_xz * shadow_length * ratio;
}

// The ground the shadow of the wall on the cell whose corner is `cell` may cover, `wall_height` world units tall
// under the step `step` (projected_shadow_step): (min x, min z, max x, max z). Its faces lie in the cell's square, and
// their tops cast up to `wall_height * step` from it.
fn wall_shadow_ground_box(cell: vec2<f32>, wall_height: f32, step: vec2<f32>) -> vec4<f32> {
    let reach = step * wall_height;
    return vec4<f32>(cell + min(reach, vec2<f32>(0.0)), cell + WALL_SHADOW_CELL_SIZE + max(reach, vec2<f32>(0.0)));
}

// The point of a face of the wall on `cell` whose shadow falls on the ground point `ground`: (x, z, its height above
// the base, 1), or 0 in w when no point of the face, between its base and `wall_height`, casts there. The south face
// (east_face false) stands on z = cell y + 1 from x = cell x to cell x + 1, the east face on x = cell x + 1.
fn wall_shadow_face_point(ground: vec2<f32>, cell: vec2<f32>, wall_height: f32, step: vec2<f32>, east_face: bool)
    -> vec4<f32> {
    // Across the face's plane (z for the south face, x for the east face) and along it.
    let step_across = select(step.y, step.x, east_face);
    let step_along = select(step.x, step.y, east_face);
    if (abs(step_across) < WALL_SHADOW_MIN_STEP_ACROSS) {
        return vec4<f32>(0.0);
    }
    let plane = select(cell.y, cell.x, east_face) + WALL_SHADOW_CELL_SIZE;
    let start = select(cell.x, cell.y, east_face);
    let height = (select(ground.y, ground.x, east_face) - plane) / step_across;
    let at = select(ground.x, ground.y, east_face) - height * step_along;
    if (height < 0.0 || height > wall_height || at < start || at > start + WALL_SHADOW_CELL_SIZE) {
        return vec4<f32>(0.0);
    }
    let point = select(vec2<f32>(at, plane), vec2<f32>(plane, at), east_face);
    return vec4<f32>(point, height, 1.0);
}

// The corner of the card (x 0 left to 1 right, y 0 top to 1 bottom, as main.wesl lays the billboard out) that shows
// the world point `point`: where the view line through it, along the camera's backward axis `view_back`, crosses the
// card's plane, upright through `anchor` across the view and spanning `local_min` to `local_max` (SpriteInstance).
// WALL_SHADOW_NO_CORNER when the camera looks along the card.
fn wall_shadow_card_corner(point: vec3<f32>, anchor: vec3<f32>, view_back: vec3<f32>, local_min: vec2<f32>,
    local_max: vec2<f32>) -> vec2<f32> {
    let view_across = dot(view_back.xz, FLAT_QUAD_FORWARD_XZ);
    if (abs(view_across) < WALL_SHADOW_MIN_VIEW_ACROSS) {
        return WALL_SHADOW_NO_CORNER;
    }
    let on_card = point + view_back * (-dot(point.xz - anchor.xz, FLAT_QUAD_FORWARD_XZ) / view_across);
    let local = vec2<f32>(dot(on_card.xz - anchor.xz, BILLBOARD_RIGHT_XZ), on_card.y - anchor.y);
    let fraction = (local - local_min) / (local_max - local_min);
    return vec2<f32>(fraction.x, 1.0 - fraction.y);
}
