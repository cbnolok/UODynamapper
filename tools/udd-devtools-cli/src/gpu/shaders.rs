//! The test shaders: one sprite shader per layout and filter, generated the way `shader_defs` removes a path before compiling.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Path {
    /// `textureLoad` of the index plane, then one palette read. Nearest, integer scale only.
    IndexLoad,
    /// One gather per plane, four palette reads, a manual blend. What a filter costs on indices.
    IndexGather,
    /// The same, taking one read when the filter's fraction is zero.
    IndexGatherSkip,
    /// Row table and a pixel in a storage buffer, then one palette read.
    SpansLoad,
    /// Two rows and four pixels from the storage buffers, four palette reads, a manual blend.
    SpansFilter,
    /// One hardware sample.
    Bc1,
    Bc3,
    Bc7,
    Rgba,
}

impl Path {
    pub fn name(self, enhanced: bool) -> &'static str {
        match (self, enhanced) {
            (Path::IndexLoad, false) => "R8, load, 1 palette read",
            (Path::IndexLoad, true) => "RG8, load, 1 palette read",
            (Path::IndexGather, false) => "R8, 1 gather, 4 palette reads",
            (Path::IndexGather, true) => "RG8, 2 gathers, 4 palette reads",
            (Path::IndexGatherSkip, false) => "R8, gather, 1 read when the fraction is zero",
            (Path::IndexGatherSkip, true) => "RG8, gathers, 1 read when the fraction is zero",
            (Path::SpansLoad, _) => "row spans, 1 row, 1 pixel, 1 palette read",
            (Path::SpansFilter, _) => "row spans, 2 rows, 4 pixels, 4 palette reads",
            (Path::Bc1, _) => "BC1, 1 hardware sample",
            (Path::Bc3, _) => "BC3, 1 hardware sample",
            (Path::Bc7, _) => "BC7, 1 hardware sample",
            (Path::Rgba, _) => "RGBA8 expanded, 1 hardware sample",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hue {
    None,
    /// Through the hue texture: per tap on an indexed layout, once after the sample on a hardware one.
    PerPixel,
    /// A hardware layout: hue texture and an R8 mask sample.
    Masked,
    /// Through a palette row hued in advance: the shader of no hue, on another row.
    CachedRow,
}

impl Hue {
    pub fn name(self) -> &'static str {
        match self {
            Hue::None => "no hue",
            Hue::PerPixel => "hue texture",
            Hue::Masked => "hue texture and R8 mask",
            Hue::CachedRow => "cached hued palette row",
        }
    }
}

const COMMON: &str = r#"
struct Instance {
    pos: vec2<f32>,
    size: vec2<f32>,
    atlas_min: vec2<f32>,
    frame_size: vec2<f32>,
    palette: u32,
    hued_palette: u32,
    hue: u32,
    path: u32,
    rows: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
};
struct Globals { viewport: vec2<f32>, atlas: vec2<f32> };
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var smp: sampler;
@group(0) @binding(3) var index_atlas: texture_2d<f32>;
@group(0) @binding(4) var bc1_atlas: texture_2d<f32>;
@group(0) @binding(5) var bc7_atlas: texture_2d<f32>;
@group(0) @binding(6) var rgba_atlas: texture_2d<f32>;
@group(0) @binding(7) var palette: texture_2d<f32>;
@group(0) @binding(8) var hues: texture_2d<f32>;
@group(0) @binding(9) var mask_atlas: texture_2d<f32>;
@group(0) @binding(10) var<storage, read> span_rows: array<u32>;
@group(0) @binding(11) var<storage, read> span_pixels: array<u32>;
@group(0) @binding(12) var bc3_atlas: texture_2d<f32>;

struct V {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) instance: u32,
};

@vertex
fn vs(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> V {
    var corners = array<vec2<f32>, 6>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0));
    let c = corners[v];
    let inst = instances[i];
    let at = inst.pos + c * inst.size;
    var out: V;
    out.position = vec4<f32>(at.x / globals.viewport.x * 2.0 - 1.0, 1.0 - at.y / globals.viewport.y * 2.0, 0.0, 1.0);
    out.local = c * inst.frame_size;
    out.instance = i;
    return out;
}

