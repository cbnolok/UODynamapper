//! Web map export: plan a facet, consume the renderer's frames, build the pyramid and the viewer.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use color_eyre::eyre::{self, WrapErr};

use webmap_export_cli::export;
use webmap_export_cli::formats::{FORMAT_VERSION, Plan, TextureSource};
use webmap_export_cli::geometry::{FRAME_KEEP_TILES, FRAME_MARGIN_PX, TILE_SIZE_PX};
use webmap_export_cli::packages;
use webmap_export_cli::storage::OutputDir;
use webmap_export_cli::viewer;
use webmap_export_cli::webp::{DEFAULT_METHOD, DEFAULT_QUALITY, WebpSettings};

/// The environment variable that puts the renderer in export mode, holding the job file's absolute path.
const EXPORT_JOB_ENV: &str = "DYNAMAPPER_EXPORT_JOB";
/// How long the renderer may take to exit after its last frame before it is killed.
const RENDERER_EXIT_GRACE: Duration = Duration::from_secs(30);
const RENDERER_EXIT_POLL: Duration = Duration::from_millis(100);
#[cfg(unix)]
const DEFAULT_PILOT_STEP_TILES: u16 = 8;
#[cfg(unix)]
const DEFAULT_PILOT_SEARCH_RADIUS_PX: u32 = 4;
#[cfg(unix)]
const DEFAULT_PILOT_SCALE: f64 = 1.0;

/// Static web map export: WebP tile pyramid and Leaflet viewer from the renderer's export mode.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create an output directory and its plan from the installed packages.
    Plan {
        /// Absolute path of the new output directory.
        #[arg(long)]
        out: PathBuf,
        /// Directory holding map{N}.uddp and tex_art_cc.uddp.
        #[arg(long)]
        packages: PathBuf,
        #[arg(long, default_value_t = 0)]
        facet: u32,
        #[arg(long, value_enum, default_value_t = TextureSource::Cc)]
        texture_source: TextureSource,
        /// Export only the tiles x0,y0,x1,y1 (end exclusive), e.g. for a test run.
        #[arg(long, value_parser = parse_region)]
        region: Option<[u32; 4]>,
        #[arg(long, default_value_t = DEFAULT_QUALITY)]
        quality: f32,
        #[arg(long, default_value_t = DEFAULT_METHOD)]
        method: i32,
        /// Sharp RGB to YUV conversion (on by default).
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        sharp_yuv: bool,
    },
    /// Write job.toml for the frames not done yet and consume the renderer's spool; then build the pyramid and
    /// the viewer. Resumes where an interrupted run stopped.
    Run {
        #[arg(long)]
        out: PathBuf,
        /// Start this renderer with DYNAMAPPER_EXPORT_JOB set; without it, start the renderer yourself with the
        /// printed job path.
        #[arg(long)]
        renderer: Option<PathBuf>,
        /// Arguments for the renderer.
        #[arg(long = "renderer-arg", allow_hyphen_values = true)]
        renderer_args: Vec<String>,
        /// Stop after the frames; skip the pyramid and the viewer.
        #[arg(long)]
        frames_only: bool,
    },
    /// Build the levels below the frame levels from the lossless tiles (every frame must be done).
    Pyramid {
        #[arg(long)]
        out: PathBuf,
    },
    /// Write index.html and the Leaflet files.
    Viewer {
        #[arg(long)]
        out: PathBuf,
    },
    /// Capture four overlapping screenshots through the development control socket and report whether the
    /// overlaps agree, with WebP sizes and times.
    #[cfg(unix)]
    Pilot {
        /// The renderer's DYNAMAPPER_CONTROL_SOCKET.
        #[arg(long)]
        socket: PathBuf,
        /// Absolute path of a new directory for the captures and pilot-report.toml.
        #[arg(long)]
        out: PathBuf,
        /// First position, x,y,z.
        #[arg(long, value_parser = parse_position)]
        start: (u16, u16, i8),
        #[arg(long, default_value_t = 0)]
        facet: u8,
        /// Tiles between captures along each axis; the view moves 44 × step client pixels.
        #[arg(long, default_value_t = DEFAULT_PILOT_STEP_TILES)]
        step: u16,
        /// Physical pixels per client pixel: the window scale factor times the camera zoom.
        #[arg(long, default_value_t = DEFAULT_PILOT_SCALE)]
        scale: f64,
        /// Pixels searched around each expected shift.
        #[arg(long, default_value_t = DEFAULT_PILOT_SEARCH_RADIUS_PX)]
        search_radius: u32,
        /// Capture rectangle x,y,w,h left out of the comparison (repeatable), e.g. the HUD.
        #[arg(long = "mask")]
        masks: Vec<webmap_export_cli::pilot::Mask>,
    },
}

fn parse_region(text: &str) -> Result<[u32; 4], String> {
    let values: Vec<u32> = text.split(',').map(|v| v.trim().parse().map_err(|_| format!("bad region {text}"))).collect::<Result<_, _>>()?;
    let [x0, y0, x1, y1] = values[..] else { return Err(format!("region {text} is not x0,y0,x1,y1")) };
    if x0 >= x1 || y0 >= y1 {
        return Err(format!("region {text} is empty"));
    }
    Ok([x0, y0, x1, y1])
}

