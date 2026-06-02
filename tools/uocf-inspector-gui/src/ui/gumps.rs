use crate::app::{GumpSource, PaperdollEquipmentInput, UopInspectorApp};
use eframe::egui;
use knuffel::Decode;
use std::sync::Arc;
use std::sync::OnceLock;
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem as RawTextureItem};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::{LoadMode, UopPackage};

const HUE_SWATCH_COUNT: usize = 32;
const PAPERDOLL_PROFILE_KDL: &str =
    include_str!("../../../../dynamapper/assets/runtime_specs/paperdoll/PaperdollProfiles.kdl");

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
        body_x: i32,
        body_y: i32,
        equipment_x: i32,
        equipment_y: i32,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            equipment_offset,
            canvas_width,
            canvas_height,
            body_x,
            body_y,
            equipment_x,
            equipment_y,
        }
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

#[derive(Clone, Copy)]
struct PaperdollSlot {
    id: &'static str,
    label: &'static str,
    layer: u8,
}

const PAPERDOLL_SLOTS: &[PaperdollSlot] = &[
    PaperdollSlot {
        id: "one_handed",
        label: "One Handed",
        layer: 1,
    },
    PaperdollSlot {
        id: "two_handed",
        label: "Two Handed",
        layer: 2,
    },
    PaperdollSlot {
        id: "footwear",
        label: "Footwear",
        layer: 3,
    },
    PaperdollSlot {
        id: "legs",
        label: "Legs",
        layer: 4,
    },
    PaperdollSlot {
        id: "shirt",
        label: "Shirt",
        layer: 5,
    },
    PaperdollSlot {
        id: "head",
        label: "Head",
        layer: 6,
    },
    PaperdollSlot {
        id: "hands",
        label: "Hands",
        layer: 7,
    },
    PaperdollSlot {
        id: "ring",
        label: "Ring",
        layer: 8,
    },
    PaperdollSlot {
        id: "neck",
        label: "Neck",
        layer: 10,
    },
    PaperdollSlot {
        id: "hair",
        label: "Hair",
        layer: 12,
    },
    PaperdollSlot {
        id: "waist",
        label: "Waist",
        layer: 13,
    },
    PaperdollSlot {
        id: "inner_torso",
        label: "Inner Torso",
        layer: 14,
    },
    PaperdollSlot {
        id: "bracelet",
        label: "Bracelet",
        layer: 17,
    },
    PaperdollSlot {
        id: "facial_hair",
        label: "Facial Hair",
        layer: 18,
    },
    PaperdollSlot {
        id: "middle_torso",
        label: "Middle Torso",
        layer: 19,
    },
    PaperdollSlot {
        id: "earrings",
        label: "Earrings",
        layer: 20,
    },
    PaperdollSlot {
        id: "arms",
        label: "Arms",
        layer: 22,
    },
    PaperdollSlot {
        id: "cloak",
        label: "Cloak",
        layer: 23,
    },
    PaperdollSlot {
        id: "outer_torso",
        label: "Outer Torso",
        layer: 25,
    },
    PaperdollSlot {
        id: "outer_legs",
        label: "Outer Legs",
        layer: 26,
    },
    PaperdollSlot {
        id: "mount",
        label: "Mount",
        layer: 29,
    },
];

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

