// The light scene of the "uod" light cover (art/light_scene.rs): for each screen pixel, the static drawn there, read
// back by the light glows (light_clip.wesl) to clip themselves. The art shaders write it in a pass of their own, under
// LIGHT_SCENE, after the opaque pass: a fragment that draws the pixel's depth writes its texel, the others discard.
// They test the depth as soon as they know it, before any texture work: most fragments of a redraw are hidden.
//
// A texel is (cell x, cell y, z + LIGHT_SCENE_Z_BIAS, code), Rgba16Uint; 0 where no static is drawn (the land, the
// mobiles), which every light lights. The z is the static's, except on a wall or a solid static: there it is the
// height of the point of its face that shows at the pixel, so a floor over a light cuts the wall it meets. The top of
// a wall or a solid static, its cap, is a floor at its top z (`light_scene_cap_texel`).

// The texel's code: LIGHT_SCENE_ART on every static, its class, and LIGHT_SCENE_LEFT on the left half of an upright
// card. Keep in sync with art/light_scene.rs.
const LIGHT_SCENE_ART: u32 = 1u;
const LIGHT_SCENE_CLASS_SHIFT: u32 = 1u;
const LIGHT_SCENE_CLASS_MASK: u32 = 3u;
// A floor, a roof, flat art or any other static: lit below LIGHT_COVER_MIN_RISE above the light.
const LIGHT_SCENE_CLASS_HEIGHT: u32 = 0u;
// A wall: its left half stands on the cell's west edge and shows its east face, its right half on the north edge
// and shows its south face.
const LIGHT_SCENE_CLASS_WALL: u32 = 1u;
// A solid static filling its cell: its left half shows its south face, its right half its east face.
const LIGHT_SCENE_CLASS_BLOCK: u32 = 2u;
const LIGHT_SCENE_LEFT: u32 = 8u;
// The static's z is stored as z + LIGHT_SCENE_Z_BIAS, so the signed UO z fits the unsigned channel.
const LIGHT_SCENE_Z_BIAS: i32 = 128;
// The largest stored z, the channel's 8 bits the light clip unpacks it from (light_clip.wesl).
const LIGHT_SCENE_Z_TOP: i32 = 255;

// SpriteInstance and GroundTileInstance is_wet_flags bits: the static's light scene class. Keep in sync with
// STATIC_FLAG_LIGHT_WALL and STATIC_FLAG_LIGHT_BLOCK in art/light_scene.rs.
const STATIC_FLAG_LIGHT_WALL: u32 = 1u << 13u;
const STATIC_FLAG_LIGHT_BLOCK: u32 = 1u << 14u;
// SpriteInstance is_wet_flags bits of a wall or a solid static: its tiledata height. Keep in sync with
// STATIC_LIGHT_HEIGHT_SHIFT and STATIC_LIGHT_HEIGHT_MASK in art/light_scene.rs.
const STATIC_LIGHT_HEIGHT_SHIFT: u32 = 16u;
const STATIC_LIGHT_HEIGHT_MASK: u32 = 0xffu;
// The least alpha of the art's texel a static's height under a pixel for the pixel to be on the static's cap.
const LIGHT_SCENE_CAP_MIN_ALPHA: f32 = 0.5;

// From the cell's corner to the statics' anchor (client_grunge_anchor). Keep in sync with
// EC_STATIC_TILE_TRANSLATION_X and EC_STATIC_TILE_TRANSLATION_Z in statics_collect.rs.
const LIGHT_SCENE_ANCHOR_OFFSET: vec2<f32> = vec2<f32>(0.5, 1.5);
// From a cell's corner to its centre.
const LIGHT_SCENE_CELL_CENTRE: vec2<f32> = vec2<f32>(0.5, 0.5);
// The least texel alpha of a fragment the opaque pass may have drawn: the art tests its depth tie with one tap.
// Below this slope of the view direction across a face's plane, the face is seen edge on and keeps the static's z.
const LIGHT_SCENE_MIN_VIEW_SLOPE: f32 = 0.001;
const LIGHT_SCENE_MIN_ALPHA: f32 = 0.0001;
// How far a fragment's depth may lie from the depth drawn at its pixel and still be the static drawn there: the
// fragments recompute the opaque pass's depth exactly, and the art's tie-break between statics is 1e-6.
const LIGHT_SCENE_DEPTH_TOLERANCE: f32 = 2.5e-7;

@if(LIGHT_SCENE && !LIGHT_SCENE_MSAA)
@group(2) @binding(0) var light_scene_depth: texture_depth_2d;
@if(LIGHT_SCENE && LIGHT_SCENE_MSAA)
@group(2) @binding(0) var light_scene_depth: texture_depth_multisampled_2d;

// The light scene class of a static from its is_wet_flags; flat art and flat quads are floors.
fn light_scene_class(is_wet_flags: u32, lies_flat: bool) -> u32 {
    if (lies_flat) {
        return LIGHT_SCENE_CLASS_HEIGHT;
    }
    if ((is_wet_flags & STATIC_FLAG_LIGHT_WALL) != 0u) {
        return LIGHT_SCENE_CLASS_WALL;
    }
    if ((is_wet_flags & STATIC_FLAG_LIGHT_BLOCK) != 0u) {
        return LIGHT_SCENE_CLASS_BLOCK;
    }
    return LIGHT_SCENE_CLASS_HEIGHT;
}

