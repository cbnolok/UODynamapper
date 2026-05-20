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
use serde::Serialize;
use udd_assets::{
    cc_tex_land_ec_transcode::{LayerDef, TerrainDefEntry, TerrainDefinitionKdl},
    ec_surface_overrides::{
        CcArtOverrideMode, EcSurfaceOverrideAction, EcSurfaceOverrideEntry, EcSurfaceOverrides,
    },
    tex_art_ec::TexArtEcPackage,
    tex_land_ec::{
        TexLandEcPackage, TexLandEcTerrainProvenanceRecord, MISSING_SLOT_ID,
        MISSING_TERRAIN_LAYER_INDEX, MISSING_TEXTURE_ID, TERRAIN_PRIMARY_FLAG_FALLBACK_REASON,
        TERRAIN_PRIMARY_FLAG_MULTIPLE_PREFERRED_NON_SUPPORT,
        TERRAIN_PRIMARY_FLAG_OPAQUE_UNK6_TIEBREAKER,
        TERRAIN_PRIMARY_FLAG_SELECTED_CURRENT_SUPPORT,
        TERRAIN_PRIMARY_FLAG_SELECTED_PREFERRED_REPETITION,
        TERRAIN_PRIMARY_FLAG_SELECTED_SUPPORT_LIKE,
        TERRAIN_PRIMARY_FLAG_SUPPORT_LIKE_OUTSIDE_CURRENT_HEURISTIC,
    },
    tilemeta::{TileMetaItemTile, TileMetaPackage},
};
use udd_conv::source_paths::find_first_existing_file;
use udd_conv::tex_art_ec::{load_tex_art_ec_sources, TexArtEcLoadedSources};
use uocf::{
    enhanced::{
        terrain_definition::{
            TerrainDefinitionEntry, TerrainDefinitionPrimaryLayerReason,
            TerrainDefinitionTextureLayer, TerrainTextureType,
        },
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

pub fn audit_ec_terrain_primary_selection(
    source_dirs: &[PathBuf],
    output: &Path,
) -> eyre::Result<()> {
    let sources = load_tex_art_ec_sources(source_dirs)?;
    let package_membership = EcPackageMembership::load(source_dirs)?;
    let mut entries = Vec::new();
    let mut reason_counts = BTreeMap::<String, u64>::new();
    let mut flag_counts = BTreeMap::<String, u64>::new();
    let mut shape_counts = BTreeMap::<String, u64>::new();
    let mut selected_support_like_count = 0u64;
    let mut selected_texture_count = 0u64;

    for entry in &sources.terrain_definition.entries {
        let Some(texture) = entry.texture.as_ref() else {
            continue;
        };
        let primary = entry.primary_texture_layer_with_reason();
        let (selected_layer_index, selected_layer, selection_reason) = primary
            .and_then(|(layer, reason)| {
                texture.layers
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, layer))
                    .map(|index| (Some(index), Some(layer), Some(reason)))
            })
            .unwrap_or((None, None, None));

        let runtime_slot_ids = entry.runtime_slot_ids();
        let alias_record_count = entry.aliases.len();
        let zero_alias_count = entry.aliases.iter().filter(|alias| alias.alias == 0).count();
        let concrete_alias_count = entry.aliases.iter().filter(|alias| alias.alias != 0).count();
        let texture_layer_count = texture
            .layers
            .iter()
            .filter(|layer| layer.texture_id.is_some())
            .count();
        let unique_texture_id_count = texture
            .layers
            .iter()
            .filter_map(|layer| layer.texture_id)
            .collect::<BTreeSet<_>>()
            .len();
        let non_support_texture_layer_count = texture
            .layers
            .iter()
            .filter(|layer| {
                layer.texture_id.is_some()
                    && !layer.is_support_layer_by_current_name_heuristic()
                    && !layer.has_support_like_name_clue()
            })
            .count();
        let support_like_layer_count = texture
            .layers
            .iter()
            .filter(|layer| layer.has_support_like_name_clue())
            .count();
        let current_support_layer_count = texture
            .layers
            .iter()
            .filter(|layer| layer.is_support_layer_by_current_name_heuristic())
            .count();
        let preferred_repetition_layer_count = texture
            .layers
            .iter()
            .filter(|layer| layer.has_preferred_primary_repetition())
            .count();
        let audit_flags = terrain_primary_audit_flags(entry, selected_layer, selection_reason);
        let audit_flag_values = split_flags(&audit_flags);
        let runtime_slot_count = runtime_slot_ids.len();
        let material_shape = terrain_material_shape(
            alias_record_count,
            concrete_alias_count,
            texture_layer_count,
            unique_texture_id_count,
            non_support_texture_layer_count,
            support_like_layer_count,
        );

        if let Some(reason) = selection_reason {
            increment_count(&mut reason_counts, reason.as_str());
        } else {
            increment_count(&mut reason_counts, "missing");
        }
        for flag in &audit_flag_values {
            increment_count(&mut flag_counts, flag);
        }
        increment_count(&mut shape_counts, material_shape);
        if let Some(layer) = selected_layer {
            selected_texture_count += 1;
            if layer.has_support_like_name_clue() {
                selected_support_like_count += 1;
            }
        }

        let selected_path = selected_layer
            .and_then(|layer| layer.path.as_deref())
            .unwrap_or("");
        let normalized_path = normalize_dictionary_path(selected_path);
        let selected_package = selected_layer
            .map(|layer| {
                physical_package_guess(&normalized_path, layer.texture_id, &package_membership)
            })
            .unwrap_or("Unknown");
        let selected_family = selected_layer
            .map(|layer| terrain_texture_family_name(layer.texture_type, selected_package))
            .unwrap_or("Unknown");

        entries.push(TerrainPrimarySelectionEntryReport {
            material_id: entry.id,
            material_name: entry.name.clone(),
            alias_records: entry
                .aliases
                .iter()
                .map(|alias| TerrainAliasReport {
                    count_index: alias.count_index,
                    alias: alias.alias,
                    tile_flags: alias.tile_flags,
                    is_placeholder: alias.alias == 0,
                })
                .collect(),
            runtime_slot_ids,
            alias_summary: TerrainAliasSummaryReport {
                alias_record_count,
                zero_alias_count,
                concrete_alias_count,
                has_alias_records: alias_record_count > 0,
                has_concrete_aliases: concrete_alias_count > 0,
                runtime_slot_count,
            },
            shader_name: texture.shader_name.clone(),
            material_shape: material_shape.to_string(),
            selected_layer: selected_layer.map(|layer| TerrainSelectedLayerReport {
                layer_index: selected_layer_index.unwrap_or_default(),
                texture_id: layer.texture_id,
                path: layer.path.clone(),
                selection_reason: selection_reason
                    .map(|reason| reason.as_str().to_string())
                    .unwrap_or_else(|| "missing".to_string()),
                current_support_heuristic: layer.is_support_layer_by_current_name_heuristic(),
                support_like_clue: layer.has_support_like_name_clue(),
                preferred_repetition: layer.has_preferred_primary_repetition(),
                texture_repetition: layer.texture_repetition,
                unk4: layer.unk4,
                unk6_opaque_order: layer.unk6,
                unk7: layer.unk7,
                physical_package_guess: selected_package.to_string(),
                logical_family: selected_family.to_string(),
            }),
            layer_summary: TerrainLayerSummaryReport {
                layer_count: texture.layers.len(),
                texture_layer_count,
                unique_texture_id_count,
                non_support_texture_layer_count,
                support_like_layer_count,
                current_support_layer_count,
                preferred_repetition_layer_count,
            },
            layers: texture
                .layers
                .iter()
                .enumerate()
                .map(|(layer_index, layer)| {
                    terrain_layer_report(layer_index, layer, &package_membership)
                })
                .collect(),
            audit_flags: audit_flag_values,
        });
    }

    let row_count = entries.len() as u64;
    let report = TerrainPrimarySelectionReport {
        schema: "ec_terrain_primary_selection",
        schema_version: 1,
        summary: TerrainPrimarySelectionSummary {
            material_count: row_count as usize,
            selection_reason_counts: reason_counts.clone(),
            audit_flag_counts: flag_counts.clone(),
            material_shape_counts: shape_counts.clone(),
            selected_support_like_count: selected_support_like_count as usize,
            selected_texture_count: selected_texture_count as usize,
        },
        entries,
    };
    let json = serde_json::to_vec_pretty(&report)?;
    fs::write(output, json)?;

    info!(
        "wrote {row_count} EC terrain primary-selection JSON entries to '{}'",
        output.display()
    );
    log_count_summary("terrain primary selection reasons", &reason_counts);
    log_count_summary("terrain primary audit flags", &flag_counts);
    log_count_summary("terrain material shapes", &shape_counts);
    if selected_texture_count > 0 {
        let percent = selected_support_like_count as f64 * 100.0 / selected_texture_count as f64;
        info!(
            "terrain primary selections with support-like clues: {selected_support_like_count}/{selected_texture_count} ({percent:.2}%)"
        );
    }

    Ok(())
}

