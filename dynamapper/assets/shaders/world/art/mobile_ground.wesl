// ============================================================================
// art::mobile_ground — Where the line of sight of a mobile's pixel below its anchor meets the ground it is drawn
// over (art/depth.wesl mobile_fragment_view_z), from the ground around the mobile (`MobileGround`,
// render/mobiles/ground.rs): the land as drawn, its corners' rises over the mobile's base, and the Background floors
// and carpets on each cell.
//
// The line runs down, away from the camera, from where it is MOBILE_GROUND_REACH_Z over the base to where it is
// MOBILE_GROUND_TIE_Z over it. The cells it crosses are taken in turn, nearest the camera first, and it meets the
// ground on the first one where it leaves the cell no higher over the ground there than a tie: the land between the
// corners of the edge it leaves by, or a floor on the cell. On flat ground at the base it meets it on the cell where
// it is a tie over the base: on an exact tie, the nearer cell. The Enhanced art of a floor drawn on a camera quad
// reaches past its cell (MOBILE_GROUND_FLOOR_OVERHANG): the line meets that floor where its art lies, as far past the
// cell's far edges. A cell outside the ground read, or a mobile whose ground is not read yet, lies flat at the base.
//
// The pixel lies on the cell it meets at the mobile's priority z, or at the lowest priority z of a static standing on
// that cell if that is lower (a Background static with height, a post or a tree, at its own z): under the static.
// ============================================================================

import package::shaders::world::art::art_bindings::{MobileGround, HEIGHT_SCALE};
import package::shaders::world::art::depth::{sight_plane_hit, MOBILE_GROUND_REACH_Z, MOBILE_GROUND_TIE_Z};

// The cells along a side of the ground read: keep in sync with MOBILE_GROUND_DIM in ground.rs.
const MOBILE_GROUND_DIM: i32 = 16;
// The bits of a cell, and the cells in a word and in a vec4 of the cells.
const MOBILE_GROUND_CELL_BITS: u32 = 4u;
const MOBILE_GROUND_CELL_MASK: u32 = 0xFu;
const MOBILE_GROUND_CELLS_PER_WORD: u32 = 8u;
const MOBILE_GROUND_WORDS_PER_VEC: u32 = 4u;
// A cell's fields (ground.rs `ground_cell_code`): the land's rise at its top corner in bits 0 and 1, 0 to
// MOBILE_GROUND_REACH_Z, or MOBILE_GROUND_LAND_HIDDEN for a cell whose land is not drawn (over the z-view's limit);
// in bits 2 and 3, 0 for no floor, or the floors' rise + 1.
const MOBILE_GROUND_FIELD_MASK: u32 = 3u;
const MOBILE_GROUND_LAND_HIDDEN: u32 = 3u;
const MOBILE_GROUND_FLOOR_SHIFT: u32 = 2u;
// MobileGround `flags`: the ground is read; the floors' art reaches past their cells.
const MOBILE_GROUND_PRESENT: u32 = 1u;
const MOBILE_GROUND_FLOORS_OVERHANG: u32 = 2u;
// How far past its cell's edges, in world units, the art of a floor on a camera quad reaches: the quad is the cell's
// diamond grown by 0.7671049 / 0.707105 about its centre (ec_item_quad.wesl, UOSA 0x45a080), so each edge moves out
// by half the growth of the cell's side, about a pixel and a third.
const MOBILE_GROUND_FLOOR_OVERHANG: f32 = (0.7671049 / 0.707105 - 1.0) * 0.5;
// The most cells the line crosses between the two heights before it is taken to meet the ground where it is a tie
// over the base: 3 in the isometric camera, more in a perspective one looking near the horizon.
const MOBILE_GROUND_MAX_CELLS: i32 = 8;

// The code of the cell `cell` of the ground `ground`, read through a pointer to the function's copy of it (a varying
// index may read the function's memory), from its `cells` or with `standing` from its `standing`: 0 outside it, flat
// land at the base, no floor and nothing standing.
fn mobile_ground_code_of(ground: ptr<function, MobileGround>, cell: vec2<i32>, standing: bool) -> u32 {
    let local = cell - (*ground).first_cell;
    if (any(local < vec2<i32>(0)) || any(local >= vec2<i32>(MOBILE_GROUND_DIM))) {
        return 0u;
    }
    let index = u32(local.y * MOBILE_GROUND_DIM + local.x);
    let word = index / MOBILE_GROUND_CELLS_PER_WORD;
    let vec_index = word / MOBILE_GROUND_WORDS_PER_VEC;
    let lane = word % MOBILE_GROUND_WORDS_PER_VEC;
    let packed = select((*ground).cells[vec_index][lane], (*ground).standing[vec_index][lane], standing);
    return (packed >> ((index % MOBILE_GROUND_CELLS_PER_WORD) * MOBILE_GROUND_CELL_BITS)) & MOBILE_GROUND_CELL_MASK;
}

// The code of the cell `cell`: its land and floors.
fn mobile_ground_code(ground: ptr<function, MobileGround>, cell: vec2<i32>) -> u32 {
    return mobile_ground_code_of(ground, cell, false);
}

// The priority z a pixel lying on the cell `cell` takes, the mobile's priority z being `priority_z`: the lowest
// priority z of the statics standing on it, how far under the base the standing code says (+ 1, 0 for none), when
// that is lower, so they cover the pixel at their tie levels (1 and up) over its 0.
fn mobile_ground_priority_z(ground: ptr<function, MobileGround>, cell: vec2<i32>, priority_z: f32) -> f32 {
    let standing = mobile_ground_code_of(ground, cell, true);
    if (standing == 0u) {
        return priority_z;
    }
    return min(priority_z, (*ground).base_z - f32(standing - 1u));
}

