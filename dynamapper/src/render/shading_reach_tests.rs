//! The shading controls reach the picture: at the end of its slider each art control and the KR Color Wash change the
//! colour plainly (art/shading.wesl, postprocess/kr_color_wash.wesl), while the presets' values keep their old look.
use crate::util_lib::wesl_compute::run_wesl_compute;
use crate::util_lib::wesl_source::wesl_f32_const;

const NOISE_WESL: &str = include_str!("../../assets/shaders/world/land/noise.wesl");
const ART_SHADING_WESL: &str = include_str!("../../assets/shaders/world/art/shading.wesl");
const KR_COLOR_WASH_WESL: &str = include_str!("../../assets/shaders/postprocess/kr_color_wash.wesl");
/// A change of a fifth of a colour: plainly visible on any art.
const MIN_VISIBLE_CHANGE: f32 = 0.2;
const TOLERANCE: f32 = 1e-5;
/// The grey the art cases shade.
const ART_GREY: f32 = 0.5;
/// An upright sprite's foot, middle and top, across its middle (uv_in_tile).
const FOOT: [f32; 2] = [0.5, 0.97];
const MIDDLE: [f32; 2] = [0.5, 0.5];
// Above the foot band of Contact Shadows (ART_CONTACT_FOOT_START, 0.4 of the sprite's height).
const UPPER_MIDDLE: [f32; 2] = [0.5, 0.35];
const TOP: [f32; 2] = [0.5, 0.03];
/// StaticDepthClass::Regular, the walls and furniture.
const REGULAR: f32 = 0.0;
/// Map positions the mottle and the wash are read at: a 16 by 16 grid one and a half tiles apart, around Britain.
const GRID_SIDE: usize = 16;
const GRID_STEP: f32 = 1.5;
const GRID_ORIGIN: [f32; 2] = [1480.0, 1600.0];

/// An art case: where on the sprite and the map, the four strengths (Shadow Shape, Contact Shadows, Mottle, Light
/// Catch), and whether the art is flat.
struct ArtCase {
    uv: [f32; 2],
    world_xz: [f32; 2],
    shadow: f32,
    contact: f32,
    mottle: f32,
    catch: f32,
    flat: bool,
}

impl Default for ArtCase {
    fn default() -> Self {
        Self { uv: MIDDLE, world_xz: GRID_ORIGIN, shadow: 0.0, contact: 0.0, mottle: 0.0, catch: 0.0, flat: false }
    }
}

