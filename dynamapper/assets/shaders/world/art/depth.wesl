import bevy_pbr::render::mesh_view_bindings::view;

// ============================================================================
// The draw order: ClassicUO's (decision of 2026-11-16, pending the user's review), one rule for every style and both
// cameras. Every fragment that writes depth (land, flat quads, upright sprites, the KR client shadows, the projected
// shadows, the light scene and the pick pass) takes the depth of one logical point:
//
//   L = row + (priority z + tie level / PRIORITY_TIE_LEVELS_PER_Z) / PRIORITY_Z_STEPS_PER_ROW
//
// row is the diagonal x + y of the point: a sprite's anchor (its cell + 2: CC art anchors at (1, 1), EC art at
// (0.5, 1.5)); for the land, a flat quad or a ground art, its cell's anchor (CELL_DEPTH_ANCHOR), the same for every
// fragment of the cell, as ClassicUO draws a cell's land and statics as whole objects in its row. The priority z is
// ClassicUO's (statics_collect.rs resolve_priority_z_units; the land's average z - 2). The whole priority z range fits
// in one row, so the nearer row always wins, whatever the heights; within a row the priority z decides, then the tie
// level. The point is the cell at a logical
// height of the priority term in world units: the isometric view runs along (1, 1, 1), so its view z is L / sqrt 3
// from the camera, and the active camera's projection maps it to the depth, orthographic or perspective alike.
// statics_collect.rs `logical_draw_distance` is the same function on the CPU (depth_tests.rs checks the two agree).
// ============================================================================

// Priority z steps in one diagonal row: more than the priority z range (UO z -128..127, ClassicUO's -2 for land and
// up to +2 for statics), so no height crosses a row. The KR client weighs a row as 35.4 z instead; this is the one
// parameter to change for that.
const PRIORITY_Z_STEPS_PER_ROW: f32 = 264.0;
// Tie levels in one priority z: level 0 for land, flat quads and the ground art, the KR client shadow one level
// behind its caster, and 1..8 for upright sprites (statics_collect.rs assign_sprite_tie_levels). One level is about
// 2 units in the last place of the orthographic depth, the least that survives its rounding.
const PRIORITY_TIE_LEVELS_PER_Z: f32 = 9.0;
// ClassicUO's land priority: z - 1, and - 1 again when the land is added to the cell's list (Map.cs AddGameObject).
const LAND_PRIORITY_Z_OFFSET: f32 = -2.0;
// A cell's anchor from its corner: row + 2, the CC art's anchor (statics_collect.rs CELL_DEPTH_ANCHOR_OFFSET). The land
// and the projected shadows take it here (cell_priority_view_z); the flat quads and the ground art carry it as their
// instance's tile_x, tile_y.
const CELL_DEPTH_ANCHOR: vec2<f32> = vec2<f32>(1.0, 1.0);

// The logical height, in world units, of a priority z and a tie level.
fn priority_logical_height(priority_z: f32, tie_level: f32) -> f32 {
    return (priority_z + tie_level / PRIORITY_TIE_LEVELS_PER_Z) / PRIORITY_Z_STEPS_PER_ROW;
}

// The view z of the logical point at world `world_xz` and logical height `logical_height` from a camera at
// `camera_position` whose view z axis is `view_z_axis` in the world. Camera-relative, so a far map position keeps the
// tie levels: one is about 4e-4 world units, a unit in the last place at x 4096.
fn logical_view_z_from(world_xz: vec2<f32>, logical_height: f32, camera_position: vec3<f32>,
    view_z_axis: vec3<f32>) -> f32 {
    let relative = vec3<f32>(world_xz.x - camera_position.x, logical_height - camera_position.y,
        world_xz.y - camera_position.z);
    return dot(view_z_axis, relative);
}

// `logical_view_z_from` for the active camera.
fn logical_view_z(world_xz: vec2<f32>, logical_height: f32) -> f32 {
    return logical_view_z_from(world_xz, logical_height, view.world_from_view[3].xyz,
        vec3<f32>(view.view_from_world[0].z, view.view_from_world[1].z, view.view_from_world[2].z));
}

// The depth of view z `view_z` through the active camera's projection, orthographic or perspective.
fn view_z_depth(view_z: f32) -> f32 {
    return projected_view_z_depth(view_z, view.clip_from_view);
}

// The depth of view z `view_z` through `projection`.
fn projected_view_z_depth(view_z: f32, projection: mat4x4<f32>) -> f32 {
    let clip_w = projection[2].w * view_z + projection[3].w;
    if (clip_w <= 0.0) {
        return 1.0;
    }
    return clamp((projection[2].z * view_z + projection[3].z) / clip_w, 0.0, 1.0);
}

// The view z of the depth `depth`: view_z_depth inverted.
fn depth_view_z(depth: f32) -> f32 {
    let projection = view.clip_from_view;
    return (projection[3].z - depth * projection[3].w) / (depth * projection[2].w - projection[2].z);
}

