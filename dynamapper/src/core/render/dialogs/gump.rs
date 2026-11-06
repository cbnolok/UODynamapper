use crate::core::controls::input_actions::{ActionCloseActiveDialog, ActionToggleGumpDialog};
use crate::core::render::dialogs::image_background::{image_background_control, ImageBackground};
use crate::core::render::scene::camera::UiCameraResource;
use crate::configs::shader_presets::UniformState;
use crate::core::uo_files_loader::{GumpClient, GumpMapRes, HuesPackageRes, TileMetaPackageRes};
use crate::ingame_sysmessage_logger;
use crate::{
    core::constants,
    core::render::dialogs,
    prelude::*,
};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass, EguiTextureHandle};
use knuffel::Decode;
use std::{collections::HashMap, path::Path};

#[derive(Resource)]
pub struct GumpDialogState {
    pub open: bool,
    pub id: String,
    open_gumps: Vec<OpenGump>,
    pub paperdoll_open: bool,
    pub paperdoll_profile_id: String,
    pub paperdoll_body_id: String,
    pub paperdoll_body_hue: String,
    paperdoll_equipment: Vec<PaperdollEquipmentRow>,
}

struct OpenGump {
    title: String,
    width: u16,
    height: u16,
    image: Handle<Image>,
    texture_id: egui::TextureId,
    content: OpenGumpContent,
    /// The preset's gump client `image` was drawn for; a preset change redraws it.
    client: GumpClient,
    /// Why the active preset has no image for this window, shown in place of the image.
    error: Option<String>,
}

/// What an open gump window draws, kept so a preset change can redraw it from the other package.
enum OpenGumpContent {
    Gump(u32),
    Paperdoll {
        profile: PaperdollProfile,
        layers: Vec<PaperdollLayer>,
    },
}

enum GumpOpenRequest {
    GumpById,
    Paperdoll,
}

#[derive(Clone)]
struct PaperdollEquipmentRow {
    item_id: String,
    hue: String,
}

impl Default for PaperdollEquipmentRow {
    fn default() -> Self {
        Self {
            item_id: String::new(),
            hue: "0".to_string(),
        }
    }
}

impl Default for GumpDialogState {
    fn default() -> Self {
        Self {
            open: false,
            id: String::new(),
            open_gumps: Vec::new(),
            paperdoll_open: false,
            paperdoll_profile_id: "human_male".to_string(),
            paperdoll_body_id: String::new(),
            paperdoll_body_hue: "0".to_string(),
            paperdoll_equipment: vec![PaperdollEquipmentRow::default(); 6],
        }
    }
}

#[derive(Resource, Clone)]
struct PaperdollProfilesRes {
    profiles: Vec<PaperdollProfile>,
}

#[derive(Resource, Default, Clone)]
struct PaperdollWearableRulesRes {
    rules: Option<PaperdollWearableRules>,
}

#[derive(Clone, Default)]
struct PaperdollWearableRules {
    paperdoll_draw_order: HashMap<String, u16>,
    wearable_rules: Vec<WearableRule>,
}

const PAPERDOLL_SPECS_DIR: &str = "runtime_specs/paperdoll";
const WEARABLE_RULES_FILE_NAME: &str = "WearableRules.kdl";
const PAPERDOLL_PROFILES_FILE_NAME: &str = "PaperdollProfiles.kdl";
/// The draw-order table this dialog reads.
const PAPERDOLL_DIRECTION: &str = "paperdoll";
const WEARABLE_ITEMS_PROPERTY: &str = "items";
const WEARABLE_PRIORITY_PROPERTY: &str = "priority";

/// `draworder "<direction>" { slot "<slot>" priority=N }`, and
/// `wearable "<slot>" items="..." { override priority=N [race=] [direction=] { when "<slot>" items="..." } }`.
const WEARABLE_RULES_SCHEMA: &[uot_kdl::strict::NodeSchema] = &[
    uot_kdl::strict::NodeSchema {
        name: "draworder",
        properties: &[],
        children: &[uot_kdl::strict::NodeSchema::leaf("slot", &[WEARABLE_PRIORITY_PROPERTY])],
    },
    uot_kdl::strict::NodeSchema {
        name: "wearable",
        properties: &[WEARABLE_ITEMS_PROPERTY],
        children: &[uot_kdl::strict::NodeSchema {
            name: "override",
            properties: &[WEARABLE_PRIORITY_PROPERTY, "race", "direction"],
            children: &[uot_kdl::strict::NodeSchema::leaf("when", &[WEARABLE_ITEMS_PROPERTY])],
        }],
    },
];

/// `profile "<id>" equipment_offset=N [label=] [canvas_width=] [canvas_height=] [body_x=] [body_y=] [equipment_x=]
/// [equipment_y=]`, decoded by `PaperdollProfilesKdl`.
pub(crate) const PAPERDOLL_PROFILES_SCHEMA: &[uot_kdl::strict::NodeSchema] = &[uot_kdl::strict::NodeSchema::leaf(
    "profile",
    &[
        "label",
        "equipment_offset",
        "canvas_width",
        "canvas_height",
        "body_x",
        "body_y",
        "equipment_x",
        "equipment_y",
    ],
)];

