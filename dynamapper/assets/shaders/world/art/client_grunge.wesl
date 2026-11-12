// ============================================================================
// art::client_grunge — the Kingdom Reborn client's grunge on statics (Client Grunge).
//
// Research P3.6 (scripts/dev-helpers/ec-research/ghidra-kr-atmosphere/results/uokr_atmosphere.md). UOSprite.psh
// multiplies the hued colour's RGB by the grunge texture, alpha untouched, on the 8-bit values:
//
//   colour.rgb *= tex2D(GrungeSampler, uv1).rgb
//
// uv1 is the grunge UV of UOSprite::BuildSpriteBatch 0x004454c0 and BuildSpriteQuad 0x00442430. For a point of the
// sprite at camera-plane offset (r right, u up) from the sprite's base point P, in the client's world (X east, Z
// north, the camera's right R and up U from FUN_0045ae50):
//
//   W   = P + R r + U u,   P = (x + 0.5, -(y + 1.5)) + z Zu U      (PlaceSprite 0x004422b0)
//   uv1 = ((W.x - 0.5, W.z + 0.5) - 0.5) / 16 + 0.5                (texture transform: scale 1/16 about 0.5)
//
// wrapping and bilinear. The client keeps one quad per sprite, so the UV is affine over it: this computes it per
// vertex from the vertex's screen offset to the static's base point and lets the rasteriser interpolate it, as the
// client does. Our screen offsets are the client's (client_shadow.wesl: view x is r, view y / pixel_aspect is u),
// and our base point (tile x + 0.5, z, tile y + 1.5) is where the client's lands on screen, so any art drawn at a
// place gets that place's grunge: CC and EC art alike.
//
// The frame is linear light, so the multiply runs on the colour's sRGB encoding; the texture is sampled raw
// (Rgba8Unorm), its 8-bit values as the client sees them. Strength s blends the factor towards 1: mix(1, g, s).
// ============================================================================

import bevy_pbr::render::mesh_view_bindings::view;
import package::shaders::world::art::art_bindings::{SpriteInstance, HEIGHT_SCALE};
import package::shaders::world::art::client_shadow::CLIENT_SHADOW_Z_UNIT;
import package::shaders::world::srgb::{encode_srgb, decode_srgb};

// SpriteInstance.is_wet_flags bit; keep in sync with STATIC_FLAG_CLIENT_GRUNGE in client_grunge.rs.
const STATIC_FLAG_CLIENT_GRUNGE: u32 = 1024u;
// The client camera's right and up axes on the ground (X, Z): the columns of Rz(-π/2) Rx(0.001 - π/4) Rz(-0.05),
// FUN_0045ae50 with 0x00b86f10 (0.001) and 0x00b87004 (-0.05). The camera looks almost straight down, 45 degrees
// round, rolled by -0.05.
const CLIENT_GRUNGE_RIGHT_XZ: vec2<f32> = vec2<f32>(0.70639932, 0.70781353);
const CLIENT_GRUNGE_UP_XZ: vec2<f32> = vec2<f32>(-0.70692895, 0.70551651);
// The quad corners' grunge offsets in BuildSpriteQuad: X - 0.5, Z + 0.5.
const CLIENT_GRUNGE_CORNER_OFFSET: vec2<f32> = vec2<f32>(-0.5, 0.5);
// The tileart grunge ref's texture_stretch, 16 tiles per repeat on every grunged item, as the transform's 1 / scale
// about the centre 0.5.
const CLIENT_GRUNGE_REPEAT: f32 = 16.0;
const CLIENT_GRUNGE_TRANSFORM_CENTER: vec2<f32> = vec2<f32>(0.5, 0.5);
const NO_GRUNGE: vec3<f32> = vec3<f32>(1.0);

fn client_grunges(inst: SpriteInstance) -> bool {
    return (inst.is_wet_flags & STATIC_FLAG_CLIENT_GRUNGE) != 0u;
}

// The grunge UV of a vertex `offset` world units from the instance's position (inst.world_*).
fn client_grunge_uv(inst: SpriteInstance, offset: vec3<f32>) -> vec2<f32> {
    // View y units per screen-square unit (as in client_shadow.wesl).
    let pixel_aspect = (view.clip_from_view[0][0] * view.viewport.z) / (view.clip_from_view[1][1] * view.viewport.w);
    let view_from_world = mat3x3<f32>(view.view_from_world[0].xyz, view.view_from_world[1].xyz,
        view.view_from_world[2].xyz);
    let anchor = vec3<f32>(inst.client_grunge_anchor.x, inst.base_world_y, inst.client_grunge_anchor.y);
    let from_anchor = view_from_world * ((vec3<f32>(inst.world_x, inst.world_y, inst.world_z) - anchor) + offset);
    let r = from_anchor.x;
    let u = from_anchor.y / pixel_aspect;
    let z = inst.base_world_y / HEIGHT_SCALE;
    // The client's world has Z = -our z (north up).
    let base = vec2<f32>(anchor.x, -anchor.z) + z * CLIENT_SHADOW_Z_UNIT * CLIENT_GRUNGE_UP_XZ;
    // Whole repeats of the base point drop out per instance, keeping the interpolated UV small and precise.
    let base_repeats = base / CLIENT_GRUNGE_REPEAT;
    let local = CLIENT_GRUNGE_RIGHT_XZ * r + CLIENT_GRUNGE_UP_XZ * u + CLIENT_GRUNGE_CORNER_OFFSET
        - CLIENT_GRUNGE_TRANSFORM_CENTER;
    return fract(base_repeats) + local / CLIENT_GRUNGE_REPEAT + CLIENT_GRUNGE_TRANSFORM_CENTER;
}

// The colour times the grunge at `uv`, on its 8-bit encoding; strength 0 leaves it as it is.
fn apply_client_grunge(rgb: vec3<f32>, uv: vec2<f32>, strength: f32, grunge_texture: texture_2d<f32>,
    grunge_sampler: sampler) -> vec3<f32> {
    let grunge = textureSampleLevel(grunge_texture, grunge_sampler, uv, 0.0).rgb;
    let factor = mix(NO_GRUNGE, grunge, clamp(strength, 0.0, 1.0));
    return decode_srgb(encode_srgb(rgb) * factor);
}
