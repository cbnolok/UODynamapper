use std::fs;
use std::path::Path;

use clap::ValueEnum;
use color_eyre::eyre::{self, WrapErr};
use serde::Deserialize;
use udd_conv::upscale::{EnhancementFilter, UpscalePass};
use udd_conv::upscale_profile::{
    UpscaleImageType, UpscaleProfile, UpscaleProfileOverride,
};

use crate::pack_cli::CliUpscaleFilter;

// Profiles are hand-written: unknown keys are errors, and every pass list must be spelled out, because an explicit
// empty `passes` disables the command-line fallback.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlUpscaleProfile {
    art_land: Option<TomlPassSection>,
    art_items: Option<TomlPassSection>,
    gumps_equip: Option<TomlPassSection>,
    gumps_non_equip: Option<TomlPassSection>,
    cc_mobile_animation_frames: Option<TomlPassSection>,
    ec_mobile_animation_frames: Option<TomlPassSection>,
    cc_land_textures: Option<TomlPassSection>,
    ec_land_textures: Option<TomlPassSection>,
    /// Optional: a profile may have no overrides.
    #[serde(default)]
    overrides: Vec<TomlOverride>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlPassSection {
    passes: Vec<PassConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlOverride {
    image_type: String,
    family: Option<u32>,
    id: u32,
    passes: Vec<PassConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PassConfig {
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

const PASS_FILTER_KEY: &str = "filter";
/// The tunable pass parameters, as spelled in TOML keys and KDL properties.
const PASS_PARAMETERS: [&str; 11] = [
    "factor",
    "radius",
    "amount",
    "intensity",
    "threshold",
    "blur_spread",
    "gamma",
    "strength",
    "deblur_offset",
    "deblur_strength",
    "smart_deblur",
];

pub fn load_upscale_profile(path: &Path) -> eyre::Result<UpscaleProfile> {
    let content = fs::read_to_string(path)
        .wrap_err_with(|| format!("read upscale profile {}", path.display()))?;
    let profile = match path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("toml") => parse_toml_profile(path, &content),
        Some("kdl") => parse_kdl_profile(path, &content),
        _ => eyre::bail!(
            "unsupported upscale profile extension for {}; expected .toml or .kdl",
            path.display()
        ),
    };
    profile.map_err(|error| uot_kdl::strict::in_file(&path.display().to_string(), error))
}

fn parse_toml_profile(_path: &Path, content: &str) -> eyre::Result<UpscaleProfile> {
    let config = toml::from_str::<TomlUpscaleProfile>(content).map_err(|error| eyre::eyre!("{error}"))?;
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
            let passes = parse_passes(&section.passes)
                .wrap_err_with(|| format!("[{}] passes", image_type.as_str()))
                .map_err(flatten)?;
            profile.set_default(image_type, passes);
        }
    }
    for (index, entry) in config.overrides.into_iter().enumerate() {
        let location = format!("[[overrides]] entry {}", index + 1);
        let image_type = parse_image_type(&entry.image_type)
            .wrap_err_with(|| format!("{location}: image_type"))
            .map_err(flatten)?;
        let passes = parse_passes(&entry.passes).wrap_err_with(|| format!("{location}: passes")).map_err(flatten)?;
        profile.add_override(UpscaleProfileOverride { image_type, family: entry.family, id: entry.id, passes });
    }
    Ok(profile)
}

/// `context: cause` as one message, so the whole text shows wherever the error is printed with `{}`.
fn flatten(error: eyre::Report) -> eyre::Report {
    let parts: Vec<String> = error.chain().map(ToString::to_string).collect();
    eyre::eyre!("{}", parts.join(": "))
}

fn parse_image_type(name: &str) -> eyre::Result<UpscaleImageType> {
    UpscaleImageType::parse(name).ok_or_else(|| {
        let names: Vec<_> = UpscaleImageType::ALL.iter().map(|image_type| image_type.as_str()).collect();
        eyre::eyre!("unknown upscale image type '{name}': expected one of {}", names.join(", "))
    })
}