// The land's rise at the top corner of the cell `cell`, as a corner of its neighbours too: a corner over the z-view's
// limit rises as high as the ground is read.
fn mobile_ground_corner_rise(ground: ptr<function, MobileGround>, cell: vec2<i32>) -> f32 {
    let land = mobile_ground_code(ground, cell) & MOBILE_GROUND_FIELD_MASK;
    return min(f32(land), MOBILE_GROUND_REACH_Z);
}

// The rise over the base, in z, of the land at `point` (world x and z) on the cell `cell`, between its corners.
fn mobile_ground_land_rise(ground: ptr<function, MobileGround>, cell: vec2<i32>, point: vec2<f32>) -> f32 {
    // The corners: top (x, y), right (x + 1, y), left (x, y + 1), bottom (x + 1, y + 1).
    let top = mobile_ground_corner_rise(ground, cell);
    let right = mobile_ground_corner_rise(ground, cell + vec2<i32>(1, 0));
    let left = mobile_ground_corner_rise(ground, cell + vec2<i32>(0, 1));
    let bottom = mobile_ground_corner_rise(ground, cell + vec2<i32>(1, 1));
    let inside = clamp(point - vec2<f32>(cell), vec2<f32>(0.0), vec2<f32>(1.0));
    return mix(mix(top, right, inside.x), mix(left, bottom, inside.x), inside.y);
}

// How far along the segment from `start` by `span` the line leaves the cell whose corner is `corner`, grown by `grown`
// past its far edges: 1 at the segment's end.
fn mobile_ground_leaves(start: vec2<f32>, span: vec2<f32>, corner: vec2<f32>, grown: f32) -> vec2<f32> {
    let edge = select(corner - vec2<f32>(grown), corner + vec2<f32>(1.0 + grown), span > vec2<f32>(0.0));
    return select(vec2<f32>(1.0), (edge - start) / span, span != vec2<f32>(0.0));
}

// The line's rise over the base, in z, at `t` along the segment.
fn mobile_ground_line_rise(t: f32) -> f32 {
    return mix(MOBILE_GROUND_REACH_Z, MOBILE_GROUND_TIE_Z, clamp(t, 0.0, 1.0));
}

// Where the line of sight through `card_point`, of a mobile whose base is at world height `base_world_y` and whose
// priority z is `priority_z`, meets the ground `ground` around it: in x and z a point on the cell it meets it on (its
// centre, so the cell is never in doubt), in y the priority z the pixel takes there (mobile_ground_priority_z), w 0
// when the line runs too level to meet it near (sight_plane_hit).
fn mobile_ground_hit(card_point: vec3<f32>, base_world_y: f32, priority_z: f32, ground_read: MobileGround)
    -> vec4<f32> {
    var ground = ground_read;
    let read = (ground.flags & MOBILE_GROUND_PRESENT) != 0u;
    let base_y = select(base_world_y, ground.base_z * HEIGHT_SCALE, read);
    let far = sight_plane_hit(card_point, base_y + MOBILE_GROUND_TIE_Z * HEIGHT_SCALE);
    if (far.w == 0.0 || !read) {
        return vec4<f32>(far.x, priority_z, far.z, far.w);
    }
    let near = sight_plane_hit(card_point, base_y + MOBILE_GROUND_REACH_Z * HEIGHT_SCALE);
    let start = near.xz;
    let span = far.xz - start;
    let overhang = select(0.0, MOBILE_GROUND_FLOOR_OVERHANG, (ground.flags & MOBILE_GROUND_FLOORS_OVERHANG) != 0u);
    var cell = vec2<i32>(floor(start));
    for (var crossed = 0; crossed < MOBILE_GROUND_MAX_CELLS; crossed += 1) {
        let corner = vec2<f32>(cell);
        let code = mobile_ground_code(&ground, cell);
        let leaves = mobile_ground_leaves(start, span, corner, 0.0);
        let t = clamp(min(leaves.x, leaves.y), 0.0, 1.0);
        let land = code & MOBILE_GROUND_FIELD_MASK;
        let floors = (code >> MOBILE_GROUND_FLOOR_SHIFT) & MOBILE_GROUND_FIELD_MASK;
        var meets = land != MOBILE_GROUND_LAND_HIDDEN && mobile_ground_line_rise(t)
            <= mobile_ground_land_rise(&ground, cell, start + span * t) + MOBILE_GROUND_TIE_Z;
        if (floors != 0u) {
            let past = mobile_ground_leaves(start, span, corner, overhang);
            meets = meets || mobile_ground_line_rise(min(past.x, past.y))
                <= f32(floors - 1u) + MOBILE_GROUND_TIE_Z;
        }
        if (meets) {
            return vec4<f32>(corner.x + 0.5, mobile_ground_priority_z(&ground, cell, priority_z), corner.y + 0.5, 1.0);
        }
        if (t >= 1.0) {
            break;
        }
        let step = vec2<i32>(sign(span));
        cell += select(vec2<i32>(0), step, leaves <= vec2<f32>(t));
    }
    return vec4<f32>(far.x, mobile_ground_priority_z(&ground, vec2<i32>(floor(far.xz)), priority_z), far.z, 1.0);
}
