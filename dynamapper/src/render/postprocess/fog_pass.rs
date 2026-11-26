// render::postprocess::fog_pass — the drifting fog: soft cloud banks over the whole frame.
//
// A fullscreen pass after tonemapping mixes the frame toward the fog colour by the density of a domain-warped
// gradient-noise layer (`fog_pass.wesl`). The layer lies on the camera plane, anchored to the map the way the KR
// client anchors its night map (`client_night::camera_plane_position`): it pans with the world, not with the screen,
// and the wind moves it slowly over the map. A second, slower warp layer makes the banks change shape as they drift.
// The client night composite runs after this pass, so the fog darkens with the night as the rest of the frame does.
//
// - `FogPostProcessUniform` is a `ShaderType` component on the `PlayerCamera` entity, extracted to the GPU each frame,
//   while the fog is on: without it the pass does not run.
// - `sys_sync_fog_uniform` builds it from `UniformState`, the camera and the virtual time (`fog_uniform`), and
//   removes it when the fog is off.
//
// The noise repeats every `FOG_NOISE_PERIOD_CELLS` cells, so the offsets are kept modulo that period in f64 and f32
// stays exact however far the camera or the wind has gone.

use crate::render::lighting::client_night::{camera_plane_position, half_view_world};
use crate::render::terrain::mesh_material::GlobalLightingUniforms;
use crate::render::uniform_state::UniformState;
use crate::render::view::camera::{PlayerCamera, RenderZoom};
use crate::prelude::*;
use bevy::core_pipeline::{
    fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin},
    tonemapping::tonemapping,
    Core3dSystems,
};
use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::BoxedSystem;
use bevy::math::DVec2;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;

// ---- Settings --------------------------------------------------------------

/// The fog's colour and opacity, `GlobalLightingUniforms::fog_tint`: `[r, g, b, amount]`, amount being the opacity
/// where the cloud is thickest. The values are the shipped default's.
pub const FOG_DEFAULT_TINT: Vec4 = Vec4::new(0.46, 0.50, 0.56, 0.22);
/// `fog_clouds.x`: the width of a cloud bank in map tiles (cell edges).
pub const FOG_DEFAULT_SCALE_TILES: f32 = 10.0;
/// `fog_clouds.y`: how much of the sky the banks cover, 0 (clear) to 1 (overcast).
pub const FOG_DEFAULT_COVERAGE: f32 = 0.5;
/// `fog_clouds.z`: the compass bearing the wind carries the fog toward, clockwise from north (decreasing UO y).
pub const FOG_DEFAULT_WIND_BEARING_DEGREES: f32 = 120.0;
/// `fog_clouds.w`: how fast the fog drifts, in map tiles per second.
pub const FOG_DEFAULT_DRIFT_TILES_PER_SECOND: f32 = 0.5;

/// The smallest cloud bank the pass draws, in tiles: below it the noise turns to grain.
pub const FOG_MIN_SCALE_TILES: f32 = 1.0;
/// The opacity below which the fog is off: the pass then does not run.
pub const FOG_MIN_AMOUNT: f32 = 0.001;
/// The noise value at which the fog starts, at coverage 0 and at coverage 1. The layer's noise lies mostly in 0.3..0.7
/// around 0.5, so coverage 0 leaves only rare wisps and coverage 1 a near-even veil.
pub const FOG_CLEAR_SKY_THRESHOLD: f32 = 0.70;
pub const FOG_OVERCAST_THRESHOLD: f32 = 0.10;
/// How far above its start the noise must be for the fog to be full: the softness of a bank's edge.
pub const FOG_EDGE_SOFTNESS: f32 = 0.45;
/// The cells after which the noise repeats. Matches `FOG_NOISE_PERIOD` in fog_noise.wesl.
pub const FOG_NOISE_PERIOD_CELLS: f64 = 64.0;
/// How fast the warp layer drifts against the cloud layer: slower, so the banks change shape as they move.
pub const FOG_WARP_DRIFT_RATIO: f64 = 0.55;

// ---- Plugin ----------------------------------------------------------------

pub struct FogPostProcessPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(FogPostProcessPlugin);

impl Plugin for FogPostProcessPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins(FullscreenMaterialPlugin::<FogPostProcessUniform>::default())
            .add_systems(Update, sys_sync_fog_uniform);
    }
}

// ---- Uniform ---------------------------------------------------------------