/// Image type nodes hold `pass` children; `override` nodes select one target.
const KDL_PASS_NODE: &str = "pass";
const KDL_OVERRIDE_NODE: &str = "override";
const KDL_OVERRIDE_PROPERTIES: [&str; 3] = ["type", "family", "id"];

fn parse_kdl_profile(path: &Path, content: &str) -> eyre::Result<UpscaleProfile> {
    use uot_kdl::ast::Node;
    use uot_kdl::span::LineSpan;
    use uot_kdl::strict::{check_properties, error_at, property, required_property, value_f32, value_string, value_u32};

    fn pass_properties() -> Vec<&'static str> {
        std::iter::once(PASS_FILTER_KEY).chain(PASS_PARAMETERS).collect()
    }

    fn parse_pass_node(node: &Node<LineSpan>) -> eyre::Result<UpscalePass> {
        check_properties(node, &pass_properties())?;
        uot_kdl::strict::reject_children(node)?;
        let filter = match (property(node, PASS_FILTER_KEY), node.arguments.as_slice()) {
            (Some(filter), []) => value_string(filter, PASS_FILTER_KEY)?,
            (None, [filter]) => value_string(filter, "pass filter")?,
            (Some(_), [extra, ..]) => {
                return Err(error_at(extra.literal.span(), "`pass`: give the filter as an argument or filter=, not both"));
            }
            (None, []) => return Err(error_at(node.node_name.span(), "`pass`: missing the filter name")),
            (None, [_, extra, ..]) => {
                return Err(error_at(extra.literal.span(), format!("`pass` takes 1 argument, found {}", node.arguments.len())));
            }
        };
        let parameter = |name: &str| property(node, name).map(|value| value_f32(value, name)).transpose();
        let config = PassConfig {
            filter,
            factor: parameter("factor")?,
            radius: parameter("radius")?,
            amount: parameter("amount")?,
            intensity: parameter("intensity")?,
            threshold: parameter("threshold")?,
            blur_spread: parameter("blur_spread")?,
            gamma: parameter("gamma")?,
            strength: parameter("strength")?,
            deblur_offset: parameter("deblur_offset")?,
            deblur_strength: parameter("deblur_strength")?,
            smart_deblur: parameter("smart_deblur")?,
        };
        parse_pass(&config).map_err(|error| error_at(node.node_name.span(), error))
    }

    fn pass_nodes(parent: &Node<LineSpan>) -> eyre::Result<Vec<UpscalePass>> {
        parent
            .children()
            .map(|node| match &**node.node_name {
                KDL_PASS_NODE => parse_pass_node(node),
                _ => Err(uot_kdl::strict::unknown_node(node, Some(&**parent.node_name), &[KDL_PASS_NODE])),
            })
            .collect()
    }

    let document = uot_kdl::parse_ast_line_span(&path.display().to_string(), content)
        .map_err(|error| uot_kdl::strict::describe_error(content, &error))?;
    let mut profile = UpscaleProfile::new();
    for node in &document.nodes {
        let name: &str = &node.node_name;
        if name == KDL_OVERRIDE_NODE {
            check_properties(node, &KDL_OVERRIDE_PROPERTIES)?;
            uot_kdl::strict::arguments::<0>(node)?;
            let image_type_value = required_property(node, "type")?;
            let image_type = parse_image_type(&value_string(image_type_value, "type")?)
                .map_err(|error| error_at(image_type_value.literal.span(), error))?;
            let id = value_u32(required_property(node, "id")?, "id")?;
            let family = property(node, "family").map(|value| value_u32(value, "family")).transpose()?;
            profile.add_override(UpscaleProfileOverride { image_type, family, id, passes: pass_nodes(node)? });
            continue;
        }

        let Some(image_type) = UpscaleImageType::parse(name) else {
            let allowed: Vec<_> = UpscaleImageType::ALL
                .iter()
                .map(|image_type| image_type.as_str())
                .chain([KDL_OVERRIDE_NODE])
                .collect();
            return Err(uot_kdl::strict::unknown_node(node, None, &allowed));
        };
        check_properties(node, &[])?;
        uot_kdl::strict::arguments::<0>(node)?;
        profile.set_default(image_type, pass_nodes(node)?);
    }
    Ok(profile)
}

