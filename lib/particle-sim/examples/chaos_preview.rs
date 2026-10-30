//! Simulates one Chaos effect of a client's `EffectTexture.uop` and writes a contact sheet PNG: frames of the
//! effect side by side, drawn by a small software rasterizer with the effect's textures and blend modes.
//!
//! ```text
//! cargo run -p particle-sim --example chaos_preview -- <name.ems | hash | path.ems> --out DIR
//!     [--client DIR] [--seconds S] [--frames K] [--frame-ms MS] [--loop] [--duration S] [--move SPEED]
//!     [--seed N] [--cell PX]
//! cargo run -p particle-sim --example chaos_preview -- --list [--client DIR]
//! ```
//!
//! The client directory defaults to `UO_EC_CLIENT_DIR`. The camera looks north (+y) from 30° above the horizon,
//! orthographic, fitted to everything the effect draws over the run. MESH particles (no NIF renderer here) are
//! drawn as magenta square outlines of their size; the node is a grey dot.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{Rgba, RgbaImage};
use particle_sim::chaos::compile::compile_chaos_source;
use particle_sim::chaos::{ChaosIssueSeverity, ChaosSystem};
use particle_sim::render::{BlendFactor, BlendState, CameraFrame, QUAD_INDICES, QuadVertex, RenderFrame, expand_sprite};
use particle_sim::{ChaosEffectInstance, Mt19937, NodeTransform, ParticleSystemInstance, Vec3};
use uocf::enhanced::effect_texture::{EffectFileKind, EffectTexturePackage};

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const DEFAULT_SECONDS: f32 = 3.0;
const DEFAULT_FRAMES: usize = 12;
const DEFAULT_FRAME_MS: u32 = 33;
const DEFAULT_SEED: u32 = 0x5EED;
const DEFAULT_CELL_PX: u32 = 256;
const SHEET_COLUMNS: usize = 4;
/// Pixels between cells.
const CELL_GAP_PX: u32 = 2;
/// Fraction of the cell left free around the fitted bounds.
const FIT_MARGIN: f32 = 0.08;
/// Smallest world extent fitted into a cell (an effect with a single point).
const MIN_FIT_EXTENT: f32 = 4.0;
/// Camera elevation above the horizon.
const CAMERA_ELEVATION_DEGREES: f32 = 30.0;
/// Distance of the camera position from the fitted centre (for the axial view vector).
const CAMERA_DISTANCE: f32 = 1000.0;
const BACKGROUND: [f32; 4] = [0.10, 0.10, 0.12, 1.0];
const GAP_COLOR: Rgba<u8> = Rgba([40, 40, 48, 255]);
const MESH_COLOR: [f32; 4] = [1.0, 0.0, 1.0, 1.0];
const NODE_COLOR: [f32; 4] = [0.6, 0.6, 0.6, 1.0];
/// Half side of the node dot, pixels.
const NODE_DOT_HALF_PX: f32 = 1.5;
/// Smallest half side of a mesh marker, pixels.
const MESH_MARKER_MIN_HALF_PX: f32 = 2.0;
/// Texture used when a texture is missing or undecodable: opaque white.
const WHITE_TEXEL: [f32; 4] = [1.0; 4];
const DDS_MAGIC: &[u8] = b"DDS ";
/// Texel centres are at half-texel offsets (bilinear filtering).
const TEXEL_CENTER_OFFSET: f32 = 0.5;
const MS_PER_SECOND: f32 = 1000.0;
const U8_MAX: f32 = 255.0;

