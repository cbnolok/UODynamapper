//! The `tex_aux_ec` array layers: decoded RGBA8, or the package's BC7 blocks copied whole.

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use color_eyre::eyre;
use udd_assets::bc7::{self, expected_bc7_byte_len, ImageExtent, VramTextureData, BC7_BLOCK_BYTES, BC7_BLOCK_EDGE};
use udd_assets::tex_aux_ec::{TexAuxEcPackage, TexAuxEcSlotRecord};

use super::{pad_aux_rgba, AUX_ATLAS_TILE_SIZE};
use crate::console_logger::{self, LogAbout, LogSev};

const RGBA_BYTES: usize = 4;
const _: () = assert!(AUX_ATLAS_TILE_SIZE % BC7_BLOCK_EDGE == 0, "BC7 aux layers need a block-aligned edge");

/// How the auxiliary array stores its texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuxLayerFormat {
    /// Decoded RGBA8888: RGBA packages, and packages that mix formats.
    Rgba8,
    /// The package's BC7 blocks. Zero blocks decode to transparent black, like the RGBA padding.
    Bc7,
}

impl AuxLayerFormat {
    /// BC7 when every texture of the package is BC7. A package that mixes formats is decoded, with one warning naming
    /// it.
    pub(crate) fn for_package(package: &TexAuxEcPackage) -> Self {
        let (format, warning) = Self::for_slots(package.slots());
        if let Some(warning) = warning {
            console_logger::one(LogSev::Warn, LogAbout::RenderWorldLand, &warning);
        }
        format
    }

    fn for_slots(slots: &[TexAuxEcSlotRecord]) -> (Self, Option<String>) {
        let bc7 = slots.iter().filter(|slot| slot.is_bc7()).count();
        match bc7 {
            0 => (Self::Rgba8, None),
            bc7 if bc7 == slots.len() => (Self::Bc7, None),
            bc7 => (Self::Rgba8, Some(format!(
                "tex_aux_ec.uddp mixes formats ({bc7} of {} textures BC7), so every texture is decoded to RGBA8 for the \
                 GPU; repack it in one format with `udd-cli pack pack-ec-tex-aux` (add --bc7 for BC7)",
                slots.len(),
            ))),
        }
    }

    /// The edge a placement rounds to: one texel, or one BC7 block.
    pub(crate) fn grid(self) -> u32 {
        match self {
            Self::Rgba8 => 1,
            Self::Bc7 => BC7_BLOCK_EDGE,
        }
    }

    /// Both formats are linear: the shaders decode sRGB themselves, as for Enhanced land.
    pub(crate) fn texture_format(self) -> TextureFormat {
        match self {
            Self::Rgba8 => TextureFormat::Rgba8Unorm,
            Self::Bc7 => TextureFormat::Bc7RgbaUnorm,
        }
    }

    fn layer_byte_len(self) -> usize {
        match self {
            Self::Rgba8 => (AUX_ATLAS_TILE_SIZE * AUX_ATLAS_TILE_SIZE) as usize * RGBA_BYTES,
            Self::Bc7 => expected_bc7_byte_len(layer_extent()),
        }
    }
}

fn layer_extent() -> ImageExtent {
    ImageExtent::new(AUX_ATLAS_TILE_SIZE, AUX_ATLAS_TILE_SIZE).expect("non-zero aux layer edge")
}

/// One auxiliary texture as the array stores it.
pub(crate) enum AuxImage {
    Rgba { extent: UVec2, pixels: Vec<u8> },
    Bc7(VramTextureData),
}

impl AuxImage {
    /// The texture of `extent` in the array's `format`; `None` when the package has no payload for it.
    pub(crate) fn load(
        package: &TexAuxEcPackage,
        texture_id: u32,
        extent: UVec2,
        format: AuxLayerFormat,
    ) -> eyre::Result<Option<Self>> {
        let image = match format {
            AuxLayerFormat::Rgba8 => package.load_rgba(texture_id)?.map(|pixels| Self::Rgba { extent, pixels }),
            AuxLayerFormat::Bc7 => package.load_bc7(texture_id)?.map(Self::Bc7),
        };
        if let Some(Self::Rgba { pixels, .. }) = &image {
            eyre::ensure!(pixels.len() == (extent.x * extent.y) as usize * RGBA_BYTES,
                "RGBA texture {texture_id} has {} bytes, not {}x{}", pixels.len(), extent.x, extent.y);
        }
        if let Some(Self::Bc7(blocks)) = &image {
            eyre::ensure!([blocks.extent().width(), blocks.extent().height()] == extent.to_array(),
                "BC7 texture {texture_id} is {:?}, not {}x{}", blocks.extent(), extent.x, extent.y);
        }
        Ok(image)
    }