pub fn audit_ec_terrain_definition_kdl(
    source_dirs: &[PathBuf],
    kdl_path: &Path,
    output: &Path,
) -> eyre::Result<()> {
    let terrain_path = find_first_existing_file(source_dirs, &["TerrainDefinition.uop"])
        .ok_or_else(|| eyre::eyre!("missing TerrainDefinition.uop"))?;
    let uop = uocf::enhanced::terrain_definition::TerrainDefinitionPackage::load(&terrain_path)?;
    let kdl = TerrainDefinitionKdl::load(kdl_path)?;
    let uop_by_id = uop
        .entries
        .iter()
        .map(|entry| (entry.id, entry))
        .collect::<HashMap<_, _>>();
    let kdl_by_id = kdl
        .entries
        .iter()
        .map(|entry| (entry.id, entry))
        .collect::<HashMap<_, _>>();
    let ids = uop_by_id
        .keys()
        .copied()
        .chain(kdl_by_id.keys().copied())
        .collect::<BTreeSet<_>>();

    let mut entries = Vec::new();
    let mut finding_counts = BTreeMap::<String, u64>::new();
    let mut manual_candidate_counts = BTreeMap::<String, u64>::new();
    let mut manual_candidates = Vec::new();
    let mut matched_id_count = 0usize;
    let mut kdl_only_count = 0usize;
    let mut uop_only_count = 0usize;

    for id in ids {
        let kdl_entry = kdl_by_id.get(&id).copied();
        let uop_entry = uop_by_id.get(&id).copied();
        match (kdl_entry.is_some(), uop_entry.is_some()) {
            (true, true) => matched_id_count += 1,
            (true, false) => kdl_only_count += 1,
            (false, true) => uop_only_count += 1,
            (false, false) => {}
        }

        let findings = terrain_definition_kdl_findings(kdl_entry, uop_entry);
        for finding in &findings {
            increment_count(&mut finding_counts, &finding.status);
            if terrain_kdl_finding_needs_manual_review(&finding.status) {
                increment_count(&mut manual_candidate_counts, &finding.status);
                manual_candidates.push(terrain_definition_manual_candidate_report(
                    id, kdl_entry, uop_entry, finding,
                ));
            }
        }

        entries.push(TerrainDefinitionKdlAuditEntry {
            id,
            kdl_present: kdl_entry.is_some(),
            uop_present: uop_entry.is_some(),
            kdl: kdl_entry.map(terrain_definition_kdl_entry_report),
            uop: uop_entry.map(terrain_definition_uop_entry_report),
            findings,
        });
    }

    let report = TerrainDefinitionKdlAuditReport {
        schema: "ec_terrain_definition_kdl_audit",
        schema_version: 1,
        summary: TerrainDefinitionKdlAuditSummary {
            kdl_entry_count: kdl.entries.len(),
            uop_entry_count: uop.entries.len(),
            matched_id_count,
            kdl_only_count,
            uop_only_count,
            finding_counts: finding_counts.clone(),
            manual_candidate_counts: manual_candidate_counts.clone(),
        },
        entries,
        manual_candidates,
    };
    let json = serde_json::to_vec_pretty(&report)?;
    fs::write(output, json)?;

    info!(
        "wrote TerrainDefinition KDL audit JSON to '{}'",
        output.display()
    );
    log_count_summary("TerrainDefinition KDL audit findings", &finding_counts);
    log_count_summary(
        "TerrainDefinition manual candidate findings",
        &manual_candidate_counts,
    );
    Ok(())
}