struct Options {
    target: Option<String>,
    out: Option<PathBuf>,
    client: Option<PathBuf>,
    seconds: f32,
    frames: usize,
    frame_ms: u32,
    looping: bool,
    duration: f32,
    move_speed: f32,
    seed: u32,
    cell: u32,
    list: bool,
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        target: None,
        out: None,
        client: std::env::var_os(EC_CLIENT_DIR_ENV).map(PathBuf::from),
        seconds: DEFAULT_SECONDS,
        frames: DEFAULT_FRAMES,
        frame_ms: DEFAULT_FRAME_MS,
        looping: false,
        duration: 0.0,
        move_speed: 0.0,
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
            "--seconds" => options.seconds = value("--seconds")?.parse().map_err(|_| parse_error("--seconds"))?,
            "--frames" => options.frames = value("--frames")?.parse().map_err(|_| parse_error("--frames"))?,
            "--frame-ms" => options.frame_ms = value("--frame-ms")?.parse().map_err(|_| parse_error("--frame-ms"))?,
            "--duration" => options.duration = value("--duration")?.parse().map_err(|_| parse_error("--duration"))?,
            "--move" => options.move_speed = value("--move")?.parse().map_err(|_| parse_error("--move"))?,
            "--seed" => options.seed = value("--seed")?.parse().map_err(|_| parse_error("--seed"))?,
            "--cell" => options.cell = value("--cell")?.parse().map_err(|_| parse_error("--cell"))?,
            "--loop" => options.looping = true,
            "--list" => options.list = true,
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => options.target = Some(other.to_owned()),
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
        for (name, hash) in chaos_scripts(&package) {
            println!("{hash:016x}  {}", name.as_deref().unwrap_or("?"));
        }
        return Ok(());
    }
    let target = options.target.clone().ok_or("name the effect (see --list)")?;
    let out_dir = options.out.clone().ok_or("pass --out DIR")?;
    let (label, script) = load_script(&package, &target)?;

    let report = compile_chaos_source(&script);
    for issue in &report.issues {
        if issue.severity() != ChaosIssueSeverity::Note {
            println!("{:?}: {issue}", issue.severity());
        }
    }
    let desc = report.desc.ok_or("the client rejects this script")?;
    println!("{} ({} sequences, minEmitTime {:?})", desc.name, desc.sequences.len(), desc.min_emit_time);
    for sequence in &desc.sequences {
        println!(
            "  {:<24} {:?} {:<28} blend {:?}/{:?} rate {} burst {:?} life {:?} max {} flags {:#x}",
            sequence.name,
            sequence.particle_type,
            sequence.resource,
            sequence.src_blend,
            sequence.dst_blend,
            sequence.emit_rate.max,
            sequence.burst,
            sequence.lifetime,
            sequence.num_particles,
            sequence.flags.0
        );
    }
    let textures = Textures::load(&package, &desc.resources());
    let mut rng = Mt19937::new(options.seed);
    let system = Arc::new(ChaosSystem::prepare(desc, &mut rng));
    let frames = simulate(&system, &options);
    let sheet = draw_sheet(&frames, &textures, options.cell);
    std::fs::create_dir_all(&out_dir)?;
    let path = out_dir.join(format!("{}.png", label.trim_end_matches(".ems")));
    sheet.save(&path)?;
    println!("wrote {}", path.display());
    Ok(())
}

/// Chaos scripts of the package: (name, hash).
fn chaos_scripts(package: &EffectTexturePackage) -> Vec<(Option<String>, u64)> {
    let mut scripts: Vec<(Option<String>, u64)> = package
        .entries()
        .into_iter()
        .filter(|entry| entry.has_payload())
        .filter(|entry| {
            package
                .read_by_hash(entry.hash)
                .ok()
                .flatten()
                .is_some_and(|payload| EffectFileKind::detect(&payload) == EffectFileKind::ChaosScript)
        })
        .map(|entry| (entry.file_name().map(str::to_owned), entry.hash))
        .collect();
    scripts.sort();
    scripts
}

/// The script named by `target`: a file on disk, a package file name or a hexadecimal name hash.
fn load_script(package: &EffectTexturePackage, target: &str) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    let path = Path::new(target);
    if path.is_file() {
        let label = path.file_name().map_or_else(|| target.to_owned(), |name| name.to_string_lossy().into_owned());
        return Ok((label, std::fs::read(path)?));
    }
    if let Some(payload) = package.read(target).map_err(|error| format!("{error:#}"))? {
        return Ok((target.to_owned(), payload));
    }
    let hash = u64::from_str_radix(target, 16).map_err(|_| format!("{target}: not in the package"))?;
    let payload = package.read_by_hash(hash).map_err(|error| format!("{error:#}"))?.ok_or("no such entry")?;
    Ok((target.to_owned(), payload))
}

/// A captured frame: what the effect drew and where the node was.
struct Capture {
    time: f32,
    frame: RenderFrame,
    node: Vec3,
    particles: usize,
}

