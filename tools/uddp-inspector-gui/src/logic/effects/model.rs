//! View models of an `effects_ec.uddp` package: the version record, the overview counts, the rows of the
//! collection, tileart, sequence frame, system and texture tables, and the definition tree of a root list.
//! No egui here.

use std::collections::{BTreeMap, BTreeSet};

use udd_assets::effects_ec::{
    DEP_SYSTEM_BIT, EffectNode, EffectsEcPackage, MANIFEST_HEADER_SIZE, ManifestHeader, PACKAGE_ENTRY_PATH,
    PACKAGE_MAGIC, PackageRecord, SYSTEM_FLAG_COMPILE_FAILED, SYSTEM_FLAG_COMPILE_WARNINGS, SYSTEM_FLAG_DEFINITION_ROOT,
    SYSTEM_FLAG_MISSING, SYSTEM_KIND_CHAOS, SYSTEM_KIND_NIF, TEXTURE_FLAG_AUTHORED_MIPS, TEXTURE_FLAG_DECODE_FAILED,
    TEXTURE_FLAG_EMBEDDED, TEXTURE_FLAG_MISSING, TEXTURE_FLAG_OPAQUE, TEXTURE_FLAG_UNDECODED, TEXTURE_SOURCE_DDS,
    TEXTURE_SOURCE_NIF_FILE, TEXTURE_SOURCE_NIF_PIXEL_DATA, TEXTURE_SOURCE_NONE, TEXTURE_SOURCE_TGA,
    system_entry_path, texture_entry_path,
};
use udd_container::{UddpReader, xxh64_virtual_path};
use uocf::enhanced::effect_definition::MAX_EFFECT_NESTING_DEPTH;

use super::texture::format_name;

/// `PropertyFlag` property that keys the cases by the object hue (`UOObject::GetHue`; `uosa_effects.md`,
/// "12 PropertyFlag").
pub const PROPERTY_FLAG_OBJECT_HUE: u32 = 7;
/// Particle names listed in a root row summary before "+N more".
const SUMMARY_MAX_NAMES: usize = 4;

/// Names of the system flags, in bit order.
pub const SYSTEM_FLAG_NAMES: [(u8, &str); 4] = [
    (SYSTEM_FLAG_MISSING, "missing"),
    (SYSTEM_FLAG_COMPILE_FAILED, "compile failed"),
    (SYSTEM_FLAG_COMPILE_WARNINGS, "warnings"),
    (SYSTEM_FLAG_DEFINITION_ROOT, "definition root"),
];

/// Names of the texture flags, in bit order.
pub const TEXTURE_FLAG_NAMES: [(u8, &str); 6] = [
    (TEXTURE_FLAG_EMBEDDED, "embedded"),
    (TEXTURE_FLAG_OPAQUE, "opaque"),
    (TEXTURE_FLAG_AUTHORED_MIPS, "authored mips"),
    (TEXTURE_FLAG_MISSING, "missing"),
    (TEXTURE_FLAG_UNDECODED, "undecoded"),
    (TEXTURE_FLAG_DECODE_FAILED, "decode failed"),
];

/// The set flags of `flags` by name, comma separated ("-" for none).
pub fn flag_names(flags: u8, names: &[(u8, &str)]) -> String {
    let set: Vec<&str> = names.iter().filter(|(bit, _)| flags & bit != 0).map(|(_, name)| *name).collect();
    if set.is_empty() { "-".to_owned() } else { set.join(", ") }
}

pub fn system_kind_name(kind: u8) -> &'static str {
    match kind {
        SYSTEM_KIND_CHAOS => "Chaos",
        SYSTEM_KIND_NIF => "NIF",
        _ => "unknown",
    }
}

pub fn texture_source_name(source: u8) -> &'static str {
    match source {
        TEXTURE_SOURCE_NONE => "none",
        TEXTURE_SOURCE_TGA => "TGA",
        TEXTURE_SOURCE_DDS => "DDS",
        TEXTURE_SOURCE_NIF_PIXEL_DATA => "NIF pixel data",
        TEXTURE_SOURCE_NIF_FILE => "NIF file",
        _ => "unknown",
    }
}

