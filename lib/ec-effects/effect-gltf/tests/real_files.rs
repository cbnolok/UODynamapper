//! Exports every NIF effect and every Chaos script of a real Enhanced Client (not in the repository) and checks
//! each `.glb`.
//!
//! Source: `UO_EC_CLIENT_DIR`, an Enhanced Client directory; the effects are the NIF and Chaos payloads of its
//! `EffectTexture.uop`, the textures its other files. With `EFFECT_GLTF_OUT_DIR` set, a few sample effects are
//! also written there, plain and with the particles baked, plus the first Chaos script with MESH particles.
//!
//! Run: `UO_EC_CLIENT_DIR=... [EFFECT_GLTF_OUT_DIR=...] cargo test -p effect-gltf --test real_files -- --ignored --nocapture`

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use effect_gltf::validate::validate_glb;
use effect_gltf::{BakeOptions, ChaosPart, EffectPart, ExportOptions, PartContent, export_effect, export_nif_file};
use particle_sim::ParticleEffectParams;
use particle_sim::chaos::ChaosParticleType;
use particle_sim::chaos::compile::compile_chaos_source;
use gamebryo_nif::NifFile;
use uocf::enhanced::effect_texture::{EffectFileKind, EffectTexturePackage};

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const OUT_DIR_ENV: &str = "EFFECT_GLTF_OUT_DIR";
/// NIF files in the surveyed `EffectTexture.uop`.
const EXPECTED_NIFS: usize = 466;
/// Effects written to the output directory.
const SAMPLES: [&str; 5] = [
    "00005036_magicreflection.nif",
    "splat_blue_sizeb.nif",
    "00005540_shadowstrike.nif",
    "00014036_largefireball.nif",
    "00005052_gate_travel.nif",
];
/// The baked samples' file name suffix.
const BAKED_SUFFIX: &str = ".baked.glb";
const PLAIN_SUFFIX: &str = ".glb";
/// Texture warnings start with this (they are listed in full).
const TEXTURE_WARNING_PREFIX: &str = "texture ";
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

/// A warning without the file name that starts it, for counting.
fn warning_kind(warning: &str) -> String {
    let message = warning.split_once(": ").map_or(warning, |(_, message)| message);
    message.split(|c: char| c.is_ascii_digit()).next().unwrap_or(message).trim().to_owned()
}

#[test]
#[ignore = "needs UO_EC_CLIENT_DIR"]
fn every_client_nif_exports_a_valid_glb() {
    let Some(client_dir) = std::env::var_os(EC_CLIENT_DIR_ENV) else {
        eprintln!("{EC_CLIENT_DIR_ENV} is not set; skipped");
        return;
    };
    let out_dir = std::env::var_os(OUT_DIR_ENV).map(PathBuf::from);
    let package = EffectTexturePackage::load_with_names(&client_dir).expect("EffectTexture.uop loads");
    let loader = |name: &str| package.read(name).ok().flatten();
    let options = ExportOptions::default();
    let started = Instant::now();
    let mut exported = 0;
    let mut totals = effect_gltf::validate::Summary::default();
    let mut largest = (0, String::new());
    let mut animated = 0;
    let mut warnings: BTreeMap<String, usize> = BTreeMap::new();
    let mut texture_warnings = Vec::new();
    let mut samples = Vec::new();
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
        let export = export_nif_file(&nif, &name, &loader, &options);
        let summary = validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{name}: {errors:#?}"));
        exported += 1;
        totals.nodes += summary.nodes;
        totals.meshes += summary.meshes;
        totals.materials += summary.materials;
        totals.images += summary.images;
        totals.animations += summary.animations;
        totals.channels += summary.channels;
        totals.bytes += summary.bytes;
        animated += usize::from(summary.animations > 0);
        if summary.bytes > largest.0 {
            largest = (summary.bytes, name.clone());
        }
        for warning in &export.warnings {
            *warnings.entry(warning_kind(warning)).or_default() += 1;
            if warning.starts_with(TEXTURE_WARNING_PREFIX) {
                texture_warnings.push(format!("{name}: {warning}"));
            }
        }
        if SAMPLES.iter().any(|sample| sample.eq_ignore_ascii_case(&name)) {
            samples.push((name, nif, export.bytes));
        }
    }
    let elapsed = started.elapsed();
    println!("{exported} NIFs exported and valid in {:.1} s", elapsed.as_secs_f64());
    println!(
        "{} nodes, {} meshes, {} materials, {} images, {animated} animated files, {} channels, {:.1} MiB (largest {:.2} MiB: {})",
        totals.nodes,
        totals.meshes,
        totals.materials,
        totals.images,
        totals.channels,
        totals.bytes as f64 / BYTES_PER_MIB,
        largest.0 as f64 / BYTES_PER_MIB,
        largest.1,
    );
    for (warning, count) in &warnings {
        println!("  {count:5} × {warning}");
    }
    for warning in &texture_warnings {
        println!("  {warning}");
    }
    assert_eq!(exported, EXPECTED_NIFS, "NIF files in the package");
    assert_eq!(samples.len(), SAMPLES.len(), "sample effects found");

    let bake = ExportOptions { bake: Some(BakeOptions::default()), ..ExportOptions::default() };
    for (name, nif, plain) in samples {
        let baked = export_nif_file(&nif, &name, &loader, &bake);
        let summary = validate_glb(&baked.bytes).unwrap_or_else(|errors| panic!("{name} baked: {errors:#?}"));
        println!(
            "{name}: plain {:.2} MiB, baked {:.2} MiB ({} nodes, {} meshes, {} channels)",
            plain.len() as f64 / BYTES_PER_MIB,
            summary.bytes as f64 / BYTES_PER_MIB,
            summary.nodes,
            summary.meshes,
            summary.channels,
        );
        if let Some(out_dir) = &out_dir {
            let stem = name.trim_end_matches(".nif");
            std::fs::create_dir_all(out_dir).expect("output directory");
            std::fs::write(out_dir.join(format!("{stem}{PLAIN_SUFFIX}")), &plain).expect("write");
            std::fs::write(out_dir.join(format!("{stem}{BAKED_SUFFIX}")), &baked.bytes).expect("write");
        }
    }
}

