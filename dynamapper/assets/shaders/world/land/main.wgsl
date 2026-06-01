// ============================================================================
// Bevy PBR WGSL — Terrain entry point (Three modes: 0 Classic, 1 Enhanced, 2 KR-like)
//
//  - Mode 0: "2D Classic"   — faithful brightness model, vertex/Gouraud.
//  - Mode 1: "2D Enhanced"  — per-fragment, subtle improvements.
//  - Mode 2: "KR-like"      — painterly: warm key, cool ambient/fill, vibrant grading.
//
// This file contains ONLY the @vertex and @fragment entry points.
// All helpers live in the co-located module files and are pulled in via #import.
//  https://wgmath.rs/docs/user_guides/wgcore/shaders_composition/
// ============================================================================

#import bevy_pbr::{
  forward_io::{Vertex, VertexOutput},
  mesh_functions,
  mesh_view_bindings::globals,
  view_transformations,
}

// ---- Land shader modules (quoted asset-path imports for on-demand loading) ----
#import "shaders/world/common_bindings.wgsl"::{
  SceneUniform, LandEffectsUniform, GlobalLightingUniforms, USE_VOLUMETRIC_NOISE
}
#import "shaders/world/land/land_bindings.wgsl"::{
  TileUniform, AtlasParams, LandLightingUniforms,
  tex_small_sampler, tex_small, tex_big, tile_meta_atlas,
  ATLAS, scene, effects, global_light, land_light
}
#import "shaders/world/land/atlas.wgsl"::{atlas_read_meta, atlas_read_height, chunk_edge_blend_factor}
#import "shaders/world/land/noise.wgsl"::{hash, fbm_billow, domain_warp}
#import "shaders/world/land/normals.wgsl"::{get_geometric_normal_local, get_bicubic_normal, get_bent_normal}
#import "shaders/postprocess/color_grading.wgsl"::{grade_color_vibrant}
#import "shaders/postprocess/global_lighting.wgsl"::{apply_global_lighting_rgb}
#import "shaders/postprocess/grunge.wgsl"::{apply_shadow_aware_land_grunge}
#import "shaders/postprocess/tonemapping.wgsl"::{tonemap_ec_kr_profile}
#import "shaders/world/land/sampling.wgsl"::{
  ec_world_uv,
  ec_material_has_liquid_normal,
  ec_liquid_perturbed_base_uv,
  sample_tile_albedo,
  sample_ec_material_albedo,
  sample_ec_material_albedo_at_world,
  apply_sharpening, blurred_albedo,
}
#import "shaders/world/land/shading.wgsl"::{
  shade_mode0_classic_vertex,
  shade_mode1_enhanced_fragment,
  shade_mode2_kr_fragment,
  apply_static_light_decals,
}
#import "shaders/world/effects/surface_effects.wgsl"::apply_water_animation

// ============================================================================
// Vertex shader
//  - Displaces the flat grid by per-tile height from the atlas.
//  - Computes the normal (geometric or bicubic), optionally bent.
//  - For Classic mode (0) pre-bakes the Lambert term into uv_b.x (Gouraud).
// ============================================================================

const TERRAIN_FLAG_REVIEWED_LIQUID: u32 = 0x4u;
const EC_TERRAIN_TRANSITION_WIDTH: f32 = 0.82;
const EC_TERRAIN_TRANSITION_BASE_WEIGHT: f32 = 0.35;