/// `package.bin` as stored, read on its own: a package of other versions does not open with
/// [`EffectsEcPackage`], but its versions are still shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageVersions {
    pub stored: PackageRecord,
    /// The versions this build reads.
    pub current: PackageRecord,
}

impl PackageVersions {
    pub fn matches(&self) -> bool {
        self.stored == self.current
    }

    /// What to do when the versions differ.
    pub fn mismatch_message(&self) -> Option<String> {
        let (stored, current) = (self.stored, self.current);
        let mut differences = Vec::new();
        let mut compare = |what: &str, stored: u32, current: u32| {
            if stored != current {
                differences.push(format!("{what} {stored} (this build: {current})"));
            }
        };
        compare("Chaos desc", stored.chaos_desc_version, current.chaos_desc_version);
        compare("NIF desc", stored.nif_desc_version, current.nif_desc_version);
        compare("desc codec", stored.desc_codec_version, current.desc_codec_version);
        (!differences.is_empty()).then(|| {
            format!(
                "Rebuild needed: the system blobs were written with {}. Repack effects_ec.uddp with this build \
                 (udd-cli pack pack-ec-effects).",
                differences.join(", ")
            )
        })
    }
}

/// Whether the package has `metadata/package.bin` (an effects package), and its versions. `None`: not an effects
/// package; `Some(Err)`: the record does not read.
pub fn read_package_versions(reader: &UddpReader) -> Option<Result<PackageVersions, String>> {
    let hash = xxh64_virtual_path(PACKAGE_ENTRY_PATH);
    reader.find_by_path_hash(hash)?;
    Some(read_package_record(reader, hash).map(|stored| PackageVersions { stored, current: PackageRecord::CURRENT }))
}

fn read_package_record(reader: &UddpReader, hash: u64) -> Result<PackageRecord, String> {
    let bytes = reader.read_file_by_path_hash(hash).map_err(|error| format!("{PACKAGE_ENTRY_PATH}: {error}"))?;
    let record_size = size_of::<PackageRecord>();
    if bytes.len() != MANIFEST_HEADER_SIZE + record_size {
        return Err(format!("{PACKAGE_ENTRY_PATH}: {} bytes, expected {}", bytes.len(), MANIFEST_HEADER_SIZE + record_size));
    }
    let header: ManifestHeader = bytemuck::pod_read_unaligned(&bytes[..MANIFEST_HEADER_SIZE]);
    if header.magic != PACKAGE_MAGIC {
        return Err(format!("{PACKAGE_ENTRY_PATH}: magic {:?}", String::from_utf8_lossy(&header.magic)));
    }
    Ok(bytemuck::pod_read_unaligned(&bytes[MANIFEST_HEADER_SIZE..]))
}