/// The fog pass's parameters, on the player camera only while the fog is on (`fog_uniform`).
///
/// Must match the `FogPostProcessUniform` struct in `fog_pass.wesl`.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, Debug, PartialEq)]
#[extract_app(bevy::render::RenderApp)]
pub struct FogPostProcessUniform {
    /// rgb: the fog colour, the night tint mixed in; a: the opacity where the cloud is thickest.
    pub color: Vec4,
    /// xy: half the view in cloud cells (x across, y down, both with the horizontal units per pixel);
    /// z, w: the noise value where the fog starts and where it is full.
    pub view: Vec4,
    /// xy: the cloud layer's position on the camera plane, zw: the warp layer's, in cells modulo
    /// `FOG_NOISE_PERIOD_CELLS`: the camera focus plus the wind's travel.
    pub offsets: Vec4,
}

/// The fog pass's place in `Core3d`, for passes that must run after it (the client night composite).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FogPassSystems;

impl FullscreenMaterial for FogPostProcessUniform {
    fn fragment_shader() -> ShaderRef {
        "shaders/world/effects/fog_pass.wesl".into()
    }

    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        // Run after tonemapping so fog is applied on the fully composed frame.
        // Upscaling (the final blit) runs after the whole `PostProcess` set.
        system.in_set(Core3dSystems::PostProcess).in_set(FogPassSystems).after(tonemapping)
    }
}

/// The unit vector the wind blows along on the map (x = UO x, z = UO y): bearing 0 north (decreasing UO y), 90 east.
pub fn fog_wind_direction(bearing_degrees: f32) -> DVec2 {
    let bearing = f64::from(bearing_degrees).to_radians();
    DVec2::new(bearing.sin(), -bearing.cos())
}

/// A map position on the camera plane in fog cells, modulo the noise period.
fn fog_cells(position_xz: DVec2, scale_tiles: f64) -> DVec2 {
    let cells = camera_plane_position(position_xz) / scale_tiles;
    DVec2::new(cells.x.rem_euclid(FOG_NOISE_PERIOD_CELLS), cells.y.rem_euclid(FOG_NOISE_PERIOD_CELLS))
}

/// The fog pass's parameters for `lighting` with the camera focused on `focus_xz` (the camera translation's x and z),
/// showing `half_view_world` (`client_night::half_view_world`), `elapsed_secs` into the run. `None` while the fog is
/// off or clear.
pub fn fog_uniform(
    lighting: &GlobalLightingUniforms,
    focus_xz: DVec2,
    half_view_world: Vec2,
    elapsed_secs: f64,
) -> Option<FogPostProcessUniform> {
    let amount = lighting.fog_tint.w.clamp(0.0, 1.0);
    if lighting.enable_fog == 0 || amount < FOG_MIN_AMOUNT {
        return None;
    }
    let [scale, coverage, bearing, drift] = lighting.fog_clouds.to_array();
    let scale = scale.max(FOG_MIN_SCALE_TILES);
    let start = FOG_CLEAR_SKY_THRESHOLD.lerp(FOG_OVERCAST_THRESHOLD, coverage.clamp(0.0, 1.0));
    let night = lighting.fog_night_color;
    let color = lighting.fog_tint.truncate().lerp(night.truncate(), night.w.clamp(0.0, 1.0));
    let travel = fog_wind_direction(bearing) * f64::from(drift.max(0.0)) * elapsed_secs;
    let scale = f64::from(scale);
    let cloud = fog_cells(focus_xz + travel, scale);
    let warp = fog_cells(focus_xz + travel * FOG_WARP_DRIFT_RATIO, scale);
    Some(FogPostProcessUniform {
        color: color.extend(amount),
        view: (half_view_world / scale as f32).extend(start).extend(start + FOG_EDGE_SOFTNESS),
        offsets: Vec4::new(cloud.x as f32, cloud.y as f32, warp.x as f32, warp.y as f32),
    })
}

// ---- System ----------------------------------------------------------------

