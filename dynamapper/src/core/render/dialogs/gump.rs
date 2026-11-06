use crate::core::controls::input_actions::{ActionCloseActiveDialog, ActionToggleGumpDialog};
use crate::core::render::scene::camera::UiCameraResource;
use crate::core::uo_files_loader::{GumpMapRes, HuesPackageRes, TileMetaPackageRes};
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
const WEARABLE_RULES_SCHEMA: &[udd_kdl::strict::NodeSchema] = &[
    udd_kdl::strict::NodeSchema {
        name: "draworder",
        properties: &[],
        children: &[udd_kdl::strict::NodeSchema::leaf("slot", &[WEARABLE_PRIORITY_PROPERTY])],
    },
    udd_kdl::strict::NodeSchema {
        name: "wearable",
        properties: &[WEARABLE_ITEMS_PROPERTY],
        children: &[udd_kdl::strict::NodeSchema {
            name: "override",
            properties: &[WEARABLE_PRIORITY_PROPERTY, "race", "direction"],
            children: &[udd_kdl::strict::NodeSchema::leaf("when", &[WEARABLE_ITEMS_PROPERTY])],
        }],
    },
];

/// `profile "<id>" equipment_offset=N [label=] [canvas_width=] [canvas_height=] [body_x=] [body_y=] [equipment_x=]
/// [equipment_y=]`, decoded by `PaperdollProfilesKdl`.
pub(crate) const PAPERDOLL_PROFILES_SCHEMA: &[udd_kdl::strict::NodeSchema] = &[udd_kdl::strict::NodeSchema::leaf(
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
        Self::parse(content).map_err(|error| udd_kdl::strict::in_file(source_name, error))
    }

    fn parse(content: &str) -> color_eyre::eyre::Result<Self> {
        use color_eyre::eyre;
        use udd_kdl::ast::{Node, Value};
        use udd_kdl::span::LineSpan;
        use udd_kdl::strict::{arguments, error_at, property, required_property, value_string, value_u16};

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

        let document = udd_kdl::parse_ast_line_span(WEARABLE_RULES_FILE_NAME, content)
            .map_err(|error| udd_kdl::strict::describe_error(content, &error))?;
        udd_kdl::strict::check_document(&document, WEARABLE_RULES_SCHEMA)?;
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
        let decoded: PaperdollProfilesKdl = udd_kdl::strict::decode(source_name, content, PAPERDOLL_PROFILES_SCHEMA)?;
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
    mut images: ResMut<Assets<Image>>,
) {
    if !state.open && state.open_gumps.is_empty() {
        return;
    }

    let mut closed_images = Vec::new();
    let open_request = {
        let Some(ctx) =
            dialogs::get_egui_context_ready_mut(&mut egui_contexts, &egui_ui_camera) else {
            return;
        };

        let open_request = if state.open {
            render_open_gump_dialog(ctx, &mut state, &paperdoll_profiles)
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
                    let response = ui.add(egui::Image::new((open_gump.texture_id, image_size)));
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
            gump_map.as_deref(),
            &mut images,
            &mut egui_contexts,
        ),
        Some(GumpOpenRequest::Paperdoll) => open_paperdoll(
            &mut state,
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

fn render_open_gump_dialog(
    ctx: &mut egui::Context,
    state: &mut GumpDialogState,
    paperdoll_profiles: &PaperdollProfilesRes,
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
    gump_map: Option<&GumpMapRes>,
    images: &mut Assets<Image>,
    egui_contexts: &mut EguiContexts,
) {
    let Some(gump_map) = gump_map else {
        ingame_sysmessage_logger::error("Classic gump source is not loaded.".to_string());
        return;
    };

    let Ok(id) = state.id.trim().parse::<u32>() else {
        ingame_sysmessage_logger::error(format!("Invalid gump ID: {}", state.id));
        return;
    };

    let mut scratch = Vec::new();
    let mut gump = match gump_map.decode_gump(id, &mut scratch) {
        Ok(gump) => gump,
        Err(error) => {
            ingame_sysmessage_logger::error(format!("Could not open gump {id}: {error}"));
            return;
        }
    };
    // bevy_egui blends user textures as premultiplied colour (`BlendState::PREMULTIPLIED_ALPHA_BLENDING`, and its
    // shader doesn't multiply by alpha), so the gump's straight colour goes up premultiplied: a transparent texel then
    // adds nothing, whatever colour it keeps, as in the clients.
    udd_assets::premultiplied::premultiply_rgba8888(&mut gump.rgba);

    let mut image = crate::util_lib::image::image_from_rgba8(
        gump.physical_width,
        gump.physical_height,
        &gump.rgba,
    );
    image.sampler = bevy::image::ImageSampler::nearest();
    let image = images.add(image);
    let texture_id = egui_contexts.add_image(EguiTextureHandle::Strong(image.clone()));
    state.open_gumps.push(OpenGump {
        title: format!("Gump {id}"),
        width: gump.logical_width.min(u16::MAX as u32) as u16,
        height: gump.logical_height.min(u16::MAX as u32) as u16,
        image,
        texture_id,
    });
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
    gump_map: Option<&GumpMapRes>,
    tilemeta: Option<&TileMetaPackageRes>,
    hues_package: Option<&HuesPackageRes>,
    paperdoll_profiles: &PaperdollProfilesRes,
    wearable_rules: &PaperdollWearableRulesRes,
    images: &mut Assets<Image>,
    egui_contexts: &mut EguiContexts,
) {
    let Some(gump_map) = gump_map else {
        ingame_sysmessage_logger::error("Classic gump source is not loaded.".to_string());
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

    let mut layers = vec![PaperdollLayer {
        gump_id: body_id,
        hue_id: body_hue,
        partial_hue: false,
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

    let gump = match compose_paperdoll(gump_map, hues_package, profile, &layers) {
        Ok(composed) => composed,
        Err(error) => {
            ingame_sysmessage_logger::error(format!("Could not compose paperdoll: {error}"));
            return;
        }
    };

    // Premultiplied already, as bevy_egui blends it: `alpha_blend_at` composes the layers over a transparent canvas.
    let mut image = crate::util_lib::image::image_from_rgba8(
        gump.physical_width,
        gump.physical_height,
        &gump.rgba,
    );
    image.sampler = bevy::image::ImageSampler::nearest();
    let image = images.add(image);
    let texture_id = egui_contexts.add_image(EguiTextureHandle::Strong(image.clone()));
    state.open_gumps.push(OpenGump {
        title: format!("Paperdoll ({})", profile.label),
        width: gump.logical_width.min(u16::MAX as u32) as u16,
        height: gump.logical_height.min(u16::MAX as u32) as u16,
        image,
        texture_id,
    });
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

fn compose_paperdoll(
    gump_map: &GumpMapRes,
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
    let mut min_x = 0i32;
    let mut min_y = 0i32;
    let mut max_x = profile.canvas_width as i32;
    let mut max_y = profile.canvas_height as i32;

    for layer in layers {
        let mut gump = gump_map.decode_gump(layer.gump_id, &mut scratch)?;
        if layer.hue_id > 0 {
            apply_hue(
                &mut gump.rgba,
                hues_package,
                hue_texture_bytes,
                layer.hue_id,
                layer.partial_hue,
            )?;
        }
        min_x = min_x.min(layer.x);
        min_y = min_y.min(layer.y);
        max_x = max_x.max(layer.x + gump.logical_width as i32);
        max_y = max_y.max(layer.y + gump.logical_height as i32);
        decoded_layers.push((layer.x, layer.y, gump));
    }

    let logical_width = (max_x - min_x).max(1) as u32;
    let logical_height = (max_y - min_y).max(1) as u32;
    let output_scale = decoded_layers
        .iter()
        .map(|(_, _, gump)| u32::from(gump.upscale_factor.max(1)))
        .max()
        .unwrap_or(1);
    let width = logical_width * output_scale;
    let height = logical_height * output_scale;
    let mut canvas = vec![0u8; width as usize * height as usize * 4];
    for (x, y, gump) in decoded_layers {
        let layer_width = gump.logical_width * output_scale;
        let layer_height = gump.logical_height * output_scale;
        let pixels = if gump.physical_width == layer_width && gump.physical_height == layer_height {
            gump.rgba
        } else {
            resize_rgba_nearest(
                gump.physical_width,
                gump.physical_height,
                &gump.rgba,
                layer_width,
                layer_height,
            )
        };
        alpha_blend_at(
            &mut canvas,
            width,
            (x - min_x) * output_scale as i32,
            (y - min_y) * output_scale as i32,
            layer_width,
            layer_height,
            &pixels,
        );
    }

    Ok(udd_assets::gumps::GumpImage {
        physical_width: width,
        physical_height: height,
        logical_width,
        logical_height,
        upscale_factor: output_scale.min(u16::MAX as u32) as u16,
        rgba: canvas,
    })
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

fn apply_hue(
    pixels: &mut [u8],
    hues_package: Option<&HuesPackageRes>,
    hue_texture_bytes: Option<&[u8]>,
    hue_id: u16,
    partial_hue: bool,
) -> color_eyre::eyre::Result<()> {
    let package = hues_package
        .ok_or_else(|| color_eyre::eyre::eyre!("hues.uddp is not loaded."))?;
    package
        .0
        .texture_coord_for_hue(hue_id)
        .ok_or_else(|| color_eyre::eyre::eyre!("Hue {hue_id} is not loaded."))?;
    let hue_texture_bytes = hue_texture_bytes
        .ok_or_else(|| color_eyre::eyre::eyre!("hues.uddp texture is not loaded."))?;

    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }
        if partial_hue && (pixel[0] != pixel[1] || pixel[0] != pixel[2]) {
            continue;
        }

        // Both clients index the hue by the red channel, not a luminance: the Enhanced client by the red byte
        // (`UOPaperdoll::DrawGumpLayer` 0x00509b30, at 0x00509d9e), Classic by the 5-bit red, `colour >> 10`
        // (`BlitGumpRowPartialHue` 0x005d8430, `BlitGumpRowFullHue` 0x005d83d0). Classic red expands to `red << 3`
        // and the hue strip holds hues.mul entry i at x = 8i, so the red byte reaches the same entry in both.
        let intensity = pixel[0];
        let color = sample_hue_lookup(hue_texture_bytes, hue_id, intensity)
            .ok_or_else(|| color_eyre::eyre::eyre!("Hue {hue_id} lookup is outside hues.uddp."))?;
        pixel[0] = color[0];
        pixel[1] = color[1];
        pixel[2] = color[2];
    }

    Ok(())
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