/// Display name of a definition node type.
pub fn node_type_name(node: &EffectNode<'_>) -> &'static str {
    match node {
        EffectNode::Particle(_) => "Particle",
        EffectNode::Sound { .. } => "Sound",
        EffectNode::RoundRobin { .. } => "RoundRobin",
        EffectNode::PositionOffset { .. } => "PositionOffset",
        EffectNode::Wait { .. } => "Wait",
        EffectNode::Lod { .. } => "Lod",
        EffectNode::PropertyFlag { .. } => "PropertyFlag",
        EffectNode::Earthquake { .. } => "Earthquake",
        EffectNode::Random { .. } => "Random",
        EffectNode::UserPreference { .. } => "UserPreference",
    }
}

/// The child lists of a node, labelled: (label, node indices).
pub fn node_child_lists<'a>(package: &'a EffectsEcPackage, node: &EffectNode<'a>) -> Vec<(String, &'a [u32])> {
    match *node {
        EffectNode::RoundRobin { children } | EffectNode::Wait { children, .. } | EffectNode::UserPreference { children, .. } => {
            vec![("children".to_owned(), children)]
        }
        EffectNode::Lod { levels } => levels
            .iter()
            .map(|level| (format!("level {}", level.key), package.list(level.list_first, level.list_count)))
            .collect(),
        EffectNode::PropertyFlag { default_children, cases, .. } => std::iter::once(("default".to_owned(), default_children))
            .chain(cases.iter().map(|case| (format!("case {}", case.key), package.list(case.list_first, case.list_count))))
            .collect(),
        EffectNode::Random { entries } => entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                (format!("entry {index} (weight {})", entry.weight()), package.list(entry.list_first, entry.list_count))
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The name of a particle's system, or why there is none.
pub fn particle_system_label(package: &EffectsEcPackage, system: Option<u32>) -> String {
    match system {
        Some(system) => package.system_name(system).to_owned(),
        None => "(no system: name not in the dictionary)".to_owned(),
    }
}

/// Every parameter of a node, labelled.
pub fn node_params(package: &EffectsEcPackage, node: &EffectNode<'_>) -> Vec<(&'static str, String)> {
    match *node {
        EffectNode::Particle(particle) => {
            let system = match particle.system {
                Some(system) => {
                    let record = package.system(system);
                    format!(
                        "{} (#{system}, {}, {})",
                        package.system_name(system),
                        system_kind_name(record.kind),
                        flag_names(record.flags, &SYSTEM_FLAG_NAMES)
                    )
                }
                None => particle_system_label(package, None),
            };
            vec![
                ("system", system),
                ("offset_x", particle.offset_x.to_string()),
                ("offset_y", particle.offset_y.to_string()),
                ("camera_offset", particle.camera_offset.to_string()),
                ("scale", particle.scale.to_string()),
                ("duration", particle.duration.to_string()),
                ("attach_mode", particle.attach_mode.to_string()),
                ("field_3c", particle.field_3c.to_string()),
                ("looping", particle.looping.to_string()),
                ("hued", particle.hued.to_string()),
            ]
        }
        EffectNode::Sound { sound_id, value_08 } => vec![("sound_id", sound_id.to_string()), ("value_08", value_08.to_string())],
        EffectNode::RoundRobin { children } => vec![("children", children.len().to_string())],
        EffectNode::PositionOffset { offsets } => {
            let offsets: Vec<String> = offsets.iter().map(|(x, y)| format!("({x}, {y})")).collect();
            vec![("offsets (N..NW)", offsets.join(" "))]
        }
        EffectNode::Wait { children, min_delay_ms, max_delay_ms, aligned } => vec![
            ("min_delay_ms", min_delay_ms.to_string()),
            ("max_delay_ms", max_delay_ms.to_string()),
            ("aligned", aligned.to_string()),
            ("children", children.len().to_string()),
        ],
        EffectNode::Lod { levels } => {
            vec![("levels", levels.iter().map(|level| level.key.to_string()).collect::<Vec<_>>().join(", "))]
        }
        EffectNode::PropertyFlag { property_mode, mask, property, default_children, cases } => {
            let mode = if property_mode { "Property" } else { "FlagMask" };
            let property = if property_mode && property == PROPERTY_FLAG_OBJECT_HUE {
                format!("{property} (object hue)")
            } else {
                property.to_string()
            };
            vec![
                ("mode", mode.to_owned()),
                ("mask", format!("{mask:#018x}")),
                ("property", property),
                ("default children", default_children.len().to_string()),
                ("cases", cases.iter().map(|case| case.key.to_string()).collect::<Vec<_>>().join(", ")),
            ]
        }
        EffectNode::Earthquake { amplitude, duration, frequency } => vec![
            ("amplitude", amplitude.to_string()),
            ("duration", duration.to_string()),
            ("frequency", frequency.to_string()),
        ],
        EffectNode::Random { entries } => {
            vec![("weights", entries.iter().map(|entry| entry.weight().to_string()).collect::<Vec<_>>().join(", "))]
        }
        EffectNode::UserPreference { mask, children } => {
            vec![("mask", format!("{mask:#x}")), ("children", children.len().to_string())]
        }
    }
}

/// One node of a definition tree, with its parameters and labelled child lists.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    pub node: u32,
    pub title: String,
    pub params: Vec<(&'static str, String)>,
    pub groups: Vec<TreeGroup>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeGroup {
    pub label: String,
    pub children: Vec<TreeNode>,
}

/// The definition tree of a root list. Lists nested deeper than the client allows (or cyclic, in a corrupt
/// package) end in a node that says so.
pub fn definition_tree(package: &EffectsEcPackage, roots: &[u32]) -> Vec<TreeNode> {
    fn build(package: &EffectsEcPackage, index: u32, depth: usize) -> TreeNode {
        if depth >= MAX_EFFECT_NESTING_DEPTH {
            return TreeNode {
                node: index,
                title: format!("#{index}: nested deeper than {MAX_EFFECT_NESTING_DEPTH} (cyclic?), not expanded"),
                params: Vec::new(),
                groups: Vec::new(),
            };
        }
        let node = package.node(index);
        let mut title = format!("#{index} {}", node_type_name(&node));
        if let EffectNode::Particle(particle) = node {
            title.push_str(&format!(": {}", particle_system_label(package, particle.system)));
        }
        let groups = node_child_lists(package, &node)
            .into_iter()
            .map(|(label, children)| TreeGroup {
                label,
                children: children.iter().map(|&child| build(package, child, depth + 1)).collect(),
            })
            .collect();
        TreeNode { node: index, title, params: node_params(package, &node), groups }
    }
    roots.iter().map(|&root| build(package, root, 0)).collect()
}

/// Visits every node of a root list depth first; returns false when a list nests deeper than
/// `MAX_EFFECT_NESTING_DEPTH` (those nodes are not visited).
pub fn walk_nodes(package: &EffectsEcPackage, roots: &[u32], visit: &mut impl FnMut(u32, &EffectNode<'_>)) -> bool {
    fn walk(package: &EffectsEcPackage, list: &[u32], depth: usize, visit: &mut impl FnMut(u32, &EffectNode<'_>)) -> bool {
        if depth >= MAX_EFFECT_NESTING_DEPTH {
            return list.is_empty();
        }
        let mut complete = true;
        for &index in list {
            let node = package.node(index);
            visit(index, &node);
            for (_, children) in node_child_lists(package, &node) {
                complete &= walk(package, children, depth + 1, visit);
            }
        }
        complete
    }
    walk(package, roots, 0, visit)
}

/// Which table a root list belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RootKey {
    Collection(u32),
    Tile(u32),
    /// `AnimationSequence` entry id (`MobileAnimEcBodyRecord::sequence_id`), action and frame.
    Frame { sequence_id: u32, action_id: u16, frame: u16 },
}

impl RootKey {
    pub fn label(&self) -> String {
        match *self {
            Self::Collection(id) => format!("collection {id}"),
            Self::Tile(tile) => format!("tile {tile} ({tile:#06x})"),
            Self::Frame { sequence_id, action_id, frame } => {
                format!("sequence {sequence_id} action {action_id} frame {frame}")
            }
        }
    }

    pub fn roots<'a>(&self, package: &'a EffectsEcPackage) -> &'a [u32] {
        match *self {
            Self::Collection(id) => package.collection_roots(id),
            Self::Tile(tile) => package.tileart_roots(tile),
            Self::Frame { sequence_id, action_id, frame } => package.frame_roots(sequence_id, action_id, frame),
        }
        .unwrap_or(&[])
    }
}

/// A row of the collection, tileart or sequence frame table.
#[derive(Debug, Clone, PartialEq)]
pub struct RootRow {
    pub key: RootKey,
    pub roots: usize,
    /// Every node of the tree.
    pub nodes: usize,
    /// The systems the particles name, in tree order, without repeats.
    pub systems: Vec<u32>,
    /// Particles naming no system.
    pub unnamed_particles: usize,
    /// The PropertyFlag switches: "hue: 0, 1153 (+default)".
    pub switches: Vec<String>,
    /// The case keys of hue-keyed PropertyFlags, ascending.
    pub hue_cases: Vec<u32>,
    /// Particles whose system is missing from the client.
    pub missing: usize,
    /// The tree nests deeper than the client allows.
    pub too_deep: bool,
    /// Particle names, shortened.
    pub summary: String,
    /// Lowercase text the filter searches.
    pub search: String,
}

fn root_row(package: &EffectsEcPackage, key: RootKey) -> RootRow {
    let roots = key.roots(package);
    let (mut nodes, mut unnamed, mut missing) = (0, 0, 0);
    let mut systems = Vec::new();
    let mut switches = Vec::new();
    let mut hue_cases = BTreeSet::new();
    let complete = walk_nodes(package, roots, &mut |_, node| {
        nodes += 1;
        match *node {
            EffectNode::Particle(particle) => match particle.system {
                Some(system) => {
                    if !systems.contains(&system) {
                        systems.push(system);
                    }
                    missing += usize::from(package.system(system).flags & SYSTEM_FLAG_MISSING != 0);
                }
                None => unnamed += 1,
            },
            EffectNode::PropertyFlag { property_mode, mask, property, default_children, cases } => {
                let keys: Vec<String> = cases.iter().map(|case| case.key.to_string()).collect();
                let what = if !property_mode {
                    format!("mask {mask:#x}")
                } else if property == PROPERTY_FLAG_OBJECT_HUE {
                    hue_cases.extend(cases.iter().map(|case| case.key));
                    "hue".to_owned()
                } else {
                    format!("property {property}")
                };
                let default = if default_children.is_empty() { "" } else { " (+default)" };
                switches.push(format!("{what}: {}{default}", keys.join(", ")));
            }
            _ => {}
        }
    });
    let names: Vec<&str> = systems.iter().map(|&system| package.system_name(system)).collect();
    let mut summary = names.iter().take(SUMMARY_MAX_NAMES).copied().collect::<Vec<_>>().join(", ");
    if names.len() > SUMMARY_MAX_NAMES {
        summary.push_str(&format!(" +{} more", names.len() - SUMMARY_MAX_NAMES));
    }
    let id_text = match key {
        RootKey::Collection(id) | RootKey::Tile(id) => format!("{id} {id:#x}"),
        RootKey::Frame { sequence_id, action_id, frame } => format!("{sequence_id} {sequence_id:#x} a{action_id} f{frame}"),
    };
    let search = format!("{id_text} {} {}", names.join(" "), switches.join(" ")).to_ascii_lowercase();
    RootRow {
        key,
        roots: roots.len(),
        nodes,
        systems,
        unnamed_particles: unnamed,
        switches,
        hue_cases: hue_cases.into_iter().collect(),
        missing,
        too_deep: !complete,
        summary,
        search,
    }
}

/// A row of the system table.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemRow {
    pub index: u32,
    pub name: String,
    pub kind: u8,
    pub flags: u8,
    /// `deps.bin` values ([`DEP_SYSTEM_BIT`]).
    pub deps: Vec<u32>,
    /// Bytes of the codec blob, unpacked; `None` without payload.
    pub blob_size: Option<u32>,
    /// Particle nodes naming the system.
    pub definition_users: usize,
    /// Systems depending on it (Chaos MESH sequences).
    pub mesh_users: Vec<u32>,
    pub search: String,
}

impl SystemRow {
    /// The dependencies by name: `mesh x.nif`, `tex y.tga`.
    pub fn dependency_names(&self, package: &EffectsEcPackage) -> Vec<String> {
        self.deps.iter().map(|&dep| dependency_name(package, dep)).collect()
    }
}

/// A `deps.bin` value by name.
pub fn dependency_name(package: &EffectsEcPackage, dep: u32) -> String {
    let target = dep & !DEP_SYSTEM_BIT;
    if dep & DEP_SYSTEM_BIT != 0 {
        format!("mesh {}", package.system_name(target))
    } else {
        format!("tex {}", package.texture_name(target))
    }
}

/// A row of the texture table.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureRow {
    pub index: u32,
    pub name: String,
    pub width: u16,
    pub height: u16,
    pub format: u8,
    pub mip_count: u8,
    pub flags: u8,
    pub source: u8,
    /// Bytes of the payload, unpacked; `None` without payload.
    pub payload_size: Option<u32>,
    /// Systems depending on it.
    pub users: Vec<u32>,
    pub search: String,
}

