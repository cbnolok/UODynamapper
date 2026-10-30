//! Compiles and simulates every Chaos script of a real Enhanced Client (not in the repository).
//!
//! Source: `UO_EC_CLIENT_DIR`, an Enhanced Client directory; the scripts are the `particlesystem` payloads of its
//! `EffectTexture.uop`. The counts are those of `scripts/dev-helpers/ec-research/ghidra-effects/verify_chaos.py`
//! on the same package.
//!
//! Run: `UO_EC_CLIENT_DIR=... cargo test -p particle-sim --test real_files -- --ignored --nocapture`

use std::collections::BTreeMap;
use std::sync::Arc;

use particle_sim::chaos::compile::compile_chaos_source;
use particle_sim::chaos::lexer_check::check_client_lexer;
use particle_sim::codec::{decode_chaos_desc, encode_chaos_desc};
use particle_sim::chaos::{ChaosIssueKind, ChaosSystem, ChaosSystemDesc};
use particle_sim::render::{CameraFrame, QuadVertex, RenderFrame, expand_frame};
use particle_sim::{ChaosEffectInstance, NodeTransform, ParticleSystemInstance, Vec3};
use uocf::enhanced::effect_texture::{EffectFileKind, EffectTexturePackage};

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";

/// Chaos scripts in the surveyed `EffectTexture.uop`.
const EXPECTED_SCRIPTS: usize = 61;
/// Event sequences in all of them.
const EXPECTED_SEQUENCES: usize = 135;
/// `at random(a, b)` event lines.
const EXPECTED_RANDOM_EVENT_TIMES: usize = 91;
/// `emitRate = random(a, b)` lines.
const EXPECTED_EMIT_RATE_RANGES: usize = 9;
/// `burst = random(a, b)` lines.
const EXPECTED_BURST_RANGES: usize = 3;
/// `random(a, b)` with `a > b`.
const EXPECTED_REVERSED_RANDOMS: usize = 3;
/// Fades whose step persists past their target.
const EXPECTED_PERSISTING_FADES: usize = 1;

/// The fixed seed of every run.
const SEED: u32 = 0x5EED;
/// Frame step: about 30 frames per second.
const FRAME_MS: u32 = 33;
/// Emission time of the looping runs (the definition duration).
const LOOP_DURATION_S: f32 = 4.0;
/// Simulated time of each run before Stop.
const RUN_SECONDS: f32 = 8.0;
/// Most simulated time after Stop before the last particle must have died.
const DRAIN_LIMIT_SECONDS: f32 = 120.0;
const MS_PER_SECOND: f32 = 1000.0;

/// A camera 45° above the horizon looking north (+y), 100 units from the origin.
fn camera() -> CameraFrame {
    const DISTANCE: f32 = 100.0;
    let half = std::f32::consts::FRAC_1_SQRT_2;
    let direction = Vec3::new(0.0, half, -half);
    let up = Vec3::new(0.0, half, half);
    CameraFrame { position: direction * -DISTANCE, direction, up, right: direction.cross(up) }
}

#[derive(Debug, Default, Clone, Copy)]
struct RunStats {
    max_particles: usize,
    max_sprites: usize,
    max_meshes: usize,
    drain_seconds: f32,
}

fn updates(seconds: f32) -> usize {
    (seconds * MS_PER_SECOND / FRAME_MS as f32).ceil() as usize
}

/// Runs a script and checks every frame; returns the peaks.
fn simulate(name: &str, system: &Arc<ChaosSystem>, looping: bool) -> RunStats {
    let desc = system.desc();
    let mut effect = ChaosEffectInstance::with_seed(Arc::clone(system), SEED);
    let camera = camera();
    let mut frame = RenderFrame::default();
    let mut vertices: Vec<QuadVertex> = Vec::new();
    let mut stats = RunStats::default();
    effect.start(LOOP_DURATION_S, looping);
    let mut step = |effect: &mut ChaosEffectInstance, index: usize| {
        // A slow circle, so emission boost, inherited velocity and axial quads see movement.
        let angle = index as f32 * FRAME_MS as f32 / MS_PER_SECOND;
        let node = NodeTransform::at(Vec3::new(angle.cos(), angle.sin(), 0.0));
        effect.update(FRAME_MS, &node);
        for (sequence, sequence_desc) in desc.sequences.iter().enumerate() {
            let count = effect.emitter().particles(sequence).len();
            assert!(count <= usize::from(sequence_desc.num_particles), "{name}: sequence {sequence} over its cap");
        }
        frame.clear();
        effect.render(&mut frame);
        vertices.clear();
        expand_frame(&frame, &camera, &mut vertices);
        for sprite in &frame.sprites {
            assert!(sprite.position.is_finite() && sprite.size.is_finite(), "{name}: non-finite sprite {sprite:?}");
            assert!(sprite.color.iter().all(|channel| (0.0..=1.0).contains(channel)), "{name}: colour {sprite:?}");
        }
        for vertex in &vertices {
            assert!(vertex.position.is_finite() && vertex.uv.iter().all(|uv| uv.is_finite()), "{name}: {vertex:?}");
        }
        for mesh in &frame.meshes {
            assert!(mesh.translation.is_finite() && mesh.scale.is_finite(), "{name}: non-finite mesh {mesh:?}");
        }
        stats.max_particles = stats.max_particles.max(effect.particle_count());
        stats.max_sprites = stats.max_sprites.max(frame.sprites.len());
        stats.max_meshes = stats.max_meshes.max(frame.meshes.len());
    };
    let run_updates = updates(RUN_SECONDS);
    for index in 0..run_updates {
        step(&mut effect, index);
    }
    effect.stop();
    let mut drained = 0;
    while !effect.is_drained() {
        assert!(drained < updates(DRAIN_LIMIT_SECONDS), "{name}: particles still alive {DRAIN_LIMIT_SECONDS} s after Stop");
        step(&mut effect, run_updates + drained);
        drained += 1;
    }
    stats.drain_seconds = drained as f32 * FRAME_MS as f32 / MS_PER_SECOND;
    stats
}

