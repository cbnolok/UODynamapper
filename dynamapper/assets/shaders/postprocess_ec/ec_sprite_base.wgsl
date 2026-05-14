// ============================================================================
// ec_sprite_base.wgsl
//
// WGSL translation of the basic EC sprite vertex + fragment shaders:
//   • Vertex shaders: Shaders.uop_B0_F2, F8  ("//#target vs_1_1")
//                     Shaders.uop_B0_F5      ("//#target vs_1_1" — effect UVs)
//                     Shaders.uop_B0_F10     ("//#target vs_2_0" — screen-space UI)
//   • Fragment shader: Shaders.uop_B0_F0     ("//#target ps_2_0" — colour × texture)
//
// ---- WHAT THESE SHADERS ARE ----
// These are the *simplest* EC rendering shaders — the foundation that
// handles every 2D sprite before any hue/glow/post-process effects.
// They form a small family of closely related variants:
//
//   VARIANT A — "World sprite" (F2 vertex + F0 fragment)
//     Simple world-space billboard: transform position by worldTransform
//     (4×3) then by viewProjMatrix (4×4), pass UVs through, output colour.
//     Applied on: all static sprites, items, effects in the game world.
//
//   VARIANT B — "World sprite with grunge" (F5 vertex + F11 fragment)
//     Same world transform as A, but outputs THREE texture coordinate sets:
//       texCoord0 — sprite UV run through effectTransform  (3×3 affine)
//       texCoord1 — sprite UV run through grungeTransform  (3×3 affine)
//       texCoord2 — pass-through (hue row, carried from vertex data)
//     The extra transforms allow the EC to apply animated UV distortion
//     (wobble/scroll) on top of the base sprite, and a separate grunge
//     detail overlay.
//     Applied on: special effect sprites (fire, explosions, magic),
//                 animated character effects.
//
//   VARIANT C — "Screen-space UI" (F10 vertex + F0 fragment)
//     Operates in screen pixel space, not world space.  The CPU passes
//     a screenParams vec4:
//       x, y = 2/width, 2/height   (NDC scale)
//       z, w = -1 + pixel_offset_x/width, -1 + pixel_offset_y/height  (bias)
//     This maps from pixel coordinates directly to NDC without a view
//     matrix.  Used for all HUD, windows, cursors, and 2D UI elements.
//
//   FRAGMENT (F0) — "Colour × texture"  or  "Flat colour"
//     Two permutations selected at compile time (USE_TEXTURE = 0 or 1):
//       USE_TEXTURE = 1: output = textureSample(base, uv) * vertex_color
//       USE_TEXTURE = 0: output = vertex_color  (no texture at all)
//     Applied on: all variants above that use F0 as their pixel shader.
//
// ---- WHAT THEY ARE APPLIED ON ----
//   • Every non-animated sprite in the world (items, decorations, effects).
//   • The entire 2D user interface (HUD panels, buttons, gump windows).
//   • Animated spell / particle effects (variant B with UV transforms).
//
// ---- WGSL NOTE ----
// HLSL uses row-major matrices and mul(vector, matrix) convention.
// WGSL uses column-major matrices and matrix * vector convention.
// The worldTransform in HLSL is float4x3 (4 rows × 3 cols) — a world-to-
// world affine transform applied as mul(pos4, worldTransform) which gives a
// vec3.  In WGSL we express the same transform as a mat4x3 (4 columns × 3
// rows) and use worldTransform * vec4 instead.  The CPU upload must
// transpose accordingly.
// ============================================================================


// ============================================================================
// ---- VARIANT A / B: World-space vertex shader ----
// (F2 = no UV transform, F5 = with effectTransform + grungeTransform)
// ============================================================================

// Bindings for the world vertex shader.
// In Bevy this would come from the mesh's BindGroup (group 0 = view, group 1
// = mesh / instance), but here we list them as uniforms for clarity.
struct WorldVsUniforms {
    // World-to-world affine transform (typically identity or billboard rotation).
    // Original HLSL: float4x3 worldTransform → mat3x4 in WGSL column-major.
    world_transform:  mat4x3<f32>, // 4 columns, 3 rows → transforms vec4 → vec3

