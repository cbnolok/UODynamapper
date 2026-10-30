//! Compiles and simulates every NIF effect of a real Enhanced Client (not in the repository).
//!
//! Source: `UO_EC_CLIENT_DIR`, an Enhanced Client directory; the effects are the NIF payloads of its
//! `EffectTexture.uop`.
//!
//! Run: `UO_EC_CLIENT_DIR=... cargo test -p particle-sim --test nif_real_files -- --ignored --nocapture`

use std::collections::BTreeMap;
use std::sync::Arc;

use gamebryo_nif::NifFile;
use particle_sim::codec::{decode_nif_desc, encode_nif_desc};
use particle_sim::nif::desc::NifLightKind;
use particle_sim::nif::{NifEffect, NifEffectInstance, compile};
use particle_sim::render::{QuadVertex, RenderFrame, SceneLighting, expand_frame, expand_mesh_draw};
use particle_sim::{NifEffectDesc, NodeTransform, ParticleSystemInstance};
use uocf::enhanced::effect_texture::{EffectFileKind, EffectTexturePackage};

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";

/// NIF files in the surveyed `EffectTexture.uop`.
const EXPECTED_NIFS: usize = 466;
/// The fixed seed of every run.
const SEED: u32 = 0x5EED;
/// Frame step: about 30 frames per second.
const FRAME_MS: u32 = 33;
/// Shortest simulated time of a run.
const MIN_RUN_SECONDS: f32 = 5.0;
/// Extra time of the looping run past one duration, so the loop wraps.
const LOOP_EXTRA_SECONDS: f32 = 1.0;
const MS_PER_SECOND: f32 = 1000.0;
/// The only shipped effect with lights: two `NiDirectionalLight`s in its root's effect list, lighting its two
/// meshes.
const LIT_EFFECT: &str = "00005540_shadowstrike.nif";
const LIT_EFFECT_LIGHTS: usize = 2;

#[derive(Debug, Default, Clone, Copy)]
struct RunStats {
    max_particles: usize,
    max_sprites: usize,
    max_mesh_draws: usize,
    frames: usize,
}

fn updates(seconds: f32) -> usize {
    (seconds * MS_PER_SECOND / FRAME_MS as f32).ceil() as usize
}

/// Runs an effect, checking every frame; returns the peaks.
fn simulate(name: &str, effect: &Arc<NifEffect>, duration: f32, looping: bool, run_seconds: f32) -> RunStats {
    let desc = effect.desc();
    let mut instance = NifEffectInstance::with_seed(Arc::clone(effect), SEED);
    let camera = *instance.camera();
    let lighting = SceneLighting::default();
    let node = NodeTransform::default();
    let mut frame = RenderFrame::default();
    let mut vertices: Vec<QuadVertex> = Vec::new();
    let mut stats = RunStats::default();
    instance.start(duration, looping);
    for _ in 0..updates(run_seconds) {
        instance.update(FRAME_MS, &node);
        stats.frames += 1;
        for (index, system) in desc.particle_systems.iter().enumerate() {
            let count = instance.particles(index).len();
            assert!(count <= usize::from(system.capacity), "{name}: system {index} has {count} > {}", system.capacity);
            for particle in instance.particles(index) {
                assert!(
                    particle.position.is_finite() && particle.velocity.is_finite() && particle.size.is_finite(),
                    "{name}: non-finite particle {particle:?}"
                );
            }
        }
        frame.clear();
        instance.render(&mut frame);
        for sprite in &frame.sprites {
            assert!(sprite.position.is_finite() && sprite.size.is_finite(), "{name}: non-finite sprite {sprite:?}");
            assert!(sprite.color.iter().all(|channel| channel.is_finite()), "{name}: colour {sprite:?}");
        }
        vertices.clear();
        expand_frame(&frame, &camera, &mut vertices);
        for draw in &frame.mesh_draws {
            assert!(draw.translation.is_finite() && draw.scale.is_finite(), "{name}: non-finite mesh {}", draw.name);
            expand_mesh_draw(draw, &lighting, &mut vertices);
        }
        for vertex in &vertices {
            assert!(
                vertex.position.is_finite() && vertex.uv.iter().chain(&vertex.color).all(|v| v.is_finite()),
                "{name}: {vertex:?}"
            );
        }
        stats.max_particles = stats.max_particles.max(instance.particle_count());
        stats.max_sprites = stats.max_sprites.max(frame.sprites.len());
        stats.max_mesh_draws = stats.max_mesh_draws.max(frame.mesh_draws.len());
        if instance.is_finished() {
            break;
        }
    }
    stats
}

