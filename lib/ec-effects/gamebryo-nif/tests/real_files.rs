//! Round-trip test over the NIF files of a real Enhanced Client (not in the repository).
//!
//! Sources, first one set wins:
//! - `GAMEBRYO_NIF_DIR`: a directory of extracted files; every file starting with the Gamebryo
//!   header string is tested (other files are skipped).
//! - `UO_EC_CLIENT_DIR`: an Enhanced Client directory; every NIF in `EffectTexture.uop` is tested,
//!   and the file count and block type counts must match the survey of that package.
//!
//! - `UO_KR_CLIENT_DIR`: a Kingdom Reborn client directory, for `kr_nifs_round_trip_byte_exact` (every NIF of its
//!   `EffectTexture.uop`).
//!
//! Run: `UO_EC_CLIENT_DIR=... cargo test -p gamebryo-nif --test real_files -- --ignored --nocapture`

use std::collections::BTreeMap;
use std::path::Path;

use gamebryo_nif::NifFile;

const DIR_ENV: &str = "GAMEBRYO_NIF_DIR";
const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
/// A Kingdom Reborn client directory (`UOKR.exe`, 2008): every NIF of its `EffectTexture.uop` is tested.
const KR_CLIENT_DIR_ENV: &str = "UO_KR_CLIENT_DIR";
/// NIF payloads in the KR `EffectTexture.uop`.
const KR_EXPECTED_NIF_FILES: usize = 451;
/// Blocks in all of them (56 block types).
const KR_EXPECTED_TOTAL_BLOCKS: usize = 36041;
const EFFECT_TEXTURE_UOP: &str = "EffectTexture.uop";
const NIF_MAGIC: &[u8] = b"Gamebryo File Format";

/// NIF payloads in the surveyed `EffectTexture.uop`.
const EXPECTED_NIF_FILES: usize = 466;
/// Blocks in all of them.
const EXPECTED_TOTAL_BLOCKS: usize = 36215;
/// Per block type: (type, blocks, files using it), from the survey of `EffectTexture.uop`.
const EXPECTED_BLOCK_TYPES: &[(&str, usize, usize)] = &[
    ("NiNode", 3425, 462),
    ("NiFloatInterpolator", 2304, 453),
    ("NiStringExtraData", 1751, 449),
    ("NiFloatData", 1240, 267),
    ("NiBoolData", 1236, 374),
    ("NiBoolInterpolator", 1228, 374),
    ("NiPSysEmitterCtlr", 1086, 359),
    ("NiPSysUpdateCtlr", 1086, 359),
    ("NiPSysAgeDeathModifier", 1086, 359),
    ("NiPSysSpawnModifier", 1086, 359),
    ("NiPSysPositionModifier", 1086, 359),
    ("NiPSysBoundUpdateModifier", 1086, 359),
    ("NiParticleSystem", 1082, 358),
    ("NiPSysData", 1082, 358),
    ("NiSourceTexture", 1055, 461),
    ("NiMaterialProperty", 1050, 462),
    ("NiPSysGrowFadeModifier", 1041, 340),
    ("NiTexturingProperty", 991, 461),
    ("NiVertexColorProperty", 985, 462),
    ("NiPSysBoxEmitter", 975, 320),
    ("NiZBufferProperty", 913, 462),
    ("NiPSysColorModifier", 772, 282),
    ("NiColorData", 772, 282),
    ("NiPSysRotationModifier", 744, 261),
    ("NiTransformController", 710, 267),
    ("NiTransformInterpolator", 685, 259),
    ("NiTriShape", 671, 201),
    ("NiTriShapeData", 668, 201),
    ("NiTransformData", 650, 256),
    ("NiAlphaProperty", 640, 460),
    ("NiPSysGravityModifier", 473, 187),
    ("NiTextureTransformController", 361, 125),
    ("NiPosData", 237, 94),
    ("NiDitherProperty", 222, 219),
    ("NiMaterialColorController", 218, 83),
    ("NiPoint3Interpolator", 218, 83),
    ("NiPSysEmitterSpeedCtlr", 193, 81),
    ("NiVisController", 142, 32),
    ("NiGeomMorpherController", 129, 41),
    ("NiMorphData", 129, 41),
    ("NiAlphaController", 98, 56),
    ("NiBillboardNode", 93, 35),
    ("NiStencilProperty", 87, 84),
    ("NiPSysSphereEmitter", 56, 42),
    ("NiPSysEmitterLifeSpanCtlr", 52, 35),
    ("NiPSysResetOnLoopCtlr", 47, 19),
    ("NiPSysMeshEmitter", 45, 39),
    ("NiPSysEmitterInitialRadiusCtlr", 41, 31),
    ("NiPSysGravityStrengthCtlr", 26, 13),
    ("NiPathInterpolator", 25, 12),
    ("NiFloatExtraData", 18, 4),
    ("NiPSysInitialRotSpeedVarCtlr", 18, 10),
    ("NiPSysInitialRotSpeedCtlr", 18, 10),
    ("NiTextureEffect", 15, 3),
    ("NiPSysCylinderEmitter", 10, 9),
    ("NiPixelData", 9, 6),
    ("NiPSysModifierActiveCtlr", 8, 4),
    ("NiBoolTimelineInterpolator", 8, 4),
    ("NiBooleanExtraData", 6, 4),
    ("NiPSysDragModifier", 5, 2),
    ("NiMeshParticleSystem", 4, 3),
    ("NiMeshPSysData", 4, 3),
    ("NiPSysMeshUpdateModifier", 4, 3),
    ("NiPSysBombModifier", 3, 3),
    ("NiIntegersExtraData", 2, 2),
    ("NiSortAdjustNode", 2, 1),
    ("NiDirectionalLight", 2, 1),
    ("NiSpecularProperty", 1, 1),
];

