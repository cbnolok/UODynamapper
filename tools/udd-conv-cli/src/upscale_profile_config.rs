use std::fs;
use std::path::Path;

use clap::ValueEnum;
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use serde::Deserialize;
use udd_conv::upscale::{UpscaleFilter, UpscalePass, UpscalePassParams};
use udd_conv::upscale_profile::{
    UpscaleImageType, UpscaleProfile, UpscaleProfileOverride,
};

use crate::pack_cli::CliUpscaleFilter;

#[derive(Debug, Deserialize, Default)]
struct TomlUpscaleProfile {
    #[serde(default)]
    art_land: Option<TomlPassSection>,
    #[serde(default)]
    art_items: Option<TomlPassSection>,
    #[serde(default)]
    gumps_equip: Option<TomlPassSection>,
    #[serde(default)]
    gumps_non_equip: Option<TomlPassSection>,
    #[serde(default)]
    cc_mobile_animation_frames: Option<TomlPassSection>,
    #[serde(default)]
    ec_mobile_animation_frames: Option<TomlPassSection>,
    #[serde(default)]
    cc_land_textures: Option<TomlPassSection>,
    #[serde(default)]
    ec_land_textures: Option<TomlPassSection>,
    #[serde(default)]
    overrides: Vec<TomlOverride>,
}

#[derive(Debug, Deserialize, Default)]
struct TomlPassSection {
    #[serde(default)]
    passes: Vec<PassConfig>,
}

#[derive(Debug, Deserialize)]
struct TomlOverride {
    #[serde(alias = "type")]
    image_type: String,
    family: Option<u32>,
    id: u32,
    #[serde(default)]
    passes: Vec<PassConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct PassConfig {
    #[serde(alias = "algorithm")]
    filter: String,
    factor: Option<f32>,
    radius: Option<f32>,
    amount: Option<f32>,
    intensity: Option<f32>,
    threshold: Option<f32>,
    blur_spread: Option<f32>,
    gamma: Option<f32>,
    strength: Option<f32>,
    deblur_offset: Option<f32>,
    deblur_strength: Option<f32>,
    smart_deblur: Option<f32>,
}

pub fn load_upscale_profile(path: &Path) -> eyre::Result<UpscaleProfile> {
    let content = fs::read_to_string(path)
        .wrap_err_with(|| format!("read upscale profile {}", path.display()))?;
    match path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("toml") => parse_toml_profile(path, &content),
        Some("kdl") => parse_kdl_profile(path, &content),
        _ => eyre::bail!(
            "unsupported upscale profile extension for {}; expected .toml or .kdl",
            path.display()
        ),
    }
}

fn parse_toml_profile(path: &Path, content: &str) -> eyre::Result<UpscaleProfile> {
    let config = toml::from_str::<TomlUpscaleProfile>(content)
        .wrap_err_with(|| format!("parse TOML upscale profile {}", path.display()))?;
    let mut profile = UpscaleProfile::new();
    for (image_type, section) in [
        (UpscaleImageType::ArtLand, config.art_land),
        (UpscaleImageType::ArtItems, config.art_items),
        (UpscaleImageType::GumpsEquip, config.gumps_equip),
        (UpscaleImageType::GumpsNonEquip, config.gumps_non_equip),
        (UpscaleImageType::CcMobileAnimationFrames, config.cc_mobile_animation_frames),
        (UpscaleImageType::EcMobileAnimationFrames, config.ec_mobile_animation_frames),
        (UpscaleImageType::CcLandTextures, config.cc_land_textures),
        (UpscaleImageType::EcLandTextures, config.ec_land_textures),
    ] {
        if let Some(section) = section {
            profile.set_default(image_type, parse_passes(&section.passes)?);
        }
    }
    for entry in config.overrides {
        let image_type = UpscaleImageType::parse(&entry.image_type)
            .ok_or_else(|| eyre::eyre!("unknown upscale image type '{}'", entry.image_type))?;
        profile.add_override(UpscaleProfileOverride {
            image_type,
            family: entry.family,
            id: entry.id,
            passes: parse_passes(&entry.passes)?,
        });
    }
    Ok(profile)
}

