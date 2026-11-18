// ============================================================================
// kr_sprite_hue.wgsl
//
// WGSL translation of the Kingdom Reborn (KR) sprite hue pixel shader
// (source: uosprite.psh and uospriteui.psh).
//
// ---- WHAT THIS SHADER IS ----
// This is the fragment shader for KR 2D sprites and UI elements.
// It handles:
//   1. SPRITE TEXTURE: samples base texture.
//   2. HUE MASK: optionally samples a mask texture to determine if a pixel
//      should be hued.
//   3. HUE REMAPPING: averages RGB to compute a luma index, which is used
//      to lookup the hue color from a palette texture.
//   4. GRUNGE: optionally multiplies the result by a grunge texture.
//
// ---- KR vs EC DIFFERENCES ----
// - Hue Index Calculation: KR uses an average `(R+G+B)/3` for luma, while
//   EC uses a Rec.709 dot product.
// - Ethereal: KR lacks the ghost/ethereal blue-shift feature.
// - Hue Mask: KR makes explicit use of a hue mask texture (alpha > 0.5).
// - Grunge: KR includes explicit grunge texture multiplication.
// ============================================================================

// ============================================================================
// GPU resource bindings
// ============================================================================

@group(0) @binding(0) var sprite_texture: texture_2d<f32>;
@group(0) @binding(1) var sprite_sampler: sampler;

@group(0) @binding(2) var hue_texture: texture_2d<f32>;
@group(0) @binding(3) var hue_sampler: sampler;

@group(0) @binding(4) var hue_mask_texture: texture_2d<f32>;
@group(0) @binding(5) var hue_mask_sampler: sampler;

@group(0) @binding(6) var grunge_texture: texture_2d<f32>;
@group(0) @binding(7) var grunge_sampler: sampler;

struct KrSpriteParams {
    has_hue:       u32, // 1 = apply hue lookup
    has_huemask:   u32, // 1 = use hue mask texture
    has_grunge:    u32, // 1 = apply grunge texture
    _pad:          u32,

    diffuse_alpha: f32, // Vertex diffuse alpha
    hue_pixel_uv_offset: f32, // = (1.0 / 1024.0) * 255.0 ≈ 0.249023
    _pad2: vec2<f32>,
}
@group(0) @binding(8) var<uniform> params: KrSpriteParams;

// ============================================================================
// Constants
// ============================================================================
const HUE_TEXTURE_HEIGHT: f32 = 1024.0;
const NUM_HUE_PIXELS:     f32 = 255.0;

// ============================================================================
// Vertex output
// ============================================================================

struct VertexOutput {
    @builtin(position)  clip_position: vec4<f32>,
    @location(0)        tex_coord_sprite: vec2<f32>,
    @location(1)        tex_coord_world:  vec2<f32>, // used for grunge
    @location(2)        hue_coord:        vec2<f32>, // offset into hue texture
}

// ============================================================================
// Fragment shader
// ============================================================================

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var out_color = textureSample(sprite_texture, sprite_sampler, in.tex_coord_sprite);

    if (out_color.a <= 0.0) {
        discard;
    }

    if (params.has_hue == 1u && (in.hue_coord.x > 0.0 || in.hue_coord.y > 0.0)) {
        var apply_hue = true;
        
        if (params.has_huemask == 1u) {
            let mask = textureSample(hue_mask_texture, hue_mask_sampler, in.tex_coord_sprite);
            if (mask.a <= 0.5) {
                apply_hue = false;
            }
        }
        
        if (apply_hue) {
            // KR specific: average RGB instead of Rec.709 dot product
            let luma = (out_color.r + out_color.g + out_color.b) / 3.0;
            let hue_index_uv = luma * params.hue_pixel_uv_offset;
            let hue_uv = in.hue_coord + vec2<f32>(hue_index_uv, 0.0);
            let hue_color = textureSample(hue_texture, hue_sampler, hue_uv);

            out_color = vec4<f32>(hue_color.rgb, out_color.a * hue_color.a);
        }
    }

    if (params.has_grunge == 1u) {
        let grunge = textureSample(grunge_texture, grunge_sampler, in.tex_coord_world);
        out_color = vec4<f32>(out_color.rgb * grunge.rgb, out_color.a);
    }

    out_color.a *= params.diffuse_alpha;

    return out_color;
}