#[test]
#[ignore = "needs an Enhanced Client directory in UO_EC_CLIENT_DIR"]
fn compile_and_simulate_every_chaos_script() {
    let Some(client_dir) = std::env::var_os(EC_CLIENT_DIR_ENV) else {
        eprintln!("{EC_CLIENT_DIR_ENV} is not set: skipped");
        return;
    };
    let package = EffectTexturePackage::load_with_names(&client_dir).expect("EffectTexture.uop loads");
    let mut scripts: Vec<(String, ChaosSystemDesc)> = Vec::new();
    let mut issue_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |kind: &ChaosIssueKind| {
        let key = format!("{kind:?}");
        let key = key.split([' ', '{', '(']).next().unwrap_or_default().to_owned();
        *issue_counts.entry(key).or_default() += 1;
    };
    let mut random_event_times = 0;
    let mut emit_rate_ranges = 0;
    let mut burst_ranges = 0;
    let mut reversed_randoms = 0;
    let mut persisting_fades = 0;
    for entry in package.entries() {
        if !entry.has_payload() {
            continue;
        }
        let payload = package.read_by_hash(entry.hash).expect("entry reads").expect("entry has a payload");
        if EffectFileKind::detect(&payload) != EffectFileKind::ChaosScript {
            continue;
        }
        let name = entry.file_name().map_or_else(|| format!("{:016x}", entry.hash), str::to_owned);
        let lexer_issues = check_client_lexer(&payload);
        assert!(lexer_issues.is_empty(), "{name}: {lexer_issues:?}");
        let report = compile_chaos_source(&payload);
        for issue in &report.issues {
            count(&issue.kind);
            match issue.kind {
                ChaosIssueKind::SharedRandomEventTime { .. } => random_event_times += 1,
                ChaosIssueKind::EmitRateRange { .. } => emit_rate_ranges += 1,
                ChaosIssueKind::BurstRange { .. } => burst_ranges += 1,
                ChaosIssueKind::ReversedRandom { .. } => reversed_randoms += 1,
                ChaosIssueKind::FadeStepPersists { .. } => persisting_fades += 1,
                _ => {}
            }
        }
        let errors: Vec<String> = report.errors().map(ToString::to_string).collect();
        let desc = report.desc.unwrap_or_else(|| panic!("{name} does not compile: {errors:?}"));
        assert_eq!(decode_chaos_desc(&encode_chaos_desc(&desc)).as_ref(), Ok(&desc), "{name}: codec round trip");
        scripts.push((name, desc));
    }
    scripts.sort_by(|a, b| a.0.cmp(&b.0));

    println!("issues: {issue_counts:#?}");
    assert_eq!(scripts.len(), EXPECTED_SCRIPTS);
    let sequences: usize = scripts.iter().map(|(_, desc)| desc.sequences.len()).sum();
    assert_eq!(sequences, EXPECTED_SEQUENCES);
    assert_eq!(random_event_times, EXPECTED_RANDOM_EVENT_TIMES);
    assert_eq!(emit_rate_ranges, EXPECTED_EMIT_RATE_RANGES);
    assert_eq!(burst_ranges, EXPECTED_BURST_RANGES);
    assert_eq!(reversed_randoms, EXPECTED_REVERSED_RANDOMS);
    assert_eq!(persisting_fades, EXPECTED_PERSISTING_FADES);

    println!(
        "{:<36} {:>4} | {:>6} {:>6} {:>5} {:>6} | {:>6} {:>6} {:>5} {:>6}",
        "script", "seqs", "L.part", "L.spr", "L.msh", "L.drain", "O.part", "O.spr", "O.msh", "O.drain"
    );
    for (name, desc) in &scripts {
        let mut rng = particle_sim::Mt19937::new(SEED);
        let system = Arc::new(ChaosSystem::prepare(desc.clone(), &mut rng));
        let looping = simulate(name, &system, true);
        let once = simulate(name, &system, false);
        println!(
            "{:<36} {:>4} | {:>6} {:>6} {:>5} {:>6.1} | {:>6} {:>6} {:>5} {:>6.1}",
            name,
            desc.sequences.len(),
            looping.max_particles,
            looping.max_sprites,
            looping.max_meshes,
            looping.drain_seconds,
            once.max_particles,
            once.max_sprites,
            once.max_meshes,
            once.drain_seconds
        );
    }
}