impl PaperdollWearableRules {
    fn load_from_path(path: &Path) -> color_eyre::eyre::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|error| color_eyre::eyre::eyre!("{}: cannot read: {error}", path.display()))?;
        Self::load_from_str(&path.display().to_string(), &content)
    }

    fn load_from_str(source_name: &str, content: &str) -> color_eyre::eyre::Result<Self> {
        Self::parse(content).map_err(|error| uot_kdl::strict::in_file(source_name, error))
    }

    fn parse(content: &str) -> color_eyre::eyre::Result<Self> {
        use color_eyre::eyre;
        use uot_kdl::ast::{Node, Value};
        use uot_kdl::span::LineSpan;
        use uot_kdl::strict::{arguments, error_at, property, required_property, value_string, value_u16};

        fn argument_string(node: &Node<LineSpan>, field: &str) -> eyre::Result<String> {
            let [value] = arguments::<1>(node)?;
            value_string(value, field)
        }

        fn items(value: &Value<LineSpan>) -> eyre::Result<WearableItemMatcher> {
            let text = value_string(value, WEARABLE_ITEMS_PROPERTY)?;
            parse_wearable_items(&text).map_err(|error| error_at(value.literal.span(), error))
        }

        fn parse_when(node: &Node<LineSpan>) -> eyre::Result<WearableCondition> {
            Ok(WearableCondition {
                slot: argument_string(node, "when slot")?,
                items: items(required_property(node, WEARABLE_ITEMS_PROPERTY)?)?,
            })
        }

        fn parse_override(node: &Node<LineSpan>) -> eyre::Result<WearableOverrideRule> {
            arguments::<0>(node)?;
            let optional_string = |name: &str| property(node, name).map(|value| value_string(value, name)).transpose();
            Ok(WearableOverrideRule {
                draw_order_priority: value_u16(required_property(node, WEARABLE_PRIORITY_PROPERTY)?, "priority")?,
                race: optional_string("race")?.map(|race| race.to_ascii_lowercase()),
                direction: optional_string("direction")?.map(|direction| direction.to_ascii_lowercase()),
                conditions: node.children().map(|child| parse_when(child)).collect::<eyre::Result<_>>()?,
            })
        }

        fn parse_wearable(node: &Node<LineSpan>) -> eyre::Result<WearableRule> {
            Ok(WearableRule {
                target_slot: argument_string(node, "wearable slot")?,
                target_items: items(required_property(node, WEARABLE_ITEMS_PROPERTY)?)?,
                overrides: node.children().map(|child| parse_override(child)).collect::<eyre::Result<_>>()?,
            })
        }

        fn parse_draworder(node: &Node<LineSpan>, paperdoll_draw_order: &mut HashMap<String, u16>) -> eyre::Result<()> {
            // Only the paperdoll direction is drawn here; other directions' tables are valid but unused.
            let is_paperdoll = argument_string(node, "draworder direction")? == PAPERDOLL_DIRECTION;
            for child in node.children() {
                let slot = argument_string(child, "draworder slot")?;
                let priority = value_u16(required_property(child, WEARABLE_PRIORITY_PROPERTY)?, "priority")?;
                if is_paperdoll {
                    paperdoll_draw_order.insert(slot, priority);
                }
            }
            Ok(())
        }

        let document = uot_kdl::parse_ast_line_span(WEARABLE_RULES_FILE_NAME, content)
            .map_err(|error| uot_kdl::strict::describe_error(content, &error))?;
        uot_kdl::strict::check_document(&document, WEARABLE_RULES_SCHEMA)?;
        let mut paperdoll_draw_order = HashMap::new();
        let mut wearable_rules = Vec::new();
        for node in document.nodes.iter() {
            match &**node.node_name {
                "draworder" => parse_draworder(node, &mut paperdoll_draw_order)?,
                "wearable" => wearable_rules.push(parse_wearable(node)?),
                _ => unreachable!("the schema check admits only draworder and wearable"),
            }
        }

        Ok(Self {
            paperdoll_draw_order,
            wearable_rules,
        })
    }

    fn base_priority(&self, slot: &str, fallback: u16) -> u16 {
        self.paperdoll_draw_order
            .get(slot)
            .copied()
            .unwrap_or(fallback)
    }

    fn priority_for(
        &self,
        target_slot: &str,
        target_item_id: u32,
        equipped: &[PaperdollEquippedItem],
        race: &str,
        fallback: u16,
    ) -> u16 {
        let mut priority = self.base_priority(target_slot, fallback);
        for rule in &self.wearable_rules {
            if rule.target_slot != target_slot || !rule.target_items.matches(target_item_id) {
                continue;
            }

            for override_rule in &rule.overrides {
                if override_rule
                    .direction
                    .as_ref()
                    .is_some_and(|direction| direction != "paperdoll")
                {
                    continue;
                }
                if override_rule
                    .race
                    .as_ref()
                    .is_some_and(|required_race| required_race != race)
                {
                    continue;
                }
                if override_rule.matches_equipped(equipped) {
                    priority = override_rule.draw_order_priority;
                }
            }
        }

        priority
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_paperdoll_specs_parse() {
        let profiles = PaperdollProfilesRes::load_from_str(
            "PaperdollProfiles.kdl",
            include_str!("../../../../assets/runtime_specs/paperdoll/PaperdollProfiles.kdl"),
        )
        .unwrap_or_else(|error| panic!("profiles parse: {error}"));
        assert!(!profiles.profiles.is_empty());

        let rules = PaperdollWearableRules::load_from_str(
            "WearableRules.kdl",
            include_str!("../../../../assets/runtime_specs/paperdoll/WearableRules.kdl"),
        )
        .unwrap_or_else(|error| panic!("wearable rules parse: {error:?}"));
        assert_eq!(rules.base_priority("cape", 0), 500);
    }

    #[test]
    fn wearable_rules_apply_paperdoll_override_priority() {
        let rules = PaperdollWearableRules::load_from_str(
            "WearableRules.kdl",
            include_str!("../../../../assets/runtime_specs/paperdoll/WearableRules.kdl"),
        )
        .unwrap_or_else(|error| panic!("wearable rules parse: {error:?}"));
        let equipped = vec![
            PaperdollEquippedItem {
                item_id: 4232,
                slot: "neck".to_string(),
                gump_id: 0,
                hue_id: 0,
                partial_hue: false,
                fallback_priority: 10,
            },
            PaperdollEquippedItem {
                item_id: 5000,
                slot: "chest".to_string(),
                gump_id: 0,
                hue_id: 0,
                partial_hue: false,
                fallback_priority: 20,
            },
        ];

        assert_eq!(
            rules.priority_for("neck", 4232, &equipped, "human", 10),
            18100
        );
    }

    fn rules_error(content: &str) -> String {
        PaperdollWearableRules::load_from_str("WearableRules.kdl", content).err().expect("rules error").to_string()
    }

    fn profiles_error(content: &str) -> String {
        PaperdollProfilesRes::load_from_str("PaperdollProfiles.kdl", content).err().expect("profiles error").to_string()
    }

    #[test]
    fn wearable_rules_reject_unknown_nodes_and_properties_with_their_position() {
        assert_eq!(
            rules_error("draworder \"paperdoll\" {\n    slot \"cape\" priority=500\n}\nwearables \"neck\" items=\"any\"\n"),
            "WearableRules.kdl: line 4, column 1: unknown node `wearables` at the top level: expected `draworder` or \
             `wearable`"
        );
        assert_eq!(
            rules_error("wearable \"neck\" items=\"any\" {\n    override priority=1 gender=\"male\"\n}\n"),
            "WearableRules.kdl: line 2, column 25: unknown property `gender` of `override`: expected one of \
             `priority`, `race`, `direction`"
        );
        assert_eq!(
            rules_error("wearable \"neck\" items=\"any\" {\n    override priority=1 {\n        unless \"chest\"\n    }\n}\n"),
            "WearableRules.kdl: line 3, column 9: unknown node `unless` in `override`: expected `when`"
        );
    }

    #[test]
    fn wearable_rules_reject_missing_and_invalid_values() {
        assert_eq!(
            rules_error("wearable \"neck\"\n"),
            "WearableRules.kdl: line 1, column 1: `wearable`: missing items="
        );
        assert_eq!(
            rules_error("wearable \"neck\" items=\"any\" {\n    override race=\"elf\"\n}\n"),
            "WearableRules.kdl: line 2, column 5: `override`: missing priority="
        );
        assert_eq!(
            rules_error("wearable \"neck\" items=\"4232,abc\"\n"),
            "WearableRules.kdl: line 1, column 23: items: `abc` is not an item id; expected `any` or ids like \
             0x1F03,7939"
        );
        assert_eq!(
            rules_error("draworder \"paperdoll\" {\n    slot priority=500\n}\n"),
            "WearableRules.kdl: line 2, column 5: `slot` takes 1 arguments, found 0"
        );
    }

    #[test]
    fn paperdoll_profiles_reject_unknown_properties_and_missing_values() {
        assert_eq!(
            profiles_error("profile \"human_male\" equipment_offset=50000 canvas_w=260\n"),
            "PaperdollProfiles.kdl: line 1, column 45: unknown property `canvas_w` of `profile`: expected one of \
             `label`, `equipment_offset`, `canvas_width`, `canvas_height`, `body_x`, `body_y`, `equipment_x`, \
             `equipment_y`"
        );
        assert_eq!(
            profiles_error("profiles \"human_male\" equipment_offset=50000\n"),
            "PaperdollProfiles.kdl: line 1, column 1: unknown node `profiles` at the top level: expected `profile`"
        );
        let missing = profiles_error("profile \"human_male\" label=\"Human Male\"\n");
        assert!(missing.starts_with("PaperdollProfiles.kdl: invalid KDL: line 1, column 1:"), "{missing}");
        assert!(missing.contains("equipment_offset"), "{missing}");
    }

    /// The paperdoll goes up to egui as composed: half-transparent white over the empty canvas is premultiplied, as
    /// bevy_egui blends it, and a transparent texel adds nothing, whatever its colour.
    #[test]
    fn paperdoll_composite_is_premultiplied() {
        const HALF_WHITE: [u8; 4] = [255, 255, 255, 128];
        const PREMULTIPLIED_HALF_WHITE: [u8; 4] = [128, 128, 128, 128];
        const TRANSPARENT_PINK: [u8; 4] = [255, 0, 255, 0];
        let mut canvas = vec![0u8; 8];
        let layer = [HALF_WHITE, TRANSPARENT_PINK].concat();
        alpha_blend_at(&mut canvas, 2, 0, 0, 2, 1, &layer);
        assert_eq!(canvas, [PREMULTIPLIED_HALF_WHITE, [0; 4]].concat());
    }

    const TRANSPARENT_BLACK: [u8; 4] = [0, 0, 0, 0];
    const OPAQUE_BLACK: [u8; 4] = [0, 0, 0, 255];

    fn test_gump(width: u32, height: u32, texels: &[[u8; 4]]) -> udd_assets::gumps::GumpImage {
        udd_assets::gumps::GumpImage::whole(width, height, 1, texels.concat())
    }

    fn test_layer(gump: udd_assets::gumps::GumpImage, hue: Option<LayerHue>) -> DecodedPaperdollLayer {
        DecodedPaperdollLayer { x: 0, y: 0, gump, hue }
    }

    /// `gump` stored as an atlas page stores it: only the texels of `logical_rect`, as the packer trims it.
    fn trimmed(gump: &udd_assets::gumps::GumpImage, logical_rect: udd_assets::gumps::GumpRect) -> udd_assets::gumps::GumpImage {
        let factor = u32::from(gump.upscale_factor);
        let stored_rect = udd_assets::gumps::GumpRect {
            x: logical_rect.x * factor,
            y: logical_rect.y * factor,
            width: logical_rect.width * factor,
            height: logical_rect.height * factor,
        };
        let stride = gump.physical_width as usize * RGBA_BYTES;
        let stored_rgba = gump
            .stored_rgba
            .chunks_exact(stride)
            .skip(stored_rect.y as usize)
            .take(stored_rect.height as usize)
            .flat_map(|row| &row[stored_rect.x as usize * RGBA_BYTES..(stored_rect.x + stored_rect.width) as usize * RGBA_BYTES])
            .copied()
            .collect();
        udd_assets::gumps::GumpImage { stored_rect, stored_rgba, ..gump.clone() }
    }

    /// Both clients blend the layers by alpha: the packing-time bake left an Enhanced layer's drawn texels opaque and the
    /// rest transparent, so a transparent texel leaves the body showing and an opaque one, black too, covers it.
    #[test]
    fn baked_layers_blend_by_alpha_in_both_clients() {
        const BODY: [u8; 4] = [90, 60, 40, 255];
        const ITEM: [u8; 4] = [30, 120, 200, 255];
        let layers = || {
            vec![
                test_layer(test_gump(4, 1, &[TRANSPARENT_BLACK, BODY, BODY, BODY]), None),
                test_layer(test_gump(4, 1, &[OPAQUE_BLACK, TRANSPARENT_BLACK, ITEM, OPAQUE_BLACK]), None),
            ]
        };
        for client in [GumpClient::Enhanced, GumpClient::Classic] {
            let composed = compose_decoded_layers(client, 4, 1, layers());
            assert_eq!(composed.stored_rgba, [OPAQUE_BLACK, BODY, ITEM, OPAQUE_BLACK].concat(), "{client:?}");
        }
    }

    /// A trimmed layer goes back at its offset in its canvas: composing trimmed layers draws what their whole canvases
    /// draw, and the canvas keeps the size the whole layers give it. Here a 2x upscaled item, trimmed to logical 1,1..3,2
    /// of its 4x3 canvas, over a 1x body trimmed to its middle texel.
    #[test]
    fn trimmed_layers_compose_at_their_offsets() {
        const BODY: [u8; 4] = [90, 60, 40, 255];
        const ITEM: [u8; 4] = [30, 120, 200, 255];
        const SOFT_ITEM: [u8; 4] = [200, 120, 30, 128];
        use udd_assets::gumps::GumpRect;
        let body = test_gump(3, 3, &[
            TRANSPARENT_BLACK, TRANSPARENT_BLACK, TRANSPARENT_BLACK,
            TRANSPARENT_BLACK, BODY, TRANSPARENT_BLACK,
            TRANSPARENT_BLACK, TRANSPARENT_BLACK, TRANSPARENT_BLACK,
        ]);
        const ITEM_FACTOR: u16 = 2;
        let (item_width, item_height) = (4 * u32::from(ITEM_FACTOR), 3 * u32::from(ITEM_FACTOR));
        let mut item_texels = vec![TRANSPARENT_BLACK; (item_width * item_height) as usize];
        for (x, y, texel) in [(2, 2, ITEM), (5, 3, SOFT_ITEM), (3, 2, ITEM)] {
            item_texels[y * item_width as usize + x] = texel;
        }
        let item = udd_assets::gumps::GumpImage::whole(item_width, item_height, ITEM_FACTOR, item_texels.concat());
        let whole = || vec![test_layer(body.clone(), None), test_layer(item.clone(), None)];
        let parts = || {
            vec![
                test_layer(trimmed(&body, GumpRect { x: 1, y: 1, width: 1, height: 1 }), None),
                test_layer(trimmed(&item, GumpRect { x: 1, y: 1, width: 2, height: 1 }), None),
            ]
        };

        for client in [GumpClient::Enhanced, GumpClient::Classic] {
            let expected = compose_decoded_layers(client, 2, 2, whole());
            let composed = compose_decoded_layers(client, 2, 2, parts());
            assert_eq!((composed.logical_width, composed.logical_height), (4, 3), "{client:?}");
            assert_eq!(composed.upscale_factor, ITEM_FACTOR);
            assert_eq!(composed, expected, "{client:?}");
            let texel = |x: usize, y: usize| &composed.stored_rgba[(y * item_width as usize + x) * RGBA_BYTES..][..RGBA_BYTES];
            assert_eq!(texel(2, 2), ITEM, "{client:?}");
            assert_eq!(texel(2, 3), BODY, "the body under the item's clear texel, {client:?}");
        }
    }

    /// Enhanced draws no 256x256 gump, which then neither covers nor grows the canvas; Classic draws it.
    #[test]
    fn enhanced_compose_skips_256x256_gumps() {
        const RED: [u8; 4] = [200, 0, 0, 255];
        const WHITE: [u8; 4] = [255, 255, 255, 255];
        const SIZE: u32 = ENHANCED_SKIPPED_GUMP_SIZE;
        let small = || test_gump(2, 1, &[TRANSPARENT_BLACK, RED]);
        let square = || {
            let mut texels = vec![WHITE; (SIZE * SIZE) as usize];
            texels[0] = TRANSPARENT_BLACK;
            test_gump(SIZE, SIZE, &texels)
        };
        let layers = || vec![test_layer(small(), None), test_layer(square(), None)];

        let enhanced = compose_decoded_layers(GumpClient::Enhanced, 2, 1, layers());
        assert_eq!((enhanced.logical_width, enhanced.logical_height), (2, 1));
        assert_eq!(enhanced.stored_rgba, [TRANSPARENT_BLACK, RED].concat());

        let classic = compose_decoded_layers(GumpClient::Classic, 2, 1, layers());
        assert_eq!((classic.logical_width, classic.logical_height), (SIZE, SIZE));
        assert_eq!(classic.stored_rgba[4..8], WHITE);

        let wide = test_gump(SIZE, 1, &vec![WHITE; SIZE as usize]);
        let drawn = compose_decoded_layers(GumpClient::Enhanced, 2, 1, vec![test_layer(wide, None)]);
        assert_eq!((drawn.logical_width, drawn.logical_height), (SIZE, 1));
    }

    /// Both clients hue by the red byte, and a partial hue only exact greys; a transparent texel stays as it is, the
    /// bake having left every texel the Enhanced client draws opaque.
    #[test]
    fn hue_follows_the_red_byte_on_exact_greys() {
        const GREY_TRANSPARENT: [u8; 4] = [100, 100, 100, 0];
        const GREY: [u8; 4] = [100, 100, 100, 255];
        const NOT_GREY: [u8; 4] = [100, 101, 100, 255];
        let table: HueTable = std::array::from_fn(|red| [red as u8, 1, u8::MAX - red as u8, u8::MAX]);
        let hued = [100, 1, 155, 255];
        let layer = [TRANSPARENT_BLACK, GREY_TRANSPARENT, GREY, NOT_GREY].concat();
        for client in [GumpClient::Enhanced, GumpClient::Classic] {
            for (partial, expected) in [(true, NOT_GREY), (false, hued)] {
                let mut pixels = layer.clone();
                apply_hue(&mut pixels, &LayerHue { table, partial }, client);
                assert_eq!(
                    pixels,
                    [TRANSPARENT_BLACK, GREY_TRANSPARENT, hued, expected].concat(),
                    "{client:?}, partial {partial}"
                );
            }
        }
    }

    /// The Enhanced client blends a hue entry whose alpha is below 255 over the canvas; Classic writes its colour.
    #[test]
    fn enhanced_hue_entry_alpha_blends_over_the_canvas() {
        const BODY: [u8; 4] = [0, 0, 200, 255];
        const GREY: [u8; 4] = [100, 100, 100, 255];
        const HALF: u8 = 128;
        let table: HueTable = std::array::from_fn(|_| [200, 0, 0, HALF]);
        let layers = || {
            vec![
                test_layer(test_gump(1, 1, &[BODY]), None),
                test_layer(test_gump(1, 1, &[GREY]), Some(LayerHue { table, partial: true })),
            ]
        };
        let enhanced = compose_decoded_layers(GumpClient::Enhanced, 1, 1, layers());
        assert_eq!(enhanced.stored_rgba, [100, 0, 99, 255]);
        let classic = compose_decoded_layers(GumpClient::Classic, 1, 1, layers());
        assert_eq!(classic.stored_rgba, [200, 0, 0, 255]);
    }

    /// Each mip level averages 2x2 texels of the one above, down to 1x1; an odd edge's last column is left out.
    #[test]
    fn gump_images_carry_their_mip_levels() {
        const WHITE: [u8; 4] = [255, 255, 255, 255];
        const RED: [u8; 4] = [255, 0, 0, 255];
        let level0 = [WHITE, TRANSPARENT_BLACK, RED, WHITE, TRANSPARENT_BLACK, RED].concat();
        let (data, levels) = with_mip_levels(3, 2, &level0);
        assert_eq!(levels, 2);
        assert_eq!(data, [level0, vec![128, 128, 128, 128]].concat());

        let (data, levels) = with_mip_levels(4, 4, &WHITE.repeat(16));
        assert_eq!(levels, 3);
        assert_eq!(data, WHITE.repeat(16 + 4 + 1));
        let image = image_from_gump(&test_gump(4, 4, &[WHITE; 16]));
        assert_eq!(image.texture_descriptor.mip_level_count, 3);
    }
}