// An upright sprite's depth: its anchor `tile_xy` at its priority z and tie level, constant over the card. A flat quad's
// or a ground art's too, at its cell's anchor and tie level 0, constant over its cell.
fn sprite_priority_view_z(tile_xy: vec2<f32>, priority_z: f32, tie_level: f32) -> f32 {
    return logical_view_z(tile_xy, priority_logical_height(priority_z, tie_level));
}

// The ground strip of a mobile's card that a fragment at `local_y` on it falls in (MobileGroundStrips,
// render/mobiles/draw.rs): 0 above `cut_local_y`, then 1 to `strips`, each `strip_height` tall, the last one taking
// everything below. The fragment is ordered that many cells nearer in each map axis, two rows a strip, as the
// Enhanced client draws the part of a mobile below its anchor: the land of the rows in front, which lies behind those
// pixels, would otherwise cover them. No strips (0) keeps the whole card at its anchor.
fn mobile_ground_strip(local_y: f32, cut_local_y: f32, strip_height: f32, strips: u32) -> f32 {
    if (strips == 0u) {
        return 0.0;
    }
    return clamp(ceil((cut_local_y - local_y) / strip_height), 0.0, f32(strips));
}

// The view z of a fragment at world `world_xz` of a surface lying over its cell at `priority_z`, as the land and the
// projected shadows: its cell's anchor at tie level 0, one depth for the whole cell.
fn cell_priority_view_z(world_xz: vec2<f32>, priority_z: f32) -> f32 {
    return logical_view_z(floor(world_xz) + CELL_DEPTH_ANCHOR, priority_logical_height(priority_z, 0.0));
}

// The rows by which a cap fragment of a wall moves back in the perspective camera. The fragment is at `card_point` on
// its upright card, whose anchor lies in row `anchor_row`, and the wall's top is at `top_world_y`.
//
// The card is drawn for the isometric view. A cap pixel's line of sight there meets the top's plane inside the
// wall's own cell, so the floor laid on that top, in the farther row behind it, never reaches the pixel. A
// perspective ray diverges from the isometric one. Off the screen's centre it can meet the plane up to a pixel or
// two past the cell's far edge, in the farther row. The cap is then ordered in that row, where its pixel shows, just
// under its top (perspective_cap_priority_z): the floor laid on the top covers it as in the isometric view, and what
// stands below the top in that row, the wall's own neighbours among them, stays under it as before; a static rising
// over the top there, as over a floor, covers it. Without this, a hairline of the wall's top shows along the edge of
// the floor laid on it. Only walls move their caps: their art is a slab whose top fills the cell, where a solid
// static's (a tree's trunk, a boulder) is no flat top.
//
// Of those, only the walls with a floor on their top in the farther row move their caps (STATIC_FLAG_CAP_MEETS_FLOOR,
// set on the CPU). A wall standing on the top there, or a roof over it, rises over the moved cap's z and would cover
// it, as would the lower half of a wall stacked on its own, whose art reaches over the upper half's cap: dashes along
// the eaves and the palisades' tops.
//
// The isometric line is the camera's own view axis, so an orthographic camera never moves a cap. The shift applies
// only to fragments above the top whose isometric line meets the plane in the cell's row: art drawn above its height
// (a bookcase's crest) keeps its anchor. A cap the perspective pulls nearer keeps its row too: the nearer row's surfaces
// cover it as in the isometric view.
fn perspective_cap_row_shift(card_point: vec3<f32>, top_world_y: f32, anchor_row: f32) -> f32 {
    let view_axis = -view.world_from_view[2].xyz;
    let rise = card_point.y - top_world_y;
    if (rise <= 0.0 || view.clip_from_view[3][3] == 1.0 || view_axis.y >= 0.0) {
        return 0.0;
    }
    let sight = card_point - view.world_from_view[3].xyz;
    if (sight.y >= 0.0) {
        return 0.0;
    }
    let isometric_hit = card_point - view_axis * (rise / view_axis.y);
    let perspective_hit = card_point - sight * (rise / sight.y);
    let isometric_row = floor(isometric_hit.x) + floor(isometric_hit.z);
    if (isometric_row < anchor_row - (CELL_DEPTH_ANCHOR.x + CELL_DEPTH_ANCHOR.y)) {
        return 0.0;
    }
    return min(floor(perspective_hit.x) + floor(perspective_hit.z) - isometric_row, 0.0);
}

// SpriteInstance is_wet_flags bit: the static is a wall with a floor laid on its top in the farther row, whose cap
// the perspective camera orders in that row (perspective_cap_row_shift). Keep in sync with STATIC_FLAG_CAP_MEETS_FLOOR
// in statics_collect.rs.
const STATIC_FLAG_CAP_MEETS_FLOOR: u32 = 1u << 24u;

