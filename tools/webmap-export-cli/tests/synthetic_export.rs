//! Synthetic frames through the spool contract to the pyramid and the viewer, on a small grid.
//!
//! A fake renderer paints every base-level pixel from its ground tile alone, so frames agree wherever they
//! overlap. The reference is the same picture painted whole: the frame levels' lossless tiles and every level
//! built from them must equal its box downsamples exactly, which also proves there are no seams between frames.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use webmap_export_cli::export;
use webmap_export_cli::formats::{
    FORMAT_VERSION, FRAME_BYTES_PER_PIXEL, FrameSidecar, Job, PackageRevision, Plan, TextureSource, read_strict, to_toml,
};
use webmap_export_cli::geometry::{FrameIndex, Layout};
use webmap_export_cli::raster::Rgb;
use webmap_export_cli::storage::OutputDir;
use webmap_export_cli::{viewer, webp};

const MAP_TILES: u32 = 64;
const TILE_SIZE_PX: u32 = 64;
const KEEP_TILES: u32 = 8;
const MARGIN_PX: u32 = 8;
const FAKE_RENDER_POLL: Duration = Duration::from_millis(5);
/// Mean absolute channel error allowed between a lossy base tile and its source.
const MAX_MEAN_WEBP_ERROR: f64 = 6.0;

fn plan(region: Option<[u32; 4]>) -> Plan {
    Plan {
        format_version: FORMAT_VERSION,
        facet: 0,
        map_width_tiles: MAP_TILES,
        map_height_tiles: MAP_TILES,
        max_sprite_height_px: 0,
        texture_source: TextureSource::Cc,
        tile_size_px: TILE_SIZE_PX,
        frame_keep_tiles: KEEP_TILES,
        frame_margin_px: MARGIN_PX,
        webp_quality: webp::DEFAULT_QUALITY,
        webp_method: webp::DEFAULT_METHOD,
        webp_sharp_yuv: webp::DEFAULT_SHARP_YUV,
        region,
    }
}

/// The colour of base pixel `(px, py)`: its ground tile's, black outside the facet and on one void tile.
fn paint(layout: &Layout, px: i64, py: i64) -> [u8; 3] {
    let (x, y) = layout.ground_tile_at(px, py);
    let inside = (0..i64::from(layout.width_tiles)).contains(&x) && (0..i64::from(layout.height_tiles)).contains(&y);
    if !inside || (x, y) == (5, 5) {
        return [0, 0, 0];
    }
    [(x * 7 + 20) as u8, (y * 5 + 40) as u8, ((x + y) * 3 + 60) as u8]
}

fn render_frame(layout: &Layout, left: i64, top: i64, size: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((u64::from(size) * u64::from(size) * FRAME_BYTES_PER_PIXEL) as usize);
    for row in 0..i64::from(size) {
        for col in 0..i64::from(size) {
            let [r, g, b] = paint(layout, left + col, top + row);
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    rgba
}

fn write_renamed(path: &Path, data: &[u8]) {
    let temp = PathBuf::from(format!("{}.tmp", path.display()));
    fs::write(&temp, data).unwrap();
    fs::rename(&temp, path).unwrap();
}

/// The renderer side of the contract: frames in job order, pixels then sidecar, at most `max_spool_frames`
/// sidecars waiting.
fn fake_renderer(job_path: PathBuf, layout: Layout) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let job: Job = read_strict(&job_path).unwrap();
        for frame in &job.frames {
            loop {
                let waiting = fs::read_dir(&job.spool_dir)
                    .unwrap()
                    .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "toml"))
                    .count();
                if waiting < job.max_spool_frames as usize {
                    break;
                }
                thread::sleep(FAKE_RENDER_POLL);
            }
            let rgba = render_frame(&layout, frame.left_px, frame.top_px, job.frame_size_px);
            write_renamed(&job.spool_dir.join(format!("{}.rgba", frame.id)), &rgba);
            let check_tile = [3, 4];
            let (px, py) = layout.project(3, 4, 0);
            let sidecar = FrameSidecar {
                format_version: FORMAT_VERSION,
                id: frame.id.clone(),
                left_px: frame.left_px,
                top_px: frame.top_px,
                width_px: job.frame_size_px,
                height_px: job.frame_size_px,
                check_tile,
                check_tile_px: [px, py],
                render_ms: 1,
                packages: vec![PackageRevision { name: "map0.uddp".into(), hash64: "0x0000000000000001".into() }],
            };
            write_renamed(&job.spool_dir.join(format!("{}.toml", frame.id)), to_toml(&sidecar).unwrap().as_bytes());
        }
    })
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("webmap-export-test-{}-{name}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    dir
}

