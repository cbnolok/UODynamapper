// ============================================================================
// art::client_shadow — the Kingdom Reborn client's static shadow quad (KR-like).
//
// Research P3.4 (scripts/dev-helpers/ec-research/ghidra-kr-atmosphere/results/uokr_atmosphere.md), placement
// 0x00440e50. The shadow is a copy of the caster's sprite in the camera plane, a hair behind the caster, rotated by
// θ about a pivot on the ground below the caster's anchor, so it leans right with the caster's full width and
// height. In camera-plane units (r right, u up) relative to the pivot P' = P - h Zu (camera up):
//
//   r0 = the sprite's left edge, W its width; u0 = its bottom edge + h Zu, u1 = u0 + its height
//   Rot(r, u) = (r cos θ + u sin θ, u cos θ - r sin θ)
//   TL = Rot(r0, u1), TR = TL + (W, 0)
//   h == 0: BL = (r0, u0), BR = (r0 + W, u0)       otherwise BL = Rot(r0, u0), BR = Rot(r0 + W, u0)
//
// Wall casters (tileart Wall) drop u0 and u1 by 1.05 and move every corner one tile west and one tile north: the
// client adds world X -1 and Z +1, and its Z is -UO y (UOSprite::PlaceSprite 0x004422b0). On screen that is one
// tile up, less the 1.05 drop; along the view it puts the quad two diagonal steps further back, so a wall's shadow
// does not fall on its own neighbours along the wall (client_shadow_logical_offset).
//
// Axes. The client's camera space is Gamebryo's: X the view direction, Y up, Z right. Ours is Bevy's: x right
// (BILLBOARD_RIGHT_XZ, world (1, 0, -1) / sqrt 2), y up, -z the view direction. So client (X, Y, Z) = our
// (-z, y, x), and r, u above are our view x and y. The client's units are ours along the screen: one tile is one
// world unit, a tile diamond 44 classic pixels = sqrt 2 units wide, and Zu = 4 classic pixels, the UO z step on
// screen. Our ortho projection stretches the view's y on screen (the classic 44 x 44 diamond from an isometric
// camera), so the maths runs in screen-square units, u_screen = u_view / pixel_aspect, and goes back after.
// The quad samples the caster's texture with its own UVs, bilinear as the client's (the shadow descriptor's +0x38 is
// 0x41, without the nearest-filter bit 2 that UOSprite::BuildSpriteBatch 0x004454c0 tests); the fragment writes
// black with the texel's alpha.
// ============================================================================

import bevy_pbr::render::mesh_view_bindings::view;
import package::shaders::world::art::art_bindings::{SpriteInstance, BILLBOARD_RIGHT_XZ};
import package::shaders::world::art::depth::{STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON, STATIC_DEPTH_TIE_BREAK_VIEW_STEP};

// SpriteInstance.client_shadow_flags; keep in sync with statics_collect.rs.
const CLIENT_SHADOW_FLAG_CASTS: u32 = 1u;
const CLIENT_SHADOW_FLAG_WALL: u32 = 2u;
// World units per UO z unit along the camera's up axis, 0x00b86f38 (= 6 K, K = 0.021427422 per art pixel).
const CLIENT_SHADOW_Z_UNIT: f32 = 0.12856454;
// The lean θ, 0x00b87170 (= π / 16, 11.25 degrees).
const CLIENT_SHADOW_LEAN: f32 = 0.19634375;
// How far behind the caster the quad lies along the view, 0x00b42f48. Its depth is the caster's priority depth
// half a tie-break step behind (CLIENT_SHADOW_DEPTH_ORDINAL_OFFSET); this only places the rasterised quad.
const CLIENT_SHADOW_BEHIND: f32 = 0.0001;
// Wall branch: both edges drop by 0x00b8716c along the camera's up axis...
const CLIENT_SHADOW_WALL_DROP: f32 = 1.05;
// ...and every corner moves by the client's world X -1.0 and Z +1.0 (0x00b4e8bc). The client's Z is -UO y, ours +UO y:
// one tile west and one tile north.
const CLIENT_SHADOW_WALL_SHIFT: vec3<f32> = vec3<f32>(-1.0, 0.0, -1.0);
// The shadow's depth sits this many tie-break steps behind its caster: behind the caster's plane (the caster hides
// its own shadow, as the client's ε does) and in front of anything sorted behind it, which receives the shadow.
const CLIENT_SHADOW_DEPTH_ORDINAL_OFFSET: f32 = 0.5;
// Upright billboards only: flat quads are surfaces, and the authored EC camera quads are the tileart Rotated items,
// which the client excludes.
const CLIENT_SHADOW_BILLBOARD: u32 = 0u;