pub fn ui_gumps(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let profiles = paperdoll_profiles();

        ui.horizontal(|ui| {
            ui.selectable_value(&mut app.selected_gump_source, GumpSource::Classic, "CC");
            ui.selectable_value(&mut app.selected_gump_source, GumpSource::Enhanced, "EC");
        });
        ui.separator();

        ui.heading("Gump");
        ui.horizontal(|ui| {
            ui.label("ID:");
            ui.text_edit_singleline(&mut app.selected_gump_id);
        });

        match parse_u32_field(&app.selected_gump_id) {
            Ok(gump_id) => {
                ui.monospace(gump_source_detail(app.selected_gump_source, gump_id));
                ui.horizontal(|ui| {
                    if app.selected_gump_source == GumpSource::Enhanced
                        && ui.button("Open raw entry").clicked()
                    {
                        if !select_or_load_ec_gump_entry(app, gump_id) {
                            app.status_message = "interface.uop entry was not loaded.".to_string();
                        }
                    }
                    if app.selected_gump_source == GumpSource::Classic
                        && ui.button("Open CC UOP entry").clicked()
                    {
                        if !select_or_load_cc_gump_entry(app, gump_id) {
                            app.status_message =
                                "No matching gumpartLegacyMUL.uop entry was loaded.".to_string();
                        }
                    }
                });
                if let Some(handle) = app.get_gump_texture(ctx, app.selected_gump_source, gump_id) {
                    ui.label(format!("{}x{}", handle.size()[0], handle.size()[1]));
                    egui::ScrollArea::both()
                        .id_salt("gump_preview_scroll")
                        .max_height(320.0)
                        .show(ui, |ui| {
                            ui.image(&handle);
                        });
                } else if !app.selected_gump_id.trim().is_empty() {
                    ui.label("Gump not found in the selected source.");
                }
            }
            Err(_) if !app.selected_gump_id.trim().is_empty() => {
                ui.label("Invalid gump ID.");
            }
            Err(_) => {}
        }

        ui.separator();
        ui.heading("Paperdoll");
        egui::ComboBox::from_label("Profile")
            .selected_text(profile_by_id(profiles, &app.selected_paperdoll_profile).label.as_str())
            .show_ui(ui, |ui| {
                for profile in profiles {
                    ui.selectable_value(
                        &mut app.selected_paperdoll_profile,
                        profile.id.clone(),
                        profile.label.as_str(),
                    );
                }
            });
        let selected_profile = profile_by_id(profiles, &app.selected_paperdoll_profile);
        ui.monospace(format!(
            "equipment_offset={} canvas={}x{} body=({}, {}) equipment=({}, {})",
            selected_profile.equipment_offset,
            selected_profile.canvas_width,
            selected_profile.canvas_height,
            selected_profile.body_x,
            selected_profile.body_y,
            selected_profile.equipment_x,
            selected_profile.equipment_y,
        ));

        ui.horizontal(|ui| {
            ui.label("Body:");
            ui.text_edit_singleline(&mut app.paperdoll_body_id);
            ui.label("Hue:");
            ui.text_edit_singleline(&mut app.paperdoll_body_hue);
            if ui.button("Use Selected Hue").clicked() {
                app.paperdoll_body_hue = app.selected_hue_id.to_string();
                app.paperdoll_preview = None;
            }
        });
        ui_hue_picker(app, ui);

        egui::Grid::new("uocf_inspector_paperdoll_equipment")
            .num_columns(7)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Slot");
                ui.label("Item ID");
                ui.label("Hue");
                ui.label("Derived");
                ui.end_row();

                let tiledata = app.cc_tiledata.as_ref().map(Arc::clone);
                let mut remove_index = None;
                let mut inspect_item_id = None;
                for (index, row) in app.paperdoll_equipment.iter_mut().enumerate() {
                    egui::ComboBox::from_id_salt(format!("paperdoll_slot_{index}"))
                        .selected_text(slot_label(&row.slot))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut row.slot, String::new(), "Any");
                            for slot in PAPERDOLL_SLOTS {
                                ui.selectable_value(
                                    &mut row.slot,
                                    slot.id.to_string(),
                                    format!("{} ({})", slot.label, slot.layer),
                                );
                            }
                        });
                    ui.text_edit_singleline(&mut row.item_id);
                    ui.text_edit_singleline(&mut row.hue);
                    ui.monospace(equipment_detail(
                        tiledata.as_deref(),
                        selected_profile,
                        row.slot.as_str(),
                        row.item_id.as_str(),
                    ));
                    if ui.button("Hue").clicked() {
                        row.hue = app.selected_hue_id.to_string();
                        app.paperdoll_preview = None;
                    }
                    if ui.button("Inspect").clicked() {
                        if let Ok(item_id) = parse_u32_field(&row.item_id) {
                            inspect_item_id = Some(item_id);
                        }
                    }
                    if ui.button("Remove").clicked() {
                        remove_index = Some(index);
                    }
                    ui.end_row();
                }
                if let Some(index) = remove_index {
                    app.paperdoll_equipment.remove(index);
                }
                if let Some(item_id) = inspect_item_id {
                    app.selected_tex_art_cc_id = Some(0x4000 + item_id);
                    app.view_mode = crate::app::ViewMode::TexArtCc;
                }
            });

        ui.horizontal(|ui| {
            if ui.button("Add Slots").clicked() {
                app.paperdoll_equipment = PAPERDOLL_SLOTS
                    .iter()
                    .map(|slot| PaperdollEquipmentInput {
                        slot: slot.id.to_string(),
                        item_id: String::new(),
                        hue: "0".to_string(),
                    })
                    .collect();
                app.paperdoll_preview = None;
            }
            if ui.button("Add Row").clicked() {
                app.paperdoll_equipment.push(PaperdollEquipmentInput::default());
            }
            if ui.button("Clear Rows").clicked() {
                app.paperdoll_equipment.clear();
                app.paperdoll_preview = None;
            }
            if ui.button("Render").clicked() {
                match render_paperdoll(app, ctx) {
                    Ok(handle) => {
                        app.paperdoll_preview = Some(handle);
                        app.status_message = "Rendered paperdoll preview.".to_string();
                    }
                    Err(error) => {
                        app.paperdoll_preview = None;
                        app.status_message = format!("Could not render paperdoll: {error}");
                    }
                }
            }
        });

        if let Some(handle) = &app.paperdoll_preview {
            egui::ScrollArea::both()
                .id_salt("paperdoll_preview_scroll")
                .show(ui, |ui| {
                    ui.image(handle);
                });
        }
    });
}