    // Combined view+projection matrix (VP, not MVP — world handled above).
    view_proj_matrix: mat4x4<f32>,

    // ---- Only used by Variant B (F5) ----
    // 3×3 affine UV transforms.  The HLSL multiplies float3(uv, 1) by these
    // to allow scrolling, rotation, and scale of the UV coordinates.
    effect_transform: mat3x3<f32>, // effectTransform
    grunge_transform: mat3x3<f32>, // grungeTransform
}

@group(1) @binding(0) var<uniform> world_vs: WorldVsUniforms;

// Input from the CPU vertex buffer.
struct WorldVsInput {
    @location(0) position:   vec4<f32>, // object-space position
    @location(1) tex_coord0: vec2<f32>, // sprite UV
    @location(2) tex_coord1: vec2<f32>, // secondary UV (world-space / grunge)
    @location(3) tex_coord2: vec2<f32>, // hue row UV (pass-through)
    @location(4) color:      vec4<f32>, // vertex diffuse colour + alpha
}

// Variant A output: two UVs + colour.
struct WorldVsOutputSimple {
    @builtin(position) clip_pos:    vec4<f32>,
    @location(0)       tex_coord0:  vec2<f32>, // sprite UV
    @location(1)       color:       vec4<f32>, // diffuse
}

// Variant B output: three UVs + colour (for hue shader F11).
struct WorldVsOutputFull {
    @builtin(position) clip_pos:    vec4<f32>,
    @location(0)       tex_coord0:  vec2<f32>, // sprite UV after effectTransform
    @location(1)       tex_coord1:  vec2<f32>, // grunge UV after grungeTransform
    @location(2)       tex_coord2:  vec2<f32>, // hue row (pass-through)
    @location(3)       color:       vec4<f32>, // diffuse
}

// ---- Variant A vertex shader (F2) ----
// Simple world sprite — position → world → clip, UVs pass-through.
@vertex
fn world_sprite_vs(in: WorldVsInput) -> WorldVsOutputSimple {
    var out: WorldVsOutputSimple;

    // Transform object-space position to world space (affine 4×3 → vec3).
    // HLSL: float3 worldPosition = mul(IN.position, worldTransform)
    let world_pos = world_vs.world_transform * in.position;

    // Project world position to clip space.
    // HLSL: OUT.screenPosition = mul(float4(worldPosition, 1), viewProjMatrix)
    out.clip_pos   = world_vs.view_proj_matrix * vec4<f32>(world_pos, 1.0);
    out.tex_coord0 = in.tex_coord0;
    out.color      = in.color;

    return out;
}

// ---- Variant B vertex shader (F5) ----
// Effect sprite — applies 3×3 UV transforms for animated effects and grunge.
@vertex
fn world_sprite_effect_vs(in: WorldVsInput) -> WorldVsOutputFull {
    var out: WorldVsOutputFull;

    let world_pos = world_vs.world_transform * in.position;
    out.clip_pos = world_vs.view_proj_matrix * vec4<f32>(world_pos, 1.0);

    // Apply effectTransform to sprite UV (scroll / rotate / scale the effect).
    // HLSL: OUT.texCoord0.xy = mul(float3(IN.texCoord0, 1), effectTransform)
    // We embed the homogeneous 1 in the z channel and extract xy.
    out.tex_coord0 = (world_vs.effect_transform * vec3<f32>(in.tex_coord0, 1.0)).xy;

    // Apply grungeTransform to secondary UV (separate detail-overlay scale).
    out.tex_coord1 = (world_vs.grunge_transform * vec3<f32>(in.tex_coord1, 1.0)).xy;

    // Hue row UV passes through untouched.
    out.tex_coord2 = in.tex_coord2;
    out.color      = in.color;

    return out;
}


// ============================================================================
// ---- VARIANT C: Screen-space UI vertex shader (F10) ----
// ============================================================================

// Screen-space parameters uploaded by the CPU:
//   x = 2.0 / render_target_width
//   y = 2.0 / render_target_height
//   z = pixel_offset_x * x - 1.0   (left-edge NDC bias)
//   w = pixel_offset_y * y - 1.0   (top-edge NDC bias)
struct ScreenVsUniforms {
    screen_params: vec4<f32>,
    // Flat colour applied when USE_VERTEX_COLORS == 0.
    flat_color:    vec4<f32>,
}

