//! Editing model of the TerrainDefinition.uop view of the UOP browser: every definition of the package by material
//! id, in the package's dialect (Enhanced Client or Kingdom Reborn), with undo, the client checks and a save that
//! writes a copy of the client package with only the changed entries. No egui here; `ui/terrain_definition_editor.rs`
//! draws it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::terrain_definition::{
    detect_terrain_definition_dialect, read_terrain_definitions, validate_terrain_definitions,
    write_terrain_definition_package, TerrainDefinitionChanges, TerrainDefinitionEntry, TerrainDefinitionIssue,
    TerrainDefinitionTileAlias,
};
use uocf::uop_container::package::UopPackage;

use crate::logic::keyed_edits::{KeyedEdits, KeyedStore, KeyedValues};

/// A definition field, for undo coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainDefinitionField {
    CoverageExponent,
    Field34,
    Field3c,
    Shader,
    Aliases,
    AliasLegacyLand(usize),
    AliasFlags(usize),
    LayerStretch(usize),
    LayerSampler(usize),
    Technique(usize),
    Tweak(usize),
}

/// The definitions by material id.
pub type TerrainDefinitions = BTreeMap<u32, TerrainDefinitionEntry>;

impl KeyedValues<u32, TerrainDefinitionEntry> for TerrainDefinitions {
    fn value(&self, key: &u32) -> Option<TerrainDefinitionEntry> {
        self.get(key).cloned()
    }
}

impl KeyedStore<u32, TerrainDefinitionEntry> for TerrainDefinitions {
    fn set_value(&mut self, key: &u32, value: Option<TerrainDefinitionEntry>) {
        match value {
            Some(value) => {
                self.insert(*key, value);
            }
            None => {
                self.remove(key);
            }
        }
    }
}

/// The edited definitions of one package.
#[derive(Debug)]
pub struct TerrainDefinitionEditor {
    /// The client package the definitions were read from, the source of the saved copies.
    pub source: PathBuf,
    /// The record layout of the package, kept on save.
    pub dialect: ClientDialect,
    pub definitions: TerrainDefinitions,
    pub edits: KeyedEdits<u32, TerrainDefinitionEntry, TerrainDefinitionField>,
    /// The shader names the package uses, with their string indices: the shaders a definition can switch to.
    pub shaders: BTreeMap<u32, String>,
    /// Shows only definitions whose id or name contains it.
    pub filter: String,
    /// The definition whose aliases, layers and shader values are shown.
    pub selected: Option<u32>,
    /// Material id typed for a duplicated definition.
    pub new_id: u32,
    pub error: Option<String>,
}

impl TerrainDefinitionEditor {
    /// Reads every definition of `package`, loaded from `source`, resolving strings with `string_dictionary`.
    pub fn open(
        source: PathBuf,
        package: &UopPackage,
        string_dictionary: Option<&UoStringDictionary>,
    ) -> Result<Self, String> {
        let dialect = detect_terrain_definition_dialect(package).map_err(|error| format!("{error:#}"))?;
        let definitions =
            read_terrain_definitions(package, string_dictionary, dialect).map_err(|error| format!("{error:#}"))?;
        let shaders = definitions
            .values()
            .filter_map(|definition| definition.texture.as_ref())
            .map(|texture| {
                let name = texture.shader_name.clone().unwrap_or_else(|| format!("string {}", texture.shader_name_index));
                (texture.shader_name_index, name)
            })
            .collect();
        Ok(Self {
            source,
            dialect,
            definitions,
            edits: KeyedEdits::default(),
            shaders,
            filter: String::new(),
            selected: None,
            new_id: 0,
            error: None,
        })
    }

    /// Edits `field` of the definition of material `id`.
    pub fn edit(&mut self, id: u32, field: TerrainDefinitionField, edit: impl FnOnce(&mut TerrainDefinitionEntry)) {
        self.edits.edit(&mut self.definitions, id, Some(field), |definition| {
            if let Some(definition) = definition.as_mut() {
                edit(definition);
            }
        });
    }