fn collect_from_dir(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read GAMEBRYO_NIF_DIR") {
        let path = entry.expect("dir entry").path();
        if !path.is_file() {
            continue;
        }
        let data = std::fs::read(&path).expect("read file");
        if data.starts_with(NIF_MAGIC) {
            out.push((path.display().to_string(), data));
        }
    }
    out
}

fn collect_from_uop(client_dir: &Path) -> Vec<(String, Vec<u8>)> {
    let package = uocf::uop_container::package::UopPackage::load(client_dir.join(EFFECT_TEXTURE_UOP))
        .expect("load EffectTexture.uop");
    let mut out = Vec::new();
    for file in package.iter_files().filter(|f| f.has_size()) {
        let data = file.unpack().expect("unpack payload");
        if data.starts_with(NIF_MAGIC) {
            out.push((format!("{:016X}", file.filename_hash()), data));
        }
    }
    out
}

/// Reads, writes and link-checks every file; asserts that each writes back byte for byte. Returns the blocks per
/// type as (blocks, files using the type) and the block total.
fn round_trip(files: &[(String, Vec<u8>)]) -> (BTreeMap<&'static str, (usize, usize)>, usize) {
    let mut failures = Vec::new();
    let mut blocks_by_type: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    let mut total_blocks = 0;
    for (name, data) in files {
        let nif = match NifFile::read(data) {
            Ok(nif) => nif,
            Err(e) => {
                failures.push(format!("{name}: read: {e}"));
                continue;
            }
        };
        match nif.write() {
            Ok(out) if out == *data => {}
            Ok(out) => {
                let at = out.iter().zip(data).position(|(a, b)| a != b).unwrap_or(out.len().min(data.len()));
                failures.push(format!(
                    "{name}: write differs at byte {at:#x} ({} vs {} bytes)",
                    out.len(),
                    data.len()
                ));
            }
            Err(e) => failures.push(format!("{name}: write: {e}")),
        }
        let issues = nif.link_issues();
        if let Some(issue) = issues.first() {
            failures.push(format!("{name}: {} bad links, first {issue:?}", issues.len()));
        }
        let mut seen = Vec::new();
        for block in &nif.blocks {
            let entry = blocks_by_type.entry(block.type_name()).or_default();
            entry.0 += 1;
            if !seen.contains(&block.type_name()) {
                seen.push(block.type_name());
                entry.1 += 1;
            }
        }
        total_blocks += nif.blocks.len();
    }

    println!("{} NIF files, {} failures, {total_blocks} blocks", files.len(), failures.len());
    for (ty, (blocks, in_files)) in &blocks_by_type {
        println!("  {ty:<40} {blocks:>6} {in_files:>5}");
    }
    for failure in failures.iter().take(20) {
        println!("FAIL {failure}");
    }
    assert!(failures.is_empty(), "{} of {} files failed", failures.len(), files.len());

    (blocks_by_type, total_blocks)
}

#[test]
#[ignore = "needs real NIF files: set UO_EC_CLIENT_DIR (or GAMEBRYO_NIF_DIR) and run with --ignored"]
fn real_nifs_round_trip_byte_exact() {
    let (files, from_package) = if let Some(dir) = std::env::var_os(DIR_ENV) {
        (collect_from_dir(Path::new(&dir)), false)
    } else if let Some(dir) = std::env::var_os(EC_CLIENT_DIR_ENV) {
        (collect_from_uop(Path::new(&dir)), true)
    } else {
        panic!("set {EC_CLIENT_DIR_ENV} or {DIR_ENV}");
    };
    assert!(!files.is_empty(), "no NIF files found");

    let (blocks_by_type, total_blocks) = round_trip(&files);

    if from_package {
        assert_eq!(files.len(), EXPECTED_NIF_FILES);
        assert_eq!(total_blocks, EXPECTED_TOTAL_BLOCKS);
        let expected: BTreeMap<&str, (usize, usize)> =
            EXPECTED_BLOCK_TYPES.iter().map(|&(ty, blocks, in_files)| (ty, (blocks, in_files))).collect();
        let found: BTreeMap<&str, (usize, usize)> = blocks_by_type.into_iter().collect();
        assert_eq!(found, expected);
    }
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored"]
fn kr_nifs_round_trip_byte_exact() {
    let dir = std::env::var_os(KR_CLIENT_DIR_ENV).unwrap_or_else(|| panic!("set {KR_CLIENT_DIR_ENV}"));
    let files = collect_from_uop(Path::new(&dir));
    let (blocks_by_type, total_blocks) = round_trip(&files);
    assert_eq!(files.len(), KR_EXPECTED_NIF_FILES);
    assert_eq!(total_blocks, KR_EXPECTED_TOTAL_BLOCKS);
    println!("{} block types", blocks_by_type.len());
    println!("KR: {} NIF files, {total_blocks} blocks round trip", files.len());
}
