use std::collections::HashMap;

use crate::upscale_pipeline::UpscalePass;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpscaleImageType {
    ArtLand,
    ArtItems,
    GumpsEquip,
    GumpsNonEquip,
    CcMobileAnimationFrames,
    EcMobileAnimationFrames,
    CcLandTextures,
    EcLandTextures,
}

impl UpscaleImageType {
    pub const ALL: &'static [Self] = &[
        Self::ArtLand,
        Self::ArtItems,
        Self::GumpsEquip,
        Self::GumpsNonEquip,
        Self::CcMobileAnimationFrames,
        Self::EcMobileAnimationFrames,
        Self::CcLandTextures,
        Self::EcLandTextures,
    ];

    /// The image type named exactly by `value` (see [`Self::as_str`]).
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|image_type| image_type.as_str() == value)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ArtLand => "art_land",
            Self::ArtItems => "art_items",
            Self::GumpsEquip => "gumps_equip",
            Self::GumpsNonEquip => "gumps_non_equip",
            Self::CcMobileAnimationFrames => "cc_mobile_animation_frames",
            Self::EcMobileAnimationFrames => "ec_mobile_animation_frames",
            Self::CcLandTextures => "cc_land_textures",
            Self::EcLandTextures => "ec_land_textures",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UpscaleTarget {
    pub image_type: UpscaleImageType,
    pub family: Option<u32>,
    pub id: u32,
}

impl UpscaleTarget {
    pub fn new(image_type: UpscaleImageType, id: u32) -> Self {
        Self {
            image_type,
            family: None,
            id,
        }
    }

    pub fn with_family(image_type: UpscaleImageType, family: u32, id: u32) -> Self {
        Self {
            image_type,
            family: Some(family),
            id,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct UpscaleProfile {
    defaults: HashMap<UpscaleImageType, Vec<UpscalePass>>,
    overrides: Vec<UpscaleProfileOverride>,
}

impl UpscaleProfile {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_default(&mut self, image_type: UpscaleImageType, passes: Vec<UpscalePass>) {
        self.defaults.insert(image_type, passes);
    }

    pub fn add_override(&mut self, override_entry: UpscaleProfileOverride) {
        self.overrides.push(override_entry);
    }

    pub fn passes_for(&self, target: UpscaleTarget, fallback: &[UpscalePass]) -> Vec<UpscalePass> {
        if let Some(override_entry) = self
            .overrides
            .iter()
            .rev()
            .find(|entry| entry.matches(target))
        {
            return override_entry.passes.clone();
        }

        if let Some(passes) = self.defaults.get(&target.image_type) {
            return passes.clone();
        }

        fallback.to_vec()
    }
}

#[derive(Debug, Clone)]
pub struct UpscaleProfileOverride {
    pub image_type: UpscaleImageType,
    pub family: Option<u32>,
    pub id: u32,
    pub passes: Vec<UpscalePass>,
}

impl UpscaleProfileOverride {
    fn matches(&self, target: UpscaleTarget) -> bool {
        self.image_type == target.image_type
            && self.id == target.id
            && self.family.is_none_or(|family| target.family == Some(family))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        UpscaleImageType, UpscaleProfile, UpscaleProfileOverride, UpscaleTarget,
    };
    use crate::upscale_pipeline::UpscalePass;
    use uot_image_postprocess::upscaling::UpscaleFilter;

    #[test]
    fn image_types_parse_only_their_canonical_names() {
        for image_type in UpscaleImageType::ALL.iter().copied() {
            assert_eq!(UpscaleImageType::parse(image_type.as_str()), Some(image_type));
        }
        for other_spelling in ["artItems", "art-items", "paperdollgumps", "cctexland", "ecmobileanimframes"] {
            assert_eq!(UpscaleImageType::parse(other_spelling), None);
        }
    }

    #[test]
    fn override_takes_precedence_over_default_and_fallback() {
        let mut profile = UpscaleProfile::new();
        profile.set_default(
            UpscaleImageType::ArtItems,
            vec![UpscalePass::from(UpscaleFilter::Nearest2x)],
        );
        profile.add_override(UpscaleProfileOverride {
            image_type: UpscaleImageType::ArtItems,
            family: None,
            id: 42,
            passes: vec![UpscalePass::from(UpscaleFilter::FsrEasu2x)],
        });

        assert_eq!(
            profile.passes_for(UpscaleTarget::new(UpscaleImageType::ArtItems, 7), &[UpscalePass::from(UpscaleFilter::Bilinear2x)])[0].filter(),
            Some(UpscaleFilter::Nearest2x)
        );
        assert_eq!(
            profile.passes_for(UpscaleTarget::new(UpscaleImageType::ArtItems, 42), &[UpscalePass::from(UpscaleFilter::Bilinear2x)])[0].filter(),
            Some(UpscaleFilter::FsrEasu2x)
        );
        assert_eq!(
            profile.passes_for(UpscaleTarget::new(UpscaleImageType::ArtLand, 42), &[UpscalePass::from(UpscaleFilter::Bilinear2x)])[0].filter(),
            Some(UpscaleFilter::Bilinear2x)
        );
    }
}