fn parse_passes(passes: &[PassConfig]) -> eyre::Result<Vec<UpscalePass>> {
    passes
        .iter()
        .enumerate()
        .map(|(index, pass)| parse_pass(pass).wrap_err_with(|| format!("pass {}", index + 1)).map_err(flatten))
        .collect()
}

fn parse_pass(pass: &PassConfig) -> eyre::Result<UpscalePass> {
    let cli_filter = CliUpscaleFilter::from_str(&pass.filter, true)
        .map_err(|error| eyre::eyre!("invalid upscale filter '{}': {error}", pass.filter))?;
    pass.check_parameters(cli_filter)?;
    Ok(pass.pass_for_filter(cli_filter))
}

/// The parameters each tunable filter reads; other filters take none.
fn supported_parameters(filter: CliUpscaleFilter) -> &'static [&'static str] {
    match filter {
        CliUpscaleFilter::Vibrance
        | CliUpscaleFilter::Saturation
        | CliUpscaleFilter::SelectiveWarm
        | CliUpscaleFilter::SelectiveGreen => &["factor"],
        CliUpscaleFilter::LocalLaplacianClarity | CliUpscaleFilter::UnsharpMask => &["radius", "amount"],
        CliUpscaleFilter::ContrastEnhance => &["intensity", "threshold", "blur_spread"],
        CliUpscaleFilter::AdaptiveLogContrast => &["radius", "gamma"],
        CliUpscaleFilter::HighPassSharpen => &["radius", "strength"],
        CliUpscaleFilter::ScaleFxSmartDeblur => &["deblur_offset", "deblur_strength", "smart_deblur"],
        _ => &[],
    }
}

