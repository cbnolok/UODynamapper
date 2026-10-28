use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use clap::Parser;
use color_eyre::eyre::{self, WrapErr};
use udd_assets::{
    tex_art_ec::TexArtEcPackage,
    tex_land_ec::{TexLandEcPackage, MISSING_SLOT_ID},
    tilemeta::{TileMetaItemTextureRef, TileMetaItemTile, TileMetaPackage},
};
use udd_conv::tex_art_ec::{is_base_eligible, LiquidExclusionReason};
use udd_conv::tilemeta::stable_role_for_tileart_texture_ref;
use udd_assets::tilemeta::TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED;
use udd_conv::terrain_meta::TerrainMetaPackage;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tile_database::ArtDefinition;
use uocf::enhanced::tileart::TileArtEntry;
use uocf::uop_container::package::UopPackage;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long)]
    ecdir: PathBuf,
    #[arg(long)]
    tilemeta: Option<PathBuf>,
    #[arg(long = "ec-art")]
    tex_art_ec: Option<PathBuf>,
    #[arg(long = "ec-land")]
    tex_land_ec: Option<PathBuf>,
    #[arg(long = "terrain-meta", help = "Optional terrain-layer sidecar binary (terrain_meta.bin) for layer graph display.")]
    terrain_meta: Option<PathBuf>,
    #[arg(required = true)]
    art_ids: Vec<u16>,
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let args = Args::parse();
    let tileart_path = args.ecdir.join("tileart.uop");
    let stringdict_path = find_string_dictionary_path(&args.ecdir)?;

    let string_dictionary = UoStringDictionary::load(&stringdict_path)
        .wrap_err_with(|| format!("load {}", stringdict_path.display()))?;
    let art_definition = ArtDefinition::load(&tileart_path, &stringdict_path)
        .wrap_err("load tileart-driven art definition")?;
    let raw_entries = load_raw_tileart_entries(&tileart_path)?;

    let tilemeta = args
        .tilemeta
        .as_ref()
        .map(TileMetaPackage::load)
        .transpose()
        .wrap_err("load tilemeta package")?;
    let tex_art_ec = args
        .tex_art_ec
        .as_ref()
        .map(TexArtEcPackage::load)
        .transpose()
        .wrap_err("load tex_art_ec package")?;
    let tex_land_ec = args
        .tex_land_ec
        .as_ref()
        .map(TexLandEcPackage::load)
        .transpose()
        .wrap_err("load tex_land_ec package")?;
    let terrain_meta = args
        .terrain_meta
        .as_ref()
        .map(TerrainMetaPackage::load)
        .transpose()
        .wrap_err("load terrain_meta sidecar")?;

    for art_id in args.art_ids {
        println!("art_id={art_id}");

        match raw_entries.get(&art_id) {
            Some(raw_entry) => print_raw_entry(raw_entry, &string_dictionary),
            None => println!("  raw_entry=missing"),
        }

        match art_definition.definitions.get(&art_id) {
            Some(art_data) => {
                for (block_i, block) in art_data.texture_items.iter().enumerate() {
                    for (item_i, item) in block.iter().enumerate() {
                        println!(
                            "  texture_items[{block_i}][{item_i}].ec_logical_family={:?}",
                            item.ec_logical_family
                        );
                        println!(
                            "  texture_items[{block_i}][{item_i}].ec_physical_package={:?}",
                            item.ec_physical_package
                        );
                        println!(
                            "  texture_items[{block_i}][{item_i}].texture_stretch={}",
                            item.texture_stretch
                        );
                        println!(
                            "  texture_items[{block_i}][{item_i}].raw_path={}",
                            item.raw_path.as_deref().unwrap_or("<missing>")
                        );
                        println!(
                            "  texture_items[{block_i}][{item_i}].normalized_path={}",
                            item.normalized_path.as_deref().unwrap_or("<missing>")
                        );
                    }
                }
                println!("  processed.tile_type={:?}", art_data.tile_type);
                println!("  processed.flags_bits=0x{:x}", art_data.flags.bits());
                println!("  processed.height={}", art_data.height);
                match art_data.ec_texture.as_ref() {
                    Some(texture) => println!(
                        "  processed.ec_texture=id={} window=({}, {})..({}, {}) offset=({}, {})",
                        texture.texture_id,
                        texture.start_x,
                        texture.start_y,
                        texture.end_x,
                        texture.end_y,
                        texture.offset_x,
                        texture.offset_y
                    ),
                    None => println!("  processed.ec_texture=missing"),
                }
                match art_data.cc_texture.as_ref() {
                    Some(texture) => println!(
                        "  processed.cc_texture=id={} lt_0x4000={} window=({}, {})..({}, {}) offset=({}, {})",
                        texture.texture_id,
                        texture.texture_id < 0x4000,
                        texture.start_x,
                        texture.start_y,
                        texture.end_x,
                        texture.end_y,
                        texture.offset_x,
                        texture.offset_y
                    ),
                    None => println!("  processed.cc_texture=missing"),
                }
                let chosen_texture_id = art_data
                    .ec_texture
                    .as_ref()
                    .map(|t| t.texture_id)
                    .or_else(|| art_data.cc_texture.as_ref().map(|t| t.texture_id));
                let stable_role = {
                    let chosen_id = chosen_texture_id;
                    let mut role = udd_assets::tilemeta::EcMaterialStableRole::UnknownSupport;
                    if let Some(id) = chosen_id {
                        'outer: for block in &art_data.texture_items {
                            for item in block {
                                if item.id == id {
                                    role = stable_role_for_tileart_texture_ref(
                                        item,
                                        TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
                                    );
                                    break 'outer;
                                }
                            }
                        }
                    }
                    role
                };
                let eligible = is_base_eligible(art_data.tile_type, chosen_texture_id, stable_role);
                println!("  liquid_base_eligible={eligible}");
                let exclusion_reason = if !eligible {
                    if chosen_texture_id.is_none() {
                        "LiquidNoBase"
                    } else if stable_role == udd_assets::tilemeta::EcMaterialStableRole::EffectOnlyMetadata {
                        "LiquidEffectOnly"
                    } else {
                        "LiquidUnknownSupportOnly"
                    }
                } else {
                    "NotExcluded"
                };
                println!("  liquid_exclusion_reason={exclusion_reason}");
            }
            None => println!("  processed=missing"),
        }

        if let Some(tilemeta) = tilemeta.as_ref() {
            print_tilemeta(tilemeta, art_id as u32);
            print_tilemeta_texture_refs(tilemeta.item_texture_refs(art_id as u32));
        }

        if tex_art_ec.is_some() || tex_land_ec.is_some() {
            print_package_routing(
                art_id as u32,
                tilemeta.as_ref().and_then(|package| package.item_tile(art_id as u32)),
                tex_art_ec.as_ref(),
                tex_land_ec.as_ref(),
            );
        }

        if let Some(terrain_meta) = terrain_meta.as_ref() {
            print_terrain_meta_layers(terrain_meta, art_id as u32);
        }

        println!();
    }

    Ok(())
}