    fn format(&self) -> AuxLayerFormat {
        match self {
            Self::Rgba { .. } => AuxLayerFormat::Rgba8,
            Self::Bc7(_) => AuxLayerFormat::Bc7,
        }
    }

    pub(crate) fn extent(&self) -> UVec2 {
        match self {
            Self::Rgba { extent, .. } => *extent,
            Self::Bc7(blocks) => UVec2::new(blocks.extent().width(), blocks.extent().height()),
        }
    }

    /// The `size` texels at `origin` as an image of their own, for a crop that must start on the block grid. BC7 is
    /// decoded and encoded again, so this is for crops whose blocks cannot be copied.
    pub(crate) fn crop(&self, origin: UVec2, size: UVec2) -> eyre::Result<Self> {
        let crop = |pixels: &[u8], width: u32| bc7::extract_rgba8_subrect(pixels, width, origin.x, origin.y, size.x, size.y);
        match self {
            Self::Rgba { extent, pixels } => Ok(Self::Rgba { extent: size, pixels: crop(pixels, extent.x) }),
            Self::Bc7(blocks) => {
                let pixels = bc7::decode_from_vram(blocks, bc7::RawImageFormat::Rgba8888)?;
                let extent = ImageExtent::new(size.x, size.y)?;
                Ok(Self::Bc7(bc7::encode_for_vram(&crop(&pixels, blocks.extent().width()), extent,
                    bc7::RawImageFormat::Rgba8888, bc7::VramTextureEncoding::Bc7(bc7::preferred_bc7_encoder_backend()))?))
            }
        }
    }
}

/// The auxiliary array's layers, each `AUX_ATLAS_TILE_SIZE` square in one [`AuxLayerFormat`].
pub(crate) struct AuxLayers {
    format: AuxLayerFormat,
    layers: Vec<Vec<u8>>,
}

impl AuxLayers {
    /// Layer 0 is the transparent sentinel.
    pub(crate) fn new(format: AuxLayerFormat) -> Self {
        let mut layers = Self { format, layers: Vec::new() };
        // Sentinel layer.
        layers.push_blank();
        layers
    }

    pub(crate) fn format(&self) -> AuxLayerFormat {
        self.format
    }

    pub(crate) fn len(&self) -> usize {
        self.layers.len()
    }

    pub(crate) fn push_blank(&mut self) -> u32 {
        self.layers.push(vec![0; self.format.layer_byte_len()]);
        (self.layers.len() - 1) as u32
    }

    /// A new layer holding `image` in its top-left corner, the rest zero. The image keeps its texel coordinates.
    pub(crate) fn push_image(&mut self, image: &AuxImage) -> u32 {
        self.assert_format(image.format());
        match image {
            AuxImage::Rgba { extent, pixels } => {
                self.layers.push(pad_aux_rgba(pixels, extent.x, extent.y, AUX_ATLAS_TILE_SIZE, AUX_ATLAS_TILE_SIZE));
                (self.layers.len() - 1) as u32
            }
            AuxImage::Bc7(_) => {
                let layer = self.push_blank();
                self.copy(layer, UVec2::ZERO, image, UVec2::ZERO, image.extent());
                layer
            }
        }
    }

    /// Copies `size` texels of `image` from `source` to `target` in `layer`. BC7 copies whole blocks: `source` and
    /// `target` must lie on the block grid, and a `size` off it takes the partial blocks whole.
    pub(crate) fn copy(&mut self, layer: u32, target: UVec2, image: &AuxImage, source: UVec2, size: UVec2) {
        assert!((target + size).max_element() <= AUX_ATLAS_TILE_SIZE, "copy of {size} at {target} leaves the layer");
        self.assert_format(image.format());
        let destination = &mut self.layers[layer as usize];
        match image {
            AuxImage::Rgba { extent, pixels } => {
                let row_bytes = size.x as usize * RGBA_BYTES;
                for line in 0..size.y {
                    let from = (((source.y + line) * extent.x + source.x) as usize) * RGBA_BYTES;
                    let to = (((target.y + line) * AUX_ATLAS_TILE_SIZE + target.x) as usize) * RGBA_BYTES;
                    destination[to..to + row_bytes].copy_from_slice(&pixels[from..from + row_bytes]);
                }
            }
            AuxImage::Bc7(blocks) => {
                assert!(source % BC7_BLOCK_EDGE == UVec2::ZERO && target % BC7_BLOCK_EDGE == UVec2::ZERO,
                    "BC7 copy from {source} to {target} is off the block grid");
                let blocks_wide = blocks.extent().blocks_wide();
                let layer_blocks_wide = layer_extent().blocks_wide();
                let (source, target) = (source / BC7_BLOCK_EDGE, target / BC7_BLOCK_EDGE);
                let span = (size + UVec2::splat(BC7_BLOCK_EDGE - 1)) / BC7_BLOCK_EDGE;
                let row_bytes = span.x as usize * BC7_BLOCK_BYTES;
                for row in 0..span.y {
                    let from = ((source.y + row) * blocks_wide + source.x) as usize * BC7_BLOCK_BYTES;
                    let to = ((target.y + row) * layer_blocks_wide + target.x) as usize * BC7_BLOCK_BYTES;
                    destination[to..to + row_bytes].copy_from_slice(&blocks.bytes()[from..from + row_bytes]);
                }
            }
        }
    }

