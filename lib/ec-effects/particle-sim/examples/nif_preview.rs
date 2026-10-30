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
//! The camera is the client's view (orthographic along the effect's local +y with +z up:
//! `CameraFrame::client_view`), fitted to everything drawn over the run; billboards face it. Draws follow the
//! scene order with the client's depth test and write, and textures have the client's mip levels
//! (`common/textures.rs`) sampled with each map's filter mode.

#[path = "common/textures.rs"]
mod textures;

use std::path::PathBuf;
use std::sync::Arc;

use gamebryo_nif::NifFile;
use image::{Rgba, RgbaImage};
use particle_sim::nif::{NifEffect, NifEffectInstance, compile};
use particle_sim::raster::{CLIENT_VIEW_ELEVATION, Canvas, OrthoView, RasterTexture, SceneDrawOptions, frame_extent};
use particle_sim::render::RenderFrame;
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
/// Camera distance from the fitted centre (orthographic: only billboards and axial sprites see it).
const CAMERA_DISTANCE: f32 = 1000.0;
const BACKGROUND: [f32; 4] = [0.10, 0.10, 0.12, 1.0];
const GAP_COLOR: Rgba<u8> = Rgba([40, 40, 48, 255]);
const MS_PER_SECOND: f32 = 1000.0;
const HALF: f32 = 0.5;

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
    for (index, effect) in desc.texture_effects.iter().enumerate() {
        let affected: Vec<&str> =
            desc.nodes.iter().filter(|node| node.texture_effects.contains(&index)).map(|node| node.name.as_str()).collect();
        println!(
            "  texture effect {:?}: {:?} {:?}, on {}, texture {:?}, listed by {affected:?}",
            desc.nodes[effect.node].name,
            effect.kind,
            effect.coord_gen,
            effect.on,
            effect.texture.and_then(|texture| desc.textures.get(texture)).map(|texture| texture.name()),
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
        Self(textures::load_nif_textures(package, desc))
    }

    fn get(&self, name: &str) -> Option<&RasterTexture> {
        self.0.iter().find(|(texture, _)| texture.eq_ignore_ascii_case(name)).map(|(_, texture)| texture)
    }
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
    let elevation = CLIENT_VIEW_ELEVATION;
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

/// Fits everything drawn over the run into a cell.
fn fit_view(captures: &[Capture], cell: u32) -> OrthoView {
    let elevation = CLIENT_VIEW_ELEVATION;
    let probe = OrthoView::new(elevation, Vec3::ZERO, CAMERA_DISTANCE, 1.0, cell as usize, cell as usize);
    let mut extent: Option<[f32; 4]> = None;
    let (x, y) = probe.project(Vec3::ZERO);
    for [min_x, max_x, min_y, max_y] in
        std::iter::once([x, x, y, y]).chain(captures.iter().filter_map(|capture| frame_extent(&capture.frame, &probe.camera)))
    {
        let bounds = extent.get_or_insert([min_x, max_x, min_y, max_y]);
        *bounds = [bounds[0].min(min_x), bounds[1].max(max_x), bounds[2].min(min_y), bounds[3].max(max_y)];
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
        canvas.draw_scene(&view, &capture.frame, &|name| textures.get(name), &SceneDrawOptions::default());
        let image = RgbaImage::from_raw(view.width as u32, view.height as u32, canvas.to_rgba8()).expect("canvas size");
        let x = (index % columns) as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
        let y = (index / columns) as u32 * (cell + CELL_GAP_PX) + CELL_GAP_PX;
        for (px, py, pixel) in image.enumerate_pixels() {
            sheet.put_pixel(x + px, y + py, *pixel);
        }
    }
    sheet
}