/// The grey of each case after `apply_art_surface_shading` without Fake Normals (Classic's and Enhanced's path), with
/// a white light, no atmosphere tint and no local light.
fn shade_art(cases: &[ArtCase]) -> Vec<f32> {
    let inputs: Vec<[f32; 4]> = cases.iter().flat_map(|case| [
        [case.uv[0], case.uv[1], case.world_xz[0], case.world_xz[1]],
        [case.shadow, case.contact, case.mottle, case.catch],
        [if case.flat { 1.0 } else { 0.0 }, REGULAR, 0.0, 0.0],
    ]).collect();
    let source = format!(r#"
import package::shaders::world::art::shading::apply_art_surface_shading;
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {{
    let place = cases[index.x * 3u];
    let strength = cases[index.x * 3u + 1u];
    let kind = cases[index.x * 3u + 2u];
    let rgb = apply_art_surface_shading(vec3<f32>({ART_GREY:?}), place.xy, vec3<f32>(place.z, 0.0, place.w),
        u32(kind.y), vec3<f32>(-1.0, 2.5, -1.0), vec3<f32>(1.0), vec3<f32>(1.0), 0u, strength.x, strength.w, 0.0,
        strength.y, strength.z, 0.0, 0.0, 0.0, vec4<f32>(0.0), false, kind.x > 0.5);
    result[index.x] = vec4<f32>(rgb, 0.0);
}}
"#);
    let results = run_wesl_compute(&[
        (ART_SHADING_WESL, "shaders/world/art/shading.wesl"),
        (NOISE_WESL, "shaders/world/land/noise.wesl"),
    ], &source, &inputs, cases.len(), cases.len());
    results.iter().map(|rgb| {
        assert!((rgb[0] - rgb[1]).abs() < TOLERANCE && (rgb[1] - rgb[2]).abs() < TOLERANCE, "a white light keeps grey");
        rgb[0]
    }).collect()
}

fn grid() -> Vec<[f32; 2]> {
    (0..GRID_SIDE * GRID_SIDE).map(|index| [GRID_ORIGIN[0] + (index % GRID_SIDE) as f32 * GRID_STEP,
        GRID_ORIGIN[1] + (index / GRID_SIDE) as f32 * GRID_STEP]).collect()
}

/// Shadow Shape darkens an upright sprite's foot, Light Catch brightens its top, and Contact Shadows, without Fake
/// Normals too, darkens its lower part alone: each by a fifth of the colour at least at the slider's end. Contact Shadows
/// leaves flat art, which has no foot, alone.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for art shading changes"]
fn gpu_the_art_controls_change_classic_art_plainly() {
    let shaded = shade_art(&[
        ArtCase { uv: FOOT, ..Default::default() },
        ArtCase { uv: FOOT, shadow: 1.0, ..Default::default() },
        ArtCase { uv: TOP, ..Default::default() },
        ArtCase { uv: TOP, catch: 1.0, ..Default::default() },
        ArtCase { uv: FOOT, contact: 1.0, ..Default::default() },
        ArtCase { uv: UPPER_MIDDLE, contact: 1.0, ..Default::default() },
        ArtCase { uv: FOOT, contact: 1.0, flat: true, ..Default::default() },
        ArtCase { uv: FOOT, flat: true, ..Default::default() },
    ]);
    assert!((shaded[0] - ART_GREY).abs() < TOLERANCE, "every strength at 0 leaves the art as it is: {}", shaded[0]);
    assert!(shaded[1] < ART_GREY * (1.0 - MIN_VISIBLE_CHANGE), "Shadow Shape at its foot: {}", shaded[1]);
    assert!(shaded[3] > ART_GREY * (1.0 + MIN_VISIBLE_CHANGE), "Light Catch at its top: {}", shaded[3]);
    assert!(shaded[4] < ART_GREY * (1.0 - MIN_VISIBLE_CHANGE), "Contact Shadows at its foot: {}", shaded[4]);
    assert!((shaded[5] - ART_GREY).abs() < TOLERANCE, "Contact Shadows leaves the upper part: {}", shaded[5]);
    assert!((shaded[6] - shaded[7]).abs() < TOLERANCE, "Contact Shadows leaves flat art: {} {}", shaded[6], shaded[7]);
}

/// Shadow Shape and Light Catch without Fake Normals take their strength up to the knee, above the Classic presets'
/// values (`classic_art_strengths_lie_below_the_plain_knee`), so the presets look as before, and rise to their most at
/// the slider's end.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for art shading changes"]
fn gpu_shadow_shape_and_light_catch_keep_the_classic_presets() {
    let knee = wesl_f32_const(ART_SHADING_WESL, "ART_PLAIN_KNEE");
    let maxima = [wesl_f32_const(ART_SHADING_WESL, "ART_PLAIN_SHADOW_MAX_AMOUNT"),
        wesl_f32_const(ART_SHADING_WESL, "ART_PLAIN_CATCH_MAX_AMOUNT")];
    let strengths = [0.0, 0.08, 0.12, knee, 0.5, 0.75, 1.0];
    let inputs: Vec<[f32; 4]> = maxima.iter()
        .flat_map(|max_amount| strengths.iter().map(move |strength| [*strength, *max_amount, 0.0, 0.0])).collect();
    let amounts = run_wesl_compute(&[
        (ART_SHADING_WESL, "shaders/world/art/shading.wesl"),
        (NOISE_WESL, "shaders/world/land/noise.wesl"),
    ], r#"
import package::shaders::world::art::shading::art_plain_amount;
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    result[index.x] = vec4<f32>(art_plain_amount(cases[index.x].x, cases[index.x].y), 0.0, 0.0, 0.0);
}
"#, &inputs, inputs.len(), inputs.len());
    for (input, amount) in inputs.iter().zip(&amounts) {
        let [strength, max_amount, ..] = *input;
        if strength <= knee {
            assert!((amount[0] - strength).abs() < TOLERANCE, "{strength}: {}", amount[0]);
        }
        if strength == 1.0 {
            assert!((amount[0] - max_amount).abs() < TOLERANCE, "{strength}: {}", amount[0]);
        }
    }
    assert!(amounts.windows(2).zip(inputs.windows(2)).filter(|(_, input)| input[0][1] == input[1][1])
        .all(|(pair, _)| pair[1][0] > pair[0][0]), "the amount rises with the strength");
}

/// Mottle breaks a wall's colour into patches that differ by a fifth at least at the slider's end, and does nothing
/// at 0, the presets' value.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for art shading changes"]
fn gpu_mottle_breaks_up_the_art_plainly() {
    let mottle_max = 1.5;
    for (mottle, visible) in [(0.0, false), (mottle_max, true)] {
        let shaded = shade_art(&grid().into_iter().map(|world_xz| ArtCase { world_xz, mottle, ..Default::default() })
            .collect::<Vec<_>>());
        let (low, high) = shaded.iter().fold((f32::MAX, f32::MIN), |(low, high), grey| (low.min(*grey), high.max(*grey)));
        if visible {
            assert!(high - low > ART_GREY * MIN_VISIBLE_CHANGE, "mottle {mottle}: {low}..{high}");
        } else {
            assert!(high - low < TOLERANCE, "mottle {mottle}: {low}..{high}");
        }
    }
}

/// The wash's amount is the strength up to the knee, so the presets (at most 0.74) look as before, and rises to the
/// most at the slider's end, where its patches differ by a fifth of the colour at least.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for KR Color Wash changes"]
fn gpu_the_kr_color_wash_keeps_the_presets_and_shows_past_the_knee() {
    let knee = wesl_f32_const(KR_COLOR_WASH_WESL, "KR_COLOR_WASH_KNEE");
    let strength_max = wesl_f32_const(KR_COLOR_WASH_WESL, "KR_COLOR_WASH_STRENGTH_MAX");
    let amount_max = wesl_f32_const(KR_COLOR_WASH_WESL, "KR_COLOR_WASH_MAX_AMOUNT");
    let min_scale = wesl_f32_const(KR_COLOR_WASH_WESL, "KR_COLOR_WASH_MIN_SCALE");
    let strengths = [0.0, 0.3, 0.55, knee, 1.0, 1.25, strength_max];
    let mut inputs: Vec<[f32; 4]> = strengths.iter().map(|strength| [*strength, 0.0, 0.0, 0.0]).collect();
    // The wash over the grid at the slider's end, its smallest patches, in the open (no shadow, no local light).
    inputs.extend(grid().into_iter().map(|[x, z]| [x, z, strength_max, min_scale]));
    let source = format!(r#"
import package::shaders::postprocess::kr_color_wash::{{kr_color_wash_amount, apply_kr_color_wash}};
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {{
    let wash_case = cases[index.x];
    if (index.x < {amounts}u) {{
        result[index.x] = vec4<f32>(kr_color_wash_amount(wash_case.x), 0.0, 0.0, 0.0);
    }} else {{
        let rgb = apply_kr_color_wash(vec3<f32>({ART_GREY:?}), wash_case.xy, 0.0, 0.0, 1u, wash_case.z, wash_case.w, 2u);
        result[index.x] = vec4<f32>(rgb, 0.0);
    }}
}}
"#, amounts = strengths.len());
    let results = run_wesl_compute(&[
        (KR_COLOR_WASH_WESL, "shaders/postprocess/kr_color_wash.wesl"),
        (NOISE_WESL, "shaders/world/land/noise.wesl"),
    ], &source, &inputs, inputs.len(), inputs.len());
    let (amounts, washed) = results.split_at(strengths.len());
    for (strength, amount) in strengths.iter().zip(amounts) {
        if *strength <= knee {
            assert!((amount[0] - strength).abs() < TOLERANCE, "up to the knee the amount is the strength: {amount:?}");
        }
    }
    assert!(amounts.windows(2).all(|pair| pair[1][0] > pair[0][0]), "the amount rises with the strength");
    assert!((amounts[strengths.len() - 1][0] - amount_max).abs() < TOLERANCE, "{:?}", amounts[strengths.len() - 1]);
    // The patches' value, the channels' mean, from the darkest to the brightest.
    let values: Vec<f32> = washed.iter().map(|rgb| (rgb[0] + rgb[1] + rgb[2]) / 3.0).collect();
    let (low, high) = values.iter().fold((f32::MAX, f32::MIN), |(low, high), value| (low.min(*value), high.max(*value)));
    assert!(high - low > ART_GREY * MIN_VISIBLE_CHANGE, "the wash's patches at the slider's end: {low}..{high}");
}

/// The headroom ceiling on land (land/shading.wesl) is 1 - reserve up to the reference sun and rises in proportion to
/// a stronger sun, so Sun Strength keeps brightening the land past it.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for land shading changes"]
fn gpu_the_headroom_ceiling_rises_with_the_sun() {
    let land_shading = include_str!("../../assets/shaders/world/land/shading.wesl");
    let reference_sun = wesl_f32_const(land_shading, "LAND_HEADROOM_REFERENCE_SUN");
    // UOD-Painterly's reserves: the night's, then the afternoon's, at its own sun and at the slider's ends.
    let cases = [[0.18, 0.62], [0.18, reference_sun], [0.37037, 0.0], [0.37037, 1.35], [0.37037, 2.0]];
    let inputs: Vec<[f32; 4]> = cases.iter().map(|[reserve, sun]| [*reserve, *sun, 0.0, 0.0]).collect();
    let ceilings = run_wesl_compute(&[
        (land_shading, "shaders/world/land/shading.wesl"),
        (include_str!("../../assets/shaders/world/land/land_bindings.wesl"), "shaders/world/land/land_bindings.wesl"),
        (include_str!("../../assets/shaders/world/land/lighting.wesl"), "shaders/world/land/lighting.wesl"),
        (include_str!("../../assets/shaders/world/common_bindings.wesl"), "shaders/world/common_bindings.wesl"),
        (NOISE_WESL, "shaders/world/land/noise.wesl"),
    ], r#"
import package::shaders::world::land::shading::land_headroom_ceiling;
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    result[index.x] = vec4<f32>(land_headroom_ceiling(cases[index.x].x, cases[index.x].y), 0.0, 0.0, 0.0);
}
"#, &inputs, inputs.len(), inputs.len());
    for ([reserve, sun], ceiling) in cases.iter().zip(&ceilings) {
        let expected = (1.0 - reserve) * (sun / reference_sun).max(1.0);
        assert!((ceiling[0] - expected).abs() < TOLERANCE, "reserve {reserve}, sun {sun}: {}", ceiling[0]);
    }
    assert!((ceilings[3][0] - 0.85).abs() < 1e-4, "UOD-Painterly's afternoon keeps its ceiling at its own sun");
    assert!(ceilings[4][0] > ceilings[3][0], "a stronger sun raises the ceiling");
}
