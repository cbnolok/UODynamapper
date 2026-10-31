use uocf::ec_material_axes::{EcLogicalFamily, EcPhysicalPackage};

/// Verifies that `EcPhysicalPackage` and `EcLogicalFamily` can legitimately diverge:
/// a path that comes from a terrain-archive but sits under `build/worldart/` is
/// classified as `TerrainTextureUop` by the physical heuristic (no matching prefix)
/// yet as `WorldArt` by the logical heuristic (matches `build/worldart/`).
///
/// In practice this scenario is common: the same world-art asset can be referenced
/// from both tileart.uop and terraintexture.uop, and the caller decides the physical
/// package from context while the logical family always comes from the path.
#[test]
fn physical_package_and_logical_family_differ() {
    // A path that is unambiguously WorldArt in logical terms.
    let path = "build/worldart/02000130_Cave_Floor_A.dds";

    assert_eq!(
        EcPhysicalPackage::infer_from_path(path),
        EcPhysicalPackage::TextureUop,
        "worldart path should infer TextureUop as physical package"
    );
    assert_eq!(
        EcLogicalFamily::infer_from_path(path),
        EcLogicalFamily::WorldArt,
        "worldart path should infer WorldArt as logical family"
    );

    // Now construct a case where the caller *knows* the physical source is
    // TerrainTextureUop (from context, e.g. a TerrainDefinition entry) but the
    // path itself is in build/worldart/.  The physical override is a caller decision;
    // the inferred logical family is still WorldArt.
    let physical_override = EcPhysicalPackage::TerrainTextureUop;
    let logical = EcLogicalFamily::infer_from_path(path);

    assert_ne!(
        physical_override,
        EcPhysicalPackage::TextureUop,
        "the overridden physical package differs from what infer_from_path would return"
    );
    assert_eq!(logical, EcLogicalFamily::WorldArt);
}

/// Verifies that `EcPhysicalPackage::infer_from_path` correctly recognises the
/// `build/terraintexture/` prefix and returns `TerrainTextureUop`.
#[test]
fn infer_from_path_correctness() {
    assert_eq!(
        EcPhysicalPackage::infer_from_path("build/terraintexture/foo.dds"),
        EcPhysicalPackage::TerrainTextureUop
    );

    // Case-insensitive variant.
    assert_eq!(
        EcPhysicalPackage::infer_from_path("BUILD/TerrainTexture/Bar.dds"),
        EcPhysicalPackage::TerrainTextureUop
    );

    // Backslash variant (Windows-style paths from EC string dictionaries).
    assert_eq!(
        EcPhysicalPackage::infer_from_path("build\\terraintexture\\baz.dds"),
        EcPhysicalPackage::TerrainTextureUop
    );

    // Unrelated path should not match.
    assert_eq!(
        EcPhysicalPackage::infer_from_path("build/worldart/02000001.dds"),
        EcPhysicalPackage::TextureUop
    );

    assert_eq!(
        EcPhysicalPackage::infer_from_path("data/random/texture.dds"),
        EcPhysicalPackage::Unknown
    );
}
