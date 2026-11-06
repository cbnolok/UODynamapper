use std::path::{Path, PathBuf};

use color_eyre::eyre;
use serde::{Deserialize, Serialize};
use uocf::classic::art::{ArtMap, ArtSource};
use uocf::classic::gump::GumpMap;
use uocf::classic::facet_sources::{ClassicFacetSources, ClassicMapFormat};
use uocf::classic::map::MapPlane;
use uocf::classic::statics::StaticsReader;
use uocf::uop_container::package::{LoadMode, UopPackage};

use crate::classic_patches::{load_verdata_if_enabled, ClassicPatchOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceFormatPreference {
    Mul,
    Uop,
}

impl SourceFormatPreference {
    pub fn preferred_first(self) -> [SourceFormat; 2] {
        match self {
            Self::Mul => [SourceFormat::Mul, SourceFormat::Uop],
            Self::Uop => [SourceFormat::Uop, SourceFormat::Mul],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Mul,
    Uop,
}

impl SourceFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mul => "MUL",
            Self::Uop => "UOP",
        }
    }
}

impl SourceFormat {
    fn from_map_format(format: ClassicMapFormat) -> Self {
        match format {
            ClassicMapFormat::Mul => Self::Mul,
            ClassicMapFormat::Uop => Self::Uop,
        }
    }

    fn map_format(self) -> ClassicMapFormat {
        match self {
            Self::Mul => ClassicMapFormat::Mul,
            Self::Uop => ClassicMapFormat::Uop,
        }
    }
}

/// A Classic facet's map, statics and dif files, found by [`resolve_classic_map_source`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicMapSource {
    pub map_id: u32,
    pub format: SourceFormat,
    /// The map file (`map0.mul` for a Trammel built from Felucca's files).
    pub path: PathBuf,
    pub facet: ClassicFacetSources,
}

impl ClassicMapSource {
    /// Opens the map plane with the facet's map difs and, as `patch_options` asks, verdata.
    pub fn load_plane(
        &self,
        source_dirs: &[PathBuf],
        patch_options: &ClassicPatchOptions,
    ) -> eyre::Result<MapPlane> {
        self.facet
            .load_map_plane(None, load_verdata_if_enabled(source_dirs, patch_options)?)
    }

    /// Opens the facet's statics for a `width` x `height` map with its static difs and, as `patch_options` asks,
    /// verdata.
    pub fn load_statics_reader(
        &self,
        source_dirs: &[PathBuf],
        width: u32,
        height: u32,
        patch_options: &ClassicPatchOptions,
    ) -> eyre::Result<StaticsReader> {
        self.facet.load_statics_reader(
            width,
            height,
            load_verdata_if_enabled(source_dirs, patch_options)?,
        )
    }
}

