//! The land texture variation list, `runtime_specs/land/LandTextureVariation.kdl`: the KR and Enhanced terrain
//! materials whose colour layers the land shader may break up when a style's `land_texture_variation` is on, and
//! which of them it may also turn by quarter turns (land/sampling.wesl). Materials it does not list keep their plain
//! world-space tiling: masonry, planks, tiles and furrows must line up, and cliffs and baked light have a direction.
//! The file is hand-written and read strictly (`uot_kdl::strict`): every error names its line. It is read at
//! startup; a user copy under the cache dir takes the bundled file's place (`configs::runtime_specs`).

use bevy::prelude::Resource;
use color_eyre::eyre;
use uot_kdl::ast::{Literal, Node, Value};
use uot_kdl::span::LineSpan;
use uot_kdl::strict::{self, NodeSchema};

use crate::configs::runtime_specs::SpecFiles;

/// The folder of the file under `runtime_specs`, and its name.
pub const LAND_TEXTURE_VARIATION_SPECS_FOLDER: &str = "land";
pub const LAND_TEXTURE_VARIATION_FILE_NAME: &str = "LandTextureVariation.kdl";
/// The material ids the file may name: the KR and Enhanced terrain materials number 0 to 200. Sizes the shaders'
/// bitmask (`LandTextureVariationUniform`), which a larger id would not fit.
pub const LAND_TEXTURE_VARIATION_MATERIAL_CAPACITY: u32 = 256;
/// The bits of a material in the bitmask: listed, and turned by quarter turns.
pub const LAND_TEXTURE_VARIATION_BITS_PER_MATERIAL: u32 = 2;
pub const LAND_TEXTURE_VARIATION_ELIGIBLE_BIT: u32 = 1;
pub const LAND_TEXTURE_VARIATION_ROTATE_BIT: u32 = 2;
/// The 32-bit words of the bitmask.
pub const LAND_TEXTURE_VARIATION_MASK_WORDS: usize =
    (LAND_TEXTURE_VARIATION_MATERIAL_CAPACITY * LAND_TEXTURE_VARIATION_BITS_PER_MATERIAL / u32::BITS) as usize;
/// What the log calls the file.
const LAND_TEXTURE_VARIATION_WHAT: &str = "the land texture variation materials";

const MATERIAL_NODE: &str = "material";
const ROTATE_PROPERTY: &str = "rotate";
pub const LAND_TEXTURE_VARIATION_SCHEMA: &[NodeSchema] = &[NodeSchema::leaf(MATERIAL_NODE, &[ROTATE_PROPERTY])];

/// The materials `first..=last` and whether they turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaterialVariationRule {
    first: u32,
    last: u32,
    rotate: bool,
    /// The rule's 1-based line, for the error of a material listed twice.
    line: usize,
}

/// The materials the land texture variation may break up.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct LandTextureVariationMaterials {
    rules: Vec<MaterialVariationRule>,
}

impl LandTextureVariationMaterials {
    /// Reads the file at startup: the bundled file is required, a missing or invalid one stops the app; a user copy
    /// that does not read leaves it in force, with a warning.
    pub fn load() -> Self {
        let loaded = SpecFiles::of(LAND_TEXTURE_VARIATION_SPECS_FOLDER, LAND_TEXTURE_VARIATION_FILE_NAME)
            .load(Self::decode)
            .unwrap_or_else(|error| crate::configs::settings::exit_on_startup_config_error(&error));
        loaded.report(LAND_TEXTURE_VARIATION_WHAT);
        loaded.value
    }

    pub fn decode(source_name: &str, text: &str) -> eyre::Result<Self> {
        let document = uot_kdl::parse_ast_line_span(source_name, text)
            .map_err(|error| strict::in_file(source_name, strict::describe_error(text, &error)))?;
        let mut rules: Vec<MaterialVariationRule> = Vec::new();
        for node in &document.nodes {
            let rule = strict::node_schema(node, None, LAND_TEXTURE_VARIATION_SCHEMA)
                .and_then(|schema| decode_rule(node, schema))
                .map_err(|error| strict::in_file(source_name, error))?;
            if let Some(earlier) = rules.iter().find(|earlier| earlier.first <= rule.last && rule.first <= earlier.last) {
                let id = rule.first.max(earlier.first);
                let error = strict::error_at(node.node_name.span(),
                    format!("material {id} is listed already, on line {}", earlier.line));
                return Err(strict::in_file(source_name, error));
            }
            rules.push(rule);
        }
        Ok(Self { rules })
    }

