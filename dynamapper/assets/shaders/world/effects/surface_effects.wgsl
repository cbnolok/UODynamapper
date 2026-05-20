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

// Applies the ClassicUO water scaling effect to a UV coordinate relative to a center.
fn apply_water_animation(uv: vec2<f32>, centre: vec2<f32>) -> vec2<f32> {
    let scale = get_water_animation_scale();
    
    // Scaling relative to centre:
    // Dividing by scale > 1.0 shrinks the range sampled from the texture.
    // This zooms IN on the texture, staying within the original boundaries.
    return centre + (uv - centre) / scale;
}
