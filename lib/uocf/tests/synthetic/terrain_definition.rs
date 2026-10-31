use uocf::enhanced::terrain_definition::*;
use uocf::uop_container::file::CompressionFlag;
use uocf::uop_container::package::UopPackage;
use byteorder::{LittleEndian, WriteBytesExt};
use std::time::{SystemTime, UNIX_EPOCH};
use std::path::PathBuf;

fn serialize_dictionary_payload(strings: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x0123_4567_89AB_CDEFu64.to_le_bytes());
    bytes.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x48u32.to_le_bytes());
    for string in strings {
        let string_bytes = string.as_bytes();
        bytes.extend_from_slice(&(string_bytes.len() as u16).to_le_bytes());
        bytes.extend_from_slice(string_bytes);
    }
    bytes
}

fn serialize_entry_payload(
    name_id: i32,
    id: u32,
    aliases: &[(u32, u32, u64)],
    shader_name_string_off: i32,
    texture_images: &[(i32, u8, f32, i32, i32)],
    techniques: &[i32],
    tweaks: &[f32],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.write_i32::<LittleEndian>(name_id).unwrap();
    bytes.write_u32::<LittleEndian>(id).unwrap();
    bytes.write_f32::<LittleEndian>(1.25).unwrap();
    bytes.write_f32::<LittleEndian>(2.5).unwrap();
    bytes.write_f32::<LittleEndian>(3.75).unwrap();
    bytes.write_u32::<LittleEndian>(aliases.len() as u32).unwrap();
    for (count_index, alias, tile_flags) in aliases {
        bytes.write_u32::<LittleEndian>(*count_index).unwrap();
        bytes.write_u32::<LittleEndian>(*alias).unwrap();
        bytes.write_u64::<LittleEndian>(*tile_flags).unwrap();
    }
    bytes.write_u8(1).unwrap();
    bytes.write_u8(0).unwrap();
    bytes.write_i32::<LittleEndian>(shader_name_string_off).unwrap();
    bytes.write_u8(texture_images.len() as u8).unwrap();
    for (name_string_off, source_kind, texture_repetition, sampler_index, uv_set) in texture_images {
        bytes.write_i32::<LittleEndian>(*name_string_off).unwrap();
        bytes.write_u8(*source_kind).unwrap();
        bytes.write_f32::<LittleEndian>(*texture_repetition).unwrap();
        bytes.write_i32::<LittleEndian>(*sampler_index).unwrap();
        bytes.write_i32::<LittleEndian>(*uv_set).unwrap();
    }
    bytes.write_u32::<LittleEndian>(techniques.len() as u32).unwrap();
    for value in techniques {
        bytes.write_i32::<LittleEndian>(*value).unwrap();
    }
    bytes.write_u32::<LittleEndian>(tweaks.len() as u32).unwrap();
    for value in tweaks {
        bytes.write_f32::<LittleEndian>(*value).unwrap();
    }
    bytes
}

fn temp_uop_path(test_name: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("uocf_terrain_{test_name}_{timestamp}.uop"))
}

fn write_uop_file(test_name: &str, payload: &[u8], virtual_path: &str) -> PathBuf {
    let path = temp_uop_path(test_name);
    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(payload, virtual_path, CompressionFlag::Zlib)
        .expect("add file payload");
    package.finalize_and_save(&path).expect("save package");
    path
}

#[test]
fn terrain_definition_decoder_reads_ids_aliases_and_texture_path() {
    let dict_path = write_uop_file(
        "dictionary",
        &serialize_dictionary_payload(&[
            "Grass",
            "UOWaterTerrainLayer",
            "01000013_water_alpha.tga",
            "02000051_water.tga",
            "01000017_cube3.tga",
        ]),
        "build/string_dictionary.bin",
    );
    let terrain_path = write_uop_file(
        "terrain",
        &serialize_entry_payload(
            1,
            1,
            &[(0, 3, 0), (1, 561, 0x200), (2, 433, 0x400)],
            2,
            &[(3, 0, 16.0, 0, 0), (4, 0, 4.0, 1, 0), (5, 0, 1.0, 2, 0)],
            &[0, 1, 0, 0],
            &[-0.01, 0.0, 0.0, 0.0],
        ),
        "build/terraindefinition/00000000.bin",
    );

    let package = TerrainDefinitionPackage::load_with_dictionary(&terrain_path, &dict_path)
        .expect("load terrain definition package");

    assert_eq!(package.entries.len(), 1);
    let entry = &package.entries[0];
    assert_eq!(entry.id, 1);
    assert_eq!(entry.name.as_deref(), Some("Grass"));
    assert_eq!(entry.aliases.len(), 3);
    assert_eq!(entry.aliases[0].legacy_land_id, 3);
    let texture = entry.texture.as_ref().expect("texture block should decode");
    assert_eq!(texture.shader_name.as_deref(), Some("UOWaterTerrainLayer"));
    assert_eq!(texture.layers.len(), 3);
    assert_eq!(texture.layers[0].path.as_deref(), Some("01000013_water_alpha.tga"));
    assert_eq!(texture.layers[1].texture_id, Some(2_000_051));
    assert_eq!(texture.layers[1].texture_repetition, 4.0);
    assert_eq!(texture.techniques, vec![0, 1, 0, 0]);
    assert_eq!(texture.tweaks, vec![-0.01, 0.0, 0.0, 0.0]);
    assert_eq!(package.land_tile_ids().into_iter().collect::<Vec<_>>(), vec![3, 433, 561]);
    assert_eq!(package.land_slot_ids().into_iter().collect::<Vec<_>>(), vec![3, 433, 561]);
    assert_eq!(
        package.land_source_texture_ids().into_iter().collect::<Vec<_>>(),
        vec![1_000_013, 1_000_017, 2_000_051]
    );
    assert_eq!(
        package.texture_selections(),
        vec![(3, 2_000_051), (433, 2_000_051), (561, 2_000_051)]
    );

    let _ = std::fs::remove_file(dict_path);
    let _ = std::fs::remove_file(terrain_path);
}

