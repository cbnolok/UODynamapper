//! art/projected_shadow.wesl, the sprite shadows cast under the sun: who casts, how far the sun's elevation leans them,
//! and the footprint of a wall's two faces, found per ground point and read through the wall's card.
use bevy::prelude::*;

use super::static_batches::{CLIENT_SHADOW_FLAG_CASTS, CLIENT_SHADOW_FLAG_WALL};
use super::static_placement::StaticDepthClass;
use crate::render::terrain::mesh_material::SunAngles;
use crate::util_lib::wesl_compute::run_wesl_compute;
use crate::util_lib::wesl_source::{wesl_const, wesl_f32_const};

const PROJECTED_SHADOW_WESL: &str = include_str!("../../../assets/shaders/world/art/projected_shadow.wesl");
/// The world height of one z (uo_coords.wesl, UO_Z_TO_WORLD_SCALE).
const HEIGHT_SCALE: f32 = 4.0 / 44.0;
/// The Britain keep's wall at the sconces (map 1, 0x0081 on row 1593, 22 high), far into the map.
const CELL: Vec2 = Vec2::new(1335.0, 1593.0);
const WALL_HEIGHT: f32 = 22.0 * HEIGHT_SCALE;
/// UOD-Painterly's afternoon Length.
const SHADOW_LENGTH: f32 = 0.44;
/// From a cell's corner to the statics' anchor (statics_collect.rs, EC_STATIC_TILE_TRANSLATION_X and _Z), and a card
/// about a wall's size around it.
const ANCHOR_OFFSET: Vec2 = Vec2::new(0.5, 1.5);
const CARD_MIN: Vec2 = Vec2::new(-0.7, -0.4);
const CARD_MAX: Vec2 = Vec2::new(0.7, 2.6);
/// The anchor's height: a wall's base a few z up.
const ANCHOR_Y: f32 = 0.37;
const TOLERANCE: f32 = 1e-4;
/// A face point found from a ground point far into the map, where the world's f32 precision is about 1e-4: its place
/// to a thousandth of a tile, its height (that precision over the step across the face) to about a fiftieth of a z.
const POSITION_TOLERANCE: f32 = 1e-3;
const HEIGHT_TOLERANCE: f32 = 2e-3;
/// Under this step across a face's plane the sun shines along the face (WALL_SHADOW_MIN_STEP_ACROSS, with margin).
const MIN_STEP_ACROSS: f32 = 1e-3;
/// The suns of the cases: the presets' default (north-west), KR-like's and Enhanced's (south), the east at a low
/// elevation, and the south-west.
const SUNS: [SunAngles; 4] = [
    SunAngles { azimuth_degrees: 315.0, elevation_degrees: 60.50379 },
    SunAngles { azimuth_degrees: 180.0, elevation_degrees: 45.0 },
    SunAngles { azimuth_degrees: 100.0, elevation_degrees: 20.0 },
    SunAngles { azimuth_degrees: 225.0, elevation_degrees: 70.0 },
];

/// The step projected_shadow.wesl takes under the default sun is the preset's Length along the ground: the lengths were
/// tuned under that sun.
#[test]
fn the_reference_sun_is_the_default_sun() {
    let light = SunAngles::default().light_direction();
    let run = light.xz().length() / light.y;
    assert!((wesl_f32_const(PROJECTED_SHADOW_WESL, "PROJECTED_SHADOW_REFERENCE_SUN_RUN") - run).abs() < 1e-6);
}

/// The shadows' darkness follows the sun's share of flat ground's light: none without a sun, more with a stronger or
/// higher sun, less with more ambient light, and all of it with no ambient light.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for projected shadow changes"]
fn gpu_the_sun_strength_darkens_the_shadows() {
    let default_sun = SunAngles::default().light_direction();
    let low_sun = SunAngles { azimuth_degrees: 315.0, elevation_degrees: 20.0 }.light_direction();
    // UOD-Painterly's afternoon ambient light and Sun Strength, then the slider's ends and the cases above.
    let (ambient, strength) = (0.25, 1.35);
    let cases = [(default_sun, ambient, 0.0), (default_sun, ambient, strength), (default_sun, ambient, 2.0),
        (low_sun, ambient, strength), (default_sun, 0.5, strength), (default_sun, 0.0, strength)];
    let inputs: Vec<[f32; 4]> = cases.iter().flat_map(|(light, ambient, strength)|
        [light.extend(0.0).to_array(), [*ambient, *strength, 0.0, 0.0]]).collect();
    let actual = run_projected_shadow_compute(r#"
import package::shaders::world::art::projected_shadow::projected_shadow_sun_share;
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    let light = cases[index.x * 2u];
    let ambient_and_sun = cases[index.x * 2u + 1u];
    result[index.x] = vec4<f32>(projected_shadow_sun_share(light.xyz, ambient_and_sun.x, ambient_and_sun.y), 0.0,
        0.0, 0.0);
}
"#, &inputs, cases.len(), cases.len());
    let share: Vec<f32> = actual.iter().map(|result| result[0]).collect();
    for ((light, ambient, strength), share) in cases.iter().zip(&share) {
        let sun = strength * light.normalize().y;
        assert!((share - sun / (ambient + sun)).abs() < TOLERANCE, "{share}");
    }
    assert_eq!(share[0], 0.0, "no sun casts no shadow");
    assert!(share[2] > share[1] && share[1] > share[3], "a stronger sun darkens, a lower one lightens: {share:?}");
    assert!(share[4] < share[1], "more ambient light lightens: {share:?}");
    assert!((share[5] - 1.0).abs() < TOLERANCE, "without ambient light the sun is all the light: {share:?}");
}

