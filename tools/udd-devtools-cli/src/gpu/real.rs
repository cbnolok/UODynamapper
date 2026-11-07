//! The renderer's real art shader, compiled headless: its bindings and passes aren't run, only its register counts are wanted.
//! Bevy's modules are stubbed with what the shader reads of them; nothing else about it changes.

use std::borrow::Cow;
use std::path::PathBuf;

const REAL_MAIN: &str = "package::shaders::world::art::main";

const FORWARD_IO: &str = r#"
struct Vertex {
    @location(0) position: vec3<f32>,
    @location(3) uv_b: vec2<f32>,
};
"#;

const MESH_VIEW_BINDINGS: &str = r#"
struct View {
    clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    world_from_view: mat4x4<f32>,
    view_from_world: mat4x4<f32>,
    clip_from_view: mat4x4<f32>,
    view_from_clip: mat4x4<f32>,
    world_position: vec3<f32>,
    viewport: vec4<f32>,
};
struct Globals {
    time: f32,
    delta_time: f32,
    frame_count: u32,
};
@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> globals: Globals;
"#;

const VIEW_TRANSFORMATIONS: &str = r#"
import bevy_pbr::render::mesh_view_bindings::view;
fn position_world_to_clip(world_pos: vec3<f32>) -> vec4<f32> {
    return view.clip_from_world * vec4<f32>(world_pos, 1.0);
}
fn position_world_to_view(world_pos: vec3<f32>) -> vec3<f32> {
    return (view.view_from_world * vec4<f32>(world_pos, 1.0)).xyz;
}
fn position_view_to_clip(view_pos: vec3<f32>) -> vec4<f32> {
    return view.clip_from_view * vec4<f32>(view_pos, 1.0);
}
fn position_view_to_world(view_pos: vec3<f32>) -> vec3<f32> {
    return (view.world_from_view * vec4<f32>(view_pos, 1.0)).xyz;
}
fn direction_world_to_view(world_dir: vec3<f32>) -> vec3<f32> {
    return (view.view_from_world * vec4<f32>(world_dir, 0.0)).xyz;
}
"#;

struct Sources {
    root: PathBuf,
    rewrites: &'static [(&'static str, &'static str)],
}

/// A stand-in for features the shader doesn't have: the only `@if`s are `STATIC_PICK` and `STATIC_PICK_MSAA`, already off.
/// Each row fixes runtime values as literal text in a scratch copy, so the compiler removes what they no longer reach.
pub struct Variant {
    pub name: &'static str,
    pub rewrites: &'static [(&'static str, &'static str)],
}

const NO_DOT_MODE: (&str, &str) = ("sprite_params.render_mode", "0u");
const BILLBOARD_ONLY: [(&str, &str); 3] = [("inst.flat_quad", "0u"), ("in.is_wet", "0u"), ("inst.is_wet_flags", "0u")];
const NO_SURFACE_PROFILE: (&str, &str) = ("effects.post_process_profile", "0u");
const NO_WATER: (&str, &str) = ("effects.enable_water_animation", "0u");
const NO_GRUNGE: (&str, &str) = ("effects.enable_grunge", "0u");
const NO_GRADING: (&str, &str) = ("global_light.enable_grading", "0u");
const NO_TONEMAP: (&str, &str) = ("global_light.enable_tonemap", "0u");
const OPAQUE_PASS: (&str, &str) = ("sprite_params.pass_mode", "PASS_MODE_OPAQUE");
const TRANSPARENT_PASS: (&str, &str) = ("sprite_params.pass_mode", "PASS_MODE_TRANSPARENT");
const SHADOW_PASS: (&str, &str) = ("sprite_params.pass_mode", "PASS_MODE_SHADOW");

pub const VARIANTS: [Variant; 8] = [
    Variant { name: "as it is, every @if off", rewrites: &[] },
    Variant { name: "opaque pass only", rewrites: &[NO_DOT_MODE, OPAQUE_PASS] },
    Variant { name: "transparent pass only", rewrites: &[NO_DOT_MODE, TRANSPARENT_PASS] },
    Variant { name: "shadow pass only", rewrites: &[NO_DOT_MODE, SHADOW_PASS] },
    Variant { name: "billboards only (no flat quads, no wet), every pass", rewrites: &[NO_DOT_MODE, BILLBOARD_ONLY[0], BILLBOARD_ONLY[1], BILLBOARD_ONLY[2]] },
    Variant { name: "billboards, transparent pass", rewrites: &[NO_DOT_MODE, TRANSPARENT_PASS, BILLBOARD_ONLY[0], BILLBOARD_ONLY[1], BILLBOARD_ONLY[2]] },
    Variant { name: "billboards, transparent pass, no water, surface profile or grunge", rewrites: &[NO_DOT_MODE, TRANSPARENT_PASS, BILLBOARD_ONLY[0], BILLBOARD_ONLY[1], BILLBOARD_ONLY[2], NO_SURFACE_PROFILE, NO_WATER, NO_GRUNGE] },
    Variant { name: "the same, also no grading or tonemap", rewrites: &[NO_DOT_MODE, TRANSPARENT_PASS, BILLBOARD_ONLY[0], BILLBOARD_ONLY[1], BILLBOARD_ONLY[2], NO_SURFACE_PROFILE, NO_WATER, NO_GRUNGE, NO_GRADING, NO_TONEMAP] },
];

impl wesl::Resolver for Sources {
    fn resolve_source<'a>(&'a self, path: &wesl::syntax::ModulePath) -> Result<Cow<'a, str>, wesl::error::ResolveError> {
        let name = path.to_string();
        if name.starts_with("bevy_pbr") {
            let last = path.components.last().map(String::as_str).unwrap_or("");
            let stub = match last {
                "forward_io" => FORWARD_IO,
                "mesh_view_bindings" => MESH_VIEW_BINDINGS,
                "view_transformations" => VIEW_TRANSFORMATIONS,
                _ => "",
            };
            return Ok(Cow::Borrowed(stub));
        }
        let mut file = self.root.clone();
        for part in &path.components {
            file.push(part);
        }
        for extension in ["wesl", "wgsl"] {
            file.set_extension(extension);
            if let Ok(mut text) = std::fs::read_to_string(&file) {
                for (from, to) in self.rewrites {
                    text = text.replace(from, to);
                }
                return Ok(Cow::Owned(text));
            }
        }
        Err(wesl::error::ResolveError::ModuleNotFound(path.clone(), format!("no file at {}", file.display())))
    }
}

/// The real shader as WGSL, with every `@if` off, as a build that isn't the picking pass has it.
pub fn real_wgsl(assets: &std::path::Path, rewrites: &'static [(&'static str, &'static str)]) -> String {
    let compiler = wesl::Compiler::new_with_resolver(
        wesl::CompileOptions { visibility: false, mangle_main: false, keep_main: true, ..Default::default() },
        Sources { root: assets.to_path_buf(), rewrites },
    );
    let main: wesl::syntax::ModulePath = REAL_MAIN.parse().expect("a module path");
    match compiler.compile_module(&main) {
        Ok(result) => result.to_string(),
        Err(error) => panic!("the real shader: {error}"),
    }
}

pub fn pipeline(device: &wgpu::Device, target: wgpu::TextureFormat, variant: &Variant, assets: &std::path::Path) -> wgpu::RenderPipeline {
    let source = real_wgsl(assets, variant.rewrites);
    eprintln!("=== PIPELINE the renderer's art/main.wesl: {}", variant.name);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("art main"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    let attributes = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 12, shader_location: 3 },
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("art main"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 20, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes })],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: target, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::GreaterEqual), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}
