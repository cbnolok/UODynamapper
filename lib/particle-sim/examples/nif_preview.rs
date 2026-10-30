//! Simulates NIF effects of a client's `EffectTexture.uop` and writes one contact sheet PNG per effect: frames
//! side by side, drawn by a small software rasterizer (particles and textured unlit triangles with the effects'
//! blend modes, alpha tests, texture transforms and vertex colours).
//!
//! ```text
//! cargo run -p particle-sim --example nif_preview -- <name.nif | hash>... --out DIR
//!     [--client DIR] [--seconds S] [--frames K] [--frame-ms MS] [--loop] [--duration S] [--seed N] [--cell PX]
//! cargo run -p particle-sim --example nif_preview -- --list [--client DIR]
//! ```
//!
//! The client directory defaults to `UO_EC_CLIENT_DIR`. The run lasts `--seconds`, by default the effect's
//! animation length (at least [`MIN_DEFAULT_SECONDS`]); the effect runs once with that duration unless `--loop`.
//! The camera looks north (+y) from 30° above the horizon, orthographic, fitted to everything drawn over the run;
//! billboards face it. There is no depth buffer: draws follow the scene order.

use std::path::PathBuf;
use std::sync::Arc;

use gamebryo_nif::NifFile;
use image::{Rgba, RgbaImage};
use particle_sim::nif::desc::{CullMode, NifTextureSource, TextureAddressing, TextureApplyMode, TextureSlot};
use particle_sim::nif::{NifEffect, NifEffectInstance, compile};
use particle_sim::raster::{Canvas, OrthoView, RasterTexture, WHITE_TEXEL};
use particle_sim::render::{
    AlphaTest, BlendFactor, BlendState, QUAD_INDICES, QuadVertex, RenderFrame, SceneLighting, TRIANGLE_VERTICES,
    combine_textures, expand_mesh_draw, expand_mesh_stage_uvs, expand_sprite,
};
use particle_sim::{NodeTransform, ParticleSystemInstance, Vec3};
use uocf::enhanced::effect_texture::{EffectFileKind, EffectTexturePackage};

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
/// Shortest default run.
const MIN_DEFAULT_SECONDS: f32 = 2.0;
const DEFAULT_FRAMES: usize = 12;
const DEFAULT_FRAME_MS: u32 = 33;
const DEFAULT_SEED: u32 = 0x5EED;
const DEFAULT_CELL_PX: u32 = 256;
const SHEET_COLUMNS: usize = 4;
/// Gap between cells, pixels.
const CELL_GAP_PX: u32 = 2;
/// Fraction of a cell left empty around the fitted content.
const FIT_MARGIN: f32 = 0.08;
/// Smallest fitted extent, world units.
const MIN_FIT_EXTENT: f32 = 0.5;
/// Camera elevation above the horizon.
const CAMERA_ELEVATION_DEGREES: f32 = 30.0;
/// Camera distance from the fitted centre (orthographic: only billboards and axial sprites see it).
const CAMERA_DISTANCE: f32 = 1000.0;
const BACKGROUND: [f32; 4] = [0.10, 0.10, 0.12, 1.0];
const GAP_COLOR: Rgba<u8> = Rgba([40, 40, 48, 255]);
const DDS_MAGIC: &[u8] = b"DDS ";
const MS_PER_SECOND: f32 = 1000.0;
/// Blending off.
const OPAQUE_BLEND: BlendState = BlendState { src: BlendFactor::One, dst: BlendFactor::Zero };
/// Pixel centres are at half-pixel offsets.
const PIXEL_CENTER_OFFSET: f32 = 0.5;
const HALF: f32 = 0.5;
const ALPHA: usize = 3;

