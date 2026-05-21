// ============================================================================
// land::atlas — Tile metadata atlas helpers.
//
// Reads per-tile data (height, texture layer, texture size, is_wet flag) from
// the paged LRU atlas: a Rg16Uint texture array that covers the visible world.
// The atlas is kept up to date by the CPU via incremental write_texture calls.
//
// G channel high-byte bit layout (set by Rg16u::pack on the Rust side):
//   bits 0-3: tex_size / source mode (0=cc-small, 1=cc-big, 2=ec-atlas, 3=missing, 4=cc-atlas)
//   bits 4-6: reviewed EC terrain flags
//   bit  7:   is_wet flag (tile has IsWet in tiledata.mul)
// ============================================================================

#import "shaders/world/land/land_bindings.wgsl"::{TileUniform, AtlasParams, ATLAS, tile_meta_atlas, land_page_lookup, CHUNK_TILE_NUM_DIM}

// Query world-space (x,z) tile coordinates and map them, through the LRU
// page table, to the physical GPU atlas layer.  Returns a zero'd TileUniform
// when the requested tile falls outside the paged region.
fn atlas_read_meta(world_x: i32, world_z: i32) -> TileUniform {
  if (world_x < 0 || world_z < 0) {
    return TileUniform(0.0, 0u, 0u, 0u, vec2<u32>(0u, 0u), vec2<u32>(0u, 0u), 0u, 0u);
  }

  let wx = u32(world_x);
  let wz = u32(world_z);

  let pw = ATLAS.page_texels.x;
  let ph = ATLAS.page_texels.y;

  let page_x = wx / pw;
  let page_y = wz / ph;

  let off_x = wx % pw;
  let off_y = wz % ph;

  let page_index = page_y * ATLAS.world_pages_x + page_x;
  var layer: u32 = 0xFFFFFFFFu;
  if (page_index < 256u) {
    let arr_idx = page_index / 4u;
    let comp = page_index % 4u;
    layer = ATLAS.page_to_layer[arr_idx][comp];
  }

  if (layer >= ATLAS.max_layers) {
    return TileUniform(0.0, 0u, 0u, 0u, vec2<u32>(0u, 0u), vec2<u32>(0u, 0u), 0u, 0u);
  }

  // Load from Rg16Uint texture array
  let packed = textureLoad(tile_meta_atlas, vec2<i32>(i32(off_x), i32(off_y)), i32(layer), 0);
  let r = packed.x;
  let g = packed.y;

  let texture_payload = r;

  // G channel low byte: height biased by +128 (so 0 = -12.8, 128 = 0.0, 255 = +12.7)
  let height_biased = g & 0xFFu;
  let z_i32 = i32(height_biased) - 128;
  let tile_height = f32(z_i32) * 0.1;

  // G channel high byte: terrain texture source/mode, reviewed EC flags, and water flag.
  // Bits 0-3: tex_size / source mode (0=cc-small, 1=cc-big, 2=ec-atlas, 3=missing, 4=cc-atlas).
  // Bits 4-6: reviewed EC terrain flags (bit 0=smooth, bit 1=follow-center, bit 2=liquid).
  // Bit 7:    is_wet flag (tile has IsWet in tiledata.mul).
  let g_high  = (g >> 8u) & 0xFFu;
  let tex_size = g_high & 0x0Fu;          // lower nibble = source mode
  let terrain_flags = (g_high >> 4u) & 0x7u;
  let is_wet   = (g_high >> 7u) & 0x1u;  // bit 7 = animated water flag

  if (tex_size == 2u || tex_size == 4u) {
    let lookup_dims = textureDimensions(land_page_lookup);
    let lookup_uv = vec2<i32>(
      i32(texture_payload % lookup_dims.x),
      i32(texture_payload / lookup_dims.x),
    );
    let slot = textureLoad(land_page_lookup, lookup_uv, 0);
    let packed_wh = slot.w;
    let w = packed_wh & 0xFFFFu;
    let h = packed_wh >> 16u;

    if (w == 0u && h == 0u) {
      return TileUniform(tile_height, 3u, 0u, 0u, vec2<u32>(0u, 0u), vec2<u32>(0u, 0u), is_wet, terrain_flags);
    }

    return TileUniform(
      tile_height,
      tex_size,
      slot.x,
      0u,
      vec2<u32>(slot.y, slot.z),
      vec2<u32>(w, h),
      is_wet,
      terrain_flags,
    );
  }

  return TileUniform(tile_height, tex_size, texture_payload, 0u, vec2<u32>(0u), vec2<u32>(0u), is_wet, terrain_flags);
}

// Convenience wrapper: just return the world-space Y height for a tile.
fn atlas_read_height(world_x: i32, world_z: i32) -> f32 {
  return atlas_read_meta(world_x, world_z).tile_height;
}

// Near the chunk edge, blend normals toward the original to hide seams.
// Returns a factor in [0,1]: 0 = fully interior, 1 = on the very edge.
fn chunk_edge_blend_factor(world_x: f32, world_z: f32) -> f32 {
  let local_x = fract(world_x / 8.0) * 8.0;
  let local_z = fract(world_z / 8.0) * 8.0;
  let tx = floor(local_x);
  let tz = floor(local_z);
  let dx = min(tx, f32(CHUNK_TILE_NUM_DIM - 1u) - tx);
  let dz = min(tz, f32(CHUNK_TILE_NUM_DIM - 1u) - tz);
  let min_dist = min(dx, dz);
  return 1.0 - smoothstep(0.0, 2.0, min_dist);
}
