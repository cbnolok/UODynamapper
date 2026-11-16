use uocf::enhanced::tileart::*;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::utils::path::extract_texture_id_from_path;

/// A dictionary naming `strings` by the indices 1 on, as tileart records name them.
fn test_dictionary(strings: &[&str]) -> UoStringDictionary {
    UoStringDictionary::from_strings(strings.iter().copied())
}

#[test]
fn tileart_get_tile_type_promotes_rotated_sprite_tiles_to_solid() {
    let dictionary = test_dictionary(&["UOSpriteShader"]);
    let entry = TileArtEntry {
        flags1: TaeFlag::Rotated,
        ..TileArtEntry::default()
    };
    let texture = TaeTexture {
        has_texture: 1,
        type_string_off: 1,
        texture_items: vec![TaeTextureImage {
            texture_stretch: 1.0,
            ..TaeTextureImage::default()
        }],
        ..TaeTexture::default()
    };

    assert_eq!(entry.get_tile_type(&texture, &dictionary), TileType::Solid);
}

#[test]
fn tileart_get_tile_type_keeps_plain_sprite_tiles_static() {
    let dictionary = test_dictionary(&["UOSpriteShader"]);
    let entry = TileArtEntry::default();
    let texture = TaeTexture {
        has_texture: 1,
        type_string_off: 1,
        texture_items: vec![TaeTextureImage {
            texture_stretch: 1.0,
            ..TaeTextureImage::default()
        }],
        ..TaeTexture::default()
    };

    assert_eq!(entry.get_tile_type(&texture, &dictionary), TileType::Static);
}

#[test]
fn tileart_extract_texture_id_normalizes_dictionary_name_patterns() {
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

#[test]
fn tileart_classify_texture_path_normalizes_case_and_slashes() {
    assert_eq!(
        classify_texture_path("data/worldart/00000002_ankh.tga"),
        TextureType::WorldArt
    );
    assert_eq!(
        classify_texture_path(r"DATA\TILEARTLEGACY\3.tga"),
        TextureType::TileArtLegacy
    );
    assert_eq!(
        classify_texture_path("Data/TileArtEnhanced/00000004_tree.tga"),
        TextureType::TileArtEnhanced
    );
}