// Whether a static's cap may move back a row in the perspective camera (perspective_cap_row_shift), by its
// is_wet_flags.
fn perspective_cap_may_move(is_wet_flags: u32) -> bool {
    return (is_wet_flags & STATIC_FLAG_CAP_MEETS_FLOOR) != 0u;
}

// How far under its top's z a cap moved back a row (perspective_cap_row_shift) is ordered: a floor laid on the top
// takes its z, or its z - 1 as a Background (ClassicUO's priority z), and covers the cap; the statics standing below
// the top in that row, the wall's neighbours at their z + 1 among them, stay under it.
const PERSPECTIVE_CAP_PRIORITY_Z_BELOW_TOP: f32 = 2.0;
// A moved cap's tie level: the highest, over the other statics of its priority z in that row.
const PERSPECTIVE_CAP_TIE_LEVEL: f32 = PRIORITY_TIE_LEVELS_PER_Z - 1.0;

// The priority z of a cap moved back a row, of a static of priority z `priority_z` whose top is at z `top_z`.
fn perspective_cap_priority_z(priority_z: f32, top_z: f32) -> f32 {
    return max(priority_z, top_z - PERSPECTIVE_CAP_PRIORITY_Z_BELOW_TOP);
}

// SpriteInstance and GroundTileInstance is_wet_flags bit: the static is part of a cave drawn over the map (caves.rs).
// Keep in sync with STATIC_FLAG_CAVE in statics_collect.rs.
const STATIC_FLAG_CAVE: u32 = 1u << 12u;
// How high the land and statics in front of a cave may rise over it, in world units: the whole UO z range, 256 z units
// (UO_Z_TO_WORLD_SCALE = 4 / 44). The logical points sit lower than the real ones (the whole z range spans one row),
// so the lift covers more than they need.
const CAVE_LIFT_WORLD_HEIGHT: f32 = 256.0 * 4.0 / 44.0;
// The least rise of the view direction the lift is solved for, so a camera near the horizon keeps a finite lift.
const CAVE_LIFT_MIN_VIEW_RISE: f32 = 0.1;

// The view z `view_z` of a cave static's fragment moved toward the camera along the view ray past anything in front
// of the cave: the land and statics in front rise at most CAVE_LIFT_WORLD_HEIGHT over it, which is that height
// divided by the view direction's rise along the ray. A cave drawn over the map then shows through the land around
// it, while its own statics keep their order. Other fragments keep `view_z`. In view z, so it holds for the
// orthographic and the perspective camera.
fn cave_lifted_view_z(view_z: f32, is_wet_flags: u32) -> f32 {
    if ((is_wet_flags & STATIC_FLAG_CAVE) == 0u) {
        return view_z;
    }
    return view_z + cave_lift();
}

// How far toward the camera a cave's fragments move, in view-space z (cave_lifted_view_z).
fn cave_lift() -> f32 {
    return CAVE_LIFT_WORLD_HEIGHT / max(abs(view.world_from_view[2].y), CAVE_LIFT_MIN_VIEW_RISE);
}

// SpriteInstance and GroundTileInstance is_wet_flags bit: the static is outside the player's cave, drawn darkened
// (caves.rs CaveSurroundings::Dimmed). Keep in sync with STATIC_FLAG_CAVE_OUTSIDE in statics_collect.rs.
const STATIC_FLAG_CAVE_OUTSIDE: u32 = 1u << 15u;
// How bright what is outside the player's cave draws, and how much of its colour it keeps, the rest going to its
// luminance: it reads as outside, not as dusk. The one place they are set: land (land/main.wesl), statics and water all
// take this tone, and the lights (art/light_clip.wesl) its colour, the light buffer multiplying the darkened scene
// (caves.rs CaveSurroundings::Dimmed).
const CAVE_OUTSIDE_SHADE: f32 = 0.3;
const CAVE_OUTSIDE_SATURATION: f32 = 0.6;
// The luminance weights of linear sRGB (Rec. 709).
const CAVE_OUTSIDE_LUMA_WEIGHTS: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// The colour `rgb` with only CAVE_OUTSIDE_SATURATION of its colour left, as what is outside the player's cave keeps.
fn cave_outside_desaturated(rgb: vec3<f32>) -> vec3<f32> {
    return mix(vec3<f32>(dot(rgb, CAVE_OUTSIDE_LUMA_WEIGHTS)), rgb, CAVE_OUTSIDE_SATURATION);
}

// The colour `rgb` as it draws outside the player's cave: darkened and partly desaturated, statics and land alike.
fn cave_outside_tone(rgb: vec3<f32>) -> vec3<f32> {
    return cave_outside_desaturated(rgb) * CAVE_OUTSIDE_SHADE;
}

// The final colour `rgb` of a static, darkened and partly desaturated when it is outside the player's cave.
fn cave_outside_rgb(rgb: vec3<f32>, is_wet_flags: u32) -> vec3<f32> {
    return select(rgb, cave_outside_tone(rgb), (is_wet_flags & STATIC_FLAG_CAVE_OUTSIDE) != 0u);
}
