#![allow(dead_code)]

use std::sync::Arc;

use bevy::prelude::*;
use color_eyre::eyre::{self, WrapErr};

use crate::{
    core::{
        system_sets::StartupSysSet,
        uo_files_loader::{CcArtPackageRes, EcArtPackageRes, TileMetaPackageRes},
    },
    external_data::settings::ClientTextureSource,
    prelude::*,
};
use uddconv::{
    bc7::{self, ImageExtent},
    cc_art::{CcArtPackage, PagePixelFormat},
    ec_art::EcArtPackage,
    tilemeta::TileMetaPackage,
};

#[derive(Debug, Clone)]
pub struct LoadedArtTexture {
    pub art_id: u32,
    pub source: ClientTextureSource,
    pub texture_id: u32,
    pub sample_start_x: i16,
    pub sample_start_y: i16,
    pub offset_x: i16,
    pub offset_y: i16,
    pub width: u16,
    pub height: u16,
    pub rgba8: Vec<u8>,
}

#[derive(Resource, Default, Clone)]
pub struct ArtTextureLoader {
    cc_art: Option<Arc<CcArtPackage>>,
    ec_art: Option<Arc<EcArtPackage>>,
    tilemeta: Option<Arc<TileMetaPackage>>,
}

impl ArtTextureLoader {
    pub fn source_available(&self, source: ClientTextureSource) -> bool {
        match source {
            ClientTextureSource::Cc => self.cc_art.is_some(),
            ClientTextureSource::Ec => self.ec_art.is_some() && self.tilemeta.is_some(),
        }
    }

    pub fn load_selected_texture(
        &self,
        art_id: u32,
        settings: &crate::external_data::settings::SectGraphics,
    ) -> eyre::Result<Option<LoadedArtTexture>> {
        self.load_texture(art_id, settings.art_texture_source)
    }

    pub fn load_texture(
        &self,
        art_id: u32,
        source: ClientTextureSource,
    ) -> eyre::Result<Option<LoadedArtTexture>> {
        match source {
            ClientTextureSource::Cc => self.load_cc_texture(art_id),
            ClientTextureSource::Ec => self.load_ec_texture(art_id),
        }
    }

    fn load_cc_texture(&self, art_id: u32) -> eyre::Result<Option<LoadedArtTexture>> {
        let Some(package) = &self.cc_art else {
            return Ok(None);
        };
        let Some(slot) = package.present_slot(art_id) else {
            return Ok(None);
        };
        let page_data = package
            .read_page_bytes(slot.page_index)
            .wrap_err_with(|| format!("read cc_art page {}", slot.page_index))?;
        let page_meta = package
            .pages()
            .get(slot.page_index as usize)
            .ok_or_else(|| eyre::eyre!("cc_art missing page metadata for {}", slot.page_index))?;
        let rgba8 = extract_slot_rgba(
            &page_data,
            package.atlas_width(),
            package.atlas_height(),
            page_meta.used_width,
            page_meta.used_height,
            page_meta.pixel_format,
            slot.x,
            slot.y,
            slot.width,
            slot.height,
        )?;

        let metadata = self.tilemeta.as_ref().and_then(|tilemeta| tilemeta.item_tile(art_id));
        let texture_id = metadata
            .and_then(|item| (item.cc_texture_id != 0 || art_id == 0).then_some(item.cc_texture_id))
            .unwrap_or(art_id);
        Ok(Some(LoadedArtTexture {
            art_id,
            source: ClientTextureSource::Cc,
            texture_id,
            sample_start_x: metadata.map(|item| item.cc_start_x).unwrap_or(0),
            sample_start_y: metadata.map(|item| item.cc_start_y).unwrap_or(0),
            offset_x: metadata.map(|item| item.cc_offset_x).unwrap_or(0),
            offset_y: metadata.map(|item| item.cc_offset_y).unwrap_or(0),
            width: slot.width,
            height: slot.height,
            rgba8,
        }))
    }

    fn load_ec_texture(&self, art_id: u32) -> eyre::Result<Option<LoadedArtTexture>> {
        let Some(tilemeta) = &self.tilemeta else {
            return Ok(None);
        };
        let Some(package) = &self.ec_art else {
            return Ok(None);
        };
        let Some(metadata) = tilemeta.item_tile(art_id) else {
            return Ok(None);
        };
        let Some(slot) = package.present_slot(art_id) else {
            return Ok(None);
        };
        let page_data = package
            .read_page_bytes(slot.page_index)
            .wrap_err_with(|| format!("read ec_art page {}", slot.page_index))?;
        let page_meta = package
            .pages()
            .get(slot.page_index as usize)
            .ok_or_else(|| eyre::eyre!("ec_art missing page metadata for {}", slot.page_index))?;
        let rgba8 = extract_slot_rgba(
            &page_data,
            package.atlas_width(),
            package.atlas_height(),
            page_meta.used_width,
            page_meta.used_height,
            page_meta.pixel_format,
            slot.x,
            slot.y,
            slot.width,
            slot.height,
        )?;

        Ok(Some(LoadedArtTexture {
            art_id,
            source: ClientTextureSource::Ec,
            texture_id: metadata.ec_texture_id,
            sample_start_x: metadata.ec_start_x,
            sample_start_y: metadata.ec_start_y,
            offset_x: metadata.ec_offset_x,
            offset_y: metadata.ec_offset_y,
            width: slot.width,
            height: slot.height,
            rgba8,
        }))
    }
}

fn extract_slot_rgba(
    page_data: &[u8],
    _atlas_width: u32,
    _atlas_height: u32,
    used_width: u32,
    used_height: u32,
    pixel_format: PagePixelFormat,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
) -> eyre::Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Ok(Vec::new());
    }

    match pixel_format {
        PagePixelFormat::Rgba8888 => Ok(bc7::extract_rgba8888_subrect_arc(
            page_data,
            used_width,
            x as u32,
            y as u32,
            width as u32,
            height as u32,
        )
        .to_vec()),
        PagePixelFormat::Bc7 => {
            let page_extent = ImageExtent::new(used_width, used_height)
                .map_err(|error| eyre::eyre!("invalid stored page extent {used_width}x{used_height}: {error}"))?;
            let slot_extent = ImageExtent::new(width as u32, height as u32)
                .map_err(|error| eyre::eyre!("invalid slot extent {}x{}: {error}", width, height))?;
            let blocks = bc7::extract_bc7_subrect(
                page_data,
                page_extent,
                x as u32,
                y as u32,
                slot_extent,
            );
            bc7::decode_bc7_to_rgba8888(&blocks, slot_extent)
                .map_err(|error| eyre::eyre!("decode art BC7 subrect: {error}"))
        }
    }
}

pub struct ArtTextureLoaderPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(ArtTextureLoaderPlugin);

impl Plugin for ArtTextureLoaderPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(
            Startup,
            sys_setup_art_texture_loader
                .in_set(StartupSysSet::SetupSceneStage1)
                .after(StartupSysSet::LoadStartupUOFiles),
        );
    }
}

fn sys_setup_art_texture_loader(
    mut commands: Commands,
    cc_art_r: Option<Res<CcArtPackageRes>>,
    ec_art_r: Option<Res<EcArtPackageRes>>,
    tilemeta_r: Option<Res<TileMetaPackageRes>>,
) {
    commands.insert_resource(ArtTextureLoader {
        cc_art: cc_art_r.map(|r| r.0.clone()),
        ec_art: ec_art_r.map(|r| r.0.clone()),
        tilemeta: tilemeta_r.map(|r| r.0.clone()),
    });
}
