// ============================================================================
// client_filter.wesl — The KR client's texture filter (texture_filter 1).
//
// The client samples its sprite, grunge and terrain maps with plain bilinear from one level: no map is
// created with a mip filter and the DDS files carry a single level (results/uokr_atmosphere.md P2.2). Its
// orthographic view is 26 world units wide at any window size, so the screen footprint of a texel never
// shrinks below what that view gives.
//
// Our camera zooms out past that view; there a single level would alias. A 2 x 2 box of bilinear taps
// stands in for the mip levels the textures lack. Per screen axis the box spans the footprint of one pixel
// in texels, less what the client already leaves to one bilinear tap: the larger of one texel and the
// footprint at the client's view (the footprint divided by client_view_scale). At or inside the client's
// view the box is empty and one bilinear tap remains, the client's filter. Zoomed out, the box grows with
// the footprint as a mip level's texel would; past a box of about four texels its taps stop overlapping
// and the far zoom-out aliases again. The box stays at four taps because every land material sample
// carries it, and more taps per sample made the land shader too slow to compile.
// ============================================================================

// Matches TEXTURE_FILTER_PIXEL_ART and TEXTURE_FILTER_CLIENT_BILINEAR in mesh_material.rs.
const TEXTURE_FILTER_PIXEL_ART: u32 = 0u;
const TEXTURE_FILTER_CLIENT_BILINEAR: u32 = 1u;

// The box's taps sit at the centres of its halves along each screen axis: a quarter of the box from the
// sample point.
const CLIENT_FILTER_BOX_TAP_OFFSET: f32 = 0.25;
// Each of the box's four taps weighs a quarter.
const CLIENT_FILTER_BOX_TAP_WEIGHT: f32 = 0.25;
// Shortest screen footprint, in texels, treated as a direction.
const CLIENT_FILTER_MIN_FOOTPRINT_TEXELS: f32 = 0.000001;

// A texel of a rectangle `extent` texels wide: repeated as a sampler's wrap address mode would, or clamped to
// the rectangle's edge texels.
fn client_rect_texel(texel: vec2<i32>, extent: vec2<i32>, wrap: bool) -> vec2<i32> {
  let size = max(extent, vec2<i32>(1));
  // WGSL `%` keeps the dividend's sign; adding the size once more brings negative texels into range.
  let wrapped = ((texel % size) + size) % size;
  return select(clamp(texel, vec2<i32>(0), size - vec2<i32>(1)), wrapped, wrap);
}

// One bilinear tap inside a rectangle of an array texture. `px` is in texels from the rectangle's origin,
// texel centres at i + 0.5. The four texels are loaded and blended here, so the taps never reach the
// atlas neighbours of the rectangle.
fn client_bilinear_rect(tex: texture_2d_array<f32>, layer: i32, origin: vec2<i32>, extent: vec2<i32>,
    px: vec2<f32>, wrap: bool) -> vec4<f32> {
  let corner = px - vec2<f32>(0.5);
  let first = vec2<i32>(floor(corner));
  let weight = corner - floor(corner);
  let t0 = client_rect_texel(first, extent, wrap);
  let t1 = client_rect_texel(first + vec2<i32>(1), extent, wrap);
  let c00 = textureLoad(tex, origin + t0, layer, 0);
  let c10 = textureLoad(tex, origin + vec2<i32>(t1.x, t0.y), layer, 0);
  let c01 = textureLoad(tex, origin + vec2<i32>(t0.x, t1.y), layer, 0);
  let c11 = textureLoad(tex, origin + t1, layer, 0);
  return mix(mix(c00, c10, weight.x), mix(c01, c11, weight.x), weight.y);
}

// The emulated mip box along one screen axis, in texels, for a footprint of `step_texels` per pixel.
fn client_mip_box_texels(step_texels: f32, client_view_scale: f32) -> f32 {
  let client_footprint = max(1.0, step_texels / max(client_view_scale, 1.0));
  return max(step_texels - client_footprint, 0.0);
}

// The emulated mip box of a sample whose texel position moves by `dx_texels` and `dy_texels` per screen
// pixel: the offsets of its taps along each screen axis, in texels. Both are zero inside the client's view.
struct ClientMipBox {
  enabled: bool,
  offset_x: vec2<f32>,
  offset_y: vec2<f32>,
}

fn client_mip_box(dx_texels: vec2<f32>, dy_texels: vec2<f32>, client_view_scale: f32) -> ClientMipBox {
  let step_x = length(dx_texels);
  let step_y = length(dy_texels);
  let box_x = client_mip_box_texels(step_x, client_view_scale);
  let box_y = client_mip_box_texels(step_y, client_view_scale);
  return ClientMipBox(
    box_x > 0.0 || box_y > 0.0,
    dx_texels * (box_x * CLIENT_FILTER_BOX_TAP_OFFSET / max(step_x, CLIENT_FILTER_MIN_FOOTPRINT_TEXELS)),
    dy_texels * (box_y * CLIENT_FILTER_BOX_TAP_OFFSET / max(step_y, CLIENT_FILTER_MIN_FOOTPRINT_TEXELS)),
  );
}