#[derive(Clone)]
struct WearableRule {
    target_slot: String,
    target_items: WearableItemMatcher,
    overrides: Vec<WearableOverrideRule>,
}

#[derive(Clone)]
struct WearableOverrideRule {
    draw_order_priority: u16,
    race: Option<String>,
    direction: Option<String>,
    conditions: Vec<WearableCondition>,
}

impl WearableOverrideRule {
    fn matches_equipped(&self, equipped: &[PaperdollEquippedItem]) -> bool {
        self.conditions.iter().all(|condition| {
            equipped.iter().any(|item| {
                item.slot == condition.slot && condition.items.matches(item.item_id)
            })
        })
    }
}

#[derive(Clone)]
struct WearableCondition {
    slot: String,
    items: WearableItemMatcher,
}

#[derive(Clone)]
enum WearableItemMatcher {
    Any,
    Ids(Vec<u32>),
}

impl WearableItemMatcher {
    fn matches(&self, item_id: u32) -> bool {
        match self {
            Self::Any => true,
            Self::Ids(ids) => ids.contains(&item_id),
        }
    }
}

#[derive(Clone)]
struct PaperdollEquippedItem {
    item_id: u32,
    slot: String,
    gump_id: u32,
    hue_id: u16,
    partial_hue: bool,
    fallback_priority: u16,
}

