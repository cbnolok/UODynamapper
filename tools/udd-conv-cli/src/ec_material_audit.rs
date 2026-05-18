//! Evidence and review tooling for Enhanced Client material resources.
//!
//! These commands produce developer-facing audit artifacts. They are intentionally
//! separate from the normal pack commands because their output is not needed to
//! unpack or repack runtime packages.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use color_eyre::eyre;
use csv::WriterBuilder;
use log::info;
use udd_conv::source_paths::find_first_existing_file;
use udd_conv::tex_art_ec::{load_tex_art_ec_sources, TexArtEcLoadedSources};
use uocf::{
    enhanced::{
        terrain_definition::TerrainTextureType,
        textures::TextureItem as EnhancedTextureItem,
        tileart::{TextureType as TileArtTextureType, TileType},
    },
    uop_container::{hash::hash_file_name_single, package::UopPackage},
    utils::path::normalize_dictionary_path,
};

pub fn audit_ec_material_refs(source_dirs: &[PathBuf], output: &Path) -> eyre::Result<()> {
    let sources = load_tex_art_ec_sources(source_dirs)?;
    let package_membership = EcPackageMembership::load(source_dirs)?;
    let mut writer = WriterBuilder::new().from_path(output)?;
    writer.write_record([
        "owner_kind",
        "owner_id",
        "owner_name",
        "owner_aliases",
        "shader_name",
        "tile_type",
        "tile_flags",
        "ref_block_index",
        "ref_item_index",
        "texture_id",
        "raw_path",
        "normalized_path",
        "physical_package_guess",
        "logical_family",
        "is_auxiliary_or_support",
        "stretch_or_repetition",
        "unk4",
        "unk6",
        "unk7",
        "directness",
    ])?;

    let mut row_count = 0u64;
    let mut owner_counts = BTreeMap::<String, u64>::new();
    let mut package_counts = BTreeMap::<String, u64>::new();
    let mut family_counts = BTreeMap::<String, u64>::new();

    let mut art_ids = sources
        .art_definition
        .definitions
        .keys()
        .copied()
        .collect::<Vec<_>>();
    art_ids.sort_unstable();

    for art_id in art_ids {
        let Some(art_data) = sources.art_definition.definitions.get(&art_id) else {
            continue;
        };
        for item in art_data.texture_items.iter().flatten() {
            let normalized_path = normalize_dictionary_path(&item.path);
            let package =
                physical_package_guess(&normalized_path, Some(item.id), &package_membership);
            let family = tileart_texture_family_name(item.texture_type);
            writer.write_record([
                "tileart",
                &art_data.id.to_string(),
                "",
                "",
                "",
                tile_type_name(art_data.tile_type),
                &format!("{:?}", art_data.flags),
                &item.block_index.to_string(),
                &item.item_index.to_string(),
                &item.id.to_string(),
                &item.path,
                &normalized_path,
                package,
                family,
                bool_str(item.is_auxiliary),
                &format_float(item.texture_stretch),
                &item.unk4.to_string(),
                &item.unk6.to_string(),
                &item.unk7.to_string(),
                "direct-textual",
            ])?;
            row_count += 1;
            increment_count(&mut owner_counts, "tileart");
            increment_count(&mut package_counts, package);
            increment_count(&mut family_counts, family);
        }
    }

    for entry in &sources.terrain_definition.entries {
        let Some(texture) = entry.texture.as_ref() else {
            continue;
        };
        let aliases = entry
            .runtime_slot_ids()
            .into_iter()
            .map(|alias| alias.to_string())
            .collect::<Vec<_>>()
            .join(";");
        let shader_name = texture.shader_name.as_deref().unwrap_or("");
        for (layer_index, layer) in texture.layers.iter().enumerate() {
            let raw_path = layer.path.as_deref().unwrap_or("");
            let normalized_path = normalize_dictionary_path(raw_path);
            let package =
                physical_package_guess(&normalized_path, layer.texture_id, &package_membership);
            let family = terrain_texture_family_name(layer.texture_type, package);
            writer.write_record([
                "terrain_definition",
                &entry.id.to_string(),
                entry.name.as_deref().unwrap_or(""),
                &aliases,
                shader_name,
                "",
                "",
                "0",
                &layer_index.to_string(),
                &layer
                    .texture_id
                    .map(|texture_id| texture_id.to_string())
                    .unwrap_or_default(),
                raw_path,
                &normalized_path,
                package,
                family,
                bool_str(terrain_layer_support_guess(raw_path)),
                &format_float(layer.texture_repetition),
                &layer.unk4.to_string(),
                &layer.unk6.to_string(),
                &layer.unk7.to_string(),
                "direct-textual",
            ])?;
            row_count += 1;
            increment_count(&mut owner_counts, "terrain_definition");
            increment_count(&mut package_counts, package);
            increment_count(&mut family_counts, family);
        }
    }

    writer.flush()?;

    info!(
        "wrote {row_count} direct EC material owner refs to '{}'",
        output.display()
    );
    log_count_summary("owner refs", &owner_counts);
    log_count_summary("physical package guesses", &package_counts);
    log_count_summary("logical families", &family_counts);

    Ok(())
}

