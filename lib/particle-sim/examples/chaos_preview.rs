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
use particle_sim::raster::{Canvas, OVERLAY_BLEND, OrthoView, RasterTexture, frame_extent};
use particle_sim::render::RenderFrame;
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
const DDS_MAGIC: &[u8] = b"DDS ";
const MS_PER_SECOND: f32 = 1000.0;

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

struct Textures(Vec<(String, RasterTexture)>);

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

/// Fits everything drawn over the run (sprite quads, meshes, the node) into a cell.
fn fit_view(captures: &[Capture], cell: u32) -> OrthoView {
    let elevation = CAMERA_ELEVATION_DEGREES.to_radians();
    let probe = OrthoView::new(elevation, Vec3::ZERO, CAMERA_DISTANCE, 1.0, cell as usize, cell as usize);
    let mut extent: Option<[f32; 4]> = None;
    for capture in captures {
        let (x, y) = probe.project(capture.node);
        for [min_x, max_x, min_y, max_y] in [Some([x, x, y, y]), frame_extent(&capture.frame, &probe.camera)].into_iter().flatten() {
            let bounds = extent.get_or_insert([min_x, max_x, min_y, max_y]);
            *bounds = [bounds[0].min(min_x), bounds[1].max(max_x), bounds[2].min(min_y), bounds[3].max(max_y)];
        }
    }
    let [min_x, max_x, min_y, max_y] = extent.unwrap_or_default();
    let extent = (max_x - min_x).max(max_y - min_y).max(MIN_FIT_EXTENT);
    let center = probe.camera.right * ((min_x + max_x) * 0.5) + probe.camera.up * ((min_y + max_y) * 0.5);
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
        let canvas = draw_frame(capture, textures, &view);
        let x = (index % columns) as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
        let y = (index / columns) as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
        for (px, py, pixel) in canvas.enumerate_pixels() {
            sheet.put_pixel(x + px, y + py, *pixel);
        }
    }
    sheet
}

fn draw_frame(capture: &Capture, textures: &Textures, view: &OrthoView) -> RgbaImage {
    let mut canvas = Canvas::new(view.width, view.height, BACKGROUND);
    let (nx, ny) = view.to_screen(capture.node);
    canvas.fill_square(nx, ny, NODE_DOT_HALF_PX, NODE_COLOR, OVERLAY_BLEND);
    canvas.draw_frame(view, &capture.frame, &|name| textures.get(name), MESH_COLOR);
    RgbaImage::from_raw(view.width as u32, view.height as u32, canvas.to_rgba8()).expect("canvas size")
}