impl PaperdollProfilesRes {
    /// The bundled profiles are required: a missing or invalid file stops the app.
    fn load() -> Self {
        let path = constants::valid_asset_dir().join(PAPERDOLL_SPECS_DIR).join(PAPERDOLL_PROFILES_FILE_NAME);
        Self::load_from_path(&path)
            .unwrap_or_else(|error| crate::configs::settings::exit_on_startup_config_error(&error))
    }

    fn load_from_path(path: &Path) -> color_eyre::eyre::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|error| color_eyre::eyre::eyre!("{}: cannot read: {error}", path.display()))?;
        Self::load_from_str(&path.display().to_string(), &content)
    }

    fn load_from_str(source_name: &str, content: &str) -> color_eyre::eyre::Result<Self> {
        let decoded: PaperdollProfilesKdl = uot_kdl::strict::decode(source_name, content, PAPERDOLL_PROFILES_SCHEMA)?;
        let profiles = decoded
            .profiles
            .into_iter()
            .map(PaperdollProfile::from)
            .collect::<Vec<_>>();
        if profiles.is_empty() {
            color_eyre::eyre::bail!("{source_name}: contains no profile nodes");
        }

        Ok(Self { profiles })
    }

    fn profile(&self, id: &str) -> Option<&PaperdollProfile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    fn default_profile(&self) -> &PaperdollProfile {
        &self.profiles[0]
    }
}

#[derive(Clone)]
struct PaperdollProfile {
    id: String,
    label: String,
    equipment_offset: u32,
    canvas_width: u32,
    canvas_height: u32,
    body_x: i32,
    body_y: i32,
    equipment_x: i32,
    equipment_y: i32,
}

#[derive(Decode)]
struct PaperdollProfilesKdl {
    #[knuffel(children(name = "profile"))]
    profiles: Vec<PaperdollProfileKdl>,
}

#[derive(Decode)]
struct PaperdollProfileKdl {
    #[knuffel(argument)]
    id: String,
    #[knuffel(property)]
    label: Option<String>,
    #[knuffel(property(name = "equipment_offset"))]
    equipment_offset: u32,
    #[knuffel(property(name = "canvas_width"))]
    canvas_width: Option<u32>,
    #[knuffel(property(name = "canvas_height"))]
    canvas_height: Option<u32>,
    #[knuffel(property(name = "body_x"))]
    body_x: Option<i32>,
    #[knuffel(property(name = "body_y"))]
    body_y: Option<i32>,
    #[knuffel(property(name = "equipment_x"))]
    equipment_x: Option<i32>,
    #[knuffel(property(name = "equipment_y"))]
    equipment_y: Option<i32>,
}

impl From<PaperdollProfileKdl> for PaperdollProfile {
    fn from(value: PaperdollProfileKdl) -> Self {
        Self {
            label: value.label.unwrap_or_else(|| value.id.clone()),
            id: value.id,
            equipment_offset: value.equipment_offset,
            canvas_width: value.canvas_width.unwrap_or(0),
            canvas_height: value.canvas_height.unwrap_or(0),
            body_x: value.body_x.unwrap_or(0),
            body_y: value.body_y.unwrap_or(0),
            equipment_x: value.equipment_x.unwrap_or(0),
            equipment_y: value.equipment_y.unwrap_or(0),
        }
    }
}

/// `any`, or a comma-separated list of item ids (decimal or `0x` hex).
fn parse_wearable_items(value: &str) -> Result<WearableItemMatcher, String> {
    if value.trim().eq_ignore_ascii_case("any") {
        return Ok(WearableItemMatcher::Any);
    }

    let ids = value
        .split(',')
        .map(|part| {
            parse_u32_field(part.trim())
                .map_err(|_| format!("items: `{}` is not an item id; expected `any` or ids like 0x1F03,7939", part.trim()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WearableItemMatcher::Ids(ids))
}

fn paperdoll_slot_from_tiledata_layer(layer: u8) -> &'static str {
    match layer {
        1 => "righthand",
        2 => "lefthand",
        3 => "feet",
        4 => "legs",
        5 => "chest",
        6 => "head",
        7 => "hands",
        8 => "finger1",
        10 => "neck",
        11 => "hair",
        12 => "waist",
        13 => "torso",
        14 => "lwrist",
        16 => "facialhair",
        17 => "abovechest",
        18 => "ears",
        19 => "arms",
        20 => "cape",
        22 => "skirt",
        23 => "legs",
        24 => "riding",
        25 => "drag",
        26 => "backpack",
        _ => "drag",
    }
}

fn race_from_profile_id(profile_id: &str) -> &'static str {
    if profile_id.contains("elf") {
        "elf"
    } else if profile_id.contains("gargoyle") {
        "gargoyle"
    } else {
        "human"
    }
}

pub struct GumpDialogPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(GumpDialogPlugin);

impl Plugin for GumpDialogPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.insert_resource(PaperdollProfilesRes::load())
            .init_resource::<PaperdollWearableRulesRes>()
            .init_resource::<GumpDialogState>()
            .add_observer(sys_gump_toggle)
            .add_observer(sys_gump_close)
            .add_systems(Startup, sys_load_paperdoll_wearable_rules)
            .add_systems(
                EguiPrimaryContextPass,
                sys_render_gump_dialog.run_if(in_state(AppState::InGame)),
            );
    }
}