pub fn inventory_ec_support_textures(
    source_dirs: &[PathBuf],
    terrain_output: &Path,
    effect_output: &Path,
) -> eyre::Result<()> {
    let sources = load_tex_art_ec_sources(source_dirs)?;
    let package_membership = EcPackageMembership::load(source_dirs)?;
    let owner_index = SupportOwnerIndex::build(&sources, &package_membership);

    let terrain_path = find_first_existing_file(source_dirs, &["TerrainTexture.uop"])
        .ok_or_else(|| eyre::eyre!("missing required file: TerrainTexture.uop"))?;
    let effect_path = find_first_existing_file(source_dirs, &["EffectTexture.uop"])
        .ok_or_else(|| eyre::eyre!("missing required file: EffectTexture.uop"))?;

    let terrain_package = UopPackage::load(&terrain_path)?;
    let effect_package = UopPackage::load(&effect_path)?;

    let terrain_summary = write_support_inventory(
        SupportPackageKind::TerrainTexture,
        &terrain_package,
        &owner_index,
        terrain_output,
    )?;
    let effect_summary = write_support_inventory(
        SupportPackageKind::EffectTexture,
        &effect_package,
        &owner_index,
        effect_output,
    )?;

    info!(
        "wrote {} TerrainTexture.uop resources to '{}'",
        terrain_summary.row_count,
        terrain_output.display()
    );
    log_inventory_summary("TerrainTexture.uop", &terrain_summary);
    info!(
        "wrote {} EffectTexture.uop resources to '{}'",
        effect_summary.row_count,
        effect_output.display()
    );
    log_inventory_summary("EffectTexture.uop", &effect_summary);

    Ok(())
}

