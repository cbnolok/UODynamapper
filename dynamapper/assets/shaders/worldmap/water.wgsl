// ============================================================================
// water.wgsl — Animated water UV distortion.
//
// Shared by the land terrain shader and the art tile sprite shaders.
//
// Applies the classic UO "wet tile" sin/cos scale animation, based on the
// ClassicUO reference implementation (View.cs / LandView.cs):
//
//   var sin = Math.Sin(Time.Ticks / 1000f);
//   var cos = Math.Cos(Time.Ticks / 1000f);
//   scale = new Vector2(1.1f + sin * 0.1f, 1.1f + cos * 0.5f * 0.1f);
//
// The effect breathes the sampled UV region slightly larger than 1.0:
//   - X range ≈ 1.0–1.2  (driven by sin)
//   - Y range ≈ 1.05–1.15 (driven by cos * 0.5)
// The independent X/Y frequencies create a gentle diagonal drift that reads
// as "wavy" without requiring any vertex displacement.
// ============================================================================

#import bevy_pbr::mesh_view_bindings::globals

// ---- Water animation constants (faithful ClassicUO port) ----

// Scales globals.time (seconds) to approximate ClassicUO's Time.Ticks/1000 (ms/1000 = s).
// ClassicUO: sin(Ticks / 1000) → one cycle per ~6.28 seconds.
// globals.time is already in seconds, so the mapping is 1:1.
const WATER_BASE_SCALE: f32    = 1.1;   // base UV over-sample factor (always > 1)
const WATER_SIN_AMP: f32       = 0.1;   // X-axis amplitude (matches ClassicUO)
const WATER_COS_AMP: f32       = 0.05;  // Y-axis amplitude (cos * 0.5 * 0.1)

// Applies animated water distortion to a UV coordinate in [0,1] tile space.
//
// The UV is rescaled around the tile centre (0.5, 0.5) by a per-axis
// animated factor > 1.0.  Sampling with scale > 1.0 means we read a region
// slightly *larger* than the tile boundary, producing a scrolling / breathing
// appearance when rendered with a tiling atlas sampler.
//
// Parameters:
//   uv — normalized UV in [0,1] tile space.
//
// Returns:
//   Distorted UV, may slightly exceed [0,1]; atlas samplers should wrap.
fn water_distort_uv(uv: vec2<f32>) -> vec2<f32> {
    // globals.time is elapsed seconds (Bevy built-in, auto-updated, wraps at 1 h).
    // ClassicUO formula: sin(Ticks / 1000) where Ticks is milliseconds.
    // Since globals.time is already in seconds this maps directly.
    let t = globals.time;

    let sin_t = sin(t);
    let cos_t = cos(t);

    // Animated per-axis scale factor — each axis oscillates independently.
    let scale_x = WATER_BASE_SCALE + sin_t * WATER_SIN_AMP;
    let scale_y = WATER_BASE_SCALE + cos_t * WATER_COS_AMP;

    // Rescale UV around the tile centre so the animation is symmetric.
    // Dividing by scale > 1 compresses the [0,1] tile into a smaller UV range,
    // which — combined with the atlas sampler — creates the scrolling effect.
    let centre = vec2<f32>(0.5, 0.5);
    return centre + (uv - centre) / vec2<f32>(scale_x, scale_y);
}