/// The bundled wearable rules are required: a missing or invalid file stops the app.
fn sys_load_paperdoll_wearable_rules(
    mut rules_res: ResMut<PaperdollWearableRulesRes>,
) {
    let path = constants::valid_asset_dir().join(PAPERDOLL_SPECS_DIR).join(WEARABLE_RULES_FILE_NAME);

    match PaperdollWearableRules::load_from_path(&path) {
        Ok(rules) => {
            log::info!("Loaded paperdoll wearable rules from {}", path.display());
            rules_res.rules = Some(rules);
        }
        Err(error) => crate::configs::settings::exit_on_startup_config_error(&error),
    }
}

fn sys_gump_toggle(
    _trigger: On<ActionToggleGumpDialog>,
    settings: Res<crate::configs::settings::Settings>,
    mut state: ResMut<GumpDialogState>,
) {
    log_system_add_one_shot::<GumpDialogPlugin>("Observer", "ActionToggleGumpDialog", fname!());
    if !settings.runtime_assets.load_gumps {
        ingame_sysmessage_logger::error(
            "Gump art is disabled. Set runtime_assets.load_gumps=true and restart to use gumps."
                .to_string(),
        );
        state.open = false;
        return;
    }

    state.open = !state.open;
}

fn sys_gump_close(
    _trigger: On<ActionCloseActiveDialog>,
    mut state: ResMut<GumpDialogState>,
) {
    log_system_add_one_shot::<GumpDialogPlugin>("Observer", "ActionCloseActiveDialog", fname!());
    state.open = false;
}

fn sys_render_gump_dialog(
    mut state: ResMut<GumpDialogState>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    gump_map: Option<Res<GumpMapRes>>,
    tilemeta: Option<Res<TileMetaPackageRes>>,
    hues_package: Option<Res<HuesPackageRes>>,
    paperdoll_profiles: Res<PaperdollProfilesRes>,
    wearable_rules: Res<PaperdollWearableRulesRes>,
    uniform_state: Res<UniformState>,
    mut settings: ResMut<Settings>,
    mut images: ResMut<Assets<Image>>,
) {
    if !state.open && state.open_gumps.is_empty() {
        return;
    }

    let client = GumpClient::from_shading_mode(uniform_state.effects.shading_mode);
    if let Some(gump_map) = gump_map.as_deref() {
        redraw_open_gumps_for_client(&mut state, client, gump_map, hues_package.as_deref(), &mut images);
    }

    let mut closed_images = Vec::new();
    let mut gump_background = settings.app.window.gump_background;
    let open_request = {
        let Some(ctx) =
            dialogs::get_egui_context_ready_mut(&mut egui_contexts, &egui_ui_camera) else {
            return;
        };

        let open_request = if state.open {
            render_open_gump_dialog(ctx, &mut state, &paperdoll_profiles, &mut gump_background)
        } else {
            None
        };

        for open_gump in &state.open_gumps {
            let mut open = true;
            let mut right_clicked = false;
            let image_size = egui::vec2(open_gump.width as f32, open_gump.height as f32);
            egui::Window::new(open_gump.title.as_str())
                .default_pos([
                    120.0 + (state.open_gumps.len() as f32 * 12.0),
                    120.0 + (state.open_gumps.len() as f32 * 12.0),
                ])
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    if let Some(error) = &open_gump.error {
                        ui.label(error.as_str());
                        return;
                    }
                    let response = ui.add(gump_background.under(egui::Image::new((open_gump.texture_id, image_size))));
                    if response.secondary_clicked() {
                        right_clicked = true;
                    }
                });

            if !open || right_clicked {
                closed_images.push(open_gump.image.id());
            }
        }

        open_request
    };
    // Written only when picked: other systems watch the settings for changes.
    if gump_background != settings.app.window.gump_background {
        settings.app.window.gump_background = gump_background;
    }

    state.open_gumps.retain(|open_gump| {
        let close = closed_images.contains(&open_gump.image.id());
        if close {
            images.remove(open_gump.image.id());
        }
        !close
    });

    for image_id in closed_images {
        egui_contexts.remove_image(image_id);
    }

    match open_request {
        Some(GumpOpenRequest::GumpById) => open_gump_by_id(
            &mut state,
            client,
            gump_map.as_deref(),
            &mut images,
            &mut egui_contexts,
        ),
        Some(GumpOpenRequest::Paperdoll) => open_paperdoll(
            &mut state,
            client,
            gump_map.as_deref(),
            tilemeta.as_deref(),
            hues_package.as_deref(),
            &paperdoll_profiles,
            &wearable_rules,
            &mut images,
            &mut egui_contexts,
        ),
        None => {}
    }
}

/// After a preset change, every open gump window is redrawn from the package the new preset reads, into its own image
/// (the egui texture id stays valid). No image is cached by gump id, so none can keep the other package's pixels.
fn redraw_open_gumps_for_client(
    state: &mut GumpDialogState,
    client: GumpClient,
    gump_map: &GumpMapRes,
    hues_package: Option<&HuesPackageRes>,
    images: &mut Assets<Image>,
) {
    for open_gump in state.open_gumps.iter_mut().filter(|open_gump| open_gump.client != client) {
        open_gump.client = client;
        open_gump.error = match draw_open_gump_content(&open_gump.content, client, gump_map, hues_package) {
            Ok(gump) => {
                (open_gump.width, open_gump.height) = logical_window_size(&gump);
                images
                    .insert(open_gump.image.id(), image_from_gump(&gump))
                    .err()
                    .map(|error| format!("Could not replace the gump image: {error}"))
            }
            Err(error) => Some(format!("No image under this preset: {error}")),
        };
    }
}

/// Decodes a single gump, or composes a paperdoll, for the preset drawing `client`, whole and premultiplied as egui
/// blends it.
fn draw_open_gump_content(
    content: &OpenGumpContent,
    client: GumpClient,
    gump_map: &GumpMapRes,
    hues_package: Option<&HuesPackageRes>,
) -> color_eyre::eyre::Result<udd_assets::gumps::GumpImage> {
    match content {
        OpenGumpContent::Gump(id) => {
            // A paperdoll gump is stored trimmed to its drawn texels: shown on its own, it gets its whole canvas back,
            // transparent around them.
            let mut gump = gump_map.decode_gump(client, *id, &mut Vec::new())?.into_whole();
            // bevy_egui blends user textures as premultiplied colour (`BlendState::PREMULTIPLIED_ALPHA_BLENDING`, and its
            // shader doesn't multiply by alpha), so the gump's straight colour goes up premultiplied: a transparent texel then
            // adds nothing, whatever colour it keeps, as in the clients.
            udd_assets::premultiplied::premultiply_rgba8888(&mut gump.stored_rgba);
            Ok(gump)
        }
        // Premultiplied already, as bevy_egui blends it: `alpha_blend_at` composes the layers over a transparent canvas.
        OpenGumpContent::Paperdoll { profile, layers } => {
            compose_paperdoll(gump_map, client, hues_package, profile, layers)
        }
    }
}