pub fn write_ec_material_baseline_report(output: &Path) -> eyre::Result<()> {
    let report = r#"# EC Material Baseline And Metadata Axes

This report freezes current UODynamapper EC material behavior before role-aware
selection changes. It is a developer review artifact, not runtime pack data.

## Current Heuristics

### Item main EC texture chooser

Source: `lib/udd-assets/src/tilemeta.rs`.

Current order:
1. non-aux WorldArt ref matching the tile id
2. non-aux primary-selected WorldArt ref
3. any non-aux WorldArt ref
4. any non-aux primary-selected ref
5. any non-aux ref
6. legacy fallback to `TileMetaItemTile.ec_texture_id`

Current limitation: this is package/family biased and does not yet use stable
roles, role confidence, or explicit rejection reasons.

### TerrainDefinition primary layer chooser

Source: `lib/uocf/src/enhanced/terrain_definition.rs`.

Current order ranks layers by:
1. support-layer name heuristic
2. preferred repetition range
3. unknown field ordering
4. name string offset ordering

Current limitation: this guesses base/support from names and repetition but does
not preserve a role graph for later authored blend or liquid paths.

### `tex_art_ec` inclusion

Source: `lib/udd-conv/src/tex_art_ec.rs`.

Current behavior: `TileType::Liquid` is excluded wholesale.

Current limitation: art-owned liquid entries with visible bases are skipped
before base eligibility can be evaluated.

## Metadata Axes To Preserve

The following axes must stay independent:

- `physical_package`: `Texture.uop`, `LegacyTexture.uop`, `TerrainTexture.uop`,
  `EffectTexture.uop`, `SystemTextures`, `ShaderResources`, `Unknown`
- `logical_family`: `WorldArt`, `TileArtLegacy`, `TileArtEnhanced`, `Textures`,
  `Effects`, `SystemTextures`, `ShaderResources`, `Unknown`
- `stable_role`: `Base`, `SecondaryBase`, `AlphaMask`, `GenericMask`, `Noise`,
  `Detail`, `Overlay`, `NormalLike`, `ImageSupport`, `EffectOnlyMetadata`,
  `UnknownSupport`
- `speculative_role`: `LiquidRipple`, `LiquidReflectionSupport`,
  `LiquidEnvProbe`, `FoamHighlight`, `FlowMapLike`,
  `RefractionDistortionLike`, `WaterfallSupport`, `LavaBubbleSupport`,
  `PostBlendSupport`, `UnknownSupport`
- `role_reason`
- `confidence`
- `metadata_only`
- `decodable_image`
- `owner_kind`
- `owner_id`
- `source_path_raw`
- `source_path_normalized`
- `block_index`
- `item_or_layer_index`
- `stretch_or_repetition`
- `unknown_fields`

## Policy Frozen For Next Changes

- Direct UOP owner evidence is stronger than current Rust heuristics.
- Package membership is not ownership.
- `TerrainTexture.uop` resources may be art-owned support.
- `EffectTexture.uop` resources are currently inventory evidence unless a direct
  owner reference or indirect NIF/EMS link proves more.
- Speculative roles must not drive visibility or routing.
- First-pass base selection must use stable roles only and log/report fallback
  reasons explicitly.
"#;

    fs::write(output, report)?;
    info!(
        "wrote EC material baseline report to '{}'",
        output.display()
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SupportPackageKind {
    TerrainTexture,
    EffectTexture,
}

impl SupportPackageKind {
    fn package_name(self) -> &'static str {
        match self {
            Self::TerrainTexture => "TerrainTexture.uop",
            Self::EffectTexture => "EffectTexture.uop",
        }
    }

    fn logical_family(self) -> &'static str {
        match self {
            Self::TerrainTexture => "Textures",
            Self::EffectTexture => "Effects",
        }
    }

    fn hash_templates(self, texture_id: u32) -> Vec<String> {
        match self {
            Self::TerrainTexture => vec![format!("build/terraintexture/{texture_id:08}.dds")],
            Self::EffectTexture => vec![
                format!("build/effecttexture/{texture_id:08}.dds"),
                format!("build/effecttexture/{texture_id:08}.tga"),
                format!("build/effecttexture/{texture_id:08}.nif"),
                format!("build/effecttexture/{texture_id:08}.ems"),
                format!("build/effecttexture/{texture_id:08}.txt"),
            ],
        }
    }

    fn from_package_name(package_name: &str) -> Option<Self> {
        match package_name {
            "TerrainTexture.uop" => Some(Self::TerrainTexture),
            "EffectTexture.uop" => Some(Self::EffectTexture),
            _ => None,
        }
    }
}

#[derive(Default)]
struct SupportOwnerLinks {
    tileart_ids: BTreeSet<u32>,
    terrain_definition_ids: BTreeSet<u32>,
    source_paths: BTreeSet<String>,
}

#[derive(Default)]
struct SupportOwnerIndex {
    by_key: HashMap<(SupportPackageKind, u32), SupportOwnerLinks>,
}