/// The Rust mirror of `projected_shadow_step`.
fn expected_step(sun: SunAngles, length: f32) -> Vec2 {
    let light = sun.light_direction();
    let reference = wesl_f32_const(PROJECTED_SHADOW_WESL, "PROJECTED_SHADOW_REFERENCE_SUN_RUN");
    let max_ratio = wesl_f32_const(PROJECTED_SHADOW_WESL, "PROJECTED_SHADOW_MAX_SUN_RUN_RATIO");
    if light.xz().length() < wesl_f32_const(PROJECTED_SHADOW_WESL, "PROJECTED_SHADOW_MIN_LIGHT_XZ") {
        return Vec2::ZERO;
    }
    let ratio = (light.xz().length() / light.y.max(f32::MIN_POSITIVE) / reference).min(max_ratio);
    -light.xz().normalize() * length * ratio
}

/// The sun's azimuth turns the shadows and its elevation lengthens them: the default sun keeps the preset's Length,
/// a sun overhead casts none and a sun on the horizon stops at the longest.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for projected shadow changes"]
fn gpu_the_sun_turns_and_lengthens_the_shadows() {
    let mut suns = SUNS.to_vec();
    suns.extend([SunAngles { azimuth_degrees: 0.0, elevation_degrees: 90.0 },
        SunAngles { azimuth_degrees: 30.0, elevation_degrees: 2.0 },
        SunAngles { azimuth_degrees: 270.0, elevation_degrees: 0.0 }]);
    let inputs: Vec<[f32; 4]> = suns.iter().map(|sun| sun.light_direction().extend(SHADOW_LENGTH).to_array()).collect();
    let actual = run_projected_shadow_compute(r#"
import package::shaders::world::art::projected_shadow::projected_shadow_step;
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    result[index.x] = vec4<f32>(projected_shadow_step(cases[index.x].xyz, cases[index.x].w), 0.0, 0.0);
}
"#, &inputs, inputs.len(), inputs.len());
    for (sun, step) in suns.iter().zip(&actual) {
        let step = Vec2::new(step[0], step[1]);
        let expected = expected_step(*sun, SHADOW_LENGTH);
        assert!((step - expected).abs().max_element() < TOLERANCE, "{sun:?}: {step}, expected {expected}");
    }
    let default_step = Vec2::new(actual[0][0], actual[0][1]);
    assert!((default_step.length() - SHADOW_LENGTH).abs() < TOLERANCE, "the default sun keeps the Length");
    assert!(default_step.x > 0.0 && default_step.y > 0.0, "a north-west sun casts to the south-east");
    let south_step = Vec2::new(actual[1][0], actual[1][1]);
    assert!(south_step.x.abs() < TOLERANCE && south_step.y < -SHADOW_LENGTH, "a lower sun from the south casts \
        longer, to the north");
    assert_eq!(actual[4][..2], [0.0, 0.0], "a sun overhead casts no shadow");
    let max_ratio = wesl_f32_const(PROJECTED_SHADOW_WESL, "PROJECTED_SHADOW_MAX_SUN_RUN_RATIO");
    for low in &actual[5..] {
        assert!((Vec2::new(low[0], low[1]).length() - SHADOW_LENGTH * max_ratio).abs() < TOLERANCE,
            "a sun on the horizon stops at the longest shadow");
    }
}