// The tiledata height of a wall or a solid static from its is_wet_flags; 0 for the other statics.
fn light_scene_height(is_wet_flags: u32) -> i32 {
    return i32((is_wet_flags >> STATIC_LIGHT_HEIGHT_SHIFT) & STATIC_LIGHT_HEIGHT_MASK);
}

// Where to look for a wall's or a solid static's cap. Its art draws each upright face `height` z tall, the same in
// every column of the art, and the cap above the face: a pixel is on the cap when the art still covers the point
// `height` z under it. The face's height under the pixel (`light_scene_texel`) cannot tell: the art draws a wall's
// face, and its cap, in front of the wall's plane by the wall's thickness, so the cap reads up to about 12 z under
// the wall's top, as low as the top of its face (Britain's walls). This gives the art's v (0 at its top, 1 at its
// bottom) of the point `height_world` (the height in world units) under the fragment at `v` on a card
// `card_height` tall; past 1 lies under the art.
fn light_scene_cap_probe_v(v: f32, height_world: f32, card_height: f32) -> f32 {
    return v + height_world / card_height;
}

// Whether the light scene texel `texel` of a wall or a solid static at its art's pixel may lie on the static's cap,
// at `top_z`: its face's point under the pixel is no higher than the top. Art drawn taller than its static's height
// (a bookcase) keeps its faces above the top.
fn light_scene_may_be_cap(texel: vec4<u32>, top_z: i32) -> bool {
    return i32(texel.z) <= top_z + LIGHT_SCENE_Z_BIAS;
}

// The texel of a pixel on the cap of the static of `texel`, at `top_z`: a floor there, a height static, which the
// light clip lights only from a light less than LIGHT_COVER_MIN_RISE under it (light_clip.wesl).
fn light_scene_cap_texel(texel: vec4<u32>, top_z: i32) -> vec4<u32> {
    return vec4<u32>(texel.xy, u32(clamp(top_z + LIGHT_SCENE_Z_BIAS, 0, LIGHT_SCENE_Z_TOP)),
        LIGHT_SCENE_ART | (LIGHT_SCENE_CLASS_HEIGHT << LIGHT_SCENE_CLASS_SHIFT));
}

// Whether a fragment at `position` with the depth `depth` may be the one the opaque pass drew at its pixel: its
// depth is the one there.
@if(LIGHT_SCENE)
fn light_scene_drawn(position: vec4<f32>, depth: f32) -> bool {
    let drawn = textureLoad(light_scene_depth, vec2<i32>(position.xy), 0);
    return abs(depth - drawn) <= LIGHT_SCENE_DEPTH_TOLERANCE;
}

// The light scene texel of a fragment of the static anchored at `anchor` (client_grunge_anchor), at the UO z `z`, of
// class `kind`, at the world point `world_pos` on its card. `view_dir` is the camera's direction and `height_scale`
// the world height of one z, which a wall's or a solid static's face height needs.
fn light_scene_texel(world_pos: vec3<f32>, anchor: vec2<f32>, z: i32, kind: u32, right_xz: vec2<f32>,
    view_dir: vec3<f32>, height_scale: f32) -> vec4<u32> {
    let cell = round(anchor - LIGHT_SCENE_ANCHOR_OFFSET);
    // The card's halves meet over the cell's centre, as its art does over the land.
    let left = dot(world_pos.xz - (cell + LIGHT_SCENE_CELL_CENTRE), right_xz) < 0.0;
    let code = LIGHT_SCENE_ART | (kind << LIGHT_SCENE_CLASS_SHIFT) | select(0u, LIGHT_SCENE_LEFT, left);
    var face_z = z;
    if (kind != LIGHT_SCENE_CLASS_HEIGHT) {
        // The face's point under the pixel lies on the view line through the card's point, where the line crosses
        // the face's plane: a wall's west edge (x = cell x) on the left half and its north edge (world z = cell y)
        // on the right; a solid static's south face (world z = cell y + 1) on the left and its east face
        // (x = cell x + 1) on the right. The card's own height overstates it by up to the cell's depth. The art
        // drawn in front of the plane, such as the top of a wall's thickness, lands below it: no point of a face lies
        // below its static's z. The top itself, its cap, goes as a floor (light_scene_cap_texel).
        let wall = kind == LIGHT_SCENE_CLASS_WALL;
        let across_x = left == wall;
        let planes = select(cell + vec2<f32>(1.0), cell, wall);
        let slope = select(view_dir.z, view_dir.x, across_x);
        if (abs(slope) > LIGHT_SCENE_MIN_VIEW_SLOPE) {
            let run = select(planes.y, planes.x, across_x) - select(world_pos.z, world_pos.x, across_x);
            face_z = max(z, i32(floor((world_pos.y + run / slope * view_dir.y) / height_scale)));
        }
    }
    return vec4<u32>(vec2<u32>(max(cell, vec2<f32>(0.0))), u32(clamp(face_z + LIGHT_SCENE_Z_BIAS, 0, LIGHT_SCENE_Z_TOP)),
        code);
}
