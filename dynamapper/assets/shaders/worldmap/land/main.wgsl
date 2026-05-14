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
#import "shaders/worldmap/land/bindings.wgsl"::{
  TileUniform,
  AtlasParams, SceneUniform, LandEffectsUniform, GlobalLightingUniforms, LandLightingUniforms,
  tex_small_sampler, tex_small, tex_big, tile_meta_atlas,
  ATLAS, scene, effects, global_light, land_light,
  USE_VOLUMETRIC_NOISE,
}
#import "shaders/worldmap/land/atlas.wgsl"::{atlas_read_meta, atlas_read_height, chunk_edge_blend_factor}
#import "shaders/worldmap/land/noise.wgsl"::{hash, fbm_billow, domain_warp}
#import "shaders/worldmap/land/normals.wgsl"::{get_geometric_normal_local, get_bicubic_normal, get_bent_normal}
#import "shaders/worldmap/land/lighting.wgsl"::{luminance, grade_color_vibrant, tonemap_reinhard_with_exposure}
#import "shaders/worldmap/land/sampling.wgsl"::{
  ec_world_uv,
  sample_tile_albedo, sample_tile_reconstructed,
  apply_sharpening, blurred_albedo,
}
#import "shaders/worldmap/land/shading.wgsl"::{
  shade_mode0_classic_vertex,
  shade_mode1_enhanced_fragment,
  shade_mode2_kr_fragment,
}
#import "shaders/worldmap/water.wgsl"::water_distort_uv

