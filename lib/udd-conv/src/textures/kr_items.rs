//! Build-time support for the Kingdom Reborn item packages, which `pack-kr-land` writes beside `tex_land_kr.uddp`.
//!
//! Each has an Enhanced layout and is built by the Enhanced packer from the Kingdom Reborn client alone, so the
//! Enhanced readers open it and nothing of KR's item data is replaced by the Enhanced Client's:
//! - [`TILEMETA_KR_FILE_NAME`]: the `tilemeta.uddp` layout, Classic `tiledata.mul` with KR's `tileart.uop` (record
//!   version 3, its flags moved to the EC layout by `uocf`), and every KR tileart record byte for byte;
//! - [`TEX_ART_KR_FILE_NAME`]: the `tex_art_ec.uddp` layout, the block-0 world art of KR's `Texture.uop`;
//! - [`TEX_AUX_KR_FILE_NAME`]: the `tex_aux_ec.uddp` layout, KR's material inputs (masks, noise, water maps), its
//!   `missing.tga`, atmosphere maps and light textures.
//!
//! The images are lossless RGBA8888 with Zstd: the alpha trim of the art keeps every opaque texel, and the trimmed
//! offsets go to the tilemeta's sampling starts, as for the Enhanced Client.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use log::info;

use crate::metadata::tilemeta::{build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources, TileMetaBuildOptions};
use crate::textures::tex_art_ec::{
    convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources, load_tex_art_ec_sources, TexArtEcAtlasOptions,
    TexArtEcBuildSummary,
};
use crate::textures::tex_aux_ec::{pack_tex_aux_ec, TexAuxEcPackOptions, TexAuxEcPackSummary};

/// The Kingdom Reborn tileart and tile data, in the `tilemeta.uddp` layout.
pub const TILEMETA_KR_FILE_NAME: &str = "tilemeta_kr.uddp";
/// The Kingdom Reborn item world art, in the `tex_art_ec.uddp` layout.
pub const TEX_ART_KR_FILE_NAME: &str = "tex_art_kr.uddp";
/// The Kingdom Reborn item material inputs, in the `tex_aux_ec.uddp` layout.
pub const TEX_AUX_KR_FILE_NAME: &str = "tex_aux_kr.uddp";

/// Where the Kingdom Reborn item packages go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrItemOutputs {
    pub tilemeta: PathBuf,
    pub art: PathBuf,
    pub aux: PathBuf,
}

impl KrItemOutputs {
    /// The packages under their file names, in the folder of `land_output` (the `tex_land_kr.uddp` they go with).
    pub fn beside(land_output: &Path) -> Self {
        Self {
            tilemeta: land_output.with_file_name(TILEMETA_KR_FILE_NAME),
            art: land_output.with_file_name(TEX_ART_KR_FILE_NAME),
            aux: land_output.with_file_name(TEX_AUX_KR_FILE_NAME),
        }
    }
}

#[derive(Debug, Clone)]
pub struct KrItemBuildSummary {
    pub art: TexArtEcBuildSummary,
    pub aux: TexAuxEcPackSummary,
}

/// Builds the Kingdom Reborn item packages from the KR client in `kr_dir`, with the Classic `tiledata.mul` (and
/// `radarcol.mul`) of `cc_dir` as the Enhanced tilemeta takes them (see the module documentation).
pub fn pack_kr_items(kr_dir: &Path, cc_dir: &Path, outputs: &KrItemOutputs) -> eyre::Result<KrItemBuildSummary> {
    info!("Converting the Kingdom Reborn items to {}, {} and {}", outputs.tilemeta.display(), outputs.art.display(),
        outputs.aux.display());
    let sources = load_tex_art_ec_sources(&[kr_dir.to_path_buf()])
        .wrap_err_with(|| format!("load the Kingdom Reborn tileart and textures from {}", kr_dir.display()))?;
    let art = convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources(
        &sources,
        &outputs.art,
        &TexArtEcAtlasOptions::default(),
    )?;
    build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources(
        cc_dir,
        &sources,
        &art.ec_crop_adjustments,
        &outputs.tilemeta,
        &TileMetaBuildOptions::default(),
    )?;
    let aux = pack_tex_aux_ec(&[kr_dir.to_path_buf()], &outputs.tilemeta, &outputs.aux, &TexAuxEcPackOptions::default())?;
    info!(
        "Kingdom Reborn items: {} art slots on {} pages; {} material textures ({} source-absent ids aliased to the \
         missing texture)",
        art.populated_slot_count, art.page_count, aux.included, aux.excluded_source_absent,
    );
    Ok(KrItemBuildSummary { art, aux })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_packages_go_beside_the_land_package() {
        let outputs = KrItemOutputs::beside(Path::new("/packages/tex_land_kr.uddp"));
        assert_eq!(outputs.tilemeta, Path::new("/packages/tilemeta_kr.uddp"));
        assert_eq!(outputs.art, Path::new("/packages/tex_art_kr.uddp"));
        assert_eq!(outputs.aux, Path::new("/packages/tex_aux_kr.uddp"));
    }
}