pub fn audit_ec_surface_redirection(
    source_dirs: &[PathBuf],
    tilemeta_path: &Path,
    tex_art_ec_path: Option<&Path>,
    tex_land_ec_path: &Path,
    surface_overrides_path: Option<&Path>,
    output: &Path,
) -> eyre::Result<()> {
    let sources = load_tex_art_ec_sources(source_dirs)?;
    let tilemeta = TileMetaPackage::load(tilemeta_path)?;
    let tex_art_ec = tex_art_ec_path
        .map(TexArtEcPackage::load)
        .transpose()?;
    let tex_land_ec = TexLandEcPackage::load(tex_land_ec_path)?;
    let surface_overrides = surface_overrides_path
        .map(EcSurfaceOverrides::load)
        .transpose()?
        .map(|overrides| overrides.to_map())
        .unwrap_or_default();
    let item_by_id = tilemeta
        .item_tiles()
        .iter()
        .map(|item| (item.tile_id, item))
        .collect::<HashMap<_, _>>();
    let mut entries = Vec::new();
    let mut routed_count = 0u64;
    let mut flag_counts = BTreeMap::<String, u64>::new();
    let mut route_decision_counts = BTreeMap::<String, u64>::new();
    let mut decision_reason_counts = BTreeMap::<String, u64>::new();
    let mut review_class_counts = BTreeMap::<String, u64>::new();
    let mut override_action_counts = BTreeMap::<String, u64>::new();

    for item in tilemeta
        .item_tiles()
        .iter()
        .filter(|item| item.tile_id != 0 && item.is_surface_like())
    {
        let art_id = item.tile_id;
        let art_data = sources.art_definition.definitions.get(&(art_id as u16));
        let (main_ec_texture_id, main_reason, main_source) =
            surface_main_ec_texture(&tilemeta, item);
        let terrain_matches = main_ec_texture_id
            .map(|texture_id| terrain_matches_for_texture(&sources, texture_id))
            .unwrap_or_default();
        let direct_slot_present = main_ec_texture_id
            .is_some_and(|texture_id| tex_land_ec.present_slot(texture_id).is_some());
        let provenance_records = main_ec_texture_id
            .map(|texture_id| {
                tex_land_ec
                    .terrain_provenance()
                    .iter()
                    .filter(|record| record.selected_texture_id == texture_id)
                    .copied()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let canonical_slots = present_canonical_slots(&tex_land_ec, &provenance_records);
        let alias_slots = present_alias_slots(&tex_land_ec, &provenance_records);
        let fallback_slot = tex_land_ec.resolve_runtime_slot_id(item.cc_texture_id);
        let art_availability =
            surface_art_availability(tex_art_ec.as_ref(), item, main_ec_texture_id);
        let resolution = resolve_surface_redirection_like_runtime(
            &tex_land_ec,
            item,
            main_ec_texture_id,
            direct_slot_present,
            canonical_slots.as_slice(),
            alias_slots.as_slice(),
            fallback_slot,
            art_availability.art_id_tex_art_slot_kind.as_deref() == Some("land"),
        );
        let abnormality_flags = surface_redirection_flags(
            item,
            main_ec_texture_id,
            direct_slot_present,
            &terrain_matches,
            &provenance_records,
            canonical_slots.as_slice(),
            alias_slots.as_slice(),
            resolution.slot_id,
        );
        let abnormality_flag_values = split_flags(&abnormality_flags);
        let primary_match_count = terrain_matches
            .iter()
            .filter(|match_row| match_row.primary_match)
            .count();
        let layer_only_match_count = terrain_matches.len() - primary_match_count;
        let review_class = surface_review_class(
            item,
            art_data.map(|data| data.tile_type),
            &art_availability,
            resolution.slot_id,
            terrain_matches.as_slice(),
        );
        let manual_override = surface_override_report(
            surface_overrides.get(&art_id),
            &surface_overrides,
            &item_by_id,
            tex_art_ec.as_ref(),
            &tex_land_ec,
        );

        if resolution.slot_id.is_some() {
            routed_count += 1;
        }
        for flag in &abnormality_flag_values {
            increment_count(&mut flag_counts, flag);
        }
        increment_count(&mut route_decision_counts, resolution.route_decision);
        increment_count(&mut decision_reason_counts, resolution.decision_reason);
        increment_count(&mut review_class_counts, review_class);
        if let Some(manual_override) = &manual_override {
            increment_count(&mut override_action_counts, &manual_override.action);
        }

        entries.push(SurfaceRedirectionEntryReport {
            art_id,
            tile_type: art_data
                .map(|data| tile_type_name(data.tile_type).to_string()),
            tile_flags: art_data.map(|data| format!("{:?}", data.flags)),
            tilemeta: SurfaceTileMetaReport {
                visual_kind: format!("{:?}", item.visual_kind()),
                surface_like: item.is_surface_like(),
                name: item.name_ascii().to_string(),
                cc_texture_id: optional_nonzero_u32(item.cc_texture_id),
                legacy_ec_texture_id: optional_nonzero_u32(item.ec_texture_id),
            },
            main_ec_texture: SurfaceMainTextureReport {
                texture_id: main_ec_texture_id,
                reason: main_reason.unwrap_or_else(|| "missing".to_string()),
                source: main_source.to_string(),
                direct_tex_land_slot_present: direct_slot_present,
            },
            art_availability,
            terrain_definition: SurfaceTerrainDefinitionReport {
                match_count: terrain_matches.len(),
                primary_match_count,
                layer_only_match_count,
                matches: terrain_matches
                    .iter()
                    .map(surface_terrain_match_report)
                    .collect(),
            },
            provenance: SurfaceProvenanceReport {
                records_considered: provenance_records
                    .iter()
                    .map(surface_provenance_record_report)
                    .collect(),
                canonical_slots_present: canonical_slots,
                alias_slots_present: alias_slots,
            },
            fallback_resolve_runtime_slot: fallback_slot,
            resolution: SurfaceResolutionReport {
                resolved_runtime_slot: resolution.slot_id,
                route_decision: resolution.route_decision.to_string(),
                decision_reason: resolution.decision_reason.to_string(),
            },
            manual_override,
            review_class: review_class.to_string(),
            abnormality_flags: abnormality_flag_values,
        });
    }

    let row_count = entries.len() as u64;
    let report = SurfaceRedirectionReport {
        schema: "ec_surface_redirection",
        schema_version: 1,
        summary: SurfaceRedirectionSummary {
            surface_like_count: row_count as usize,
            resolved_count: routed_count as usize,
            unresolved_count: (row_count - routed_count) as usize,
            route_decision_counts: route_decision_counts.clone(),
            decision_reason_counts: decision_reason_counts.clone(),
            review_class_counts: review_class_counts.clone(),
            manual_override_action_counts: override_action_counts.clone(),
            abnormality_flag_counts: flag_counts.clone(),
        },
        entries,
    };
    let json = serde_json::to_vec_pretty(&report)?;
    fs::write(output, json)?;

    info!(
        "wrote {row_count} EC surface redirection JSON entries to '{}'",
        output.display()
    );
    info!("EC surface redirection resolved rows: {routed_count}/{row_count}");
    log_count_summary("EC surface redirection route decisions", &route_decision_counts);
    log_count_summary("EC surface redirection decision reasons", &decision_reason_counts);
    log_count_summary("EC surface redirection review classes", &review_class_counts);
    log_count_summary(
        "EC surface redirection manual override actions",
        &override_action_counts,
    );
    log_count_summary("EC surface redirection flags", &flag_counts);

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

#[derive(Serialize)]
struct TerrainPrimarySelectionReport {
    schema: &'static str,
    schema_version: u32,
    summary: TerrainPrimarySelectionSummary,
    entries: Vec<TerrainPrimarySelectionEntryReport>,
}

#[derive(Serialize)]
struct TerrainPrimarySelectionSummary {
    material_count: usize,
    selection_reason_counts: BTreeMap<String, u64>,
    audit_flag_counts: BTreeMap<String, u64>,
    material_shape_counts: BTreeMap<String, u64>,
    selected_support_like_count: usize,
    selected_texture_count: usize,
}

#[derive(Serialize)]
struct TerrainPrimarySelectionEntryReport {
    material_id: u32,
    material_name: Option<String>,
    alias_records: Vec<TerrainAliasReport>,
    runtime_slot_ids: Vec<u32>,
    alias_summary: TerrainAliasSummaryReport,
    shader_name: Option<String>,
    material_shape: String,
    selected_layer: Option<TerrainSelectedLayerReport>,
    layer_summary: TerrainLayerSummaryReport,
    layers: Vec<TerrainLayerReport>,
    audit_flags: Vec<String>,
}

#[derive(Serialize)]
struct TerrainAliasReport {
    count_index: u32,
    alias: u32,
    tile_flags: u64,
    is_placeholder: bool,
}

#[derive(Serialize)]
struct TerrainAliasSummaryReport {
    alias_record_count: usize,
    zero_alias_count: usize,
    concrete_alias_count: usize,
    has_alias_records: bool,
    has_concrete_aliases: bool,
    runtime_slot_count: usize,
}

#[derive(Serialize)]
struct TerrainSelectedLayerReport {
    layer_index: usize,
    texture_id: Option<u32>,
    path: Option<String>,
    selection_reason: String,
    current_support_heuristic: bool,
    support_like_clue: bool,
    preferred_repetition: bool,
    texture_repetition: f32,
    unk4: u8,
    unk6_opaque_order: i32,
    unk7: i32,
    physical_package_guess: String,
    logical_family: String,
}

#[derive(Serialize)]
struct TerrainLayerSummaryReport {
    layer_count: usize,
    texture_layer_count: usize,
    unique_texture_id_count: usize,
    non_support_texture_layer_count: usize,
    support_like_layer_count: usize,
    current_support_layer_count: usize,
    preferred_repetition_layer_count: usize,
}

#[derive(Serialize)]
struct TerrainLayerReport {
    layer_index: usize,
    texture_id: Option<u32>,
    path: Option<String>,
    texture_type: String,
    current_support_heuristic: bool,
    support_like_clue: bool,
    preferred_repetition: bool,
    texture_repetition: f32,
    unk4: u8,
    unk6_opaque_order: i32,
    unk7: i32,
    physical_package_guess: String,
    logical_family: String,
}

#[derive(Serialize)]
struct TerrainDefinitionKdlAuditReport {
    schema: &'static str,
    schema_version: u32,
    summary: TerrainDefinitionKdlAuditSummary,
    entries: Vec<TerrainDefinitionKdlAuditEntry>,
    manual_candidates: Vec<TerrainDefinitionManualCandidateReport>,
}

#[derive(Serialize)]
struct TerrainDefinitionKdlAuditSummary {
    kdl_entry_count: usize,
    uop_entry_count: usize,
    matched_id_count: usize,
    kdl_only_count: usize,
    uop_only_count: usize,
    finding_counts: BTreeMap<String, u64>,
    manual_candidate_counts: BTreeMap<String, u64>,
}

#[derive(Serialize)]
struct TerrainDefinitionKdlAuditEntry {
    id: u32,
    kdl_present: bool,
    uop_present: bool,
    kdl: Option<TerrainDefinitionKdlEntryReport>,
    uop: Option<TerrainDefinitionUopEntryReport>,
    findings: Vec<TerrainDefinitionKdlFindingReport>,
}

#[derive(Serialize)]
struct TerrainDefinitionKdlEntryReport {
    terrain_type: String,
    flags: Vec<String>,
    speed: Option<f32>,
    waveheight: Option<f32>,
    textureid: Option<u32>,
    layers: Vec<TerrainDefinitionKdlLayerReport>,
}

#[derive(Serialize)]
struct TerrainDefinitionKdlLayerReport {
    role: &'static str,
    texture_id: u32,
    stretch: f32,
}

#[derive(Serialize)]
struct TerrainDefinitionUopEntryReport {
    name: Option<String>,
    unknown_floats: [f32; 3],
    alias_record_count: usize,
    concrete_alias_count: usize,
    runtime_slot_ids: Vec<u32>,
    shader_name: Option<String>,
    layers: Vec<TerrainDefinitionUopLayerReport>,
}

#[derive(Serialize)]
struct TerrainDefinitionUopLayerReport {
    layer_index: usize,
    texture_id: Option<u32>,
    repetition: f32,
    path: Option<String>,
    support_like_name: bool,
    current_support_heuristic: bool,
    unk4: u8,
    unk6: i32,
    unk7: i32,
}

#[derive(Serialize)]
struct TerrainDefinitionKdlFindingReport {
    field: String,
    status: String,
    detail: String,
}

#[derive(Serialize)]
struct TerrainDefinitionManualCandidateReport {
    id: u32,
    field: String,
    code: String,
    detail: String,
    kdl_terrain_type: Option<String>,
    uop_shader_name: Option<String>,
    runtime_slot_ids: Vec<u32>,
}

#[derive(Serialize)]
struct SurfaceRedirectionReport {
    schema: &'static str,
    schema_version: u32,
    summary: SurfaceRedirectionSummary,
    entries: Vec<SurfaceRedirectionEntryReport>,
}

#[derive(Serialize)]
struct SurfaceRedirectionSummary {
    surface_like_count: usize,
    resolved_count: usize,
    unresolved_count: usize,
    route_decision_counts: BTreeMap<String, u64>,
    decision_reason_counts: BTreeMap<String, u64>,
    review_class_counts: BTreeMap<String, u64>,
    manual_override_action_counts: BTreeMap<String, u64>,
    abnormality_flag_counts: BTreeMap<String, u64>,
}

#[derive(Serialize)]
struct SurfaceRedirectionEntryReport {
    art_id: u32,
    tile_type: Option<String>,
    tile_flags: Option<String>,
    tilemeta: SurfaceTileMetaReport,
    main_ec_texture: SurfaceMainTextureReport,
    art_availability: SurfaceArtAvailabilityReport,
    terrain_definition: SurfaceTerrainDefinitionReport,
    provenance: SurfaceProvenanceReport,
    fallback_resolve_runtime_slot: Option<u32>,
    resolution: SurfaceResolutionReport,
    manual_override: Option<SurfaceManualOverrideReport>,
    review_class: String,
    abnormality_flags: Vec<String>,
}

#[derive(Serialize)]
struct SurfaceManualOverrideReport {
    action: String,
    target_id: Option<u32>,
    mode: Option<String>,
    package: Option<String>,
    reason_code: Option<String>,
    valid: bool,
    validation_flags: Vec<String>,
    resolved_preview: Option<SurfaceManualOverridePreviewReport>,
}

#[derive(Serialize)]
struct SurfaceManualOverridePreviewReport {
    target_exists_in_tilemeta: bool,
    target_tex_art_slot_kind: Option<String>,
    target_tex_land_runtime_slot: Option<u32>,
}

#[derive(Serialize)]
struct SurfaceTileMetaReport {
    visual_kind: String,
    surface_like: bool,
    name: String,
    cc_texture_id: Option<u32>,
    legacy_ec_texture_id: Option<u32>,
}

#[derive(Serialize)]
struct SurfaceMainTextureReport {
    texture_id: Option<u32>,
    reason: String,
    source: String,
    direct_tex_land_slot_present: bool,
}

#[derive(Serialize)]
struct SurfaceArtAvailabilityReport {
    ec_art_package_provided: bool,
    art_id_tex_art_slot_present: Option<bool>,
    art_id_tex_art_slot_kind: Option<String>,
    selected_texture_tex_art_slot_present: Option<bool>,
    selected_texture_tex_art_slot_kind: Option<String>,
    legacy_ec_texture_tex_art_slot_present: Option<bool>,
    legacy_ec_texture_tex_art_slot_kind: Option<String>,
}

#[derive(Serialize)]
struct SurfaceTerrainDefinitionReport {
    match_count: usize,
    primary_match_count: usize,
    layer_only_match_count: usize,
    matches: Vec<SurfaceTerrainMatchReport>,
}

#[derive(Serialize)]
struct SurfaceTerrainMatchReport {
    material_id: u32,
    material_name: String,
    layer_index: usize,
    primary_match: bool,
    aliases: Vec<u32>,
    layer_path: String,
    layer_current_support: bool,
    layer_support_like: bool,
    primary_texture_id: Option<u32>,
    primary_reason: String,
}

#[derive(Serialize)]
struct SurfaceProvenanceReport {
    records_considered: Vec<SurfaceProvenanceRecordReport>,
    canonical_slots_present: Vec<u32>,
    alias_slots_present: Vec<u32>,
}

#[derive(Serialize)]
struct SurfaceProvenanceRecordReport {
    material_id: u32,
    material_name_id: i32,
    alias_count_index: u32,
    alias_slot_id: Option<u32>,
    alias_tile_flags: u64,
    selected_texture_id: Option<u32>,
    canonical_slot_id: Option<u32>,
    primary_texture_id: Option<u32>,
    primary_layer_index: Option<u32>,
    primary_selection_reason: String,
    primary_selection_flags: Vec<String>,
}

#[derive(Serialize)]
struct SurfaceResolutionReport {
    resolved_runtime_slot: Option<u32>,
    route_decision: String,
    decision_reason: String,
}

struct TerrainTextureMatch {
    material_id: u32,
    material_name: String,
    layer_index: usize,
    primary_match: bool,
    aliases: Vec<u32>,
    layer_path: String,
    layer_current_support: bool,
    layer_support_like: bool,
    primary_texture_id: Option<u32>,
    primary_reason: String,
}

struct SurfaceRedirectionResolution {
    slot_id: Option<u32>,
    route_decision: &'static str,
    decision_reason: &'static str,
}

fn surface_main_ec_texture(
    tilemeta: &TileMetaPackage,
    item: &TileMetaItemTile,
) -> (Option<u32>, Option<String>, &'static str) {
    match tilemeta.main_ec_texture_id_with_reason(item.tile_id) {
        Some((texture_id, reason)) => {
            let source = if reason.as_str() == "legacy_ec_texture_id_fallback" {
                "tilemeta_item_legacy_ec_texture_id"
            } else {
                "tilemeta_texture_ref_chooser"
            };
            (Some(texture_id), Some(reason.as_str().to_string()), source)
        }
        None => (None, None, "missing"),
    }
}

fn terrain_matches_for_texture(
    sources: &TexArtEcLoadedSources,
    texture_id: u32,
) -> Vec<TerrainTextureMatch> {
    let mut matches = Vec::new();
    for entry in &sources.terrain_definition.entries {
        let Some(texture) = entry.texture.as_ref() else {
            continue;
        };
        let primary = entry.primary_texture_layer_with_reason();
        for (layer_index, layer) in texture.layers.iter().enumerate() {
            if layer.texture_id != Some(texture_id) {
                continue;
            }
            let primary_match = primary
                .map(|(primary_layer, _)| std::ptr::eq(primary_layer, layer))
                .unwrap_or(false);
            matches.push(TerrainTextureMatch {
                material_id: entry.id,
                material_name: entry.name.clone().unwrap_or_default(),
                layer_index,
                primary_match,
                aliases: entry.runtime_slot_ids(),
                layer_path: layer.path.clone().unwrap_or_default(),
                layer_current_support: layer.is_support_layer_by_current_name_heuristic(),
                layer_support_like: layer.has_support_like_name_clue(),
                primary_texture_id: primary.and_then(|(primary_layer, _)| primary_layer.texture_id),
                primary_reason: primary
                    .map(|(_, reason)| reason.as_str().to_string())
                    .unwrap_or_else(|| "missing".to_string()),
            });
        }
    }
    matches
}

fn surface_art_availability(
    tex_art_ec: Option<&TexArtEcPackage>,
    item: &TileMetaItemTile,
    main_ec_texture_id: Option<u32>,
) -> SurfaceArtAvailabilityReport {
    let Some(tex_art_ec) = tex_art_ec else {
        return SurfaceArtAvailabilityReport {
            ec_art_package_provided: false,
            art_id_tex_art_slot_present: None,
            art_id_tex_art_slot_kind: None,
            selected_texture_tex_art_slot_present: None,
            selected_texture_tex_art_slot_kind: None,
            legacy_ec_texture_tex_art_slot_present: None,
            legacy_ec_texture_tex_art_slot_kind: None,
        };
    };

    let art_id_slot = tex_art_ec.present_slot(item.tile_id);
    let selected_texture_slot =
        main_ec_texture_id.and_then(|texture_id| tex_art_ec.present_slot(texture_id));
    let legacy_texture_slot = optional_nonzero_u32(item.ec_texture_id)
        .and_then(|texture_id| tex_art_ec.present_slot(texture_id));

    SurfaceArtAvailabilityReport {
        ec_art_package_provided: true,
        art_id_tex_art_slot_present: Some(art_id_slot.is_some()),
        art_id_tex_art_slot_kind: art_id_slot.map(tex_art_ec_slot_kind).map(str::to_string),
        selected_texture_tex_art_slot_present: main_ec_texture_id
            .map(|_| selected_texture_slot.is_some()),
        selected_texture_tex_art_slot_kind: selected_texture_slot
            .map(tex_art_ec_slot_kind)
            .map(str::to_string),
        legacy_ec_texture_tex_art_slot_present: optional_nonzero_u32(item.ec_texture_id)
            .map(|_| legacy_texture_slot.is_some()),
        legacy_ec_texture_tex_art_slot_kind: legacy_texture_slot
            .map(tex_art_ec_slot_kind)
            .map(str::to_string),
    }
}

fn tex_art_ec_slot_kind(slot: &udd_assets::tex_art_ec::TexArtEcSlotRecord) -> &'static str {
    if slot.is_land() {
        "land"
    } else if slot.is_static() {
        "static"
    } else {
        "unknown"
    }
}

fn surface_override_report(
    entry: Option<&EcSurfaceOverrideEntry>,
    overrides: &HashMap<u32, EcSurfaceOverrideEntry>,
    item_by_id: &HashMap<u32, &TileMetaItemTile>,
    tex_art_ec: Option<&TexArtEcPackage>,
    tex_land_ec: &TexLandEcPackage,
) -> Option<SurfaceManualOverrideReport> {
    let entry = entry?;
    let mut validation_flags = Vec::new();

    if entry.active_action_count() != 1 {
        validation_flags.push("override_must_have_exactly_one_action".to_string());
    }

    let mut report = match entry.action() {
        EcSurfaceOverrideAction::CcArt(cc_art) => {
            let mode = cc_art.mode_kind();

            if mode == CcArtOverrideMode::Unknown {
                validation_flags.push("unknown_cc_art_override_mode".to_string());
            }

            if mode == CcArtOverrideMode::ResolveEc {
                if detects_resolve_ec_cycle(entry.cc_id, cc_art.target_id, overrides) {
                    validation_flags.push("resolve_ec_cycle".to_string());
                }
            }

            let preview = if let Some(target_item) = item_by_id.get(&cc_art.target_id) {
                let target_tex_art_slot_kind = tex_art_ec
                    .and_then(|package| package.present_slot(cc_art.target_id))
                    .map(tex_art_ec_slot_kind)
                    .map(str::to_string);
                Some(SurfaceManualOverridePreviewReport {
                    target_exists_in_tilemeta: true,
                    target_tex_art_slot_kind,
                    target_tex_land_runtime_slot: tex_land_ec
                        .resolve_runtime_slot_id(target_item.cc_texture_id),
                })
            } else {
                validation_flags.push("target_cc_id_missing_from_tilemeta".to_string());
                Some(SurfaceManualOverridePreviewReport {
                    target_exists_in_tilemeta: false,
                    target_tex_art_slot_kind: None,
                    target_tex_land_runtime_slot: None,
                })
            };

            SurfaceManualOverrideReport {
                action: "cc-art".to_string(),
                target_id: Some(cc_art.target_id),
                mode: Some(cc_art.mode.clone()),
                package: None,
                reason_code: cc_art.reason_code.clone(),
                valid: false,
                validation_flags,
                resolved_preview: preview,
            }
        }
        EcSurfaceOverrideAction::EcMaterial(material) => {
            let target_tex_art_slot_kind = tex_art_ec
                .and_then(|package| package.present_slot(material.material_id))
                .map(tex_art_ec_slot_kind)
                .map(str::to_string);
            let target_tex_land_runtime_slot =
                tex_land_ec.resolve_runtime_slot_id(material.material_id);
            if target_tex_art_slot_kind.is_none() && target_tex_land_runtime_slot.is_none() {
                validation_flags.push("target_ec_material_missing_from_known_packages".to_string());
            }

            SurfaceManualOverrideReport {
                action: "ec-material".to_string(),
                target_id: Some(material.material_id),
                mode: None,
                package: material.package.clone(),
                reason_code: material.reason_code.clone(),
                valid: false,
                validation_flags,
                resolved_preview: Some(SurfaceManualOverridePreviewReport {
                    target_exists_in_tilemeta: item_by_id.contains_key(&material.material_id),
                    target_tex_art_slot_kind,
                    target_tex_land_runtime_slot,
                }),
            }
        }
        EcSurfaceOverrideAction::Ignore(ignore) => SurfaceManualOverrideReport {
            action: "ignore".to_string(),
            target_id: None,
            mode: None,
            package: None,
            reason_code: ignore.reason_code.clone(),
            valid: false,
            validation_flags,
            resolved_preview: None,
        },
        EcSurfaceOverrideAction::Invalid => SurfaceManualOverrideReport {
            action: "invalid".to_string(),
            target_id: None,
            mode: None,
            package: None,
            reason_code: None,
            valid: false,
            validation_flags,
            resolved_preview: None,
        },
    };

    report.valid = report.validation_flags.is_empty();
    Some(report)
}

fn detects_resolve_ec_cycle(
    origin_id: u32,
    mut target_id: u32,
    overrides: &HashMap<u32, EcSurfaceOverrideEntry>,
) -> bool {
    let mut visited = BTreeSet::from([origin_id]);
    loop {
        if !visited.insert(target_id) {
            return true;
        }

        let Some(entry) = overrides.get(&target_id) else {
            return false;
        };
        let EcSurfaceOverrideAction::CcArt(cc_art) = entry.action() else {
            return false;
        };
        if cc_art.mode_kind() != CcArtOverrideMode::ResolveEc {
            return false;
        }
        target_id = cc_art.target_id;
    }
}

fn surface_review_class(
    item: &TileMetaItemTile,
    tile_type: Option<TileType>,
    art_availability: &SurfaceArtAvailabilityReport,
    resolved_slot: Option<u32>,
    terrain_matches: &[TerrainTextureMatch],
) -> &'static str {
    if resolved_slot.is_some() {
        return "resolved_tex_land_ec";
    }

    if art_availability.art_id_tex_art_slot_kind.as_deref() == Some("land") {
        return "tileart_surface_material_available";
    }

    if art_availability.art_id_tex_art_slot_present == Some(true) {
        return "regular_ec_art_available";
    }

    if !art_availability.ec_art_package_provided {
        return "manual_review_candidate_missing_ec_art_package";
    }

    if surface_is_liquid_like(item, tile_type) {
        return "liquid_like_without_provenance";
    }

    if surface_is_terrain_like(item) {
        return "terrain_like_without_provenance";
    }

    if !terrain_matches.is_empty() {
        return "terrain_layer_match_without_route";
    }

    "missing_regular_and_land_route"
}

fn surface_is_liquid_like(item: &TileMetaItemTile, tile_type: Option<TileType>) -> bool {
    if tile_type == Some(TileType::Liquid) {
        return true;
    }

    let name = item.name_ascii().to_ascii_lowercase();
    name.contains("water")
        || name.contains("lava")
        || name.contains("swamp")
        || name.contains("pond")
        || name.contains("whirlpool")
}

fn surface_is_terrain_like(item: &TileMetaItemTile) -> bool {
    let name = item.name_ascii().to_ascii_lowercase();
    name.contains("grass")
        || name.contains("dirt")
        || name.contains("sand")
        || name.contains("rock")
        || name.contains("stone")
        || name.contains("paver")
        || name.contains("flagstone")
}

fn surface_terrain_match_report(match_row: &TerrainTextureMatch) -> SurfaceTerrainMatchReport {
    SurfaceTerrainMatchReport {
        material_id: match_row.material_id,
        material_name: match_row.material_name.clone(),
        layer_index: match_row.layer_index,
        primary_match: match_row.primary_match,
        aliases: match_row.aliases.clone(),
        layer_path: match_row.layer_path.clone(),
        layer_current_support: match_row.layer_current_support,
        layer_support_like: match_row.layer_support_like,
        primary_texture_id: match_row.primary_texture_id,
        primary_reason: match_row.primary_reason.clone(),
    }
}

fn surface_provenance_record_report(
    record: &TexLandEcTerrainProvenanceRecord,
) -> SurfaceProvenanceRecordReport {
    SurfaceProvenanceRecordReport {
        material_id: record.material_id,
        material_name_id: record.material_name_id,
        alias_count_index: record.alias_count_index,
        alias_slot_id: optional_slot_id(record.alias_slot_id),
        alias_tile_flags: record.alias_tile_flags,
        selected_texture_id: optional_texture_id(record.selected_texture_id),
        canonical_slot_id: optional_slot_id(record.canonical_slot_id),
        primary_texture_id: optional_texture_id(record.primary_texture_id),
        primary_layer_index: optional_terrain_layer_index(record.primary_layer_index),
        primary_selection_reason: terrain_primary_reason_name(record.primary_selection_reason)
            .to_string(),
        primary_selection_flags: terrain_primary_flags_vec(record.primary_selection_flags)
            .into_iter()
            .map(str::to_string)
            .collect(),
    }
}

fn present_canonical_slots(
    package: &TexLandEcPackage,
    provenance_records: &[TexLandEcTerrainProvenanceRecord],
) -> Vec<u32> {
    let mut slots = BTreeSet::new();
    for record in provenance_records {
        if record.canonical_slot_id != 0
            && record.canonical_slot_id != MISSING_SLOT_ID
            && package.present_slot(record.canonical_slot_id).is_some()
        {
            slots.insert(record.canonical_slot_id);
        }
    }
    slots.into_iter().collect()
}

fn present_alias_slots(
    package: &TexLandEcPackage,
    provenance_records: &[TexLandEcTerrainProvenanceRecord],
) -> Vec<u32> {
    let mut slots = BTreeSet::new();
    for record in provenance_records {
        if record.alias_slot_id != 0
            && record.alias_slot_id != MISSING_SLOT_ID
            && package.present_slot(record.alias_slot_id).is_some()
        {
            slots.insert(record.alias_slot_id);
        }
    }
    slots.into_iter().collect()
}

fn resolve_surface_redirection_like_runtime(
    package: &TexLandEcPackage,
    item: &TileMetaItemTile,
    main_ec_texture_id: Option<u32>,
    direct_slot_present: bool,
    canonical_slots: &[u32],
    alias_slots: &[u32],
    fallback_slot: Option<u32>,
    tileart_surface_material_available: bool,
) -> SurfaceRedirectionResolution {
    let Some(main_ec_texture_id) = main_ec_texture_id else {
        return SurfaceRedirectionResolution {
            slot_id: fallback_slot,
            route_decision: if fallback_slot.is_some() {
                "TexLandEcArt"
            } else {
                "EcRegularArt"
            },
            decision_reason: "missing_main_ec_texture_fallback_to_cc_runtime_slot",
        };
    };

    if direct_slot_present {
        return SurfaceRedirectionResolution {
            slot_id: Some(main_ec_texture_id),
            route_decision: "TexLandEcArt",
            decision_reason: "main_ec_texture_id_present_as_tex_land_slot",
        };
    }

    if canonical_slots.len() == 1 {
        return SurfaceRedirectionResolution {
            slot_id: canonical_slots.first().copied(),
            route_decision: "TexLandEcArt",
            decision_reason: "unique_present_canonical_slot_from_provenance",
        };
    }

    if canonical_slots.is_empty() && alias_slots.len() == 1 {
        return SurfaceRedirectionResolution {
            slot_id: alias_slots.first().copied(),
            route_decision: "TexLandEcArt",
            decision_reason: "unique_present_alias_slot_from_provenance",
        };
    }

    let slot_id = package.resolve_runtime_slot_id(item.cc_texture_id);
    if slot_id.is_none() && tileart_surface_material_available {
        return SurfaceRedirectionResolution {
            slot_id: None,
            route_decision: "EcTileartSurfaceMaterial",
            decision_reason: "tileart_surface_material_slot_available",
        };
    }

    SurfaceRedirectionResolution {
        slot_id,
        route_decision: if slot_id.is_some() {
            "TexLandEcArt"
        } else {
            "EcRegularArt"
        },
        decision_reason: if canonical_slots.len() > 1 {
            "ambiguous_canonical_slots_fallback_to_cc_runtime_slot"
        } else if alias_slots.len() > 1 {
            "ambiguous_alias_slots_fallback_to_cc_runtime_slot"
        } else {
            "no_provenance_slot_fallback_to_cc_runtime_slot"
        },
    }
}

fn surface_redirection_flags(
    item: &TileMetaItemTile,
    main_ec_texture_id: Option<u32>,
    direct_slot_present: bool,
    terrain_matches: &[TerrainTextureMatch],
    provenance_records: &[TexLandEcTerrainProvenanceRecord],
    canonical_slots: &[u32],
    alias_slots: &[u32],
    resolved_slot: Option<u32>,
) -> String {
    let mut flags = Vec::new();
    if main_ec_texture_id.is_none() {
        flags.push("missing_main_ec_texture");
    }
    if !direct_slot_present && provenance_records.is_empty() {
        flags.push("no_provenance_records_for_main_texture");
    }
    if terrain_matches.is_empty() {
        flags.push("no_terrain_definition_layer_match");
    }
    if !terrain_matches.is_empty() && terrain_matches.iter().all(|match_row| !match_row.primary_match) {
        flags.push("layer_only_terrain_matches");
    }
    if canonical_slots.len() > 1 {
        flags.push("ambiguous_canonical_slots");
    }
    if alias_slots.len() > 1 {
        flags.push("ambiguous_alias_slots");
    }
    if resolved_slot.is_none() {
        flags.push("unresolved_runtime_slot");
    }
    if item.cc_texture_id == 0 {
        flags.push("missing_cc_texture_id_fallback");
    }
    flags.join(";")
}

fn terrain_primary_reason_name(reason: u8) -> &'static str {
    match reason {
        1 => "non_support_preferred_repetition",
        2 => "non_support_repetition_fallback",
        3 => "support_preferred_repetition_fallback",
        4 => "support_repetition_fallback",
        _ => "unknown",
    }
}