    /// Whether the material is listed, and then whether it turns: `None` keeps its plain tiling.
    pub fn material(&self, id: u32) -> Option<bool> {
        self.rules.iter().find(|rule| (rule.first..=rule.last).contains(&id)).map(|rule| rule.rotate)
    }

    /// The shaders' bitmask: `LAND_TEXTURE_VARIATION_BITS_PER_MATERIAL` bits per material id, from the lowest bit of
    /// the first word, `LAND_TEXTURE_VARIATION_ELIGIBLE_BIT` when listed and `LAND_TEXTURE_VARIATION_ROTATE_BIT` when
    /// it also turns.
    pub fn bitmask(&self) -> [u32; LAND_TEXTURE_VARIATION_MASK_WORDS] {
        let mut words = [0u32; LAND_TEXTURE_VARIATION_MASK_WORDS];
        let per_word = u32::BITS / LAND_TEXTURE_VARIATION_BITS_PER_MATERIAL;
        for rule in &self.rules {
            for id in rule.first..=rule.last {
                let bits = LAND_TEXTURE_VARIATION_ELIGIBLE_BIT | if rule.rotate { LAND_TEXTURE_VARIATION_ROTATE_BIT } else { 0 };
                words[(id / per_word) as usize] |= bits << ((id % per_word) * LAND_TEXTURE_VARIATION_BITS_PER_MATERIAL);
            }
        }
        words
    }
}

fn decode_rule(node: &Node<LineSpan>, schema: &NodeSchema) -> eyre::Result<MaterialVariationRule> {
    strict::check_properties(node, schema.properties)?;
    strict::reject_children(node)?;
    let (first, last) = id_range(node)?;
    let value = strict::required_property(node, ROTATE_PROPERTY)?;
    let rotate = match &*value.literal {
        Literal::Bool(rotate) => *rotate,
        _ => return Err(strict::error_at(value.literal.span(), format!("{ROTATE_PROPERTY}: expected true or false"))),
    };
    Ok(MaterialVariationRule { first, last, rotate, line: node.node_name.span().0.line + 1 })
}

/// The node's material id, or the first and last id of its range.
fn id_range(node: &Node<LineSpan>) -> eyre::Result<(u32, u32)> {
    match node.arguments.as_slice() {
        [id] => {
            let id = material_id(id, "the id")?;
            Ok((id, id))
        }
        [first, last] => {
            let (first_id, last_id) = (material_id(first, "the first id")?, material_id(last, "the last id")?);
            if last_id < first_id {
                return Err(strict::error_at(last.literal.span(),
                    format!("the range ends at {last_id}, before its first id {first_id}")));
            }
            Ok((first_id, last_id))
        }
        arguments => {
            let span = arguments.get(2).map_or(node.node_name.span(), |extra| extra.literal.span());
            Err(strict::error_at(span, format!(
                "`{MATERIAL_NODE}` takes a material id, or the first and last id of a range; found {} arguments",
                arguments.len())))
        }
    }
}