fn apply_kr_liquid_material_response(color: vec3<f32>,
                                     base_albedo: vec3<f32>,
                                     world_pos: vec3<f32>,
                                     Nw: vec3<f32>,
                                     V: vec3<f32>,
                                     L: vec3<f32>,
                                     tile: TileUniform,
                                     specular_strength: f32,
                                     enable_water: u32) -> vec3<f32> {
  let is_liquid = tile.is_wet == 1u || (tile.terrain_flags & TERRAIN_FLAG_REVIEWED_LIQUID) != 0u;
  if (!is_liquid || specular_strength <= 0.0001) {
    return color;
  }

  let N = normalize(Nw);
  let view_dir = normalize(V);
  let light_dir = normalize(L);
  let H = normalize(view_dir + light_dir);
  let view_glance = pow(max(1.0 - dot(N, view_dir), 0.0), 2.0);
  let sun_glint = pow(max(dot(N, H), 0.0), 22.0);
  let phase_time = select(0.0, globals.time * 1.35, enable_water == 1u);
  let ripple = 0.72 + 0.28 * sin(dot(world_pos.xz, vec2<f32>(1.7, 2.3)) + phase_time);

  let warm_liquid = clamp(base_albedo.r - max(base_albedo.g, base_albedo.b), 0.0, 1.0);
  let cool_tint = vec3<f32>(0.70, 0.92, 1.04);
  let warm_tint = vec3<f32>(1.08, 0.58, 0.24);
  let tint = mix(cool_tint, warm_tint, warm_liquid);
  let response = (sun_glint * 0.55 + view_glance * 0.18) * ripple;
  return color + tint * response * clamp(specular_strength, 0.0, 0.25);
}

fn ec_transition_edge_weight(edge_distance: f32) -> f32 {
  return 1.0 - smoothstep(0.0, EC_TERRAIN_TRANSITION_WIDTH, edge_distance);
}

fn ec_transition_neighbor_weight(offset: vec2<i32>, uv_in_tile: vec2<f32>) -> f32 {
  var weight = 0.0;

  if (offset.x < 0) {
    weight = max(weight, ec_transition_edge_weight(uv_in_tile.x));
  } else if (offset.x > 0) {
    weight = max(weight, ec_transition_edge_weight(1.0 - uv_in_tile.x));
  }

  if (offset.y < 0) {
    weight = max(weight, ec_transition_edge_weight(uv_in_tile.y));
  } else if (offset.y > 0) {
    weight = max(weight, ec_transition_edge_weight(1.0 - uv_in_tile.y));
  }

  return weight;
}

fn ec_transition_boundary_neighbor_weight(
  current_tile: TileUniform,
  neighbor_tile: TileUniform,
  offset: vec2<i32>,
  uv_in_tile: vec2<f32>,
) -> f32 {
  if (neighbor_tile.texture_size != 2u) {
    return 0.0;
  }

  if (neighbor_tile.texture_payload == current_tile.texture_payload) {
    return 0.0;
  }

  if (neighbor_tile.texture_extent.x == 0u || neighbor_tile.texture_extent.y == 0u) {
    return 0.0;
  }

  return ec_transition_neighbor_weight(offset, uv_in_tile);
}

fn ec_transition_neighbor_color(
  current_tile: TileUniform,
  neighbor_tile: TileUniform,
  world_xz: vec2<f32>,
  base_color: vec3<f32>,
  weight: f32,
) -> vec4<f32> {
  if (weight <= 0.0 || neighbor_tile.texture_size != 2u) {
    return vec4<f32>(base_color, 0.0);
  }

  if (neighbor_tile.texture_payload == current_tile.texture_payload) {
    return vec4<f32>(base_color, 0.0);
  }

  if (neighbor_tile.texture_extent.x == 0u || neighbor_tile.texture_extent.y == 0u) {
    return vec4<f32>(base_color, 0.0);
  }

  return vec4<f32>(sample_ec_material_albedo_at_world(world_xz, neighbor_tile), weight);
}

fn ec_transition_diagonal_weight(offset: vec2<i32>, uv_in_tile: vec2<f32>) -> f32 {
  let x_weight = ec_transition_neighbor_weight(vec2<i32>(offset.x, 0), uv_in_tile);
  let y_weight = ec_transition_neighbor_weight(vec2<i32>(0, offset.y), uv_in_tile);
  return min(x_weight, y_weight) * 0.75;
}

fn ec_transition_boundary_diagonal_weight(
  current_tile: TileUniform,
  neighbor_tile: TileUniform,
  offset: vec2<i32>,
  uv_in_tile: vec2<f32>,
) -> f32 {
  if (neighbor_tile.texture_size != 2u) {
    return 0.0;
  }

  if (neighbor_tile.texture_payload == current_tile.texture_payload) {
    return 0.0;
  }

  if (neighbor_tile.texture_extent.x == 0u || neighbor_tile.texture_extent.y == 0u) {
    return 0.0;
  }

  return ec_transition_diagonal_weight(offset, uv_in_tile);
}