/// Seconds of a Chaos bake (the scripts have no length of their own).
const CHAOS_CLIP_SECONDS: f32 = 2.0;

#[test]
#[ignore = "needs UO_EC_CLIENT_DIR"]
fn every_client_chaos_script_exports_as_a_baked_effect() {
    let Some(client_dir) = std::env::var_os(EC_CLIENT_DIR_ENV) else {
        eprintln!("{EC_CLIENT_DIR_ENV} is not set; skipped");
        return;
    };
    let out_dir = std::env::var_os(OUT_DIR_ENV).map(PathBuf::from);
    let package = EffectTexturePackage::load_with_names(&client_dir).expect("EffectTexture.uop loads");
    let loader = |name: &str| package.read(name).ok().flatten();
    let options = ExportOptions {
        clip_length: Some(CHAOS_CLIP_SECONDS),
        bake: Some(BakeOptions::default()),
        ..ExportOptions::default()
    };
    let started = Instant::now();
    let (mut exported, mut rejected, mut with_meshes, mut bytes) = (0, 0, 0, 0);
    for entry in package.entries() {
        if !entry.has_payload() {
            continue;
        }
        let payload = package.read_by_hash(entry.hash).expect("entry reads").expect("entry has a payload");
        if EffectFileKind::detect(&payload) != EffectFileKind::ChaosScript {
            continue;
        }
        let name = entry.file_name().map_or_else(|| format!("{:016x}", entry.hash), str::to_owned);
        let Some(desc) = compile_chaos_source(&payload).desc else {
            rejected += 1;
            continue;
        };
        let mut meshes = Vec::new();
        for sequence in desc.sequences.iter().filter(|sequence| sequence.particle_type == ChaosParticleType::Mesh) {
            if let Some(mesh) = loader(&sequence.resource).and_then(|bytes| NifFile::read(&bytes).ok()) {
                meshes.push(particle_sim::nif::compile(&mesh, &sequence.resource).desc);
            }
        }
        // The first script with MESH particles is written to the output directory.
        let sample = !meshes.is_empty() && with_meshes == 0;
        with_meshes += usize::from(!meshes.is_empty());
        let source = Some(String::from_utf8_lossy(&payload).into_owned());
        let params = ParticleEffectParams { looping: true, ..ParticleEffectParams::default() };
        let parts = [EffectPart { label: name.clone(), params, content: PartContent::Chaos(ChaosPart { desc, source, meshes }) }];
        let export = export_effect(&name, &parts, &loader, &options);
        validate_glb(&export.bytes).unwrap_or_else(|errors| panic!("{name}: {errors:#?}"));
        exported += 1;
        bytes += export.bytes.len();
        if let Some(out_dir) = out_dir.as_ref().filter(|_| sample) {
            println!("sample: {name}");
            std::fs::create_dir_all(out_dir).expect("output directory");
            let stem = name.trim_end_matches(".ems");
            std::fs::write(out_dir.join(format!("{stem}{BAKED_SUFFIX}")), &export.bytes).expect("write");
        }
    }
    println!(
        "{exported} Chaos scripts exported and valid ({rejected} rejected by the compiler, {with_meshes} with MESH .nif files), \
         {:.1} MiB baked, in {:.1} s",
        bytes as f64 / BYTES_PER_MIB,
        started.elapsed().as_secs_f64()
    );
    assert!(exported > 0);
}