    fn assert_format(&self, format: AuxLayerFormat) {
        assert_eq!(self.format, format, "a {format:?} image in a {:?} array", self.format);
    }

    /// The layers in order, as the array image takes them.
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.layers.concat()
    }

    #[cfg(test)]
    pub(crate) fn layer(&self, layer: u32) -> &[u8] {
        &self.layers[layer as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::bc7::VramTextureFormat;

    /// Distinct non-zero bytes, so a misplaced or missing block shows.
    const BYTE_CYCLE: usize = 251;

    fn bc7_image(width: u32, height: u32) -> AuxImage {
        let extent = ImageExtent::new(width, height).unwrap();
        let bytes: Vec<u8> = (0..expected_bc7_byte_len(extent)).map(|i| (i % BYTE_CYCLE) as u8 + 1).collect();
        AuxImage::Bc7(VramTextureData::new(extent, VramTextureFormat::Bc7RgbaUnormSrgb, bytes.into()).unwrap())
    }

    /// A BC7 texture lands in the top-left of a BC7-sized layer as its own block rows, zero blocks elsewhere, at the
    /// BC7 byte length of the layer; an off-grid width keeps its partial blocks whole.
    #[test]
    fn bc7_layers_hold_the_texture_blocks_top_left() {
        const WIDTH: u32 = 66;
        const HEIGHT: u32 = 32;
        let image = bc7_image(WIDTH, HEIGHT);
        let AuxImage::Bc7(blocks) = &image else { unreachable!() };
        let mut layers = AuxLayers::new(AuxLayerFormat::Bc7);
        let layer = layers.push_image(&image);
        assert_eq!((layer, layers.len()), (1, 2));
        assert_eq!(layers.layer(0), vec![0; expected_bc7_byte_len(layer_extent())]);
        assert_eq!(layers.layer(layer).len(), expected_bc7_byte_len(layer_extent()));
        let row = blocks.extent().blocks_wide() as usize * BC7_BLOCK_BYTES;
        let layer_row = layer_extent().blocks_wide() as usize * BC7_BLOCK_BYTES;
        for (index, bytes) in layers.layer(layer).chunks_exact(layer_row).enumerate() {
            let source = blocks.bytes().chunks_exact(row).nth(index);
            assert_eq!(&bytes[..row], source.unwrap_or(&vec![0; row][..]), "row {index}");
            assert!(bytes[row..].iter().all(|&byte| byte == 0), "row {index}");
        }
        let total = layers.len() * expected_bc7_byte_len(layer_extent());
        assert_eq!(layers.into_bytes().len(), total);
    }

    /// Only a package of BC7 textures alone keeps them as blocks; a mixed one is decoded with a warning naming it.
    #[test]
    fn mixed_packages_decode_with_one_warning() {
        const BC7: u8 = udd_assets::tex_art_cc::PagePixelFormat::Bc7 as u8;
        let rgba = TexAuxEcSlotRecord::default();
        let bc7 = TexAuxEcSlotRecord { pixel_format: BC7, ..rgba };
        assert_eq!(AuxLayerFormat::for_slots(&[bc7, bc7]), (AuxLayerFormat::Bc7, None));
        assert_eq!(AuxLayerFormat::for_slots(&[rgba, rgba]), (AuxLayerFormat::Rgba8, None));
        assert_eq!(AuxLayerFormat::for_slots(&[]), (AuxLayerFormat::Rgba8, None));
        let (format, warning) = AuxLayerFormat::for_slots(&[rgba, bc7]);
        assert_eq!(format, AuxLayerFormat::Rgba8);
        assert!(warning.is_some_and(|warning| warning.contains("tex_aux_ec.uddp") && warning.contains("--bc7")));
    }
}