/// The counts of the overview.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overview {
    pub collection_entries: usize,
    pub tiles: usize,
    pub sequence_frames: usize,
    /// Distinct `AnimationSequence` entries among the frames.
    pub sequences: usize,
    pub nodes: usize,
    pub systems: usize,
    pub chaos: usize,
    pub nif: usize,
    pub missing: usize,
    pub compile_failed: usize,
    pub warnings: usize,
    /// Systems no particle definition names (only a Chaos MESH sequence uses them).
    pub mesh_only: usize,
    pub textures: usize,
    /// Count per format name.
    pub texture_formats: BTreeMap<&'static str, usize>,
    /// Count per source name.
    pub texture_sources: BTreeMap<&'static str, usize>,
    /// Count per flag name.
    pub texture_flags: BTreeMap<&'static str, usize>,
    /// Textures with more than one level, and the levels of every payload.
    pub mipmapped_textures: usize,
    pub texture_levels: usize,
    /// Systems and textures no root list reaches (through particles and dependencies).
    pub unreferenced_systems: Vec<u32>,
    pub unreferenced_textures: Vec<u32>,
    /// Definition nodes no root list reaches.
    pub unreachable_nodes: usize,
}

/// Every view model of an opened package.
pub struct EffectsModel {
    pub versions: PackageVersions,
    pub overview: Overview,
    pub collection: Vec<RootRow>,
    pub tileart: Vec<RootRow>,
    pub frames: Vec<RootRow>,
    pub systems: Vec<SystemRow>,
    pub textures: Vec<TextureRow>,
}