struct Options {
    targets: Vec<String>,
    out: Option<PathBuf>,
    client: Option<PathBuf>,
    seconds: Option<f32>,
    frames: usize,
    frame_ms: u32,
    looping: bool,
    duration: Option<f32>,
    seed: u32,
    cell: u32,
    list: bool,
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        targets: Vec::new(),
        out: None,
        client: std::env::var_os(EC_CLIENT_DIR_ENV).map(PathBuf::from),
        seconds: None,
        frames: DEFAULT_FRAMES,
        frame_ms: DEFAULT_FRAME_MS,
        looping: false,
        duration: None,
        seed: DEFAULT_SEED,
        cell: DEFAULT_CELL_PX,
        list: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        let parse_error = |name: &str| format!("bad value for {name}");
        match arg.as_str() {
            "--out" => options.out = Some(PathBuf::from(value("--out")?)),
            "--client" => options.client = Some(PathBuf::from(value("--client")?)),
            "--seconds" => options.seconds = Some(value("--seconds")?.parse().map_err(|_| parse_error("--seconds"))?),
            "--frames" => options.frames = value("--frames")?.parse().map_err(|_| parse_error("--frames"))?,
            "--frame-ms" => options.frame_ms = value("--frame-ms")?.parse().map_err(|_| parse_error("--frame-ms"))?,
            "--duration" => options.duration = Some(value("--duration")?.parse().map_err(|_| parse_error("--duration"))?),
            "--seed" => options.seed = value("--seed")?.parse().map_err(|_| parse_error("--seed"))?,
            "--cell" => options.cell = value("--cell")?.parse().map_err(|_| parse_error("--cell"))?,
            "--loop" => options.looping = true,
            "--list" => options.list = true,
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => options.targets.push(other.to_owned()),
        }
    }
    if options.frames == 0 || options.frame_ms == 0 || options.cell == 0 {
        return Err("--frames, --frame-ms and --cell must be positive".to_owned());
    }
    Ok(options)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = parse_options()?;
    let client = options.client.clone().ok_or("no client directory: pass --client or set UO_EC_CLIENT_DIR")?;
    let package = EffectTexturePackage::load_with_names(&client).map_err(|error| format!("{error:#}"))?;
    if options.list {
        let mut nifs: Vec<(Option<String>, u64)> = package
            .entries()
            .into_iter()
            .filter(|entry| entry.has_payload())
            .filter(|entry| {
                package
                    .read_by_hash(entry.hash)
                    .ok()
                    .flatten()
                    .is_some_and(|payload| EffectFileKind::detect(&payload) == EffectFileKind::Nif)
            })
            .map(|entry| (entry.file_name().map(str::to_owned), entry.hash))
            .collect();
        nifs.sort();
        for (name, hash) in nifs {
            println!("{hash:016x}  {}", name.as_deref().unwrap_or("?"));
        }
        return Ok(());
    }
    if options.targets.is_empty() {
        return Err("name the effects (see --list)".into());
    }
    let out_dir = options.out.clone().ok_or("pass --out DIR")?;
    std::fs::create_dir_all(&out_dir)?;
    for target in &options.targets {
        let payload = match package.read(target).map_err(|error| format!("{error:#}"))? {
            Some(payload) => payload,
            None => {
                let hash = u64::from_str_radix(target, 16).map_err(|_| format!("{target}: not in the package"))?;
                package.read_by_hash(hash).map_err(|error| format!("{error:#}"))?.ok_or("no such entry")?
            }
        };
        let nif = NifFile::read(&payload).map_err(|error| format!("{target}: {error}"))?;
        let output = compile(&nif, target);
        for warning in &output.warnings {
            println!("{target}: {warning}");
        }
        let desc = output.desc;
        describe(&desc);
        let textures = Textures::load(&package, &desc);
        let effect = Arc::new(NifEffect::prepare(desc));
        let captures = simulate(&effect, &options);
        let sheet = draw_sheet(&captures, &textures, options.cell);
        let stem = target.trim_end_matches(".nif");
        let path = out_dir.join(format!("{stem}.png"));
        sheet.save(&path)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn describe(desc: &particle_sim::NifEffectDesc) {
    println!(
        "{}: {} nodes, {} meshes ({} morphed), {} particle systems, {} controllers, animation {:.3} s",
        desc.name,
        desc.nodes.len(),
        desc.meshes.len(),
        desc.meshes.iter().filter(|mesh| mesh.morph.is_some()).count(),
        desc.particle_systems.len(),
        desc.controllers.len(),
        desc.animation_length()
    );
    for system in &desc.particle_systems {
        let modifiers: Vec<String> = system
            .modifiers
            .iter()
            .map(|modifier| {
                let kind = format!("{:?}", modifier.kind);
                kind.split(['(', ' ', '{']).next().unwrap_or_default().to_owned()
            })
            .collect();
        println!(
            "  system {:?}: capacity {}, world space {}, blend {:?}, modifiers {modifiers:?}",
            desc.nodes[system.node].name,
            system.capacity,
            system.world_space,
            system.render_state.alpha.blend_state()
        );
    }
    for texture in &desc.textures {
        println!("  texture {}", texture.name());
    }
}

/// Textures by render name.
struct Textures(Vec<(String, RasterTexture)>);

impl Textures {
    fn load(package: &EffectTexturePackage, desc: &particle_sim::NifEffectDesc) -> Self {
        let mut textures = Vec::new();
        for texture in &desc.textures {
            let name = texture.name();
            let decoded = match &texture.source {
                NifTextureSource::External { file_name } => match package.read(file_name).ok().flatten() {
                    Some(payload) => decode(&payload),
                    None => Err("not in the package".to_owned()),
                },
                NifTextureSource::Embedded { width, height, rgba } => {
                    RasterTexture::from_rgba8(*width as usize, *height as usize, rgba).ok_or_else(|| "bad size".to_owned())
                }
                NifTextureSource::Undecoded { format } => Err(format!("pixel format {format} not decoded")),
            };
            match decoded {
                Ok(raster) => textures.push((name, raster)),
                Err(error) => println!("  texture {name}: {error}"),
            }
        }
        Self(textures)
    }

    fn get(&self, name: &str) -> Option<&RasterTexture> {
        self.0.iter().find(|(texture, _)| texture.eq_ignore_ascii_case(name)).map(|(_, texture)| texture)
    }
}

fn decode(payload: &[u8]) -> Result<RasterTexture, String> {
    let rgba = if payload.starts_with(DDS_MAGIC) {
        let mut decoder = dds::Decoder::new(std::io::Cursor::new(payload)).map_err(|error| error.to_string())?;
        let size = decoder.main_size();
        let length = dds::ColorFormat::RGBA_U8.buffer_size(size).ok_or("DDS too large")?;
        let mut rgba = vec![0u8; length];
        let view = dds::ImageViewMut::new(&mut rgba, size, dds::ColorFormat::RGBA_U8).ok_or("DDS buffer")?;
        decoder.read_surface(view).map_err(|error| error.to_string())?;
        RgbaImage::from_raw(size.width, size.height, rgba).ok_or("DDS size")?
    } else {
        image::load_from_memory_with_format(payload, image::ImageFormat::Tga)
            .map_err(|error| error.to_string())?
            .into_rgba8()
    };
    RasterTexture::from_rgba8(rgba.width() as usize, rgba.height() as usize, rgba.as_raw()).ok_or_else(|| "empty texture".to_owned())
}

struct Capture {
    time: f32,
    frame: RenderFrame,
    particles: usize,
}

fn simulate(effect: &Arc<NifEffect>, options: &Options) -> Vec<Capture> {
    let length = effect.desc().animation_length().max(MIN_DEFAULT_SECONDS);
    let seconds = options.seconds.unwrap_or(length);
    let duration = options.duration.unwrap_or(length);
    let elevation = CAMERA_ELEVATION_DEGREES.to_radians();
    let view = OrthoView::new(elevation, Vec3::ZERO, CAMERA_DISTANCE, 1.0, 1, 1);
    let mut instance = NifEffectInstance::with_seed(Arc::clone(effect), options.seed);
    instance.set_camera(view.camera);
    instance.start(duration, options.looping);
    let dt = options.frame_ms as f32 / MS_PER_SECOND;
    let updates = (seconds / dt).ceil().max(1.0) as usize;
    let mut captures = Vec::new();
    let mut next_capture = 0;
    for update in 1..=updates {
        instance.update(options.frame_ms, &NodeTransform::default());
        // Capture times are spread evenly over the run, the last one at its end.
        let capture_update = (next_capture + 1) * updates / options.frames;
        if update >= capture_update && next_capture < options.frames {
            let mut frame = RenderFrame::default();
            instance.render(&mut frame);
            captures.push(Capture { time: update as f32 * dt, frame, particles: instance.particle_count() });
            next_capture += 1;
        }
    }
    for capture in &captures {
        println!(
            "  t = {:.3} s: {} particles, {} sprites, {} mesh draws",
            capture.time,
            capture.particles,
            capture.frame.sprites.len(),
            capture.frame.mesh_draws.len()
        );
    }
    captures
}

/// One texture stage of a draw: the texture name and the stage's texture coordinates per vertex.
struct Stage {
    slot: TextureSlot,
    texture: Arc<str>,
    uvs: Vec<[f32; 2]>,
    addressing: TextureAddressing,
}

/// The triangles of one draw (a mesh draw or a sprite batch) and their render state.
struct Triangles {
    vertices: Vec<QuadVertex>,
    stages: Vec<Stage>,
    apply_mode: TextureApplyMode,
    blend: BlendState,
    alpha_test: Option<AlphaTest>,
    cull: CullMode,
}

/// The draws of a frame as triangle lists, in scene order.
fn triangles(frame: &RenderFrame, view: &OrthoView) -> Vec<(u32, Triangles)> {
    let lighting = SceneLighting::default();
    let mut out = Vec::new();
    for draw in &frame.mesh_draws {
        let mut vertices = Vec::new();
        expand_mesh_draw(draw, &lighting, &mut vertices);
        let stages = draw
            .textures
            .iter()
            .map(|stage| {
                let mut uvs = Vec::with_capacity(vertices.len());
                expand_mesh_stage_uvs(draw, stage, &mut uvs);
                Stage { slot: stage.slot, texture: stage.texture.clone(), uvs, addressing: stage.addressing }
            })
            .collect();
        out.push((
            draw.order,
            Triangles {
                vertices,
                stages,
                apply_mode: draw.apply_mode,
                blend: draw.blend.unwrap_or(OPAQUE_BLEND),
                alpha_test: draw.alpha_test,
                cull: draw.cull,
            },
        ));
    }
    for (index, batch) in frame.batches.iter().enumerate() {
        let state = frame.batch_state(index).cloned().unwrap_or_default();
        let mut vertices = Vec::new();
        for sprite in &frame.sprites[batch.sprites.clone()] {
            let quad = expand_sprite(batch.kind, sprite, &view.camera);
            vertices.extend(QUAD_INDICES.iter().map(|&corner| quad[corner as usize]));
        }
        let stages = state
            .textures
            .iter()
            .map(|stage| Stage {
                slot: stage.slot,
                texture: stage.texture.clone(),
                uvs: vertices.iter().map(|vertex| stage.uv_transform.apply(vertex.uv)).collect(),
                addressing: stage.addressing,
            })
            .collect();
        out.push((
            state.order,
            Triangles {
                vertices,
                stages,
                apply_mode: state.apply_mode,
                blend: batch.blend,
                alpha_test: state.alpha_test,
                cull: CullMode::None,
            },
        ));
    }
    out.sort_by_key(|(order, _)| *order);
    out
}

/// Fits everything drawn over the run into a cell.
fn fit_view(captures: &[Capture], cell: u32) -> OrthoView {
    let elevation = CAMERA_ELEVATION_DEGREES.to_radians();
    let probe = OrthoView::new(elevation, Vec3::ZERO, CAMERA_DISTANCE, 1.0, cell as usize, cell as usize);
    let mut extent: Option<[f32; 4]> = None;
    let mut include = |point: Vec3| {
        let (x, y) = probe.project(point);
        let bounds = extent.get_or_insert([x, x, y, y]);
        *bounds = [bounds[0].min(x), bounds[1].max(x), bounds[2].min(y), bounds[3].max(y)];
    };
    include(Vec3::ZERO);
    for capture in captures {
        for (_, triangles) in triangles(&capture.frame, &probe) {
            for vertex in &triangles.vertices {
                include(vertex.position);
            }
        }
    }
    let [min_x, max_x, min_y, max_y] = extent.unwrap_or_default();
    let extent = (max_x - min_x).max(max_y - min_y).max(MIN_FIT_EXTENT);
    let center = probe.camera.right * ((min_x + max_x) * HALF) + probe.camera.up * ((min_y + max_y) * HALF);
    let cell = cell as f32;
    OrthoView::new(elevation, center, CAMERA_DISTANCE, cell * (1.0 - 2.0 * FIT_MARGIN) / extent, cell as usize, cell as usize)
}

fn draw_sheet(captures: &[Capture], textures: &Textures, cell: u32) -> RgbaImage {
    let view = fit_view(captures, cell);
    let columns = SHEET_COLUMNS.min(captures.len()).max(1);
    let rows = captures.len().div_ceil(columns).max(1);
    let width = columns as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
    let height = rows as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
    let mut sheet = RgbaImage::from_pixel(width, height, GAP_COLOR);
    for (index, capture) in captures.iter().enumerate() {
        let mut canvas = Canvas::new(view.width, view.height, BACKGROUND);
        for (_, triangles) in triangles(&capture.frame, &view) {
            let samplers: Vec<Option<&RasterTexture>> =
                triangles.stages.iter().map(|stage| textures.get(&stage.texture)).collect();
            for first in (0..triangles.vertices.len()).step_by(TRIANGLE_VERTICES) {
                fill_triangle(&mut canvas, &view, first, &triangles, &samplers);
            }
        }
        let image = RgbaImage::from_raw(view.width as u32, view.height as u32, canvas.to_rgba8()).expect("canvas size");
        let x = (index % columns) as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
        let y = (index / columns) as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
        for (px, py, pixel) in image.enumerate_pixels() {
            sheet.put_pixel(x + px, y + py, *pixel);
        }
    }
    sheet
}

/// One triangle (vertices `first..first + 3` of `state`) with per-vertex colours: every texture stage sampled
/// (wrapped or clamped) and combined, alpha test, blend. Front faces are counter-clockwise on the view plane;
/// a missing texture samples white.
fn fill_triangle(
    canvas: &mut Canvas,
    view: &OrthoView,
    first: usize,
    state: &Triangles,
    samplers: &[Option<&RasterTexture>],
) {
    let corners = &state.vertices[first..first + TRIANGLE_VERTICES];
    let [(ax, ay), (bx, by), (cx, cy)] = [0, 1, 2].map(|k| view.to_screen(corners[k].position));
    // Screen y points down: a counter-clockwise view-plane triangle has a negative screen area.
    let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
    if area.abs() < f32::EPSILON || !area.is_finite() {
        return;
    }
    let front = area < 0.0;
    match state.cull {
        CullMode::Back if !front => return,
        CullMode::Front if front => return,
        _ => {}
    }
    let (limit_x, limit_y) = (canvas.width as f32 - 1.0, canvas.height as f32 - 1.0);
    let min_x = ax.min(bx).min(cx).floor().clamp(0.0, limit_x) as usize;
    let max_x = ax.max(bx).max(cx).ceil().clamp(0.0, limit_x) as usize;
    let min_y = ay.min(by).min(cy).floor().clamp(0.0, limit_y) as usize;
    let max_y = ay.max(by).max(cy).ceil().clamp(0.0, limit_y) as usize;
    let address = |value: f32, wrap: bool| if wrap { value } else { value.clamp(0.0, 1.0) };
    let mut samples = Vec::with_capacity(state.stages.len());
    for py in min_y..=max_y {
        for px in min_x..=max_x {
            let (x, y) = (px as f32 + PIXEL_CENTER_OFFSET, py as f32 + PIXEL_CENTER_OFFSET);
            let wa = ((bx - x) * (cy - y) - (by - y) * (cx - x)) / area;
            let wb = ((cx - x) * (ay - y) - (cy - y) * (ax - x)) / area;
            let wc = 1.0 - wa - wb;
            if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                continue;
            }
            let weights = [wa, wb, wc];
            let lerp = |value: &dyn Fn(usize) -> f32| (0..TRIANGLE_VERTICES).map(|k| weights[k] * value(k)).sum::<f32>();
            let color: [f32; 4] = std::array::from_fn(|channel| lerp(&|k| corners[k].color[channel]));
            samples.clear();
            for (stage, sampler) in state.stages.iter().zip(samplers) {
                let uvs = &stage.uvs[first..first + TRIANGLE_VERTICES];
                let u = address(lerp(&|k| uvs[k][0]), stage.addressing.wrap_s);
                let v = address(lerp(&|k| uvs[k][1]), stage.addressing.wrap_t);
                samples.push((stage.slot, sampler.map_or(WHITE_TEXEL, |texture| texture.sample(u, v))));
            }
            let source = combine_textures(state.apply_mode, color, &samples);
            if let Some(test) = state.alpha_test {
                if !test.function.passes(source[ALPHA], test.reference) {
                    continue;
                }
            }
            canvas.blend(px, py, source, state.blend);
        }
    }
}
