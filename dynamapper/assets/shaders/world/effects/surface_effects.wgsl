#import bevy_pbr::mesh_view_bindings::globals

// Faithful port of ClassicUO's water animation scaling algorithm.
//
// From ClassicUO (View.cs / LandView.cs):
//   var sin = (float)Math.Sin(Time.Ticks / 1000f);
//   var cos = (float)Math.Cos(Time.Ticks / 1000f);
//   scale = new Vector2(1.1f + sin * 0.1f, 1.1f + cos * 0.5f * 0.1f);
//
// This produces a per-axis scale factor that breathes over time.
// Applying this scale to UVs relative to the tile/sprite center
// ensures we sample a *smaller* sub-region of the texture, effectively
// zooming in and out, which avoids bleeding into neighboring atlas tiles.

fn get_water_animation_scale() -> vec2<f32> {
    let t = globals.time;
    let s = sin(t);
    let c = cos(t);
    
    // Scale factors oscillate around ~1.3 to allow stronger amplitude
    // without dropping below 1.0 (which would sample neighboring tiles).
    return vec2<f32>(
        1.3 + s * 0.28,
        1.3 + c * 0.14 // 0.5 * 0.28 = 0.14
    );
}

// Half-Lambert wrap used for the flat liquid diffuse term (see below).
const WATER_DIFFUSE_WRAP: f32 = 0.5;

// Wrapped Lambert term for a level (up-facing) liquid surface.
//
// Water is flat, so its lit brightness must NOT depend on the underlying heightfield
// slope: shading wet terrain by the seabed's central-difference normal bakes a faceted
// per-tile brightness gradient into the water that quantizes into tile-grid-aligned
// bands at low night luminance. Using a plain up-normal Lambert (`max(dot(up, L), 0)`)
// removes that variation but collapses to ~0 when the night sun is near the horizon,
// crushing the water to black. A half-Lambert wrap keeps a soft, uniform mid-brightness
// instead. The result is a single scalar shared by the terrain-water path (land Classic
// vertex Lambert) and the CC flat-quad water path (art fragment), so both liquid
// surfaces resolve to the same even tone and seam without a visible brightness step.
fn water_flat_wrapped_lambert(light_dir: vec3<f32>) -> f32 {
    let up_ndotl = max(light_dir.y, 0.0);
    return clamp(up_ndotl * (1.0 - WATER_DIFFUSE_WRAP) + WATER_DIFFUSE_WRAP, 0.0, 1.0);
}

// Applies the ClassicUO water scaling effect to a UV coordinate relative to a center.
fn apply_water_animation(uv: vec2<f32>, centre: vec2<f32>) -> vec2<f32> {
    let scale = get_water_animation_scale();
    
    // Scaling relative to centre:
    // Dividing by scale > 1.0 shrinks the range sampled from the texture.
    // This zooms IN on the texture, staying within the original boundaries.
    return centre + (uv - centre) / scale;
}