fn find_string_dictionary_path(ecdir: &Path) -> eyre::Result<PathBuf> {
    let path = ecdir.join("string_dictionary.uop");
    if path.exists() {
        Ok(path)
    } else {
        eyre::bail!("missing string_dictionary.uop in {}", ecdir.display())
    }
}

fn load_raw_tileart_entries(tileart_path: &Path) -> eyre::Result<HashMap<u16, TileArtEntry>> {
    let package = UopPackage::load(tileart_path)
        .wrap_err_with(|| format!("load {}", tileart_path.display()))?;
    let mut entries = HashMap::new();

    for file in package.iter_files() {
        let Ok(entry) = TileArtEntry::parse_raw(file) else {
            continue;
        };
        entries.insert(entry.tile_id as u16, entry);
    }

    Ok(entries)
}

fn print_raw_entry(raw_entry: &TileArtEntry, string_dictionary: &UoStringDictionary) {
    println!("  raw.old_id={}", raw_entry.old_id);
    println!("  raw.body_type={}", raw_entry.body_type);
    println!("  raw.flags1_bits=0x{:x}", raw_entry.flags1.bits());
    println!("  raw.flags2_bits=0x{:x}", raw_entry.flags2.bits());
    println!(
        "  raw.ec_img_offset=({}, {})..({}, {}) off=({}, {})",
        raw_entry.ec_img_offset.x_start,
        raw_entry.ec_img_offset.y_start,
        raw_entry.ec_img_offset.x_end,
        raw_entry.ec_img_offset.y_end,
        raw_entry.ec_img_offset.x_off,
        raw_entry.ec_img_offset.y_off
    );
    println!(
        "  raw.cc_img_offset=({}, {})..({}, {}) off=({}, {})",
        raw_entry.cc_img_offset.x_start,
        raw_entry.cc_img_offset.y_start,
        raw_entry.cc_img_offset.x_end,
        raw_entry.cc_img_offset.y_end,
        raw_entry.cc_img_offset.x_off,
        raw_entry.cc_img_offset.y_off
    );

    for (block_index, texture_block) in raw_entry.texture_vector.iter().enumerate() {
        if texture_block.has_texture != 1 {
            continue;
        }

        let shader_name = dictionary_string(string_dictionary, texture_block.type_string_off);
        println!(
            "  raw.texture_block[{block_index}].shader={} item_count={}",
            shader_name.as_deref().unwrap_or("<missing>"),
            texture_block.texture_items.len()
        );

        for (item_index, item) in texture_block.texture_items.iter().enumerate() {
            let path = dictionary_string(string_dictionary, item.name_string_off);
            println!(
                "    item[{item_index}] stretch={} source_kind={} sampler_index={} uv_set={} path={}",
                item.texture_stretch,
                item.source_kind,
                item.sampler_index,
                item.uv_set,
                path.as_deref().unwrap_or("<missing>")
            );
        }
    }
}