fn paperdoll_profiles() -> &'static [PaperdollProfile] {
    static PROFILES: OnceLock<Vec<PaperdollProfile>> = OnceLock::new();
    PROFILES.get_or_init(|| {
        knuffel::parse::<PaperdollProfilesKdl>("PaperdollProfiles.kdl", PAPERDOLL_PROFILE_KDL)
            .ok()
            .map(|decoded| {
                decoded
                    .profiles
                    .into_iter()
                    .map(PaperdollProfile::from)
                    .collect::<Vec<_>>()
            })
            .filter(|profiles| !profiles.is_empty())
            .unwrap_or_else(default_profiles)
    })
}

fn default_profiles() -> Vec<PaperdollProfile> {
    vec![
        PaperdollProfile::new("human_male", "Human Male", 50_000, 260, 300, 0, 0, 0, 0),
        PaperdollProfile::new("human_female", "Human Female", 60_000, 260, 300, 0, 0, 0, 0),
        PaperdollProfile::new("elf_male", "Elf Male", 50_000, 260, 300, 0, 0, 0, 0),
        PaperdollProfile::new("elf_female", "Elf Female", 60_000, 260, 300, 0, 0, 0, 0),
        PaperdollProfile::new("gargoyle_male", "Gargoyle Male", 50_000, 300, 340, 0, 0, 0, 0),
        PaperdollProfile::new("gargoyle_female", "Gargoyle Female", 60_000, 300, 340, 0, 0, 0, 0),
    ]
}

fn profile_by_id<'a>(profiles: &'a [PaperdollProfile], id: &str) -> &'a PaperdollProfile {
    profiles
        .iter()
        .find(|profile| profile.id == id)
        .unwrap_or(&profiles[0])
}

fn slot_by_id(id: &str) -> Option<PaperdollSlot> {
    PAPERDOLL_SLOTS.iter().copied().find(|slot| slot.id == id)
}

fn slot_label(id: &str) -> String {
    slot_by_id(id)
        .map(|slot| format!("{} ({})", slot.label, slot.layer))
        .unwrap_or_else(|| "Any".to_string())
}

fn ec_gump_path(gump_id: u32, extension: &str) -> String {
    format!("data/interface/default/textures/gumpart/{gump_id:08}.{extension}")
}

fn ec_gump_candidates(gump_id: u32) -> [(String, ECImageFormat); 2] {
    [
        (ec_gump_path(gump_id, "tga"), ECImageFormat::TGA),
        (ec_gump_path(gump_id, "dds"), ECImageFormat::DDS),
    ]
}

