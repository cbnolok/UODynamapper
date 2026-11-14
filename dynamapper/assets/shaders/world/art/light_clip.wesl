// The light glows' fragment shader (art/light_scene.rs, LightClipMaterial): the light.mul decals of Classic and
// UOD-Painterly and the light sprites of KR-like and Enhanced. Bevy's unlit StandardMaterial path, and under
// LIGHT_CLIP the "uod" light cover: a glow lights only the pixels whose static the light reaches
// (`light_reaches`), read from the light scene (light_scene.wesl).
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, main_pass_post_lighting_processing},
    mesh_functions::{get_tag, get_world_from_local},
};
import package::shaders::world::art::light_scene::{LIGHT_SCENE_ART, LIGHT_SCENE_CLASS_SHIFT, LIGHT_SCENE_CLASS_MASK,
    LIGHT_SCENE_CLASS_HEIGHT, LIGHT_SCENE_LEFT, LIGHT_SCENE_Z_BIAS};

@group(3) @binding(100) var light_scene: texture_2d<u32>;

// The light's MeshTag: its cell from the cell under the quad's origin, then its z and its ceiling. Keep in sync with
// `light_clip_tag` in art/light_scene.rs.
const LIGHT_CLIP_CELL_BITS: u32 = 6u;
const LIGHT_CLIP_DY_SHIFT: u32 = 6u;
const LIGHT_CLIP_Z_SHIFT: u32 = 12u;
const LIGHT_CLIP_CEILING_SHIFT: u32 = 20u;
const LIGHT_CLIP_Z_BITS: u32 = 8u;
// The ceiling of a light with nothing over it.
const LIGHT_CLIP_NO_CEILING: i32 = -128;
// How far above a light a static must start to be over it, in z units. Keep in sync with LIGHT_COVER_MIN_RISE in
// art/light_cover.rs.
const LIGHT_COVER_MIN_RISE: i32 = 5;
// How far from a glow's pixel, in pixels, the clip also reads the light scene. A static's filtered edge fades its
// coverage, and the transparent pass blends that fade over what lies behind without writing depth, so the light
// scene holds the static behind it there (the floor under a roof, the land): a seam up to about this wide along the
// edges of the roof tiles. A glow lights a pixel only when its light reaches the statics this far around it too.
const LIGHT_CLIP_EDGE_REACH: i32 = 2;

// Whether a light on `light_cell` at `light_z`, under the floor or roof at `ceiling`, lights the static of the light
// scene texel `texel`:
// - the land and anything else that is not a static: always;
// - a floor, a roof or another static (LIGHT_SCENE_CLASS_HEIGHT): below LIGHT_COVER_MIN_RISE above the light. The
//   top of a wall or a solid static, its cap, is one at the static's top z (light_scene.wesl), so the cap flush with
//   the floor over a light stays dark with the floor;
// - a wall or a solid static: below the ceiling, on the faces turned to the light or on its own row or column. The
//   face stays dark from LIGHT_COVER_MIN_RISE under the ceiling up: the seams between a wall's cap tiles and the
//   rim where the cap meets the face read as the face's top, just under the floor flush with it.
fn light_reaches(texel: vec4<u32>, light_cell: vec2<i32>, light_z: i32, ceiling: i32) -> bool {
    let code = texel.w;
    if ((code & LIGHT_SCENE_ART) == 0u) {
        return true;
    }
    let z = i32(texel.z) - LIGHT_SCENE_Z_BIAS;
    let kind = (code >> LIGHT_SCENE_CLASS_SHIFT) & LIGHT_SCENE_CLASS_MASK;
    if (kind == LIGHT_SCENE_CLASS_HEIGHT) {
        return z < light_z + LIGHT_COVER_MIN_RISE;
    }
    if (ceiling != LIGHT_CLIP_NO_CEILING && z + LIGHT_COVER_MIN_RISE > ceiling) {
        return false;
    }
    let cell = vec2<i32>(texel.xy);
    // The south face (the card's left half) and the east face of a wall or a solid static: lit from beyond them, and
    // from their own row or column. A wall's art stands on its cell's south and east edges, so a sconce on the cell
    // in front of it lights the whole run of the wall along its row or column; taking them for the west and north
    // edges cut the glow at the sconce. A light on the run's own line lies in the wall's thickness, a window or a
    // torch set in the wall, and lights the run's face on both sides as it lights its own cell's.
    let left = (code & LIGHT_SCENE_LEFT) != 0u;
    return select(light_cell.x >= cell.x, light_cell.y >= cell.y, left);
}

// Whether the light reaches the light scene texel at `pixel`, clamped into the light scene of size `size`.
fn light_reaches_at(pixel: vec2<i32>, size: vec2<i32>, light_cell: vec2<i32>, light_z: i32, ceiling: i32) -> bool {
    let texel = textureLoad(light_scene, clamp(pixel, vec2<i32>(0), size - vec2<i32>(1)), 0);
    return light_reaches(texel, light_cell, light_z, ceiling);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    @if(LIGHT_CLIP) {
        let tag = i32(get_tag(in.instance_index));
        let origin = vec2<i32>(floor(get_world_from_local(in.instance_index)[3].xz));
        let light_cell = origin + vec2<i32>(extractBits(tag, 0u, LIGHT_CLIP_CELL_BITS),
            extractBits(tag, LIGHT_CLIP_DY_SHIFT, LIGHT_CLIP_CELL_BITS));
        let light_z = extractBits(tag, LIGHT_CLIP_Z_SHIFT, LIGHT_CLIP_Z_BITS);
        let ceiling = extractBits(tag, LIGHT_CLIP_CEILING_SHIFT, LIGHT_CLIP_Z_BITS);
        let pixel = vec2<i32>(in.position.xy);
        let size = vec2<i32>(textureDimensions(light_scene));
        let reach_x = vec2<i32>(LIGHT_CLIP_EDGE_REACH, 0);
        let reach_y = vec2<i32>(0, LIGHT_CLIP_EDGE_REACH);
        // Until the light scene takes the view's size, the glow is not clipped.
        if (all(pixel < size) && (!light_reaches_at(pixel, size, light_cell, light_z, ceiling)
            || !light_reaches_at(pixel + reach_x, size, light_cell, light_z, ceiling)
            || !light_reaches_at(pixel - reach_x, size, light_cell, light_z, ceiling)
            || !light_reaches_at(pixel + reach_y, size, light_cell, light_z, ceiling)
            || !light_reaches_at(pixel - reach_y, size, light_cell, light_z, ceiling))) {
            discard;
        }
    }

    // Bevy's forward fragment (bevy_pbr pbr.wesl) for its unlit materials.
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material.flags, pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, pbr_input.material.base_color);
    return out;
}