fn parse_kdl_profile(path: &Path, content: &str) -> eyre::Result<UpscaleProfile> {
    use knuffel::ast::{Literal, Node, Value};
    use knuffel::span::{Span, Spanned};

    fn value_string(value: &Value<Span>, field: &str) -> eyre::Result<String> {
        match &*value.literal {
            Literal::String(value) => Ok(value.to_string()),
            _ => eyre::bail!("{field}: expected string"),
        }
    }

    fn value_u32(value: &Value<Span>, field: &str) -> eyre::Result<u32> {
        match &*value.literal {
            Literal::Int(value) => u32::try_from(value)
                .wrap_err_with(|| format!("{field}: invalid unsigned integer")),
            _ => eyre::bail!("{field}: expected integer"),
        }
    }

    fn value_f32(value: &Value<Span>, field: &str) -> eyre::Result<f32> {
        match &*value.literal {
            Literal::Int(value) => {
                let value = i32::try_from(value)
                    .wrap_err_with(|| format!("{field}: invalid integer number"))?;
                Ok(value as f32)
            }
            Literal::Decimal(value) => {
                f32::try_from(value).wrap_err_with(|| format!("{field}: invalid number"))
            }
            _ => eyre::bail!("{field}: expected number"),
        }
    }

    fn property<'a>(node: &'a Node<Span>, name: &str) -> Option<&'a Value<Span>> {
        node.properties
            .iter()
            .find(|(key, _)| key.as_ref() == name)
            .map(|(_, value)| value)
    }

    fn property_string(node: &Node<Span>, name: &str) -> eyre::Result<Option<String>> {
        property(node, name).map(|value| value_string(value, name)).transpose()
    }

    fn property_u32(node: &Node<Span>, name: &str) -> eyre::Result<Option<u32>> {
        property(node, name).map(|value| value_u32(value, name)).transpose()
    }

    fn property_f32(node: &Node<Span>, name: &str) -> eyre::Result<Option<f32>> {
        property(node, name).map(|value| value_f32(value, name)).transpose()
    }

    fn parse_pass_node(node: &Node<Span>) -> eyre::Result<PassConfig> {
        let filter = if let Some(value) = property_string(node, "filter")?
            .or(property_string(node, "algorithm")?)
        {
            value
        } else {
            let value = node.arguments.first().context("pass: missing filter argument")?;
            value_string(value, "pass filter")?
        };
        Ok(PassConfig {
            filter,
            factor: property_f32(node, "factor")?,
            radius: property_f32(node, "radius")?,
            amount: property_f32(node, "amount")?,
            intensity: property_f32(node, "intensity")?,
            threshold: property_f32(node, "threshold")?,
            blur_spread: property_f32(node, "blur_spread")?,
            gamma: property_f32(node, "gamma")?,
            strength: property_f32(node, "strength")?,
            deblur_offset: property_f32(node, "deblur_offset")?,
            deblur_strength: property_f32(node, "deblur_strength")?,
            smart_deblur: property_f32(node, "smart_deblur")?,
        })
    }

    fn pass_nodes<'a, I>(nodes: I) -> eyre::Result<Vec<PassConfig>>
    where
        I: IntoIterator<Item = &'a Spanned<Node<Span>, Span>>,
    {
        nodes
            .into_iter()
            .filter(|node| node.node_name.as_ref() == "pass")
            .map(|node| parse_pass_node(node))
            .collect()
    }

    let document = knuffel::parse_ast::<Span>(&path.display().to_string(), content)
        .wrap_err_with(|| format!("parse KDL upscale profile {}", path.display()))?;
    let mut profile = UpscaleProfile::new();
    for node in &document.nodes {
        if node.node_name.as_ref() == "override" {
            let image_type = property_string(node, "type")?
                .or(property_string(node, "image_type")?)
                .context("override: missing type property")?;
            let image_type = UpscaleImageType::parse(&image_type)
                .ok_or_else(|| eyre::eyre!("unknown upscale image type '{image_type}'"))?;
            let id = property_u32(node, "id")?.context("override: missing id property")?;
            profile.add_override(UpscaleProfileOverride {
                image_type,
                family: property_u32(node, "family")?,
                id,
                passes: parse_passes(&pass_nodes(node.children())?)?,
            });
            continue;
        }

        if let Some(image_type) = UpscaleImageType::parse(node.node_name.as_ref()) {
            profile.set_default(image_type, parse_passes(&pass_nodes(node.children())?)?);
        }
    }
    Ok(profile)
}