@group(1) @binding(0) var<uniform> screen_vs: ScreenVsUniforms;

struct ScreenVsInput {
    @location(0) position:   vec4<f32>, // pixel-space position (XY used, ZW unused)
    @location(1) tex_coord0: vec2<f32>, // sprite UV
    @location(2) color:      vec4<f32>, // per-vertex colour (USE_VERTEX_COLORS = 1)
}

struct ScreenVsOutput {
    @builtin(position) clip_pos:   vec4<f32>,
    @location(0)       tex_coord0: vec2<f32>,
    @location(1)       color:      vec4<f32>,
}

// ---- Screen-space UI vertex shader (F10) ----
// Converts pixel coordinates to NDC without any world/view matrix.
// Set use_vertex_colors = true to use per-vertex colour (e.g. gradients);
// set it to false to apply a uniform flat colour from the uniform block.
@vertex
fn screen_sprite_vs(in: ScreenVsInput,
                    @builtin(vertex_index) vi: u32,
                    // Passed as push-constant or specialisation constant
                    // if the pipeline supports it; here as a flag in the UBO.
                    ) -> ScreenVsOutput {
    var out: ScreenVsOutput;

    // HLSL: OUT.position.x = (IN.position.x * screenParams.x + screenParams.z) - 1
    // HLSL: OUT.position.y = 1 - (IN.position.y * screenParams.y + screenParams.w)
    // Note the Y flip: HLSL clip-space Y grows upward from −1; screen pixels
    // grow downward, so we subtract from 1 to match.
    let ndcx = in.position.x * screen_vs.screen_params.x + screen_vs.screen_params.z - 1.0;
    let ndcy = 1.0 - (in.position.y * screen_vs.screen_params.y + screen_vs.screen_params.w);
    out.clip_pos = vec4<f32>(ndcx, ndcy, 0.0, 1.0);

    out.tex_coord0 = in.tex_coord0;

    // For this reference translation we always supply per-vertex colour.
    // When USE_VERTEX_COLORS == 0 in the original HLSL, the CPU uploads a
    // uniform flat colour — replicate that by setting all vertex colours to
    // screen_vs.flat_color on the CPU side, or swap in a specialised pipeline.
    out.color = in.color;

    return out;
}


// ============================================================================
// ---- FRAGMENT shader (F0) — Colour × Texture  or  Flat colour ----
//
// Applied by all three vertex variants above (A, B, C).
// Note: Variant B typically uses the hue fragment shader (ec_sprite_hue.wgsl)
// instead of this one; F0 is used only for non-hued sprites.
// ============================================================================

@group(0) @binding(0) var base_texture: texture_2d<f32>;
@group(0) @binding(1) var base_sampler: sampler;

// Runtime flag replacing the compile-time USE_TEXTURE #define.
struct SpriteFragParams {
    use_texture: u32,   // 1 = sample texture; 0 = flat vertex colour only
    _pad: vec3<u32>,
}
@group(0) @binding(2) var<uniform> frag_params: SpriteFragParams;

// The fragment inputs differ between variants A/C (no tex_coord2/color at
// location 2/3) and variant B.  WGSL does not support function overloading,
// so we define a "maximal" input and rely on the pipeline to only connect
// the locations that the linked vertex shader actually outputs.
struct SpriteFragInput {
    @builtin(position) clip_pos:   vec4<f32>,
    @location(0)       tex_coord0: vec2<f32>,
    @location(1)       color:      vec4<f32>,
}

@fragment
fn sprite_fragment(in: SpriteFragInput) -> @location(0) vec4<f32> {
    if (frag_params.use_texture == 1u) {
        // Modulate texture sample by vertex diffuse colour.
        // This is the standard UO sprite rendering path:
        //   vertex colour carries the entity's tint (e.g. partial visibility,
        //   day-night modulation) while the texture carries the artwork.
        let tex_color = textureSample(base_texture, base_sampler, in.tex_coord0);
        return tex_color * in.color;
    } else {
        // No texture — flat vertex colour (used by solid-colour UI primitives
        // such as rectangle fills, selection boxes, etc.).
        return in.color;
    }
}