// The seam filter of world/pixel_art_filters.wesl, in frame pixels, with the gradients taken before any branch.
fn iq(local: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec2<f32> {
    let seam = floor(local + vec2<f32>(0.5));
    let width = max(abs(dx) + abs(dy), vec2<f32>(0.000001));
    return seam + clamp((local - seam) / width, vec2<f32>(-0.5), vec2<f32>(0.5));
}

fn span_row(inst: Instance, y: i32) -> vec2<u32> {
    if (y < 0 || y >= i32(inst.frame_size.y)) { return vec2<u32>(0u); }
    let at = inst.rows + u32(y) * 2u;
    return vec2<u32>(span_rows[at], span_rows[at + 1u]);
}
"#;

const LAST_BYTE: f32 = 255.0;

fn rgb_of(hue: Hue) -> String {
    let row = if hue == Hue::CachedRow { "inst.hued_palette" } else { "inst.palette" };
    let read = format!("textureLoad(palette, vec2<i32>(i32(i), i32({row})), 0)");
    match hue {
        Hue::PerPixel => format!(
            r#"fn rgb_of(i: u32, inst: Instance) -> vec3<f32> {{
    let c = {read};
    if (c.a != 0.0) {{ return c.rgb; }}
    return textureLoad(hues, vec2<i32>(i32(c.r * {LAST_BYTE:.1} + 0.5), i32(inst.hue)), 0).rgb;
}}
"#
        ),
        _ => format!(
            r#"fn rgb_of(i: u32, inst: Instance) -> vec3<f32> {{
    return {read}.rgb;
}}
"#
        ),
    }
}

/// What a tap in a row of the spans holds: the index, and on Enhanced its coverage above it; zero outside the span.
fn span_tap(enhanced: bool) -> String {
    let (bytes, mask) = if enhanced { (2, "65535u") } else { (1, "255u") };
    format!(
        r#"fn span_tap(row: vec2<u32>, x: i32) -> u32 {{
    let at = x - i32(row.x & 65535u);
    if (at < 0 || at >= i32(row.x >> 16u)) {{ return 0u; }}
    let o = row.y + u32(at) * {bytes}u;
    return (span_pixels[o >> 2u] >> ((o & 3u) * 8u)) & {mask};
}}
"#
    )
}

/// The four taps' blend, with partial coverage counted: each colour weighs its filter weight times its own alpha.
const BLEND: &str = r#"
    let weights = vec4<f32>((1.0 - f.x) * f.y, f.x * f.y, f.x * (1.0 - f.y), (1.0 - f.x) * (1.0 - f.y));
    var colour = vec3<f32>(0.0);
    var alpha = 0.0;
    for (var n = 0; n < 4; n += 1) {
        let a = weights[n] * alphas[n];
        colour += rgb_of(indices[n], inst) * a;
        alpha += a;
    }
    return vec4<f32>(colour, alpha);
"#;

fn gather_setup(enhanced: bool) -> String {
    let alphas = if enhanced {
        format!("let alphas = textureGather(1, index_atlas, smp, corner) * {LAST_BYTE:.1} / 16.0;")
    } else {
        "let alphas = vec4<f32>(vec4<u32>(indices != vec4<u32>(0u)));".to_string()
    };
    format!(
        r#"
    let p = iq(local, dx, dy) - vec2<f32>(0.5);
    let base = floor(p);
    let f = p - base;
    let corner = (inst.atlas_min + base + vec2<f32>(1.0)) / globals.atlas;
    let indices = vec4<u32>(textureGather(0, index_atlas, smp, corner) * {LAST_BYTE:.1} + vec4<f32>(0.5));
    {alphas}
"#
    )
}

fn load_body(enhanced: bool) -> String {
    let texel = "let t = textureLoad(index_atlas, vec2<i32>(inst.atlas_min + floor(local)), 0);";
    if enhanced {
        format!(
            r#"
    {texel}
    let a = t.g * {LAST_BYTE:.1} / 16.0;
    if (a == 0.0) {{ return vec4<f32>(0.0); }}
    return vec4<f32>(rgb_of(u32(t.r * {LAST_BYTE:.1} + 0.5), inst) * a, a);
"#
        )
    } else {
        format!(
            r#"
    {texel}
    let i = u32(t.r * {LAST_BYTE:.1} + 0.5);
    if (i == 0u) {{ return vec4<f32>(0.0); }}
    return vec4<f32>(rgb_of(i, inst), 1.0);
"#
        )
    }
}