#[test]
fn terrain_definition_decoder_keeps_entry_index_separate_from_land_tile_ids() {
    let terrain_path = write_uop_file(
        "terrain_nodict",
        &serialize_entry_payload(0, 77, &[(0, 900, 0), (1, 901, 0)], 0, &[], &[], &[]),
        "build/terraindefinition/00000000.bin",
    );

    let package = TerrainDefinitionPackage::load(&terrain_path)
        .expect("load terrain definition package without dictionary");

    assert_eq!(package.entries[0].id, 77);
    assert_eq!(package.land_tile_ids().into_iter().collect::<Vec<_>>(), vec![900, 901]);
    assert!(package.land_source_texture_ids().is_empty());
    assert!(package.texture_selections().is_empty());

    let _ = std::fs::remove_file(terrain_path);
}

#[test]
fn terrain_definition_default_primary_is_item_zero() {
    let dict_path = write_uop_file(
        "dictionary_support_layers",
        &serialize_dictionary_payload(&[
            "Sand Cliff E-W",
            "UODefaultTerrainLayer",
            "02000540_Sand_Cliff_EW_A.tga",
            "02000541_Sand_Cliff_EW_B.tga",
            "01000003_noise_alpha.tga",
        ]),
        "build/string_dictionary.bin",
    );
    let terrain_path = write_uop_file(
        "terrain_support_layers",
        &serialize_entry_payload(
            1,
            54,
            &[(0, 54, 0)],
            2,
            &[(3, 0, 1.0, 0, 0), (4, 0, 1.0, 1, 0), (5, 0, 8.0, 2, 0)],
            &[],
            &[],
        ),
        "build/terraindefinition/00000000.bin",
    );

    let package = TerrainDefinitionPackage::load_with_dictionary(&terrain_path, &dict_path)
        .expect("load terrain definition package");

    assert_eq!(package.entries.len(), 1);
    assert_eq!(package.entries[0].primary_texture_id(), Some(2_000_540));
    let (primary_position, primary_layer) = package.entries[0]
        .primary_texture_layer()
        .expect("primary terrain layer");
    assert_eq!(primary_position, 0);
    assert_eq!(primary_layer.texture_id, Some(2_000_540));
    assert_eq!(package.texture_selections(), vec![(54, 2_000_540)]);

    let _ = std::fs::remove_file(dict_path);
    let _ = std::fs::remove_file(terrain_path);
}

#[test]
fn terrain_definition_placeholder_alias_entries_use_material_id_as_runtime_slot() {
    let dict_path = write_uop_file(
        "dictionary_placeholder_alias_slot",
        &serialize_dictionary_payload(&[
            "Tile (blue slate) Ornate",
            "UODefaultTerrainLayer",
            "02000880_Tile_Blue_Slate_A.tga",
            "02000881_Tile_Blue_Slate_B.tga",
            "01000003_noise_alpha.tga",
        ]),
        "build/string_dictionary.bin",
    );
    let terrain_path = write_uop_file(
        "terrain_placeholder_alias_slot",
        &serialize_entry_payload(
            1,
            100,
            &[(0, 0, 0)],
            2,
            &[(3, 0, 4.0, 0, 0), (4, 0, 4.0, 1, 0), (5, 0, 16.0, 2, 0)],
            &[],
            &[],
        ),
        "build/terraindefinition/00000000.bin",
    );

    let package = TerrainDefinitionPackage::load_with_dictionary(&terrain_path, &dict_path)
        .expect("load terrain definition package");

    assert_eq!(package.entries[0].runtime_slot_ids(), vec![100]);
    assert_eq!(package.land_tile_ids().into_iter().collect::<Vec<_>>(), vec![100]);
    assert_eq!(package.texture_selections(), vec![(100, 2_000_880)]);

    let _ = std::fs::remove_file(dict_path);
    let _ = std::fs::remove_file(terrain_path);
}