fn ec_transition_boundary_weight(
  current_tile: TileUniform,
  world_tile: vec2<i32>,
  uv_in_tile: vec2<f32>,
) -> f32 {
  if (current_tile.texture_size != 2u) {
    return 0.0;
  }

  var weight = 0.0;
  weight = max(weight, ec_transition_boundary_neighbor_weight(current_tile, atlas_read_meta(world_tile.x - 1, world_tile.y), vec2<i32>(-1, 0), uv_in_tile));
  weight = max(weight, ec_transition_boundary_neighbor_weight(current_tile, atlas_read_meta(world_tile.x + 1, world_tile.y), vec2<i32>(1, 0), uv_in_tile));
  weight = max(weight, ec_transition_boundary_neighbor_weight(current_tile, atlas_read_meta(world_tile.x, world_tile.y - 1), vec2<i32>(0, -1), uv_in_tile));
  weight = max(weight, ec_transition_boundary_neighbor_weight(current_tile, atlas_read_meta(world_tile.x, world_tile.y + 1), vec2<i32>(0, 1), uv_in_tile));
  weight = max(weight, ec_transition_boundary_diagonal_weight(current_tile, atlas_read_meta(world_tile.x - 1, world_tile.y - 1), vec2<i32>(-1, -1), uv_in_tile));
  weight = max(weight, ec_transition_boundary_diagonal_weight(current_tile, atlas_read_meta(world_tile.x + 1, world_tile.y - 1), vec2<i32>(1, -1), uv_in_tile));
  weight = max(weight, ec_transition_boundary_diagonal_weight(current_tile, atlas_read_meta(world_tile.x - 1, world_tile.y + 1), vec2<i32>(-1, 1), uv_in_tile));
  weight = max(weight, ec_transition_boundary_diagonal_weight(current_tile, atlas_read_meta(world_tile.x + 1, world_tile.y + 1), vec2<i32>(1, 1), uv_in_tile));
  return clamp(weight, 0.0, 1.0);
}

fn apply_kr_transition_grime(
  color: vec3<f32>,
  current_tile: TileUniform,
  world_tile: vec2<i32>,
  uv_in_tile: vec2<f32>,
  world_xz: vec2<f32>,
  strength: f32,
) -> vec3<f32> {
  let boundary = ec_transition_boundary_weight(current_tile, world_tile, uv_in_tile);
  let grime_strength = clamp(strength, 0.0, 1.0);
  if (boundary <= 0.0001 || grime_strength <= 0.0001) {
    return color;
  }

  let breakup = 0.78 + 0.22 * hash(floor(world_xz * 2.0));
  let amount = boundary * grime_strength * breakup;
  return mix(color, color * vec3<f32>(0.74, 0.70, 0.60), amount * 0.32);
}

fn apply_kr_material_micro_contrast(
  color: vec3<f32>,
  tile: TileUniform,
  world_xz: vec2<f32>,
  strength: f32,
) -> vec3<f32> {
  let material_strength = clamp(strength, 0.0, 1.5);
  let is_liquid = tile.is_wet == 1u || (tile.terrain_flags & TERRAIN_FLAG_REVIEWED_LIQUID) != 0u;
  if (material_strength <= 0.0001 || is_liquid) {
    return color;
  }

  let luma = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
  let max_channel = max(color.r, max(color.g, color.b));
  let min_channel = min(color.r, min(color.g, color.b));
  let chroma = max_channel - min_channel;
  let green_bias = clamp(color.g - max(color.r, color.b), 0.0, 1.0);
  let cool_bright = smoothstep(0.58, 0.90, luma) * smoothstep(0.00, 0.16, color.b - color.r);
  let foliage_like = smoothstep(0.03, 0.18, green_bias) * smoothstep(0.10, 0.42, chroma);
  let stone_like = (1.0 - foliage_like) * (1.0 - cool_bright) * (1.0 - smoothstep(0.46, 0.82, chroma));
  let material_mask = clamp(cool_bright * 0.75 + foliage_like * 0.55 + stone_like * 0.42, 0.0, 1.0);
  if (material_mask <= 0.0001) {
    return color;
  }

  let grain = hash(floor(world_xz * 2.7) + vec2<f32>(f32(tile.texture_payload & 0xFFu), f32((tile.texture_payload >> 8u) & 0xFFu)));
  let contrast = 1.0 + material_strength * material_mask * (0.045 + (grain - 0.5) * 0.025);
  let saturated = vec3<f32>(luma) + (color - vec3<f32>(luma)) * (1.0 + material_strength * material_mask * 0.06);
  let shaped = vec3<f32>(luma) + (saturated - vec3<f32>(luma)) * contrast;
  return max(shaped, vec3<f32>(0.0));
}