fn terrain_primary_flags_vec(flags: u16) -> Vec<&'static str> {
    let mut names = Vec::new();
    if flags & TERRAIN_PRIMARY_FLAG_SELECTED_CURRENT_SUPPORT != 0 {
        names.push("selected_current_support");
    }
    if flags & TERRAIN_PRIMARY_FLAG_SELECTED_SUPPORT_LIKE != 0 {
        names.push("selected_support_like");
    }
    if flags & TERRAIN_PRIMARY_FLAG_SELECTED_PREFERRED_REPETITION != 0 {
        names.push("selected_preferred_repetition");
    }
    if flags & TERRAIN_PRIMARY_FLAG_FALLBACK_REASON != 0 {
        names.push("fallback_reason");
    }
    if flags & TERRAIN_PRIMARY_FLAG_MULTIPLE_PREFERRED_NON_SUPPORT != 0 {
        names.push("multiple_preferred_non_support");
    }
    if flags & TERRAIN_PRIMARY_FLAG_SUPPORT_LIKE_OUTSIDE_CURRENT_HEURISTIC != 0 {
        names.push("support_like_outside_current_heuristic");
    }
    if flags & TERRAIN_PRIMARY_FLAG_OPAQUE_UNK6_TIEBREAKER != 0 {
        names.push("opaque_unk6_tiebreaker");
    }
    names
}