impl SupportOwnerIndex {
    fn build(sources: &TexArtEcLoadedSources, membership: &EcPackageMembership) -> Self {
        let mut index = Self::default();

        for art_data in sources.art_definition.definitions.values() {
            for item in art_data.texture_items.iter().flatten() {
                let normalized_path = normalize_dictionary_path(&item.path);
                let package = physical_package_guess(&normalized_path, Some(item.id), membership);
                let Some(kind) = SupportPackageKind::from_package_name(package) else {
                    continue;
                };
                let links = index.by_key.entry((kind, item.id)).or_default();
                links.tileart_ids.insert(art_data.id as u32);
                links.source_paths.insert(item.path.clone());
            }
        }

        for entry in &sources.terrain_definition.entries {
            let Some(texture) = entry.texture.as_ref() else {
                continue;
            };
            for layer in &texture.layers {
                let Some(texture_id) = layer.texture_id else {
                    continue;
                };
                let raw_path = layer.path.as_deref().unwrap_or("");
                let normalized_path = normalize_dictionary_path(raw_path);
                let package =
                    physical_package_guess(&normalized_path, Some(texture_id), membership);
                let Some(kind) = SupportPackageKind::from_package_name(package) else {
                    continue;
                };
                let links = index.by_key.entry((kind, texture_id)).or_default();
                links.terrain_definition_ids.insert(entry.id);
                if !raw_path.is_empty() {
                    links.source_paths.insert(raw_path.to_string());
                }
            }
        }

        index
    }

    fn links(
        &self,
        kind: SupportPackageKind,
        texture_id: Option<u32>,
    ) -> Option<&SupportOwnerLinks> {
        texture_id.and_then(|id| self.by_key.get(&(kind, id)))
    }
}

#[derive(Default)]
struct InventorySummary {
    row_count: u64,
    decodable_images: u64,
    owner_linked: u64,
    kind_counts: BTreeMap<String, u64>,
    keyword_counts: BTreeMap<String, u64>,
    stable_role_counts: BTreeMap<String, u64>,
    speculative_role_counts: BTreeMap<String, u64>,
}

fn write_support_inventory(
    kind: SupportPackageKind,
    package: &UopPackage,
    owner_index: &SupportOwnerIndex,
    output: &Path,
) -> eyre::Result<InventorySummary> {
    let mut writer = WriterBuilder::new().from_path(output)?;
    writer.write_record([
        "physical_package",
        "block_index",
        "file_index",
        "filename_hash",
        "internal_uop_path",
        "normalized_basename",
        "texture_id",
        "inferred_file_kind",
        "decodable_image",
        "image_width",
        "image_height",
        "logical_family_guess",
        "provisional_stable_role_guess",
        "provisional_speculative_role_guess",
        "keyword_buckets",
        "direct_tileart_refs_count",
        "direct_tileart_ref_ids",
        "direct_terrain_definition_refs_count",
        "direct_terrain_definition_ref_ids",
        "reference_confidence",
        "decompressed_size",
        "notes",
    ])?;

    let mut summary = InventorySummary::default();
    let mut known_paths = known_support_paths(kind, package, owner_index);

    for (block_index, block) in package.blocks().iter().enumerate() {
        for (file_index, file) in block.files().iter().enumerate() {
            if !file.has_size() {
                continue;
            }

            let filename_hash = file.filename_hash();
            let internal_path = known_paths
                .remove(&filename_hash)
                .unwrap_or_else(|| format!("hash:0x{filename_hash:016x}"));
            let texture_id = extract_texture_id_for_inventory(&internal_path)
                .or_else(|| owner_id_from_hash(kind, filename_hash, owner_index));
            let owner_links = owner_index.links(kind, texture_id);
            let owner_name = owner_links
                .and_then(|links| links.source_paths.iter().next())
                .map(|path| normalized_basename(path));
            let basename = owner_name.unwrap_or_else(|| normalized_basename(&internal_path));
            let bytes = file.unpack().unwrap_or_default();
            let image_info = decode_image_info(&bytes);
            let payload_text = ascii_payload_probe(&bytes);
            let file_kind =
                infer_file_kind(&internal_path, &basename, &bytes, image_info.is_some());
            let keyword_probe = format!("{basename} {payload_text}");
            let keywords = keyword_buckets(&keyword_probe);
            let stable_role = provisional_stable_role(&file_kind, &keywords);
            let speculative_role = provisional_speculative_role(&keywords);
            let notes = inventory_notes(&internal_path, owner_links, image_info.is_some());

            let tileart_ids = owner_links
                .map(|links| join_u32_set(&links.tileart_ids))
                .unwrap_or_default();
            let terrain_ids = owner_links
                .map(|links| join_u32_set(&links.terrain_definition_ids))
                .unwrap_or_default();
            let tileart_count = owner_links
                .map(|links| links.tileart_ids.len())
                .unwrap_or(0);
            let terrain_count = owner_links
                .map(|links| links.terrain_definition_ids.len())
                .unwrap_or(0);
            let reference_confidence = if owner_links.is_some() {
                "direct-textual-id"
            } else if !internal_path.starts_with("hash:") {
                "package-hash-path"
            } else {
                "package-entry-only"
            };

            writer.write_record([
                kind.package_name(),
                &block_index.to_string(),
                &file_index.to_string(),
                &format!("0x{filename_hash:016x}"),
                &internal_path,
                &basename,
                &texture_id.map(|id| id.to_string()).unwrap_or_default(),
                file_kind,
                bool_str(image_info.is_some()),
                &image_info
                    .map(|(width, _)| width.to_string())
                    .unwrap_or_default(),
                &image_info
                    .map(|(_, height)| height.to_string())
                    .unwrap_or_default(),
                kind.logical_family(),
                stable_role,
                speculative_role,
                &keywords.join(";"),
                &tileart_count.to_string(),
                &tileart_ids,
                &terrain_count.to_string(),
                &terrain_ids,
                reference_confidence,
                &file.decompressed_size().to_string(),
                &notes,
            ])?;

            summary.row_count += 1;
            if image_info.is_some() {
                summary.decodable_images += 1;
            }
            if owner_links.is_some() {
                summary.owner_linked += 1;
            }
            increment_count(&mut summary.kind_counts, file_kind);
            increment_count(&mut summary.stable_role_counts, stable_role);
            increment_count(&mut summary.speculative_role_counts, speculative_role);
            for keyword in keywords {
                increment_count(&mut summary.keyword_counts, &keyword);
            }
        }
    }

    writer.flush()?;
    Ok(summary)
}

