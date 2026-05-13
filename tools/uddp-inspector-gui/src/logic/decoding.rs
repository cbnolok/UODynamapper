use crate::models::{AtlasPageInfo, AtlasPixelFormat, DecodedAtlasPage};

pub fn decode_atlas_page(page_info: AtlasPageInfo, data: Vec<u8>) -> Option<DecodedAtlasPage> {
    match page_info.pixel_format {
        AtlasPixelFormat::Bc7 => {
            let extent =
                udd_assets::bc7::ImageExtent::new(page_info.atlas_width, page_info.atlas_height)
                    .ok()?;
            let rgba = udd_assets::bc7::decode_bc7_to_rgba8888(&data, extent).ok()?;
            Some(DecodedAtlasPage {
                rgba,
                width: page_info.atlas_width,
                height: page_info.atlas_height,
            })
        }
        AtlasPixelFormat::Rgba8888 => {
            let used_len = page_info.used_width as usize * page_info.used_height as usize * 4;
            let atlas_len = page_info.atlas_width as usize * page_info.atlas_height as usize * 4;
            if data.len() == used_len {
                Some(DecodedAtlasPage {
                    rgba: data,
                    width: page_info.used_width,
                    height: page_info.used_height,
                })
            } else if data.len() == atlas_len {
                Some(DecodedAtlasPage {
                    rgba: data,
                    width: page_info.atlas_width,
                    height: page_info.atlas_height,
                })
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_atlas_page_uses_manifest_used_size_for_rgba() {
        let info = AtlasPageInfo {
            atlas_width: 4096,
            atlas_height: 2048,
            used_width: 3,
            used_height: 2,
            pixel_format: AtlasPixelFormat::Rgba8888,
        };
        let data = vec![255u8; 3 * 2 * 4];
        let decoded = decode_atlas_page(info, data).unwrap();

        assert_eq!(decoded.width, 3);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.rgba.len(), 24);
    }
}