fn dictionary_string(
    string_dictionary: &UoStringDictionary,
    offset: u32,
) -> Option<String> {
    offset
        .checked_sub(1)
        .and_then(|index| string_dictionary.get_string(index as usize))
        .map(str::to_string)
}

fn print_tilemeta(tilemeta: &TileMetaPackage, art_id: u32) {
    let item = tilemeta.item_tile(art_id);
    match item {
        Some(item) => {
            println!("  tilemeta.visual_kind={:?}", item.visual_kind());
            println!("  tilemeta.is_surface_like={}", item.is_surface_like());
            println!(
                "  tilemeta.cc_texture_id={} lt_0x4000={}",
                item.cc_texture_id,
                item.cc_texture_id < 0x4000
            );
            println!("  tilemeta.ec_texture_id={}", item.ec_texture_id);
            println!(
                "  tilemeta.ec_start=({}, {}) cc_start=({}, {})",
                item.ec_start_x,
                item.ec_start_y,
                item.cc_start_x,
                item.cc_start_y
            );
            match tilemeta.main_ec_texture_id_with_reason(art_id) {
                Some((texture_id, reason)) => println!(
                    "  tilemeta.main_ec_texture_id={} reason={}",
                    texture_id,
                    reason.as_str()
                ),
                None => println!("  tilemeta.main_ec_texture_id=missing"),
            }
            match tilemeta.main_ec_texture_id_role_aware(art_id) {
                Some((texture_id, _, reason)) => {
                    println!("  tilemeta.role_aware_base_id={texture_id}");
                    println!("  tilemeta.role_aware_reason={}", reason.as_str());
                }
                None => {
                    println!("  tilemeta.role_aware_base_id=none");
                    println!("  tilemeta.role_aware_reason=not_found");
                }
            }
            println!(
                "  tilemeta.low_confidence={}",
                tilemeta.main_ec_texture_selection_is_low_confidence(art_id)
            );
        }
        None => println!("  tilemeta=missing"),
    }
}

fn print_tilemeta_texture_refs(texture_refs: &[TileMetaItemTextureRef]) {
    if texture_refs.is_empty() {
        println!("  tilemeta.texture_refs=none");
        return;
    }

    println!("  tilemeta.texture_refs.count={}", texture_refs.len());
    for (index, texture_ref) in texture_refs.iter().enumerate() {
        println!(
            "    ref[{index}] id={} family={:?} package={:?} stable_role={:?} speculative_role={:?} readiness={} block={} item={} aux={} primary={} stretch={} source_kind={} sampler_index={} uv_set={}",
            texture_ref.texture_id,
            texture_ref.logical_family(),
            texture_ref.physical_package(),
            texture_ref.stable_role(),
            texture_ref.speculative_role(),
            texture_ref.implementation_readiness().as_str(),
            texture_ref.block_index,
            texture_ref.item_index,
            texture_ref.is_auxiliary(),
            texture_ref.is_primary_selected(),
            texture_ref.texture_stretch,
            texture_ref.source_kind,
            texture_ref.sampler_index,
            texture_ref.uv_set,
        );
        println!(
            "      ref[{index}].physical_package={:?}",
            texture_ref.physical_package()
        );
        println!(
            "      ref[{index}].logical_family={:?}",
            texture_ref.logical_family()
        );
        println!(
            "      ref[{index}].stable_role={:?}",
            texture_ref.stable_role()
        );
        println!(
            "      ref[{index}].speculative_role={:?}",
            texture_ref.speculative_role()
        );
        println!(
            "      ref[{index}].implementation_readiness={}",
            texture_ref.implementation_readiness().as_str()
        );
        // TODO Phase 6: raw_path and normalized_path are not yet stored in TileMetaItemTextureRef
        // (deferred from Phase 5 — POD format extension requires a parallel string sidecar table).
        println!("      ref[{index}].raw_path=TODO(Phase6)");
        println!("      ref[{index}].normalized_path=TODO(Phase6)");
    }
}