/// Runs the frames through the spool, as `run` does.
fn run_frames(out: &OutputDir, plan: &Plan, layout: &Layout, frames: &[FrameIndex]) {
    let job = export::write_job(out, plan, layout, frames).unwrap();
    let renderer = fake_renderer(job, *layout);
    export::consume_spool(out, plan, layout, frames, None).unwrap();
    renderer.join().unwrap();
}

/// The whole base level painted, padded to whole tiles at level 0.
fn reference_base(layout: &Layout) -> Rgb {
    let side = TILE_SIZE_PX << layout.base_level();
    let mut image = Rgb::black(side, side);
    for py in 0..side {
        for px in 0..side {
            let i = (py as usize * side as usize + px as usize) * 3;
            image.pixels[i..i + 3].copy_from_slice(&paint(layout, i64::from(px), i64::from(py)));
        }
    }
    image
}

fn tile_files(out: &OutputDir, level: u32) -> BTreeMap<(u32, u32), PathBuf> {
    let mut tiles = BTreeMap::new();
    let dir = out.root.join("tiles").join(level.to_string());
    let Ok(columns) = fs::read_dir(&dir) else { return tiles };
    for column in columns {
        let column = column.unwrap().path();
        let x: u32 = column.file_name().unwrap().to_str().unwrap().parse().unwrap();
        for file in fs::read_dir(&column).unwrap() {
            let file = file.unwrap().path();
            assert!(!file.to_str().unwrap().ends_with(".tmp"), "{}", file.display());
            let y: u32 = file.file_stem().unwrap().to_str().unwrap().parse().unwrap();
            tiles.insert((x, y), file);
        }
    }
    tiles
}