impl EffectsModel {
    /// The models of `package`; `reader` is the package's container (payload sizes).
    pub fn build(package: &EffectsEcPackage, reader: &UddpReader) -> Self {
        let collection: Vec<RootRow> =
            package.collection().iter().map(|record| root_row(package, RootKey::Collection(record.collection_id))).collect();
        let tileart: Vec<RootRow> = package.tileart().iter().map(|record| root_row(package, RootKey::Tile(record.tile_id))).collect();
        let frames: Vec<RootRow> = package
            .sequence_frames()
            .iter()
            .map(|record| {
                let key =
                    RootKey::Frame { sequence_id: record.sequence_id, action_id: record.action_id, frame: record.frame };
                root_row(package, key)
            })
            .collect();

        // Particle users of each system, and every node a root list reaches.
        let mut definition_users = vec![0usize; package.systems().len()];
        let mut reached_nodes = vec![false; package.nodes().len()];
        let all_roots = collection.iter().chain(&tileart).chain(&frames).map(|row| row.key.roots(package));
        for roots in all_roots {
            walk_nodes(package, roots, &mut |index, node| {
                if std::mem::replace(&mut reached_nodes[index as usize], true) {
                    return;
                }
                if let EffectNode::Particle(particle) = node {
                    if let Some(system) = particle.system {
                        definition_users[system as usize] += 1;
                    }
                }
            });
        }

        let payload_size = |path: String| reader.find_by_path_hash(xxh64_virtual_path(&path)).map(|file| file.raw_size);
        let mut mesh_users = vec![Vec::new(); package.systems().len()];
        let mut texture_users = vec![Vec::new(); package.textures().len()];
        for index in 0..package.systems().len() as u32 {
            for &dep in package.system_dependencies(index) {
                let target = (dep & !DEP_SYSTEM_BIT) as usize;
                if dep & DEP_SYSTEM_BIT != 0 {
                    mesh_users[target].push(index);
                } else {
                    texture_users[target].push(index);
                }
            }
        }

        // Reachability: systems named by reached particles, then their dependencies.
        let mut reached_systems = vec![false; package.systems().len()];
        let mut reached_textures = vec![false; package.textures().len()];
        let mut pending: Vec<u32> = (0..package.systems().len() as u32).filter(|&s| definition_users[s as usize] > 0).collect();
        while let Some(system) = pending.pop() {
            if std::mem::replace(&mut reached_systems[system as usize], true) {
                continue;
            }
            for &dep in package.system_dependencies(system) {
                let target = dep & !DEP_SYSTEM_BIT;
                if dep & DEP_SYSTEM_BIT != 0 {
                    pending.push(target);
                } else {
                    reached_textures[target as usize] = true;
                }
            }
        }

        let systems: Vec<SystemRow> = package
            .systems()
            .iter()
            .enumerate()
            .map(|(index, record)| {
                let index = index as u32;
                let name = package.system_name(index).to_owned();
                let search = format!("{index} {name} {}", system_kind_name(record.kind)).to_ascii_lowercase();
                SystemRow {
                    index,
                    kind: record.kind,
                    flags: record.flags,
                    deps: package.system_dependencies(index).to_vec(),
                    blob_size: record.has_payload().then(|| payload_size(system_entry_path(index as usize, record.kind))).flatten(),
                    definition_users: definition_users[index as usize],
                    mesh_users: std::mem::take(&mut mesh_users[index as usize]),
                    name,
                    search,
                }
            })
            .collect();
        let textures: Vec<TextureRow> = package
            .textures()
            .iter()
            .enumerate()
            .map(|(index, record)| {
                let name = package.texture_name(index as u32).to_owned();
                let search = format!("{index} {name} {}", format_name(record.format)).to_ascii_lowercase();
                TextureRow {
                    index: index as u32,
                    width: record.width,
                    height: record.height,
                    format: record.format,
                    mip_count: record.mip_count,
                    flags: record.flags,
                    source: record.source,
                    payload_size: record.has_payload().then(|| payload_size(texture_entry_path(index))).flatten(),
                    users: std::mem::take(&mut texture_users[index]),
                    name,
                    search,
                }
            })
            .collect();

        let mut overview = Overview {
            collection_entries: collection.len(),
            tiles: tileart.len(),
            sequence_frames: frames.len(),
            sequences: package.sequence_frames().iter().map(|record| record.sequence_id).collect::<BTreeSet<_>>().len(),
            nodes: package.nodes().len(),
            systems: systems.len(),
            textures: textures.len(),
            unreferenced_systems: (0..systems.len() as u32).filter(|&s| !reached_systems[s as usize]).collect(),
            unreferenced_textures: (0..textures.len() as u32).filter(|&t| !reached_textures[t as usize]).collect(),
            unreachable_nodes: reached_nodes.iter().filter(|&&reached| !reached).count(),
            ..Overview::default()
        };
        for system in &systems {
            overview.chaos += usize::from(system.kind == SYSTEM_KIND_CHAOS);
            overview.nif += usize::from(system.kind == SYSTEM_KIND_NIF);
            overview.missing += usize::from(system.flags & SYSTEM_FLAG_MISSING != 0);
            overview.compile_failed += usize::from(system.flags & SYSTEM_FLAG_COMPILE_FAILED != 0);
            overview.warnings += usize::from(system.flags & SYSTEM_FLAG_COMPILE_WARNINGS != 0);
            overview.mesh_only += usize::from(system.flags & SYSTEM_FLAG_DEFINITION_ROOT == 0);
        }
        for texture in &textures {
            *overview.texture_formats.entry(format_name(texture.format)).or_default() += 1;
            *overview.texture_sources.entry(texture_source_name(texture.source)).or_default() += 1;
            for (bit, name) in TEXTURE_FLAG_NAMES {
                if texture.flags & bit != 0 {
                    *overview.texture_flags.entry(name).or_default() += 1;
                }
            }
            overview.mipmapped_textures += usize::from(texture.mip_count > 1);
            overview.texture_levels += usize::from(texture.mip_count);
        }
        let versions = PackageVersions { stored: PackageRecord::CURRENT, current: PackageRecord::CURRENT };
        Self { versions, overview, collection, tileart, frames, systems, textures }
    }
}