fn client_shadow_casts(inst: SpriteInstance) -> bool {
    return (inst.client_shadow_flags & CLIENT_SHADOW_FLAG_CASTS) != 0u && inst.flat_quad == CLIENT_SHADOW_BILLBOARD;
}

fn client_shadow_rotate(p: vec2<f32>) -> vec2<f32> {
    let c = cos(CLIENT_SHADOW_LEAN);
    let s = sin(CLIENT_SHADOW_LEAN);
    return vec2<f32>(p.x * c + p.y * s, p.y * c - p.x * s);
}

// The shadow corner's offset from the caster's anchor (inst.world_*), in world units. `corner` is the sprite's own
// corner: x 0 left, 1 right; y 0 top, 1 bottom, as the billboard path in main.wesl lays it out.
fn client_shadow_world_offset(inst: SpriteInstance, corner: vec2<f32>) -> vec3<f32> {
    // View y units per screen-square unit (as for the EC camera quads in main.wesl).
    let pixel_aspect = (view.clip_from_view[0][0] * view.viewport.z) / (view.clip_from_view[1][1] * view.viewport.w);
    let view_from_world = mat3x3<f32>(view.view_from_world[0].xyz, view.view_from_world[1].xyz,
        view.view_from_world[2].xyz);
    let world_from_view = mat3x3<f32>(view.world_from_view[0].xyz, view.world_from_view[1].xyz,
        view.world_from_view[2].xyz);
    // The caster's own quad in the camera plane, relative to its anchor.
    let low = view_from_world * vec3<f32>(inst.local_min.x * BILLBOARD_RIGHT_XZ.x, inst.local_min.y,
        inst.local_min.x * BILLBOARD_RIGHT_XZ.y);
    let high = view_from_world * vec3<f32>(inst.local_max.x * BILLBOARD_RIGHT_XZ.x, inst.local_max.y,
        inst.local_max.x * BILLBOARD_RIGHT_XZ.y);
    let r0 = min(low.x, high.x);
    let width = abs(high.x - low.x);
    let height = abs(high.y - low.y) / pixel_aspect;
    let h = inst.client_shadow_height;
    let drop = h * CLIENT_SHADOW_Z_UNIT;
    let wall = (inst.client_shadow_flags & CLIENT_SHADOW_FLAG_WALL) != 0u;
    var u0 = min(low.y, high.y) / pixel_aspect + drop;
    if (wall) {
        u0 = u0 - CLIENT_SHADOW_WALL_DROP;
    }
    let u1 = u0 + height;
    var p: vec2<f32>;
    if (corner.y < 0.5) {
        p = client_shadow_rotate(vec2<f32>(r0, u1)) + vec2<f32>(corner.x * width, 0.0);
    } else if (h == 0.0) {
        p = vec2<f32>(r0 + corner.x * width, u0);
    } else {
        p = client_shadow_rotate(vec2<f32>(r0 + corner.x * width, u0));
    }
    // Back from the pivot to the anchor; Bevy's view looks down -z, so behind is -z.
    var offset = world_from_view * vec3<f32>(p.x, (p.y - drop) * pixel_aspect, -CLIENT_SHADOW_BEHIND);
    if (wall) {
        offset = offset + CLIENT_SHADOW_WALL_SHIFT;
    }
    return offset;
}

// The shadow's priority point relative to its caster's, in world units. The wall shift moves the client's quad along
// the view as well as on screen, so a wall's shadow sorts as if cast from the cell one tile west and one tile north.
fn client_shadow_logical_offset(inst: SpriteInstance) -> vec3<f32> {
    if ((inst.client_shadow_flags & CLIENT_SHADOW_FLAG_WALL) != 0u) {
        return CLIENT_SHADOW_WALL_SHIFT;
    }
    return vec3<f32>(0.0);
}

// static_priority_depth (depth.wesl) with the shadow's fractional tie-break step.
fn client_shadow_priority_depth(clip_position: vec4<f32>, clip_from_view: mat4x4<f32>, ordinal: u32) -> f32 {
    let steps = f32(ordinal) + CLIENT_SHADOW_DEPTH_ORDINAL_OFFSET;
    if (clip_from_view[3][3] == 1.0) {
        let depth = clip_position.z / clip_position.w;
        return clamp(depth - steps * STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON, 0.0, 1.0);
    }
    let offset = clip_from_view * vec4<f32>(0.0, 0.0, -steps * STATIC_DEPTH_TIE_BREAK_VIEW_STEP, 0.0);
    let biased = clip_position + offset;
    return clamp(biased.z / biased.w, 0.0, 1.0);
}