fn parse_passes(passes: &[PassConfig]) -> eyre::Result<Vec<UpscalePass>> {
    passes.iter().map(parse_pass).collect()
}

fn parse_pass(pass: &PassConfig) -> eyre::Result<UpscalePass> {
    let cli_filter = CliUpscaleFilter::from_str(&pass.filter, true)
        .map_err(|error| eyre::eyre!("invalid upscale filter '{}': {error}", pass.filter))?;
    let filter = UpscaleFilter::from(cli_filter);
    if !pass.has_custom_params() {
        return Ok(cli_filter.into_pass());
    }
    let params = pass.params_for_filter(filter)?;
    Ok(UpscalePass::parameterized_filter(filter, params))
}

impl PassConfig {
    fn has_custom_params(&self) -> bool {
        self.factor.is_some()
            || self.radius.is_some()
            || self.amount.is_some()
            || self.intensity.is_some()
            || self.threshold.is_some()
            || self.blur_spread.is_some()
            || self.gamma.is_some()
            || self.strength.is_some()
            || self.deblur_offset.is_some()
            || self.deblur_strength.is_some()
            || self.smart_deblur.is_some()
    }

    fn params_for_filter(&self, filter: UpscaleFilter) -> eyre::Result<UpscalePassParams> {
        match filter {
            UpscaleFilter::Vibrance20
            | UpscaleFilter::Vibrance30
            | UpscaleFilter::Vibrance40
            | UpscaleFilter::Saturation115
            | UpscaleFilter::Saturation125
            | UpscaleFilter::Saturation130
            | UpscaleFilter::SelectiveWarm20
            | UpscaleFilter::SelectiveWarm30
            | UpscaleFilter::SelectiveWarm40
            | UpscaleFilter::SelectiveGreen20
            | UpscaleFilter::SelectiveGreen30
            | UpscaleFilter::SelectiveGreen40 => Ok(UpscalePassParams::ColorFactor {
                factor: self.factor.unwrap_or_else(|| default_color_factor(filter)),
            }),
            UpscaleFilter::LocalLaplacianClarity15
            | UpscaleFilter::LocalLaplacianClarity25
            | UpscaleFilter::LocalLaplacianClarity30 => {
                let (radius, amount) = default_local_laplacian(filter);
                Ok(UpscalePassParams::LocalLaplacianClarity {
                    radius: self.radius.map(f32_to_radius).unwrap_or(radius),
                    amount: self.amount.unwrap_or(amount),
                })
            }
            UpscaleFilter::UnityContrastEnhance20
            | UpscaleFilter::UnityContrastEnhance35
            | UpscaleFilter::UnityContrastEnhance50 => {
                let (intensity, threshold, blur_spread) = default_contrast_enhance(filter);
                Ok(UpscalePassParams::ContrastEnhance {
                    intensity: self.intensity.unwrap_or(intensity),
                    threshold: self.threshold.unwrap_or(threshold),
                    blur_spread: self.blur_spread.unwrap_or(blur_spread),
                })
            }
            UpscaleFilter::AdaptiveLogContrast75
            | UpscaleFilter::AdaptiveLogContrast80
            | UpscaleFilter::AdaptiveLogContrast90 => {
                let (radius, gamma) = default_adaptive_log(filter);
                Ok(UpscalePassParams::AdaptiveLogContrast {
                    radius: self.radius.unwrap_or(radius),
                    gamma: self.gamma.unwrap_or(gamma),
                })
            }
            UpscaleFilter::UnsharpMaskSmall => Ok(UpscalePassParams::UnsharpMask {
                radius: self.radius.unwrap_or(1.0),
                amount: self.amount.unwrap_or(0.35),
            }),
            UpscaleFilter::HighPassSharpen => Ok(UpscalePassParams::HighPassSharpen {
                radius: self.radius.unwrap_or(2.0),
                strength: self.strength.or(self.amount).unwrap_or(0.18),
            }),
            UpscaleFilter::ScaleFxSmartDeblur => Ok(UpscalePassParams::ScaleFxSmartDeblur {
                deblur_offset: self.deblur_offset.unwrap_or(0.6),
                deblur_strength: self.deblur_strength.unwrap_or(0.55),
                smart_deblur: self.smart_deblur.unwrap_or(0.4),
            }),
            _ => eyre::bail!(
                "upscale filter '{}' does not support custom pass parameters",
                self.filter
            ),
        }
    }
}