fn cc_gump_paths(gump_id: u32) -> [String; 2] {
    [
        format!("build/gumpartlegacymul/{gump_id:08}.tga"),
        format!("build/gumpartlegacymul/{gump_id:07}.tga"),
    ]
}

fn gump_source_detail(source: GumpSource, gump_id: u32) -> String {
    match source {
        GumpSource::Classic => {
            let paths = cc_gump_paths(gump_id);
            format!(
                "CC source: gumpidx.mul/gumpart.mul or gumpartLegacyMUL.uop candidates {} / {}",
                paths[0], paths[1]
            )
        }
        GumpSource::Enhanced => {
            let path = ec_gump_path(gump_id, "tga");
            let hash = hash_file_name_single(&path);
            format!("EC source: interface.uop {path} hash={hash:016X}")
        }
    }
}

fn ui_hue_picker(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let Some(hues) = app
        .client_data
        .as_ref()
        .and_then(|client| client.hues.as_ref())
        .map(Arc::clone)
    else {
        ui.label("Load hues.mul to enable hue swatches.");
        return;
    };

    ui.horizontal_wrapped(|ui| {
        ui.label("Hue:");
        ui.add(egui::DragValue::new(&mut app.selected_hue_id).range(0..=hues.len() as u16));
        if app.selected_hue_id > 0 {
            if let Some(hue) = hues.get(app.selected_hue_id as usize - 1) {
                ui.label(hue_name(hue));
            }
        }
    });

    let hue_count = hues.len().min(u16::MAX as usize);
    let page_count = hue_swatch_page_count(hue_count);
    let mut page = hue_swatch_page_for_id(app.selected_hue_id, hue_count);
    ui.horizontal(|ui| {
        if ui.add_enabled(page > 0, egui::Button::new("Prev")).clicked() {
            page -= 1;
            app.selected_hue_id = hue_swatch_page_first_id(page);
        }
        ui.label(format!("Swatches {} / {}", page + 1, page_count.max(1)));
        if ui
            .add_enabled(page + 1 < page_count, egui::Button::new("Next"))
            .clicked()
        {
            page += 1;
            app.selected_hue_id = hue_swatch_page_first_id(page);
        }
    });

    ui.horizontal_wrapped(|ui| {
        for hue in hues.iter().skip(page * HUE_SWATCH_COUNT).take(HUE_SWATCH_COUNT) {
            let color = hue_swatch_color(hue);
            let selected = app.selected_hue_id == hue.id as u16;
            let text = if selected { format!("[{}]", hue.id) } else { hue.id.to_string() };
            let button = egui::Button::new(text).fill(color);
            if ui.add(button).clicked() {
                app.selected_hue_id = hue.id as u16;
            }
        }
    });
}

fn hue_swatch_page_count(hue_count: usize) -> usize {
    hue_count.saturating_add(HUE_SWATCH_COUNT - 1) / HUE_SWATCH_COUNT
}

fn hue_swatch_page_for_id(hue_id: u16, hue_count: usize) -> usize {
    let page_count = hue_swatch_page_count(hue_count);
    if page_count == 0 || hue_id == 0 {
        return 0;
    }

    let hue_index = hue_id.saturating_sub(1) as usize;
    (hue_index / HUE_SWATCH_COUNT).min(page_count - 1)
}

fn hue_swatch_page_first_id(page: usize) -> u16 {
    (page * HUE_SWATCH_COUNT + 1).min(u16::MAX as usize) as u16
}

fn hue_name(hue: &uocf::classic::hues::HueEntry) -> String {
    std::str::from_utf8(&hue.name)
        .unwrap_or("")
        .trim_matches('\0')
        .to_string()
}

fn hue_swatch_color(hue: &uocf::classic::hues::HueEntry) -> egui::Color32 {
    let color = hue.color_table[24];
    let r = (((color >> 10) & 0x1F) as u8) << 3;
    let g = (((color >> 5) & 0x1F) as u8) << 3;
    let b = ((color & 0x1F) as u8) << 3;
    egui::Color32::from_rgb(r, g, b)
}

