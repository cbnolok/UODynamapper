use crate::core::controls::input_actions::{ActionCloseActiveDialog, ActionToggleGumpDialog};
use crate::core::render::scene::camera::UiCameraResource;
use crate::core::uo_files_loader::{ClassicHuesRes, GumpMapRes, TileMetaPackageRes};
use crate::configs::settings::Settings;
use crate::ingame_sysmessage_logger;
use crate::{
    core::constants,
    core::render::dialogs,
    prelude::*,
};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass, EguiTextureHandle, EguiUserTextures};
use knuffel::Decode;
use regex::Regex;
use std::{collections::HashMap, path::{Path, PathBuf}};

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

impl PaperdollWearableRules {
    fn load_from_path(path: &Path) -> color_eyre::eyre::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::parse(&content)
    }

    fn parse(content: &str) -> color_eyre::eyre::Result<Self> {
        let tag_re = Regex::new(r#"(?s)<(/?)(draworder|wearable|override)\b([^>]*)/?>"#)?;
        let attr_re = Regex::new(r#"([A-Za-z0-9_]+)\s*=\s*"([^"]*)""#)?;
        let mut paperdoll_draw_order = HashMap::new();
        let mut wearable_rules = Vec::new();
        let mut current_wearable: Option<WearableRule> = None;

        for capture in tag_re.captures_iter(content) {
            let closing = capture.get(1).map_or("", |m| m.as_str()) == "/";
            let tag_name = capture.get(2).map_or("", |m| m.as_str());
            let attrs = parse_xml_attrs(capture.get(3).map_or("", |m| m.as_str()), &attr_re);

            match (closing, tag_name) {
                (false, "draworder") => {
                    if attrs.get("direction").is_none_or(|direction| direction == "paperdoll") {
                        for (key, value) in attrs {
                            if key == "direction" {
                                continue;
                            }
                            if let Ok(priority) = value.parse::<u16>() {
                                paperdoll_draw_order.insert(key, priority);
                            }
                        }
                    }
                }
                (false, "wearable") => {
                    current_wearable = wearable_rule_from_attrs(attrs);
                }
                (false, "override") => {
                    if let Some(rule) = current_wearable.as_mut() {
                        if let Some(override_rule) = override_rule_from_attrs(attrs) {
                            rule.overrides.push(override_rule);
                        }
                    }
                }
                (true, "wearable") => {
                    if let Some(rule) = current_wearable.take() {
                        wearable_rules.push(rule);
                    }
                }
                _ => {}
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
    fn load() -> Self {
        let path = constants::valid_asset_dir().join("cc_ec_convtables/PaperdollProfiles.kdl");
        match Self::load_from_path(&path) {
            Ok(profiles) => profiles,
            Err(error) => {
                bevy::log::warn!(
                    "Failed to load PaperdollProfiles.kdl from {}: {error}. Using built-in profiles.",
                    path.display()
                );
                Self::default()
            }
        }
    }

    fn load_from_path(path: &Path) -> color_eyre::eyre::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let decoded: PaperdollProfilesKdl =
            knuffel::parse(path.to_str().unwrap_or("PaperdollProfiles.kdl"), &content)?;
        let profiles = decoded
            .profiles
            .into_iter()
            .map(PaperdollProfile::from)
            .collect::<Vec<_>>();
        if profiles.is_empty() {
            color_eyre::eyre::bail!("PaperdollProfiles.kdl contains no profile nodes");
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

impl Default for PaperdollProfilesRes {
    fn default() -> Self {
        Self {
            profiles: vec![
                PaperdollProfile::new("human_male", "Human Male", 50_000, 260, 300),
                PaperdollProfile::new("human_female", "Human Female", 60_000, 260, 300),
                PaperdollProfile::new("elf_male", "Elf Male", 50_000, 260, 300),
                PaperdollProfile::new("elf_female", "Elf Female", 60_000, 260, 300),
                PaperdollProfile::new("gargoyle_male", "Gargoyle Male", 50_000, 300, 340),
                PaperdollProfile::new("gargoyle_female", "Gargoyle Female", 60_000, 300, 340),
            ],
        }
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

impl PaperdollProfile {
    fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        equipment_offset: u32,
        canvas_width: u32,
        canvas_height: u32,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            equipment_offset,
            canvas_width,
            canvas_height,
            body_x: 0,
            body_y: 0,
            equipment_x: 0,
            equipment_y: 0,
        }
    }
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

fn parse_xml_attrs(attrs: &str, attr_re: &Regex) -> HashMap<String, String> {
    attr_re
        .captures_iter(attrs)
        .filter_map(|capture| {
            let key = capture.get(1)?.as_str().to_ascii_lowercase();
            let value = capture.get(2)?.as_str().trim().to_string();
            Some((key, value))
        })
        .collect()
}

fn wearable_rule_from_attrs(attrs: HashMap<String, String>) -> Option<WearableRule> {
    let (target_slot, target_items) = attrs
        .into_iter()
        .find_map(|(slot, value)| parse_wearable_items(&value).map(|items| (slot, items)))?;

    Some(WearableRule {
        target_slot,
        target_items,
        overrides: Vec::new(),
    })
}

fn override_rule_from_attrs(attrs: HashMap<String, String>) -> Option<WearableOverrideRule> {
    let draw_order_priority = attrs.get("draworderpriority")?.parse::<u16>().ok()?;
    let race = attrs.get("race").map(|race| race.to_ascii_lowercase());
    let mut conditions = Vec::new();

    for (slot, value) in attrs {
        if slot == "draworderpriority" || slot == "direction" || slot == "race" {
            continue;
        }
        if let Some(items) = parse_wearable_items(&value) {
            conditions.push(WearableCondition { slot, items });
        }
    }

    Some(WearableOverrideRule {
        draw_order_priority,
        race,
        conditions,
    })
}

fn parse_wearable_items(value: &str) -> Option<WearableItemMatcher> {
    if value.trim().eq_ignore_ascii_case("any") {
        return Some(WearableItemMatcher::Any);
    }

    let ids = value
        .split(',')
        .filter_map(|part| parse_u32_field(part.trim()).ok())
        .collect::<Vec<_>>();
    (!ids.is_empty()).then_some(WearableItemMatcher::Ids(ids))
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

fn sys_load_paperdoll_wearable_rules(
    settings: Res<Settings>,
    mut rules_res: ResMut<PaperdollWearableRulesRes>,
) {
    let source_root = PathBuf::from(&settings.runtime_assets.udd_path);
    let asset_root = constants::valid_asset_dir();
    let candidates = [
        asset_root.join("cc_ec_convtables/wearables.xml"),
        source_root.join("data/wearables.xml"),
        source_root.join("wearables.xml"),
    ];

    for path in candidates {
        if !path.is_file() {
            continue;
        }

        match PaperdollWearableRules::load_from_path(&path) {
            Ok(rules) => {
                log::info!("Loaded paperdoll wearable rules from {}", path.display());
                rules_res.rules = Some(rules);
                return;
            }
            Err(error) => {
                log::warn!(
                    "Failed to load paperdoll wearable rules from {}: {error}",
                    path.display()
                );
            }
        }
    }

    log::info!("No paperdoll wearable rules file selected; using tiledata layer order only.");
}

fn sys_gump_toggle(
    _trigger: On<ActionToggleGumpDialog>,
    mut state: ResMut<GumpDialogState>,
) {
    log_system_add_one_shot::<GumpDialogPlugin>("Observer", "ActionToggleGumpDialog", fname!());
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
    classic_hues: Option<Res<ClassicHuesRes>>,
    paperdoll_profiles: Res<PaperdollProfilesRes>,
    wearable_rules: Res<PaperdollWearableRulesRes>,
    mut images: ResMut<Assets<Image>>,
    mut egui_user_textures: ResMut<EguiUserTextures>,
) {
    if !state.open && state.open_gumps.is_empty() {
        return;
    }

    let Some(ctx) = dialogs::get_egui_context_ready_mut(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    if state.open {
        render_open_gump_dialog(
            ctx,
            &mut state,
            gump_map.as_deref(),
            tilemeta.as_deref(),
            classic_hues.as_deref(),
            &paperdoll_profiles,
            &wearable_rules,
            &mut images,
            &mut egui_user_textures,
        );
    }

    let mut closed_images = Vec::new();
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

    state.open_gumps.retain(|open_gump| {
        let close = closed_images.contains(&open_gump.image.id());
        if close {
            images.remove(open_gump.image.id());
        }
        !close
    });

    for image_id in closed_images {
        egui_user_textures.remove_image(image_id);
    }
}

fn render_open_gump_dialog(
    ctx: &mut egui::Context,
    state: &mut GumpDialogState,
    gump_map: Option<&GumpMapRes>,
    tilemeta: Option<&TileMetaPackageRes>,
    classic_hues: Option<&ClassicHuesRes>,
    paperdoll_profiles: &PaperdollProfilesRes,
    wearable_rules: &PaperdollWearableRulesRes,
    images: &mut Assets<Image>,
    egui_user_textures: &mut EguiUserTextures,
) {
    let mut window_open = state.open;
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
                open_gump_by_id(state, gump_map, images, egui_user_textures);
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
                        open_paperdoll(
                            state,
                            gump_map,
                            tilemeta,
                            classic_hues,
                            paperdoll_profiles,
                            wearable_rules,
                            images,
                            egui_user_textures,
                        );
                    }
                });
            }
        });

    state.open = window_open;
}

fn open_gump_by_id(
    state: &mut GumpDialogState,
    gump_map: Option<&GumpMapRes>,
    images: &mut Assets<Image>,
    egui_user_textures: &mut EguiUserTextures,
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
    let (width, height, pixels) = match gump_map.0.decode_gump(id, &mut scratch) {
        Ok(gump) => gump,
        Err(error) => {
            ingame_sysmessage_logger::error(format!("Could not open gump {id}: {error}"));
            return;
        }
    };

    let mut image = crate::util_lib::image::image_from_rgba8(
        width as u32,
        height as u32,
        &pixels,
    );
    image.sampler = bevy::image::ImageSampler::nearest();
    let image = images.add(image);
    let texture_id = egui_user_textures.add_image(EguiTextureHandle::Strong(image.clone()));
    state.open_gumps.push(OpenGump {
        title: format!("Gump {id}"),
        width,
        height,
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
    classic_hues: Option<&ClassicHuesRes>,
    paperdoll_profiles: &PaperdollProfilesRes,
    wearable_rules: &PaperdollWearableRulesRes,
    images: &mut Assets<Image>,
    egui_user_textures: &mut EguiUserTextures,
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

    let (width, height, pixels) = match compose_paperdoll(gump_map, classic_hues, profile, &layers) {
        Ok(composed) => composed,
        Err(error) => {
            ingame_sysmessage_logger::error(format!("Could not compose paperdoll: {error}"));
            return;
        }
    };

    let mut image = crate::util_lib::image::image_from_rgba8(width, height, &pixels);
    image.sampler = bevy::image::ImageSampler::nearest();
    let image = images.add(image);
    let texture_id = egui_user_textures.add_image(EguiTextureHandle::Strong(image.clone()));
    state.open_gumps.push(OpenGump {
        title: format!("Paperdoll ({})", profile.label),
        width: width as u16,
        height: height as u16,
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
    classic_hues: Option<&ClassicHuesRes>,
    profile: &PaperdollProfile,
    layers: &[PaperdollLayer],
) -> color_eyre::eyre::Result<(u32, u32, Vec<u8>)> {
    let mut decoded_layers = Vec::with_capacity(layers.len());
    let mut scratch = Vec::new();
    let mut min_x = 0i32;
    let mut min_y = 0i32;
    let mut max_x = profile.canvas_width as i32;
    let mut max_y = profile.canvas_height as i32;

    for layer in layers {
        let (layer_width, layer_height, mut pixels) =
            gump_map.0.decode_gump(layer.gump_id, &mut scratch)?;
        if layer.hue_id > 0 {
            apply_hue(&mut pixels, classic_hues, layer.hue_id, layer.partial_hue)?;
        }
        min_x = min_x.min(layer.x);
        min_y = min_y.min(layer.y);
        max_x = max_x.max(layer.x + layer_width as i32);
        max_y = max_y.max(layer.y + layer_height as i32);
        decoded_layers.push((layer.x, layer.y, layer_width as u32, layer_height as u32, pixels));
    }

    let width = (max_x - min_x).max(1) as u32;
    let height = (max_y - min_y).max(1) as u32;
    let mut canvas = vec![0u8; width as usize * height as usize * 4];
    for (x, y, layer_width, layer_height, pixels) in decoded_layers {
        alpha_blend_at(
            &mut canvas,
            width,
            x - min_x,
            y - min_y,
            layer_width,
            layer_height,
            &pixels,
        );
    }

    Ok((width, height, canvas))
}

fn apply_hue(
    pixels: &mut [u8],
    classic_hues: Option<&ClassicHuesRes>,
    hue_id: u16,
    partial_hue: bool,
) -> color_eyre::eyre::Result<()> {
    let hue = classic_hues
        .and_then(|hues| hues.0.get(hue_id.saturating_sub(1) as usize))
        .ok_or_else(|| color_eyre::eyre::eyre!("Hue {hue_id} is not loaded."))?;

    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }
        if partial_hue && (pixel[0] != pixel[1] || pixel[0] != pixel[2]) {
            continue;
        }

        let intensity = (((pixel[0] as u16 >> 3)
            + (pixel[1] as u16 >> 3)
            + (pixel[2] as u16 >> 3))
            / 3)
            .min(31) as usize;
        let color = hue.color_table[intensity];
        pixel[0] = (((color >> 10) & 0x1F) as u8) << 3;
        pixel[1] = (((color >> 5) & 0x1F) as u8) << 3;
        pixel[2] = ((color & 0x1F) as u8) << 3;
    }

    Ok(())
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
