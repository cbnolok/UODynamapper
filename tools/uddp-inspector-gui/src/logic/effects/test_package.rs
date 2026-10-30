//! A small synthetic `effects_ec.uddp` for the unit tests: every node type the preview resolves, a Chaos script
//! with a texture, a Chaos script with a MESH sequence, a hued NIF, a missing and a failed system, a missing and an
//! unreferenced texture, and a cyclic list.

use std::sync::Arc;

use particle_sim::NifEffectDesc;
use particle_sim::chaos::compile::compile_chaos_source;
use particle_sim::codec::{encode_chaos_desc, encode_nif_desc};
use particle_sim::nif::desc::{NifTexture, NifTextureSource};
use udd_assets::EffectsEcPackage;
use udd_assets::effects_ec::*;
use udd_container::{AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder, UddpReader};
use uocf::enhanced::effect_definition::{
    EFFECT_TYPE_LOD, EFFECT_TYPE_PARTICLE, EFFECT_TYPE_PROPERTY_FLAG, EFFECT_TYPE_RANDOM, EFFECT_TYPE_ROUND_ROBIN,
    EFFECT_TYPE_SOUND, PARTICLE_ATTACH_PROJECTILE_ONLY, RANDOM_WEIGHT_TOTAL,
};

/// The PropertyFlag case of the hued NIF.
pub const CASE_HUE: u32 = 5;
/// A sound id.
const SOUND_ID: u32 = 77;

/// One additive point sequence emitting from the start.
pub const SPARK_SCRIPT: &str = "particlesystem \"spark\" 1\n{\neventsequence \"a\"\n{\ntype = point\n\
    texture = \"spark.tga\"\nsourceBlendMode = ALPHA_SRCALPHA\ndestBlendMode = ALPHA_ONE\nlifetime = 1\n\
    emitRate = 40\nnumParticles = 64\ninitial scale = 1\ninitial velocity = XYZ(random(-1, 1), 0, 2)\n}\n}\n";
/// A MESH sequence drawing `smoke.nif`, and a point sequence on the missing `gone.tga`.
pub const MESH_SCRIPT: &str = "particlesystem \"mesh\" 1\n{\neventsequence \"a\"\n{\ntype = mesh\n\
    meshName = \"smoke.nif\"\nlifetime = 10\nburst = 1\n}\neventsequence \"b\"\n{\ntype = point\n\
    texture = \"gone.tga\"\nlifetime = 1\nemitRate = 10\nnumParticles = 8\ninitial scale = 1\n}\n}\n";

/// System indices (sorted by name).
pub mod systems {
    pub const LOST: u32 = 0;
    pub const MESH: u32 = 1;
    pub const ORPHAN: u32 = 2;
    pub const SMOKE: u32 = 3;
    pub const SPARK: u32 = 4;
}

/// Texture indices (sorted by name).
pub mod textures {
    pub const GONE: u32 = 0;
    pub const SMOKE_EMBEDDED: u32 = 1;
    pub const SPARK: u32 = 2;
    pub const UNUSED: u32 = 3;
}

/// Root keys.
pub mod ids {
    /// spark, lost, a sound, a particle without system.
    pub const PLAIN: u32 = 1;
    /// PropertyFlag by hue: default sound, case 0 spark, case `CASE_HUE` hued smoke.
    pub const HUE_SWITCH: u32 = 2;
    /// Random (spark), Lod (lost at 1, spark at 3), RoundRobin (smoke, spark).
    pub const CHOICES: u32 = 3;
    /// A RoundRobin that lists itself.
    pub const CYCLE: u32 = 4;
    /// The MESH script, attach mode 2.
    pub const MESHY: u32 = 5;
    pub const COLLECTION_COUNT: usize = 5;
    /// The MESH script (attach mode 2) and spark.
    pub const TILE: u32 = 0x1234;
    pub const SEQUENCE: u32 = 100;
    pub const ACTION: u16 = 2;
    pub const FRAME: u16 = 3;
}

