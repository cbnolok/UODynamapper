//! The capture pilot through the renderer's development control socket (`DYNAMAPPER_CONTROL_SOCKET`): four
//! screenshots at tile offsets that move the view by a known number of pixels right, down and diagonally. The
//! overlaps show whether two captures of the same ground agree pixel for pixel, which the export mode needs, and
//! the WebP sizes and times of real frames.
//!
//! The renderer must run with the control socket and a window it is not resized during the pilot.

use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use color_eyre::eyre::{self, WrapErr};
use serde::Serialize;
use serde_json::{Value, json};

use crate::geometry::TILE_HALF_PX;
use crate::raster::{RGB_CHANNELS, Rgb};
use crate::storage::write_atomic;
use crate::webp::{self, WebpSettings};

/// Status polls between readiness checks, and how many consecutive ready polls count as settled.
pub const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub const READY_STABLE_POLLS: u32 = 3;
pub const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a capture may take to appear on disk.
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REPLY_BYTES: usize = 1 << 20;
pub const REPORT_FILE: &str = "pilot-report.toml";
/// Captures: (0, 0), then (n, −n) one step right, (n, n) one step down, (2n, 0) one step diagonally.
const CAPTURE_OFFSETS: [(i64, i64); 4] = [(0, 0), (1, -1), (1, 1), (2, 0)];
/// Capture pairs compared, with the screen step between them in units of `2 · 22 · n` pixels.
const PAIRS: [(usize, usize, i64, i64); 4] = [(0, 1, 1, 0), (0, 2, 0, 1), (0, 3, 1, 1), (1, 2, -1, 1)];

/// A rectangle of the capture, in physical pixels, left out of the comparison (HUD, overlays).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Mask {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

impl std::str::FromStr for Mask {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let parts: Vec<u32> = text.split(',').map(|v| v.trim().parse().map_err(|_| format!("bad mask {text}"))).collect::<Result<_, _>>()?;
        let [left, top, width, height] = parts[..] else { return Err(format!("mask {text} is not x,y,w,h")) };
        Ok(Self { left, top, width, height })
    }
}

pub struct PilotOptions {
    pub socket: PathBuf,
    pub out_dir: PathBuf,
    pub start: (u16, u16, i8),
    pub facet: u8,
    pub step_tiles: u16,
    /// Physical pixels per client pixel (window scale factor times camera zoom).
    pub scale: f64,
    pub search_radius_px: u32,
    pub masks: Vec<Mask>,
    pub webp: WebpSettings,
}

#[derive(Debug, Serialize)]
struct PairReport {
    first: usize,
    second: usize,
    expected_shift_px: [i64; 2],
    best_shift_px: [i64; 2],
    best_mean_abs_diff: f64,
    /// Pixels that differ at the best shift, over the compared pixels.
    best_differing_fraction: f64,
    compared_pixels: u64,
}

#[derive(Debug, Serialize)]
struct CaptureReport {
    file: String,
    tile: [i64; 2],
    width: u32,
    height: u32,
    settle_ms: u64,
    webp_bytes: u64,
    webp_bits_per_pixel: f64,
    webp_encode_ms: u64,
}

#[derive(Debug, Serialize)]
struct Report {
    step_tiles: u16,
    scale: f64,
    masks: Vec<Mask>,
    captures: Vec<CaptureReport>,
    pairs: Vec<PairReport>,
    /// The status reply after the last capture.
    status: String,
}

struct Control {
    socket: UnixDatagram,
    client_path: PathBuf,
    server: PathBuf,
}

impl Control {
    fn connect(server: &Path, out_dir: &Path) -> eyre::Result<Self> {
        let client_path = out_dir.join(format!("pilot-{}.sock", std::process::id()));
        let socket = UnixDatagram::bind(&client_path).wrap_err_with(|| format!("bind {}", client_path.display()))?;
        socket.set_read_timeout(Some(REPLY_TIMEOUT))?;
        Ok(Self { socket, client_path, server: server.to_path_buf() })
    }

