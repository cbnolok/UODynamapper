// ============================================================================
// ec_sprite_hue.wgsl
//
// WGSL translation of the Enhanced Client (EC) sprite/hue pixel shader
// (source: Shaders.uop_B0_F11, "//#target ps_2_0").
//
// ---- WHAT THIS SHADER IS ----
// This is the primary fragment shader for *all EC 2D sprites* (characters,
// mobiles, items, particles, and effects) that can be tinted by the UO hue
// system.  It handles three layered effects:
//
//   1. SPRITE TEXTURE: samples the base sprite/art texture at texCoord0.
//   2. HUE REMAPPING: if the entity has a hue index, the pixel's luminance
//      is used as an X-offset into a 1D hue lookup texture (a colour palette),
//      replacing the original RGB with the hued colour.  This is the standard
//      UO hue system — every tintable item/creature uses this path.
//   3. ETHEREAL (ghost) effect: a subtle blue-shift applied on top of the
//      hue result for translucent/ghost entities.
//   4. ALPHA MODULATION: the final alpha is multiplied by the vertex diffuse
//      alpha (IN.color.a), allowing fade-in/out and transparency.
//
// ---- COMPILE-TIME PERMUTATIONS ----
// The original HLSL uses #define/#if to compile multiple permutation variants:
//   HAS_HUE_TEX     (0/1) — whether a hue texture is bound at all.
//   HAS_HUEMASK_TEX (0/1) — whether a per-pixel hue-mask texture exists
//                           (unused in this translation — always 0 in practice).
//   HAS_GRUNGE_TEX  (0/1) — grunge overlay (unused in this translation).
//   IS_ETHEREAL     (0/1) — ghost/ethereal blue-shift mode.
//
// In WGSL we replace the compile-time flags with dynamic uniforms, which is
// equivalent for GPU performance on modern hardware (uniforms are scalar
// branches compiled to predicated instructions).
//
// ---- WHAT IT IS APPLIED ON ----
// Every 2D sprite billboard drawn in the world view:
//   • Player characters and NPCs / mobiles
//   • Worn equipment (armour, weapons, clothes)
//   • Static and dynamic items on the ground
//   • Particle effects and projectiles
//
// It runs per-fragment after the vertex shader (ec_sprite.wgsl) has projected
// the billboard into clip space and computed sprite UV + world-space UV.
//
// ---- HUE TEXTURE FORMAT ----
// The EC hue texture is a 2D RGBA8 image laid out as:
//   Width:  hue-entry stride (256 pixels × N hues)
//   Height: 1024 rows (up to 1024 distinct hues)
//
// Each row is one palette.  The X coordinate is derived from the pixel's
// luminance scaled by 255 (the number of colour entries per palette).
// texCoord2 from the vertex shader carries (hue_row / texture_height, 0).
// ============================================================================


// ============================================================================
// GPU resource bindings (group 0)
// ============================================================================

// Base sprite/art texture (RGBA8 or DXT1/BC1).
@group(0) @binding(0) var sprite_texture: texture_2d<f32>;
@group(0) @binding(1) var sprite_sampler: sampler;

// 1D hue palette texture (2D layout, see format note above).
@group(0) @binding(2) var hue_texture: texture_2d<f32>;
@group(0) @binding(3) var hue_sampler: sampler;

// Optional hue-mask texture (not used in standard hue path; reserved).
// @group(0) @binding(4) var hue_mask_texture: texture_2d<f32>;

// Optional grunge/detail texture (not used in standard hue path; reserved).
// @group(0) @binding(5) var grunge_texture: texture_2d<f32>;

struct SpriteHueParams {
    // Runtime permutation flags (replace the HLSL compile-time #defines).
    // 1 = apply hue lookup;  0 = pass sprite colour unchanged.
    has_hue:       u32,
    // 1 = apply ethereal blue-shift on top of hue result.
    is_ethereal:   u32,
    _pad:          vec2<u32>,

    // Vertex diffuse alpha (equivalent to IN.color.a in the HLSL).
    // Typically comes from the instance data / sprite batch.
    diffuse_alpha: f32,
    _pad2:         vec3<f32>,

    // Hue texture parameters (pre-computed on the CPU to avoid per-fragment
    // divisions).  Mirror the HLSL static constants:
    //   hue_texture_height = 1024.0
    //   num_hue_pixels     = 255.0
    //   hue_pixel_uv_offset = (1.0 / hue_texture_height) * num_hue_pixels
    hue_pixel_uv_offset: f32, // = (1.0 / 1024.0) * 255.0 ≈ 0.249023
    _pad3: vec3<f32>,
}
@group(0) @binding(4) var<uniform> params: SpriteHueParams;