#[cfg(unix)]
fn parse_position(text: &str) -> Result<(u16, u16, i8), String> {
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    let [x, y, z] = parts[..] else { return Err(format!("position {text} is not x,y,z")) };
    let bad = |_| format!("bad position {text}");
    Ok((x.parse().map_err(bad)?, y.parse().map_err(bad)?, z.parse().map_err(bad)?))
}

fn require_absolute(path: &Path) -> eyre::Result<()> {
    if !path.is_absolute() {
        eyre::bail!("{} is not an absolute path", path.display());
    }
    Ok(())
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    match Cli::parse().command {
        Commands::Plan { out, packages: packages_dir, facet, texture_source, region, quality, method, sharp_yuv } => {
            require_absolute(&out)?;
            let output = OutputDir::new(&out);
            if !output.plan_file().exists() && export::is_non_empty_dir(&out) {
                eyre::bail!("{} is not empty; plan into a new directory", out.display());
            }
            let (width, height) = packages::read_facet_size(&packages_dir, facet)?;
            let max_sprite_height_px = packages::read_max_sprite_height(&packages_dir)?;
            let plan = Plan {
                format_version: FORMAT_VERSION,
                facet,
                map_width_tiles: width,
                map_height_tiles: height,
                max_sprite_height_px,
                texture_source,
                tile_size_px: TILE_SIZE_PX,
                frame_keep_tiles: FRAME_KEEP_TILES,
                frame_margin_px: FRAME_MARGIN_PX,
                webp_quality: quality,
                webp_method: method,
                webp_sharp_yuv: sharp_yuv,
                region,
            };
            WebpSettings { quality, method, sharp_yuv }.validate().map_err(|error| eyre::eyre!(error))?;
            export::write_plan(&output, &plan)?;
            let layout = plan.layout()?;
            let frames = layout.frames(plan.region);
            println!(
                "facet {facet}: {width}x{height} tiles, tallest sprite {max_sprite_height_px} px, {} levels, {} frames of \
                 {} px; plan {}",
                layout.base_level() + 1,
                frames.len(),
                layout.grid.frame_size_px(),
                output.plan_file().display()
            );
        }
        Commands::Run { out, renderer, renderer_args, frames_only } => {
            require_absolute(&out)?;
            let output = OutputDir::new(&out);
            let plan = export::read_plan(&output)?;
            let layout = plan.layout()?;
            let frames = export::remaining_frames(&output, &plan, &layout)?;
            if !frames.is_empty() {
                let job = export::write_job(&output, &plan, &layout, &frames)?;
                println!("{} frames to render; job {}", frames.len(), job.display());
                let mut child = match &renderer {
                    Some(exe) => Some(
                        Command::new(exe)
                            .args(&renderer_args)
                            .env(EXPORT_JOB_ENV, &job)
                            .spawn()
                            .wrap_err_with(|| format!("start {}", exe.display()))?,
                    ),
                    None => {
                        println!("start the renderer with {EXPORT_JOB_ENV}={}", job.display());
                        None
                    }
                };
                export::consume_spool(&output, &plan, &layout, &frames, child.as_mut())?;
                if let Some(child) = child.as_mut() {
                    wait_or_kill(child)?;
                }
            }
            if !frames_only {
                export::build_pyramid(&output, &plan, &layout)?;
                viewer::write_viewer(&output, &plan, &layout)?;
                println!("viewer: {}", out.join(viewer::INDEX_FILE).display());
            }
        }
        Commands::Pyramid { out } => {
            let output = OutputDir::new(&out);
            let plan = export::read_plan(&output)?;
            export::build_pyramid(&output, &plan, &plan.layout()?)?;
        }
        Commands::Viewer { out } => {
            let output = OutputDir::new(&out);
            let plan = export::read_plan(&output)?;
            viewer::write_viewer(&output, &plan, &plan.layout()?)?;
            println!("viewer: {}", out.join(viewer::INDEX_FILE).display());
        }
        #[cfg(unix)]
        Commands::Pilot { socket, out, start, facet, step, scale, search_radius, masks } => {
            require_absolute(&out)?;
            require_absolute(&socket)?;
            webmap_export_cli::pilot::run_pilot(&webmap_export_cli::pilot::PilotOptions {
                socket,
                out_dir: out,
                start,
                facet,
                step_tiles: step,
                scale,
                search_radius_px: search_radius,
                masks,
                webp: WebpSettings::default(),
            })?;
        }
    }
    Ok(())
}

/// Lets the renderer exit by itself after its last frame, then kills it.
fn wait_or_kill(child: &mut Child) -> eyre::Result<()> {
    let started = Instant::now();
    while started.elapsed() < RENDERER_EXIT_GRACE {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                eprintln!("warning: the renderer exited with {status} after the last frame");
            }
            return Ok(());
        }
        std::thread::sleep(RENDERER_EXIT_POLL);
    }
    eprintln!("warning: the renderer did not exit within {RENDERER_EXIT_GRACE:?} after the last frame; killing it");
    child.kill()?;
    child.wait()?;
    Ok(())
}
