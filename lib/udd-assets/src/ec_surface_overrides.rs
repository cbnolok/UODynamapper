use std::collections::HashMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use knuffel::Decode;

#[derive(Decode, Debug, Clone)]
pub struct EcSurfaceOverrides {
    #[knuffel(children(name = "cc_art"))]
    pub cc_art_entries: Vec<CcArtOverrideEntry>,
    #[knuffel(children(name = "ec_material"))]
    pub ec_material_entries: Vec<EcMaterialOverrideEntry>,
    #[knuffel(children(name = "ignore"))]
    pub ignore_entries: Vec<IgnoreOverrideEntry>,
}

impl EcSurfaceOverrides {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .wrap_err_with(|| format!("Failed to read KDL file: {:?}", path.as_ref()))?;
        Self::parse(
            path.as_ref().to_str().unwrap_or("EcSurfaceOverrides.kdl"),
            &content,
        )
    }

    pub fn parse(name: &str, content: &str) -> eyre::Result<Self> {
        knuffel::parse(name, content).wrap_err("Failed to parse EcSurfaceOverrides KDL")
    }

    pub fn to_map(&self) -> HashMap<u32, EcSurfaceOverrideEntry> {
        let mut map = HashMap::new();
        for entry in &self.cc_art_entries {
            map.entry(entry.cc_id)
                .or_insert_with(|| EcSurfaceOverrideEntry::new(entry.cc_id))
                .cc_art = Some(CcArtOverride {
                    target_id: entry.target_id,
                    mode: entry.mode.clone(),
                    reason_code: entry.code.clone(),
                });
        }
        for entry in &self.ec_material_entries {
            map.entry(entry.cc_id)
                .or_insert_with(|| EcSurfaceOverrideEntry::new(entry.cc_id))
                .ec_material = Some(EcMaterialOverride {
                    material_id: entry.material_id,
                    package: entry.package.clone(),
                    reason_code: entry.code.clone(),
                });
        }
        for entry in &self.ignore_entries {
            map.entry(entry.cc_id)
                .or_insert_with(|| EcSurfaceOverrideEntry::new(entry.cc_id))
                .ignore = Some(IgnoreOverride {
                    reason_code: entry.code.clone(),
                });
        }
        map
    }
}

#[derive(Decode, Debug, Clone)]
pub struct CcArtOverrideEntry {
    #[knuffel(argument)]
    pub cc_id: u32,
    #[knuffel(argument)]
    pub target_id: u32,
    #[knuffel(property)]
    pub mode: String,
    #[knuffel(property)]
    pub code: Option<String>,
}

#[derive(Decode, Debug, Clone)]
pub struct EcMaterialOverrideEntry {
    #[knuffel(argument)]
    pub cc_id: u32,
    #[knuffel(argument)]
    pub material_id: u32,
    #[knuffel(property)]
    pub package: Option<String>,
    #[knuffel(property)]
    pub code: Option<String>,
}

#[derive(Decode, Debug, Clone)]
pub struct IgnoreOverrideEntry {
    #[knuffel(argument)]
    pub cc_id: u32,
    #[knuffel(property)]
    pub code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EcSurfaceOverrideEntry {
    pub cc_id: u32,
    pub cc_art: Option<CcArtOverride>,
    pub ec_material: Option<EcMaterialOverride>,
    pub ignore: Option<IgnoreOverride>,
}

impl EcSurfaceOverrideEntry {
    fn new(cc_id: u32) -> Self {
        Self {
            cc_id,
            cc_art: None,
            ec_material: None,
            ignore: None,
        }
    }

    pub fn active_action_count(&self) -> usize {
        usize::from(self.cc_art.is_some())
            + usize::from(self.ec_material.is_some())
            + usize::from(self.ignore.is_some())
    }

    pub fn action(&self) -> EcSurfaceOverrideAction<'_> {
        if let Some(cc_art) = &self.cc_art {
            EcSurfaceOverrideAction::CcArt(cc_art)
        } else if let Some(ec_material) = &self.ec_material {
            EcSurfaceOverrideAction::EcMaterial(ec_material)
        } else if let Some(ignore) = &self.ignore {
            EcSurfaceOverrideAction::Ignore(ignore)
        } else {
            EcSurfaceOverrideAction::Invalid
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum EcSurfaceOverrideAction<'a> {
    CcArt(&'a CcArtOverride),
    EcMaterial(&'a EcMaterialOverride),
    Ignore(&'a IgnoreOverride),
    Invalid,
}

#[derive(Debug, Clone)]
pub struct CcArtOverride {
    pub target_id: u32,
    pub mode: String,
    pub reason_code: Option<String>,
}

impl CcArtOverride {
    pub fn mode_kind(&self) -> CcArtOverrideMode {
        match self.mode.as_str() {
            "raw-cc" => CcArtOverrideMode::RawCc,
            "resolve-ec" => CcArtOverrideMode::ResolveEc,
            _ => CcArtOverrideMode::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CcArtOverrideMode {
    RawCc,
    ResolveEc,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct EcMaterialOverride {
    pub material_id: u32,
    pub package: Option<String>,
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IgnoreOverride {
    pub reason_code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_supported_override_actions() {
        let overrides = EcSurfaceOverrides::parse(
            "test.kdl",
            r#"
cc_art 39698 1170 mode="resolve-ec" code="reuse_reviewed_material"
cc_art 39699 40000 mode="raw-cc"
ec_material 39700 1179 package="Texture.uop" code="direct_material"
ignore 39805 code="not_ported"
"#,
        )
        .expect("parse overrides");
        let map = overrides.to_map();

        assert_eq!(map.len(), 4);
        let EcSurfaceOverrideAction::CcArt(cc_art) = map[&39698].action() else {
            panic!("expected cc-art action");
        };
        assert_eq!(cc_art.target_id, 1170);
        assert_eq!(cc_art.mode_kind(), CcArtOverrideMode::ResolveEc);

        let EcSurfaceOverrideAction::CcArt(cc_art) = map[&39699].action() else {
            panic!("expected cc-art action");
        };
        assert_eq!(cc_art.mode_kind(), CcArtOverrideMode::RawCc);

        let EcSurfaceOverrideAction::EcMaterial(material) = map[&39700].action() else {
            panic!("expected ec-material action");
        };
        assert_eq!(material.material_id, 1179);

        let EcSurfaceOverrideAction::Ignore(ignore) = map[&39805].action() else {
            panic!("expected ignore action");
        };
        assert_eq!(ignore.reason_code.as_deref(), Some("not_ported"));
    }
}
