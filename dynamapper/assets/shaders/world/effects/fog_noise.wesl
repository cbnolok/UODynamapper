// ============================================================================
// fog_noise.wesl — the drifting fog's density (effects/fog_pass.wesl).
//
// A domain-warped gradient noise on the camera plane: a two-octave warp layer at half frequency bends a four-octave
// cloud layer, and the cloud layer's value between the fog's start and full values is the density. The noise repeats
// every FOG_NOISE_PERIOD cells, every octave on whole multiples of it, so the CPU keeps the layers' offsets modulo
// the period and the pattern never jumps.
// ============================================================================

// The cells after which the noise repeats. Matches FOG_NOISE_PERIOD_CELLS in fog_pass.rs.
const FOG_NOISE_PERIOD: i32 = 64;
// The cloud layer's octaves: four, the finest an eighth of a bank, give the banks soft, wispy edges.
const FOG_CLOUD_OCTAVES: i32 = 4;
// The warp layer: two octaves at half the cloud layer's frequency bend the banks without streaking them.
const FOG_WARP_OCTAVES: i32 = 2;
const FOG_WARP_FREQUENCY: f32 = 0.5;
// How far the warp moves a bank's shape, in cells.
const FOG_WARP_CELLS: f32 = 0.4;
// The warp's second component reads the warp layer this far away, so the two components are unrelated.
const FOG_WARP_SECOND_SAMPLE: vec2<f32> = vec2<f32>(17.3, 41.9);
// The seeds of the three noise fields; each octave adds its index.
const FOG_CLOUD_SEED: u32 = 0u;
const FOG_WARP_X_SEED: u32 = 16u;
const FOG_WARP_Y_SEED: u32 = 32u;
// Gradient noise spans about ±0.7; this scales it to about ±0.5 around the 0.5 the thresholds expect.
const FOG_GRADIENT_NOISE_GAIN: f32 = 0.9;
const FOG_TAU: f32 = 6.2831853;
// The multiplier of the integer hash (Numerical Recipes' LCG) and the golden-ratio seed step.
const FOG_HASH_MULTIPLIER: u32 = 1664525u;
const FOG_HASH_INCREMENT: u32 = 1013904223u;
const FOG_HASH_SEED_STEP: u32 = 0x9E3779B9u;
const FOG_HASH_SHIFT: u32 = 16u;
const FOG_U32_MAX: f32 = 4294967295.0;

// A hash of a lattice point, wrapped to the period first, in 0..1.
fn fog_hash(cell: vec2<i32>, period: i32, seed: u32) -> f32 {
    let wrapped = vec2<u32>(((cell % period) + period) % period);
    var v = wrapped * FOG_HASH_MULTIPLIER + vec2<u32>(FOG_HASH_INCREMENT + seed * FOG_HASH_SEED_STEP, FOG_HASH_INCREMENT);
    v.x += v.y * FOG_HASH_MULTIPLIER;
    v.y += v.x * FOG_HASH_MULTIPLIER;
    v = v ^ (v >> vec2<u32>(FOG_HASH_SHIFT));
    v.x += v.y * FOG_HASH_MULTIPLIER;
    v.y += v.x * FOG_HASH_MULTIPLIER;
    v = v ^ (v >> vec2<u32>(FOG_HASH_SHIFT));
    return f32(v.x) / FOG_U32_MAX;
}

// The dot of a lattice point's random unit gradient with the offset from it.
fn fog_gradient(cell: vec2<i32>, period: i32, seed: u32, offset: vec2<f32>) -> f32 {
    let angle = fog_hash(cell, period, seed) * FOG_TAU;
    return dot(vec2<f32>(cos(angle), sin(angle)), offset);
}

// Gradient noise in about 0..1 around 0.5, repeating every `period` cells.
fn fog_noise(p: vec2<f32>, period: i32, seed: u32) -> f32 {
    let base = floor(p);
    let cell = vec2<i32>(base);
    let f = p - base;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = fog_gradient(cell, period, seed, f);
    let b = fog_gradient(cell + vec2<i32>(1, 0), period, seed, f - vec2<f32>(1.0, 0.0));
    let c = fog_gradient(cell + vec2<i32>(0, 1), period, seed, f - vec2<f32>(0.0, 1.0));
    let d = fog_gradient(cell + vec2<i32>(1, 1), period, seed, f - vec2<f32>(1.0, 1.0));
    return 0.5 + FOG_GRADIENT_NOISE_GAIN * mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Fractal noise: `octaves` octaves from `frequency` up, each twice the frequency at half the weight, normalised to
// about 0..1. `frequency` times the period must be whole.
fn fog_fbm(p: vec2<f32>, octaves: i32, frequency: f32, seed: u32) -> f32 {
    var sum = 0.0;
    var weight = 0.5;
    var total = 0.0;
    var f = frequency;
    for (var octave = 0; octave < octaves; octave += 1) {
        sum += weight * fog_noise(p * f, i32(f32(FOG_NOISE_PERIOD) * f), seed + u32(octave));
        total += weight;
        weight *= 0.5;
        f *= 2.0;
    }
    return sum / total;
}

// The fog's density, 0..1, at a screen offset from the focus `s` (cells), for the layers' offsets and the fog's
// start and full noise values.
fn fog_density(s: vec2<f32>, offsets: vec4<f32>, start: f32, full: f32) -> f32 {
    let warp_point = s + offsets.zw;
    let warp = vec2<f32>(
        fog_fbm(warp_point, FOG_WARP_OCTAVES, FOG_WARP_FREQUENCY, FOG_WARP_X_SEED),
        fog_fbm(warp_point + FOG_WARP_SECOND_SAMPLE, FOG_WARP_OCTAVES, FOG_WARP_FREQUENCY, FOG_WARP_Y_SEED),
    ) - vec2<f32>(0.5);
    let n = fog_fbm(s + offsets.xy + warp * FOG_WARP_CELLS, FOG_CLOUD_OCTAVES, 1.0, FOG_CLOUD_SEED);
    return smoothstep(start, full, n);
}
