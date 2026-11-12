//! The KR client's grunge on static items: the Client Grunge shader setting (art/client_grunge.wesl).
//!
//! The KR sprite shader multiplies a static's colour, after the hue, by its grunge texture `01000045_noise` when the
//! tileart block it draws binds that texture as shader map 1 (research P3.6 in
//! `scripts/dev-helpers/ec-research/ghidra-kr-atmosphere/results/uokr_atmosphere.md`). Which items are grunged comes
//! from tilemeta by item id, whatever art the renderer draws ([`udd_assets::tilemeta::item_refs_have_client_grunge`]),
//! and the grunge is mapped from the item's map position, so CC and EC art at the same place get the same grunge.
//!
//! The texture is KR's: the optional grunge entry of `tex_land_kr.uddp`. Without it the Enhanced Client's image of the
//! same id in `tex_aux_ec.uddp` stands in (a different picture, mean 0.898 against KR's 0.840); without either, one
//! warning, and a white texture that leaves the colour as it is.

use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::uo_files_loader::{TexAuxEcPackageRes, TexLandKrPackageRes};
use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use udd_assets::tilemeta::CLIENT_GRUNGE_TEXTURE_ID;

/// `SpriteInstance::is_wet_flags` bit: the KR client grunges this static. Kept in sync with art/client_grunge.wesl.
pub(crate) const STATIC_FLAG_CLIENT_GRUNGE: u32 = 1 << 10;

/// The texture with no grunge: one white texel, so the multiply leaves the colour as it is.
const NO_GRUNGE_SIDE: u32 = 1;
const NO_GRUNGE_TEXEL: [u8; 4] = [u8::MAX; 4];

/// Where the grunge texture came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientGrungeSource {
    /// KR's own image, `tex_land_kr.uddp`.
    KrLand,
    /// The Enhanced Client's image of the same id, `tex_aux_ec.uddp`.
    EcAux,
    /// Neither package carries it.
    None,
}

/// The packages the grunge texture can come from, in order.
#[derive(SystemParam)]
pub struct ClientGrungePackages<'w> {
    kr_land: Option<Res<'w, TexLandKrPackageRes>>,
    tex_aux: Option<Res<'w, TexAuxEcPackageRes>>,
}

impl ClientGrungePackages<'_> {
    /// [`client_grunge_image`] of these packages.
    pub(crate) fn image(&self) -> (Image, ClientGrungeSource) {
        client_grunge_image(self.kr_land.as_deref(), self.tex_aux.as_deref())
    }
}

/// The grunge texture from the first package that carries it, and which one that was.
pub(crate) fn client_grunge_image(
    kr_land: Option<&TexLandKrPackageRes>,
    tex_aux: Option<&TexAuxEcPackageRes>,
) -> (Image, ClientGrungeSource) {
    if let Some(grunge) = kr_land.and_then(|package| {
        package.0.grunge_rgba()
            .inspect_err(|error| log::warn!("Cannot read the KR grunge texture from tex_land_kr.uddp: {error:#}"))
            .ok()
            .flatten()
    }) {
        return (client_grunge_texture(grunge.width, grunge.height, grunge.rgba), ClientGrungeSource::KrLand);
    }
    if let Some((width, height, rgba)) = tex_aux.and_then(|package| ec_aux_grunge(package)) {
        return (client_grunge_texture(width, height, rgba), ClientGrungeSource::EcAux);
    }
    log::warn!(
        "No package carries the KR client grunge texture ({CLIENT_GRUNGE_TEXTURE_ID:08}): neither tex_land_kr.uddp \
         nor tex_aux_ec.uddp. Client Grunge draws nothing; repack tex_land_kr.uddp to get it."
    );
    (client_grunge_texture(NO_GRUNGE_SIDE, NO_GRUNGE_SIDE, NO_GRUNGE_TEXEL.to_vec()), ClientGrungeSource::None)
}

/// The Enhanced Client's image of the grunge id, when `tex_aux_ec.uddp` holds its pixels.
fn ec_aux_grunge(package: &TexAuxEcPackageRes) -> Option<(u32, u32, Vec<u8>)> {
    let slot = package.0.slot_for_texture_id(CLIENT_GRUNGE_TEXTURE_ID).filter(|slot| !slot.is_missing_source())?;
    let rgba = package.0.load_rgba(CLIENT_GRUNGE_TEXTURE_ID)
        .inspect_err(|error| log::warn!("Cannot read texture {CLIENT_GRUNGE_TEXTURE_ID:08} from tex_aux_ec.uddp: {error:#}"))
        .ok()
        .flatten()?;
    console_logger::one(
        LogSev::Info,
        LogAbout::RenderWorldArt,
        &format!("Client Grunge: tex_land_kr.uddp has no grunge texture; using the Enhanced Client's \
            {CLIENT_GRUNGE_TEXTURE_ID:08} from tex_aux_ec.uddp, a different image."),
    );
    Some((u32::from(slot.width), u32::from(slot.height), rgba))
}

/// The grunge as the client samples it: its 8-bit values as they are (no sRGB decode: the client multiplies the 8-bit
/// colour), wrapping, bilinear, one level.
fn client_grunge_texture(width: u32, height: u32, rgba: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_package_the_grunge_is_one_white_texel() {
        let (image, source) = client_grunge_image(None, None);
        assert_eq!(source, ClientGrungeSource::None);
        assert_eq!(image.width(), NO_GRUNGE_SIDE);
        assert_eq!(image.data.as_deref(), Some(NO_GRUNGE_TEXEL.as_slice()));
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba8Unorm);
    }
}
