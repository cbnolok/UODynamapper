// ============================================================================
// Bevy PBR WGSL — Terrain entry point (Three modes: 0 Classic, 1 Enhanced, 2 KR-like)
//
//  - Mode 0: "2D Classic"   — faithful brightness model, vertex/Gouraud.
//  - Mode 1: "2D Enhanced"  — per-fragment, subtle improvements.
//  - Mode 2: "KR-like"      — painterly: warm key, cool ambient/fill, vibrant grading.
//
// This file contains ONLY the @vertex and @fragment entry points.
// All helpers live in the co-located module files and are pulled in via import.
//  https://wgmath.rs/docs/user_guides/wgcore/shaders_composition/
// ============================================================================

import bevy_pbr::render::{
  forward_io::{Vertex, VertexOutput},
  mesh_functions,
  mesh_view_bindings::{globals, view},
};

// ---- Land shader modules (package asset-path imports for on-demand loading) ----
import package::shaders::world::common_bindings::{
  SceneUniform, LandEffectsUniform, GlobalLightingUniforms
};
import package::shaders::world::land::land_bindings::{
  TileUniform, AtlasParams, LandLightingUniforms,
  tex_small_sampler, tex_small, tex_big, tile_meta_atlas,
  land_page_atlas, land_page_lookup,
  visual_grunge_texture,
  ATLAS, scene, effects, global_light, land_light,
  TERRAIN_SHADING_CLIENT_PER_VERTEX
};
import package::shaders::world::land::atlas::{atlas_read_meta, atlas_read_height, chunk_edge_blend_factor};
import package::shaders::world::land::noise::{hash, fbm_billow, domain_warp};
import package::shaders::world::land::normals::{get_geometric_normal_local, get_bicubic_normal, get_bent_normal, client_vertex_terrain_shade};
import package::shaders::postprocess::color_grading::{grade_color_vibrant};
import package::shaders::postprocess::global_lighting::{apply_global_lighting_rgb};
import package::shaders::postprocess::grunge::{apply_texture_shadow_aware_land_grunge, apply_texture_visual_grunge, visual_grunge_uv};
import package::shaders::postprocess::kr_color_wash::apply_kr_color_wash;
import package::shaders::postprocess::tonemapping::{tonemap_with_curve};
import package::shaders::world::land::sampling::{
  LandGradients, ec_tile_gradients, ec_world_uv,
  ec_material_has_liquid_normal,
  sample_ec_material_normal_world,
  sample_tile_albedo,
  sample_tile_albedo_filtered,
  sample_tile_albedo_grad,
  sample_ec_material_albedo,
  sample_ec_material_albedo_at_world,
  apply_sharpening, blurred_albedo,
  classic_land_art_is_present,
  sample_classic_land_art,
};
import package::shaders::world::client_filter::TEXTURE_FILTER_CLIENT_BILINEAR;
import package::shaders::world::land::shading::{
  shade_mode0_classic_vertex,
  shade_mode1_enhanced_fragment,
  shade_mode2_kr_fragment,
  apply_static_light_decals,
};
import package::shaders::world::effects::surface_effects::{apply_water_animation, water_flat_wrapped_lambert};
import package::shaders::world::ec_liquid::{sample_ec_liquid, ec_liquid_world_uv};
import package::shaders::world::ec_liquid_camera::ec_liquid_camera_reflection_uv;
import package::shaders::world::texture_data::texture_color_rgb;
import package::shaders::world::land::coverage::{EcTerrainStack, EcTerrainCache, EcTerrainLayer, ec_coverage_params, ec_terrain_cache, ec_cache_layer, ec_cache_grime_mask, ec_terrain_stack_uncached, ec_layer_tile};
import package::shaders::world::land::liquid_bank::{ec_liquid_bank, ec_bank_vertex};
import package::shaders::world::land::cc_transitions::{blend_cc_terrain_transitions, cc_transition_dry_material};
import package::shaders::world::projection::camera_relative_world_to_clip;

import package::shaders::world::land::transitions::{
  ec_transition_neighbor_weight, ec_transition_diagonal_weight,
  ec_transition_materials_compatible, ec_transition_boundary_weight,
  ec_transition_boundary_weight_from_mask,
};

// ============================================================================
// Vertex shader
//  - Displaces the flat grid by per-tile height from the atlas.
//  - Computes the normal (geometric or bicubic), optionally bent.
//  - For Classic mode (0) pre-bakes the Lambert term into uv_b.x (Gouraud).
// ============================================================================