fn select_or_load_ec_gump_entry(app: &mut UopInspectorApp, gump_id: u32) -> bool {
    for (path, _) in ec_gump_candidates(gump_id) {
        let hash = hash_file_name_single(&path);
        if app.select_raw_uop_entry("interface.uop", hash) {
            return true;
        }
    }

    let Some(ec_path) = app.settings.ec_path.clone() else {
        return false;
    };
    let uop_path = ec_path.join("interface.uop");
    if !uop_path.exists() {
        return false;
    }

    match UopPackage::load_with_mode(&uop_path, LoadMode::Lazy) {
        Ok(package) => {
            app.uop_cache.add(uop_path, package);
            for (path, _) in ec_gump_candidates(gump_id) {
                let hash = hash_file_name_single(&path);
                if app.select_raw_uop_entry("interface.uop", hash) {
                    return true;
                }
            }
            false
        }
        Err(error) => {
            app.status_message = format!("Failed to open interface.uop lazily: {error}");
            false
        }
    }
}

fn select_or_load_cc_gump_entry(app: &mut UopInspectorApp, gump_id: u32) -> bool {
    for path in cc_gump_paths(gump_id) {
        let hash = hash_file_name_single(&path);
        if app.select_raw_uop_entry("gumpartLegacyMUL.uop", hash)
            || app.select_raw_uop_entry("gumpartlegacymul.uop", hash)
        {
            return true;
        }
    }

    let Some(cc_path) = app.settings.cc_path.clone() else {
        return false;
    };
    for package_name in ["gumpartLegacyMUL.uop", "GumpartLegacyMUL.uop", "gumpartlegacymul.uop"] {
        let uop_path = cc_path.join(package_name);
        if !uop_path.exists() {
            continue;
        }
        match UopPackage::load_with_mode(&uop_path, LoadMode::Lazy) {
            Ok(package) => {
                app.uop_cache.add(uop_path, package);
                for path in cc_gump_paths(gump_id) {
                    let hash = hash_file_name_single(&path);
                    if app.select_raw_uop_entry(package_name, hash) {
                        return true;
                    }
                }
            }
            Err(error) => {
                app.status_message =
                    format!("Failed to open {package_name} lazily: {error}");
                return false;
            }
        }
    }

    false
}

fn equipment_detail(
    tiledata: Option<&uocf::classic::tiledata::TileData>,
    profile: &PaperdollProfile,
    slot_id: &str,
    item_id: &str,
) -> String {
    if item_id.trim().is_empty() {
        return String::new();
    }

    let Ok(item_id) = parse_u32_field(item_id) else {
        return "invalid item id".to_string();
    };
    let Some(item) = tiledata.and_then(|tiledata| tiledata.item_tiles().get(item_id as usize)) else {
        return "unknown item".to_string();
    };
    if item.anim_id == 0 {
        return format!("{} anim_id=0", item.name_ascii());
    }

    let slot_note = slot_by_id(slot_id)
        .map(|slot| {
            if slot.layer == item.quality {
                format!(" slot={}", slot.label)
            } else {
                format!(" slot={} expected_layer={}", slot.label, slot.layer)
            }
        })
        .unwrap_or_default();

    format!(
        "{} anim_id={} gump={} layer={} partial_hue={}{}",
        item.name_ascii(),
        item.anim_id,
        item.anim_id as u32 + profile.equipment_offset,
        item.quality,
        item.flags.partialhue(),
        slot_note,
    )
}