fn log_inventory_summary(label: &str, summary: &InventorySummary) {
    info!("{label}: resources={}", summary.row_count);
    info!("{label}: decodable_images={}", summary.decodable_images);
    info!(
        "{label}: directly_owner_linked_resources={}",
        summary.owner_linked
    );
    log_count_summary(&format!("{label}: file kinds"), &summary.kind_counts);
    log_count_summary(
        &format!("{label}: stable roles"),
        &summary.stable_role_counts,
    );
    log_count_summary(
        &format!("{label}: speculative roles"),
        &summary.speculative_role_counts,
    );
    log_count_summary(
        &format!("{label}: keyword buckets"),
        &summary.keyword_counts,
    );
}

fn known_support_paths(
    kind: SupportPackageKind,
    package: &UopPackage,
    owner_index: &SupportOwnerIndex,
) -> HashMap<u64, String> {
    let package_hashes = package
        .iter_files()
        .map(|file| file.filename_hash())
        .collect::<BTreeSet<_>>();
    let mut paths = HashMap::new();

    for ((owner_kind, texture_id), _) in &owner_index.by_key {
        if *owner_kind != kind {
            continue;
        }
        for candidate in kind.hash_templates(*texture_id) {
            let hash = hash_file_name_single(&candidate);
            if package_hashes.contains(&hash) {
                paths.insert(hash, candidate);
            }
        }
    }

    if kind == SupportPackageKind::TerrainTexture {
        for texture_id in 0..=4096 {
            let candidate = format!("build/terraintexture/{texture_id:08}.dds");
            let hash = hash_file_name_single(&candidate);
            if package_hashes.contains(&hash) {
                paths.entry(hash).or_insert(candidate);
            }
        }
    }

    paths
}

fn owner_id_from_hash(
    kind: SupportPackageKind,
    filename_hash: u64,
    owner_index: &SupportOwnerIndex,
) -> Option<u32> {
    owner_index
        .by_key
        .keys()
        .filter(|(candidate_kind, _)| *candidate_kind == kind)
        .find_map(|(_, texture_id)| {
            kind.hash_templates(*texture_id)
                .into_iter()
                .any(|path| hash_file_name_single(&path) == filename_hash)
                .then_some(*texture_id)
        })
}