fn simulate(system: &Arc<ChaosSystem>, options: &Options) -> Vec<Capture> {
    let mut effect = ChaosEffectInstance::with_seed(Arc::clone(system), options.seed);
    effect.start(options.duration, options.looping);
    let dt = options.frame_ms as f32 / MS_PER_SECOND;
    let updates = (options.seconds / dt).ceil().max(1.0) as usize;
    let mut captures = Vec::new();
    let mut next_capture = 0;
    for update in 1..=updates {
        let time = update as f32 * dt;
        let node = Vec3::new(time * options.move_speed, 0.0, 0.0);
        effect.update(options.frame_ms, &NodeTransform::at(node));
        // Capture times are spread evenly over the run, the last one at its end.
        let capture_update = (next_capture + 1) * updates / options.frames;
        if update >= capture_update && next_capture < options.frames {
            let mut frame = RenderFrame::default();
            effect.render(&mut frame);
            captures.push(Capture { time, frame, node, particles: effect.particle_count() });
            next_capture += 1;
        }
    }
    for capture in &captures {
        println!(
            "t = {:.3} s: {} particles, {} sprites, {} meshes",
            capture.time,
            capture.particles,
            capture.frame.sprites.len(),
            capture.frame.meshes.len()
        );
    }
    captures
}

struct Texture {
    width: usize,
    height: usize,
    texels: Vec<[f32; 4]>,
}

impl Texture {
    /// Bilinear sample with wrap addressing (Gamebryo's default, probable for the client).
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = u * self.width as f32 - TEXEL_CENTER_OFFSET;
        let y = v * self.height as f32 - TEXEL_CENTER_OFFSET;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let texel = |dx: f32, dy: f32| {
            let column = ((x0 + dx) as i64).rem_euclid(self.width as i64) as usize;
            let row = ((y0 + dy) as i64).rem_euclid(self.height as i64) as usize;
            self.texels[row * self.width + column]
        };
        let (a, b, c, d) = (texel(0.0, 0.0), texel(1.0, 0.0), texel(0.0, 1.0), texel(1.0, 1.0));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            top + (bottom - top) * fy
        })
    }
}

struct Textures(Vec<(String, Texture)>);

impl Textures {
    fn load(package: &EffectTexturePackage, names: &[&str]) -> Self {
        let mut textures = Vec::new();
        for name in names {
            if name.to_ascii_lowercase().ends_with(".nif") {
                continue;
            }
            match package.read(name).ok().flatten().map(|payload| decode(&payload)) {
                Some(Ok(texture)) => {
                    println!("texture {name}: {}x{}", texture.width, texture.height);
                    textures.push((name.to_string(), texture));
                }
                Some(Err(error)) => println!("texture {name}: {error}"),
                None => println!("texture {name}: not in the package"),
            }
        }
        Self(textures)
    }

    fn get(&self, name: &str) -> Option<&Texture> {
        self.0.iter().find(|(texture, _)| texture.eq_ignore_ascii_case(name)).map(|(_, texture)| texture)
    }
}

fn decode(payload: &[u8]) -> Result<Texture, String> {
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
    let texels = rgba.pixels().map(|pixel| pixel.0.map(|channel| f32::from(channel) / U8_MAX)).collect();
    Ok(Texture { width: rgba.width() as usize, height: rgba.height() as usize, texels })
}

/// The orthographic view: camera axes and the fit of world space into a cell.
struct View {
    camera: CameraFrame,
    center: Vec3,
    pixels_per_unit: f32,
    cell: f32,
}

impl View {
    fn to_screen(&self, position: Vec3) -> (f32, f32) {
        let offset = position - self.center;
        let half = self.cell * 0.5;
        (half + offset.dot(self.camera.right) * self.pixels_per_unit, half - offset.dot(self.camera.up) * self.pixels_per_unit)
    }
}

fn camera_axes() -> (Vec3, Vec3, Vec3) {
    let elevation = CAMERA_ELEVATION_DEGREES.to_radians();
    let direction = Vec3::new(0.0, elevation.cos(), -elevation.sin());
    let up = Vec3::new(0.0, elevation.sin(), elevation.cos());
    (direction, up, direction.cross(up))
}