fn render_paperdoll(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
) -> color_eyre::eyre::Result<egui::TextureHandle> {
    let body_id = parse_u32_field(&app.paperdoll_body_id)?;
    let profile = profile_by_id(paperdoll_profiles(), &app.selected_paperdoll_profile);
    let mut layers = vec![PaperdollLayer {
        gump_id: body_id,
        hue_id: parse_optional_hue(&app.paperdoll_body_hue),
        partial_hue: false,
        sort_key: 0,
        x: profile.body_x,
        y: profile.body_y,
    }];

    let tiledata = app.cc_tiledata.as_ref().map(Arc::clone);
    for row in &app.paperdoll_equipment {
        if row.item_id.trim().is_empty() {
            continue;
        }

        let item_id = parse_u32_field(&row.item_id)?;
        let item = tiledata
            .as_ref()
            .and_then(|tiledata| tiledata.item_tiles().get(item_id as usize))
            .ok_or_else(|| color_eyre::eyre::eyre!("Unknown equipment item ID {item_id}"))?;
        if item.anim_id == 0 {
            color_eyre::eyre::bail!("Equipment item {item_id} has no paperdoll anim_id.");
        }

        layers.push(PaperdollLayer {
            gump_id: item.anim_id as u32 + profile.equipment_offset,
            hue_id: parse_optional_hue(&row.hue),
            partial_hue: item.flags.partialhue(),
            sort_key: item.quality as u16,
            x: profile.equipment_x,
            y: profile.equipment_y,
        });
    }

    layers[1..].sort_by_key(|layer| layer.sort_key);
    let (width, height, pixels) = compose_paperdoll(app, profile, &layers)?;
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        &pixels,
    );
    let key = 0x7000000000000000 | u64::from(body_id);
    app.register_current_image_preview(
        key,
        format!("paperdoll {} {}", app.selected_paperdoll_profile, body_id),
        width,
        height,
        &pixels,
    );
    Ok(ctx.load_texture(
        format!("paperdoll_{:?}_{}", app.selected_gump_source, app.selected_paperdoll_profile),
        image,
        Default::default(),
    ))
}