/// The image of a drawn gump or paperdoll (`draw_open_gump_content`). The window shows it at its logical size, so an
/// upscaled gump is shrunk: it is sampled linearly through mip levels when shrunk, and nearest when a texel covers more
/// than a pixel, which keeps 1x gumps sharp at 100% and above.
fn image_from_gump(gump: &udd_assets::gumps::GumpImage) -> Image {
    let canvas = gump.canvas_rgba();
    if gump.physical_width == 0 || gump.physical_height == 0 {
        let mut image = crate::util_lib::image::image_from_rgba8(gump.physical_width, gump.physical_height, &canvas);
        image.sampler = bevy::image::ImageSampler::nearest();
        return image;
    }
    let (data, mip_level_count) = with_mip_levels(gump.physical_width, gump.physical_height, &canvas);
    let mut image = Image::new_uninit(
        bevy::render::render_resource::Extent3d {
            width: gump.physical_width,
            height: gump.physical_height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mip_level_count;
    image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        mag_filter: bevy::image::ImageFilterMode::Nearest,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// A premultiplied RGBA8 image followed by its mip levels down to 1x1, and how many levels that makes. Each level
/// averages 2x2 texels of the one above; an odd edge's last row or column is left out, as GPUs halve sizes.
fn with_mip_levels(width: u32, height: u32, rgba: &[u8]) -> (Vec<u8>, u32) {
    let mut data = rgba.to_vec();
    let mut level_count = 1;
    let (mut level_width, mut level_height) = (width as usize, height as usize);
    let mut level_start = 0;
    while level_width > 1 || level_height > 1 {
        let (next_width, next_height) = ((level_width / 2).max(1), (level_height / 2).max(1));
        let mut next = Vec::with_capacity(next_width * next_height * RGBA_BYTES);
        let level = &data[level_start..level_start + level_width * level_height * RGBA_BYTES];
        for y in 0..next_height {
            let rows = [(2 * y).min(level_height - 1), (2 * y + 1).min(level_height - 1)];
            for x in 0..next_width {
                let columns = [(2 * x).min(level_width - 1), (2 * x + 1).min(level_width - 1)];
                for channel in 0..RGBA_BYTES {
                    let sum: u16 = rows
                        .iter()
                        .flat_map(|&row| columns.iter().map(move |&column| (row, column)))
                        .map(|(row, column)| u16::from(level[(row * level_width + column) * RGBA_BYTES + channel]))
                        .sum();
                    next.push(((sum + MIP_BOX_TEXELS / 2) / MIP_BOX_TEXELS) as u8);
                }
            }
        }
        level_start = data.len();
        data.extend_from_slice(&next);
        (level_width, level_height) = (next_width, next_height);
        level_count += 1;
    }
    (data, level_count)
}

fn logical_window_size(gump: &udd_assets::gumps::GumpImage) -> (u16, u16) {
    (
        gump.logical_width.min(u16::MAX as u32) as u16,
        gump.logical_height.min(u16::MAX as u32) as u16,
    )
}

/// Draws `content` for the preset drawing `client` and opens a window showing it.
fn open_gump_window(
    state: &mut GumpDialogState,
    title: String,
    content: OpenGumpContent,
    client: GumpClient,
    gump_map: &GumpMapRes,
    hues_package: Option<&HuesPackageRes>,
    images: &mut Assets<Image>,
    egui_contexts: &mut EguiContexts,
) -> color_eyre::eyre::Result<()> {
    let gump = draw_open_gump_content(&content, client, gump_map, hues_package)?;
    let image = images.add(image_from_gump(&gump));
    let texture_id = egui_contexts.add_image(EguiTextureHandle::Strong(image.clone()));
    let (width, height) = logical_window_size(&gump);
    state.open_gumps.push(OpenGump {
        title,
        width,
        height,
        image,
        texture_id,
        content,
        client,
        error: None,
    });
    Ok(())
}

fn render_open_gump_dialog(
    ctx: &mut egui::Context,
    state: &mut GumpDialogState,
    paperdoll_profiles: &PaperdollProfilesRes,
    gump_background: &mut ImageBackground,
) -> Option<GumpOpenRequest> {
    let mut window_open = state.open;
    let mut open_request = None;
    egui::Window::new("Open Gump")
        .default_pos([400.0, 240.0])
        .default_size([300.0, 260.0])
        .collapsible(false)
        .open(&mut window_open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("ID:");
                ui.text_edit_singleline(&mut state.id);
            });

            let submit = ui.button("Open").clicked()
                || ui.input(|input| input.key_pressed(egui::Key::Enter));
            if submit {
                open_request = Some(GumpOpenRequest::GumpById);
            }

            ui.separator();
            // Behind the image of every gump and paperdoll window.
            ui.horizontal_wrapped(|ui| {
                let window_fill = ui.visuals().window_fill;
                image_background_control(ui, gump_background, window_fill);
            });

            ui.separator();
            ui.checkbox(&mut state.paperdoll_open, "Paperdoll");
            if state.paperdoll_open {
                egui::ComboBox::from_label("Profile")
                    .selected_text(
                        paperdoll_profiles
                            .profile(&state.paperdoll_profile_id)
                            .unwrap_or_else(|| paperdoll_profiles.default_profile())
                            .label
                            .as_str(),
                    )
                    .show_ui(ui, |ui| {
                        for profile in &paperdoll_profiles.profiles {
                            ui.selectable_value(
                                &mut state.paperdoll_profile_id,
                                profile.id.clone(),
                                profile.label.as_str(),
                            );
                        }
                });
                ui.horizontal(|ui| {
                    ui.label("Body:");
                    ui.text_edit_singleline(&mut state.paperdoll_body_id);
                    ui.label("Hue:");
                    ui.text_edit_singleline(&mut state.paperdoll_body_hue);
                });

                egui::Grid::new("paperdoll_equipment_grid")
                    .num_columns(3)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Item");
                        ui.label("Hue");
                        ui.end_row();

                        for row in &mut state.paperdoll_equipment {
                            ui.text_edit_singleline(&mut row.item_id);
                            ui.text_edit_singleline(&mut row.hue);
                            ui.end_row();
                        }
                    });

                ui.horizontal(|ui| {
                    if ui.button("Add Row").clicked() {
                        state.paperdoll_equipment.push(PaperdollEquipmentRow::default());
                    }
                    if ui.button("Open Paperdoll").clicked() {
                        open_request = Some(GumpOpenRequest::Paperdoll);
                    }
                });
            }
        });

    state.open = window_open;
    open_request
}

fn open_gump_by_id(
    state: &mut GumpDialogState,
    client: GumpClient,
    gump_map: Option<&GumpMapRes>,
    images: &mut Assets<Image>,
    egui_contexts: &mut EguiContexts,
) {
    let Some(gump_map) = gump_map else {
        ingame_sysmessage_logger::error("No gump source is loaded.".to_string());
        return;
    };

    let Ok(id) = state.id.trim().parse::<u32>() else {
        ingame_sysmessage_logger::error(format!("Invalid gump ID: {}", state.id));
        return;
    };

    if let Err(error) = open_gump_window(
        state,
        format!("Gump {id}"),
        OpenGumpContent::Gump(id),
        client,
        gump_map,
        None,
        images,
        egui_contexts,
    ) {
        ingame_sysmessage_logger::error(format!("Could not open gump {id}: {error}"));
    }
}

struct PaperdollLayer {
    gump_id: u32,
    hue_id: u16,
    partial_hue: bool,
    sort_key: u16,
    x: i32,
    y: i32,
}

