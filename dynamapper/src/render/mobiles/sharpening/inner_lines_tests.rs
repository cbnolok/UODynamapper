//! The interior lines (art/mobile_inner_edges.wesl `mobile_inner_line`) on synthetic line windows: a long edge darkens
//! the foot of its dark side, and neither the texel behind the foot, its light side, a flat colour, a fleck nor a window
//! that reaches the rim takes a line.
use crate::util_lib::wesl_compute::run_wesl_compute;

const INNER_EDGES_WESL: &str = include_str!("../../../../assets/shaders/world/art/mobile_inner_edges.wesl");
const MODULES: [(&str, &str); 7] = [
    (INNER_EDGES_WESL, "shaders/world/art/mobile_inner_edges.wesl"),
    (include_str!("../../../../assets/shaders/world/art/art_bindings.wesl"), "shaders/world/art/art_bindings.wesl"),
    (include_str!("../../../../assets/shaders/world/common_bindings.wesl"), "shaders/world/common_bindings.wesl"),
    (include_str!("../../../../assets/shaders/world/uo_coords.wesl"), "shaders/world/uo_coords.wesl"),
    (include_str!("../../../../assets/shaders/world/srgb.wesl"), "shaders/world/srgb.wesl"),
    (include_str!("../../../../assets/shaders/world/art/mobile_edges.wesl"), "shaders/world/art/mobile_edges.wesl"),
    (include_str!("../../../../assets/shaders/world/art/mobile_sharpen.wesl"), "shaders/world/art/mobile_sharpen.wesl"),
];
/// The line window's side and its reach from the centre (`MOBILE_INNER_LINE_SIDE`, `MOBILE_INNER_LINE_REACH`).
const SIDE: i32 = 5;
const REACH: i32 = 2;
const TAPS: usize = (SIDE * SIDE) as usize;
/// The column of the edge's foot, the last dark one west of the light side.
const FOOT: i32 = 0;
/// The frame's threshold, the Sharpening.kdl value the lines are tuned at.
const THRESHOLD: f32 = 0.1;
/// Two greys (sRGB-encoded) a plain form's step apart, well past the threshold.
const DARK: f32 = 0.2;
const LIGHT: f32 = 0.8;
const OPAQUE: f32 = 1.0;
const TOLERANCE: f32 = 1e-5;

/// The line's weight through the centre of each window, its taps (sRGB-encoded grey, alpha) row by row from the
/// north-west.
fn line_weights(windows: &[[[f32; 2]; TAPS]]) -> Vec<f32> {
    let inputs: Vec<[f32; 4]> = windows.iter().flatten().map(|&[grey, alpha]| [grey, grey, grey, alpha]).collect();
    let source = format!(r#"
import package::shaders::world::art::art_bindings::MobileFrameFilter;
import package::shaders::world::srgb::decode_srgb;
import package::shaders::world::art::mobile_inner_edges::{{mobile_inner_line, mobile_inner_line_start,
    mobile_inner_line_add, mobile_inner_line_offset, MOBILE_INNER_LINE_TAPS}};

@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;

@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {{
    var sum = mobile_inner_line_start();
    for (var tap = 0u; tap < MOBILE_INNER_LINE_TAPS; tap++) {{
        let encoded = cases[index.x * MOBILE_INNER_LINE_TAPS + tap];
        sum = mobile_inner_line_add(sum, mobile_inner_line_offset(tap), vec4<f32>(decode_srgb(encoded.rgb), encoded.a));
    }}
    let frame_filter = MobileFrameFilter(0.0, 0.0, 0.0, 0.0, 0.0, 0u, 0.0, 0u, 1.0, 0.0, {THRESHOLD:?}, 1u);
    result[index.x] = vec4<f32>(mobile_inner_line(sum, frame_filter), 0.0, 0.0, 0.0);
}}
"#);
    run_wesl_compute(&MODULES, &source, &inputs, windows.len(), windows.len()).iter().map(|weight| weight[0]).collect()
}

/// A window whose tap at `(x, y)` from the centre is `tap(x, y)`.
fn window(tap: impl Fn(i32, i32) -> [f32; 2]) -> [[f32; 2]; TAPS] {
    std::array::from_fn(|index| tap(index as i32 % SIDE - REACH, index as i32 / SIDE - REACH))
}

fn grey(value: f32) -> [f32; 2] {
    [value, OPAQUE]
}

#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for interior line changes"]
fn gpu_a_long_edge_lines_its_dark_side_alone() {
    // An edge running north to south between the centre's column and the next one east.
    let edge_at = |x: i32| grey(if x <= FOOT { DARK } else { LIGHT });
    let weights = line_weights(&[
        window(|x, _| edge_at(x)),
        // The texel behind the foot, deeper in the dark side: the same hard edge a column further east.
        window(|x, _| grey(if x <= FOOT + 1 { DARK } else { LIGHT })),
        // The first texel of the light side: the same edge a column further west.
        window(|x, _| grey(if x < FOOT { DARK } else { LIGHT })),
        window(|_, _| grey(LIGHT)),
        // A dark fleck at the centre, and one beside it.
        window(|x, y| grey(if x == 0 && y == 0 { DARK } else { LIGHT })),
        window(|x, y| grey(if x == 1 && y == 0 { DARK } else { LIGHT })),
        // The edge with a corner of the window on the transparent rim.
        window(|x, y| if x == REACH && y == REACH { [LIGHT, 0.0] } else { edge_at(x) }),
    ]);
    assert!((weights[0] - 1.0).abs() < TOLERANCE, "the edge's dark side takes a whole line: {}", weights[0]);
    for (index, what) in [(1, "the texel behind the foot"), (2, "the light side"), (3, "a flat colour"), (4, "a fleck"),
        (5, "a fleck beside the centre"), (6, "a window on the rim")] {
        assert_eq!(weights[index], 0.0, "{what} takes no line");
    }
}