#[test]
fn synthetic_frames_build_a_seamless_pyramid() {
    let root = scratch_dir("pyramid");
    let out = OutputDir::new(&root);
    let plan = plan(None);
    export::write_plan(&out, &plan).unwrap();
    assert!(export::write_plan(&out, &plan).is_err(), "a second plan must be refused");
    let layout = plan.layout().unwrap();
    assert_eq!(layout.base_level(), 6);
    assert_eq!(layout.intermediate_level(), 3);
    let all = export::remaining_frames(&out, &plan, &layout).unwrap();
    let (fx, fy) = layout.frames_per_side();
    assert!(!all.is_empty() && all.len() < (fx * fy) as usize, "{} of {}", all.len(), fx * fy);

    // An interrupted run: half the frames, then a resumed run for the rest.
    let half = all.len() / 2;
    run_frames(&out, &plan, &layout, &all[..half]);
    let remaining = export::remaining_frames(&out, &plan, &layout).unwrap();
    assert_eq!(remaining, all[half..]);
    assert!(export::build_pyramid(&out, &plan, &layout).is_err(), "the pyramid needs every frame");
    run_frames(&out, &plan, &layout, &remaining);
    assert!(export::remaining_frames(&out, &plan, &layout).unwrap().is_empty());
    assert_eq!(fs::read_dir(out.spool()).unwrap().count(), 0, "the spool is emptied");
    let counts = export::build_pyramid(&out, &plan, &layout).unwrap();
    assert_eq!(counts.iter().map(|c| c.0).collect::<Vec<_>>(), vec![2, 1, 0]);

    // Every level against the reference, tile by tile.
    let mut reference = reference_base(&layout);
    for level in (0..=layout.base_level()).rev() {
        if level < layout.base_level() {
            reference = reference.downsample_half();
        }
        let per_side = reference.width / TILE_SIZE_PX;
        let files = tile_files(&out, level);
        let mut expected = 0;
        for y in 0..per_side {
            for x in 0..per_side {
                let tile = reference.crop(x * TILE_SIZE_PX, y * TILE_SIZE_PX, TILE_SIZE_PX);
                if tile.is_black() {
                    assert!(!files.contains_key(&(x, y)), "level {level} tile {x},{y} is black but written");
                    continue;
                }
                expected += 1;
                let path = files.get(&(x, y)).unwrap_or_else(|| panic!("level {level} tile {x},{y} is missing"));
                if level <= layout.intermediate_level() {
                    let lossless = out.read_intermediate(level, x, y, TILE_SIZE_PX).unwrap().unwrap();
                    assert!(lossless == tile, "level {level} tile {x},{y}: the lossless tile differs from the reference");
                }
                let (w, h, decoded) = webp::decode_rgb(&fs::read(path).unwrap()).unwrap();
                assert_eq!((w, h), (TILE_SIZE_PX, TILE_SIZE_PX));
                let error: u64 = decoded.iter().zip(&tile.pixels).map(|(a, b)| u64::from(a.abs_diff(*b))).sum();
                let mean = error as f64 / decoded.len() as f64;
                assert!(mean < MAX_MEAN_WEBP_ERROR, "level {level} tile {x},{y}: mean error {mean}");
            }
        }
        assert_eq!(files.len(), expected, "level {level}");
        if let Some(&(_, count)) = counts.iter().find(|c| c.0 == level) {
            assert_eq!(count, expected, "level {level} count");
        }
    }
    assert!(webp::decode_rgb(&fs::read(out.empty_tile()).unwrap()).unwrap().2.iter().all(|&v| v == 0));

    viewer::write_viewer(&out, &plan, &layout).unwrap();
    let html = fs::read_to_string(root.join(viewer::INDEX_FILE)).unwrap();
    for needle in ["maxNativeZoom: facet.baseZoom", "baseZoom: 6, maxZoom: 8, tileSize: 64", "errorTileUrl: \"tiles/empty.webp\"", "zTop: 512"] {
        assert!(html.contains(needle), "index.html lacks {needle}");
    }
    assert!(!html.contains('@'), "an unreplaced placeholder is left");
    assert_eq!(fs::read(root.join("leaflet/leaflet.js")).unwrap().len(), 147_552);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn spool_rejects_a_misplaced_frame_and_cleans_done_leftovers() {
    let root = scratch_dir("reject");
    let out = OutputDir::new(&root);
    let plan = plan(Some([0, 0, 4, 4]));
    export::write_plan(&out, &plan).unwrap();
    let layout = plan.layout().unwrap();
    let frames = export::remaining_frames(&out, &plan, &layout).unwrap();
    assert!(!frames.is_empty() && frames.len() <= 4, "{frames:?}");
    run_frames(&out, &plan, &layout, &frames[..1]);

    // A leftover of a done frame is removed; a frame whose projection disagrees stops the run.
    let done = frames[0].id();
    fs::write(out.spool().join(format!("{done}.toml")), "").unwrap();
    let Some(&next) = frames.get(1) else {
        fs::remove_dir_all(&root).unwrap();
        return;
    };
    let rect = layout.frame_rect(next);
    let (px, py) = layout.project(3, 4, 0);
    let sidecar = FrameSidecar {
        format_version: FORMAT_VERSION,
        id: next.id(),
        left_px: rect.left,
        top_px: rect.top,
        width_px: rect.width as u32,
        height_px: rect.height as u32,
        check_tile: [3, 4],
        check_tile_px: [px + 1, py],
        render_ms: 1,
        packages: Vec::new(),
    };
    fs::write(out.spool().join(format!("{}.rgba", next.id())), render_frame(&layout, rect.left, rect.top, rect.width as u32)).unwrap();
    fs::write(out.spool().join(format!("{}.toml", next.id())), to_toml(&sidecar).unwrap()).unwrap();
    let error = export::consume_spool(&out, &plan, &layout, &frames[1..], None).unwrap_err();
    assert!(error.to_string().contains("projection"), "{error}");
    assert!(!out.spool().join(format!("{done}.toml")).exists());
    fs::remove_dir_all(&root).unwrap();
}
