use image_postprocess::palette::{
    palette_safe_upscale, PaletteModel, PaletteUpscaleConfig, RgbaFilterScaler, SnapMode,
    TransparencyPolicy,
};
use image_postprocess::upscaling::{
    UpscaleFilter, UpscalePass as FilterUpscalePass, UpscalePassParams,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpscalePass {
    Filter(FilterUpscalePass),
    PaletteSnapStrict,
    PaletteSnapRampAware,
    PaletteSnapExpanded { max_derived_colors: usize },
}

impl From<UpscaleFilter> for UpscalePass {
    fn from(value: UpscaleFilter) -> Self {
        Self::Filter(FilterUpscalePass::from(value))
    }
}

impl UpscalePass {
    pub fn parameterized_filter(filter: UpscaleFilter, params: UpscalePassParams) -> Self {
        Self::Filter(FilterUpscalePass { filter, params })
    }

    pub fn filter(self) -> Option<UpscaleFilter> {
        match self {
            Self::Filter(pass) => Some(pass.filter),
            Self::PaletteSnapStrict
            | Self::PaletteSnapRampAware
            | Self::PaletteSnapExpanded { .. } => None,
        }
    }

    pub fn scale_factor(self) -> u32 {
        self.filter().map_or(1, UpscaleFilter::scale_factor)
    }

    fn palette_config(self) -> Option<PaletteUpscaleConfig> {
        let mut config = PaletteUpscaleConfig::default();
        config.snap_mode = match self {
            Self::PaletteSnapStrict => SnapMode::StrictSnap,
            Self::PaletteSnapRampAware => SnapMode::RampAwareSnap,
            Self::PaletteSnapExpanded { max_derived_colors } => {
                SnapMode::ExpandedPalette { max_derived_colors }
            }
            Self::Filter(_) => return None,
        };
        Some(config)
    }
}

pub fn apply_upscale_passes(
    width: u32,
    height: u32,
    rgba: &[u8],
    passes: &[UpscalePass],
) -> (u32, u32, Vec<u8>, u32, UpscaleFilter) {
    apply_upscale_passes_owned(width, height, rgba.to_vec(), passes)
}

pub fn apply_upscale_passes_owned(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    passes: &[UpscalePass],
) -> (u32, u32, Vec<u8>, u32, UpscaleFilter) {
    let transparency = TransparencyPolicy::default();
    let source_palette = PaletteModel::from_rgba(width, height, &rgba, &transparency).ok();
    let mut width = width;
    let mut height = height;
    let mut rgba = rgba;
    let mut scale_factor = 1u32;
    let mut last_filter = UpscaleFilter::None;
    let mut index = 0usize;

    while index < passes.len() {
        let pass = passes[index];
        if let Some(config) = pass.palette_config() {
            if let Ok((next_width, next_height, next_rgba)) =
                apply_palette_snap_pass(width, height, &rgba, source_palette.as_ref(), &config, UpscaleFilter::None)
            {
                width = next_width;
                height = next_height;
                rgba = next_rgba;
            }
            index += 1;
            continue;
        }

        let filter = pass.filter().unwrap_or(UpscaleFilter::None);
        if matches!(filter, UpscaleFilter::None) {
            index += 1;
            continue;
        }

        if let Some(next_pass) = passes.get(index + 1).copied() {
            if let Some(config) = next_pass.palette_config() {
                if let Ok((next_width, next_height, next_rgba)) =
                    apply_palette_snap_pass(width, height, &rgba, source_palette.as_ref(), &config, filter)
                {
                    width = next_width;
                    height = next_height;
                    rgba = next_rgba;
                    scale_factor = scale_factor.saturating_mul(filter.scale_factor());
                    last_filter = filter;
                    index += 2;
                    continue;
                }
            }
        }

        let (next_width, next_height, next_rgba) = match pass {
            UpscalePass::Filter(pass) => pass.apply_owned(width, height, rgba),
            UpscalePass::PaletteSnapStrict
            | UpscalePass::PaletteSnapRampAware
            | UpscalePass::PaletteSnapExpanded { .. } => unreachable!(),
        };
        width = next_width;
        height = next_height;
        rgba = next_rgba;
        scale_factor = scale_factor.saturating_mul(filter.scale_factor());
        last_filter = filter;
        index += 1;
    }

    (width, height, rgba, scale_factor, last_filter)
}

fn apply_palette_snap_pass(
    width: u32,
    height: u32,
    rgba: &[u8],
    source_palette: Option<&PaletteModel>,
    config: &PaletteUpscaleConfig,
    filter: UpscaleFilter,
) -> Result<(u32, u32, Vec<u8>), ()> {
    let scaler = RgbaFilterScaler::new(filter);
    palette_safe_upscale(width, height, rgba, source_palette, &scaler, config)
        .map(|result| (result.width, result.height, result.rgba))
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_palette_snap_restricts_bilinear_output_to_source_colors() {
        let rgba = vec![
            255, 0, 0, 255, 0, 0, 255, 255,
            0, 0, 255, 255, 255, 0, 0, 255,
        ];
        let passes = [
            UpscalePass::from(UpscaleFilter::Bilinear2x),
            UpscalePass::PaletteSnapStrict,
        ];

        let (width, height, pixels, scale, last_filter) =
            apply_upscale_passes(2, 2, &rgba, &passes);

        assert_eq!((width, height), (4, 4));
        assert_eq!(scale, 2);
        assert_eq!(last_filter, UpscaleFilter::Bilinear2x);
        for pixel in pixels.chunks_exact(4) {
            assert!(pixel == [255, 0, 0, 255] || pixel == [0, 0, 255, 255]);
        }
    }

    #[test]
    fn standalone_palette_snap_has_unit_scale() {
        let rgba = vec![255, 0, 0, 255, 254, 0, 0, 255];
        let passes = [UpscalePass::PaletteSnapStrict];

        let (width, height, pixels, scale, last_filter) =
            apply_upscale_passes(2, 1, &rgba, &passes);

        assert_eq!((width, height), (2, 1));
        assert_eq!(scale, 1);
        assert_eq!(last_filter, UpscaleFilter::None);
        assert_eq!(pixels, rgba);
    }
}
