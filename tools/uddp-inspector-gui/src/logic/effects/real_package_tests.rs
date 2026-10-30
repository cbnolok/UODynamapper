//! Checks against the packed client effects (ignored: they need the packages in [`TEST_PACKAGE_DIR`]). Run with
//! `cargo test -p uddp-inspector-gui real_package -- --ignored --nocapture`.

use std::path::{Path, PathBuf};

use particle_sim::Vec3;
use particle_sim::render::RenderFrame;
use udd_assets::EffectsEcPackage;
use udd_assets::effects_ec::{DEP_SYSTEM_BIT, EffectNode, SYSTEM_FLAG_COMPILE_FAILED, SYSTEM_FLAG_MISSING, SYSTEM_KIND_CHAOS, SYSTEM_KIND_NIF};
use udd_container::UddpReader;

use super::hues::{HueAtlas, load_hues_package};
use super::model::{EffectsModel, RootKey, RootRow, walk_nodes};
use super::preview::{
    PartStatus, PreviewScene, PreviewSimulation, PreviewTarget, ResourceCache, ResourceState, ResourceUse,
    SimulationSettings, ViewSettings, WorldBounds, fit_bounds, preview_view, render_canvas,
};
use crate::ui::effects::EffectsViewer;

const TEST_PACKAGE_DIR: &str = "/home/claudio/test/uddp";
const PACKAGES: [&str; 2] = ["effects_ec.uddp", "effects_ec_bc7.uddp"];
const HUES_PACKAGE: &str = "hues.uddp";
/// Where the reference renders go.
const RENDER_DIR: &str = "/home/claudio/rust-target/uodynamapper-scratch/uddp-effects-viewer";
/// Steps simulated per root list in the sweep.
const SWEEP_STEPS: u32 = 6;
/// Canvas side of the sweep renders (they are only drawn to exercise the rasterizer).
const SWEEP_CANVAS_PX: usize = 48;
/// World box of the sweep renders.
const SWEEP_EXTENT: f32 = 8.0;
/// Times of the reference render strips.
const STRIP_TIMES_MS: [u32; 4] = [250, 750, 1500, 3000];
/// Object hue of the hued tileart render when its PropertyFlag has no hue case (a saturated red, `hues.uddp` 38).
const DEMO_HUE: u32 = 38;
const RGBA: usize = 4;
/// Zoom of the hued tileart strip: the fit includes the hue quad, much larger than a thrown item's mesh.
const HUED_STRIP_ZOOM: f32 = 6.0;

fn package_path(name: &str) -> PathBuf {
    Path::new(TEST_PACKAGE_DIR).join(name)
}

fn open(name: &str) -> (EffectsEcPackage, UddpReader) {
    let reader = UddpReader::load(package_path(name)).expect("package container");
    let package = EffectsEcPackage::from_reader(reader.clone()).expect("package opens");
    (package, reader)
}

/// Every texture a frame draws with.
fn frame_textures(frame: &RenderFrame) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    names.extend(frame.batches.iter().map(|batch| batch.texture.to_string()));
    for state in &frame.batch_states {
        names.extend(state.textures.iter().map(|stage| stage.texture.to_string()));
    }
    for draw in &frame.mesh_draws {
        names.extend(draw.textures.iter().map(|stage| stage.texture.to_string()));
        names.extend(draw.env_map.iter().map(|map| map.texture.to_string()));
    }
    names.retain(|name| !name.is_empty());
    names
}

/// A resource resolves, or the package says it has no payload.
fn check_resource(package: &EffectsEcPackage, what: &str, resource: &ResourceUse) -> Result<(), String> {
    match &resource.state {
        ResourceState::Loaded => Ok(()),
        ResourceState::NoPayload { .. } if resource.index.is_some() => Ok(()),
        state => Err(format!("{what}: {} {state:?} (record {:?})", resource.name, package.texture_by_name(&resource.name))),
    }
}

/// What the sweep exercised.
#[derive(Default)]
struct SweepCounts {
    playing_parts: usize,
    particles: usize,
    textures_drawn: usize,
}