/// Whether a row's search text matches the filter (case-insensitive substring; empty matches everything).
pub fn matches_filter(search: &str, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty() || search.contains(&filter.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::effects::test_package::{self, ids, systems, textures};

    #[test]
    fn rows_and_counts_follow_the_package() {
        let (package, reader) = test_package::open(test_package::TestPackage::standard());
        let model = EffectsModel::build(&package, &reader);
        let overview = &model.overview;
        assert_eq!(
            (overview.collection_entries, overview.tiles, overview.sequence_frames, overview.sequences),
            (ids::COLLECTION_COUNT, 1, 1, 1)
        );
        assert_eq!((overview.systems, overview.chaos, overview.nif), (5, 3, 2));
        assert_eq!((overview.missing, overview.compile_failed, overview.mesh_only), (1, 1, 1));
        assert_eq!(overview.texture_formats.get("RGBA8"), Some(&3));
        assert_eq!(overview.texture_flags.get("missing"), Some(&1));
        assert_eq!(overview.unreferenced_systems, [systems::ORPHAN]);
        assert_eq!(overview.unreferenced_textures, [textures::UNUSED]);

        let plain = model.collection.iter().find(|row| row.key == RootKey::Collection(ids::PLAIN)).unwrap();
        assert_eq!(plain.systems, [systems::SPARK, systems::LOST]);
        assert_eq!((plain.missing, plain.unnamed_particles), (1, 1));
        let flagged = model.collection.iter().find(|row| row.key == RootKey::Collection(ids::HUE_SWITCH)).unwrap();
        assert_eq!(flagged.hue_cases, [0, test_package::CASE_HUE]);
        assert_eq!(flagged.switches, [format!("hue: 0, {} (+default)", test_package::CASE_HUE)]);

        let mesh = &model.systems[systems::MESH as usize];
        assert_eq!(mesh.dependency_names(&package), ["mesh smoke.nif", "tex gone.tga"]);
        assert!(mesh.blob_size.is_some_and(|size| size > 0));
        assert_eq!(model.systems[systems::SMOKE as usize].mesh_users, [systems::MESH]);
        assert_eq!(model.systems[systems::LOST as usize].blob_size, None);
        assert_eq!(model.textures[textures::SPARK as usize].users, [systems::SPARK]);
        assert_eq!(model.textures[textures::SPARK as usize].mip_count, 2);

        assert!(matches_filter(&plain.search, "SPARK"));
        assert!(matches_filter(&plain.search, &format!("{:#x}", ids::PLAIN)));
        assert!(!matches_filter(&plain.search, "smoke"));
    }

    #[test]
    fn the_tree_shows_every_list_and_stops_at_cycles() {
        let (package, _) = test_package::open(test_package::TestPackage::standard());
        let tree = definition_tree(&package, package.collection_roots(ids::HUE_SWITCH).unwrap());
        assert_eq!(tree.len(), 1);
        let labels: Vec<&str> = tree[0].groups.iter().map(|group| group.label.as_str()).collect();
        assert_eq!(labels, ["default".to_owned(), "case 0".to_owned(), format!("case {}", test_package::CASE_HUE)]);
        assert!(tree[0].params.iter().any(|(name, value)| *name == "property" && value.contains("object hue")));

        let cyclic = definition_tree(&package, package.collection_roots(ids::CYCLE).unwrap());
        let mut depth = 0;
        let mut node = &cyclic[0];
        while let Some(group) = node.groups.first() {
            node = &group.children[0];
            depth += 1;
        }
        assert_eq!(depth, MAX_EFFECT_NESTING_DEPTH);
        assert!(node.title.contains("cyclic"));
        let row = root_row(&package, RootKey::Collection(ids::CYCLE));
        assert!(row.too_deep);
    }

    #[test]
    fn versions_read_even_when_the_package_does_not_open() {
        let other = PackageRecord { desc_codec_version: PackageRecord::CURRENT.desc_codec_version + 1, ..PackageRecord::CURRENT };
        let bytes = test_package::TestPackage { versions: other, ..test_package::TestPackage::standard() }.into_bytes();
        let reader = UddpReader::open(bytes).unwrap();
        let versions = read_package_versions(&reader).unwrap().unwrap();
        assert!(!versions.matches());
        let message = versions.mismatch_message().unwrap();
        assert!(message.contains("Rebuild needed") && message.contains("desc codec"), "{message}");
        let error = EffectsEcPackage::from_reader(reader).err().unwrap().to_string();
        assert!(error.contains("repack"), "{error}");
    }
}