/// The lit effect's lights are on its root and reach both of its meshes.
fn check_lit_effect(effect: &Arc<NifEffect>) {
    let desc = effect.desc();
    assert_eq!(desc.lights.len(), LIT_EFFECT_LIGHTS);
    assert!(desc.lights.iter().all(|light| light.kind == NifLightKind::Directional && light.on));
    let root = desc.root.expect("the lit effect has a scene root");
    assert_eq!(desc.nodes[root].lights, (0..LIT_EFFECT_LIGHTS).collect::<Vec<_>>());
    let instance = NifEffectInstance::with_seed(Arc::clone(effect), SEED);
    let mut frame = RenderFrame::default();
    instance.render(&mut frame);
    assert!(!frame.mesh_draws.is_empty());
    assert!(frame.mesh_draws.iter().all(|draw| draw.lights.len() == LIT_EFFECT_LIGHTS));
}

#[test]
#[ignore = "needs UO_EC_CLIENT_DIR"]
fn every_client_nif_compiles_and_simulates() {
    let Some(client_dir) = std::env::var_os(EC_CLIENT_DIR_ENV) else {
        eprintln!("{EC_CLIENT_DIR_ENV} is not set; skipped");
        return;
    };
    let package = EffectTexturePackage::load_with_names(&client_dir).expect("EffectTexture.uop loads");
    let mut effects: Vec<(String, NifEffectDesc)> = Vec::new();
    let mut warnings: BTreeMap<String, usize> = BTreeMap::new();
    for entry in package.entries() {
        if !entry.has_payload() {
            continue;
        }
        let payload = package.read_by_hash(entry.hash).expect("entry reads").expect("entry has a payload");
        if EffectFileKind::detect(&payload) != EffectFileKind::Nif {
            continue;
        }
        let name = entry.file_name().map_or_else(|| format!("{:016x}", entry.hash), str::to_owned);
        let nif = NifFile::read(&payload).unwrap_or_else(|error| panic!("{name} does not parse: {error}"));
        let output = compile(&nif, &name);
        let decoded = decode_nif_desc(&encode_nif_desc(&output.desc));
        assert_eq!(decoded.as_ref(), Ok(&output.desc), "{name}: codec round trip");
        for warning in output.warnings {
            *warnings.entry(warning).or_default() += 1;
        }
        effects.push((name, output.desc));
    }
    assert_eq!(effects.len(), EXPECTED_NIFS, "NIF files in the package");

    let mut systems = 0;
    let mut meshes = 0;
    let mut morphs = 0;
    let mut controllers = 0;
    let mut textures_embedded = 0;
    let mut no_root = 0;
    let mut lights = 0;
    let mut lit_effects = Vec::new();
    let mut never_drawn = Vec::new();
    let mut busiest: Vec<(usize, String)> = Vec::new();
    for (name, desc) in &effects {
        systems += desc.particle_systems.len();
        meshes += desc.meshes.len();
        morphs += desc.meshes.iter().filter(|mesh| mesh.morph.is_some()).count();
        controllers += desc.controllers.len();
        textures_embedded += desc
            .textures
            .iter()
            .filter(|texture| matches!(texture.source, particle_sim::nif::desc::NifTextureSource::Embedded { .. }))
            .count();
        no_root += usize::from(desc.root.is_none());
        lights += desc.lights.len();
        if !desc.lights.is_empty() {
            lit_effects.push(name.clone());
        }
        let length = desc.animation_length().max(MIN_RUN_SECONDS);
        let effect = Arc::new(NifEffect::prepare(desc.clone()));
        let looping = simulate(name, &effect, length, true, length + LOOP_EXTRA_SECONDS);
        let once = simulate(name, &effect, length, false, length + LOOP_EXTRA_SECONDS);
        let peak = looping.max_particles.max(once.max_particles);
        if looping.max_sprites == 0 && looping.max_mesh_draws == 0 {
            never_drawn.push(name.clone());
        }
        busiest.push((peak, name.clone()));
        if name == LIT_EFFECT {
            check_lit_effect(&effect);
        }
    }
    assert_eq!(lit_effects, [LIT_EFFECT], "effects with lights");
    busiest.sort_unstable_by(|a, b| b.cmp(a));
    println!("{} NIF effects: {systems} particle systems, {meshes} meshes ({morphs} morphed), {controllers} controllers, {textures_embedded} embedded textures, {no_root} without a scene root, {lights} lights in {lit_effects:?}", effects.len());
    println!("busiest (peak particles): {:?}", &busiest[..busiest.len().min(10)]);
    println!("{} effects never draw anything: {never_drawn:?}", never_drawn.len());
    println!("compile warnings:");
    for (warning, count) in &warnings {
        println!("  {count:4} {warning}");
    }
}