/// Builds `key`, simulates a few steps and renders; returns the problems found.
fn sweep_root(package: &EffectsEcPackage, cache: &mut ResourceCache, key: RootKey, counts: &mut SweepCounts) -> Vec<String> {
    let mut problems = Vec::new();
    let settings = SimulationSettings::default();
    let scene = PreviewScene::build(package, cache, PreviewTarget::Root(key), &settings);
    let label = key.label();
    for part in &scene.parts {
        let what = format!("{label} [{}]", part.path);
        let record = part.system.map(|system| *package.system(system));
        let expected = match (&part.status, record) {
            (PartStatus::NotInClient { .. }, Some(record)) => record.flags & SYSTEM_FLAG_MISSING != 0,
            (PartStatus::CompileFailed { .. }, Some(record)) => {
                record.flags & (SYSTEM_FLAG_MISSING | SYSTEM_FLAG_COMPILE_FAILED) == SYSTEM_FLAG_COMPILE_FAILED
            }
            (PartStatus::Chaos { .. }, Some(record)) => record.kind == SYSTEM_KIND_CHAOS && record.has_payload(),
            (PartStatus::Nif { .. }, Some(record)) => record.kind == SYSTEM_KIND_NIF && record.has_payload(),
            (PartStatus::NoSystem, None) | (PartStatus::NotVisual { .. }, _) | (PartStatus::ProjectileOnly { .. }, _) => true,
            _ => false,
        };
        if !expected {
            problems.push(format!("{what}: status {:?} does not match the system record {record:?}", part.status));
        }
        for texture in &part.textures {
            if let Err(problem) = check_resource(package, &what, texture) {
                problems.push(problem);
            }
        }
        for mesh in &part.meshes {
            if !matches!(mesh.state, ResourceState::Loaded | ResourceState::NoPayload { .. }) {
                problems.push(format!("{what}: MESH {} {:?}", mesh.name, mesh.state));
            }
        }
    }
    counts.playing_parts += scene.playing_part_count();
    let mut simulation = PreviewSimulation::new(&scene);
    for _ in 0..SWEEP_STEPS {
        simulation.step();
        for name in frame_textures(&simulation.render()) {
            counts.textures_drawn += 1;
            let resolved = match package.texture_by_name(&name) {
                Some(index) => scene.texture(&name).is_some() || !package.texture(index).has_payload(),
                None => false,
            };
            if !resolved {
                problems.push(format!("{label}: the frame draws {name}, which does not resolve"));
            }
        }
    }
    counts.particles += simulation.particle_count();
    let bounds: WorldBounds = [Vec3::new(-SWEEP_EXTENT, -SWEEP_EXTENT, -SWEEP_EXTENT), Vec3::new(SWEEP_EXTENT, SWEEP_EXTENT, SWEEP_EXTENT)];
    let view = ViewSettings { resolution_px: SWEEP_CANVAS_PX, ..ViewSettings::default() };
    render_canvas(&scene, &simulation, &preview_view(bounds, view.zoom, view.resolution_px), &view, None);
    problems.sort();
    problems.dedup();
    problems
}