/// Fits everything drawn over the run (sprite quads, meshes, the node) into a cell.
fn fit_view(captures: &[Capture], cell: u32) -> View {
    let (direction, up, right) = camera_axes();
    let mut points: Vec<Vec3> = Vec::new();
    for capture in captures {
        points.push(capture.node);
        points.extend(capture.frame.sprites.iter().map(|sprite| sprite.position));
        points.extend(capture.frame.meshes.iter().map(|mesh| mesh.translation));
    }
    let project = |point: &Vec3| (point.dot(right), point.dot(up));
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for point in &points {
        let (x, y) = project(point);
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    // Pad by the largest sprite so quads at the edge stay inside.
    let pad = captures
        .iter()
        .flat_map(|capture| capture.frame.sprites.iter().map(|sprite| sprite.size.abs() * 0.5 + sprite.length.abs()))
        .fold(0.0f32, f32::max);
    let extent = ((max_x - min_x).max(max_y - min_y) + 2.0 * pad).max(MIN_FIT_EXTENT);
    let center = right * ((min_x + max_x) * 0.5) + up * ((min_y + max_y) * 0.5);
    let camera = CameraFrame { position: center - direction * CAMERA_DISTANCE, direction, up, right };
    let cell = cell as f32;
    View { camera, center, pixels_per_unit: cell * (1.0 - 2.0 * FIT_MARGIN) / extent, cell }
}

fn draw_sheet(captures: &[Capture], textures: &Textures, cell: u32) -> RgbaImage {
    let view = fit_view(captures, cell);
    let columns = SHEET_COLUMNS.min(captures.len()).max(1);
    let rows = captures.len().div_ceil(columns).max(1);
    let width = columns as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
    let height = rows as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
    let mut sheet = RgbaImage::from_pixel(width, height, GAP_COLOR);
    for (index, capture) in captures.iter().enumerate() {
        let canvas = draw_frame(capture, textures, &view);
        let x = (index % columns) as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
        let y = (index / columns) as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
        for (px, py, pixel) in canvas.enumerate_pixels() {
            sheet.put_pixel(x + px, y + py, *pixel);
        }
    }
    sheet
}

/// A float RGBA canvas with D3D-style blending.
struct Canvas {
    size: usize,
    pixels: Vec<[f32; 4]>,
}

impl Canvas {
    fn blend(&mut self, x: usize, y: usize, source: [f32; 4], blend: BlendState) {
        let destination = &mut self.pixels[y * self.size + x];
        let source_factor = blend_factor(blend.src, source, *destination);
        let destination_factor = blend_factor(blend.dst, source, *destination);
        *destination = std::array::from_fn(|k| {
            (source[k] * source_factor[k] + destination[k] * destination_factor[k]).clamp(0.0, 1.0)
        });
    }
}

fn blend_factor(factor: BlendFactor, source: [f32; 4], destination: [f32; 4]) -> [f32; 4] {
    let alpha = |value: f32| [value; 4];
    let invert = |color: [f32; 4]| color.map(|channel| 1.0 - channel);
    const ALPHA: usize = 3;
    match factor {
        BlendFactor::One => alpha(1.0),
        BlendFactor::Zero => alpha(0.0),
        BlendFactor::SrcColor => source,
        BlendFactor::InvSrcColor => invert(source),
        BlendFactor::DestColor => destination,
        BlendFactor::InvDestColor => invert(destination),
        BlendFactor::SrcAlpha => alpha(source[ALPHA]),
        BlendFactor::InvSrcAlpha => alpha(1.0 - source[ALPHA]),
        BlendFactor::DestAlpha => alpha(destination[ALPHA]),
        BlendFactor::InvDestAlpha => alpha(1.0 - destination[ALPHA]),
        BlendFactor::SrcAlphaSat => {
            let f = source[ALPHA].min(1.0 - destination[ALPHA]);
            [f, f, f, 1.0]
        }
    }
}

fn draw_frame(capture: &Capture, textures: &Textures, view: &View) -> RgbaImage {
    let size = view.cell as usize;
    let mut canvas = Canvas { size, pixels: vec![BACKGROUND; size * size] };
    let opaque = BlendState { src: BlendFactor::SrcAlpha, dst: BlendFactor::InvSrcAlpha };
    let (nx, ny) = view.to_screen(capture.node);
    fill_square(&mut canvas, nx, ny, NODE_DOT_HALF_PX, NODE_COLOR, opaque);
    for batch in &capture.frame.batches {
        let texture = textures.get(&batch.texture);
        for sprite in &capture.frame.sprites[batch.sprites.clone()] {
            let vertices = expand_sprite(batch.kind, sprite, &view.camera);
            for triangle in QUAD_INDICES.chunks_exact(3) {
                let corners = [0, 1, 2].map(|k| vertices[triangle[k] as usize]);
                fill_triangle(&mut canvas, view, &corners, texture, batch.blend);
            }
        }
    }
    for mesh in &capture.frame.meshes {
        let (x, y) = view.to_screen(mesh.translation);
        let half = (mesh.scale.abs() * view.pixels_per_unit * 0.5).max(MESH_MARKER_MIN_HALF_PX);
        outline_square(&mut canvas, x, y, half, MESH_COLOR, opaque);
    }
    let pixels = canvas.pixels.iter().flat_map(|pixel| pixel.map(|channel| (channel * U8_MAX).round() as u8)).collect();
    RgbaImage::from_raw(size as u32, size as u32, pixels).expect("canvas size")
}

fn fill_square(canvas: &mut Canvas, x: f32, y: f32, half: f32, color: [f32; 4], blend: BlendState) {
    let limit = canvas.size as f32;
    let (x0, x1) = ((x - half).max(0.0) as usize, (x + half).min(limit) as usize);
    let (y0, y1) = ((y - half).max(0.0) as usize, (y + half).min(limit) as usize);
    for py in y0..y1 {
        for px in x0..x1 {
            canvas.blend(px, py, color, blend);
        }
    }
}

/// A one-pixel square outline.
fn outline_square(canvas: &mut Canvas, x: f32, y: f32, half: f32, color: [f32; 4], blend: BlendState) {
    let limit = canvas.size as i64 - 1;
    let clamp = |value: f32| (value.round() as i64).clamp(0, limit) as usize;
    let (x0, x1, y0, y1) = (clamp(x - half), clamp(x + half), clamp(y - half), clamp(y + half));
    for px in x0..=x1 {
        canvas.blend(px, y0, color, blend);
        canvas.blend(px, y1, color, blend);
    }
    for py in y0 + 1..y1 {
        canvas.blend(x0, py, color, blend);
        canvas.blend(x1, py, color, blend);
    }
}

/// Rasterizes one textured triangle (pixel centres, both windings), modulating the texture by the vertex
/// colour.
fn fill_triangle(canvas: &mut Canvas, view: &View, corners: &[QuadVertex; 3], texture: Option<&Texture>, blend: BlendState) {
    let screen = corners.map(|vertex| view.to_screen(vertex.position));
    let [(ax, ay), (bx, by), (cx, cy)] = screen;
    let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
    if area.abs() < f32::EPSILON {
        return;
    }
    let limit = canvas.size as f32 - 1.0;
    let min_x = ax.min(bx).min(cx).floor().clamp(0.0, limit) as usize;
    let max_x = ax.max(bx).max(cx).ceil().clamp(0.0, limit) as usize;
    let min_y = ay.min(by).min(cy).floor().clamp(0.0, limit) as usize;
    let max_y = ay.max(by).max(cy).ceil().clamp(0.0, limit) as usize;
    let color = corners[0].color;
    for py in min_y..=max_y {
        for px in min_x..=max_x {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            let wa = ((bx - x) * (cy - y) - (by - y) * (cx - x)) / area;
            let wb = ((cx - x) * (ay - y) - (cy - y) * (ax - x)) / area;
            let wc = 1.0 - wa - wb;
            if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                continue;
            }
            let u = wa * corners[0].uv[0] + wb * corners[1].uv[0] + wc * corners[2].uv[0];
            let v = wa * corners[0].uv[1] + wb * corners[1].uv[1] + wc * corners[2].uv[1];
            let texel = texture.map_or(WHITE_TEXEL, |texture| texture.sample(u, v));
            let source = std::array::from_fn(|k| texel[k] * color[k]);
            canvas.blend(px, py, source, blend);
        }
    }
}