// The client's filter on a rectangle: one bilinear tap, or the emulated mip box of four. `dx_texels` and
// `dy_texels` are the screen derivatives of `px`, taken before any wrap. Every tap loads its own texels, for
// textures bound without a bilinear sampler (the sprite atlas's samples nearest).
fn client_filtered_rect(tex: texture_2d_array<f32>, layer: i32, origin: vec2<i32>, extent: vec2<i32>,
    px: vec2<f32>, wrap: bool, dx_texels: vec2<f32>, dy_texels: vec2<f32>, client_view_scale: f32) -> vec4<f32> {
  let box = client_mip_box(dx_texels, dy_texels, client_view_scale);
  if (!box.enabled) {
    return client_bilinear_rect(tex, layer, origin, extent, px, wrap);
  }
  return (client_bilinear_rect(tex, layer, origin, extent, px - box.offset_x - box.offset_y, wrap)
    + client_bilinear_rect(tex, layer, origin, extent, px + box.offset_x - box.offset_y, wrap)
    + client_bilinear_rect(tex, layer, origin, extent, px - box.offset_x + box.offset_y, wrap)
    + client_bilinear_rect(tex, layer, origin, extent, px + box.offset_x + box.offset_y, wrap)) * CLIENT_FILTER_BOX_TAP_WEIGHT;
}

// One hardware bilinear tap of a rectangle: the point wrapped into the rectangle when it repeats, then kept
// half a texel inside it so the sampler never reaches the rectangle's atlas neighbours.
fn client_sampled_rect_tap(tex: texture_2d_array<f32>, smp: sampler, layer: i32, origin: vec2<i32>,
    extent: vec2<i32>, px: vec2<f32>, wrap: bool) -> vec4<f32> {
  let size = vec2<f32>(max(extent, vec2<i32>(1)));
  let wrapped = px - floor(px / size) * size;
  let inside = clamp(select(px, wrapped, wrap), vec2<f32>(0.5), size - vec2<f32>(0.5));
  let atlas_size = vec2<f32>(textureDimensions(tex).xy);
  return textureSampleLevel(tex, smp, (vec2<f32>(origin) + inside) / atlas_size, layer, 0.0);
}

// As client_filtered_rect, for a texture with a bilinear sampler `smp`: the single tap still loads its four
// texels, so a repeating texture wraps exactly as the client's sampler does, while the emulated mip box takes
// the sampler's taps, which keep the many land samples small enough to compile. At a repetition seam the box
// taps clamp; the client never draws that zoom-out.
fn client_filtered_rect_sampled(tex: texture_2d_array<f32>, smp: sampler, layer: i32, origin: vec2<i32>,
    extent: vec2<i32>, px: vec2<f32>, wrap: bool, dx_texels: vec2<f32>, dy_texels: vec2<f32>,
    client_view_scale: f32) -> vec4<f32> {
  let box = client_mip_box(dx_texels, dy_texels, client_view_scale);
  if (!box.enabled) {
    return client_bilinear_rect(tex, layer, origin, extent, px, wrap);
  }
  return (client_sampled_rect_tap(tex, smp, layer, origin, extent, px - box.offset_x - box.offset_y, wrap)
    + client_sampled_rect_tap(tex, smp, layer, origin, extent, px + box.offset_x - box.offset_y, wrap)
    + client_sampled_rect_tap(tex, smp, layer, origin, extent, px - box.offset_x + box.offset_y, wrap)
    + client_sampled_rect_tap(tex, smp, layer, origin, extent, px + box.offset_x + box.offset_y, wrap)) * CLIENT_FILTER_BOX_TAP_WEIGHT;
}

// The client's filter on a sprite's atlas rectangle. `uv`, `uv_min`, `uv_max` and the derivatives are
// atlas-normalized.
fn client_filtered_atlas_rect(tex: texture_2d_array<f32>, layer: i32, uv: vec2<f32>, uv_min: vec2<f32>,
    uv_max: vec2<f32>, wrap: bool, uv_dx: vec2<f32>, uv_dy: vec2<f32>, client_view_scale: f32) -> vec4<f32> {
  let atlas_size = vec2<f32>(textureDimensions(tex).xy);
  let origin = vec2<i32>(round(uv_min * atlas_size));
  let extent = max(vec2<i32>(round((uv_max - uv_min) * atlas_size)), vec2<i32>(1));
  return client_filtered_rect(tex, layer, origin, extent, (uv - uv_min) * atlas_size, wrap,
    uv_dx * atlas_size, uv_dy * atlas_size, client_view_scale);
}
