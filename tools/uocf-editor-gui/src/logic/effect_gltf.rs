//! glTF (`.glb`) export of the Effects tab: one NIF file, or the particles a previewed definition list plays
//! (see the `effect-gltf` crate for what is exported).
//!
//! A whole-effect export takes the parts the preview resolved (the same random draws, LOD levels and
//! attach-mode choices) and each part's own definition fields (offsets, scale, duration, loop flag). Position
//! offset definitions and the preview's overrides (duration, loop mode, movement, hue) are not applied.

use std::path::Path;
use std::sync::Arc;

use effect_gltf::{ChaosPart, EffectPart, ExportOptions, GlbExport, PartContent};
use gamebryo_nif::NifFile;
use particle_sim::chaos::ChaosParticleType;
use particle_sim::ParticleEffectParams;
use uocf::enhanced::effect_definition::EffectDefinition;

use crate::logic::effect_preview::{path_label, PartStatus, PreviewScene};
use crate::logic::effects_editor::definition_at;

/// File dialog filter of glTF binaries.
pub const GLB_FILTER: (&str, &[&str]) = ("glTF binary", &["glb"]);
/// The glTF binary extension.
pub const GLB_EXTENSION: &str = "glb";
/// The NIF extension (MESH particle resources).
const NIF_EXTENSION: &str = ".nif";
/// Warnings listed in the status message at most (all go to the log).
const MAX_LISTED_WARNINGS: usize = 3;

/// Reads `EffectTexture.uop` files by name, with the pending changes, and the NIF drafts that do not write.
pub struct GltfSources<'a> {
    pub payload: &'a dyn Fn(&str) -> Option<Arc<[u8]>>,
    pub nif_drafts: &'a dyn Fn(&str) -> Option<Arc<NifFile>>,
}

impl GltfSources<'_> {
    /// The texture loader of the exporter.
    fn texture(&self, name: &str) -> Option<Vec<u8>> {
        (self.payload)(name).map(|bytes| bytes.to_vec())
    }

    /// A NIF by name: its draft, or its parsed payload.
    fn nif(&self, name: &str) -> Result<Arc<NifFile>, String> {
        if let Some(draft) = (self.nif_drafts)(name) {
            return Ok(draft);
        }
        let payload = (self.payload)(name).ok_or_else(|| format!("{name}: not in EffectTexture.uop"))?;
        NifFile::read(&payload).map(Arc::new).map_err(|error| format!("{name}: {error}"))
    }

    fn compiled_nif(&self, name: &str) -> Result<particle_sim::NifEffectDesc, String> {
        self.nif(name).map(|nif| particle_sim::nif::compile(&nif, name).desc)
    }

    fn chaos(&self, name: &str) -> Result<ChaosPart, String> {
        let payload = (self.payload)(name).ok_or_else(|| format!("{name}: not in EffectTexture.uop"))?;
        let report = particle_sim::chaos::compile::compile_chaos_source(&payload);
        let desc = report.desc.ok_or_else(|| format!("{name}: the script does not compile"))?;
        let mut meshes = Vec::new();
        for sequence in desc.sequences.iter().filter(|sequence| sequence.particle_type == ChaosParticleType::Mesh) {
            let resource = &sequence.resource;
            if !resource.to_ascii_lowercase().ends_with(NIF_EXTENSION) || meshes.iter().any(|m: &particle_sim::NifEffectDesc| m.name == *resource) {
                continue;
            }
            if let Ok(mesh) = self.compiled_nif(resource) {
                meshes.push(mesh);
            }
        }
        Ok(ChaosPart { desc, source: Some(String::from_utf8_lossy(&payload).into_owned()), meshes })
    }
}

/// Exports one NIF file.
pub fn export_nif(nif: &NifFile, name: &str, sources: &GltfSources<'_>, options: &ExportOptions) -> GlbExport {
    effect_gltf::export_nif_file(nif, name, &|texture: &str| sources.texture(texture), options)
}