fn compose_paperdoll(
    app: &UopInspectorApp,
    profile: &PaperdollProfile,
    layers: &[PaperdollLayer],
) -> color_eyre::eyre::Result<(u32, u32, Vec<u8>)> {
    let mut decoded_layers = Vec::with_capacity(layers.len());
    let mut min_x = 0i32;
    let mut min_y = 0i32;
    let mut max_x = profile.canvas_width as i32;
    let mut max_y = profile.canvas_height as i32;

    for layer in layers {
        let (layer_width, layer_height, mut pixels) =
            decode_gump_rgba(app, app.selected_gump_source, layer.gump_id)?;
        if layer.hue_id > 0 {
            apply_hue(app, &mut pixels, layer.hue_id, layer.partial_hue)?;
        }

        min_x = min_x.min(layer.x);
        min_y = min_y.min(layer.y);
        max_x = max_x.max(layer.x + layer_width as i32);
        max_y = max_y.max(layer.y + layer_height as i32);
        decoded_layers.push((layer.x, layer.y, layer_width, layer_height, pixels));
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

fn decode_gump_rgba(
    app: &UopInspectorApp,
    source: GumpSource,
    gump_id: u32,
) -> color_eyre::eyre::Result<(u32, u32, Vec<u8>)> {
    match source {
        GumpSource::Classic => {
            if let Some(package) = &app.cc_gumps_package {
                if let Ok(gump) = package.read_gump_rgba(gump_id) {
                    return Ok(gump);
                }
            }
            let gumps = app
                .cc_gumps
                .as_ref()
                .ok_or_else(|| color_eyre::eyre::eyre!("CC gump source is not loaded."))?;
            let mut scratch = Vec::new();
            let (width, height, pixels) = gumps.decode_gump(gump_id, &mut scratch)?;
            Ok((width as u32, height as u32, pixels))
        }
        GumpSource::Enhanced => decode_ec_gump_rgba(app, gump_id),
    }
}

fn decode_ec_gump_rgba(
    app: &UopInspectorApp,
    gump_id: u32,
) -> color_eyre::eyre::Result<(u32, u32, Vec<u8>)> {
    if let Some(package) = &app.ec_gumps_package {
        if let Ok(gump) = package.read_gump_rgba(gump_id) {
            return Ok(gump);
        }
    }

    for loaded in &app.uop_cache.loaded_uops {
        let is_interface = loaded
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case("interface.uop"))
            .unwrap_or(false);
        if !is_interface {
            continue;
        }

        for (path, format) in ec_gump_candidates(gump_id) {
            let hash = hash_file_name_single(&path);
            if loaded.package.get_file_by_hash(hash).is_some() {
                let data: Arc<[u8]> = loaded.package.unpack_file_by_hash(hash)?
                    .ok_or_else(|| color_eyre::eyre::eyre!("EC gump entry disappeared from interface.uop."))?
                    .into();
                let tex_file = TextureFile {
                    metadata: RawTextureItem::absent(),
                    is_ec: true,
                    format,
                    props: None,
                    raw_data: data,
                    image_data_offset: 0,
                };
                let image = tex_file.decode_to_rgba()?;
                let rgba = image.to_rgba8();
                return Ok((rgba.width(), rgba.height(), rgba.into_raw()));
            }
        }
    }

    color_eyre::eyre::bail!("EC gump {gump_id} was not found in interface.uop.");
}

fn apply_hue(
    app: &UopInspectorApp,
    pixels: &mut [u8],
    hue_id: u16,
    partial_hue: bool,
) -> color_eyre::eyre::Result<()> {
    let hue = app
        .client_data
        .as_ref()
        .and_then(|client| client.hues.as_ref())
        .and_then(|hues| hues.get(hue_id.saturating_sub(1) as usize))
        .ok_or_else(|| color_eyre::eyre::eyre!("Hue {hue_id} is not loaded."))?;

    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }

        let color = ((pixel[3] as u32) << 24)
            | ((pixel[0] as u32) << 16)
            | ((pixel[1] as u32) << 8)
            | pixel[2] as u32;
        let hued = hue.apply_to_color32(color, partial_hue);
        pixel[0] = ((hued >> 16) & 0xFF) as u8;
        pixel[1] = ((hued >> 8) & 0xFF) as u8;
        pixel[2] = (hued & 0xFF) as u8;
        pixel[3] = ((hued >> 24) & 0xFF) as u8;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ec_gump_path_uses_interface_gumpart_folder() {
        assert_eq!(
            ec_gump_path(9504, "tga"),
            "data/interface/default/textures/gumpart/00009504.tga"
        );
    }

    #[test]
    fn ec_gump_candidates_cover_tga_and_dds() {
        let candidates = ec_gump_candidates(9504);

        assert_eq!(
            candidates[0],
            (
                "data/interface/default/textures/gumpart/00009504.tga".to_string(),
                ECImageFormat::TGA,
            )
        );
        assert_eq!(
            candidates[1],
            (
                "data/interface/default/textures/gumpart/00009504.dds".to_string(),
                ECImageFormat::DDS,
            )
        );
    }

    #[test]
    fn cc_gump_paths_cover_eight_and_seven_digit_uop_names() {
        assert_eq!(
            cc_gump_paths(9504),
            [
                "build/gumpartlegacymul/00009504.tga".to_string(),
                "build/gumpartlegacymul/0009504.tga".to_string(),
            ]
        );
    }

    #[test]
    fn paperdoll_profiles_load_from_kdl() {
        let profiles = paperdoll_profiles();
        let male = profile_by_id(profiles, "human_male");
        let gargoyle = profile_by_id(profiles, "gargoyle_female");

        assert_eq!(male.equipment_offset, 50_000);
        assert_eq!(male.canvas_width, 260);
        assert_eq!(gargoyle.equipment_offset, 60_000);
        assert_eq!(gargoyle.canvas_height, 340);
    }

    #[test]
    fn paperdoll_slot_labels_include_layers() {
        assert_eq!(slot_label("head"), "Head (6)");
        assert_eq!(slot_label("outer_torso"), "Outer Torso (25)");
        assert_eq!(slot_label(""), "Any");
    }

    #[test]
    fn hue_swatch_pages_follow_selected_hue() {
        assert_eq!(hue_swatch_page_count(0), 0);
        assert_eq!(hue_swatch_page_count(33), 2);
        assert_eq!(hue_swatch_page_for_id(0, 100), 0);
        assert_eq!(hue_swatch_page_for_id(1, 100), 0);
        assert_eq!(hue_swatch_page_for_id(32, 100), 0);
        assert_eq!(hue_swatch_page_for_id(33, 100), 1);
        assert_eq!(hue_swatch_page_for_id(500, 100), 3);
        assert_eq!(hue_swatch_page_first_id(2), 65);
    }

    #[test]
    fn parse_u32_field_accepts_hex_and_decimal() {
        assert_eq!(parse_u32_field("9504").unwrap(), 9504);
        assert_eq!(parse_u32_field("0x2520").unwrap(), 9504);
    }
}