#[test]
#[ignore = "needs the packed client effects in /home/claudio/test/uddp"]
fn real_package_every_entry_previews() {
    for name in PACKAGES {
        let path = package_path(name);
        let reader = UddpReader::load(&path).expect("package container");
        let viewer = EffectsViewer::open(&path, &reader).expect("an effects package");
        assert!(viewer.open_error().is_none(), "{name}: {:?}", viewer.open_error());
        let (package, reader) = open(name);
        let model = EffectsModel::build(&package, &reader);
        let mut cache = ResourceCache::default();
        let mut problems = Vec::new();
        let mut counts = SweepCounts::default();
        let rows: Vec<&RootRow> = model.collection.iter().chain(&model.tileart).chain(&model.frames).collect();
        for row in &rows {
            problems.extend(sweep_root(&package, &mut cache, row.key, &mut counts));
        }
        for system in 0..package.systems().len() as u32 {
            let scene = PreviewScene::build(&package, &mut cache, PreviewTarget::System(system), &SimulationSettings::default());
            let mut simulation = PreviewSimulation::new(&scene);
            simulation.advance_to(SWEEP_STEPS * super::preview::PREVIEW_STEP_MS);
            if let Some(PartStatus::Unreadable { name, error }) = scene.parts.first().map(|part| &part.status) {
                problems.push(format!("system {name}: {error}"));
            }
        }
        let overview = &model.overview;
        println!(
            "{name}: {} collection, {} tiles, {} frames, {} systems ({} missing, {} failed), {} textures, \
             {} unreferenced systems, {} unreferenced textures; swept {} root lists: {} playing parts, {} particles, \
             {} texture draws; {} problems",
            overview.collection_entries,
            overview.tiles,
            overview.sequence_frames,
            overview.systems,
            overview.missing,
            overview.compile_failed,
            overview.textures,
            overview.unreferenced_systems.len(),
            overview.unreferenced_textures.len(),
            rows.len(),
            counts.playing_parts,
            counts.particles,
            counts.textures_drawn,
            problems.len()
        );
        for problem in problems.iter().take(40) {
            println!("  {problem}");
        }
        assert!(problems.is_empty(), "{name}: {} problems", problems.len());
    }
}

/// The first root list among `rows` whose particles name a system matching `wanted`.
fn find_row<'a>(
    package: &EffectsEcPackage,
    rows: impl IntoIterator<Item = &'a RootRow>,
    wanted: impl Fn(u32) -> bool,
) -> Option<RootKey> {
    rows.into_iter().find(|row| row.systems.iter().any(|&system| wanted(system)) && !row.too_deep).map(|row| row.key)
        .filter(|key| !key.roots(package).is_empty())
}

/// Whether a root list has a hued NIF particle.
fn has_hued_nif(package: &EffectsEcPackage, key: RootKey) -> bool {
    let mut hued = false;
    walk_nodes(package, key.roots(package), &mut |_, node| {
        if let EffectNode::Particle(particle) = node {
            hued |= particle.hued
                && particle.system.is_some_and(|system| {
                    let record = package.system(system);
                    record.kind == SYSTEM_KIND_NIF && record.has_payload()
                });
        }
    });
    hued
}

/// Renders `key` at [`STRIP_TIMES_MS`] side by side into `<RENDER_DIR>/<file>.png`.
fn render_strip(
    package: &EffectsEcPackage,
    key: RootKey,
    settings: &SimulationSettings,
    atlas: Option<&HueAtlas>,
    zoom: f32,
    file: &str,
) -> PathBuf {
    let scene = PreviewScene::build(package, &mut ResourceCache::default(), PreviewTarget::Root(key), settings);
    let view = ViewSettings { zoom, ..ViewSettings::default() };
    let ortho = preview_view(fit_bounds(&scene), view.zoom, view.resolution_px);
    let mut strip = image::RgbaImage::new((ortho.width * STRIP_TIMES_MS.len()) as u32, ortho.height as u32);
    let mut simulation = PreviewSimulation::new(&scene);
    for (column, time) in STRIP_TIMES_MS.into_iter().enumerate() {
        simulation.advance_to(time);
        let canvas = render_canvas(&scene, &simulation, &ortho, &view, atlas.map(|atlas| atlas.texture.as_ref()));
        let rgba = canvas.to_rgba8();
        for y in 0..ortho.height {
            for x in 0..ortho.width {
                let pixel = &rgba[(y * ortho.width + x) * RGBA..][..RGBA];
                strip.put_pixel((column * ortho.width + x) as u32, y as u32, image::Rgba(pixel.try_into().unwrap()));
            }
        }
    }
    let path = Path::new(RENDER_DIR).join(format!("{file}.png"));
    strip.save(&path).expect("write png");
    let parts: Vec<String> = scene
        .parts
        .iter()
        .map(|part| {
            let meshes: Vec<String> = part.meshes.iter().map(|mesh| format!("MESH {} {:?}", mesh.name, mesh.state)).collect();
            format!("{} {}", part.status.describe(), meshes.join(" "))
        })
        .collect();
    println!("{}: {} (hue {}): {}", path.display(), key.label(), settings.hue, parts.join(" | "));
    path
}