fn print_package_routing(
    art_id: u32,
    tilemeta: Option<&TileMetaItemTile>,
    tex_art_ec: Option<&TexArtEcPackage>,
    tex_land_ec: Option<&TexLandEcPackage>,
) {
    let tex_art_ec_slot = tex_art_ec.and_then(|package| package.present_slot(art_id));
    println!(
        "  tex_art_ec.present_slot(art_id)={}",
        slot_summary(tex_art_ec_slot.map(|slot| (slot.page_index, slot.x, slot.y, slot.width, slot.height)))
    );

    let tex_land_ec_direct = tex_land_ec.and_then(|package| package.present_slot(art_id));
    println!(
        "  tex_land_ec.present_slot(art_id)={}",
        slot_summary(tex_land_ec_direct.map(|slot| (slot.page_index, slot.x, slot.y, slot.width, slot.height)))
    );

    let tex_land_ec_runtime_slot = resolve_surface_like_tex_land_ec_slot_id(tilemeta, tex_land_ec);
    println!("  tex_land_ec.runtime_slot={:?}", tex_land_ec_runtime_slot);

    let tex_land_ec_runtime_record = tex_land_ec_runtime_slot.and_then(|slot_id| {
        tex_land_ec.and_then(|package| package.present_slot(slot_id))
    });
    println!(
        "  tex_land_ec.present_slot(runtime_slot)={}",
        slot_summary(tex_land_ec_runtime_record.map(|slot| (slot.page_index, slot.x, slot.y, slot.width, slot.height)))
    );

    let renderer_decision = if tilemeta.is_some_and(|item| item.is_surface_like()) {
        match tex_land_ec_runtime_slot {
            Some(runtime_slot_id) => format!("TexLandEcArt(runtime_slot_id={runtime_slot_id})"),
            None => "TexLandEcArt(unresolved)".to_string(),
        }
    } else if tex_art_ec_slot.is_some() {
        format!("EcRegularArt(art_id={art_id})")
    } else {
        format!("EcRegularArt(missing art_id={art_id})")
    };
    println!("  renderer_decision={renderer_decision}");
}

fn slot_summary(slot: Option<(u32, u16, u16, u16, u16)>) -> String {
    match slot {
        Some((page_index, x, y, width, height)) => {
            format!("present page={} pos=({}, {}) size={}x{}", page_index, x, y, width, height)
        }
        None => "missing".to_string(),
    }
}

fn print_terrain_meta_layers(terrain_meta: &TerrainMetaPackage, terrain_tile_id: u32) {
    let layers = terrain_meta.layers_for_tile(terrain_tile_id);
    if layers.is_empty() {
        println!("  terrain_meta.layers=none");
        return;
    }
    println!("  terrain_meta.layers.count={}", layers.len());
    for (index, layer) in layers.iter().enumerate() {
        println!(
            "    layer[{index}] texture_id={} repetition={} is_support={} has_name_clue={} has_pref_rep={} unk4={} unk6={} unk7={}",
            layer.texture_id,
            layer.texture_repetition,
            layer.is_support,
            layer.has_name_clue,
            layer.has_pref_rep,
            layer.unk4,
            layer.unk6,
            layer.unk7,
        );
    }
}

fn resolve_surface_like_tex_land_ec_slot_id(
    tilemeta: Option<&TileMetaItemTile>,
    tex_land_ec: Option<&TexLandEcPackage>,
) -> Option<u32> {
    let meta = tilemeta?;
    if !meta.is_surface_like() {
        return None;
    }

    let package = tex_land_ec?;

    if let Some(slot_id) = package.resolve_runtime_slot_id(meta.cc_texture_id) {
        return Some(slot_id);
    }

    let mut unique_slots = BTreeSet::new();
    for record in package
        .terrain_provenance()
        .iter()
        .filter(|record| record.selected_texture_id == meta.ec_texture_id)
    {
        if record.canonical_slot_id != 0
            && record.canonical_slot_id != MISSING_SLOT_ID
            && package.present_slot(record.canonical_slot_id).is_some()
        {
            unique_slots.insert(record.canonical_slot_id);
        }

        if record.alias_slot_id != 0
            && record.alias_slot_id != MISSING_SLOT_ID
            && package.present_slot(record.alias_slot_id).is_some()
        {
            unique_slots.insert(record.alias_slot_id);
        }

        if unique_slots.len() > 1 {
            return None;
        }
    }

    unique_slots.into_iter().next()
}