    /// Switches the shader of material `id` to the package shader with string index `shader_name_index`.
    pub fn set_shader(&mut self, id: u32, shader_name_index: u32) {
        let name = self.shaders.get(&shader_name_index).cloned();
        self.edit(id, TerrainDefinitionField::Shader, |definition| {
            if let Some(texture) = definition.texture.as_mut() {
                texture.shader_name_index = shader_name_index;
                texture.shader_name = name;
            }
        });
    }

    /// Appends an alias with the next subtype (subtypes stay equal to their positions) and no flags.
    pub fn add_alias(&mut self, id: u32) {
        self.edits.break_coalescing();
        self.edit(id, TerrainDefinitionField::Aliases, |definition| {
            let subtype = definition.aliases.iter().map(|alias| alias.subtype + 1).max().unwrap_or_default();
            definition.aliases.push(TerrainDefinitionTileAlias { subtype, ..TerrainDefinitionTileAlias::default() });
        });
        self.edits.break_coalescing();
    }

    /// Removes the last alias of material `id`; the subtype-0 alias, which the client needs, stays.
    pub fn remove_last_alias(&mut self, id: u32) {
        self.edits.break_coalescing();
        self.edit(id, TerrainDefinitionField::Aliases, |definition| {
            if definition.aliases.len() > 1 {
                definition.aliases.pop();
            }
        });
        self.edits.break_coalescing();
    }

    /// Adds the definition of material `new_id` as a copy of `template`'s (a new material needs a shader and
    /// textures, so it always starts from an existing one).
    pub fn duplicate(&mut self, template: u32, new_id: u32) -> Result<(), String> {
        if self.definitions.contains_key(&new_id) {
            return Err(format!("material {new_id} already has a definition"));
        }
        let template = self.definitions.get(&template).ok_or("the definition to duplicate is missing")?;
        let definition = TerrainDefinitionEntry { id: new_id, ..template.clone() };
        self.edits.edit(&mut self.definitions, new_id, None, |slot| *slot = Some(definition));
        Ok(())
    }

    /// Removes the definition of material `id`.
    pub fn remove(&mut self, id: u32) {
        self.edits.edit(&mut self.definitions, id, None, |definition| *definition = None);
    }

    pub fn undo(&mut self) -> bool {
        self.edits.undo(&mut self.definitions).is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.edits.redo(&mut self.definitions).is_some()
    }

    /// The ids of the definitions the filter shows, in id order.
    pub fn visible_ids(&self) -> Vec<u32> {
        let filter = self.filter.trim().to_lowercase();
        self.definitions
            .values()
            .filter(|definition| {
                filter.is_empty()
                    || definition.id.to_string().contains(&filter)
                    || definition.name.as_deref().is_some_and(|name| name.to_lowercase().contains(&filter))
            })
            .map(|definition| definition.id)
            .collect()
    }

    /// The client checks of every definition.
    pub fn issues(&self) -> Vec<TerrainDefinitionIssue> {
        validate_terrain_definitions(&self.definitions)
    }