/// Only what the KR client casts a shadow for casts one (client_shadow.wesl): no roof, nor stairs and the other
/// Surface items, whose tiles' shadows, lying on the plane of each tile's base, fell on the tiles beside them. Floors,
/// the Background statics and the EC item surfaces cast none either; the other statics and foliage do.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for projected shadow changes"]
fn gpu_roofs_floors_and_surfaces_cast_no_shadow() {
    assert_eq!(wesl_const(PROJECTED_SHADOW_WESL, "PROJECTED_SHADOW_CLIENT_CASTS"), format!("{CLIENT_SHADOW_FLAG_CASTS}u"));
    const FLAT_QUADS: [u32; 4] = [0, 1, 2, 3];
    let classes = [StaticDepthClass::Regular, StaticDepthClass::Background, StaticDepthClass::Foliage,
        StaticDepthClass::Roof, StaticDepthClass::SurfaceLikeFloor];
    let mut inputs = Vec::new();
    for class in classes {
        for flat_quad in FLAT_QUADS {
            for client_shadow_flags in [0, CLIENT_SHADOW_FLAG_CASTS, CLIENT_SHADOW_FLAG_CASTS | CLIENT_SHADOW_FLAG_WALL] {
                inputs.push([class.encoded() as f32, flat_quad as f32, client_shadow_flags as f32, 0.0]);
            }
        }
    }
    let actual = run_projected_shadow_compute(r#"
import package::shaders::world::art::projected_shadow::projected_shadow_casts;
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    let casts = projected_shadow_casts(u32(cases[index.x].x), u32(cases[index.x].y), u32(cases[index.x].z));
    result[index.x] = vec4<f32>(select(0.0, 1.0, casts), 0.0, 0.0, 0.0);
}
"#, &inputs, inputs.len(), inputs.len());
    for (input, result) in inputs.iter().zip(&actual) {
        let class = classes.into_iter().find(|class| class.encoded() as f32 == input[0]).unwrap();
        let casts = matches!(class, StaticDepthClass::Regular | StaticDepthClass::Foliage) && input[1] < 2.0
            && input[2] as u32 & CLIENT_SHADOW_FLAG_CASTS != 0;
        assert_eq!(result[0] == 1.0, casts, "{class:?}, flat quad {}, client shadow flags {}", input[1], input[2]);
    }
}