fn apply_kr_land_atmosphere_depth(
  color: vec3<f32>,
  world_pos: vec3<f32>,
  camera_pos: vec3<f32>,
  normal_world: vec3<f32>,
  enable_fog: u32,
  visual_profile: u32,
) -> vec3<f32> {
  if (enable_fog != 1u || visual_profile != 2u) {
    return color;
  }

  let dist_density = clamp(global_light.fog_params.x, 0.0, 1.0);
  let height_density = clamp(global_light.fog_params.y, 0.0, 1.0);
  if (dist_density <= 0.0001 && height_density <= 0.0001) {
    return color;
  }

  let dist_tiles = distance(world_pos.xz, camera_pos.xz);
  let distance_term = smoothstep(96.0, 520.0, dist_tiles) * dist_density * 5.5;
  let height_term = smoothstep(2.0, 14.0, max(world_pos.y, 0.0)) * height_density * 10.0;
  let slope_term = clamp(1.0 - dot(normalize(normal_world), vec3<f32>(0.0, 1.0, 0.0)), 0.0, 1.0) * height_density * 3.0;

  let fog_cap = clamp(global_light.fog_color.a, 0.0, 0.45);
  let amount = clamp((distance_term + height_term + slope_term) * fog_cap * 0.38, 0.0, 0.14);
  if (amount <= 0.0001) {
    return color;
  }

  let night_blend = clamp(global_light.fog_night_color.a, 0.0, 1.0);
  let fog_tint = mix(global_light.fog_color.rgb, global_light.fog_night_color.rgb, night_blend);
  let atmosphere_tint = mix(global_light.atmosphere_tint, fog_tint, 0.25);
  let luma = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
  let softened = mix(color, vec3<f32>(luma), amount * 0.22);
  return max(mix(softened, atmosphere_tint, amount), vec3<f32>(0.0));
}

fn kr_height_contact_shadow(world_tile: vec2<i32>, center_height: f32, strength: f32) -> f32 {
  let relief_strength = clamp(strength, 0.0, 1.5);
  if (relief_strength <= 0.0001) {
    return 0.0;
  }

  let west = atlas_read_height(world_tile.x - 1, world_tile.y);
  let east = atlas_read_height(world_tile.x + 1, world_tile.y);
  let north = atlas_read_height(world_tile.x, world_tile.y - 1);
  let south = atlas_read_height(world_tile.x, world_tile.y + 1);
  let cover_height = max(max(west, east), max(north, south)) - center_height;
  return smoothstep(0.04, 0.55, max(cover_height, 0.0)) * relief_strength;
}