/// Finds Classic facet `map_id` in `source_dirs` through the shared resolver
/// ([`ClassicFacetSources::resolve`]): the facet's own difs and the map edition are as `patch_options` asks, and a
/// standard Trammel with no map1 file is built from map0 and the facet-1 difs.
pub fn resolve_classic_map_source(
    source_dirs: &[PathBuf],
    map_id: u32,
    preference: SourceFormatPreference,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<ClassicMapSource> {
    let formats = preference.preferred_first().map(SourceFormat::map_format);
    let facet =
        ClassicFacetSources::resolve(source_dirs, map_id, &formats, patch_options.difs(), patch_options.map_edition)?;
    Ok(ClassicMapSource {
        map_id,
        format: SourceFormat::from_map_format(facet.map.format),
        path: facet.map.path.clone(),
        facet,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicArtSource {
    pub format: SourceFormat,
    pub root: PathBuf,
    pub primary_path: PathBuf,
}

impl ClassicArtSource {
    pub fn art_source(&self) -> ArtSource {
        match self.format {
            SourceFormat::Mul => ArtSource::Mul,
            SourceFormat::Uop => ArtSource::CcUop,
        }
    }

    pub fn load_art_map(&self) -> eyre::Result<ArtMap> {
        ArtMap::load(&self.root)
    }
}

pub fn resolve_classic_art_source(
    source_dirs: &[PathBuf],
    preference: SourceFormatPreference,
) -> eyre::Result<ClassicArtSource> {
    for format in preference.preferred_first() {
        if let Some(source) = resolve_classic_art_source_for_format(source_dirs, format) {
            return Ok(source);
        }
    }

    eyre::bail!(
        "no art sources found in any provided path: expected artLegacyMUL.uop or art.mul/artidx.mul"
    )
}

fn resolve_classic_art_source_for_format(
    source_dirs: &[PathBuf],
    format: SourceFormat,
) -> Option<ClassicArtSource> {
    source_dirs.iter().find_map(|dir| match format {
        SourceFormat::Mul => {
            let idx_path = first_existing(dir, &["artidx.mul"])?;
            let primary_path = first_existing(dir, &["art.mul"])?;
            idx_path.is_file().then(|| ClassicArtSource {
                format,
                root: dir.clone(),
                primary_path,
            })
        }
        SourceFormat::Uop => first_existing(dir, &["artlegacymul.uop", "artLegacyMUL.uop"])
            .map(|primary_path| ClassicArtSource {
                format,
                root: dir.clone(),
                primary_path,
            }),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicGumpSource {
    pub format: SourceFormat,
    pub root: PathBuf,
    pub primary_path: PathBuf,
}

impl ClassicGumpSource {
    pub fn load_gump_map(&self) -> eyre::Result<GumpMap> {
        match self.format {
            SourceFormat::Mul => GumpMap::load(&self.root),
            SourceFormat::Uop => {
                let package = UopPackage::load_with_mode(&self.primary_path, LoadMode::Lazy)?;
                Ok(GumpMap::load_standalone_uop(package))
            }
        }
    }
}

pub fn resolve_classic_gump_source(
    source_dirs: &[PathBuf],
    preference: SourceFormatPreference,
) -> eyre::Result<ClassicGumpSource> {
    for format in preference.preferred_first() {
        if let Some(source) = resolve_classic_gump_source_for_format(source_dirs, format) {
            return Ok(source);
        }
    }

    eyre::bail!("missing Classic gump source")
}

fn resolve_classic_gump_source_for_format(
    source_dirs: &[PathBuf],
    format: SourceFormat,
) -> Option<ClassicGumpSource> {
    source_dirs.iter().find_map(|dir| match format {
        SourceFormat::Mul => {
            let idx_path = first_existing(dir, &["gumpidx.mul", "Gumpidx.mul"])?;
            let primary_path = first_existing(dir, &["gumpart.mul", "Gumpart.mul"])?;
            idx_path.is_file().then(|| ClassicGumpSource {
                format,
                root: dir.clone(),
                primary_path,
            })
        }
        SourceFormat::Uop => first_existing(
            dir,
            &[
                "gumpartLegacyMUL.uop",
                "GumpartLegacyMUL.uop",
                "gumpartlegacymul.uop",
            ],
        )
        .map(|primary_path| ClassicGumpSource {
            format,
            root: dir.clone(),
            primary_path,
        }),
    })
}

fn first_existing(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use uocf::map_edition::MapEdition;

    fn temp_source_dir(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "udd_classic_sources_{test_name}_{}_{}",
            std::process::id(),
            unique
        ));
        fs::create_dir_all(&dir).expect("create temp source dir");
        dir
    }

    #[test]
    fn map_source_preference_can_select_uop_before_mul() {
        let dir = temp_source_dir("map_uop_first");
        fs::write(dir.join("map0.mul"), b"").unwrap();
        fs::write(dir.join("map0LegacyMUL.uop"), b"").unwrap();

        let selected =
            resolve_classic_map_source(
                std::slice::from_ref(&dir),
                0,
                SourceFormatPreference::Uop,
                &ClassicPatchOptions::NONE,
            )
            .unwrap();

        assert_eq!(selected.format, SourceFormat::Uop);
        assert_eq!(selected.path, dir.join("map0LegacyMUL.uop"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn map_source_falls_back_to_uop_when_mul_is_missing() {
        let dir = temp_source_dir("map_mul_fallback");
        fs::write(dir.join("map0LegacyMUL.uop"), b"").unwrap();

        let selected =
            resolve_classic_map_source(
                std::slice::from_ref(&dir),
                0,
                SourceFormatPreference::Mul,
                &ClassicPatchOptions::NONE,
            )
            .unwrap();

        assert_eq!(selected.format, SourceFormat::Uop);
        assert_eq!(selected.path, dir.join("map0LegacyMUL.uop"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn map1_falls_back_to_map0_with_the_facet_1_difs() {
        let dir = temp_source_dir("map1_from_map0");
        for name in [
            "map0.mul",
            "staidx0.mul",
            "statics0.mul",
            "mapdif1.mul",
            "mapdifl1.mul",
            "stadif1.mul",
            "stadifl1.mul",
            "stadifi1.mul",
        ] {
            fs::write(dir.join(name), b"").unwrap();
        }

        let selected = resolve_classic_map_source(
            std::slice::from_ref(&dir),
            1,
            SourceFormatPreference::Mul,
            &ClassicPatchOptions::NONE,
        )
        .unwrap();

        assert_eq!(selected.format, SourceFormat::Mul);
        assert_eq!(selected.path, dir.join("map0.mul"));
        assert_eq!(selected.facet.base_facet, Some(0));
        assert!(selected.facet.map_difs.is_some() && selected.facet.static_difs.is_some());

        fs::remove_file(dir.join("stadif1.mul")).unwrap();
        let error = resolve_classic_map_source(
            std::slice::from_ref(&dir),
            1,
            SourceFormatPreference::Mul,
            &ClassicPatchOptions::NONE,
        )
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("Missing map data for map1"), "{message}");
        assert!(message.contains("stadif1.mul is missing"), "{message}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn map_edition_variant_selects_the_x_files_without_fallback() {
        let dir = temp_source_dir("map_variant");
        for name in ["map1.mul", "staidx1.mul", "statics1.mul", "map1xLegacyMUL.uop", "staidx1x.mul", "statics1x.mul"] {
            fs::write(dir.join(name), b"").unwrap();
        }
        let variant = ClassicPatchOptions { map_edition: MapEdition::Variant, ..ClassicPatchOptions::NONE };

        let selected =
            resolve_classic_map_source(std::slice::from_ref(&dir), 1, SourceFormatPreference::Mul, &variant).unwrap();
        assert_eq!(selected.format, SourceFormat::Uop);
        assert_eq!(selected.path, dir.join("map1xLegacyMUL.uop"));
        assert_eq!(selected.facet.statics_files().unwrap().index, dir.join("staidx1x.mul"));

        let message = format!(
            "{:#}",
            resolve_classic_map_source(std::slice::from_ref(&dir), 0, SourceFormatPreference::Mul, &variant)
                .unwrap_err()
        );
        assert!(message.contains("variant edition of map0") && message.contains("map0x.mul"), "{message}");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn art_source_preference_can_select_mul_before_uop() {
        let dir = temp_source_dir("art_mul_first");
        fs::write(dir.join("artidx.mul"), b"").unwrap();
        fs::write(dir.join("art.mul"), b"").unwrap();
        fs::write(dir.join("artLegacyMUL.uop"), b"").unwrap();

        let selected =
            resolve_classic_art_source(std::slice::from_ref(&dir), SourceFormatPreference::Mul).unwrap();

        assert_eq!(selected.format, SourceFormat::Mul);
        assert_eq!(selected.primary_path, dir.join("art.mul"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gump_source_preference_can_select_uop_before_mul() {
        let dir = temp_source_dir("gump_uop_first");
        fs::write(dir.join("gumpidx.mul"), b"").unwrap();
        fs::write(dir.join("gumpart.mul"), b"").unwrap();
        fs::write(dir.join("gumpartLegacyMUL.uop"), b"").unwrap();

        let selected =
            resolve_classic_gump_source(std::slice::from_ref(&dir), SourceFormatPreference::Uop).unwrap();

        assert_eq!(selected.format, SourceFormat::Uop);
        assert_eq!(selected.primary_path, dir.join("gumpartLegacyMUL.uop"));
        fs::remove_dir_all(dir).unwrap();
    }
}