    fn request(&self, command: &Value) -> eyre::Result<Value> {
        self.socket.send_to(command.to_string().as_bytes(), &self.server).wrap_err_with(|| format!("send to {}", self.server.display()))?;
        let mut buffer = vec![0; MAX_REPLY_BYTES];
        let size = self.socket.recv(&mut buffer).wrap_err("no reply from the renderer")?;
        let reply: Value = serde_json::from_slice(&buffer[..size]).wrap_err("parse the reply")?;
        if reply["ok"] != json!(true) {
            eyre::bail!("the renderer refused {command}: {}", reply["error"]);
        }
        Ok(reply)
    }

    /// Waits until the view shows the teleport target with every upload done, for several polls in a row.
    fn wait_ready(&self) -> eyre::Result<Duration> {
        let started = Instant::now();
        let mut stable = 0;
        while stable < READY_STABLE_POLLS {
            if started.elapsed() > READY_TIMEOUT {
                eyre::bail!("the renderer did not settle within {READY_TIMEOUT:?}");
            }
            std::thread::sleep(READY_POLL_INTERVAL);
            let status = self.request(&json!({"command": "status"}))?;
            // No terrain block (no terrain atlas) counts as nothing pending; the art block must be there, since it
            // only appears after the first collection.
            let ready = !status["position"].is_null()
                && status["position"] == status["rendered_position"]
                && status["terrain"]["metadata_pending"] != json!(true)
                && status["art"]["pending_pages"] == json!(0);
            stable = if ready { stable + 1 } else { 0 };
        }
        Ok(started.elapsed())
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.client_path);
    }
}

fn load_png(path: &Path) -> eyre::Result<Rgb> {
    let image = image::open(path).wrap_err_with(|| format!("read {}", path.display()))?.into_rgb8();
    Ok(Rgb { width: image.width(), height: image.height(), pixels: image.into_raw() })
}

fn wait_for_file(path: &Path) -> eyre::Result<()> {
    let started = Instant::now();
    let mut last_len = None;
    loop {
        let len = std::fs::metadata(path).ok().map(|m| m.len());
        if len.is_some() && len == last_len && image::open(path).is_ok() {
            return Ok(());
        }
        if started.elapsed() > CAPTURE_TIMEOUT {
            eyre::bail!("{} did not appear within {CAPTURE_TIMEOUT:?}", path.display());
        }
        last_len = len;
        std::thread::sleep(READY_POLL_INTERVAL);
    }
}

fn masked(masks: &[Mask], x: u32, y: u32) -> bool {
    masks.iter().any(|m| x >= m.left && x < m.left + m.width && y >= m.top && y < m.top + m.height)
}

/// Compares `b` against `a` moved by `(dx, dy)`: pixel `(x, y)` of `b` shows what pixel `(x + dx, y + dy)` of `a`
/// shows. Returns the mean absolute channel difference, the differing pixel fraction and the compared pixels.
fn compare(a: &Rgb, b: &Rgb, dx: i64, dy: i64, masks: &[Mask]) -> (f64, f64, u64) {
    let (mut sum, mut differing, mut count) = (0u64, 0u64, 0u64);
    for y in 0..b.height {
        let ay = i64::from(y) + dy;
        if ay < 0 || ay >= i64::from(a.height) {
            continue;
        }
        for x in 0..b.width {
            let ax = i64::from(x) + dx;
            if ax < 0 || ax >= i64::from(a.width) || masked(masks, x, y) || masked(masks, ax as u32, ay as u32) {
                continue;
            }
            let pb = (y as usize * b.width as usize + x as usize) * RGB_CHANNELS;
            let pa = (ay as usize * a.width as usize + ax as usize) * RGB_CHANNELS;
            let diff: u64 = (0..RGB_CHANNELS).map(|c| u64::from(a.pixels[pa + c].abs_diff(b.pixels[pb + c]))).sum();
            sum += diff;
            differing += u64::from(diff != 0);
            count += 1;
        }
    }
    if count == 0 {
        return (f64::INFINITY, 1.0, 0);
    }
    (sum as f64 / (count * RGB_CHANNELS as u64) as f64, differing as f64 / count as f64, count)
}