#[test]
fn terrain_definition_extract_texture_id_normalizes_dictionary_name_patterns() {
    use uocf::utils::path::extract_texture_id_from_path;
    assert_eq!(
        extract_texture_id_from_path(r"Data\WorldArt\00000002_ankh.tga"),
        Some(2)
    );
    assert_eq!(
        extract_texture_id_from_path(r"Data\TileArtLegacy\3.tga"),
        Some(3)
    );
    assert_eq!(
        extract_texture_id_from_path("Data/TileArtEnhanced/02000540_Sand_Cliff_EW_A.tga"),
        Some(2000540)
    );
}

fn layered_entry(shader: &str, repetitions: &[f32]) -> TerrainDefinitionEntry {
    TerrainDefinitionEntry {
        texture: Some(TerrainDefinitionTexture {
            shader_name: Some(shader.to_string()),
            layers: repetitions
                .iter()
                .enumerate()
                .map(|(position, repetition)| TerrainDefinitionTextureLayer {
                    texture_id: Some(position as u32 + 10),
                    path: Some("01000003_noise_alpha.tga".to_string()),
                    texture_repetition: *repetition,
                    ..TerrainDefinitionTextureLayer::default()
                })
                .collect(),
            ..TerrainDefinitionTexture::default()
        }),
        ..TerrainDefinitionEntry::default()
    }
}

#[test]
fn terrain_layer_class_comes_from_the_shader_name_only() {
    assert_eq!(TerrainLayerClass::for_shader(Some("UOWaterTerrainLayer")), TerrainLayerClass::Water);
    assert_eq!(TerrainLayerClass::for_shader(Some("UOWaterShader")), TerrainLayerClass::Water);
    assert_eq!(TerrainLayerClass::for_shader(Some("UOBumpMapTerrainLayer")), TerrainLayerClass::Bump);
    assert_eq!(TerrainLayerClass::for_shader(Some("UODefaultTerrainLayer")), TerrainLayerClass::Default);
    assert_eq!(TerrainLayerClass::for_shader(Some("UOTerrainShader")), TerrainLayerClass::Default);
    assert_eq!(TerrainLayerClass::for_shader(None), TerrainLayerClass::Default);
    for class in [TerrainLayerClass::Default, TerrainLayerClass::Bump, TerrainLayerClass::Water] {
        assert_eq!(TerrainLayerClass::from_u8(class as u8), Some(class));
    }
}

#[test]
fn terrain_layer_roles_are_fixed_by_position() {
    use TerrainLayerRole as R;
    let default_roles = (0..4).map(|p| TerrainLayerClass::Default.role(p)).collect::<Vec<_>>();
    assert_eq!(default_roles, vec![R::ColourA, R::ColourB, R::BlendAlpha, R::Unused]);
    let bump_roles = (0..6).map(|p| TerrainLayerClass::Bump.role(p)).collect::<Vec<_>>();
    assert_eq!(
        bump_roles,
        vec![R::ColourA, R::ColourB, R::BlendAlpha, R::LuminanceBump, R::EnvironmentMap, R::Unused]
    );
    let water_roles = (0..4).map(|p| TerrainLayerClass::Water.role(p)).collect::<Vec<_>>();
    assert_eq!(water_roles, vec![R::Distortion, R::FallbackColour, R::MainColour, R::Unused]);
    assert_eq!(TerrainLayerClass::Water.position_of(R::MainColour), Some(2));
    assert_eq!(TerrainLayerClass::Default.position_of(R::Distortion), None);
    for role in [R::Unused, R::ColourA, R::BlendAlpha, R::MainColour] {
        assert_eq!(R::from_u8(role as u8), Some(role));
    }
}

#[test]
fn terrain_primary_ignores_names_and_stretch() {
    // Grass: item 0 stretch 10 stays the primary (no 4..8 range rule).
    let grass = layered_entry("UODefaultTerrainLayer", &[10.0, 4.0, 32.0]);
    assert_eq!(grass.layer_class(), TerrainLayerClass::Default);
    assert_eq!(grass.primary_texture_layer().map(|(p, _)| p), Some(0));
    assert_eq!(grass.primary_texture_id(), Some(10));

    let water = layered_entry("UOWaterTerrainLayer", &[16.0, 4.0, 1.0]);
    assert_eq!(water.primary_texture_layer().map(|(p, _)| p), Some(1));
    let texture = water.texture.as_ref().expect("texture");
    assert_eq!(
        texture.layer_with_role(TerrainLayerRole::Distortion).map(|(p, _)| p),
        Some(0)
    );
    assert_eq!(texture.layer_role(2), TerrainLayerRole::MainColour);
}