fn optional_nonzero_u32(value: u32) -> Option<u32> {
    (value != 0).then_some(value)
}

fn optional_slot_id(value: u32) -> Option<u32> {
    (value != 0 && value != MISSING_SLOT_ID).then_some(value)
}

fn optional_texture_id(value: u32) -> Option<u32> {
    (value != 0 && value != MISSING_TEXTURE_ID).then_some(value)
}

fn optional_terrain_layer_index(value: u32) -> Option<u32> {
    (value != MISSING_TERRAIN_LAYER_INDEX).then_some(value)
}

fn terrain_material_shape(
    alias_record_count: usize,
    concrete_alias_count: usize,
    texture_layer_count: usize,
    unique_texture_id_count: usize,
    non_support_texture_layer_count: usize,
    support_like_layer_count: usize,
) -> &'static str {
    let has_aliases = alias_record_count > 0;
    let has_concrete_aliases = concrete_alias_count > 0;
    let has_multiple_unique_textures = unique_texture_id_count > 1;
    let has_multiple_non_support = non_support_texture_layer_count > 1;
    let has_support = support_like_layer_count > 0;
    let is_complex = texture_layer_count > 1
        && (has_multiple_unique_textures || has_multiple_non_support || has_support);

    match (has_aliases, has_concrete_aliases, is_complex) {
        (true, true, true) => "concrete_aliases_complex_material",
        (true, true, false) => "concrete_aliases_simple_material",
        (true, false, true) => "placeholder_alias_complex_material",
        (true, false, false) => "placeholder_alias_simple_material",
        (false, _, true) => "no_alias_complex_material",
        (false, _, false) => "no_alias_simple_material",
    }
}