/// Keeps the `FogPostProcessUniform` on the `PlayerCamera` current while the fog is on, and removes it when the fog is
/// off, so the fullscreen pass, which would pass every pixel through unchanged, does not run.
///
/// The drift reads the virtual clock, the one every animation reads: it advances by each frame's own delta, so the
/// fog moves evenly at any frame rate and the frame limiter's sleeps do not make it jump.
fn sys_sync_fog_uniform(
    mut commands: Commands,
    uniform_state: Res<UniformState>,
    time: Res<Time>,
    zoom: Res<RenderZoom>,
    mut camera_q: Query<(Entity, &Camera, &Projection, &Transform, Option<&mut FogPostProcessUniform>), With<PlayerCamera>>,
) {
    let Ok((entity, camera, projection, transform, current)) = camera_q.single_mut() else {
        return;
    };
    let focus = DVec2::new(f64::from(transform.translation.x), f64::from(transform.translation.z));
    let uniform = half_view_world(camera, projection, zoom.0)
        .and_then(|half_view| fog_uniform(&uniform_state.lighting, focus, half_view, time.elapsed_secs_f64()));
    match (uniform, current) {
        (Some(uniform), Some(mut current)) => {
            if *current != uniform {
                *current = uniform;
            }
        }
        (Some(uniform), None) => {
            commands.entity(entity).insert(uniform);
        }
        (None, Some(_)) => {
            commands.entity(entity).remove::<FogPostProcessUniform>();
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF_VIEW: Vec2 = Vec2::new(13.0, 8.125);
    const TOLERANCE: f32 = 1e-4;

    fn lighting(enable_fog: u32) -> GlobalLightingUniforms {
        GlobalLightingUniforms {
            enable_fog,
            fog_tint: FOG_DEFAULT_TINT,
            fog_clouds: Vec4::new(
                FOG_DEFAULT_SCALE_TILES,
                FOG_DEFAULT_COVERAGE,
                FOG_DEFAULT_WIND_BEARING_DEGREES,
                FOG_DEFAULT_DRIFT_TILES_PER_SECOND,
            ),
            ..default()
        }
    }

    /// The cloud-layer cell a screen uv (0..1, y down) samples, as the shader computes it.
    fn cloud_cell(uniform: &FogPostProcessUniform, screen_uv: Vec2) -> Vec2 {
        (screen_uv * 2.0 - Vec2::ONE) * uniform.view.xy() + uniform.offsets.xy()
    }

    fn wrapped_distance(a: Vec2, b: Vec2) -> f32 {
        let period = FOG_NOISE_PERIOD_CELLS as f32;
        let d = (a - b).abs();
        d.min(Vec2::splat(period) - d).max_element()
    }

    #[test]
    fn the_fog_is_off_without_its_toggle_or_its_amount() {
        assert!(fog_uniform(&lighting(0), DVec2::ZERO, HALF_VIEW, 0.0).is_none());
        let mut clear = lighting(1);
        clear.fog_tint.w = 0.0;
        assert!(fog_uniform(&clear, DVec2::ZERO, HALF_VIEW, 0.0).is_none());
        let on = fog_uniform(&lighting(1), DVec2::ZERO, HALF_VIEW, 0.0).expect("fog on");
        assert_eq!(on.color, FOG_DEFAULT_TINT);
    }

    #[test]
    fn a_map_point_keeps_its_cloud_when_the_camera_pans() {
        let fog = lighting(1);
        let focus = DVec2::new(1484.0, 1700.0);
        let point = focus + DVec2::new(3.0, -2.0);
        // Where the point is on screen for a given focus: its camera-plane offset over the half view, y down.
        let uv_of = |focus: DVec2| {
            let offset = camera_plane_position(point) - camera_plane_position(focus);
            (offset.as_vec2() / HALF_VIEW + Vec2::ONE) * 0.5
        };
        let before = fog_uniform(&fog, focus, HALF_VIEW, 0.0).unwrap();
        let panned_focus = focus + DVec2::new(2.0, 1.0);
        let after = fog_uniform(&fog, panned_focus, HALF_VIEW, 0.0).unwrap();
        let (cell_before, cell_after) = (cloud_cell(&before, uv_of(focus)), cloud_cell(&after, uv_of(panned_focus)));
        assert!(wrapped_distance(cell_before, cell_after) < TOLERANCE, "{cell_before} vs {cell_after}");
        // Far across the map f32 still keeps it.
        let far = DVec2::new(5100.0, 3900.0);
        let far_before = fog_uniform(&fog, far, HALF_VIEW, 0.0).unwrap();
        let far_after = fog_uniform(&fog, far + DVec2::X, HALF_VIEW, 0.0).unwrap();
        let shift = (far_after.offsets.xy() - far_before.offsets.xy()) * FOG_DEFAULT_SCALE_TILES;
        assert!((shift - Vec2::splat(std::f32::consts::FRAC_1_SQRT_2)).abs().max_element() < TOLERANCE, "{shift}");
    }

    #[test]
    fn the_wind_carries_the_fog_toward_its_bearing() {
        let mut fog = lighting(1);
        let seconds = 10.0;
        // East (UO x + 1) is half right and half down on the screen; the layer moves with the wind.
        fog.fog_clouds.z = 90.0;
        let still = fog_uniform(&fog, DVec2::ZERO, HALF_VIEW, 0.0).unwrap();
        let moved = fog_uniform(&fog, DVec2::ZERO, HALF_VIEW, seconds).unwrap();
        let travel_cells = (FOG_DEFAULT_DRIFT_TILES_PER_SECOND * seconds as f32) / FOG_DEFAULT_SCALE_TILES;
        let expected = Vec2::splat(travel_cells * std::f32::consts::FRAC_1_SQRT_2);
        assert!((moved.offsets.xy() - still.offsets.xy() - expected).abs().max_element() < TOLERANCE);
        // The warp layer lags behind, so the banks change shape as they go.
        let warp = moved.offsets.zw() - still.offsets.zw();
        assert!((warp - expected * FOG_WARP_DRIFT_RATIO as f32).abs().max_element() < TOLERANCE, "{warp}");
        // No drift, no motion: what the regression harness freezes.
        fog.fog_clouds.w = 0.0;
        let frozen = fog_uniform(&fog, DVec2::ZERO, HALF_VIEW, seconds).unwrap();
        assert_eq!(frozen.offsets, fog_uniform(&fog, DVec2::ZERO, HALF_VIEW, 0.0).unwrap().offsets);
        // North is straight up and right: (x - z) / sqrt 2 = +, (x + z) / sqrt 2 = -.
        assert!((fog_wind_direction(0.0) - DVec2::new(0.0, -1.0)).length() < 1e-12);
    }

    #[test]
    fn the_offsets_stay_in_one_noise_period_after_hours_of_wind() {
        let fog = lighting(1);
        let hours = 3600.0 * 12.0;
        let uniform = fog_uniform(&fog, DVec2::new(6000.0, 4000.0), HALF_VIEW, hours).unwrap();
        let period = FOG_NOISE_PERIOD_CELLS as f32;
        assert!(uniform.offsets.cmpge(Vec4::ZERO).all() && uniform.offsets.cmplt(Vec4::splat(period)).all());
    }

    #[test]
    fn coverage_scale_and_night_tint_shape_the_layer() {
        let mut fog = lighting(1);
        fog.fog_clouds.y = 0.0;
        let clear = fog_uniform(&fog, DVec2::ZERO, HALF_VIEW, 0.0).unwrap();
        fog.fog_clouds.y = 1.0;
        let overcast = fog_uniform(&fog, DVec2::ZERO, HALF_VIEW, 0.0).unwrap();
        assert!((clear.view.z - FOG_CLEAR_SKY_THRESHOLD).abs() < TOLERANCE);
        assert!((overcast.view.z - FOG_OVERCAST_THRESHOLD).abs() < TOLERANCE);
        assert!((overcast.view.w - overcast.view.z - FOG_EDGE_SOFTNESS).abs() < TOLERANCE);
        // The view in cells is the view in tiles over the scale, and a scale below the floor is the floor.
        assert!((overcast.view.xy() - HALF_VIEW / FOG_DEFAULT_SCALE_TILES).abs().max_element() < TOLERANCE);
        fog.fog_clouds.x = 0.0;
        let floor = fog_uniform(&fog, DVec2::ZERO, HALF_VIEW, 0.0).unwrap();
        assert!((floor.view.xy() - HALF_VIEW / FOG_MIN_SCALE_TILES).abs().max_element() < TOLERANCE);
        // The night tint replaces the colour by its blend; the amount stays.
        let night = Vec3::new(0.05, 0.06, 0.09);
        fog.fog_night_color = night.extend(1.0);
        let dark = fog_uniform(&fog, DVec2::ZERO, HALF_VIEW, 0.0).unwrap();
        assert!((dark.color.truncate() - night).abs().max_element() < TOLERANCE);
        assert_eq!(dark.color.w, FOG_DEFAULT_TINT.w);
    }

    const FOG_NOISE_WESL: &str = include_str!("../../../assets/shaders/world/effects/fog_noise.wesl");
    /// The grid the GPU cases sample: 64 by 64 points a quarter cell apart, 16 cells across.
    const GRID_SIDE: usize = 64;
    const GRID_STEP_CELLS: f32 = 0.25;
    /// A screen pixel at the default view (26 tiles across 1280 pixels) in cells at the default scale.
    const PIXEL_CELLS: f32 = 26.0 / 1280.0 / FOG_DEFAULT_SCALE_TILES;
    /// The most a pixel step may change the density: the banks have soft edges, never grain.
    const MAX_DENSITY_STEP_PER_PIXEL: f32 = 0.02;

    /// `fog_density` at each case: `[s.x, s.y, start, full]` and the offsets.
    fn gpu_fog_density(cases: &[([f32; 4], [f32; 4])]) -> Vec<f32> {
        let inputs: Vec<[f32; 4]> = cases.iter().flat_map(|(place, offsets)| [*place, *offsets]).collect();
        let source = r#"
import package::shaders::world::effects::fog_noise::fog_density;
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    let place = cases[index.x * 2u];
    result[index.x] = vec4<f32>(fog_density(place.xy, cases[index.x * 2u + 1u], place.z, place.w), 0.0, 0.0, 0.0);
}
"#;
        crate::util_lib::wesl_compute::run_wesl_compute(
            &[(FOG_NOISE_WESL, "shaders/world/effects/fog_noise.wesl")],
            source,
            &inputs,
            cases.len(),
            cases.len(),
        )
        .iter()
        .map(|density| density[0])
        .collect()
    }

    fn grid_cases(coverage: f32, shift: Vec2, offsets: Vec4) -> Vec<([f32; 4], [f32; 4])> {
        let start = FOG_CLEAR_SKY_THRESHOLD.lerp(FOG_OVERCAST_THRESHOLD, coverage);
        (0..GRID_SIDE * GRID_SIDE)
            .map(|index| {
                let s = Vec2::new((index % GRID_SIDE) as f32, (index / GRID_SIDE) as f32) * GRID_STEP_CELLS + shift;
                ([s.x, s.y, start, start + FOG_EDGE_SOFTNESS], offsets.to_array())
            })
            .collect()
    }

    fn mean(values: &[f32]) -> f32 {
        values.iter().sum::<f32>() / values.len() as f32
    }

    /// Coverage runs from rare wisps to a near-even veil, with patchy banks in between at the presets' 0.5.
    #[test]
    #[ignore = "requires a Vulkan adapter; run explicitly for fog changes"]
    fn gpu_coverage_runs_from_wisps_to_a_veil() {
        let offsets = Vec4::new(5.0, 9.0, 21.0, 33.0);
        let clear = mean(&gpu_fog_density(&grid_cases(0.0, Vec2::ZERO, offsets)));
        let patchy = gpu_fog_density(&grid_cases(FOG_DEFAULT_COVERAGE, Vec2::ZERO, offsets));
        let overcast = mean(&gpu_fog_density(&grid_cases(1.0, Vec2::ZERO, offsets)));
        let (thin, thick) = (patchy.iter().filter(|d| **d < 0.05).count(), patchy.iter().filter(|d| **d > 0.5).count());
        assert!(clear < 0.05, "clear sky {clear}");
        assert!(overcast > 0.8, "overcast {overcast}");
        assert!((0.1..0.5).contains(&mean(&patchy)), "patchy {}", mean(&patchy));
        // Patchy: both clear gaps and thick banks.
        assert!(thin > patchy.len() / 10 && thick > patchy.len() / 20, "{thin} clear, {thick} thick");
    }

    /// The pattern repeats after one noise period, so the CPU may keep the offsets modulo it; within it, a pixel's
    /// step changes the density only a little.
    #[test]
    #[ignore = "requires a Vulkan adapter; run explicitly for fog changes"]
    fn gpu_the_fog_repeats_on_its_period_and_has_no_grain() {
        let period = FOG_NOISE_PERIOD_CELLS as f32;
        let offsets = Vec4::new(3.5, 60.25, 12.0, 47.5);
        let base = gpu_fog_density(&grid_cases(FOG_DEFAULT_COVERAGE, Vec2::ZERO, offsets));
        let wrapped = gpu_fog_density(&grid_cases(FOG_DEFAULT_COVERAGE, Vec2::ZERO, offsets + Vec4::splat(period)));
        let largest = base.iter().zip(&wrapped).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(largest < 1e-3, "{largest}");
        let stepped = gpu_fog_density(&grid_cases(FOG_DEFAULT_COVERAGE, Vec2::splat(PIXEL_CELLS), offsets));
        let step = base.iter().zip(&stepped).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(step < MAX_DENSITY_STEP_PER_PIXEL, "{step}");
    }
}