fn blend_ec_terrain_transitions(
  base_color: vec3<f32>,
  current_tile: TileUniform,
  world_tile: vec2<i32>,
  world_xz: vec2<f32>,
  uv_in_tile: vec2<f32>,
) -> vec3<f32> {
  if (current_tile.texture_size != 2u) {
    return base_color;
  }

  var accum = base_color * EC_TERRAIN_TRANSITION_BASE_WEIGHT;
  var total_weight = EC_TERRAIN_TRANSITION_BASE_WEIGHT;

  let west_offset = vec2<i32>(-1, 0);
  let east_offset = vec2<i32>(1, 0);
  let north_offset = vec2<i32>(0, -1);
  let south_offset = vec2<i32>(0, 1);
  let northwest_offset = vec2<i32>(-1, -1);
  let northeast_offset = vec2<i32>(1, -1);
  let southwest_offset = vec2<i32>(-1, 1);
  let southeast_offset = vec2<i32>(1, 1);

  let west = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x - 1, world_tile.y),
    world_xz,
    base_color,
    ec_transition_neighbor_weight(west_offset, uv_in_tile),
  );
  accum += west.rgb * west.a;
  total_weight += west.a;

  let east = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x + 1, world_tile.y),
    world_xz,
    base_color,
    ec_transition_neighbor_weight(east_offset, uv_in_tile),
  );
  accum += east.rgb * east.a;
  total_weight += east.a;

  let north = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x, world_tile.y - 1),
    world_xz,
    base_color,
    ec_transition_neighbor_weight(north_offset, uv_in_tile),
  );
  accum += north.rgb * north.a;
  total_weight += north.a;

  let south = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x, world_tile.y + 1),
    world_xz,
    base_color,
    ec_transition_neighbor_weight(south_offset, uv_in_tile),
  );
  accum += south.rgb * south.a;
  total_weight += south.a;

  let northwest = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x - 1, world_tile.y - 1),
    world_xz,
    base_color,
    ec_transition_diagonal_weight(northwest_offset, uv_in_tile),
  );
  accum += northwest.rgb * northwest.a;
  total_weight += northwest.a;

  let northeast = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x + 1, world_tile.y - 1),
    world_xz,
    base_color,
    ec_transition_diagonal_weight(northeast_offset, uv_in_tile),
  );
  accum += northeast.rgb * northeast.a;
  total_weight += northeast.a;

  let southwest = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x - 1, world_tile.y + 1),
    world_xz,
    base_color,
    ec_transition_diagonal_weight(southwest_offset, uv_in_tile),
  );
  accum += southwest.rgb * southwest.a;
  total_weight += southwest.a;

  let southeast = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x + 1, world_tile.y + 1),
    world_xz,
    base_color,
    ec_transition_diagonal_weight(southeast_offset, uv_in_tile),
  );
  accum += southeast.rgb * southeast.a;
  total_weight += southeast.a;

  return accum / max(total_weight, 0.0001);
}

@vertex
fn vertex(in: Vertex, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
  var out: VertexOutput;

  let shading_mode: u32 = effects.shading_mode;
  let normal_mode:  u32 = effects.normal_mode;
  let enable_bent:  u32 = land_light.enable_bent;

  // Apply mesh local_to_world ON THE FLAT GRID FIRST to get actual world tile coords.
  // We need the world-space XZ before adding height so atlas_read_height works correctly.
  let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
  var flat_local_pos = in.position;
  flat_local_pos.y = 0.0;
  let flat_world_pos = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(flat_local_pos, 1.0));

  let wx = i32(round(flat_world_pos.x));
  let wz = i32(round(flat_world_pos.z));
  let final_y = atlas_read_height(wx, wz);

  var final_world_pos = flat_world_pos;
  final_world_pos.y += final_y;

  out.world_position = final_world_pos;
  out.position       = view_transformations::position_world_to_clip(out.world_position.xyz);
  out.uv = in.uv;
  out.instance_index = in.instance_index;

  // Base geometric normal (fast central-difference)
  let geometric_normal_local = get_geometric_normal_local(out.world_position.xyz);
  var Nw = mesh_functions::mesh_normal_local_to_world(geometric_normal_local, in.instance_index);

  // Optional smooth/bicubic normal with edge blend to avoid inter-chunk seams
  if (normal_mode == 1u) {
    let smooth_local = get_bicubic_normal(out.world_position.xyz);
    let smooth_world = mesh_functions::mesh_normal_local_to_world(smooth_local, in.instance_index);
    let blend_edge = chunk_edge_blend_factor(out.world_position.x, out.world_position.z);
    Nw = normalize(mix(smooth_world, Nw, blend_edge));
  }

  // Optional bent normal (same occlusion-proxy logic reused in fragment path)
  if (enable_bent == 1u) {
    Nw = get_bent_normal(out.world_position.xyz, Nw);
  }

  out.world_normal = Nw;

  // Classic vertex path: pre-compute Lambert in uv_b.x using the final Nw
  out.uv_b = vec2<f32>(0.0, 0.0);
  if (shading_mode == 0u) {
    out.uv_b.x = max(dot(normalize(Nw), scene.light_direction), 0.0);
  }

  return out;
}