/// A wall's shadow is its two faces' parallelogram along the ground: each ground point finds the face point that casts
/// onto it, and none past its top or its end, and the ground box drawn for the shadow holds every such point.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for projected shadow changes"]
fn gpu_a_walls_shadow_is_its_faces_cast_along_the_ground() {
    // (face point x, z, height, east face) on the south face (z = cell y + 1) and the east face (x = cell x + 1).
    let mut faces = Vec::new();
    for along in [0.05, 0.3, 0.75, 0.95] {
        for height in [0.1, 0.5, 0.99] {
            faces.push((Vec2::new(CELL.x + along, CELL.y + 1.0), height * WALL_HEIGHT, false));
            faces.push((Vec2::new(CELL.x + 1.0, CELL.y + along), height * WALL_HEIGHT, true));
        }
    }
    let mut inputs = Vec::new();
    // Per case: the face point found (x, z, height, found) and whether the ground box holds the ground point.
    let mut expected = Vec::new();
    for sun in SUNS {
        let step = expected_step(sun, SHADOW_LENGTH);
        for &(point, height, east) in &faces {
            let face = if east { 1.0 } else { 0.0 };
            // A sun along the face's plane casts it as a line: no ground point finds it.
            let across = if east { step.x } else { step.y };
            let ground = point + step * height;
            inputs.extend([[ground.x, ground.y, face, 0.0], [step.x, step.y, 0.0, 0.0]]);
            expected.push((across.abs() > MIN_STEP_ACROSS).then_some((point, height)));
            // Past the wall's top, and beyond the end of the face, no point of it casts there.
            let above = point + step * (WALL_HEIGHT * 1.2);
            inputs.extend([[above.x, above.y, face, 0.0], [step.x, step.y, 0.0, 0.0]]);
            expected.push(None);
            let past_end = ground + if east { Vec2::Y } else { Vec2::X } * 1.1;
            inputs.extend([[past_end.x, past_end.y, face, 0.0], [step.x, step.y, 0.0, 0.0]]);
            expected.push(None);
        }
    }
    let actual = run_projected_shadow_compute(&format!(r#"
import package::shaders::world::art::projected_shadow::{{wall_shadow_face_point, wall_shadow_ground_box}};
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {{
    let ground = cases[index.x * 2u].xy;
    let east = cases[index.x * 2u].z > 0.5;
    let step = cases[index.x * 2u + 1u].xy;
    let cell = vec2<f32>({cell_x:?}, {cell_y:?});
    let found = wall_shadow_face_point(ground, cell, {height:?}, step, east);
    let box = wall_shadow_ground_box(cell, {height:?}, step);
    let in_box = all(ground >= box.xy) && all(ground <= box.zw);
    result[index.x * 2u] = found;
    result[index.x * 2u + 1u] = vec4<f32>(select(0.0, 1.0, in_box), 0.0, 0.0, 0.0);
}}
"#, cell_x = CELL.x, cell_y = CELL.y, height = WALL_HEIGHT), &inputs, expected.len(), inputs.len());
    for (case, want) in expected.iter().enumerate() {
        let [x, z, height, found] = actual[case * 2];
        let [in_box, _, _, _] = actual[case * 2 + 1];
        match want {
            Some((point, want_height)) => {
                assert_eq!(found, 1.0, "case {case}: the face point casting on the ground point");
                assert!((Vec2::new(x, z) - *point).abs().max_element() < POSITION_TOLERANCE
                    && (height - want_height).abs() < HEIGHT_TOLERANCE, "case {case}: found ({x}, {z}) at {height}, expected \
                    {point} at {want_height}");
                assert_eq!(in_box, 1.0, "case {case}: the ground box holds the shadow");
            }
            None => assert_eq!(found, 0.0, "case {case}: no point of the face casts there"),
        }
    }
}

/// The face point's place on the wall's card: the corner the billboard (main.wesl) shows it at, for every point along
/// the view line through that corner.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for projected shadow changes"]
fn gpu_a_face_point_reads_the_card_where_the_billboard_shows_it() {
    let anchor = Vec3::new(CELL.x + ANCHOR_OFFSET.x, ANCHOR_Y, CELL.y + ANCHOR_OFFSET.y);
    let right = Vec3::new(std::f32::consts::FRAC_1_SQRT_2, 0.0, -std::f32::consts::FRAC_1_SQRT_2);
    // The default isometric camera's backward axis, and a turned one.
    let backs = [Vec3::ONE.normalize(), Vec3::new(0.8, 1.0, 0.6).normalize()];
    let corners = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.25, 0.6), Vec2::new(0.8, 0.1)];
    let mut inputs = Vec::new();
    let mut expected = Vec::new();
    for back in backs {
        for corner in corners {
            // The billboard's layout: local = mix(min, max, (x, 1 - y)), upright through the anchor across the view.
            let local = CARD_MIN + (CARD_MAX - CARD_MIN) * Vec2::new(corner.x, 1.0 - corner.y);
            let on_card = anchor + right * local.x + Vec3::Y * local.y;
            for along in [-1.3, 0.0, 0.8] {
                inputs.push((on_card + back * along).extend(0.0).to_array());
                inputs.push(back.extend(0.0).to_array());
                expected.push(corner);
            }
        }
    }
    let actual = run_projected_shadow_compute(&format!(r#"
import package::shaders::world::art::projected_shadow::wall_shadow_card_corner;
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {{
    let corner = wall_shadow_card_corner(cases[index.x * 2u].xyz, vec3<f32>({ax:?}, {ay:?}, {az:?}),
        cases[index.x * 2u + 1u].xyz, vec2<f32>({min_x:?}, {min_y:?}), vec2<f32>({max_x:?}, {max_y:?}));
    result[index.x] = vec4<f32>(corner, 0.0, 0.0);
}}
"#, ax = anchor.x, ay = anchor.y, az = anchor.z, min_x = CARD_MIN.x, min_y = CARD_MIN.y, max_x = CARD_MAX.x,
        max_y = CARD_MAX.y), &inputs, expected.len(), expected.len());
    for (case, (corner, want)) in actual.iter().zip(&expected).enumerate() {
        let corner = Vec2::new(corner[0], corner[1]);
        assert!((corner - *want).abs().max_element() < TOLERANCE, "case {case}: {corner}, expected {want}");
    }
}

/// Runs `invocations` invocations of `main` of the WESL compute shader `source`, which imports
/// art/projected_shadow.wesl, over `inputs` (binding 0) and reads `results` vec4s back (binding 1).
fn run_projected_shadow_compute(source: &str, inputs: &[[f32; 4]], invocations: usize, results: usize)
    -> Vec<[f32; 4]> {
    run_wesl_compute(&[
        (PROJECTED_SHADOW_WESL, "shaders/world/art/projected_shadow.wesl"),
        (include_str!("../../../assets/shaders/world/art/art_bindings.wesl"), "shaders/world/art/art_bindings.wesl"),
        (include_str!("../../../assets/shaders/world/common_bindings.wesl"), "shaders/world/common_bindings.wesl"),
        (include_str!("../../../assets/shaders/world/uo_coords.wesl"), "shaders/world/uo_coords.wesl"),
    ], source, inputs, invocations, results)
}