fn open_paperdoll(
    state: &mut GumpDialogState,
    client: GumpClient,
    gump_map: Option<&GumpMapRes>,
    tilemeta: Option<&TileMetaPackageRes>,
    hues_package: Option<&HuesPackageRes>,
    paperdoll_profiles: &PaperdollProfilesRes,
    wearable_rules: &PaperdollWearableRulesRes,
    images: &mut Assets<Image>,
    egui_contexts: &mut EguiContexts,
) {
    let Some(gump_map) = gump_map else {
        ingame_sysmessage_logger::error("No gump source is loaded.".to_string());
        return;
    };
    let Some(tilemeta) = tilemeta else {
        ingame_sysmessage_logger::error("Tile metadata is not loaded.".to_string());
        return;
    };

    let Ok(body_id) = parse_u32_field(&state.paperdoll_body_id) else {
        ingame_sysmessage_logger::error(format!(
            "Invalid paperdoll body gump ID: {}",
            state.paperdoll_body_id
        ));
        return;
    };
    let body_hue = parse_optional_hue(&state.paperdoll_body_hue);
    let profile = paperdoll_profiles
        .profile(&state.paperdoll_profile_id)
        .unwrap_or_else(|| paperdoll_profiles.default_profile());

    // The clients hue the body's greys only. Enhanced: `UOPaperdoll::BuildImage` 0x0050c790 draws it with partial hue
    // set. Classic: `DrawPaperdollGumpLayers` 0x0051a4a0 draws the bodies of its table with `BlitGumpRowPartialHue`
    // 0x005d8430, and only a body it knows by race alone with full hue; the dialog takes the gump, not the body.
    let mut layers = vec![PaperdollLayer {
        gump_id: body_id,
        hue_id: body_hue,
        partial_hue: true,
        sort_key: 0,
        x: profile.body_x,
        y: profile.body_y,
    }];

    let mut equipped = Vec::new();
    for row in &state.paperdoll_equipment {
        if row.item_id.trim().is_empty() {
            continue;
        }

        let Ok(item_id) = parse_u32_field(&row.item_id) else {
            ingame_sysmessage_logger::error(format!("Invalid equipment item ID: {}", row.item_id));
            return;
        };
        let Some(item) = tilemeta.0.item_tile(item_id) else {
            ingame_sysmessage_logger::error(format!("Unknown equipment item ID: {item_id}"));
            return;
        };
        if item.anim_id == 0 {
            ingame_sysmessage_logger::error(format!(
                "Equipment item {item_id} has no paperdoll anim_id."
            ));
            return;
        }

        let slot = paperdoll_slot_from_tiledata_layer(item.quality).to_string();
        equipped.push(PaperdollEquippedItem {
            item_id,
            slot,
            gump_id: item.anim_id as u32 + profile.equipment_offset,
            hue_id: parse_optional_hue(&row.hue),
            partial_hue: (item.flags & 0x40000) != 0,
            fallback_priority: u16::from(item.quality),
        });
    }

    let race = race_from_profile_id(&profile.id);
    for item in &equipped {
        let sort_key = wearable_rules
            .rules
            .as_ref()
            .map(|rules| {
                rules.priority_for(
                    &item.slot,
                    item.item_id,
                    &equipped,
                    race,
                    item.fallback_priority,
                )
            })
            .unwrap_or(item.fallback_priority);

        if sort_key == 0 {
            continue;
        }

        layers.push(PaperdollLayer {
            gump_id: item.gump_id,
            hue_id: item.hue_id,
            partial_hue: item.partial_hue,
            sort_key,
            x: profile.equipment_x,
            y: profile.equipment_y,
        });
    }

    layers[1..].sort_by_key(|layer| layer.sort_key);

    let title = format!("Paperdoll ({})", profile.label);
    let content = OpenGumpContent::Paperdoll {
        profile: profile.clone(),
        layers,
    };
    if let Err(error) =
        open_gump_window(state, title, content, client, gump_map, hues_package, images, egui_contexts)
    {
        ingame_sysmessage_logger::error(format!("Could not compose paperdoll: {error}"));
    }
}

fn parse_u32_field(text: &str) -> Result<u32, std::num::ParseIntError> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        text.parse::<u32>()
    }
}

fn parse_optional_hue(text: &str) -> u16 {
    parse_u32_field(text).unwrap_or(0).min(u16::MAX as u32) as u16
}

/// The bytes of an RGBA texel.
const RGBA_BYTES: usize = 4;
/// Its R, G and B bytes, all `UOPaperdoll::DrawGumpLayer` reads of a texel.
const RGB_BYTES: usize = 3;
const ALPHA_BYTE: usize = 3;
/// `UOPaperdoll::BuildImage` (UOSA 0x0050c790) skips a gump of this width and height.
const ENHANCED_SKIPPED_GUMP_SIZE: u32 = 256;
/// A hue table has an entry for each red byte.
const HUE_TABLE_ENTRIES: usize = u8::MAX as usize + 1;
/// The texels a mip level averages from the level above: 2x2.
const MIP_BOX_TEXELS: u16 = 4;

/// Each entry's R, G, B and alpha.
type HueTable = [[u8; RGBA_BYTES]; HUE_TABLE_ENTRIES];

/// Each layer's gump comes from the package the preset drawing `client` reads; the hues follow that client's drawing.
fn compose_paperdoll(
    gump_map: &GumpMapRes,
    client: GumpClient,
    hues_package: Option<&HuesPackageRes>,
    profile: &PaperdollProfile,
    layers: &[PaperdollLayer],
) -> color_eyre::eyre::Result<udd_assets::gumps::GumpImage> {
    let hue_texture_bytes = if layers.iter().any(|layer| layer.hue_id > 0) {
        let package = hues_package
            .ok_or_else(|| color_eyre::eyre::eyre!("hues.uddp is not loaded."))?;
        Some(package.0.texture_bytes())
    } else {
        None
    };
    let mut decoded_layers = Vec::with_capacity(layers.len());
    let mut scratch = Vec::new();

    for layer in layers {
        let gump = gump_map.decode_gump(client, layer.gump_id, &mut scratch)?;
        let hue = if layer.hue_id > 0 {
            Some(LayerHue {
                table: hue_table(hues_package, hue_texture_bytes, layer.hue_id)?,
                partial: layer.partial_hue,
            })
        } else {
            None
        };
        decoded_layers.push(DecodedPaperdollLayer { x: layer.x, y: layer.y, gump, hue });
    }

    Ok(compose_decoded_layers(client, profile.canvas_width, profile.canvas_height, decoded_layers))
}

/// A paperdoll layer's gump with its canvas at a logical position, and the hue it is drawn with.
struct DecodedPaperdollLayer {
    x: i32,
    y: i32,
    gump: udd_assets::gumps::GumpImage,
    hue: Option<LayerHue>,
}

struct LayerHue {
    table: HueTable,
    partial: bool,
}

/// Composes `layers` in order on a canvas covering at least `canvas_width` x `canvas_height` logical texels, as
/// `client` draws them: each layer's stored texels go at its position plus their offset in its gump's canvas (an atlas
/// gump is stored trimmed to its drawn texels, `udd_assets::gumps::GumpImage`), hued (`apply_hue`) and blended by
/// alpha. Enhanced follows `UOPaperdoll::BuildImage` (UOSA 0x0050c790) and skips a 256x256 gump; the rest of its rule,
/// which texels `UOPaperdoll::DrawGumpLayer` draws, is baked into gumps_ec.uddp's alpha at packing time
/// (`udd_conv::ec_gumps::bake_enhanced_paperdoll_rule`), so both clients blend by alpha. A layer that came from the
/// Classic source, for an id gumps_ec.uddp lacks, draws by its alpha too: its colour 0 is transparent.
fn compose_decoded_layers(
    client: GumpClient,
    canvas_width: u32,
    canvas_height: u32,
    mut layers: Vec<DecodedPaperdollLayer>,
) -> udd_assets::gumps::GumpImage {
    if client == GumpClient::Enhanced {
        layers.retain(|layer| {
            layer.gump.logical_width != ENHANCED_SKIPPED_GUMP_SIZE
                || layer.gump.logical_height != ENHANCED_SKIPPED_GUMP_SIZE
        });
    }
    let mut min_x = 0i32;
    let mut min_y = 0i32;
    let mut max_x = canvas_width as i32;
    let mut max_y = canvas_height as i32;
    for layer in &layers {
        min_x = min_x.min(layer.x);
        min_y = min_y.min(layer.y);
        max_x = max_x.max(layer.x + layer.gump.logical_width as i32);
        max_y = max_y.max(layer.y + layer.gump.logical_height as i32);
    }

    let logical_width = (max_x - min_x).max(1) as u32;
    let logical_height = (max_y - min_y).max(1) as u32;
    let output_scale = layers
        .iter()
        .map(|layer| u32::from(layer.gump.upscale_factor.max(1)))
        .max()
        .unwrap_or(1);
    let width = logical_width * output_scale;
    let height = logical_height * output_scale;
    let mut canvas = vec![0u8; width as usize * height as usize * RGBA_BYTES];
    for DecodedPaperdollLayer { x, y, mut gump, hue } in layers {
        if let Some(hue) = &hue {
            apply_hue(&mut gump.stored_rgba, hue, client);
        }
        let stored = gump.logical_stored_rect();
        let layer_width = stored.width * output_scale;
        let layer_height = stored.height * output_scale;
        let pixels = if gump.stored_rect.width == layer_width && gump.stored_rect.height == layer_height {
            gump.stored_rgba
        } else {
            resize_rgba_nearest(
                gump.stored_rect.width,
                gump.stored_rect.height,
                &gump.stored_rgba,
                layer_width,
                layer_height,
            )
        };
        let dst_x = (x + stored.x as i32 - min_x) * output_scale as i32;
        let dst_y = (y + stored.y as i32 - min_y) * output_scale as i32;
        alpha_blend_at(&mut canvas, width, dst_x, dst_y, layer_width, layer_height, &pixels);
    }

    udd_assets::gumps::GumpImage::whole(width, height, output_scale.min(u32::from(u16::MAX)) as u16, canvas)
}

