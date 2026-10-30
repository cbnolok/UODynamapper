//! Checks `effects_ec.uddp` against the Enhanced Client files it is packed from: every definition entry
//! resolves to the same tree, every stored system decodes to what a fresh compile of its source gives, every
//! texture reads back with the size, format and mip count of its source, and decoded systems simulate exactly
//! as freshly compiled ones.

use std::path::PathBuf;
use std::sync::Arc;

use gamebryo_nif::NifFile;
use particle_sim::{
    ChaosEffectInstance, ChaosSystem, Mt19937, NifEffect, NifEffectInstance, NodeTransform, ParticleSystemInstance,
    RenderFrame, Vec3,
};
use udd_assets::effects_ec::{
    DefBranchRecord, EffectNode, EffectsEcPackage, SYSTEM_FLAG_DEFINITION_ROOT, SYSTEM_KIND_CHAOS, SYSTEM_KIND_NIF,
    TEXTURE_FLAG_AUTHORED_MIPS, TEXTURE_FLAG_EMBEDDED, TEXTURE_FORMAT_BC1, TEXTURE_FORMAT_BC3, TEXTURE_FORMAT_BC7,
    TEXTURE_FORMAT_RGBA8, TEXTURE_SOURCE_DDS, TEXTURE_SOURCE_NIF_PIXEL_DATA, TEXTURE_SOURCE_TGA,
    normalize_resource_name,
};
use udd_container::UddpReader;
use udd_conv::effects_ec::{
    EffectsEcPackOptions, compile_chaos_for_package, compile_nif_for_package, pack_effects_ec_to_bytes,
};
use uocf::enhanced::effect_definition::{
    EFFECT_DEFINITION_COLLECTION_UOP_NAME, EffectDefinition, EffectDefinitionCollection, EffectEntryKey,
    EffectMapEntry, PropertyFlagMode,
};
use uocf::enhanced::effect_texture::EffectTexturePackage;
use uocf::enhanced::string_dictionary::UoStringDictionary;

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
/// Path of a prebuilt package to check; when unset the package is packed in memory from the client.
const PACKAGE_ENV: &str = "UDD_EFFECTS_EC_UDDP";
const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";
/// Seed of both simulations of a system.
const SEED: u32 = 0x5EED;
/// Update step of the simulations, as the client's frame time.
const FRAME_MS: u32 = 33;
/// Updates simulated per system (about four seconds).
const FRAMES: usize = 120;
/// Effect duration passed to `start`.
const DURATION_S: f32 = 4.0;
/// Systems of each kind simulated: this many, spread evenly over the stored ones.
const SIMULATED_PER_KIND: usize = 8;
/// TGA header: width and height offsets.
const TGA_WIDTH_OFFSET: usize = 12;
const TGA_HEIGHT_OFFSET: usize = 14;
/// DDS header: height, width and mip count offsets, FourCC offset.
const DDS_HEIGHT_OFFSET: usize = 12;
const DDS_WIDTH_OFFSET: usize = 16;
const DDS_MIP_COUNT_OFFSET: usize = 28;
const DDS_FOURCC_OFFSET: usize = 84;
const DDS_FOURCC_DXT1: &[u8; 4] = b"DXT1";
const DDS_FOURCC_DXT5: &[u8; 4] = b"DXT5";

fn client_dir() -> Option<PathBuf> {
    let client = std::env::var_os(EC_CLIENT_DIR_ENV).map(PathBuf::from);
    if client.is_none() {
        println!("set {EC_CLIENT_DIR_ENV} (and optionally {PACKAGE_ENV}); skipped");
    }
    client
}