// ============================================================================
// Fragment shader
// ============================================================================

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
  let shading_mode   = effects.shading_mode;
  let enable_tonemap = global_light.enable_tonemap;
  let enable_grading = global_light.enable_grading;
  let visual_profile = effects.post_process_profile;

  // Debug mode: return a flat color immediately to isolate non-fragment bottlenecks.
  if (shading_mode == 3u) {
    return vec4<f32>(0.5, 0.5, 0.5, 1.0);
  }

  // ---- Zoom-based shader LOD: disable expensive features when zoomed out ----
  // zoom > 10 : disable blur + sharpening + bicubic normals (saves ~30 tex reads)
  // zoom > 20 : disable bent normals (saves ~4 tex reads)
  // zoom > 30 : disable volumetric fog → flat fog (saves heavy FBM ALU)
  let zoom = scene.render_zoom;
  var normal_mode    = effects.normal_mode;
  var enable_bent    = land_light.enable_bent;
  var enable_fog     = global_light.enable_fog;
  var enable_gloom   = global_light.enable_gloom;
  var enable_blur    = effects.enable_blur;
  var use_volumetric_fog = USE_VOLUMETRIC_NOISE == 1u;
  var disable_sharpen = false;

  if (zoom > 10.0) {
    enable_blur = 0u;      // skip 9-tap blur
    disable_sharpen = true; // skip sharpening (4 extra taps)
    normal_mode = 0u;       // geometric normals only (skip bicubic 4×4 kernel)
  }
  if (zoom > 20.0) {
    enable_bent = 0u;       // skip bent normals (4 extra height reads)
  }
  if (zoom > 30.0) {
    use_volumetric_fog = false; // skip domain-warped FBM fog when zoomed far out
  }

  // Water animation: disable at high zoom where individual tiles are sub-pixel.
  var enable_water = effects.enable_water_animation;
  if (zoom > 10.0) {
    enable_water = 0u;
  }

  let ambient_strength  = global_light.ambient_strength;
  let diffuse_strength  = land_light.diffuse_strength;
  let specular_strength = land_light.specular_strength;
  let rim_strength      = land_light.rim_strength;

  let fill_strength     = land_light.fill_strength;
  let sharpness_factor  = land_light.sharpness_factor;
  let sharpness_mix     = land_light.sharpness_mix;

  let blur_strength     = effects.blur_strength;
  let blur_radius       = effects.blur_radius;

  let exposure          = global_light.exposure;

  // Local UV (fractional position within the current tile, in [0,1))
  var uv_in_tile = vec2<f32>(fract(in.world_position.x), fract(in.world_position.z));
  let world_tile = vec2<i32>(i32(floor(in.world_position.x)), i32(floor(in.world_position.z)));
  let tile = atlas_read_meta(world_tile.x, world_tile.y);

  // ---- Animated water ----
  // Apply sin/cos UV distortion to tiles with the IsWet tiledata flag.
  // The distortion breathes the sampled UV region slightly larger than 1.0,
  // creating a gentle wavy appearance (faithful port of ClassicUO's formula).
  // Skipped at high zoom where individual tiles are sub-pixel (already gated above).
  let reviewed_liquid = (tile.terrain_flags & TERRAIN_FLAG_REVIEWED_LIQUID) != 0u;
  let ec_liquid_normal = tile.texture_size == 2u && reviewed_liquid && ec_material_has_liquid_normal(tile);
  if (enable_water == 1u && (tile.is_wet == 1u || reviewed_liquid) && !ec_liquid_normal) {
    uv_in_tile = apply_water_animation(uv_in_tile, vec2<f32>(0.5, 0.5));
  }

  // ---- EC world-space UV ----
  // For Enhanced Client (EC) land textures (texture_size == 2), the texture is
  // NOT mapped one-to-one per tile.  EC textures tile across multiple world tiles
  // using world-space coordinates divided by a stretch factor derived from the
  // texture's pixel width (stretch = texture_extent.x / CC_TILE_PX = 44 px).
  // Classic Client textures (size 0/1) still use the per-tile uv_in_tile.
  // Water-distorted uv_in_tile is intentionally preserved for CC/wet tiles;
  // EC wet tiles use distorted world UVs so the wave effect is consistent.
  var sample_uv = uv_in_tile;
  if (tile.texture_size == 2u) {
    // Compute world-space tiling UV.  We use the (possibly water-distorted)
    // uv_in_tile offset so water animation stays coherent with EC textures too.
    let world_xz = vec2<f32>(floor(in.world_position.x), floor(in.world_position.z)) + uv_in_tile;
    sample_uv = ec_world_uv(world_xz, tile);
    if (enable_water == 1u && ec_liquid_normal) {
      sample_uv = ec_liquid_perturbed_base_uv(world_xz, sample_uv, tile, globals.time);
    }
  }

  let base_alpha: f32 = 1.0; // tile textures assumed opaque for terrain

  // ---- Zoom-adaptive cheap path ----
  // Skips expensive reconstruction/blur/sharpen/lighting for a fraction of
  // pixels at high zoom-out, saving GPU time.
  // IMPORTANT: this branch happens BEFORE expensive ops, so skipped pixels
  // are truly cheaper to compute.
  let adaptive = clamp(scene.adaptive_zoom_simplification, 0.0, 1.0);
  var use_cheap_path = false;
  if (adaptive > 0.001) {
    let px = i32(in.position.x);
    let py = i32(in.position.y);
    var skip_expensive = false;

    if (adaptive < 0.5) {
      // 50% checkerboard keep-rate.
      skip_expensive = ((px + py) & 1) != 0;
    } else {
      // ~25% keep-rate: only one pixel every 2×2 quad keeps full shading.
      skip_expensive = ((px & 1) != 0) || ((py & 1) != 0);
    }

    if (skip_expensive) {
      use_cheap_path = true;
    }
  }

  // ---- Base albedo ----
  var base_albedo = vec3<f32>(0.0);
  if (use_cheap_path) {
    // Quantize UV to a coarser grid and use direct nearest sample.
    // For EC tiles sample_uv is already world-space; we quantize in the same space.
    // We still run full lighting/fog/shadows below to keep visual consistency.
    let cells = mix(2.0, 4.0, smoothstep(0.0, 1.0, adaptive));
    let uv_q = (floor(sample_uv * cells) + vec2<f32>(0.5)) / cells;
    if (tile.texture_size == 2u) {
      let world_xz = vec2<f32>(floor(in.world_position.x), floor(in.world_position.z)) + uv_in_tile;
      base_albedo = sample_ec_material_albedo(world_xz, uv_q, tile);
    } else {
      base_albedo = sample_tile_albedo(uv_q, tile);
    }
  } else {
    if (tile.texture_size == 2u) {
      let world_xz = vec2<f32>(floor(in.world_position.x), floor(in.world_position.z)) + uv_in_tile;
      base_albedo = sample_ec_material_albedo(world_xz, sample_uv, tile);
      base_albedo = blend_ec_terrain_transitions(
        base_albedo,
        tile,
        world_tile,
        world_xz,
        uv_in_tile,
      );
    } else {
      base_albedo = sample_tile_albedo(sample_uv, tile);
    }
    if (enable_blur == 1u && blur_strength > 0.001 && blur_radius > 0.0) {
      let blurred = blurred_albedo(sample_uv, tile, blur_radius, vec2<f32>(in.world_position.x, in.world_position.z));
      base_albedo = mix(base_albedo, blurred, clamp(blur_strength, 0.0, 1.0));
    }
    if (!disable_sharpen && effects.sharpening_amount > 0.0) {
      base_albedo = apply_sharpening(base_albedo, sample_uv, tile, effects.sharpening_amount);
    }
  }
  // ---- Normals ----
  // We already computed in the vertex shader and passed in.world_normal.
  // For non-classic modes we can still override with bicubic if desired.
  var Nw = normalize(in.world_normal);
  if (normal_mode == 1u) {
    let smooth_local = get_bicubic_normal(in.world_position.xyz);
    let smooth_world = mesh_functions::mesh_normal_local_to_world(smooth_local, in.instance_index);
    let blend_edge = chunk_edge_blend_factor(in.world_position.x, in.world_position.z);
    Nw = normalize(mix(smooth_world, Nw, blend_edge));
  }
  if (enable_bent == 1u) {
    Nw = get_bent_normal(in.world_position.xyz, Nw);
  }

  // Light & view vectors
  let L = scene.light_direction; // normalized by CPU
  let V = normalize(scene.camera_position - in.world_position.xyz);

  if (effects.enable_grunge == 1u) {
    let raw_ndotl = max(dot(normalize(Nw), normalize(L)), 0.0);
    let slope_shadow = clamp(1.0 - dot(normalize(Nw), vec3<f32>(0.0, 1.0, 0.0)), 0.0, 1.0);
    let grunge_shadow = clamp((1.0 - raw_ndotl) * 0.82 + slope_shadow * 0.28, 0.0, 1.0);
    if (visual_profile == 2u) {
      base_albedo = apply_kr_transition_grime(
        base_albedo,
        tile,
        world_tile,
        uv_in_tile,
        in.world_position.xz,
        effects.grunge_strength,
      );
    }
    base_albedo = apply_shadow_aware_land_grunge(
      base_albedo,
      in.world_position.xz,
      effects.grunge_strength,
      visual_profile,
      grunge_shadow,
    );
  }
  if (visual_profile == 2u) {
    base_albedo = apply_kr_material_micro_contrast(
      base_albedo,
      tile,
      in.world_position.xz,
      effects.kr_land_shadow_mottle_strength,
    );
  }

  // ---- Shade ----
  var hdr_rgb = vec3<f32>(0.0);
  if (shading_mode == 0u) {
    hdr_rgb = shade_mode0_classic_vertex(base_albedo, in.uv_b.x, ambient_strength, diffuse_strength);
  } else if (shading_mode == 1u) {
    hdr_rgb = shade_mode1_enhanced_fragment(
      base_albedo, in.world_position.xyz, Nw, V, L,
      ambient_strength, diffuse_strength, sharpness_factor, sharpness_mix,
      fill_strength, rim_strength, specular_strength, enable_gloom
    );
  } else { // 2 = KR-like
    hdr_rgb = shade_mode2_kr_fragment(
      base_albedo, in.world_position.xyz, Nw, V, L,
      ambient_strength, diffuse_strength, sharpness_factor, sharpness_mix,
      fill_strength, rim_strength, specular_strength, enable_gloom
    );
    let contact_shadow = kr_height_contact_shadow(
      world_tile,
      tile.tile_height,
      effects.kr_land_relief_shadow_strength,
    );
    hdr_rgb *= 1.0 - contact_shadow * 0.10;
    hdr_rgb = apply_kr_liquid_material_response(
      hdr_rgb,
      base_albedo,
      in.world_position.xyz,
      Nw,
      V,
      L,
      tile,
      specular_strength,
      enable_water,
    );
  }
  hdr_rgb = apply_static_light_decals(hdr_rgb, base_albedo, in.world_position.xyz);

  hdr_rgb = apply_global_lighting_rgb(hdr_rgb, scene.global_lighting);
  hdr_rgb = apply_kr_land_atmosphere_depth(
    hdr_rgb,
    in.world_position.xyz,
    scene.camera_position,
    Nw,
    enable_fog,
    visual_profile,
  );

  // ============================================================================
  // Grading (vibrant, neutral contrast) + Tonemap (Reinhard + exposure)
  // ============================================================================
  var post = hdr_rgb;
  if (enable_grading == 1u) {
    post = grade_color_vibrant(post, global_light);
  }
  var final_rgb = post;
  if (enable_tonemap == 1u) {
    final_rgb = tonemap_ec_kr_profile(max(post, vec3<f32>(0.0)), exposure, visual_profile);
  }

  final_rgb = max(final_rgb, vec3<f32>(0.0));
  return vec4<f32>(final_rgb, base_alpha);
}