impl PassConfig {
    /// The names of the parameters this pass sets.
    fn set_parameters(&self) -> impl Iterator<Item = &'static str> {
        let values = [
            self.factor,
            self.radius,
            self.amount,
            self.intensity,
            self.threshold,
            self.blur_spread,
            self.gamma,
            self.strength,
            self.deblur_offset,
            self.deblur_strength,
            self.smart_deblur,
        ];
        PASS_PARAMETERS.into_iter().zip(values).filter_map(|(name, value)| value.map(|_| name))
    }

    /// Rejects parameters the filter does not read, which would otherwise be ignored.
    fn check_parameters(&self, filter: CliUpscaleFilter) -> eyre::Result<()> {
        let supported = supported_parameters(filter);
        match self.set_parameters().find(|name| !supported.contains(name)) {
            None => Ok(()),
            Some(name) if supported.is_empty() => {
                eyre::bail!("upscale filter '{}' takes no parameters, found `{name}`", self.filter)
            }
            Some(name) => eyre::bail!(
                "upscale filter '{}' does not take `{name}`: expected {}",
                self.filter,
                supported.join(", ")
            ),
        }
    }

    fn pass_for_filter(&self, filter: CliUpscaleFilter) -> UpscalePass {
        match filter {
            CliUpscaleFilter::Vibrance => {
                UpscalePass::from(EnhancementFilter::Vibrance { factor: self.factor.unwrap_or(0.30) })
            }
            CliUpscaleFilter::Saturation => {
                UpscalePass::from(EnhancementFilter::Saturation { factor: self.factor.unwrap_or(1.25) })
            }
            CliUpscaleFilter::SelectiveWarm => {
                UpscalePass::from(EnhancementFilter::SelectiveWarm { factor: self.factor.unwrap_or(0.30) })
            }
            CliUpscaleFilter::SelectiveGreen => {
                UpscalePass::from(EnhancementFilter::SelectiveGreen { factor: self.factor.unwrap_or(0.30) })
            }
            CliUpscaleFilter::LocalLaplacianClarity => {
                UpscalePass::from(EnhancementFilter::LocalLaplacianClarity {
                    radius: self.radius.map(f32_to_radius).unwrap_or(3),
                    amount: self.amount.unwrap_or(0.25),
                })
            }
            CliUpscaleFilter::ContrastEnhance => {
                UpscalePass::from(EnhancementFilter::ContrastEnhance {
                    intensity: self.intensity.unwrap_or(0.35),
                    threshold: self.threshold.unwrap_or(0.08),
                    blur_spread: self.blur_spread.unwrap_or(2.5),
                })
            }
            CliUpscaleFilter::AdaptiveLogContrast => {
                UpscalePass::from(EnhancementFilter::AdaptiveLogContrast {
                    radius: self.radius.unwrap_or(3.0),
                    gamma: self.gamma.unwrap_or(0.80),
                })
            }
            CliUpscaleFilter::UnsharpMask => {
                UpscalePass::from(EnhancementFilter::UnsharpMask {
                    radius: self.radius.unwrap_or(1.0),
                    amount: self.amount.unwrap_or(0.35),
                })
            }
            CliUpscaleFilter::HighPassSharpen => {
                UpscalePass::from(EnhancementFilter::HighPassSharpen {
                    radius: self.radius.unwrap_or(2.0),
                    strength: self.strength.unwrap_or(0.18),
                })
            }
            CliUpscaleFilter::ScaleFxSmartDeblur => {
                UpscalePass::from(EnhancementFilter::ScaleFxSmartDeblur {
                    deblur_offset: self.deblur_offset.unwrap_or(0.6),
                    deblur_strength: self.deblur_strength.unwrap_or(0.55),
                    smart_deblur: self.smart_deblur.unwrap_or(0.4),
                })
            }
            CliUpscaleFilter::GuestrDeblur => UpscalePass::from(EnhancementFilter::GuestrDeblur),
            _ => filter.into_pass(),
        }
    }
}

fn f32_to_radius(value: f32) -> u32 {
    value.round().max(1.0) as u32
}

#[cfg(test)]
mod tests {
    use super::{parse_kdl_profile, parse_toml_profile};
    use std::path::Path;
    use udd_conv::upscale::{EnhancementFilter, FilterUpscalePass, UpscaleFilter, UpscalePass};
    use udd_conv::upscale_profile::{UpscaleImageType, UpscaleTarget};

    #[test]
    fn toml_profile_parses_defaults_and_overrides() {
        let profile = parse_toml_profile(
            Path::new("profile.toml"),
            r#"
[art_items]
passes = [{ filter = "vibrance", factor = 0.35 }]

[[overrides]]
image_type = "art_items"
id = 4000
passes = [{ filter = "nearest2x" }]
"#,
        )
        .expect("parse profile");

        let passes = profile.passes_for(UpscaleTarget::new(UpscaleImageType::ArtItems, 1), &[]);
        assert!(matches!(
            passes[0],
            UpscalePass::Filter(FilterUpscalePass::Enhancement(
                EnhancementFilter::Vibrance { factor: 0.35 }
            ))
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
    pass "contrast-enhance" intensity=0.4 threshold=0.08
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
            UpscalePass::Filter(FilterUpscalePass::Enhancement(
                EnhancementFilter::ContrastEnhance {
                    intensity: 0.4,
                    threshold: 0.08,
                    blur_spread: 2.5,
                }
            ))
        ));
        assert_eq!(
            profile.passes_for(
                UpscaleTarget::with_family(UpscaleImageType::CcMobileAnimationFrames, 42, 3),
                &[],
            )[0].filter(),
            Some(UpscaleFilter::Nearest2x)
        );
    }