fn package(client: &PathBuf) -> EffectsEcPackage {
    match std::env::var_os(PACKAGE_ENV) {
        Some(path) => EffectsEcPackage::load(PathBuf::from(path)).unwrap(),
        None => {
            let (bytes, summary) = pack_effects_ec_to_bytes(&[client.clone()], &EffectsEcPackOptions::default()).unwrap();
            println!("packed in memory: {summary}");
            EffectsEcPackage::from_reader(UddpReader::open(bytes).unwrap()).unwrap()
        }
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u32 {
    u32::from(u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Checks that the package nodes of `list` are `definitions`.
fn assert_list(package: &EffectsEcPackage, strings: &UoStringDictionary, list: &[u32], definitions: &[EffectDefinition], at: &str) {
    assert_eq!(list.len(), definitions.len(), "{at}: list length");
    for (index, (&node, definition)) in list.iter().zip(definitions).enumerate() {
        assert_node(package, strings, node, definition, &format!("{at}[{index}]"));
    }
}

/// The client keeps the last of repeated map keys, by ascending key.
fn map_entries(entries: &[EffectMapEntry]) -> Vec<(u32, &[EffectDefinition])> {
    let map: std::collections::BTreeMap<u32, &[EffectDefinition]> =
        entries.iter().map(|entry| (entry.key, entry.children.as_slice())).collect();
    map.into_iter().collect()
}

fn assert_branches(
    package: &EffectsEcPackage,
    strings: &UoStringDictionary,
    branches: &[DefBranchRecord],
    expected: &[(u32, &[EffectDefinition])],
    at: &str,
) {
    assert_eq!(branches.len(), expected.len(), "{at}: branch count");
    for (branch, (key, children)) in branches.iter().zip(expected) {
        assert_eq!(branch.key, *key, "{at}: branch key");
        let list = package.list(branch.list_first, branch.list_count);
        assert_list(package, strings, list, children, &format!("{at}/{key:#x}"));
    }
}

fn assert_node(package: &EffectsEcPackage, strings: &UoStringDictionary, node: u32, definition: &EffectDefinition, at: &str) {
    match (package.node(node), definition) {
        (EffectNode::Particle(particle), EffectDefinition::Particle(source)) => {
            let name = source.name(strings).map(normalize_resource_name);
            let stored = particle.system.map(|system| package.system_name(system).to_owned());
            assert_eq!(stored, name, "{at}: system");
            assert_eq!(
                (particle.offset_x, particle.offset_y, particle.camera_offset, particle.scale, particle.duration),
                (source.offset_x, source.offset_y, source.camera_offset, source.scale, source.duration),
                "{at}: placement"
            );
            assert_eq!(particle.attach_mode, source.attach_mode, "{at}: attach mode");
        }
        (EffectNode::Sound { sound_id, value_08 }, EffectDefinition::Sound(source)) => {
            assert_eq!((sound_id, value_08.to_bits()), (source.sound_id, source.value_08.to_bits()), "{at}: sound");
        }
        (EffectNode::RoundRobin { children }, EffectDefinition::RoundRobin(source)) => {
            assert_list(package, strings, children, &source.children, at);
        }
        (EffectNode::PositionOffset { offsets }, EffectDefinition::PositionOffset(source)) => {
            let expected: Vec<(i8, i8)> = source.offsets.iter().map(|offset| (offset.x, offset.y)).collect();
            assert_eq!(offsets.to_vec(), expected, "{at}: offsets");
        }
        (EffectNode::Wait { children, min_delay_ms, max_delay_ms, aligned }, EffectDefinition::Wait(source)) => {
            assert_eq!(
                (min_delay_ms, max_delay_ms, aligned),
                (source.min_delay_ms, source.max_delay_ms, source.aligned),
                "{at}: wait"
            );
            assert_list(package, strings, children, &source.children, at);
        }
        (EffectNode::Lod { levels }, EffectDefinition::Lod(source)) => {
            assert_branches(package, strings, levels, &map_entries(&source.levels), at);
        }
        (
            EffectNode::PropertyFlag { property_mode, mask, property, default_children, cases },
            EffectDefinition::PropertyFlag(source),
        ) => {
            assert_eq!(property_mode, source.mode == PropertyFlagMode::Property, "{at}: mode");
            assert_eq!((mask, property), (source.mask, source.property), "{at}: mask, property");
            assert_list(package, strings, default_children, &source.default_children, &format!("{at}/default"));
            assert_branches(package, strings, cases, &map_entries(&source.cases), at);
        }
        (EffectNode::Earthquake { amplitude, duration, frequency }, EffectDefinition::Earthquake(source)) => {
            assert_eq!(
                (amplitude.to_bits(), duration.to_bits(), frequency.to_bits()),
                (source.amplitude.to_bits(), source.duration.to_bits(), source.frequency.to_bits()),
                "{at}: earthquake"
            );
        }
        (EffectNode::Random { entries }, EffectDefinition::Random(source)) => {
            let expected: Vec<(u32, &[EffectDefinition])> =
                source.entries.iter().map(|entry| (entry.weight.to_bits(), entry.children.as_slice())).collect();
            assert_branches(package, strings, entries, &expected, at);
        }
        (EffectNode::UserPreference { mask, children }, EffectDefinition::UserPreference(source)) => {
            assert_eq!(mask, source.mask, "{at}: mask");
            assert_list(package, strings, children, &source.children, at);
        }
        (stored, _) => panic!("{at}: stored {stored:?}, source type key {}", definition.type_key()),
    }
}

/// Walks a stored tree: every node and system index resolves.
fn walk(package: &EffectsEcPackage, list: &[u32], systems: &mut usize) {
    for &node in list {
        match package.node(node) {
            EffectNode::Particle(particle) => {
                let system = particle.system.expect("particle nodes of stored trees name a system");
                assert!(package.system(system).flags & SYSTEM_FLAG_DEFINITION_ROOT != 0);
                *systems += 1;
            }
            EffectNode::RoundRobin { children }
            | EffectNode::Wait { children, .. }
            | EffectNode::UserPreference { children, .. } => walk(package, children, systems),
            EffectNode::Lod { levels: branches } | EffectNode::Random { entries: branches } => {
                for branch in branches {
                    walk(package, package.list(branch.list_first, branch.list_count), systems);
                }
            }
            EffectNode::PropertyFlag { default_children, cases, .. } => {
                walk(package, default_children, systems);
                for branch in cases {
                    walk(package, package.list(branch.list_first, branch.list_count), systems);
                }
            }
            EffectNode::Sound { .. } | EffectNode::PositionOffset { .. } | EffectNode::Earthquake { .. } => {}
        }
    }
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR (and optionally UDD_EFFECTS_EC_UDDP), run with --ignored"]
fn real_package_definitions_resolve_to_the_client_trees() {
    let Some(client) = client_dir() else { return };
    let package = package(&client);
    let strings = UoStringDictionary::load(&client.join(STRING_DICTIONARY_UOP_NAME)).unwrap();
    let collection = EffectDefinitionCollection::load(&client.join(EFFECT_DEFINITION_COLLECTION_UOP_NAME)).unwrap();

    let mut compared = 0;
    for entry in &collection.entries {
        let EffectEntryKey::Id(id) = entry.key else { continue };
        let roots = package.collection_roots(id).unwrap_or_else(|| panic!("collection entry {id} missing"));
        assert_list(&package, &strings, roots, &entry.definitions, &format!("collection {id}"));
        compared += 1;
    }
    assert_eq!(compared, package.collection().len(), "stored collection entries");

    let mut particle_nodes = 0;
    for record in package.tileart() {
        let roots = package.tileart_roots(record.tile_id).expect("tile resolves");
        assert!(!roots.is_empty(), "tile {} stores no definitions", record.tile_id);
        walk(&package, roots, &mut particle_nodes);
    }
    for record in package.sequence_frames() {
        let roots = package.frame_roots(record.sequence_id, record.action_id, record.frame).expect("frame resolves");
        assert!(!roots.is_empty(), "sequence frame {record:?} stores no definitions");
        walk(&package, roots, &mut particle_nodes);
    }
    println!(
        "{compared} collection entries equal the client trees; {} tiles and {} sequence frames resolve \
         ({particle_nodes} particle nodes)",
        package.tileart().len(),
        package.sequence_frames().len()
    );
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR (and optionally UDD_EFFECTS_EC_UDDP), run with --ignored"]
fn real_package_systems_and_textures_match_their_sources() {
    let Some(client) = client_dir() else { return };
    let package = package(&client);
    let files = EffectTexturePackage::load_with_names(&client).unwrap();

    let (mut chaos, mut nif) = (Vec::new(), Vec::new());
    for index in 0..package.systems().len() as u32 {
        let record = *package.system(index);
        if !record.has_payload() {
            continue;
        }
        let name = package.system_name(index);
        let source = files.read(name).unwrap().unwrap_or_else(|| panic!("{name}: stored but not in the client"));
        match record.kind {
            SYSTEM_KIND_CHAOS => {
                let fresh = compile_chaos_for_package(&source).unwrap_or_else(|errors| panic!("{name}: {errors:?}")).0;
                let decoded = package.read_chaos_desc(index).unwrap();
                assert_eq!(decoded, fresh, "{name}: decoded desc differs from a fresh compile");
                chaos.push((name.to_owned(), decoded, fresh));
            }
            SYSTEM_KIND_NIF => {
                let fresh = compile_nif_for_package(&NifFile::read(&source).unwrap(), name).desc;
                let decoded = package.read_nif_desc(index).unwrap();
                assert_eq!(decoded, fresh, "{name}: decoded desc differs from a fresh compile");
                nif.push((name.to_owned(), decoded, fresh));
            }
            kind => panic!("{name}: kind {kind}"),
        }
    }

    let mut textures = 0;
    for index in 0..package.textures().len() as u32 {
        let record = *package.texture(index);
        if !record.has_payload() {
            continue;
        }
        let name = package.texture_name(index);
        let texture = package.read_texture(index).unwrap();
        assert_eq!(
            (texture.width, texture.height, texture.format, texture.mip_count),
            (u32::from(record.width), u32::from(record.height), record.format, u32::from(record.mip_count)),
            "{name}: payload and record"
        );
        let full_chain = texture.width.max(texture.height).ilog2() + 1;
        assert!(
            texture.mip_count == 1 || texture.mip_count == full_chain || record.flags & TEXTURE_FLAG_AUTHORED_MIPS != 0,
            "{name}: {} mips of {}x{}",
            texture.mip_count,
            texture.width,
            texture.height
        );
        match record.source {
            TEXTURE_SOURCE_TGA => {
                let source = files.read(name).unwrap().expect("TGA source");
                let size = (read_u16(&source, TGA_WIDTH_OFFSET), read_u16(&source, TGA_HEIGHT_OFFSET));
                assert_eq!((texture.width, texture.height), size, "{name}: size");
                assert!(matches!(texture.format, TEXTURE_FORMAT_RGBA8 | TEXTURE_FORMAT_BC7), "{name}: format");
            }
            TEXTURE_SOURCE_DDS => {
                let source = files.read(name).unwrap().expect("DDS source");
                let size = (read_u32(&source, DDS_WIDTH_OFFSET), read_u32(&source, DDS_HEIGHT_OFFSET));
                assert_eq!((texture.width, texture.height), size, "{name}: size");
                let fourcc = &source[DDS_FOURCC_OFFSET..DDS_FOURCC_OFFSET + 4];
                let expected_format = match fourcc {
                    _ if texture.width % 4 != 0 || texture.height % 4 != 0 => texture.format,
                    f if f == DDS_FOURCC_DXT1 => TEXTURE_FORMAT_BC1,
                    f if f == DDS_FOURCC_DXT5 => TEXTURE_FORMAT_BC3,
                    _ => texture.format,
                };
                assert_eq!(texture.format, expected_format, "{name}: format of a {fourcc:?} source");
                if record.flags & TEXTURE_FLAG_AUTHORED_MIPS != 0 {
                    assert_eq!(texture.mip_count, read_u32(&source, DDS_MIP_COUNT_OFFSET), "{name}: authored mips");
                }
            }
            TEXTURE_SOURCE_NIF_PIXEL_DATA => assert!(record.flags & TEXTURE_FLAG_EMBEDDED != 0, "{name}: embedded"),
            source => panic!("{name}: source {source}"),
        }
        assert!(matches!(
            texture.format,
            TEXTURE_FORMAT_RGBA8 | TEXTURE_FORMAT_BC7 | TEXTURE_FORMAT_BC1 | TEXTURE_FORMAT_BC3
        ));
        textures += 1;
    }

    let node = |frame: usize| {
        let angle = (frame as u32 * FRAME_MS) as f32 / 1000.0;
        NodeTransform::at(Vec3::new(angle.cos(), angle.sin(), 0.0))
    };
    let run = |mut instance: Box<dyn ParticleSystemInstance>| {
        let mut frames = Vec::with_capacity(FRAMES);
        instance.start(DURATION_S, false);
        for frame in 0..FRAMES {
            instance.update(FRAME_MS, &node(frame));
            let mut out = RenderFrame::default();
            instance.render(&mut out);
            frames.push(format!("{:?}", (out.batches, out.sprites, out.meshes, out.mesh_draws, out.batch_states)));
        }
        frames
    };
    let spread = |count: usize| (0..count).step_by((count / SIMULATED_PER_KIND).max(1)).take(SIMULATED_PER_KIND);
    for index in spread(chaos.len()) {
        let (name, decoded, fresh) = &chaos[index];
        let start = |desc: &particle_sim::ChaosSystemDesc| -> Box<dyn ParticleSystemInstance> {
            let system = Arc::new(ChaosSystem::prepare(desc.clone(), &mut Mt19937::new(SEED)));
            Box::new(ChaosEffectInstance::with_seed(system, SEED))
        };
        assert_eq!(run(start(decoded)), run(start(fresh)), "{name}: simulated frames differ");
    }
    for index in spread(nif.len()) {
        let (name, decoded, fresh) = &nif[index];
        let start = |desc: &particle_sim::NifEffectDesc| -> Box<dyn ParticleSystemInstance> {
            Box::new(NifEffectInstance::with_seed(Arc::new(NifEffect::prepare(desc.clone())), SEED))
        };
        assert_eq!(run(start(decoded)), run(start(fresh)), "{name}: simulated frames differ");
    }
    println!(
        "{} Chaos and {} NIF systems equal fresh compiles; {textures} textures match their sources; {} systems simulated",
        chaos.len(),
        nif.len(),
        spread(chaos.len()).count() + spread(nif.len()).count()
    );
}