/// What to vary.
#[derive(Debug, Clone, Copy)]
pub struct TestPackage {
    pub versions: PackageRecord,
    /// Replace the spark blob with bytes that do not decode.
    pub corrupt_spark: bool,
}

impl TestPackage {
    pub fn standard() -> Self {
        Self { versions: PackageRecord::CURRENT, corrupt_spark: false }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        let mut tables = Tables::default();
        let system_names = ["lost.nif", "mesh.ems", "orphan.ems", "smoke.nif", "spark.ems"];
        let texture_names = ["gone.tga", "smoke.nif#0", "spark.tga", "unused.tga"];
        let root = SYSTEM_FLAG_DEFINITION_ROOT;
        let system_specs = [
            (SYSTEM_KIND_NIF, SYSTEM_FLAG_MISSING | root, vec![]),
            (SYSTEM_KIND_CHAOS, root, vec![DEP_SYSTEM_BIT | systems::SMOKE, textures::GONE]),
            (SYSTEM_KIND_CHAOS, SYSTEM_FLAG_COMPILE_FAILED, vec![]),
            (SYSTEM_KIND_NIF, root, vec![textures::SMOKE_EMBEDDED]),
            (SYSTEM_KIND_CHAOS, root, vec![textures::SPARK]),
        ];
        let mut system_records = Vec::new();
        for (name, (kind, flags, deps)) in system_names.iter().zip(system_specs) {
            let dep_first = tables.deps.len() as u32;
            tables.deps.extend_from_slice(&deps);
            let name = tables.string(name);
            system_records.push(SystemRecord { name, kind, flags, dep_count: deps.len() as u16, dep_first });
        }
        let pixel = |value: u8| [value; RGBA8_BYTES_PER_PIXEL];
        let texture_specs: [(u16, u8, u8, u8, Vec<u8>); 4] = [
            (0, 0, TEXTURE_FLAG_MISSING, TEXTURE_SOURCE_NONE, vec![]),
            (2, 1, TEXTURE_FLAG_EMBEDDED, TEXTURE_SOURCE_NIF_PIXEL_DATA, pixel(200).repeat(4)),
            (4, 2, TEXTURE_FLAG_OPAQUE, TEXTURE_SOURCE_TGA, [pixel(u8::MAX).repeat(16), pixel(u8::MAX).repeat(4)].concat()),
            (1, 1, TEXTURE_FLAG_OPAQUE, TEXTURE_SOURCE_TGA, pixel(u8::MAX).to_vec()),
        ];
        let mut texture_records = Vec::new();
        let mut texture_payloads = Vec::new();
        for (index, (name, (side, mip_count, flags, source, data))) in texture_names.iter().zip(texture_specs).enumerate() {
            let format = if data.is_empty() { TEXTURE_FORMAT_NONE } else { TEXTURE_FORMAT_RGBA8 };
            let name = tables.string(name);
            texture_records.push(TextureRecord { name, width: side, height: side, format, mip_count, flags, source });
            if !data.is_empty() {
                texture_payloads.push((texture_entry_path(index), data));
            }
        }

        let particle = |tables: &mut Tables, system: Option<u32>, looping: bool, hued: bool, attach_mode: u8| {
            let node = ParticleNode {
                system,
                offset_x: 0.0,
                offset_y: 0.0,
                camera_offset: 0.0,
                scale: 1.0,
                duration: 0.0,
                attach_mode,
                field_3c: 0,
                looping,
                hued,
            };
            let flags = if looping { NODE_FLAG_LOOPING } else { 0 } | if hued { NODE_FLAG_HUED } else { 0 };
            tables.node(EFFECT_TYPE_PARTICLE, flags, &particle_params(&node), (0, 0))
        };
        let spark = particle(&mut tables, Some(systems::SPARK), true, false, 0);
        let smoke = particle(&mut tables, Some(systems::SMOKE), true, true, 0);
        let lost = particle(&mut tables, Some(systems::LOST), true, false, 0);
        let sound = tables.node(EFFECT_TYPE_SOUND, 0, &[SOUND_ID, 1.0f32.to_bits()], (0, 0));
        let default = tables.list(&[sound]);
        let cases = tables.branches(&[(0, vec![spark]), (CASE_HUE, vec![smoke])]);
        let flag_params = property_flag_params(0, crate::logic::effects::model::PROPERTY_FLAG_OBJECT_HUE, default.0, default.1);
        let switch = tables.node(EFFECT_TYPE_PROPERTY_FLAG, NODE_FLAG_PROPERTY_MODE, &flag_params, cases);
        let unnamed = particle(&mut tables, None, true, false, 0);
        let meshy = particle(&mut tables, Some(systems::MESH), true, false, PARTICLE_ATTACH_PROJECTILE_ONLY);
        let random_entries = tables.branches(&[(RANDOM_WEIGHT_TOTAL.to_bits(), vec![spark])]);
        let random = tables.node(EFFECT_TYPE_RANDOM, 0, &[], random_entries);
        let levels = tables.branches(&[(1, vec![lost]), (3, vec![spark])]);
        let lod = tables.node(EFFECT_TYPE_LOD, 0, &[], levels);
        let round_children = tables.list(&[smoke, spark]);
        let round_robin = tables.node(EFFECT_TYPE_ROUND_ROBIN, 0, &[], round_children);
        let cycle = tables.nodes.len() as u32;
        let cycle_children = tables.list(&[cycle]);
        tables.node(EFFECT_TYPE_ROUND_ROBIN, 0, &[], cycle_children);

        let collection_lists = [
            (ids::PLAIN, vec![spark, lost, sound, unnamed]),
            (ids::HUE_SWITCH, vec![switch]),
            (ids::CHOICES, vec![random, lod, round_robin]),
            (ids::CYCLE, vec![cycle]),
            (ids::MESHY, vec![meshy]),
        ];
        let collection: Vec<CollectionRecord> = collection_lists
            .iter()
            .map(|(id, nodes)| {
                let (list_first, list_count) = tables.list(nodes);
                CollectionRecord { collection_id: *id, list_first, list_count }
            })
            .collect();
        let (list_first, list_count) = tables.list(&[meshy, spark]);
        let tileart = [TileArtRecord { tile_id: ids::TILE, list_first, list_count }];
        let (list_first, list_count) = tables.list(&[smoke]);
        let frames = [SequenceFrameRecord {
            sequence_id: ids::SEQUENCE,
            action_id: ids::ACTION,
            frame: ids::FRAME,
            list_first,
            list_count,
        }];

        let chaos = |source: &str| encode_chaos_desc(&compile_chaos_source(source.as_bytes()).desc.expect("test script compiles"));
        let spark_blob = if self.corrupt_spark { b"PSCH not a blob".to_vec() } else { chaos(SPARK_SCRIPT) };
        let blobs = [
            (system_entry_path(systems::MESH as usize, SYSTEM_KIND_CHAOS), chaos(MESH_SCRIPT)),
            (system_entry_path(systems::SMOKE as usize, SYSTEM_KIND_NIF), encode_nif_desc(&smoke_desc())),
            (system_entry_path(systems::SPARK as usize, SYSTEM_KIND_CHAOS), spark_blob),
        ];

        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        let mut add = |path: String, data: Vec<u8>, data_type: DataType| {
            builder
                .add_owned_file(AddOwnedFileRequest {
                    data_type: data_type as u8,
                    compression: CompressionFlag::None,
                    width: 0,
                    height: 0,
                    virtual_path: Some(path),
                    path_hash64: None,
                    id: None,
                    data,
                })
                .expect("add test file");
        };
        let manifests = [
            (PACKAGE_ENTRY_PATH, encode_manifest(PACKAGE_MAGIC, &[self.versions])),
            (STRINGS_ENTRY_PATH, encode_strings(&tables.strings)),
            (COLLECTION_ENTRY_PATH, encode_manifest(COLLECTION_MAGIC, &collection)),
            (TILEART_ENTRY_PATH, encode_manifest(TILEART_MAGIC, &tileart)),
            (SEQUENCE_FRAMES_ENTRY_PATH, encode_manifest(SEQUENCE_FRAMES_MAGIC, &frames)),
            (DEF_NODES_ENTRY_PATH, encode_manifest(DEF_NODES_MAGIC, &tables.nodes)),
            (DEF_LISTS_ENTRY_PATH, encode_manifest(DEF_LISTS_MAGIC, &tables.lists)),
            (DEF_BRANCHES_ENTRY_PATH, encode_manifest(DEF_BRANCHES_MAGIC, &tables.branches)),
            (DEF_PARAMS_ENTRY_PATH, encode_manifest(DEF_PARAMS_MAGIC, &tables.params)),
            (SYSTEMS_ENTRY_PATH, encode_manifest(SYSTEMS_MAGIC, &system_records)),
            (DEPS_ENTRY_PATH, encode_manifest(DEPS_MAGIC, &tables.deps)),
            (TEXTURES_ENTRY_PATH, encode_manifest(TEXTURES_MAGIC, &texture_records)),
        ];
        for (path, data) in manifests {
            add(path.to_owned(), data, DataType::Metadata);
        }
        for (path, data) in blobs {
            add(path, data, DataType::Effect);
        }
        for (path, data) in texture_payloads {
            add(path, data, DataType::Texture);
        }
        builder.into_bytes().expect("test package bytes")
    }
}