fn split_flags(flags: &str) -> Vec<String> {
    if flags.is_empty() {
        Vec::new()
    } else {
        flags.split(';').map(str::to_string).collect()
    }
}

fn terrain_layer_report(
    layer_index: usize,
    layer: &TerrainDefinitionTextureLayer,
    package_membership: &EcPackageMembership,
) -> TerrainLayerReport {
    let raw_path = layer.path.as_deref().unwrap_or("");
    let normalized_path = normalize_dictionary_path(raw_path);
    let physical_package =
        physical_package_guess(&normalized_path, layer.texture_id, package_membership);
    TerrainLayerReport {
        layer_index,
        texture_id: layer.texture_id,
        path: layer.path.clone(),
        texture_type: terrain_texture_type_name(layer.texture_type).to_string(),
        current_support_heuristic: layer.is_support_layer_by_current_name_heuristic(),
        support_like_clue: layer.has_support_like_name_clue(),
        preferred_repetition: layer.has_preferred_primary_repetition(),
        texture_repetition: layer.texture_repetition,
        unk4: layer.unk4,
        unk6_opaque_order: layer.unk6,
        unk7: layer.unk7,
        physical_package_guess: physical_package.to_string(),
        logical_family: terrain_texture_family_name(layer.texture_type, physical_package)
            .to_string(),
    }
}

