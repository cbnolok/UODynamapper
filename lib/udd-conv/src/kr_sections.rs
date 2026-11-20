//! Nesting the Kingdom Reborn packages in the Enhanced packages they belong to.
//!
//! Every Enhanced package Kingdom Reborn draws differently holds a Kingdom Reborn package, whole and uncompressed, at
//! `udd_assets::common::KR_SECTION_ENTRY_PATH`; readers open it in place (`udd_assets::common::open_kr_section`).
//! `tex_land_ec.uddp` holds the Kingdom Reborn land (`crate::tex_land_kr`), `tilemeta.uddp` its tile data, and
//! `tex_art_ec.uddp` and `tex_aux_ec.uddp` the overlays of what its items draw differently (`crate::kr_items`).
//!
//! The packers build each section from a Kingdom Reborn client folder that [`KrSectionWork`] rebuilds beside their
//! output (`crate::kr_delta`), write it there, and add it to the Enhanced package they just wrote
//! ([`embed_kr_section`]). An overlay is measured against the Enhanced package it is nested in, packed in the same run.

use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use log::info;

use udd_assets::common::KR_SECTION_ENTRY_PATH;
use udd_container::uddp::FormatError;
use udd_container::{xxh64_virtual_path, BuildProgress, DataType, UddpBuilder, UddpReader};

use crate::kr_delta::{prepare_kr_client, KrClientSource, RebuiltKrClient};
use crate::kr_items::{
    load_ec_sources_with_kr_windows, pack_kr_art_and_tilemeta, pack_kr_aux, KrArtBuildSummary, KrTilemetaOutput,
};
use crate::package_progress::AssetTaskProgress;
use crate::tex_art_ec::{TexArtEcAtlasOptions, TexArtEcLoadedSources};
use crate::tex_aux_ec::TexAuxEcOverlaySummary;
use crate::tilemeta_overrides::apply_bundled_tilemeta_overrides;
use crate::tex_land_kr::{convert_kr_land_to_tex_land_kr_uddp_with_progress, TexLandKrBuildSummary, TexLandKrPackOptions};

/// The work file of the Kingdom Reborn land section.
pub const KR_LAND_SECTION_FILE_NAME: &str = crate::tex_land_kr::TEX_LAND_KR_FILE_NAME;
/// The work file of the Kingdom Reborn tile data section.
pub const KR_TILEMETA_SECTION_FILE_NAME: &str = "tilemeta_kr.uddp";
/// The work file of the Kingdom Reborn item art section.
pub const KR_ART_SECTION_FILE_NAME: &str = "tex_art_kr.uddp";
/// The work file of the Kingdom Reborn material texture section.
pub const KR_AUX_SECTION_FILE_NAME: &str = "tex_aux_kr.uddp";
/// The suffix of a package being written, renamed over the package once complete.
const PARTIAL_SUFFIX: &str = ".partial";

/// A Kingdom Reborn client folder rebuilt beside a packer's output, where the sections are written before being
/// nested; removed with everything in it when dropped.
#[derive(Debug)]
pub struct KrSectionWork {
    client: RebuiltKrClient,
}

impl KrSectionWork {
    /// Rebuilds the Kingdom Reborn client files from `source` over the Enhanced Client in `ec_dir`, in a folder beside
    /// `output`.
    pub fn prepare(source: &KrClientSource, ec_dir: &Path, output: &Path) -> eyre::Result<Self> {
        let parent = match output.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        Ok(Self { client: prepare_kr_client(source, ec_dir, parent)? })
    }

    /// The rebuilt Kingdom Reborn client folder.
    pub fn kr_dir(&self) -> &Path {
        self.client.dir()
    }

    /// Where the section `file_name` is written.
    pub fn section_path(&self, file_name: &str) -> PathBuf {
        self.client.dir().join(file_name)
    }

    /// The Enhanced sources in `ec_dirs` the packages with a Kingdom Reborn section are packed from, with the Kingdom
    /// Reborn world-art windows they adopt (`crate::kr_items::load_ec_sources_with_kr_windows`).
    pub fn load_ec_sources(&self, ec_dirs: &[PathBuf]) -> eyre::Result<TexArtEcLoadedSources> {
        load_ec_sources_with_kr_windows(ec_dirs, self.kr_dir())
    }
}