fn hardware_body(atlas: &str, hue: Hue) -> String {
    let sample = format!("let uv = (inst.atlas_min + iq(local, dx, dy)) / globals.atlas;\n    let c = textureSampleLevel({atlas}, smp, uv, 0.0);");
    let hued = format!("let hued = textureLoad(hues, vec2<i32>(i32(c.r * {LAST_BYTE:.1} + 0.5), i32(inst.hue)), 0).rgb;");
    match hue {
        Hue::PerPixel => format!("\n    {sample}\n    {hued}\n    return vec4<f32>(hued * c.a, c.a);\n"),
        Hue::Masked => format!("\n    {sample}\n    {hued}\n    let m = textureSampleLevel(mask_atlas, smp, uv, 0.0).r;\n    return vec4<f32>(mix(c.rgb, hued, m) * c.a, c.a);\n"),
        _ => format!("\n    {sample}\n    return vec4<f32>(c.rgb * c.a, c.a);\n"),
    }
}

fn spans_load_body(enhanced: bool) -> String {
    let finish = if enhanced {
        format!(
            r#"
    let a = f32(t >> 8u) / 16.0;
    if (a == 0.0) {{ return vec4<f32>(0.0); }}
    return vec4<f32>(rgb_of(t & 255u, inst) * a, a);
"#
        )
    } else {
        r#"
    if (t == 0u) { return vec4<f32>(0.0); }
    return vec4<f32>(rgb_of(t, inst), 1.0);
"#
        .to_string()
    };
    format!(
        r#"
    let at = vec2<i32>(floor(local));
    let t = span_tap(span_row(inst, at.y), at.x);
{finish}"#
    )
}

fn spans_filter_body(enhanced: bool) -> String {
    let split = if enhanced {
        r#"
    let packed = vec4<u32>(span_tap(lower, x + 0), span_tap(lower, x + 1), span_tap(upper, x + 1), span_tap(upper, x + 0));
    let indices = packed & vec4<u32>(255u);
    let alphas = vec4<f32>(packed >> vec4<u32>(8u)) / 16.0;
"#
    } else {
        r#"
    let indices = vec4<u32>(span_tap(lower, x + 0), span_tap(lower, x + 1), span_tap(upper, x + 1), span_tap(upper, x + 0));
    let alphas = vec4<f32>(vec4<u32>(indices != vec4<u32>(0u)));
"#
    };
    format!(
        r#"
    let p = iq(local, dx, dy) - vec2<f32>(0.5);
    let base = floor(p);
    let f = p - base;
    let x = i32(base.x);
    let upper = span_row(inst, i32(base.y));
    let lower = span_row(inst, i32(base.y) + 1);
{split}{BLEND}"#
    )
}

fn shade(name: &str, path: Path, hue: Hue, enhanced: bool) -> String {
    let body = match path {
        Path::IndexLoad => load_body(enhanced),
        Path::IndexGather => format!("{}{BLEND}", gather_setup(enhanced)),
        Path::IndexGatherSkip => format!(
            r#"
    if (all(fract(iq(local, dx, dy) - vec2<f32>(0.5)) == vec2<f32>(0.0))) {{
        {}
    }}
    {}{BLEND}"#,
            load_body(enhanced).replace("local", "iq(local, dx, dy)"),
            gather_setup(enhanced)
        ),
        Path::SpansLoad => spans_load_body(enhanced),
        Path::SpansFilter => spans_filter_body(enhanced),
        Path::Bc1 => hardware_body("bc1_atlas", hue),
        Path::Bc3 => hardware_body("bc3_atlas", hue),
        Path::Bc7 => hardware_body("bc7_atlas", hue),
        Path::Rgba => hardware_body("rgba_atlas", hue),
    };
    format!("fn {name}(local: vec2<f32>, inst: Instance, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {{{body}}}\n")
}

/// A shader with one path, or two behind a per-instance branch.
pub fn source(paths: &[Path], hue: Hue, enhanced: bool) -> String {
    let mut out = String::from(COMMON);
    out.push_str(&rgb_of(hue));
    out.push_str(&span_tap(enhanced));
    let names = ["shade_a", "shade_b"];
    for (path, name) in paths.iter().zip(names) {
        out.push_str(&shade(name, *path, hue, enhanced));
    }
    let call = if paths.len() == 1 {
        "let c = shade_a(in.local, inst, dx, dy);".to_string()
    } else {
        "var c: vec4<f32>;\n    if (inst.path == 0u) { c = shade_a(in.local, inst, dx, dy); } else { c = shade_b(in.local, inst, dx, dy); }".to_string()
    };
    out.push_str(&format!(
        r#"
@fragment
fn fs(in: V) -> @location(0) vec4<f32> {{
    let inst = instances[in.instance];
    let dx = dpdx(in.local);
    let dy = dpdy(in.local);
    {call}
    if (c.a < 0.0001) {{ discard; }}
    return c;
}}
"#
    ));
    out
}
