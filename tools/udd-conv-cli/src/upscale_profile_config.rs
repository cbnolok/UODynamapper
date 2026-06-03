use std::fs;
use std::path::Path;

use clap::ValueEnum;
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use serde::Deserialize;
use udd_conv::upscale::UpscalePass;
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

    fn parse_pass_node(node: &Node<Span>) -> eyre::Result<PassConfig> {
        let filter = if let Some(value) = property_string(node, "filter")?
            .or(property_string(node, "algorithm")?)
        {
            value
        } else {
            let value = node.arguments.first().context("pass: missing filter argument")?;
            value_string(value, "pass filter")?
        };
        Ok(PassConfig { filter })
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
    Ok(cli_filter.into_pass())
}

#[cfg(test)]
mod tests {
    use super::{parse_kdl_profile, parse_toml_profile};
    use std::path::Path;
    use udd_conv::upscale::UpscaleFilter;
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

        assert_eq!(
            profile.passes_for(UpscaleTarget::new(UpscaleImageType::ArtItems, 1), &[])[0].filter(),
            Some(UpscaleFilter::Vibrance30)
        );
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

        assert_eq!(
            profile.passes_for(
                UpscaleTarget::with_family(UpscaleImageType::CcMobileAnimationFrames, 42, 3),
                &[],
            )[0].filter(),
            Some(UpscaleFilter::Nearest2x)
        );
    }
}