fn extract_texture_id_for_inventory(path: &str) -> Option<u32> {
    if path.starts_with("hash:") {
        return None;
    }
    uocf::utils::path::extract_texture_id_from_path(path)
}

fn normalized_basename(path: &str) -> String {
    let normalized = normalize_dictionary_path(path);
    normalized
        .rsplit('\\')
        .next()
        .unwrap_or(normalized.as_str())
        .to_string()
}

fn decode_image_info(bytes: &[u8]) -> Option<(u32, u32)> {
    decode_image_info_from_slice(bytes).or_else(|| {
        let mut cursor = Cursor::new(bytes);
        EnhancedTextureItem::read(&mut cursor).ok()?;
        let pos = cursor.position() as usize;
        (pos < bytes.len())
            .then(|| decode_image_info_from_slice(&bytes[pos..]))
            .flatten()
    })
}

fn decode_image_info_from_slice(bytes: &[u8]) -> Option<(u32, u32)> {
    image::load(Cursor::new(bytes), image::ImageFormat::Dds)
        .or_else(|_| image::load(Cursor::new(bytes), image::ImageFormat::Tga))
        .ok()
        .map(|image| (image.width(), image.height()))
}

fn infer_file_kind(
    internal_path: &str,
    basename: &str,
    bytes: &[u8],
    decodable_image: bool,
) -> &'static str {
    if decodable_image {
        return "image";
    }

    let probe = format!(
        "{} {}",
        internal_path.to_ascii_lowercase(),
        basename.to_ascii_lowercase()
    );
    if bytes.starts_with(b"Gamebryo File Format")
        || probe.ends_with(".nif")
        || probe.contains(".nif")
    {
        "nif"
    } else if probe.ends_with(".ems") || probe.contains(".ems") {
        "ems"
    } else if probe.ends_with(".txt") || looks_like_text(bytes) {
        "text"
    } else if probe.contains("shader") {
        "shader_resource_like"
    } else {
        "unknown_binary"
    }
}

fn ascii_payload_probe(bytes: &[u8]) -> String {
    let sample = &bytes[..bytes.len().min(8192)];
    let mut out = String::new();
    let mut current = Vec::new();

    for byte in sample {
        if matches!(byte, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'-' | b'.' | b' ') {
            current.push(*byte);
        } else {
            if current.len() >= 4 {
                append_ascii_probe_word(&mut out, &current);
            }
            current.clear();
        }
    }
    if current.len() >= 4 {
        append_ascii_probe_word(&mut out, &current);
    }

    out
}

fn append_ascii_probe_word(out: &mut String, bytes: &[u8]) {
    if out.len() > 4096 {
        return;
    }
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(&String::from_utf8_lossy(bytes));
}

fn looks_like_text(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let sample = &bytes[..bytes.len().min(1024)];
    let printable = sample
        .iter()
        .filter(|byte| matches!(byte, b'\n' | b'\r' | b'\t' | 0x20..=0x7e))
        .count();
    printable * 100 / sample.len() > 85
}

fn keyword_buckets(name: &str) -> Vec<String> {
    const KEYWORDS: &[&str] = &[
        "ripple",
        "water_alpha",
        "water",
        "cube",
        "env",
        "sphere",
        "reflect",
        "reflection",
        "detail",
        "noise",
        "blur",
        "splash",
        "bubble",
        "waterfall",
        "lava",
        "flare",
        "glow",
        "mask",
        "alpha",
        "normal",
        "foam",
        "flow",
        "distort",
        "light",
    ];
    let lower = name.to_ascii_lowercase();
    KEYWORDS
        .iter()
        .filter(|keyword| lower.contains(**keyword))
        .map(|keyword| (*keyword).to_string())
        .collect()
}