/// The parts `scene` plays (built from `definitions`) as exporter parts; parts that do not play are listed as
/// unavailable, parts that draw nothing are left out.
pub fn effect_parts(scene: &PreviewScene, definitions: &[EffectDefinition], sources: &GltfSources<'_>) -> Vec<EffectPart> {
    let mut parts = Vec::new();
    for part in &scene.parts {
        let params = match definition_at(definitions, &part.path) {
            Some(EffectDefinition::Particle(particle)) => ParticleEffectParams::from(particle),
            _ => ParticleEffectParams::default(),
        };
        let (name, content) = match &part.status {
            PartStatus::Nif { name, .. } => {
                (name.clone(), sources.compiled_nif(name).map_or_else(PartContent::Unavailable, PartContent::Nif))
            }
            PartStatus::Chaos { name, .. } => {
                (name.clone(), sources.chaos(name).map_or_else(PartContent::Unavailable, PartContent::Chaos))
            }
            PartStatus::NotVisual => continue,
            status => (String::new(), PartContent::Unavailable(status.describe())),
        };
        let label = format!("{} {name}", path_label(&part.path)).trim().to_owned();
        parts.push(EffectPart { label, params, content });
    }
    parts
}

/// Exports the parts `scene` plays.
pub fn export_effect(
    name: &str,
    scene: &PreviewScene,
    definitions: &[EffectDefinition],
    sources: &GltfSources<'_>,
    options: &ExportOptions,
) -> GlbExport {
    let parts = effect_parts(scene, definitions, sources);
    effect_gltf::export_effect(name, &parts, &|texture: &str| sources.texture(texture), options)
}

/// Writes an export; the status message counts the warnings (each is logged).
pub fn write_glb(path: &Path, export: &GlbExport) -> Result<String, String> {
    std::fs::write(path, &export.bytes).map_err(|error| format!("Export failed: {}: {error}", path.display()))?;
    for warning in &export.warnings {
        log::warn!("glTF export {}: {warning}", path.display());
    }
    let mut message = format!("Exported {}", path.display());
    if !export.warnings.is_empty() {
        let listed: Vec<&str> = export.warnings.iter().take(MAX_LISTED_WARNINGS).map(String::as_str).collect();
        let more = export.warnings.len().saturating_sub(MAX_LISTED_WARNINGS);
        message += &format!(" ({} warning(s): {}{})", export.warnings.len(), listed.join("; "), if more > 0 { "; ..." } else { "" });
    }
    Ok(message + ".")
}

/// A file name for an export: `stem` without characters file systems refuse, with the glTF extension.
pub fn glb_file_name(stem: &str) -> String {
    let stem = stem.strip_suffix(NIF_EXTENSION).unwrap_or(stem);
    let clean: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') { c } else { '_' })
        .collect();
    format!("{clean}.{GLB_EXTENSION}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::effect_preview::{PreviewInput, SimulationSettings};
    use crate::logic::effects_editor::DefinitionKind;
    use crate::logic::nif_editor::tests::{test_nif, test_nif_bytes, TEST_MESH_NAME};
    use effect_gltf::validate::{glb_json, validate_glb};

    const NIF_NAME: &str = "test.nif";

    #[test]
    fn nif_files_and_previewed_effects_export() {
        let bytes: Arc<[u8]> = Arc::from(test_nif_bytes());
        let payload = move |name: &str| name.eq_ignore_ascii_case(NIF_NAME).then(|| Arc::clone(&bytes));
        let no_drafts = |_: &str| None;
        let sources = GltfSources { payload: &payload, nif_drafts: &no_drafts };

        let export = export_nif(&test_nif(), NIF_NAME, &sources, &ExportOptions::default());
        validate_glb(&export.bytes).unwrap();
        let document = glb_json(&export.bytes).unwrap();
        assert!(document["nodes"].as_array().unwrap().iter().any(|node| node["name"] == TEST_MESH_NAME));

        // The script preview of the file: one particle with default fields.
        let input = PreviewInput { strings: None, payload: &payload, nif_drafts: None };
        let settings = SimulationSettings::default();
        let scene = PreviewScene::build_script(NIF_NAME, &settings, &input);
        let definitions = vec![DefinitionKind::Particle.new_definition()];
        let parts = effect_parts(&scene, &definitions, &sources);
        assert_eq!(parts.len(), 1);
        assert!(matches!(parts[0].content, PartContent::Nif(_)));
        let export = export_effect("test effect", &scene, &definitions, &sources, &ExportOptions::default());
        validate_glb(&export.bytes).unwrap();
        assert_eq!(glb_file_name("a b/c.nif"), "a_b_c.glb");
    }
}