fn resize_rgba_nearest(
    src_width: u32,
    src_height: u32,
    src: &[u8],
    dst_width: u32,
    dst_height: u32,
) -> Vec<u8> {
    if src_width == 0 || src_height == 0 || dst_width == 0 || dst_height == 0 {
        return Vec::new();
    }

    let mut dst = vec![0u8; dst_width as usize * dst_height as usize * 4];
    for y in 0..dst_height {
        let src_y = (y as u64 * src_height as u64 / dst_height as u64) as usize;
        for x in 0..dst_width {
            let src_x = (x as u64 * src_width as u64 / dst_width as u64) as usize;
            let src_i = (src_y * src_width as usize + src_x) * 4;
            let dst_i = (y as usize * dst_width as usize + x as usize) * 4;
            dst[dst_i..dst_i + 4].copy_from_slice(&src[src_i..src_i + 4]);
        }
    }
    dst
}

/// The colour of a hue for each red byte (`apply_hue`): `UOHues::GetHueTable` of the Enhanced client.
fn hue_table(
    hues_package: Option<&HuesPackageRes>,
    hue_texture_bytes: Option<&[u8]>,
    hue_id: u16,
) -> color_eyre::eyre::Result<HueTable> {
    let package = hues_package
        .ok_or_else(|| color_eyre::eyre::eyre!("hues.uddp is not loaded."))?;
    package
        .0
        .texture_coord_for_hue(hue_id)
        .ok_or_else(|| color_eyre::eyre::eyre!("Hue {hue_id} is not loaded."))?;
    let hue_texture_bytes = hue_texture_bytes
        .ok_or_else(|| color_eyre::eyre::eyre!("hues.uddp texture is not loaded."))?;

    let mut table = [[0; RGBA_BYTES]; HUE_TABLE_ENTRIES];
    for (intensity, entry) in table.iter_mut().enumerate() {
        *entry = sample_hue_lookup(hue_texture_bytes, hue_id, intensity as u8)
            .ok_or_else(|| color_eyre::eyre::eyre!("Hue {hue_id} lookup is outside hues.uddp."))?;
    }
    Ok(table)
}

/// Whether a texel takes the hue: every texel under a full hue, only an exact grey under a partial one (in the
/// Enhanced client the `CMP`s of `UOPaperdoll::DrawGumpLayer` at 0x00509d64 and 0x00509d6c).
fn is_hued(rgb: &[u8], partial_hue: bool) -> bool {
    !partial_hue || (rgb[0] == rgb[1] && rgb[0] == rgb[2])
}

/// Hues a layer as `client` draws it: the texels that are not transparent and take the hue (`is_hued`) become the
/// entry of their red. The Enhanced client blends an entry whose alpha is below 255 over the target
/// (`UOPaperdoll::DrawGumpLayer`, 0x00509ded..0x00509e51), so there the entry's alpha scales the texel's, which the
/// packing-time bake left opaque, and `alpha_blend_at` blends it; every client table is opaque
/// (`uocf::enhanced::hues`), so in practice the texel keeps its alpha. Classic writes the entry's colour.
fn apply_hue(pixels: &mut [u8], hue: &LayerHue, client: GumpClient) {
    for pixel in pixels.chunks_exact_mut(RGBA_BYTES) {
        if pixel[ALPHA_BYTE] == 0 {
            continue;
        }
        if !is_hued(pixel, hue.partial) {
            continue;
        }

        // Both clients index the hue by the red channel, not a luminance: the Enhanced client by the red byte
        // (`UOPaperdoll::DrawGumpLayer` 0x00509b30, at 0x00509d9e), Classic by the 5-bit red, `colour >> 10`
        // (`BlitGumpRowPartialHue` 0x005d8430, `BlitGumpRowFullHue` 0x005d83d0). Classic red expands to `red << 3`
        // and the hue strip holds hues.mul entry i at x = 8i, so the red byte reaches the same entry in both.
        let entry = hue.table[usize::from(pixel[0])];
        pixel[..RGB_BYTES].copy_from_slice(&entry[..RGB_BYTES]);
        if client == GumpClient::Enhanced {
            pixel[ALPHA_BYTE] =
                (u16::from(pixel[ALPHA_BYTE]) * u16::from(entry[ALPHA_BYTE]) / u16::from(u8::MAX)) as u8;
        }
    }
}

fn sample_hue_lookup(texture_bytes: &[u8], hue_id: u16, intensity: u8) -> Option<[u8; 4]> {
    let coord = uocf::enhanced::hues::atlas_coord_for_hue(hue_id)?;
    let x = coord.x + u32::from(intensity).min(udd_assets::hues::HUE_STRIP_WIDTH - 1);
    let y = coord.y;
    if x >= udd_assets::hues::HUES_TEXTURE_WIDTH || y >= udd_assets::hues::HUES_TEXTURE_HEIGHT {
        return None;
    }

    let offset = ((y * udd_assets::hues::HUES_TEXTURE_WIDTH + x) * 4) as usize;
    let color = texture_bytes.get(offset..offset.checked_add(4)?)?;
    Some([color[0], color[1], color[2], color[3]])
}

fn alpha_blend_at(
    canvas: &mut [u8],
    canvas_width: u32,
    dst_x: i32,
    dst_y: i32,
    layer_width: u32,
    layer_height: u32,
    layer: &[u8],
) {
    for y in 0..layer_height as usize {
        for x in 0..layer_width as usize {
            let canvas_x = x as i32 + dst_x;
            let canvas_y = y as i32 + dst_y;
            if canvas_x < 0 || canvas_y < 0 {
                continue;
            }

            let src_i = (y * layer_width as usize + x) * 4;
            let src_a = layer[src_i + 3] as u16;
            if src_a == 0 {
                continue;
            }

            let dst_i = (canvas_y as usize * canvas_width as usize + canvas_x as usize) * 4;
            if dst_i + 4 > canvas.len() {
                continue;
            }

            if src_a == 255 {
                canvas[dst_i..dst_i + 4].copy_from_slice(&layer[src_i..src_i + 4]);
                continue;
            }

            let inv_a = 255 - src_a;
            canvas[dst_i] =
                ((layer[src_i] as u16 * src_a + canvas[dst_i] as u16 * inv_a) / 255) as u8;
            canvas[dst_i + 1] =
                ((layer[src_i + 1] as u16 * src_a + canvas[dst_i + 1] as u16 * inv_a) / 255) as u8;
            canvas[dst_i + 2] =
                ((layer[src_i + 2] as u16 * src_a + canvas[dst_i + 2] as u16 * inv_a) / 255) as u8;
            canvas[dst_i + 3] = (src_a + (canvas[dst_i + 3] as u16 * inv_a) / 255) as u8;
        }
    }
}