#[test]
#[ignore = "needs the packed client effects in /home/claudio/test/uddp; writes PNGs"]
fn real_package_reference_renders() {
    std::fs::create_dir_all(RENDER_DIR).expect("render dir");
    let atlas = load_hues_package(&package_path(HUES_PACKAGE)).ok();
    for name in PACKAGES {
        let (package, reader) = open(name);
        let model = EffectsModel::build(&package, &reader);
        let stem = name.trim_end_matches(".uddp");
        // A Chaos system named `needle` with a MESH sequence (a `.nif` dependency).
        let named = |needle: &'static str| {
            let package = &package;
            move |system: u32| {
                package.system_name(system).to_ascii_lowercase().contains(needle)
                    && package.system_dependencies(system).iter().any(|dep| dep & DEP_SYSTEM_BIT != 0)
            }
        };
        let meshes: Vec<&str> = (0..package.systems().len() as u32)
            .filter(|&system| package.system_dependencies(system).iter().any(|dep| dep & DEP_SYSTEM_BIT != 0))
            .map(|system| package.system_name(system))
            .collect();
        println!("{stem}: Chaos systems with MESH sequences: {}", meshes.join(", "));
        let roots = || model.collection.iter().chain(&model.tileart);
        let settings = SimulationSettings::default();

        // Only NIF particles that play.
        let nif_only = model.collection.iter().find(|row| {
            !row.systems.is_empty()
                && row.systems.iter().all(|&system| {
                    let record = package.system(system);
                    record.kind == SYSTEM_KIND_NIF && record.has_payload()
                })
        });
        if let Some(row) = nif_only {
            render_strip(&package, row.key, &settings, None, 1.0, &format!("{stem}_nif_only"));
        }
        for (needle, file) in [("firepit01", "firepit01"), ("lightning", "lightning")] {
            match find_row(&package, roots(), named(needle)) {
                Some(key) => {
                    let settings = SimulationSettings { play_projectile_only: true, ..settings };
                    render_strip(&package, key, &settings, None, 1.0, &format!("{stem}_{file}"));
                }
                None => println!("{stem}: no root list plays {needle}"),
            }
        }
        match find_row(&package, roots(), |system| package.system_name(system).contains("magicreflection")) {
            Some(key) => {
                render_strip(&package, key, &settings, None, 1.0, &format!("{stem}_magicreflection"));
            }
            None => println!("{stem}: no root list plays magicreflection"),
        }
        // A tileart effect with a hued NIF particle, played with a hue through the atlas.
        // Prefer one switching on the hue; the hued tileart particles may all be attach mode 2 (thrown items),
        // which the preview plays on request.
        let hued_rows: Vec<&RootRow> = model.tileart.iter().filter(|row| has_hued_nif(&package, row.key)).collect();
        let cases: Vec<String> = model.tileart.iter().filter(|row| !row.hue_cases.is_empty()).map(|row| row.key.label()).collect();
        println!("{stem}: {} tileart with hued NIFs; hue-keyed tileart: {}", hued_rows.len(), cases.join(", "));
        match hued_rows.iter().find(|row| !row.hue_cases.is_empty()).or(hued_rows.first()) {
            Some(row) => {
                let hue = row.hue_cases.iter().copied().find(|&case| case != 0).unwrap_or(DEMO_HUE);
                let settings = SimulationSettings { hue, play_projectile_only: true, ..settings };
                render_strip(&package, row.key, &settings, atlas.as_ref(), HUED_STRIP_ZOOM, &format!("{stem}_hued_tileart"));
            }
            None => println!("{stem}: no tileart effect with a hued NIF particle"),
        }
        // A tileart effect whose PropertyFlag picks its particles by the object hue.
        if let Some(row) = model.tileart.iter().find(|row| row.hue_cases.iter().any(|&case| case != 0)) {
            let hue = row.hue_cases.iter().copied().find(|&case| case != 0).unwrap_or(DEMO_HUE);
            let settings = SimulationSettings { hue, ..settings };
            render_strip(&package, row.key, &settings, atlas.as_ref(), 1.0, &format!("{stem}_hue_case_tileart"));
        }
    }
}