// ============================================================================
// Vertex shader
//  - Displaces the flat grid by per-tile height from the atlas.
//  - Computes the normal (geometric or bicubic), optionally bent.
//  - For Classic mode (0) pre-bakes the Lambert term into uv_b.x (Gouraud).
// ============================================================================

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

  // Debug mode: return a flat color immediately to isolate non-fragment bottlenecks.
  if (shading_mode == 3u) {
    return vec4<f32>(0.5, 0.5, 0.5, 1.0);
  }

  // ---- Zoom-based shader LOD: disable expensive features when zoomed out ----
  // zoom > 5  : bicubic/FSR reconstruction → nearest (saves ~15 tex reads)
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
  var force_nearest  = false;
  var disable_sharpen = false;

  if (zoom > 5.0) {
    force_nearest = true;  // skip bicubic/FSR, use direct nearest sample
  }
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
  let tile = atlas_read_meta(i32(floor(in.world_position.x)), i32(floor(in.world_position.z)));

  // ---- Animated water ----
  // Apply sin/cos UV distortion to tiles with the IsWet tiledata flag.
  // The distortion breathes the sampled UV region slightly larger than 1.0,
  // creating a gentle wavy appearance (faithful port of ClassicUO's formula).
  // Skipped at high zoom where individual tiles are sub-pixel (already gated above).
  if (enable_water == 1u && tile.is_wet == 1u) {
    uv_in_tile = water_distort_uv(uv_in_tile);
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
    base_albedo = sample_tile_albedo(uv_q, tile);
  } else {
    // force_nearest: skip bicubic/FSR at high zoom to save ~15 tex reads per pixel
    if (force_nearest) {
      base_albedo = sample_tile_albedo(sample_uv, tile);
    } else {
      base_albedo = sample_tile_reconstructed(sample_uv, tile);
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
  }

  // Apply global scene lighting scaler (UI: "Global Lighting / Scene Luminosity")
  hdr_rgb *= max(scene.global_lighting, 0.0);

  // ============================================================================
  // Fog
  // ============================================================================
  // IMPORTANT: UI-controlled inputs are remapped to internal ranges:
  //  - fog_params.x -> distance_density (UI 0..0.2) mapped to fog_end distance
  //  - fog_params.y -> height_density   (UI 0..0.2) mapped to vertical falloff
  //  - fog_params.z -> noise_scale      (UI 0..2)   mapped to world noise scale
  //  - fog_params.w -> noise_strength   (UI 0..1)   cloud contrast/detail/coverage
  if (enable_fog == 1u) {
    // Night-aware fog: blend fog_color toward a darker fog tint (e.g. deep blue night fog).
    // At blend=0 (day presets): no change. At blend=1: fully replaces with night tint.
    let night_blend = clamp(global_light.fog_night_color.a, 0.0, 1.0);
    let effective_fog_color = mix(global_light.fog_color.rgb, global_light.fog_night_color.rgb, night_blend);

    // Read raw UI uniforms (defensive clamps)
    let dist_density_ui   = clamp(global_light.fog_params.x, 0.0, 1.0);
    let height_density_ui = clamp(global_light.fog_params.y, 0.0, 1.0);
    let noise_scale_ui    = clamp(global_light.fog_params.z, 0.0, 2.0);
    let noise_strength_ui = clamp(global_light.fog_params.w, 0.0, 1.0);

    // Fog height bias: -1 valley, 0 neutral, +1 high-alt haze
    let hBias = clamp(global_light.gloom_params.w, -1.0, 1.0);
    let high_w = max(hBias, 0.0);
    let low_w  = max(-hBias, 0.0);

    // Distance mapping: translate UI density -> fog_end (meters).
    // Small UI values -> very far (clear). Larger UI -> closer fog end.
    let fog_end = mix(6000.0, 40.0, smoothstep(0.0, 0.2, dist_density_ui));
    let fog_start = max(0.0, fog_end * 0.06);
    let d = length(in.world_position.xyz - scene.camera_position);
    // Softer ramp for distance-based fog
    let dist_factor = smoothstep(fog_start, fog_end, d);

    // Height mapping: UI -> falloff scale in meters (0.2 UI -> short falloff)
    let height_falloff = mix(800.0, 6.0, smoothstep(0.0, 0.2, height_density_ui));
    let y = in.world_position.y;
    var height_term_high = 0.0;
    var height_term_low  = 0.0;
    if (height_density_ui > 1e-6) {
      height_term_high = 1.0 - exp(-max(y, 0.0) / height_falloff);
      height_term_low  = 1.0 - exp(-max(-y, 0.0) / height_falloff);
    }
    let height_factor = clamp(high_w * height_term_high + low_w * height_term_low, 0.0, 1.0);

    // Combine distance & height (union) -> base_fog [0..1]
    let base_fog = clamp(dist_factor + height_factor - dist_factor * height_factor, 0.0, 1.0);

    // ---- Noise / clouds ----
    // Map user noise_scale_ui [0..2] to world-scale: smaller -> finer clouds.
    let noise_scale_world = mix(0.004, 0.25, clamp(noise_scale_ui / 2.0, 0.0, 1.0));
    let noise_strength = noise_strength_ui;

    // Animated wind derived from sun direction (perpendicular flow across sun).
    // Uses Bevy's built-in globals.time (auto-updated, wraps at 1h).
    let base_time_speed = 0.02;
    let time_speed = base_time_speed + noise_strength * 0.08;
    let t = globals.time * time_speed;

    let sun2 = normalize(vec2<f32>(L.x, L.z));
    // Defensively handle degenerate light_dir
    var wind = vec2<f32>(0.7, 0.3);
    if (length(sun2) > 1e-5) {
      wind = normalize(vec2<f32>(L.z, -L.x));
    }

    // Sample coords in world meters; domain-warp to break tiling
    let p0 = (in.world_position.xz * noise_scale_world) + wind * (t * 6.0);

    var n_billow = 0.0;
    if (noise_strength > 0.001) {
      // Gentle domain warp — skip if noise_strength is very low to save massive ALU
      var p_final = p0;
      if (noise_strength > 0.2) {
        let warp_strength = 0.4 * noise_strength + 0.08;
        p_final = domain_warp(p0, warp_strength);
      }

      // Multi-scale billow FBM: skip second octave if zoomed out or low strength
      let n1 = fbm_billow(p_final * 1.0);
      var n2 = 0.0;
      if (zoom < 10.0 && noise_strength > 0.4) {
        n2 = fbm_billow(p_final * 2.3) * 0.55;
        n_billow = clamp(n1 * 0.7 + n2 * 0.3, 0.0, 1.0);
      } else {
        n_billow = n1;
      }
    }

    // Coverage/contrast control:
    //  Lower threshold -> more coverage.
    //  width controls softness of cloud edges.
    let base_threshold = mix(0.72, 0.46, noise_strength); // higher noise_strength -> more clouds
    let edge_width = mix(0.12, 0.20, 1.0 - noise_strength); // stronger noise -> sharper edges
    let cloud_mask = smoothstep(base_threshold, base_threshold + edge_width, n_billow);

    // Baseline ensures there are some clear areas when desired
    let baseline = mix(0.03, 0.18, 1.0 - noise_strength);
    let peak_gain = mix(1.0, 1.8, noise_strength);

    let cloud_mod = baseline + (peak_gain - baseline) * cloud_mask;

    var fog_factor = clamp(base_fog * cloud_mod, 0.0, 1.0);

    // Subtle breathing so it doesn't look totally static; scaled by noise_strength
    let breath = 0.5 + 0.5 * sin(globals.time * (0.06 + 0.02 * noise_strength) + (hash(in.world_position.xz * 0.11) * 6.2831));
    fog_factor = clamp(fog_factor * mix(0.97, 1.03, (breath - 0.5) * 0.6 * noise_strength), 0.0, 1.0);

    // Final cap set by UI alpha
    let fog_mix = clamp(fog_factor * global_light.fog_color.a, 0.0, 1.0);

    if (use_volumetric_fog) {
      hdr_rgb = mix(hdr_rgb, effective_fog_color, fog_mix);
    } else {
      // Simple fallback: linearized distance*height blend capped by alpha
      let flat_mix = clamp(base_fog * global_light.fog_color.a, 0.0, 1.0);
      hdr_rgb = mix(hdr_rgb, effective_fog_color, flat_mix);
    }
  }

  // ============================================================================
  // Grading (vibrant, neutral contrast) + Tonemap (Reinhard + exposure)
  // ============================================================================
  var post = hdr_rgb;
  if (enable_grading == 1u) {
    post = grade_color_vibrant(post); // no pre-tonemap clamping
  }
  var final_rgb = post;
  if (enable_tonemap == 1u) {
    final_rgb = tonemap_reinhard_with_exposure(max(post, vec3<f32>(0.0)), exposure);
  }

  final_rgb = max(final_rgb, vec3<f32>(0.0));
  return vec4<f32>(final_rgb, base_alpha);
}