fn material_id(value: &Value<LineSpan>, field: &str) -> eyre::Result<u32> {
    let id = strict::value_u32(value, field)?;
    if id >= LAND_TEXTURE_VARIATION_MATERIAL_CAPACITY {
        return Err(strict::error_at(value.literal.span(),
            format!("{field}: {id} is past the last material id, {}", LAND_TEXTURE_VARIATION_MATERIAL_CAPACITY - 1)));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "LandTextureVariation.kdl";

    fn decode(text: &str) -> LandTextureVariationMaterials {
        LandTextureVariationMaterials::decode(FILE, text).unwrap_or_else(|error| panic!("{error:#}"))
    }

    /// The error starts with the file and the line, and says what is wrong.
    fn assert_error(text: &str, line: usize, needle: &str) {
        let error = format!("{:#}", LandTextureVariationMaterials::decode(FILE, text).expect_err("the file should not read"));
        assert!(error.starts_with(&format!("{FILE}: line {line}, column ")), "{error}");
        assert!(error.contains(needle), "{error}");
    }

    #[test]
    fn materials_and_ranges_take_their_rotation_and_the_rest_stay_plain() {
        let materials = decode("material 1 rotate=true // grass\nmaterial 129 144 rotate=false\n");
        assert_eq!(materials.material(1), Some(true));
        assert_eq!(materials.material(129), Some(false));
        assert_eq!(materials.material(144), Some(false));
        assert_eq!(materials.material(2), None);
        assert_eq!(materials.material(145), None);
        assert_eq!(decode("").material(1), None);
    }

    #[test]
    fn the_bitmask_packs_two_bits_per_material() {
        let per_word = u32::BITS / LAND_TEXTURE_VARIATION_BITS_PER_MATERIAL;
        let materials = decode("material 1 rotate=true\nmaterial 17 rotate=false\nmaterial 255 rotate=true\n");
        let mask = materials.bitmask();
        let bits = |id: u32| (mask[(id / per_word) as usize] >> ((id % per_word) * LAND_TEXTURE_VARIATION_BITS_PER_MATERIAL)) & 3;
        assert_eq!(bits(0), 0);
        assert_eq!(bits(1), LAND_TEXTURE_VARIATION_ELIGIBLE_BIT | LAND_TEXTURE_VARIATION_ROTATE_BIT);
        assert_eq!(bits(17), LAND_TEXTURE_VARIATION_ELIGIBLE_BIT);
        assert_eq!(bits(255), LAND_TEXTURE_VARIATION_ELIGIBLE_BIT | LAND_TEXTURE_VARIATION_ROTATE_BIT);
        assert_eq!(mask.iter().map(|word| word.count_ones()).sum::<u32>(), 5);
    }

    /// The land shader sizes its copy of the bitmask (land/land_bindings.wesl) by the same capacity.
    #[test]
    fn the_shaders_bitmask_has_room_for_every_material() {
        const WORDS_PER_VEC4: usize = 4;
        let bindings = include_str!("../../assets/shaders/world/land/land_bindings.wesl");
        for line in [
            format!("const LAND_TEXTURE_VARIATION_MATERIAL_CAPACITY: u32 = {LAND_TEXTURE_VARIATION_MATERIAL_CAPACITY}u;"),
            format!("const LAND_TEXTURE_VARIATION_VEC4_COUNT: u32 = {}u;", LAND_TEXTURE_VARIATION_MASK_WORDS / WORDS_PER_VEC4),
        ] {
            assert!(bindings.contains(&line), "land_bindings.wesl lacks `{line}`");
        }
    }

    /// The seed list: the natural ground breaks up, the masonry, planks, tiles and furrows never do, and the
    /// directional textures do not turn.
    #[test]
    fn the_bundled_file_lists_natural_ground_and_leaves_masonry_alone() {
        const GRASS: u32 = 1;
        const SAND: u32 = 7;
        const COBBLESTONE: u32 = 9;
        const PLANKS: u32 = 11;
        const FURROWS: u32 = 20;
        const CLIFF: u32 = 46;
        let materials = decode(include_str!("../../assets/runtime_specs/land/LandTextureVariation.kdl"));
        assert_eq!(materials.material(GRASS), Some(true));
        assert_eq!(materials.material(SAND), Some(false));
        for id in [COBBLESTONE, PLANKS, FURROWS, CLIFF] {
            assert_eq!(materials.material(id), None, "material {id}");
        }
    }

    #[test]
    fn errors_name_the_line() {
        assert_error("material 1 rotate=true\ntexture 2 rotate=true\n", 2,
            "unknown node `texture` at the top level: expected `material`");
        assert_error("material 1 turn=true\n", 1, "unknown property `turn` of `material`");
        assert_error("material 1\n", 1, "`material`: missing rotate=");
        assert_error("material 1 rotate=1\n", 1, "rotate: expected true or false");
        assert_error("material 1 rotate=\"yes\"\n", 1, "rotate: expected true or false");
        assert_error("material 256 rotate=true\n", 1, "the id: 256 is past the last material id, 255");
        assert_error("material -1 rotate=true\n", 1, "the id: not a u32");
        assert_error("material 9 3 rotate=true\n", 1, "the range ends at 3, before its first id 9");
        assert_error("material rotate=true\n", 1, "found 0 arguments");
        assert_error("material 1 2 3 rotate=true\n", 1, "found 3 arguments");
        assert_error("material 1 rotate=true { material 2 rotate=true; }\n", 1, "`material` takes no children");
        assert_error("material 10 20 rotate=true\nmaterial 5 12 rotate=false\n", 2,
            "material 10 is listed already, on line 1");
    }
}