pub fn run_pilot(options: &PilotOptions) -> eyre::Result<()> {
    std::fs::create_dir_all(&options.out_dir)?;
    let control = Control::connect(&options.socket, &options.out_dir)?;
    let (x0, y0, z) = options.start;
    let n = i64::from(options.step_tiles);
    let mut captures = Vec::new();
    let mut images = Vec::new();
    for (k, (ox, oy)) in CAPTURE_OFFSETS.iter().enumerate() {
        let (x, y) = (i64::from(x0) + ox * n, i64::from(y0) + oy * n);
        let (x, y) = (u16::try_from(x).wrap_err("tile x out of range")?, u16::try_from(y).wrap_err("tile y out of range")?);
        control.request(&json!({"command": "teleport", "x": x, "y": y, "z": z, "map": options.facet}))?;
        let settle = control.wait_ready()?;
        let file = format!("pilot-{k}.png");
        let path = options.out_dir.join(&file);
        if path.exists() {
            eyre::bail!("{} exists; use a new output directory", path.display());
        }
        control.request(&json!({"command": "capture", "path": path}))?;
        wait_for_file(&path)?;
        let image = load_png(&path)?;
        let started = Instant::now();
        let encoded = webp::encode_rgb(&image.pixels, image.width, image.height, options.webp)?;
        let encode_ms = started.elapsed().as_millis() as u64;
        captures.push(CaptureReport {
            file,
            tile: [i64::from(x), i64::from(y)],
            width: image.width,
            height: image.height,
            settle_ms: settle.as_millis() as u64,
            webp_bytes: encoded.len() as u64,
            webp_bits_per_pixel: encoded.len() as f64 * 8.0 / (f64::from(image.width) * f64::from(image.height)),
            webp_encode_ms: encode_ms,
        });
        eprintln!("capture {k}: tile {x},{y}, settled in {} ms", settle.as_millis());
        images.push(image);
    }
    let step_px = 2.0 * TILE_HALF_PX as f64 * n as f64 * options.scale;
    let radius = i64::from(options.search_radius_px);
    let mut pairs = Vec::new();
    for (first, second, sx, sy) in PAIRS {
        let expected = [(sx as f64 * step_px).round() as i64, (sy as f64 * step_px).round() as i64];
        let mut best = ([0, 0], f64::INFINITY, 1.0, 0);
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let shift = [expected[0] + dx, expected[1] + dy];
                let (mad, fraction, count) = compare(&images[first], &images[second], shift[0], shift[1], &options.masks);
                if mad < best.1 {
                    best = (shift, mad, fraction, count);
                }
            }
        }
        eprintln!("pair {first}-{second}: expected {expected:?}, best {:?}, mean diff {:.3}, differing {:.4}", best.0, best.1, best.2);
        pairs.push(PairReport {
            first,
            second,
            expected_shift_px: expected,
            best_shift_px: best.0,
            best_mean_abs_diff: best.1,
            best_differing_fraction: best.2,
            compared_pixels: best.3,
        });
    }
    let status = control.request(&json!({"command": "status"}))?.to_string();
    let report = Report {
        step_tiles: options.step_tiles,
        scale: options.scale,
        masks: options.masks.clone(),
        captures,
        pairs,
        status,
    };
    let path = options.out_dir.join(REPORT_FILE);
    write_atomic(&path, toml::to_string(&report).wrap_err("serialize the report")?.as_bytes())?;
    eprintln!("report: {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_finds_the_shift() {
        let a = Rgb { width: 8, height: 8, pixels: (0..8 * 8 * 3).map(|v| (v * 7 % 251) as u8).collect() };
        let mut b = Rgb::black(8, 8);
        // b shows a moved two pixels left: b(x, y) = a(x + 2, y).
        for y in 0..8 {
            for x in 0..6 {
                let (pa, pb) = ((y * 8 + x + 2) * 3, (y * 8 + x) * 3);
                b.pixels[pb..pb + 3].copy_from_slice(&a.pixels[pa..pa + 3].to_vec());
            }
        }
        assert_eq!(compare(&a, &b, 2, 0, &[]).0, 0.0);
        assert!(compare(&a, &b, 1, 0, &[]).0 > 0.0);
        assert!("1,2,3".parse::<Mask>().is_err());
        assert_eq!("1,2,3,4".parse::<Mask>().unwrap().height, 4);
    }

    const FAKE_WIDTH: u32 = 320;
    const FAKE_HEIGHT: u32 = 200;
    const FAKE_STEP_TILES: u16 = 2;

    /// A renderer stand-in: screen pixel (sx, sy) shows world pixel ((x − y)·22 + sx, (x + y)·22 + sy).
    fn fake_renderer(socket_path: PathBuf) -> std::thread::JoinHandle<()> {
        let socket = UnixDatagram::bind(&socket_path).unwrap();
        std::thread::spawn(move || {
            let mut position = json!(null);
            let mut buffer = vec![0; MAX_REPLY_BYTES];
            loop {
                let (size, peer) = socket.recv_from(&mut buffer).unwrap();
                let command: Value = serde_json::from_slice(&buffer[..size]).unwrap();
                let reply = match command["command"].as_str().unwrap() {
                    "teleport" => {
                        position = json!([command["x"], command["y"], command["z"], command["map"]]);
                        json!({"ok": true})
                    }
                    "status" => json!({"ok": true, "position": position, "rendered_position": position,
                        "terrain": {"metadata_pending": false}, "art": {"pending_pages": 0}}),
                    "capture" => {
                        let (x, y) = (position[0].as_i64().unwrap(), position[1].as_i64().unwrap());
                        let (cx, cy) = ((x - y) * TILE_HALF_PX, (x + y) * TILE_HALF_PX);
                        let image = image::RgbImage::from_fn(FAKE_WIDTH, FAKE_HEIGHT, |sx, sy| {
                            let (wx, wy) = (cx + i64::from(sx), cy + i64::from(sy));
                            image::Rgb([(wx * 31 + wy * 17) as u8, (wx * wy) as u8, (wy * 13) as u8])
                        });
                        image.save(command["path"].as_str().unwrap()).unwrap();
                        json!({"ok": true})
                    }
                    _ => return,
                };
                socket.send_to(reply.to_string().as_bytes(), peer.as_pathname().unwrap()).unwrap();
            }
        })
    }

    #[test]
    fn pilot_against_a_fake_renderer_finds_seam_free_overlaps() {
        let dir = std::env::temp_dir().join(format!("webmap-pilot-test-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        let server = dir.join("renderer.sock");
        let renderer = fake_renderer(server.clone());
        let options = PilotOptions {
            socket: server.clone(),
            out_dir: dir.join("out"),
            start: (100, 100, 0),
            facet: 0,
            step_tiles: FAKE_STEP_TILES,
            scale: 1.0,
            search_radius_px: 2,
            masks: vec![Mask { left: 0, top: 0, width: 10, height: 10 }],
            webp: WebpSettings::default(),
        };
        run_pilot(&options).unwrap();
        let report: toml::Value = toml::from_str(&std::fs::read_to_string(dir.join("out").join(REPORT_FILE)).unwrap()).unwrap();
        let pairs = report["pairs"].as_array().unwrap();
        assert_eq!(pairs.len(), PAIRS.len());
        for pair in pairs {
            assert_eq!(pair["best_shift_px"], pair["expected_shift_px"], "{pair}");
            assert_eq!(pair["best_mean_abs_diff"].as_float(), Some(0.0), "{pair}");
            assert!(pair["compared_pixels"].as_integer().unwrap() > 0);
        }
        // Stop the stand-in.
        let client = UnixDatagram::unbound().unwrap();
        client.send_to(br#"{"command":"quit"}"#, &server).unwrap();
        renderer.join().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