fn provisional_stable_role(file_kind: &str, keywords: &[String]) -> &'static str {
    if file_kind != "image" {
        return "EffectOnlyMetadata";
    }
    if has_keyword(keywords, "normal") {
        "NormalLike"
    } else if has_keyword(keywords, "noise") {
        "Noise"
    } else if has_keyword(keywords, "alpha") {
        "AlphaMask"
    } else if has_keyword(keywords, "mask") {
        "GenericMask"
    } else if has_keyword(keywords, "detail") {
        "Detail"
    } else if has_keyword(keywords, "light") {
        "Overlay"
    } else {
        "ImageSupport"
    }
}

fn provisional_speculative_role(keywords: &[String]) -> &'static str {
    if has_keyword(keywords, "ripple") {
        "LiquidRipple"
    } else if has_keyword(keywords, "reflect") || has_keyword(keywords, "reflection") {
        "LiquidReflectionSupport"
    } else if has_keyword(keywords, "cube")
        || has_keyword(keywords, "env")
        || has_keyword(keywords, "sphere")
    {
        "LiquidEnvProbe"
    } else if has_keyword(keywords, "foam") || has_keyword(keywords, "splash") {
        "FoamHighlight"
    } else if has_keyword(keywords, "waterfall") {
        "WaterfallSupport"
    } else if has_keyword(keywords, "lava") || has_keyword(keywords, "bubble") {
        "LavaBubbleSupport"
    } else if has_keyword(keywords, "glow") || has_keyword(keywords, "flare") {
        "PostBlendSupport"
    } else {
        "UnknownSupport"
    }
}

fn has_keyword(keywords: &[String], keyword: &str) -> bool {
    keywords.iter().any(|value| value == keyword)
}

fn join_u32_set(values: &BTreeSet<u32>) -> String {
    values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(";")
}

fn inventory_notes(
    internal_path: &str,
    owner_links: Option<&SupportOwnerLinks>,
    decodable_image: bool,
) -> String {
    let mut notes = Vec::new();
    if internal_path.starts_with("hash:") {
        notes.push("unresolved_internal_path");
    }
    if owner_links.is_none() {
        notes.push("no_direct_owner_reference");
    }
    if !decodable_image {
        notes.push("not_decodable_as_dds_or_tga");
    }
    notes.join(";")
}

fn increment_count(counts: &mut BTreeMap<String, u64>, key: &str) {
    *counts.entry(key.to_string()).or_default() += 1;
}

fn log_count_summary(label: &str, counts: &BTreeMap<String, u64>) {
    for (key, count) in counts {
        info!("{label}: {key}={count}");
    }
}

fn bool_str(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn format_float(value: f32) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        value.to_string()
    }
}

fn tile_type_name(tile_type: TileType) -> &'static str {
    match tile_type {
        TileType::Static => "Static",
        TileType::Solid => "Solid",
        TileType::Liquid => "Liquid",
    }
}

fn tileart_texture_family_name(texture_type: TileArtTextureType) -> &'static str {
    match texture_type {
        TileArtTextureType::Undefined => "Unknown",
        TileArtTextureType::WorldArt => "WorldArt",
        TileArtTextureType::TileArtLegacy => "TileArtLegacy",
        TileArtTextureType::TileArtEnhanced => "TileArtEnhanced",
        TileArtTextureType::Textures => "Textures",
    }
}

fn terrain_texture_family_name(
    texture_type: TerrainTextureType,
    physical_package: &str,
) -> &'static str {
    match texture_type {
        TerrainTextureType::Undefined => logical_family_from_physical_package(physical_package),
        TerrainTextureType::WorldArt => "WorldArt",
        TerrainTextureType::TileArtLegacy => "TileArtLegacy",
        TerrainTextureType::TileArtEnhanced => "TileArtEnhanced",
        TerrainTextureType::Textures => "Textures",
    }
}

fn logical_family_from_physical_package(physical_package: &str) -> &'static str {
    match physical_package {
        "Texture.uop" => "WorldArt",
        "LegacyTexture.uop" => "TileArtLegacy",
        "TerrainTexture.uop" => "Textures",
        "EffectTexture.uop" => "Effects",
        "SystemTextures" => "SystemTextures",
        "ShaderResources" => "ShaderResources",
        _ => "Unknown",
    }
}