const TERRAIN_FLAG_REVIEWED_LIQUID: u32 = 0x4u;
const EC_TERRAIN_TRANSITION_BASE_WEIGHT: f32 = 0.35;

fn land_cell_is_flat(world_tile: vec2<i32>, tile_height: f32) -> bool {
  let epsilon = 0.0001;
  return abs(atlas_read_height(world_tile.x + 1, world_tile.y) - tile_height) <= epsilon
    && abs(atlas_read_height(world_tile.x, world_tile.y + 1) - tile_height) <= epsilon
    && abs(atlas_read_height(world_tile.x + 1, world_tile.y + 1) - tile_height) <= epsilon;
}

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
  if (specular_strength <= 0.0001) {
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

  let luma = dot(base_albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
  let max_channel = max(base_albedo.r, max(base_albedo.g, base_albedo.b));
  let min_channel = min(base_albedo.r, min(base_albedo.g, base_albedo.b));
  let chroma = max_channel - min_channel;
  let warm_liquid = clamp(base_albedo.r - max(base_albedo.g, base_albedo.b), 0.0, 1.0);
  let lava_like = select(0.0, smoothstep(0.04, 0.22, warm_liquid) * smoothstep(0.18, 0.55, chroma), is_liquid);
  let swamp_like = select(0.0, smoothstep(0.02, 0.16, base_albedo.g - max(base_albedo.r, base_albedo.b)) * (1.0 - smoothstep(0.32, 0.58, luma)), is_liquid);
  let water_like = select(0.0, clamp(1.0 - max(lava_like, swamp_like), 0.0, 1.0), is_liquid);
  let ice_snow_like = (1.0 - select(0.0, 1.0, is_liquid)) * smoothstep(0.52, 0.84, luma) * (1.0 - smoothstep(0.20, 0.46, chroma)) * smoothstep(-0.06, 0.14, base_albedo.b - base_albedo.r);
  let strength = clamp(specular_strength, 0.0, 0.25);

  let water_response = (sun_glint * 0.52 + view_glance * 0.20) * ripple * water_like;
  let lava_response = (0.18 + sun_glint * 0.24 + view_glance * 0.08) * lava_like;
  let swamp_response = (sun_glint * 0.22 + view_glance * 0.08) * ripple * swamp_like;
  let ice_response = (sun_glint * 0.20 + view_glance * 0.30) * ice_snow_like;

  var out_color = color;
  out_color += vec3<f32>(0.70, 0.92, 1.04) * water_response * strength;
  out_color += vec3<f32>(1.16, 0.42, 0.16) * lava_response * strength;
  out_color += vec3<f32>(0.38, 0.56, 0.44) * swamp_response * strength * 0.55;
  out_color += vec3<f32>(0.82, 0.96, 1.08) * ice_response * strength * 0.75;
  return out_color;
}

fn ec_transition_neighbor_color(
  current_tile: TileUniform,
  neighbor_tile: TileUniform,
  world_xz: vec2<f32>,
  base_color: vec3<f32>,
  weight: f32,
  world_gradients: LandGradients,
) -> vec4<f32> {
  if (weight <= 0.0 || neighbor_tile.texture_size != 2u) {
    return vec4<f32>(base_color, 0.0);
  }

  if (!ec_transition_materials_compatible(current_tile, neighbor_tile)) {
    return vec4<f32>(base_color, 0.0);
  }

  if (neighbor_tile.texture_payload == current_tile.texture_payload) {
    return vec4<f32>(base_color, 0.0);
  }

  if (neighbor_tile.texture_extent.x == 0u || neighbor_tile.texture_extent.y == 0u) {
    return vec4<f32>(base_color, 0.0);
  }

  return vec4<f32>(sample_ec_material_albedo_at_world(world_xz, neighbor_tile, world_gradients), weight);
}

fn apply_kr_transition_grime(
  color: vec3<f32>,
  current_tile: TileUniform,
  world_tile: vec2<i32>,
  uv_in_tile: vec2<f32>,
  world_xz: vec2<f32>,
  strength: f32,
  grime_mask: u32,
) -> vec3<f32> {
  var boundary: f32;
  if (grime_mask <= 255u) {
    boundary = ec_transition_boundary_weight_from_mask(current_tile, uv_in_tile, grime_mask);
  } else {
    boundary = ec_transition_boundary_weight(current_tile, world_tile, uv_in_tile);
  }
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

fn kr_cliff_bank_shadow(world_tile: vec2<i32>, center_height: f32, Nw: vec3<f32>, L: vec3<f32>, strength: f32) -> f32 {
  let relief_strength = clamp(strength, 0.0, 1.5);
  if (relief_strength <= 0.0001) {
    return 0.0;
  }

  let west = atlas_read_height(world_tile.x - 1, world_tile.y);
  let east = atlas_read_height(world_tile.x + 1, world_tile.y);
  let north = atlas_read_height(world_tile.x, world_tile.y - 1);
  let south = atlas_read_height(world_tile.x, world_tile.y + 1);
  let max_height = max(max(west, east), max(north, south));
  let min_height = min(min(west, east), min(north, south));
  let height_range = max(max_height - min_height, 0.0);
  let height_step = max(max_height - center_height, center_height - min_height);
  let steep_height = smoothstep(0.18, 0.95, height_range) * smoothstep(0.08, 0.62, height_step);
  let slope_steepness = smoothstep(0.12, 0.55, 1.0 - normalize(Nw).y);
  let unlit_side = 1.0 - smoothstep(0.18, 0.72, max(dot(normalize(Nw), normalize(L)), 0.0));
  return steep_height * (0.35 + 0.65 * max(slope_steepness, unlit_side)) * relief_strength;
}

fn blend_ec_terrain_transitions(
  base_color: vec3<f32>,
  current_tile: TileUniform,
  world_tile: vec2<i32>,
  world_xz: vec2<f32>,
  uv_in_tile: vec2<f32>,
  world_gradients: LandGradients,
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
    world_gradients,
  );
  accum += west.rgb * west.a;
  total_weight += west.a;

  let east = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x + 1, world_tile.y),
    world_xz,
    base_color,
    ec_transition_neighbor_weight(east_offset, uv_in_tile),
    world_gradients,
  );
  accum += east.rgb * east.a;
  total_weight += east.a;

  let north = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x, world_tile.y - 1),
    world_xz,
    base_color,
    ec_transition_neighbor_weight(north_offset, uv_in_tile),
    world_gradients,
  );
  accum += north.rgb * north.a;
  total_weight += north.a;

  let south = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x, world_tile.y + 1),
    world_xz,
    base_color,
    ec_transition_neighbor_weight(south_offset, uv_in_tile),
    world_gradients,
  );
  accum += south.rgb * south.a;
  total_weight += south.a;

  let northwest = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x - 1, world_tile.y - 1),
    world_xz,
    base_color,
    ec_transition_diagonal_weight(northwest_offset, uv_in_tile),
    world_gradients,
  );
  accum += northwest.rgb * northwest.a;
  total_weight += northwest.a;

  let northeast = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x + 1, world_tile.y - 1),
    world_xz,
    base_color,
    ec_transition_diagonal_weight(northeast_offset, uv_in_tile),
    world_gradients,
  );
  accum += northeast.rgb * northeast.a;
  total_weight += northeast.a;

  let southwest = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x - 1, world_tile.y + 1),
    world_xz,
    base_color,
    ec_transition_diagonal_weight(southwest_offset, uv_in_tile),
    world_gradients,
  );
  accum += southwest.rgb * southwest.a;
  total_weight += southwest.a;

  let southeast = ec_transition_neighbor_color(
    current_tile,
    atlas_read_meta(world_tile.x + 1, world_tile.y + 1),
    world_xz,
    base_color,
    ec_transition_diagonal_weight(southeast_offset, uv_in_tile),
    world_gradients,
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
  let bank_vertex = ec_bank_vertex(vec2<i32>(wx, wz), atlas_read_meta(wx, wz));

  var final_world_pos = flat_world_pos;
  // Keep the visible mountain face's authored crust contact fixed. Relax only
  // inset corners where the rising bank faces away from the camera.
  let relax_corner = smoothstep(0.0, 0.1, dot(bank_vertex.xz, view.world_from_view[2].xz));
  final_world_pos += vec4<f32>(bank_vertex.x * relax_corner, bank_vertex.y, bank_vertex.z * relax_corner, 0.0);

  out.world_position = final_world_pos;
  let local_world_pos = world_from_local * vec4<f32>(flat_local_pos, 0.0);
  out.position = camera_relative_world_to_clip(world_from_local[3].xyz,
      local_world_pos.xyz + vec3<f32>(bank_vertex.x * relax_corner, bank_vertex.y, bank_vertex.z * relax_corner));
  // Terrain albedo uses world coordinates; use the otherwise unused UV varying
  // for the original vertex-generated liquid reflection coordinates.
  out.uv = ec_liquid_camera_reflection_uv(final_world_pos.xyz, scene.ec_reflection_camera_distance);
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

// Geometry is shared by every material in the ownership stack. Keep its
// height reconstruction outside the per-layer shading calls.
fn land_surface_normal(in: VertexOutput) -> vec3<f32> {
  // Classic lighting uses the vertex Lambert, not a fragment normal. Only
  // KR-profile shadow weathering consumes it when those controls are combined.
  let shadow_grunge = effects.enable_grunge == 1u && effects.post_process_profile == 2u
    && clamp(effects.grunge_strength, 0.0, 1.0) > 0.0001;
  if (effects.shading_mode == 0u && !shadow_grunge) {
    return in.world_normal;
  }
  var normal_mode = effects.normal_mode;
  var enable_bent = land_light.enable_bent;
  if (scene.render_zoom > 10.0) { normal_mode = 0u; } // geometric normals only (skip bicubic 4×4 kernel)
  if (scene.render_zoom > 20.0) { enable_bent = 0u; } // skip bent normals (4 extra height reads)
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
  return Nw;
}

// The clients' per-vertex terrain grey, computed once per fragment for every
// material layer. 1 when the per-pixel model or Classic shades the land.
fn land_client_shade(in: VertexOutput) -> f32 {
  if (effects.shading_mode == 0u || land_light.terrain_shading_model != TERRAIN_SHADING_CLIENT_PER_VERTEX) {
    return 1.0;
  }
  return client_vertex_terrain_shade(in.world_position.xyz, scene.light_direction);
}

fn shade_land(in: VertexOutput, tile: TileUniform, ordered_coverage: bool, world_gradients: LandGradients, surface_normal: vec3<f32>, client_shade: f32, grime_mask: u32) -> vec3<f32> {
  let shading_mode   = effects.shading_mode;
  let visual_profile = effects.post_process_profile;

  // ---- Zoom-based shader LOD: disable expensive features when zoomed out ----
  // zoom > 10 : disable blur + sharpening + bicubic normals (saves ~30 tex reads)
  // zoom > 20 : disable bent normals (saves ~4 tex reads)
  let zoom = scene.render_zoom;
  var enable_gloom   = global_light.enable_gloom;
  var enable_blur    = effects.enable_blur;
  var disable_sharpen = false;

  if (zoom > 10.0) {
    enable_blur = 0u;      // skip 9-tap blur
    disable_sharpen = true; // skip sharpening (4 extra taps)
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


  // Local UV (fractional position within the current tile, in [0,1))
  var uv_in_tile = vec2<f32>(fract(in.world_position.x), fract(in.world_position.z));
  let world_tile = vec2<i32>(i32(floor(in.world_position.x)), i32(floor(in.world_position.z)));
  let use_classic_flat_wet_land_art = tile.texture_size == 4u
    && tile.is_wet == 1u
    && classic_land_art_is_present(tile.texture_payload)
    && land_cell_is_flat(world_tile, tile.tile_height);

  // ---- Animated water ----
  // Apply sin/cos UV distortion to tiles with the IsWet tiledata flag.
  // The distortion breathes the sampled UV region slightly larger than 1.0,
  // creating a gentle wavy appearance (faithful port of ClassicUO's formula).
  // Skipped at high zoom where individual tiles are sub-pixel (already gated above).
  let reviewed_liquid = (tile.terrain_flags & TERRAIN_FLAG_REVIEWED_LIQUID) != 0u;
  let use_ec_liquid = tile.texture_size == 2u && (tile.is_wet == 1u || reviewed_liquid);
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
  var uv_gradients: LandGradients;
  if (tile.texture_size == 2u) { uv_gradients = ec_tile_gradients(world_gradients, tile); }
  let cc_transitions = cc_transition_dry_material(tile) && (effects.terrain_transition_flags & 1u) != 0u;
  if (cc_transitions) { uv_gradients = world_gradients; }
  // The client filter sizes its emulated mips from the footprint of every texture; a per-tile uv is the
  // fractional world position, so its derivatives are the world's.
  if (effects.texture_filter == TEXTURE_FILTER_CLIENT_BILINEAR && tile.texture_size != 2u) {
    uv_gradients = world_gradients;
  }
  if (tile.texture_size == 2u) {
    // Compute world-space tiling UV.  We use the (possibly water-distorted)
    // uv_in_tile offset so water animation stays coherent with EC textures too.
    let world_xz = vec2<f32>(floor(in.world_position.x), floor(in.world_position.z)) + uv_in_tile;
    sample_uv = ec_world_uv(world_xz, tile);
  }


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
  if (use_ec_liquid) {
    base_albedo = sample_ec_liquid(land_page_atlas, land_page_lookup, tex_small_sampler,
      tile.texture_payload, in.world_position.xz, ec_liquid_world_uv(in.world_position.xz), in.uv, globals.time,
      enable_water == 1u, effects.enable_linear_filtering == 1u).rgb;
    let energy = ambient_strength + diffuse_strength * water_flat_wrapped_lambert(scene.light_direction);
    base_albedo = texture_color_rgb(clamp(base_albedo * energy, vec3<f32>(0.0), vec3<f32>(1.0)));
  } else if (use_cheap_path && !cc_transitions) {
    // Quantize UV to a coarser grid and use direct nearest sample.
    // For EC tiles sample_uv is already world-space; we quantize in the same space.
    // We still run full lighting/fog/shadows below to keep visual consistency.
    let cells = mix(2.0, 4.0, smoothstep(0.0, 1.0, adaptive));
    let uv_q = (floor(sample_uv * cells) + vec2<f32>(0.5)) / cells;
    if (tile.texture_size == 2u) {
      let world_xz = vec2<f32>(floor(in.world_position.x), floor(in.world_position.z)) + uv_in_tile;
      base_albedo = sample_ec_material_albedo(world_xz, uv_q, tile, world_gradients);
    } else {
      base_albedo = sample_tile_albedo(uv_q, tile, uv_gradients);
    }
  } else {
    if (tile.texture_size == 2u) {
      let world_xz = vec2<f32>(floor(in.world_position.x), floor(in.world_position.z)) + uv_in_tile;
      base_albedo = sample_ec_material_albedo(world_xz, sample_uv, tile, world_gradients);
      if (!ordered_coverage && (effects.terrain_transition_flags & 2u) == 0u) {
        base_albedo = blend_ec_terrain_transitions(
          base_albedo,
          tile,
          world_tile,
          world_xz,
          uv_in_tile,
          world_gradients,
        );
      }
    } else if (cc_transitions) {
      base_albedo = sample_tile_albedo_grad(sample_uv, tile, uv_gradients.dx, uv_gradients.dy);
      base_albedo = blend_cc_terrain_transitions(base_albedo, tile, world_tile, uv_in_tile, uv_gradients);
    } else {
      base_albedo = sample_tile_albedo_filtered(sample_uv, tile, uv_gradients);
    }
    if (enable_blur == 1u && blur_strength > 0.001 && blur_radius > 0.0) {
      let blurred = blurred_albedo(sample_uv, tile, blur_radius, vec2<f32>(in.world_position.x, in.world_position.z), uv_gradients);
      base_albedo = mix(base_albedo, blurred, clamp(blur_strength, 0.0, 1.0));
    }
    if (!disable_sharpen && effects.sharpening_amount > 0.0) {
      base_albedo = apply_sharpening(base_albedo, sample_uv, tile, effects.sharpening_amount, uv_gradients);
    }
  }

  // The normal Classic terrain path samples square texmaps.mul textures. On a
  // flat wet cell, sample the corresponding art.mul land diamond instead so
  // shoreline statics and the terrain underneath share their authored source
  // albedo. This override intentionally happens after terrain blur/sharpening:
  // static art is not filtered by those controls, and preserving the exact art
  // pixels is what removes the material seam. Sloped wet cells retain texmaps.
  if (use_classic_flat_wet_land_art) {
    base_albedo = sample_classic_land_art(uv_in_tile, tile.texture_payload);
  }

  // TODO Phase 10G — post-lighting extra-pass effects (not yet implemented):
  // FoamHighlight, WaterfallSupport, LavaBubbleSupport, RefractionDistortionLike.
  // These require either additive compositing after tone-mapping (to avoid highlight
  // crush) or a screen-space texture copy before the transparent pass (refraction).
  // Both cases need a custom Bevy render-graph node beyond the current single-pass
  // ExtendedMaterial setup.  The speculative_role field in TileMetaItemTextureRef and
  // the ec_tex_aux slots are already in place to drive them.

  // ---- Normals ----
  // We already computed in the vertex shader and passed in.world_normal.
  // For non-classic modes we can still override with bicubic if desired.
  var Nw = surface_normal;
  if (effects.enable_normal_maps == 1u && shading_mode != 0u && tile.texture_size == 2u && !use_ec_liquid) {
    let normal_map = sample_ec_material_normal_world(in.world_position.xz, tile, world_gradients);
    if (normal_map.w > 0.5) {
      let normal_map_strength = clamp(effects.land_normal_map_strength, 0.0, 1.0);
      Nw = normalize(mix(Nw, normal_map.xyz, normal_map_strength));
    }
  }

  // Light & view vectors
  let L = scene.light_direction; // normalized by CPU
  let V = normalize(scene.camera_position - in.world_position.xyz);

  // Transition grime has its own toggle and strength, so it does not depend on Grunge.
  if (visual_profile == 2u && effects.enable_kr_transition_grime == 1u && !use_classic_flat_wet_land_art && !use_ec_liquid) {
    base_albedo = apply_kr_transition_grime(
      base_albedo,
      tile,
      world_tile,
      uv_in_tile,
      in.world_position.xz,
      effects.kr_transition_grime_strength,
      grime_mask,
    );
  }
  if (effects.enable_grunge == 1u) {
    let raw_ndotl = max(dot(normalize(Nw), normalize(L)), 0.0);
    let slope_shadow = clamp(1.0 - dot(normalize(Nw), vec3<f32>(0.0, 1.0, 0.0)), 0.0, 1.0);
    let grunge_shadow = clamp((1.0 - raw_ndotl) * 0.82 + slope_shadow * 0.28, 0.0, 1.0);
    let visual_grunge = textureSample(visual_grunge_texture, tex_small_sampler, visual_grunge_uv(in.world_position.xz, visual_profile)).rgb;
    if (use_classic_flat_wet_land_art || use_ec_liquid) {
      // Flat CC terrain water and flat CC art water are one material with two
      // geometry owners. Do not inject terrain-normal shadow deposits on only
      // one side of their boundary.
      base_albedo = apply_texture_visual_grunge(
        base_albedo,
        in.world_position.xz,
        effects.grunge_strength,
        visual_profile,
        visual_grunge,
      );
    } else {
      base_albedo = apply_texture_shadow_aware_land_grunge(
        base_albedo,
        in.world_position.xz,
        effects.grunge_strength,
        visual_profile,
        grunge_shadow,
        visual_grunge,
      );
    }
  }
  if (visual_profile == 2u) {
    base_albedo = apply_kr_material_micro_contrast(
      base_albedo,
      tile,
      in.world_position.xz,
      effects.kr_land_material_contrast_strength,
    );
  }

  // ---- Shade ----
  var hdr_rgb = vec3<f32>(0.0);
  var kr_color_wash_shadow = 0.0;
  if (use_ec_liquid) {
    // Original encoded stage lighting was applied before conversion above.
    hdr_rgb = base_albedo;
  } else if (use_classic_flat_wet_land_art) {
    // The adjoining authored flat-water statics use this same level-liquid
    // response in every visual profile. KR/Enhanced styling is added later by
    // the shared grunge, wash, global-light, grading, tonemap, and fullscreen fog
    // stages; terrain relief and seabed normals must not shade the water plane.
    let flat_water_lambert = water_flat_wrapped_lambert(scene.light_direction);
    hdr_rgb = shade_mode0_classic_vertex(base_albedo, flat_water_lambert, ambient_strength, diffuse_strength);
  } else if (shading_mode == 0u) {
    // Wet ownership is per cell, while the terrain grid vertices are shared. Resolve the
    // level-water Lambert from the owning fragment tile so it exactly matches canonical
    // water-fill statics without interpolating wet/dry lighting across the cell diagonal.
    let classic_lambert = select(
      in.uv_b.x,
      water_flat_wrapped_lambert(scene.light_direction),
      tile.is_wet == 1u || reviewed_liquid,
    );
    hdr_rgb = shade_mode0_classic_vertex(base_albedo, classic_lambert, ambient_strength, diffuse_strength);
  } else if (land_light.terrain_shading_model == TERRAIN_SHADING_CLIENT_PER_VERTEX) {
    // The KR and Enhanced clients' terrain: the material times the per-vertex
    // half-Lambert grey, with no ambient, fill, rim, specular or relief terms.
    hdr_rgb = base_albedo * client_shade;
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
    let bank_shadow = kr_cliff_bank_shadow(
      world_tile,
      tile.tile_height,
      Nw,
      L,
      effects.kr_land_relief_shadow_strength,
    );
    hdr_rgb *= 1.0 - contact_shadow * 0.10 - bank_shadow * 0.08;
    let ndotl_shadow = 1.0 - smoothstep(0.18, 0.76, max(dot(normalize(Nw), normalize(L)), 0.0));
    let slope_shadow = clamp(1.0 - normalize(Nw).y, 0.0, 1.0);
    kr_color_wash_shadow = clamp(ndotl_shadow * 0.58 + slope_shadow * 0.22 + contact_shadow * 0.32 + bank_shadow * 0.42, 0.0, 1.0);
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

  // Classic's accepted output retains its existing light decals. In the
  // Enhanced/KR flat-water material, the adjoining authored water quads have
  // no equivalent land-light buffer, so applying decals only to terrain would
  // recreate the bright bands and color step at their boundary.
  if (!use_ec_liquid && (!use_classic_flat_wet_land_art || shading_mode == 0u)) {
    hdr_rgb = apply_static_light_decals(hdr_rgb, base_albedo, in.world_position.xyz);
  }
  hdr_rgb = apply_kr_color_wash(
    hdr_rgb,
    in.world_position.xz,
    kr_color_wash_shadow,
    0.0,
    effects.enable_kr_color_wash,
    effects.kr_color_wash_strength,
    effects.kr_color_wash_scale,
    visual_profile,
  );

  return hdr_rgb;
}

fn finish_land(hdr_rgb: vec3<f32>) -> vec4<f32> {
  let enable_tonemap = global_light.enable_tonemap;
  let enable_grading = global_light.enable_grading;
  let exposure = global_light.exposure;
  let visual_profile = effects.post_process_profile;
  // Fog is applied once to the fully composed frame by effects/fog_pass.wesl.

  // ============================================================================
  // Grading (vibrant, neutral contrast) + Tonemap (Reinhard + exposure)
  // ============================================================================
  var post = hdr_rgb;
  if (enable_grading == 1u) {
    post = grade_color_vibrant(post, global_light);
  }
  var final_rgb = post;
  if (enable_tonemap == 1u) {
    final_rgb = tonemap_with_curve(max(post, vec3<f32>(0.0)), exposure, visual_profile, global_light.kr_tonemap_params.x, global_light.tone_curve);
  }

  final_rgb = max(final_rgb, vec3<f32>(0.0));
  return vec4<f32>(final_rgb, 1.0); // tile textures assumed opaque for terrain
}

// Each call supplies a literal selector: the cached call has no live fallback
// arrays; the uncached call reads the original reconstruction. Keep their layer
// ordering, bank selection and shading call sites in one routine.
fn ec_composition_layer(cache: EcTerrainCache, stack: ptr<function, EcTerrainStack>, cached: bool, index: u32) -> EcTerrainLayer {
  if (cached) { return ec_cache_layer(cache, index); }
  return EcTerrainLayer((*stack).layers[index], (*stack).alpha[index]);
}

fn ec_composition_grime_mask(cache: EcTerrainCache, material: vec4<u32>, cached: bool) -> u32 {
  if (cached) { return ec_cache_grime_mask(cache, material); }
  return 256u;
}

fn compose_ec_terrain(in: VertexOutput, tile: TileUniform, world_gradients: LandGradients, surface_normal: vec3<f32>, client_shade: f32,
    cache: EcTerrainCache, fallback: EcTerrainStack, cached: bool) -> vec3<f32> {
  var stack = fallback;
  let count = select(stack.count, cache.count, cached);
  let cell = vec2<i32>(floor(in.world_position.xz));
  var hdr_rgb: vec3<f32>;
  // Opaque later passes cover all earlier shading. This keeps homogeneous
  // terrain at one material sample while retaining the complete layer order.
  var first = 0u;
  for (var index = 1u; index < count; index++) {
    if (ec_composition_layer(cache, &stack, cached, index).alpha == 1.0) { first = index; }
  }
  var liquid_material = 0xffffffffu;
  var liquid_coverage = 1.0;
  for (var index = first; index < count; index++) {
    let layer = ec_composition_layer(cache, &stack, cached, index);
    if ((index == first || layer.alpha > 0.0) && ((layer.material.w >> 16u) & 7u) == 2u) {
      liquid_material = layer.material.x;
      liquid_coverage = layer.alpha;
    }
  }
  var bank_weight = 0.0;
  var bank_cell = cell;
  if (liquid_material != 0xffffffffu) {
    let bank = ec_liquid_bank(in.world_position.xyz, tile, liquid_material, liquid_coverage);
    bank_weight = bank.weight;
    bank_cell = bank.cell;
  }
  if (bank_weight > 0.0) {
    // Reveal the existing solid pass stack, including its authored blends.
    // A single neighboring texture would introduce hard material patches.
    first = 0u;
    for (var index = 1u; index < count; index++) {
      let layer = ec_composition_layer(cache, &stack, cached, index);
      let alpha = layer.alpha * select(1.0, 1.0 - bank_weight, layer.material.x == liquid_material);
      if (alpha == 1.0) { first = index; }
    }
  }
  if (first == 0u && bank_weight > 0.0 && ec_composition_layer(cache, &stack, cached, 0u).material.x == liquid_material) {
    // An all-liquid chunk can meet a bank across its edge without having
    // a local solid base pass. Only that case needs the neighboring material.
    hdr_rgb = shade_land(in, atlas_read_meta(bank_cell.x, bank_cell.y), true, world_gradients, surface_normal, client_shade, 256u);
    if (bank_weight < 1.0) {
      let layer = ec_composition_layer(cache, &stack, cached, 0u);
      hdr_rgb = mix(hdr_rgb, shade_land(in, ec_layer_tile(layer.material, tile.tile_height), true, world_gradients, surface_normal, client_shade,
        ec_composition_grime_mask(cache, layer.material, cached)), 1.0 - bank_weight);
    }
  } else {
    let layer = ec_composition_layer(cache, &stack, cached, first);
    hdr_rgb = shade_land(in, ec_layer_tile(layer.material, tile.tile_height), true, world_gradients, surface_normal, client_shade,
      ec_composition_grime_mask(cache, layer.material, cached));
  }
  for (var index = first + 1u; index < count; index++) {
    let layer = ec_composition_layer(cache, &stack, cached, index);
    let alpha = layer.alpha * select(1.0, 1.0 - bank_weight, layer.material.x == liquid_material);
    if (alpha > 0.0) {
      let layer_color = shade_land(in, ec_layer_tile(layer.material, tile.tile_height), true, world_gradients, surface_normal, client_shade,
        ec_composition_grime_mask(cache, layer.material, cached));
      hdr_rgb = mix(hdr_rgb, layer_color, alpha);
    }
  }
  return hdr_rgb;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
  // Debug mode: return a flat color immediately to isolate non-fragment bottlenecks.
  if (effects.shading_mode == 3u) {
    return vec4<f32>(0.5, 0.5, 0.5, 1.0);
  }
  // Capture the continuous world footprint before ownership/bank/LOD branches.
  // Material-specific repetition scales it without differentiating another pass's UVs.
  let world_gradients = LandGradients(dpdx(in.world_position.xz), dpdy(in.world_position.xz));
  let surface_normal = land_surface_normal(in);
  let client_shade = land_client_shade(in);
  let cell = vec2<i32>(floor(in.world_position.xz));
  let tile = atlas_read_meta(cell.x, cell.y);
  var hdr_rgb: vec3<f32>;
  var composed = false;
  let ec_transitions = (effects.terrain_transition_flags & 2u) == 0u;
  if (ec_transitions && tile.texture_size == 2u && ec_coverage_params(tile.texture_payload).w != 0u) {
    let cache = ec_terrain_cache(in.world_position.xz);
    if (cache.count > 0u) {
      hdr_rgb = compose_ec_terrain(in, tile, world_gradients, surface_normal, client_shade, cache, EcTerrainStack(), true);
      composed = true;
    } else {
      let stack = ec_terrain_stack_uncached(in.world_position.xz);
      if (stack.count > 0u) {
        hdr_rgb = compose_ec_terrain(in, tile, world_gradients, surface_normal, client_shade, cache, stack, false);
        composed = true;
      }
    }
  }
  if (!composed) {
    hdr_rgb = shade_land(in, tile, false, world_gradients, surface_normal, client_shade, 256u);
    if (!ec_transitions && tile.texture_size == 2u && (tile.is_wet == 1u || (tile.terrain_flags & 4u) != 0u)) {
      // Disabling neighbor smoothing must not put level lava back onto raised
      // mountain quads. Keep the height/shore guard, using one dry bank material.
      let bank = ec_liquid_bank(in.world_position.xyz, tile, ec_coverage_params(tile.texture_payload).x, 1.0);
      if (bank.weight > 0.0) {
        let bank_tile = atlas_read_meta(bank.cell.x, bank.cell.y);
        let bank_color = shade_land(in, bank_tile, true, world_gradients, surface_normal, client_shade, 256u);
        hdr_rgb = mix(hdr_rgb, bank_color, bank.weight);
      }
    }
  }
  return finish_land(apply_global_lighting_rgb(hdr_rgb, scene.global_lighting));
}
