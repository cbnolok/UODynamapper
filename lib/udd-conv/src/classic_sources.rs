use std::path::{Path, PathBuf};

use color_eyre::eyre;
use log::warn;
use serde::{Deserialize, Serialize};
use uocf::classic::art::{ArtMap, ArtSource};
use uocf::classic::gump::GumpMap;
use uocf::classic::map::MapPlane;
use uocf::uop_container::package::{LoadMode, UopPackage};

use crate::classic_patches::{
    load_map_diff_if_enabled, load_verdata_if_enabled, ClassicPatchOptions,
};
use crate::source_paths::find_first_existing_file;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicMapSource {
    pub map_id: u32,
    pub format: SourceFormat,
    pub path: PathBuf,
}

impl ClassicMapSource {
    pub fn load_plane(
        &self,
        source_dirs: &[PathBuf],
        patch_options: &ClassicPatchOptions,
    ) -> eyre::Result<MapPlane> {
        let mut plane = match self.format {
            SourceFormat::Uop => MapPlane::init_uop(self.path.clone(), self.map_id)?,
            SourceFormat::Mul => {
                if let Some(map_diff) =
                    load_map_diff_if_enabled(source_dirs, self.map_id, patch_options)?
                {
                    MapPlane::init_with_diff(self.path.clone(), self.map_id, map_diff)?
                } else {
                    MapPlane::init(self.path.clone(), self.map_id)?
                }
            }
        };

        if self.format == SourceFormat::Mul {
            if let Some(verdata) = load_verdata_if_enabled(source_dirs, patch_options)? {
                plane = plane.with_verdata(verdata);
            }
        } else if patch_options.any() {
            warn!(
                "Classic map patch files are ignored when converting map{} from UOP.",
                self.map_id
            );
        }

        Ok(plane)
    }
}

pub fn resolve_classic_map_source(
    source_dirs: &[PathBuf],
    map_id: u32,
    preference: SourceFormatPreference,
) -> eyre::Result<ClassicMapSource> {
    let mul_name = format!("map{}.mul", map_id);
    let uop_names = [
        format!("map{}LegacyMUL.uop", map_id),
        format!("map{}.uop", map_id),
        format!("map{}xLegacyMUL.uop", map_id),
        format!("map{}x.uop", map_id),
    ];
    let uop_name_refs = uop_names.iter().map(|name| name.as_str()).collect::<Vec<_>>();

    for format in preference.preferred_first() {
        let path = match format {
            SourceFormat::Mul => find_first_existing_file(source_dirs, &[&mul_name]),
            SourceFormat::Uop => find_first_existing_file(source_dirs, &uop_name_refs),
        };
        if let Some(path) = path {
            if format != source_format_from_preference(preference) {
                warn!(
                    "map{} {} source not found, falling back to {}",
                    map_id,
                    source_format_from_preference(preference).label(),
                    format.label()
                );
            }
            return Ok(ClassicMapSource {
                map_id,
                format,
                path,
            });
        }
    }

    eyre::bail!("Missing map data for map{} (tried .mul and .uop)", map_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicArtSource {
    pub format: SourceFormat,
    pub root: PathBuf,
    pub primary_path: PathBuf,
}

impl ClassicArtSource {
    pub fn art_source(self: &Self) -> ArtSource {
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

fn source_format_from_preference(preference: SourceFormatPreference) -> SourceFormat {
    match preference {
        SourceFormatPreference::Mul => SourceFormat::Mul,
        SourceFormatPreference::Uop => SourceFormat::Uop,
    }
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
            resolve_classic_map_source(&[dir.clone()], 0, SourceFormatPreference::Uop).unwrap();

        assert_eq!(selected.format, SourceFormat::Uop);
        assert_eq!(selected.path, dir.join("map0LegacyMUL.uop"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn map_source_falls_back_to_uop_when_mul_is_missing() {
        let dir = temp_source_dir("map_mul_fallback");
        fs::write(dir.join("map0LegacyMUL.uop"), b"").unwrap();

        let selected =
            resolve_classic_map_source(&[dir.clone()], 0, SourceFormatPreference::Mul).unwrap();

        assert_eq!(selected.format, SourceFormat::Uop);
        assert_eq!(selected.path, dir.join("map0LegacyMUL.uop"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn art_source_preference_can_select_mul_before_uop() {
        let dir = temp_source_dir("art_mul_first");
        fs::write(dir.join("artidx.mul"), b"").unwrap();
        fs::write(dir.join("art.mul"), b"").unwrap();
        fs::write(dir.join("artLegacyMUL.uop"), b"").unwrap();

        let selected =
            resolve_classic_art_source(&[dir.clone()], SourceFormatPreference::Mul).unwrap();

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
            resolve_classic_gump_source(&[dir.clone()], SourceFormatPreference::Uop).unwrap();

        assert_eq!(selected.format, SourceFormat::Uop);
        assert_eq!(selected.primary_path, dir.join("gumpartLegacyMUL.uop"));
        fs::remove_dir_all(dir).unwrap();
    }
}