fn terrain_layer_support_guess(path: &str) -> bool {
    let normalized = normalize_dictionary_path(path);
    let file_name = normalized
        .rsplit('\\')
        .next()
        .unwrap_or(normalized.as_str());
    let stem = file_name.split('.').next().unwrap_or(file_name);
    stem.contains("noise")
        || stem.contains("normal")
        || stem.contains("mask")
        || stem.contains("_alpha")
}

struct EcPackageMembership {
    texture: Option<UopPackage>,
    legacy_texture: Option<UopPackage>,
    terrain_texture: Option<UopPackage>,
    effect_texture: Option<UopPackage>,
}

impl EcPackageMembership {
    fn load(source_dirs: &[PathBuf]) -> eyre::Result<Self> {
        Ok(Self {
            texture: load_optional_uop(source_dirs, &["Texture.uop"])?,
            legacy_texture: load_optional_uop(source_dirs, &["LegacyTexture.uop"])?,
            terrain_texture: load_optional_uop(source_dirs, &["TerrainTexture.uop"])?,
            effect_texture: load_optional_uop(source_dirs, &["EffectTexture.uop"])?,
        })
    }

    fn contains_path(&self, package: &Option<UopPackage>, path: &str) -> bool {
        let Some(package) = package.as_ref() else {
            return false;
        };
        let hash = hash_file_name_single(path);
        package.get_file_by_hash(hash).is_some()
    }
}

fn load_optional_uop(
    source_dirs: &[PathBuf],
    file_names: &[&str],
) -> eyre::Result<Option<UopPackage>> {
    Ok(find_first_existing_file(source_dirs, file_names)
        .map(|path| UopPackage::load(&path))
        .transpose()?)
}

fn physical_package_guess(
    normalized_path: &str,
    texture_id: Option<u32>,
    membership: &EcPackageMembership,
) -> &'static str {
    if normalized_path.contains("data\\worldart\\") {
        "Texture.uop"
    } else if normalized_path.contains("data\\tileartlegacy\\")
        || normalized_path.contains("data\\legacyland\\")
    {
        "LegacyTexture.uop"
    } else if normalized_path.contains("data\\textures\\")
        || normalized_path.contains("data\\terraintexture\\")
    {
        "TerrainTexture.uop"
    } else if normalized_path.contains("data\\effecttexture\\")
        || normalized_path.contains("data\\effects\\")
    {
        "EffectTexture.uop"
    } else if normalized_path.contains("data\\systemtextures\\") {
        "SystemTextures"
    } else if normalized_path.contains("shader") {
        "ShaderResources"
    } else if let Some(texture_id) = texture_id {
        physical_package_by_hash_membership(texture_id, membership)
    } else {
        "Unknown"
    }
}

fn physical_package_by_hash_membership(
    texture_id: u32,
    membership: &EcPackageMembership,
) -> &'static str {
    let worldart_path = format!("build/worldart/{texture_id:08}.dds");
    if membership.contains_path(&membership.texture, &worldart_path) {
        return "Texture.uop";
    }

    let land_path = format!("build/worldart/land/{texture_id:08}.dds");
    if membership.contains_path(&membership.texture, &land_path) {
        return "Texture.uop";
    }

    let legacy_path = format!("build/tileartlegacy/{texture_id:08}.dds");
    if membership.contains_path(&membership.legacy_texture, &legacy_path) {
        return "LegacyTexture.uop";
    }

    let legacy_land_path = format!("build/legacyland/{texture_id:08}.dat");
    if membership.contains_path(&membership.legacy_texture, &legacy_land_path) {
        return "LegacyTexture.uop";
    }

    let terrain_texture_path = format!("build/terraintexture/{texture_id:08}.dds");
    if membership.contains_path(&membership.terrain_texture, &terrain_texture_path) {
        return "TerrainTexture.uop";
    }

    let effect_texture_path = format!("build/effecttexture/{texture_id:08}.dds");
    if membership.contains_path(&membership.effect_texture, &effect_texture_path) {
        return "EffectTexture.uop";
    }

    "Unknown"
}