fn terrain_definition_kdl_entry_report(
    entry: &TerrainDefEntry,
) -> TerrainDefinitionKdlEntryReport {
    TerrainDefinitionKdlEntryReport {
        terrain_type: entry.terrain_type.clone(),
        flags: terrain_type_flags(&entry.terrain_type),
        speed: entry.speed,
        waveheight: entry.waveheight,
        textureid: entry.textureid,
        layers: kdl_layers(entry)
            .into_iter()
            .map(|(role, layer)| TerrainDefinitionKdlLayerReport {
                role,
                texture_id: layer.id,
                stretch: layer.stretch,
            })
            .collect(),
    }
}

fn terrain_definition_uop_entry_report(
    entry: &TerrainDefinitionEntry,
) -> TerrainDefinitionUopEntryReport {
    TerrainDefinitionUopEntryReport {
        name: entry.name.clone(),
        unknown_floats: [entry.unk, entry.unk2, entry.unk3],
        alias_record_count: entry.aliases.len(),
        concrete_alias_count: entry.aliases.iter().filter(|alias| alias.alias != 0).count(),
        runtime_slot_ids: entry.runtime_slot_ids(),
        shader_name: entry
            .texture
            .as_ref()
            .and_then(|texture| texture.shader_name.clone()),
        layers: entry
            .texture
            .as_ref()
            .map(|texture| {
                texture
                    .layers
                    .iter()
                    .enumerate()
                    .map(|(layer_index, layer)| TerrainDefinitionUopLayerReport {
                        layer_index,
                        texture_id: layer.texture_id,
                        repetition: layer.texture_repetition,
                        path: layer.path.clone(),
                        support_like_name: layer.has_support_like_name_clue(),
                        current_support_heuristic: layer
                            .is_support_layer_by_current_name_heuristic(),
                        unk4: layer.unk4,
                        unk6: layer.unk6,
                        unk7: layer.unk7,
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn terrain_definition_kdl_findings(
    kdl_entry: Option<&TerrainDefEntry>,
    uop_entry: Option<&TerrainDefinitionEntry>,
) -> Vec<TerrainDefinitionKdlFindingReport> {
    let mut findings = Vec::new();
    match (kdl_entry, uop_entry) {
        (Some(_), None) => {
            findings.push(terrain_kdl_finding(
                "entry",
                "kdl_only_entry",
                "KDL entry id has no TerrainDefinition.uop entry with the same id",
            ));
            return findings;
        }
        (None, Some(_)) => {
            findings.push(terrain_kdl_finding(
                "entry",
                "uop_only_entry",
                "TerrainDefinition.uop entry id has no KDL entry with the same id",
            ));
            return findings;
        }
        (None, None) => return findings,
        (Some(_), Some(_)) => {}
    }

    let kdl_entry = kdl_entry.expect("checked");
    let uop_entry = uop_entry.expect("checked");
    let uop_layers = uop_entry
        .texture
        .as_ref()
        .map(|texture| texture.layers.as_slice())
        .unwrap_or(&[]);
    let uop_texture_ids = uop_layers
        .iter()
        .filter_map(|layer| layer.texture_id)
        .collect::<BTreeSet<_>>();

    for flag in terrain_type_flags(&kdl_entry.terrain_type) {
        let status = terrain_type_flag_status(&flag, uop_entry);
        findings.push(terrain_kdl_finding(
            &format!("terrain_type.{flag}"),
            status,
            terrain_type_flag_detail(&flag, uop_entry),
        ));
    }

    if let Some(speed) = kdl_entry.speed {
        findings.push(terrain_kdl_float_finding("speed", speed, uop_entry));
    }
    if let Some(waveheight) = kdl_entry.waveheight {
        findings.push(terrain_kdl_float_finding("waveheight", waveheight, uop_entry));
    }
    if let Some(textureid) = kdl_entry.textureid {
        let status = if uop_texture_ids.contains(&textureid) {
            "matches_uop_layer_texture"
        } else {
            "kdl_only_texture_id"
        };
        findings.push(terrain_kdl_finding(
            "textureid",
            status,
            format!("KDL textureid={textureid}"),
        ));
    }

    for (role, layer) in kdl_layers(kdl_entry) {
        findings.push(terrain_kdl_layer_finding(role, layer, uop_layers));
    }

    for uop_layer in uop_layers {
        if let Some(texture_id) = uop_layer.texture_id {
            let in_kdl = kdl_layers(kdl_entry)
                .iter()
                .any(|(_, kdl_layer)| kdl_layer.id == texture_id);
            if !in_kdl {
                findings.push(terrain_kdl_finding(
                    &format!("uop_layer.{texture_id}"),
                    "uop_only_layer_texture",
                    uop_layer
                        .path
                        .clone()
                        .unwrap_or_else(|| "missing path".to_string()),
                ));
            }
        }
    }

    findings
}

fn terrain_kdl_finding_needs_manual_review(status: &str) -> bool {
    matches!(
        status,
        "kdl_only_entry"
            | "kdl_only_layer_texture"
            | "kdl_only_no_known_uop_field"
            | "kdl_only_or_unproven"
            | "kdl_only_runtime_policy"
            | "kdl_only_texture_id"
    )
}

fn terrain_definition_manual_candidate_report(
    id: u32,
    kdl_entry: Option<&TerrainDefEntry>,
    uop_entry: Option<&TerrainDefinitionEntry>,
    finding: &TerrainDefinitionKdlFindingReport,
) -> TerrainDefinitionManualCandidateReport {
    TerrainDefinitionManualCandidateReport {
        id,
        field: finding.field.clone(),
        code: finding.status.clone(),
        detail: finding.detail.clone(),
        kdl_terrain_type: kdl_entry.map(|entry| entry.terrain_type.clone()),
        uop_shader_name: uop_entry.and_then(|entry| {
            entry
                .texture
                .as_ref()
                .and_then(|texture| texture.shader_name.clone())
        }),
        runtime_slot_ids: uop_entry
            .map(TerrainDefinitionEntry::runtime_slot_ids)
            .unwrap_or_default(),
    }
}

fn terrain_type_flags(terrain_type: &str) -> Vec<String> {
    terrain_type
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn terrain_type_flag_status(flag: &str, entry: &TerrainDefinitionEntry) -> &'static str {
    let shader = entry
        .texture
        .as_ref()
        .and_then(|texture| texture.shader_name.as_deref())
        .unwrap_or("");
    match flag {
        "Liquid" if shader.to_ascii_lowercase().contains("water") => {
            "inferable_from_uop_shader"
        }
        "Single" if entry.texture.as_ref().is_some_and(|texture| texture.layers.len() == 1) => {
            "inferable_from_uop_layer_count"
        }
        "Solid" => "default_policy_not_direct_uop",
        "Smooth" | "FollowCenter" => "kdl_only_runtime_policy",
        _ => "kdl_only_or_unproven",
    }
}

fn terrain_type_flag_detail(flag: &str, entry: &TerrainDefinitionEntry) -> String {
    let shader = entry
        .texture
        .as_ref()
        .and_then(|texture| texture.shader_name.clone())
        .unwrap_or_else(|| "missing shader".to_string());
    format!("flag={flag}; uop_shader={shader}")
}

fn terrain_kdl_float_finding(
    field: &'static str,
    value: f32,
    entry: &TerrainDefinitionEntry,
) -> TerrainDefinitionKdlFindingReport {
    let unknowns = [entry.unk, entry.unk2, entry.unk3];
    let status = if unknowns
        .iter()
        .any(|unknown| floats_match(*unknown, value))
    {
        "matches_uop_unknown_float"
    } else {
        "kdl_only_no_known_uop_field"
    };
    terrain_kdl_finding(
        field,
        status,
        format!(
            "kdl={value}; uop_unknowns={},{},{}",
            entry.unk, entry.unk2, entry.unk3
        ),
    )
}

fn terrain_kdl_layer_finding(
    role: &'static str,
    kdl_layer: &LayerDef,
    uop_layers: &[TerrainDefinitionTextureLayer],
) -> TerrainDefinitionKdlFindingReport {
    let matching_layers = uop_layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| layer.texture_id == Some(kdl_layer.id))
        .collect::<Vec<_>>();

    if matching_layers.is_empty() {
        return terrain_kdl_finding(
            &format!("layer.{role}"),
            "kdl_only_layer_texture",
            format!("texture_id={}; stretch={}", kdl_layer.id, kdl_layer.stretch),
        );
    }

    let stretch_match = matching_layers.iter().any(|(_, layer)| {
        floats_match(layer.texture_repetition, kdl_layer.stretch)
    });
    let support_name_match = matching_layers
        .iter()
        .any(|(_, layer)| terrain_role_matches_layer_name(role, layer));
    let status = match (stretch_match, support_name_match) {
        (true, true) => "matches_uop_layer_and_role_name",
        (true, false) => "matches_uop_layer_id_and_stretch",
        (false, true) => "matches_uop_layer_id_and_role_name",
        (false, false) => "matches_uop_layer_id_only",
    };
    let layer_descriptions = matching_layers
        .iter()
        .map(|(index, layer)| {
            format!(
                "uop_index={index}; repetition={}; path={}",
                layer.texture_repetition,
                layer.path.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join(" | ");

    terrain_kdl_finding(
        &format!("layer.{role}"),
        status,
        format!(
            "kdl_texture_id={}; kdl_stretch={}; {layer_descriptions}",
            kdl_layer.id, kdl_layer.stretch
        ),
    )
}

fn terrain_role_matches_layer_name(role: &str, layer: &TerrainDefinitionTextureLayer) -> bool {
    let path = layer
        .path
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    match role {
        "m" => path.contains("mask") || path.contains("alpha"),
        "n" => path.contains("normal") || path.contains("bump") || path.contains("ripple"),
        "t0" | "t1" => !layer.has_support_like_name_clue(),
        "s" => true,
        _ => false,
    }
}

fn kdl_layers(entry: &TerrainDefEntry) -> Vec<(&'static str, &LayerDef)> {
    [
        ("t0", entry.t0.as_ref()),
        ("t1", entry.t1.as_ref()),
        ("m", entry.m.as_ref()),
        ("s", entry.s.as_ref()),
        ("n", entry.n.as_ref()),
    ]
    .into_iter()
    .filter_map(|(role, layer)| layer.map(|layer| (role, layer)))
    .collect()
}

fn terrain_kdl_finding(
    field: impl Into<String>,
    status: impl Into<String>,
    detail: impl Into<String>,
) -> TerrainDefinitionKdlFindingReport {
    TerrainDefinitionKdlFindingReport {
        field: field.into(),
        status: status.into(),
        detail: detail.into(),
    }
}

fn floats_match(left: f32, right: f32) -> bool {
    (left - right).abs() <= 0.001
}

fn terrain_primary_audit_flags(
    entry: &TerrainDefinitionEntry,
    selected_layer: Option<&TerrainDefinitionTextureLayer>,
    selection_reason: Option<TerrainDefinitionPrimaryLayerReason>,
) -> String {
    let mut flags = Vec::new();

    let Some(texture) = entry.texture.as_ref() else {
        flags.push("missing_texture_block");
        return flags.join(";");
    };
    let Some(selected_layer) = selected_layer else {
        flags.push("missing_selected_layer");
        return flags.join(";");
    };

    if selected_layer.has_support_like_name_clue() {
        flags.push("selected_support_like_name");
    }
    if selected_layer.is_support_layer_by_current_name_heuristic() {
        flags.push("selected_current_support_heuristic");
    }
    if matches!(
        selection_reason,
        Some(TerrainDefinitionPrimaryLayerReason::NonSupportRepetitionFallback)
            | Some(TerrainDefinitionPrimaryLayerReason::SupportPreferredRepetitionFallback)
            | Some(TerrainDefinitionPrimaryLayerReason::SupportRepetitionFallback)
    ) {
        flags.push("fallback_selection_reason");
    }

    let preferred_non_support_count = texture
        .layers
        .iter()
        .filter(|layer| {
            layer.texture_id.is_some()
                && !layer.is_support_layer_by_current_name_heuristic()
                && layer.has_preferred_primary_repetition()
        })
        .count();
    if preferred_non_support_count > 1 {
        flags.push("multiple_preferred_non_support_layers");
    }

    let selected_rank_peer_count = texture
        .layers
        .iter()
        .filter(|layer| {
            layer.texture_id.is_some()
                && layer.is_support_layer_by_current_name_heuristic()
                    == selected_layer.is_support_layer_by_current_name_heuristic()
                && layer.has_preferred_primary_repetition()
                    == selected_layer.has_preferred_primary_repetition()
        })
        .count();
    if selected_rank_peer_count > 1 {
        flags.push("opaque_unk6_order_tiebreaker_involved");
    }

    let support_like_not_current_count = texture
        .layers
        .iter()
        .filter(|layer| {
            layer.texture_id.is_some()
                && layer.has_support_like_name_clue()
                && !layer.is_support_layer_by_current_name_heuristic()
        })
        .count();
    if support_like_not_current_count > 0 {
        flags.push("support_like_outside_current_heuristic");
    }

    let selected_texture_id = selected_layer.texture_id;
    if selected_texture_id.is_none() {
        flags.push("selected_layer_missing_texture_id");
    }

    flags.join(";")
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

fn terrain_texture_type_name(texture_type: TerrainTextureType) -> &'static str {
    match texture_type {
        TerrainTextureType::Undefined => "Undefined",
        TerrainTextureType::WorldArt => "WorldArt",
        TerrainTextureType::TileArtLegacy => "TileArtLegacy",
        TerrainTextureType::TileArtEnhanced => "TileArtEnhanced",
        TerrainTextureType::Textures => "Textures",
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
