//! Tests verifying that `TextureItem` correctly preserves Textures-style refs,
//! raw/normalized paths, and `texture_stretch` through the tileart parsing pipeline.
//!
//! These tests operate at the struct-construction level because a real UOP file
//! is not available in the unit-test environment.

use uocf::ec_material_axes::{EcLogicalFamily, EcPhysicalPackage};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tileart::{
    TaeTexture, TaeTextureImage, TileArtEntry, TextureItem,
};

fn test_dictionary(strings: &[&str]) -> UoStringDictionary {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for string in strings {
        let bytes = string.as_bytes();
        payload.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        payload.extend_from_slice(bytes);
    }
    UoStringDictionary::from_payload_bytes(&payload, "tileart-ref-preservation-tests")
        .expect("build test string dictionary")
}

fn build_entry_with_texture_path(shader: &str, texture_path: &str, stretch: f32) -> (TileArtEntry, UoStringDictionary) {
    let dictionary = test_dictionary(&[shader, texture_path]);
    let entry = TileArtEntry {
        tile_id: 100,
        texture_vector: vec![
            TaeTexture {
                has_texture: 1,
                type_string_off: 1,
                texture_items_count: 1,
                texture_items: vec![TaeTextureImage {
                    name_string_off: 2,
                    texture_stretch: stretch,
                    ..TaeTextureImage::default()
                }],
                ..TaeTexture::default()
            },
            TaeTexture::default(),
            TaeTexture::default(),
            TaeTexture::default(),
        ],
        ..TileArtEntry::default()
    };
    (entry, dictionary)
}

/// A `build/terraintexture/` path must produce `EcLogicalFamily::Textures`,
/// not `Unknown` or `WorldArt`.
#[test]
fn textures_style_ref_not_collapsed() {
    let (entry, dictionary) =
        build_entry_with_texture_path("UOSpriteShader", "build/terraintexture/foo.dds", 1.0);
    let art_data = entry.process(&dictionary);

    let item = art_data
        .texture_items
        .iter()
        .flatten()
        .next()
        .expect("at least one texture item");

    assert_eq!(
        item.ec_logical_family,
        EcLogicalFamily::Textures,
        "build/terraintexture/ path must resolve to EcLogicalFamily::Textures"
    );
    assert_ne!(
        item.ec_logical_family,
        EcLogicalFamily::WorldArt,
        "must not be misclassified as WorldArt"
    );
    assert_ne!(
        item.ec_logical_family,
        EcLogicalFamily::Unknown,
        "must not fall through to Unknown"
    );
}

/// Regression: a `build/worldart/` (or `data/worldart/`) path must still
/// produce `EcLogicalFamily::WorldArt` after adding the new field.
#[test]
fn worldart_ref_still_classified_correctly() {
    let (entry, dictionary) = build_entry_with_texture_path(
        "UOSpriteShader",
        r"Data\WorldArt\02000042_soil_a.tga",
        1.0,
    );
    let art_data = entry.process(&dictionary);

    let item = art_data
        .texture_items
        .iter()
        .flatten()
        .next()
        .expect("at least one texture item");

    assert_eq!(
        item.ec_logical_family,
        EcLogicalFamily::WorldArt,
        "Data\\WorldArt\\ path must resolve to EcLogicalFamily::WorldArt"
    );
}

/// `raw_path` must preserve the original string from the dictionary (including
/// backslashes and original casing), while `normalized_path` must be lowercased
/// with forward slashes. The two values must be distinct for a mixed-case
/// backslash path.
#[test]
fn raw_and_normalized_path_both_preserved() {
    let original = r"Data\WorldArt\02000042_Soil_A.tga";
    let (entry, dictionary) =
        build_entry_with_texture_path("UOSpriteShader", original, 1.0);
    let art_data = entry.process(&dictionary);

    let item = art_data
        .texture_items
        .iter()
        .flatten()
        .next()
        .expect("at least one texture item");

    let raw = item.raw_path.as_deref().expect("raw_path must be Some");
    let normalized = item.normalized_path.as_deref().expect("normalized_path must be Some");

    assert_eq!(raw, original, "raw_path must equal the original dictionary string");
    assert!(
        normalized.contains('/'),
        "normalized_path must use forward slashes, got: {normalized}"
    );
    assert!(
        !normalized.contains('\\'),
        "normalized_path must not contain backslashes, got: {normalized}"
    );
    assert_eq!(
        normalized,
        normalized.to_ascii_lowercase(),
        "normalized_path must be fully lowercase"
    );
    assert_ne!(raw, normalized, "raw and normalized must differ for mixed-case backslash path");
}

/// `texture_stretch` must survive from the raw binary item through to the
/// processed `TextureItem` field.
#[test]
fn texture_stretch_preserved() {
    const STRETCH: f32 = 4.5;
    let (entry, dictionary) =
        build_entry_with_texture_path("UOSpriteShader", r"Data\WorldArt\02000042_soil.tga", STRETCH);
    let art_data = entry.process(&dictionary);

    let item = art_data
        .texture_items
        .iter()
        .flatten()
        .next()
        .expect("at least one texture item");

    assert_eq!(
        item.texture_stretch, STRETCH,
        "texture_stretch must round-trip from raw to TextureItem"
    );
}

/// Direct unit test against `EcLogicalFamily::infer_from_path` for a
/// `data/textures/` path — validates the path inference without needing a
/// full parse round-trip.
#[test]
fn ec_logical_family_infer_data_textures_path() {
    assert_eq!(
        EcLogicalFamily::infer_from_path(r"Data\Textures\00001234_water_alpha.dds"),
        EcLogicalFamily::Textures
    );
}

/// Direct unit test: `EcPhysicalPackage::infer_from_path` classifies
/// `build/terraintexture/` as `TerrainTextureUop`.
#[test]
fn ec_physical_package_infer_terraintexture_path() {
    assert_eq!(
        EcPhysicalPackage::infer_from_path("build/terraintexture/some_texture.dds"),
        EcPhysicalPackage::TerrainTextureUop
    );
}

/// Struct-level construction test: directly create a `TextureItem` with a
/// non-zero stretch and verify the field is accessible (covers Step 3 without
/// needing a full parse round-trip).
#[test]
fn texture_stretch_field_accessible_on_constructed_item() {
    let item = TextureItem {
        texture_stretch: 8.0,
        ..TextureItem::default()
    };
    assert_eq!(item.texture_stretch, 8.0);
}