fn f32_to_radius(value: f32) -> u32 {
    value.round().max(1.0) as u32
}

fn default_color_factor(filter: UpscaleFilter) -> f32 {
    match filter {
        UpscaleFilter::Vibrance20
        | UpscaleFilter::SelectiveWarm20
        | UpscaleFilter::SelectiveGreen20 => 0.20,
        UpscaleFilter::Vibrance30
        | UpscaleFilter::SelectiveWarm30
        | UpscaleFilter::SelectiveGreen30 => 0.30,
        UpscaleFilter::Vibrance40
        | UpscaleFilter::SelectiveWarm40
        | UpscaleFilter::SelectiveGreen40 => 0.40,
        UpscaleFilter::Saturation115 => 1.15,
        UpscaleFilter::Saturation125 => 1.25,
        UpscaleFilter::Saturation130 => 1.30,
        _ => 1.0,
    }
}

fn default_local_laplacian(filter: UpscaleFilter) -> (u32, f32) {
    match filter {
        UpscaleFilter::LocalLaplacianClarity15 => (2, 0.15),
        UpscaleFilter::LocalLaplacianClarity30 => (4, 0.30),
        _ => (3, 0.25),
    }
}

fn default_contrast_enhance(filter: UpscaleFilter) -> (f32, f32, f32) {
    match filter {
        UpscaleFilter::UnityContrastEnhance20 => (0.20, 0.05, 2.0),
        UpscaleFilter::UnityContrastEnhance50 => (0.50, 0.15, 3.0),
        _ => (0.35, 0.08, 2.5),
    }
}

fn default_adaptive_log(filter: UpscaleFilter) -> (f32, f32) {
    match filter {
        UpscaleFilter::AdaptiveLogContrast75 => (3.0, 0.75),
        UpscaleFilter::AdaptiveLogContrast90 => (3.0, 0.90),
        _ => (3.0, 0.80),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_kdl_profile, parse_toml_profile};
    use std::path::Path;
    use udd_conv::upscale::{UpscaleFilter, UpscalePass, UpscalePassParams};
    use udd_conv::upscale_profile::{UpscaleImageType, UpscaleTarget};

    #[test]
    fn toml_profile_parses_defaults_and_overrides() {
        let profile = parse_toml_profile(
            Path::new("profile.toml"),
            r#"
[art_items]
passes = [{ filter = "vibrance30", factor = 0.35 }]

[[overrides]]
image_type = "art_items"
id = 4000
passes = [{ filter = "nearest2x" }]
"#,
        )
        .expect("parse profile");

        let passes = profile.passes_for(UpscaleTarget::new(UpscaleImageType::ArtItems, 1), &[]);
        assert_eq!(passes[0].filter(), Some(UpscaleFilter::Vibrance30));
        assert!(matches!(
            passes[0],
            UpscalePass::Filter(pass)
                if pass.params == (UpscalePassParams::ColorFactor { factor: 0.35 })
        ));
        assert_eq!(
            profile.passes_for(UpscaleTarget::new(UpscaleImageType::ArtItems, 4000), &[])[0].filter(),
            Some(UpscaleFilter::Nearest2x)
        );
    }

    #[test]
    fn kdl_profile_parses_family_override() {
        let profile = parse_kdl_profile(
            Path::new("profile.kdl"),
            r#"
cc_mobile_animation_frames {
    pass "unity-contrast-enhance35" intensity=0.4 threshold=0.08
}
override type="cc_mobile_animation_frames" family=42 id=3 {
    pass "nearest2x"
}
"#,
        )
        .expect("parse profile");

        let default_passes = profile.passes_for(
            UpscaleTarget::with_family(UpscaleImageType::CcMobileAnimationFrames, 42, 2),
            &[],
        );
        assert!(matches!(
            default_passes[0],
            UpscalePass::Filter(pass)
                if pass.params == (UpscalePassParams::ContrastEnhance {
                    intensity: 0.4,
                    threshold: 0.08,
                    blur_spread: 2.5,
                })
        ));
        assert_eq!(
            profile.passes_for(
                UpscaleTarget::with_family(UpscaleImageType::CcMobileAnimationFrames, 42, 3),
                &[],
            )[0].filter(),
            Some(UpscaleFilter::Nearest2x)
        );
    }
}