    fn toml_error(content: &str) -> String {
        parse_toml_profile(Path::new("profile.toml"), content).expect_err("profile error").to_string()
    }

    fn kdl_error(content: &str) -> String {
        parse_kdl_profile(Path::new("profile.kdl"), content).expect_err("profile error").to_string()
    }

    #[test]
    fn toml_profile_rejects_unknown_and_missing_keys() {
        let unknown = toml_error("[art_items]\npasses = []\n\n[art_itemz]\npasses = []\n");
        assert!(unknown.contains("line 4, column 2"), "{unknown}");
        assert!(unknown.contains("unknown field `art_itemz`, expected one of `art_land`, `art_items`"), "{unknown}");

        let unknown_param = toml_error("[art_items]\npasses = [{ filter = \"vibrance\", factr = 0.3 }]\n");
        assert!(unknown_param.contains("line 2, column 34"), "{unknown_param}");
        assert!(unknown_param.contains("unknown field `factr`"), "{unknown_param}");

        let missing = toml_error("[art_items]\n");
        assert!(missing.contains("missing field `passes`"), "{missing}");
    }

    #[test]
    fn profiles_reject_parameters_the_filter_does_not_read() {
        assert_eq!(
            toml_error("[art_items]\npasses = [{ filter = \"nearest2x\" }, { filter = \"vibrance\", radius = 2 }]\n"),
            "[art_items] passes: pass 2: upscale filter 'vibrance' does not take `radius`: expected factor"
        );
        assert_eq!(
            kdl_error("art_items {\n    pass \"xbr2x\" factor=0.3\n}\n"),
            "line 2, column 5: upscale filter 'xbr2x' takes no parameters, found `factor`"
        );
        assert_eq!(
            toml_error("[[overrides]]\nimage_type = \"art_item\"\nid = 1\npasses = []\n"),
            "[[overrides]] entry 1: image_type: unknown upscale image type 'art_item': expected one of art_land, \
             art_items, gumps_equip, gumps_non_equip, cc_mobile_animation_frames, ec_mobile_animation_frames, \
             cc_land_textures, ec_land_textures"
        );
    }

    #[test]
    fn kdl_profile_rejects_unknown_nodes_and_properties_with_their_position() {
        assert_eq!(
            kdl_error("art_items {\n    pass \"nearest2x\"\n}\nart_itemz {\n}\n"),
            "line 4, column 1: unknown node `art_itemz` at the top level: expected one of `art_land`, `art_items`, \
             `gumps_equip`, `gumps_non_equip`, `cc_mobile_animation_frames`, `ec_mobile_animation_frames`, \
             `cc_land_textures`, `ec_land_textures`, `override`"
        );
        assert_eq!(
            kdl_error("art_items {\n    step \"nearest2x\"\n}\n"),
            "line 2, column 5: unknown node `step` in `art_items`: expected `pass`"
        );
        assert_eq!(
            kdl_error("art_items {\n    pass \"vibrance\" factr=0.3\n}\n"),
            "line 2, column 21: unknown property `factr` of `pass`: expected one of `filter`, `factor`, `radius`, \
             `amount`, `intensity`, `threshold`, `blur_spread`, `gamma`, `strength`, `deblur_offset`, \
             `deblur_strength`, `smart_deblur`"
        );
        assert_eq!(
            kdl_error("override type=\"art_items\" ids=4000 {\n}\n"),
            "line 1, column 27: unknown property `ids` of `override`: expected one of `type`, `family`, `id`"
        );
        assert_eq!(
            kdl_error("override type=\"art_items\" {\n}\n"),
            "line 1, column 1: `override`: missing id="
        );
        assert_eq!(kdl_error("art_items {\n    pass\n}\n"), "line 2, column 5: `pass`: missing the filter name");
    }
}