/// `smoke.nif`: no scene, one texture embedded in the source file (renamed as the packer renames it).
fn smoke_desc() -> NifEffectDesc {
    let texture = NifTexture {
        index: 0,
        source: NifTextureSource::External { file_name: "smoke.nif#0".to_owned() },
        mipmapped: false,
    };
    NifEffectDesc { name: "smoke.nif".to_owned(), textures: vec![texture], ..NifEffectDesc::default() }
}

/// The package opened, and its container.
pub fn open(package: TestPackage) -> (Arc<EffectsEcPackage>, UddpReader) {
    let reader = UddpReader::open(package.into_bytes()).expect("test container");
    let package = EffectsEcPackage::from_reader(reader.clone()).expect("test package opens");
    (Arc::new(package), reader)
}

#[derive(Default)]
struct Tables {
    strings: Vec<String>,
    nodes: Vec<DefNodeRecord>,
    lists: Vec<u32>,
    branches: Vec<DefBranchRecord>,
    params: Vec<u32>,
    deps: Vec<u32>,
}

impl Tables {
    fn string(&mut self, value: &str) -> u32 {
        self.strings.push(value.to_owned());
        self.strings.len() as u32 - 1
    }

    /// A node with its parameter words and links (first, count); returns its index.
    fn node(&mut self, type_key: u32, flags: u8, params: &[u32], (first, count): (u32, u32)) -> u32 {
        let param = if params.is_empty() { NONE } else { self.params.len() as u32 };
        self.params.extend_from_slice(params);
        self.nodes.push(DefNodeRecord { type_key: type_key as u8, flags, reserved: 0, param, first, count });
        self.nodes.len() as u32 - 1
    }

    fn list(&mut self, nodes: &[u32]) -> (u32, u32) {
        let first = self.lists.len() as u32;
        self.lists.extend_from_slice(nodes);
        (first, nodes.len() as u32)
    }

    fn branches(&mut self, entries: &[(u32, Vec<u32>)]) -> (u32, u32) {
        let lists: Vec<(u32, u32)> = entries.iter().map(|(_, nodes)| self.list(nodes)).collect();
        let first = self.branches.len() as u32;
        for ((key, _), (list_first, list_count)) in entries.iter().zip(lists) {
            self.branches.push(DefBranchRecord { key: *key, list_first, list_count });
        }
        (first, entries.len() as u32)
    }
}
