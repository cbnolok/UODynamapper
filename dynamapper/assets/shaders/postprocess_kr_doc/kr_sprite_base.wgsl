// ============================================================================
// kr_sprite_base.wgsl
//
// WGSL translation of the basic KR sprite vertex shaders:
//   • Shaders.uop_B0_F1 ("//#target vs_1_1" — simple world billboard)
//   • uosprite.vsh      ("//#target vs_1_1" — world sprite with grunge/effect UVs)
//   • uospriteui.vsh    ("//#target vs_1_1" — screen-space UI sprite)
//
// ---- KR vs EC DIFFERENCES ----
// - The UI Vertex Shader (`uospriteui.vsh`) calculates the hue texture
//   coordinate internally from a single float `hueIndex`, rather than receiving
//   it as a pre-calculated vertex attribute. It unpacks the 2D coordinate:
//     x = floor(hueIndex / 1024) / 1024 * 255
//     y = fmod(hueIndex, 1024) / 1024
// - The basic world billboard (`F1`) is identical to EC `F2`.
// ============================================================================

// ============================================================================
// ---- VARIANT A & B: World-space vertex shader ----
// ============================================================================

struct WorldVsUniforms {
    world_transform:  mat4x3<f32>,
    view_proj_matrix: mat4x4<f32>,
    sprite_transform: mat3x3<f32>, // called spriteTransform in KR, effectTransform in EC
    grunge_transform: mat3x3<f32>,
}
@group(1) @binding(0) var<uniform> world_vs: WorldVsUniforms;

struct WorldVsInput {
    @location(0) position:   vec4<f32>,
    @location(1) tex_coord0: vec2<f32>,
    @location(2) tex_coord1: vec2<f32>,
    @location(3) tex_coord2: vec2<f32>,
    @location(4) color:      vec4<f32>,
}

struct WorldVsOutputSimple {
    @builtin(position) clip_pos:   vec4<f32>,
    @location(0)       tex_coord0: vec2<f32>,
}

struct WorldVsOutputFull {
    @builtin(position) clip_pos:   vec4<f32>,
    @location(0)       tex_coord0: vec2<f32>,
    @location(1)       tex_coord1: vec2<f32>,
    @location(2)       tex_coord2: vec2<f32>,
    @location(3)       color:      vec4<f32>,
}

// ---- Variant A (F1) ----
@vertex
fn world_sprite_vs(in: WorldVsInput) -> WorldVsOutputSimple {
    var out: WorldVsOutputSimple;
    let world_pos = world_vs.world_transform * in.position;
    out.clip_pos   = world_vs.view_proj_matrix * vec4<f32>(world_pos, 1.0);
    out.tex_coord0 = in.tex_coord0;
    return out;
}

// ---- Variant B (uosprite.vsh) ----
@vertex
fn world_sprite_effect_vs(in: WorldVsInput) -> WorldVsOutputFull {
    var out: WorldVsOutputFull;
    let world_pos = world_vs.world_transform * in.position;
    out.clip_pos = world_vs.view_proj_matrix * vec4<f32>(world_pos, 1.0);
    
    out.tex_coord0 = (world_vs.sprite_transform * vec3<f32>(in.tex_coord0, 1.0)).xy;
    out.tex_coord1 = (world_vs.grunge_transform * vec3<f32>(in.tex_coord1, 1.0)).xy;
    out.tex_coord2 = in.tex_coord2;
    out.color      = in.color;
    
    return out;
}

// ============================================================================
// ---- VARIANT C: Screen-space UI vertex shader (uospriteui.vsh) ----
// ============================================================================

struct ScreenVsUniforms {
    world_transform:  mat4x3<f32>,
    view_proj_matrix: mat4x4<f32>,
    sprite_transform: mat3x3<f32>,
    hue_index:        f32,
}
@group(1) @binding(1) var<uniform> screen_vs: ScreenVsUniforms;

struct ScreenVsInput {
    @location(0) position:   vec4<f32>,
    @location(1) tex_coord0: vec2<f32>,
    @location(2) color:      vec4<f32>,
}

struct ScreenVsOutput {
    @builtin(position) clip_pos:   vec4<f32>,
    @location(0)       tex_coord0: vec2<f32>,
    @location(1)       tex_coord1: vec2<f32>, // hue_coord
    @location(2)       color:      vec4<f32>,
}

const HUE_TEXTURE_HEIGHT: f32 = 1024.0;
const NUM_HUE_PIXELS: f32 = 255.0;

// ---- Variant C (uospriteui.vsh) ----
// Note: In KR, UI sprites still use worldTransform and viewProjMatrix,
// unlike EC which computes NDC coordinates directly from screenParams.
@vertex
fn screen_sprite_ui_vs(in: ScreenVsInput) -> ScreenVsOutput {
    var out: ScreenVsOutput;
    
    let world_pos = screen_vs.world_transform * in.position;
    out.clip_pos = screen_vs.view_proj_matrix * vec4<f32>(world_pos, 1.0);
    
    out.tex_coord0 = (screen_vs.sprite_transform * vec3<f32>(in.tex_coord0, 1.0)).xy;
    out.color = in.color;
    
    // KR specific: dynamically compute hue texture UV coordinates from a scalar index.
    out.tex_coord1.x = floor(screen_vs.hue_index / HUE_TEXTURE_HEIGHT) / HUE_TEXTURE_HEIGHT * NUM_HUE_PIXELS;
    
    // Equivalent to fmod in WGSL is % for integers or `x - y * floor(x/y)`
    let mod_hue = screen_vs.hue_index - HUE_TEXTURE_HEIGHT * floor(screen_vs.hue_index / HUE_TEXTURE_HEIGHT);
    out.tex_coord1.y = mod_hue / HUE_TEXTURE_HEIGHT;
    
    return out;
}