// ============================================================================
// Constants — match the HLSL static constants
// ============================================================================

// Hue texture height (number of palette rows).
const HUE_TEXTURE_HEIGHT: f32 = 1024.0;
// Number of colour entries per palette row.
const NUM_HUE_PIXELS:     f32 = 255.0;
// UV offset per luma unit (pre-computed: 1/1024 * 255 ≈ 0.249).
// Uploaded via SpriteHueParams.hue_pixel_uv_offset for flexibility.

// Rec.709 luma weights (same as the rest of the EC pipeline).
const LUMINANCE_CONV: vec3<f32> = vec3<f32>(0.2125, 0.7154, 0.0721);

// Ethereal blue-shift bias added per channel.
// In the original HLSL: EtherealConv = { 0.0f, 0.01f, 0.025f }
// This adds a subtle cool-blue tint to ghost/ethereal entities.
const ETHEREAL_BIAS: vec3<f32> = vec3<f32>(0.0, 0.01, 0.025);


// ============================================================================
// Vertex output (from ec_sprite.wgsl / ec_sprite_vs.wgsl)
// ============================================================================

struct VertexOutput {
    @builtin(position)  clip_position: vec4<f32>,
    // Sprite/art UV — [0,1] within the sprite's atlas region.
    @location(0)        tex_coord_sprite: vec2<f32>,
    // World-space UV — used for grunge/overlay textures (not used here).
    @location(1)        tex_coord_world:  vec2<f32>,
    // Hue texture row index packed as (row_uv, 0.0).
    //   row_uv = hue_index / HUE_TEXTURE_HEIGHT
    // (0, 0) means "no hue" — pass-through.
    @location(2)        hue_coord:        vec2<f32>,
}


// ============================================================================
// Fragment shader
// ============================================================================

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // ---- 1. Sample base sprite texture ----
    var out_color = textureSample(sprite_texture, sprite_sampler, in.tex_coord_sprite);

    // Fully transparent pixels are discarded immediately — no further
    // processing needed.  This also avoids writing hue/ethereal colour
    // into fully transparent texels (which would be discarded by the
    // alpha test anyway).
    if (out_color.a <= 0.0) {
        discard;
    }

    // ---- 2. Hue remapping (UO palette lookup) ----
    // Only executed when the entity has a hue index assigned (has_hue == 1)
    // and the vertex shader has provided a non-zero hue texture row (hue_coord).
    if (params.has_hue == 1u && (in.hue_coord.x > 0.0 || in.hue_coord.y > 0.0)) {
        // Compute the sprite pixel's luminance.
        // This is the X-axis index into the palette: brighter pixels map to
        // higher-index (usually lighter/more saturated) entries in the hue.
        let luma = dot(out_color.rgb, LUMINANCE_CONV);

        // Scale luma [0,1] to UV offset within the current palette row.
        // hue_pixel_uv_offset = (1 / texture_height) * num_pixels_per_row.
        let hue_index_uv = luma * params.hue_pixel_uv_offset;

        // Final hue texture UV:
        //   X = luminance-based column within the palette row.
        //   Y = row selected by the entity's hue index (from hue_coord.x).
        let hue_uv = in.hue_coord + vec2<f32>(hue_index_uv, 0.0);

        let hue_color = textureSample(hue_texture, hue_sampler, hue_uv);

        // Replace sprite RGB with the palette colour.
        // Multiply alpha: the palette texture can also tint transparency
        // (e.g. semi-transparent clothing effects in UO).
        out_color = vec4<f32>(hue_color.rgb, out_color.a * hue_color.a);
    }

    // ---- 3. Ethereal / ghost blue-shift ----
    // Adds a fixed cool-blue bias to make the entity look "ghostly".
    // Applied regardless of hue so ghosts of hued creatures still look ethereal.
    if (params.is_ethereal == 1u) {
        out_color = vec4<f32>(out_color.rgb + ETHEREAL_BIAS, out_color.a);
    }

    // ---- 4. Alpha modulation by vertex diffuse ----
    // The HLSL comment explains: we only need the diffuse *alpha* because
    // the RGB colour has already been applied via the hue system above.
    // This allows the EC to fade sprites in/out (diffuse_alpha < 1) or
    // apply per-vertex transparency for effects like stealth/invisible.
    out_color.a *= params.diffuse_alpha;

    return out_color;
}