/// Adds the package `section` to `package` at [`KR_SECTION_ENTRY_PATH`], stored uncompressed; `package` must not hold
/// one yet. The package's own entries are copied as stored.
pub fn embed_kr_section(package: &Path, section: &Path) -> eyre::Result<()> {
    let section_bytes = fs::read(section).wrap_err_with(|| format!("read {}", section.display()))?;
    let section_size = section_bytes.len();
    let base = UddpReader::load_in_memory(package).wrap_err_with(|| format!("read {}", package.display()))?;
    let path_hash64 = xxh64_virtual_path(KR_SECTION_ENTRY_PATH);
    if base.find_by_path_hash(path_hash64).is_some() {
        eyre::bail!("{} already holds a Kingdom Reborn section", package.display());
    }
    let bytes = UddpBuilder::extend_package(&base, vec![(path_hash64, DataType::Metadata as u8, section_bytes)])
        .wrap_err_with(|| format!("add the Kingdom Reborn section to {}", package.display()))?;
    drop(base);
    let mut partial = package.as_os_str().to_owned();
    partial.push(PARTIAL_SUFFIX);
    let partial = PathBuf::from(partial);
    fs::write(&partial, &bytes).wrap_err_with(|| format!("write {}", partial.display()))?;
    fs::rename(&partial, package).wrap_err_with(|| format!("replace {}", package.display()))?;
    info!("Nested the Kingdom Reborn section ({section_size} bytes) in {} ({} bytes)", package.display(), bytes.len());
    Ok(())
}

/// Packs the Kingdom Reborn item art against `ec` ([`KrSectionWork::load_ec_sources`]) and nests it in `art_package`,
/// the `tex_art_ec.uddp` packed from `ec` with the options beside it, whose encoding the Kingdom Reborn art takes; with
/// `tilemeta` (the Classic folder and the `tilemeta.uddp` packed from `ec`), also the Kingdom Reborn tile data, nested
/// in that `tilemeta.uddp`, and then the bundled tilemeta overrides, which read it (`crate::tilemeta_overrides`).
/// Without `art_package` the art is packed for the tile data alone.
pub fn add_kr_art_and_tilemeta_sections(work: &KrSectionWork, ec: &TexArtEcLoadedSources,
    art_package: Option<(&Path, &TexArtEcAtlasOptions)>, tilemeta: Option<(&Path, &Path)>)
    -> eyre::Result<KrArtBuildSummary> {
    let art_section = work.section_path(KR_ART_SECTION_FILE_NAME);
    let tilemeta_section = work.section_path(KR_TILEMETA_SECTION_FILE_NAME);
    let default_art_options = TexArtEcAtlasOptions::default();
    let ec_art_options = art_package.map_or(&default_art_options, |(_, options)| options);
    let summary = pack_kr_art_and_tilemeta(work.kr_dir(), ec, &art_section, ec_art_options,
        tilemeta.map(|(cc_dir, _)| KrTilemetaOutput { cc_dir, out: &tilemeta_section }))?;
    if let Some((art_package, _)) = art_package {
        embed_kr_section(art_package, &art_section)?;
    }
    if let Some((_, tilemeta_package)) = tilemeta {
        embed_kr_section(tilemeta_package, &tilemeta_section)?;
        apply_bundled_tilemeta_overrides(tilemeta_package)?;
    }
    Ok(summary)
}

/// Packs the Kingdom Reborn land with `options` (those of the Enhanced land) and nests it in `land_package`, the
/// `tex_land_ec.uddp` packed from the Enhanced Client in `ec_dir`.
pub fn add_kr_land_section(
    work: &KrSectionWork,
    ec_dir: &Path,
    land_package: &Path,
    options: &TexLandKrPackOptions,
    payload_progress: impl Fn(AssetTaskProgress) + Sync,
    package_progress: impl FnMut(BuildProgress),
) -> eyre::Result<TexLandKrBuildSummary> {
    let section = work.section_path(KR_LAND_SECTION_FILE_NAME);
    let summary = convert_kr_land_to_tex_land_kr_uddp_with_progress(work.kr_dir(), ec_dir, land_package, &section, options,
        payload_progress, package_progress)?;
    info!("Kingdom Reborn land: {summary}");
    embed_kr_section(land_package, &section)?;
    Ok(summary)
}