    /// Writes a copy of the source package to `output` with the definitions changed since it was opened, in the
    /// package's dialect. Refused while a definition would crash the client.
    pub fn save(&mut self, output: &Path) -> Result<TerrainDefinitionChanges, String> {
        if let Some(issue) = self.issues().into_iter().find(TerrainDefinitionIssue::crashes_client) {
            return Err(format!("not saved: {issue}"));
        }
        let (mut definitions, mut removed) = (Vec::new(), Vec::new());
        for (id, definition) in self.edits.changes_since_opened(&self.definitions) {
            match definition {
                Some(definition) => definitions.push(definition),
                None => removed.push(id),
            }
        }
        let changes = write_terrain_definition_package(&self.source, output, &definitions, &removed, self.dialect)
            .map_err(|error| format!("{error:#}"))?;
        self.edits.mark_saved();
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use uocf::enhanced::terrain_definition::{
        terrain_definition_entry_path, TerrainDefinitionTexture, BASE_SUBTYPE, SHADER_PARAMETER_COUNT,
        TERRAIN_FLAG_NO_DRAW,
    };
    use uocf::uop_container::file::CompressionFlag;

    const GRASS: u32 = 3;
    const WATER: u32 = 9;
    const NEW_MATERIAL: u32 = 250;
    const GRASS_SHADER: u32 = 11;
    const WATER_SHADER: u32 = 12;

    fn definition(id: u32, shader_name_index: u32) -> TerrainDefinitionEntry {
        TerrainDefinitionEntry {
            id,
            aliases: vec![TerrainDefinitionTileAlias { subtype: BASE_SUBTYPE, ..TerrainDefinitionTileAlias::default() }],
            texture: Some(TerrainDefinitionTexture {
                shader_name_index,
                techniques: vec![0; SHADER_PARAMETER_COUNT],
                tweaks: vec![0.0; SHADER_PARAMETER_COUNT],
                ..TerrainDefinitionTexture::default()
            }),
            ..TerrainDefinitionEntry::default()
        }
    }

    fn editor() -> TerrainDefinitionEditor {
        let mut package = UopPackage::new_default();
        for (id, shader) in [(GRASS, GRASS_SHADER), (WATER, WATER_SHADER)] {
            let path = terrain_definition_entry_path(id);
            package.add_file_from_memory(&definition(id, shader).to_bytes().unwrap(), &path, CompressionFlag::Zlib).unwrap();
        }
        TerrainDefinitionEditor::open(PathBuf::new(), &package, None).unwrap()
    }

    #[test]
    fn definitions_are_edited_duplicated_removed_and_undone() {
        let mut editor = editor();
        assert_eq!(editor.dialect, ClientDialect::Enhanced);
        assert_eq!(editor.shaders.keys().copied().collect::<Vec<_>>(), [GRASS_SHADER, WATER_SHADER]);
        editor.edit(GRASS, TerrainDefinitionField::CoverageExponent, |definition| definition.set_coverage_exponent(0.5));
        editor.edit(GRASS, TerrainDefinitionField::CoverageExponent, |definition| definition.set_coverage_exponent(1.5));
        editor.set_shader(GRASS, WATER_SHADER);
        editor.add_alias(GRASS);
        editor.edit(GRASS, TerrainDefinitionField::AliasFlags(1), |definition| {
            definition.aliases[1].tile_flags |= TERRAIN_FLAG_NO_DRAW
        });
        assert_eq!(editor.definitions[&GRASS].aliases[1].subtype, 1);
        assert!(editor.duplicate(GRASS, WATER).is_err());
        editor.duplicate(GRASS, NEW_MATERIAL).unwrap();
        assert_eq!(editor.definitions[&NEW_MATERIAL].coverage_exponent(), 1.5);
        editor.remove(WATER);
        assert!(editor.issues().is_empty());

        let changes = editor.edits.changes_since_opened(&editor.definitions);
        assert_eq!(changes.keys().copied().collect::<Vec<_>>(), [GRASS, WATER, NEW_MATERIAL]);
        while editor.undo() {}
        assert_eq!(editor.definitions[&GRASS].coverage_exponent(), 0.0, "the dragged value is one undo step");
        assert_eq!(editor.definitions[&GRASS].texture.as_ref().unwrap().shader_name_index, GRASS_SHADER);
        assert!(editor.definitions.contains_key(&WATER) && !editor.definitions.contains_key(&NEW_MATERIAL));
        assert!(!editor.edits.is_dirty());
        assert!(editor.redo());
        assert_eq!(editor.definitions[&GRASS].coverage_exponent(), 1.5);
    }

    #[test]
    fn the_base_alias_stays_and_a_crashing_definition_is_not_saved() {
        let mut editor = editor();
        editor.remove_last_alias(GRASS);
        assert_eq!(editor.definitions[&GRASS].aliases.len(), 1);
        editor.edit(GRASS, TerrainDefinitionField::Aliases, |definition| definition.aliases.clear());
        let error = editor.save(Path::new("TerrainDefinition.uop")).unwrap_err();
        assert!(error.contains("subtype-0"), "{error}");
        editor.undo();
        assert!(editor.save(Path::new("TerrainDefinition.uop")).is_err(), "saving needs a source package");
    }
}