/// Packs the Kingdom Reborn material textures and nests them in `aux_package`, the `tex_aux_ec.uddp` packed from the
/// Enhanced Client in `ec_dir`, the Classic Client in `cc_dir` and `tilemeta_package`, whose Kingdom Reborn section
/// gives the Kingdom Reborn texture refs.
pub fn add_kr_aux_section(work: &KrSectionWork, ec_dir: &Path, cc_dir: &Path, tilemeta_package: &Path,
    aux_package: &Path) -> eyre::Result<TexAuxEcOverlaySummary> {
    let tilemeta_section = work.section_path(KR_TILEMETA_SECTION_FILE_NAME);
    if !extract_kr_section(tilemeta_package, &tilemeta_section)? {
        eyre::bail!("{} holds no Kingdom Reborn tile data: repack it with the current udd-cli first",
            tilemeta_package.display());
    }
    let section = work.section_path(KR_AUX_SECTION_FILE_NAME);
    let summary = pack_kr_aux(work.kr_dir(), ec_dir, cc_dir, &tilemeta_section, &section)?;
    embed_kr_section(aux_package, &section)?;
    Ok(summary)
}

/// Writes the Kingdom Reborn package nested in `package` to `out`; `false` when `package` holds none.
pub fn extract_kr_section(package: &Path, out: &Path) -> eyre::Result<bool> {
    let reader = UddpReader::load(package).wrap_err_with(|| format!("read {}", package.display()))?;
    let bytes = match reader.read_file_by_path_hash_cow(xxh64_virtual_path(KR_SECTION_ENTRY_PATH)) {
        Ok(bytes) => bytes,
        Err(FormatError::FileNotFound) => return Ok(false),
        Err(error) => return Err(eyre::eyre!("{}: {KR_SECTION_ENTRY_PATH}: {error}", package.display())),
    };
    fs::write(out, &bytes).wrap_err_with(|| format!("write {}", out.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::common::open_kr_section;
    use udd_container::{AddOwnedFileRequest, CompressionFlag, LookupMode, UddpReaderOptions};

    const OUTER_PATH: &str = "metadata/outer.bin";
    const INNER_PATH: &str = "metadata/inner.bin";

    fn package(path: &str, data: &[u8]) -> Vec<u8> {
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        builder
            .add_owned_file(AddOwnedFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::ZstdNoDict,
                width: 0,
                height: 0,
                virtual_path: Some(path.to_owned()),
                path_hash64: None,
                id: None,
                data: data.to_vec(),
            })
            .unwrap();
        builder.into_bytes().unwrap()
    }

    #[test]
    fn a_nested_section_reads_back_in_place_and_is_added_once() {
        let dir = std::env::temp_dir().join(format!("kr_sections_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (outer, inner, extracted) = (dir.join("outer.uddp"), dir.join("inner.uddp"), dir.join("extracted.uddp"));
        fs::write(&outer, package(OUTER_PATH, b"ec")).unwrap();
        fs::write(&inner, package(INNER_PATH, b"kr")).unwrap();

        assert!(!extract_kr_section(&outer, &extracted).unwrap());
        embed_kr_section(&outer, &inner).unwrap();
        let reader = UddpReader::load(&outer).unwrap();
        assert_eq!(reader.read_file_by_path_hash(xxh64_virtual_path(OUTER_PATH)).unwrap(), b"ec");
        let section = open_kr_section(&reader, UddpReaderOptions::disabled()).unwrap().expect("a section");
        assert_eq!(section.read_file_by_path_hash(xxh64_virtual_path(INNER_PATH)).unwrap(), b"kr");
        assert!(extract_kr_section(&outer, &extracted).unwrap());
        assert_eq!(fs::read(&extracted).unwrap(), fs::read(&inner).unwrap());
        assert!(embed_kr_section(&outer, &inner).is_err());

        fs::remove_dir_all(&dir).unwrap();
    }
}
