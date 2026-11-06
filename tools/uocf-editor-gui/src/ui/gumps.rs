use crate::app::{GumpSource, GumpViewerTab, PaperdollEquipmentInput, UocfEditorApp};
use crate::ui::ec_interface_textures;
use crate::ui::ec_paperdoll::{self, Definitions, FrameSets, WearableRules, WornItem, PAPERDOLL_UOP_NAME};
use crate::ui::{arrow_delta, centered_row_offset, go_to_field, list_sort_controls, move_selection, sorted_indices_by};
use crate::ui::{enhanced_files_availability, source_choice, ClientSource, SourceAvailability};
use eframe::egui;
use knuffel::Decode;
use std::path::Path;
use std::sync::Arc;
use std::sync::OnceLock;
use uocf::classic::gump::GumpContainer;
use uocf::enhanced::hues::{EcHueTables, HUES_UOP_NAME};
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem as RawTextureItem};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::{LoadMode, UopPackage};
use uocf::classic::gump::GumpMap;
use uocf::classic::gump_write::GumpColors;
use uocf::image_export::gump_file_stem;
use crate::logic::uop_cache::LoadedUop;
use crate::ui::image_export::{
    export_menu, save_image, save_status, start_bulk, BulkEntry, BulkExport, ExportClicked, ExportImage, ExportMenu,
};

const HUE_SWATCH_COUNT: usize = 32;
const PAPERDOLL_SPRITE_ID_START: u32 = 50_000;
const PAPERDOLL_SPRITE_ID_END: u32 = 69_999;
const MAX_RAW_UOP_GUMP_ID: u32 = 99_999;
const GUMP_SPLIT_DEFAULT_RATIO: f32 = 1.0 / 3.0;
const GUMP_SPLIT_MIN_RATIO: f32 = 0.20;
const GUMP_SPLIT_MAX_RATIO: f32 = 0.80;
const PAPERDOLL_PROFILE_KDL: &str =
    include_str!("../../../../dynamapper/assets/runtime_specs/paperdoll/PaperdollProfiles.kdl");
/// Image preview keys of the gumps of each source, or-ed with the gump id.
const MUL_GUMP_PREVIEW_KEY: u64 = 0x6D00000000000000;
const CC_UOP_GUMP_PREVIEW_KEY: u64 = 0x6C00000000000000;
const EC_GUMP_PREVIEW_KEY: u64 = 0x6E00000000000000;
/// The gump sources, in the order every tab shows them. An Enhanced-format client's files hold one set of gump images,
/// in `Interface.uop`: [`GumpSource::EcUop`] is the KR UOP or EC UOP button by the client shown.
const GUMP_SOURCES: [GumpSource; 3] = [GumpSource::Mul, GumpSource::CcUop, GumpSource::EcUop];
const INTERFACE_UOP_NAME: &str = "interface.uop";
const INTERFACE_UOP_FILE: &str = "Interface.uop";
/// Exported gump files are named `gump_0x0FA0`.
const GUMP_NOT_HUED: &str = "The gump preview draws the colours as stored (only the paperdoll is hued).";
/// Image preview keys of the rendered paperdolls, or-ed with the body id.
const PAPERDOLL_PREVIEW_KEY: u64 = 0x7000000000000000;
const EC_PAPERDOLL_PREVIEW_KEY: u64 = 0x7100000000000000;
/// egui memory of the `Paperdoll.uop` frame sets, with the Enhanced Client folder.
const EC_PAPERDOLL_FRAME_SETS_ID: &str = "uocf_ec_paperdoll_frame_sets";
const EC_PAPERDOLL_NOTE: &str = "EC: the body and item frames of Paperdoll.uop, placed around the body's anchor and \
                                 drawn in the paperdoll order of wearables.xml (WearableRules.kdl). An item's layer \
                                 comes from its tileart.uop Layer property, else its tiledata layer; the Slot column \
                                 and the profile are not used.";

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

/// The Classic gump container a source reads; `None` for the Enhanced Client's gumps.
fn classic_container(source: GumpSource) -> Option<GumpContainer> {
    match source {
        GumpSource::Mul => Some(GumpContainer::Mul),
        GumpSource::CcUop => Some(GumpContainer::Uop),
        GumpSource::EcUop => None,
    }
}

/// The source button `source` stands for: the Enhanced-format client shown for [`GumpSource::EcUop`].
fn client_source(app: &UocfEditorApp, source: GumpSource) -> Option<ClientSource> {
    match source {
        GumpSource::Mul => Some(ClientSource::Mul),
        GumpSource::CcUop => Some(ClientSource::CcUop),
        GumpSource::EcUop => app.shown_enhanced,
    }
}

fn source_label(app: &UocfEditorApp, source: GumpSource) -> &'static str {
    client_source(app, source).map_or("", ClientSource::label)
}

/// Whether the loaded client files hold `source`.
fn source_present(app: &UocfEditorApp, source: GumpSource) -> bool {
    match classic_container(source) {
        Some(container) => app.cc_gumps.as_ref().is_some_and(|gumps| gumps.has_container(container)),
        None => app.uop_cache.loaded_uops.iter().any(|loaded| {
            loaded.path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.eq_ignore_ascii_case(INTERFACE_UOP_NAME))
        }),
    }
}

/// The tooltip of a Classic source the loaded files hold.
fn source_found_text(source: GumpSource) -> &'static str {
    match source {
        GumpSource::Mul => "gumpidx.mul and gumpart.mul in the Classic Client folder.",
        GumpSource::CcUop => "gumpartLegacyMUL.uop in the Classic Client folder.",
        GumpSource::EcUop => "",
    }
}

/// The tooltip of a Classic source the loaded files do not hold.
fn source_missing_text(source: GumpSource) -> &'static str {
    match source {
        GumpSource::Mul => "gumpidx.mul and gumpart.mul were not found in the Classic Client folder.",
        GumpSource::CcUop => "gumpartLegacyMUL.uop was not found in the Classic Client folder.",
        GumpSource::EcUop => "",
    }
}

pub fn ui_gumps(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        let profiles = paperdoll_profiles();
        if !source_present(app, app.selected_gump_source) {
            if let Some(&source) = GUMP_SOURCES.iter().find(|&&source| source_present(app, source)) {
                app.selected_gump_source = source;
            }
        }
        if app.gump_list_source != Some(app.selected_gump_source)
            || app.gump_list_profile != app.selected_paperdoll_profile
        {
            refresh_gump_lists(app, profiles);
        }

        ui.horizontal(|ui| {
            let classic_availability = |source| {
                if source_present(app, source) {
                    SourceAvailability::Found(source_found_text(source).to_string())
                } else {
                    SourceAvailability::Missing(source_missing_text(source).to_string())
                }
            };
            let selected = client_source(app, app.selected_gump_source);
            let clicked = source_choice(ui, selected, |source| match source {
                ClientSource::Mul => classic_availability(GumpSource::Mul),
                ClientSource::CcUop => classic_availability(GumpSource::CcUop),
                ClientSource::KrUop | ClientSource::EcUop => enhanced_files_availability(
                    app,
                    source,
                    INTERFACE_UOP_FILE,
                    &[INTERFACE_UOP_FILE],
                    source_present(app, GumpSource::EcUop),
                ),
            });
            match clicked {
                Some(ClientSource::Mul) => app.selected_gump_source = GumpSource::Mul,
                Some(ClientSource::CcUop) => app.selected_gump_source = GumpSource::CcUop,
                Some(source) => {
                    app.selected_gump_source = GumpSource::EcUop;
                    app.show_enhanced_client(source);
                }
                None => {}
            }
            ui.separator();
            ui.selectable_value(
                &mut app.selected_gump_viewer_tab,
                GumpViewerTab::StandardGumps,
                "Gumps",
            );
            ui.selectable_value(
                &mut app.selected_gump_viewer_tab,
                GumpViewerTab::Paperdoll,
                "Paperdoll",
            );
            ui.selectable_value(
                &mut app.selected_gump_viewer_tab,
                GumpViewerTab::InterfaceImages,
                "EC interface images",
            );
            ui.separator();
            if ui.button("Refresh Lists").clicked() {
                refresh_gump_lists(app, profiles);
            }
        });
        if !app.gump_list_status.is_empty() {
            ui.monospace(app.gump_list_status.as_str());
        }
        ui.separator();

        match app.selected_gump_viewer_tab {
            GumpViewerTab::StandardGumps => ui_standard_gumps(app, ctx, ui),
            GumpViewerTab::Paperdoll => ui_paperdoll(app, ctx, ui, profiles),
            GumpViewerTab::InterfaceImages => ec_interface_textures::ui_interface_textures(app, ctx, ui),
        }
    });
}

fn ui_standard_gumps(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let gump_ids = app.standard_gump_ids.clone();
    let selected_gump_id_text = app.selected_gump_id.clone();
    let mut selected_gump_id = None;
    ui_gump_split(
        ui,
        "standard_gump_split",
        |left| {
            left.heading("Standard Gumps");
            selected_gump_id = ui_gump_id_list(
                left,
                &gump_ids,
                &selected_gump_id_text,
                "standard_gump_list",
            );
        },
        |right| {
            right.heading("Gump");
            ui_gump_preview_controls(app, ctx, right, &gump_ids);
        },
    );
    if let Some(gump_id) = selected_gump_id {
        app.selected_gump_id = gump_id.to_string();
    }
}

fn ui_gump_split(
    ui: &mut egui::Ui,
    id_salt: &'static str,
    add_left: impl FnOnce(&mut egui::Ui),
    add_right: impl FnOnce(&mut egui::Ui),
) {
    let id = ui.make_persistent_id(("uocf_gump_split_ratio", id_salt));
    let mut ratio = ui
        .data_mut(|data| data.get_temp::<f32>(id))
        .unwrap_or(GUMP_SPLIT_DEFAULT_RATIO)
        .clamp(GUMP_SPLIT_MIN_RATIO, GUMP_SPLIT_MAX_RATIO);

    let available_height = ui.available_height();
    let divider_width = 8.0;
    let content_width = (ui.available_width() - divider_width).max(1.0);
    let left_width = (content_width * ratio).clamp(1.0, content_width);
    let right_width = (content_width - left_width).max(1.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;

        ui.allocate_ui_with_layout(
            egui::vec2(left_width, available_height),
            egui::Layout::top_down(egui::Align::Min),
            |left| {
                left.set_width(left_width);
                left.set_height(available_height);
                add_left(left);
            },
        );
        let (divider_rect, divider_response) = ui.allocate_exact_size(
            egui::vec2(divider_width, available_height),
            egui::Sense::click_and_drag(),
        );
        let divider_response =
            divider_response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if divider_response.dragged() {
            let delta_x = ui.input(|input| input.pointer.delta().x);
            ratio = (ratio + delta_x / content_width)
                .clamp(GUMP_SPLIT_MIN_RATIO, GUMP_SPLIT_MAX_RATIO);
            ui.ctx().request_repaint();
        }
        let visuals = ui.visuals();
        let divider_color = if divider_response.dragged() {
            visuals.widgets.active.bg_fill
        } else if divider_response.hovered() {
            visuals.widgets.hovered.bg_fill
        } else {
            visuals.widgets.noninteractive.bg_stroke.color
        };
        ui.painter().rect_filled(
            divider_rect.shrink2(egui::vec2(3.0, 0.0)),
            0.0,
            divider_color,
        );
        ui.allocate_ui_with_layout(
            egui::vec2(right_width, available_height),
            egui::Layout::top_down(egui::Align::Min),
            |right| {
                right.set_width(right_width);
                right.set_height(available_height);
                add_right(right);
            },
        );
    });
    ui.data_mut(|data| data.insert_temp(id, ratio));
}

/// The preview of the selected gump; `shown` the ids the list beside it shows.
fn ui_gump_preview_controls(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui, shown: &[u32]) {
    ui.horizontal(|ui| {
        ui.label("ID:");
        ui.text_edit_singleline(&mut app.selected_gump_id);
    });

    match parse_u32_field(&app.selected_gump_id) {
        Ok(gump_id) => {
            ui.monospace(gump_source_detail(app.selected_gump_source, gump_id));
            ui.horizontal(|ui| {
                if app.selected_gump_source == GumpSource::EcUop
                    && ui.button("Open raw entry").clicked()
                {
                    if !select_or_load_ec_gump_entry(app, gump_id) {
                        app.status_message = "interface.uop entry was not loaded.".to_string();
                    }
                }
                if app.selected_gump_source == GumpSource::CcUop
                    && ui.button("Open CC UOP entry").clicked()
                {
                    if !select_or_load_cc_gump_entry(app, gump_id) {
                        app.status_message =
                            "No matching gumpartLegacyMUL.uop entry was loaded.".to_string();
                    }
                }
            });
            if let Some(handle) = get_panel_gump_texture(app, ctx, app.selected_gump_source, gump_id) {
                ui.horizontal(|ui| {
                    ui.label(format!("{}x{}", handle.size()[0], handle.size()[1]));
                    ui_gump_export(app, ui, gump_id, shown);
                });
                egui::ScrollArea::both()
                    .id_salt("gump_preview_scroll")
                    .max_height(320.0)
                    .show(ui, |ui| {
                        ui.image(&handle);
                    });
            } else if !app.selected_gump_id.trim().is_empty() {
                ui.label("Gump not found in the selected source.");
            }
            if let Some(container) = classic_container(app.selected_gump_source) {
                let preferred = match container {
                    GumpContainer::Mul => super::classic_image_import::PreferredContainer::Mul,
                    GumpContainer::Uop => super::classic_image_import::PreferredContainer::Uop,
                };
                ui.separator();
                super::classic_image_import::ui_classic_image_panel(
                    app,
                    ctx,
                    ui,
                    uocf::classic::image_files::ClassicImageKind::Gump,
                    gump_id,
                    preferred,
                );
            }
        }
        Err(_) if !app.selected_gump_id.trim().is_empty() => {
            ui.label("Invalid gump ID.");
        }
        Err(_) => {}
    }
}

/// The export menu of gump `gump_id`, and the bulk export of the `shown` ids.
fn ui_gump_export(app: &mut UocfEditorApp, ui: &mut egui::Ui, gump_id: u32, shown: &[u32]) {
    let source = app.selected_gump_source;
    let menu = ExportMenu {
        hue: Err(GUMP_NOT_HUED),
        frame: false,
        bulk: vec![BulkEntry {
            label: format!("Export all shown ({})...", shown.len()),
            hover: format!(
                "Every gump the list shows, each as {}.<format>, into a folder you pick (files of the same name are \
                 replaced).",
                gump_file_stem(gump_id)
            ),
        }],
    };
    let Some(clicked) = export_menu(ui, &mut app.image_export, &menu) else {
        return;
    };
    let format = app.image_export.format;
    let gumps = app.cc_gumps.clone();
    let interface = interface_uop(app);
    match clicked {
        ExportClicked::Shown => {
            let result = gump_export_image(gumps.as_deref(), interface.as_deref(), source, gump_id)
                .map_err(|error| format!("{error:#}"))
                .and_then(|image| save_image(&gump_file_stem(gump_id), &image, format));
            if let Some(status) = save_status(&format!("gump {gump_id:#06x}"), result) {
                app.status_message = status;
            }
        }
        ExportClicked::Bulk(_) => {
            let ids = shown.to_vec();
            start_bulk(app, |folder| {
                BulkExport::start(format!("Export {} gumps", ids.len()), folder, format, ids, move |gump_id| {
                    let image = gump_export_image(gumps.as_deref(), interface.as_deref(), source, gump_id)?;
                    Ok(vec![(gump_file_stem(gump_id), image)])
                })
            });
        }
    }
}

/// The loaded interface.uop, which holds the Enhanced gumps.
fn interface_uop(app: &UocfEditorApp) -> Option<Arc<LoadedUop>> {
    app.uop_cache
        .loaded_uops
        .iter()
        .find(|loaded| loaded.path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.eq_ignore_ascii_case(INTERFACE_UOP_NAME)))
        .cloned()
}

/// Gump `gump_id` of `source` to export: a Classic gump as stored (16-bit), an Enhanced gump decoded to RGBA.
fn gump_export_image(
    gumps: Option<&GumpMap>,
    interface: Option<&LoadedUop>,
    source: GumpSource,
    gump_id: u32,
) -> color_eyre::eyre::Result<ExportImage> {
    match classic_container(source) {
        Some(container) => {
            let gumps = gumps.ok_or_else(|| color_eyre::eyre::eyre!("CC gump source is not loaded."))?;
            let mut raw = Vec::new();
            let dimensions = gumps.get_raw_gump_data_in(Some(container), gump_id, &mut raw)?;
            Ok(ExportImage::gump(&GumpColors::decode_rows(&raw, dimensions.width, dimensions.height)?))
        }
        None => {
            let interface = interface.ok_or_else(|| color_eyre::eyre::eyre!("{INTERFACE_UOP_NAME} is not loaded."))?;
            let (width, height, rgba) = ec_gump_rgba_in(&interface.package, gump_id)?
                .ok_or_else(|| color_eyre::eyre::eyre!("EC gump {gump_id} was not found in interface.uop."))?;
            Ok(ExportImage::Rgba { width, height, rgba })
        }
    }
}

fn get_panel_gump_texture(
    app: &mut UocfEditorApp,
    ctx: &egui::Context,
    source: GumpSource,
    gump_id: u32,
) -> Option<egui::TextureHandle> {
    let key = match source {
        GumpSource::Mul => MUL_GUMP_PREVIEW_KEY,
        GumpSource::CcUop => CC_UOP_GUMP_PREVIEW_KEY,
        GumpSource::EcUop => EC_GUMP_PREVIEW_KEY,
    } | u64::from(gump_id);
    if let Some(handle) = app.texture_previews.get(&key).cloned() {
        app.select_image_preview(key);
        return Some(handle);
    }

    let (width, height, pixels) = decode_gump_rgba(app, source, gump_id).ok()?;
    let label = format!("{} gump {gump_id}", source_label(app, source));
    let handle = app.checked_texture(ctx, label.clone(), [width as usize, height as usize], &pixels, Default::default())?;
    app.texture_previews.insert(key, handle.clone());
    app.register_current_image_preview(key, label, width, height, &pixels);
    Some(handle)
}

fn ui_paperdoll(
    app: &mut UocfEditorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    profiles: &[PaperdollProfile],
) {
    let gump_ids = app.paperdoll_gump_ids.clone();
    let selected_gump_id_text = app.selected_gump_id.clone();
    let mut selected_gump_id = None;
    ui_gump_split(
        ui,
        "paperdoll_gump_split",
        |left| {
            left.heading("Paperdoll Sprites");
            selected_gump_id = ui_gump_id_list(
                left,
                &gump_ids,
                &selected_gump_id_text,
                "paperdoll_sprite_list",
            );
        },
        |right| {
        right.heading("Selected Sprite");
        ui_gump_preview_controls(app, ctx, right, &gump_ids);

        right.separator();
        right.heading("Paperdoll");
        let enhanced = app.selected_gump_source == GumpSource::EcUop;
        if enhanced {
            right.label(EC_PAPERDOLL_NOTE);
        }
        egui::ComboBox::from_label("Profile")
            .selected_text(profile_by_id(profiles, &app.selected_paperdoll_profile).label.as_str())
            .show_ui(right, |ui| {
                for profile in profiles {
                    ui.selectable_value(
                        &mut app.selected_paperdoll_profile,
                        profile.id.clone(),
                        profile.label.as_str(),
                    );
                }
            });
        let selected_profile = profile_by_id(profiles, &app.selected_paperdoll_profile);
        right.monospace(format!(
            "equipment_offset={} canvas={}x{} body=({}, {}) equipment=({}, {})",
            selected_profile.equipment_offset,
            selected_profile.canvas_width,
            selected_profile.canvas_height,
            selected_profile.body_x,
            selected_profile.body_y,
            selected_profile.equipment_x,
            selected_profile.equipment_y,
        ));

        right.horizontal(|ui| {
            ui.label("Body:");
            ui.text_edit_singleline(&mut app.paperdoll_body_id);
            ui.label("Hue:");
            ui.text_edit_singleline(&mut app.paperdoll_body_hue);
            if ui.button("Use Selected Hue").clicked() {
                app.paperdoll_body_hue = app.selected_hue_id.to_string();
                app.paperdoll_preview = None;
            }
        });
        ui_hue_picker(app, right);

        egui::Grid::new("uocf_editor_paperdoll_equipment")
            .num_columns(7)
            .spacing([8.0, 4.0])
            .show(right, |ui| {
                ui.label("Slot");
                ui.label("Item ID");
                ui.label("Hue");
                ui.label("Derived");
                ui.end_row();

                let tiledata = app.cc_tiledata.as_ref().map(Arc::clone);
                let ec_tileart = app.ec_tileart_entries.as_ref().map(Arc::clone);
                let ec_frame_sets = if enhanced { cached_ec_paperdoll_frame_sets(app, ctx) } else { None };
                let body_id = parse_u32_field(&app.paperdoll_body_id).unwrap_or_default();
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
                    if enhanced {
                        ui.monospace(ec_equipment_detail(
                            ec_frame_sets.as_deref(),
                            ec_tileart.as_deref().map(Vec::as_slice),
                            tiledata.as_deref(),
                            body_id,
                            row.item_id.as_str(),
                        ));
                    } else {
                        ui.monospace(equipment_detail(
                            tiledata.as_deref(),
                            selected_profile,
                            row.slot.as_str(),
                            row.item_id.as_str(),
                        ));
                    }
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

        right.horizontal(|ui| {
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
                    Ok((handle, status)) => {
                        app.paperdoll_preview = Some(handle);
                        app.status_message = status;
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
                .show(right, |ui| {
                    ui.image(handle);
                });
        }
        },
    );
    if let Some(gump_id) = selected_gump_id {
        app.selected_gump_id = gump_id.to_string();
    }
}

fn ui_gump_id_list(
    ui: &mut egui::Ui,
    ids: &[u32],
    selected_gump_id: &str,
    id_salt: &'static str,
) -> Option<u32> {
    if ids.is_empty() {
        ui.label("No gumps are available in the selected source.");
        return None;
    }

    let selected_id = parse_u32_field(selected_gump_id).ok();
    let go_to = go_to_field(ui, (id_salt, "go_to"));
    let sort = list_sort_controls(ui, (id_salt, "gump_ids"), &["ID"], 0);
    let sorted_indices = sorted_indices_by(ids, sort.ordering(), |left, right| left.cmp(right));
    let mut selected = None;
    let visible_ids = sorted_indices
        .iter()
        .map(|index| ids[*index])
        .collect::<Vec<_>>();
    let list_rect = ui.available_rect_before_wrap();
    let keyboard_moved = if ui.rect_contains_pointer(list_rect) {
        arrow_delta(ui, go_to.has_focus)
            .and_then(|delta| move_selection(&visible_ids, selected_id, delta))
            .map(|gump_id| {
                selected = Some(gump_id);
                true
            })
            .unwrap_or(false)
    } else {
        false
    };
    let row_height = ui.text_style_height(&egui::TextStyle::Body);
    let max_height = ui.available_height().max(row_height);
    let mut area = egui::ScrollArea::vertical().id_salt(id_salt).max_height(max_height);
    if let Some(position) = go_to.position_in(visible_ids.iter().map(|&gump_id| u64::from(gump_id))) {
        selected = Some(visible_ids[position]);
        area = area.vertical_scroll_offset(centered_row_offset(ui, position, row_height));
    }
    area.show_rows(ui, row_height, sorted_indices.len(), |ui, range| {
        for index in range {
            let gump_id = ids[sorted_indices[index]];
            let response = ui.add_sized(
                egui::vec2(ui.available_width(), row_height),
                egui::Button::selectable(
                    selected_id == Some(gump_id),
                    format!("{gump_id:>5}  0x{gump_id:04X}"),
                ),
            );
            if keyboard_moved && selected == Some(gump_id) {
                response.scroll_to_me(None);
            }
            if response.clicked() {
                selected = Some(gump_id);
            }
        }
    });
    selected
}

fn refresh_gump_lists(app: &mut UocfEditorApp, profiles: &[PaperdollProfile]) {
    let mut ids = collect_available_gump_ids(app, app.selected_gump_source);
    ids.sort_unstable();
    ids.dedup();

    let profile = profile_by_id(profiles, &app.selected_paperdoll_profile);
    let paperdoll_start = profile.equipment_offset;
    let paperdoll_end = profile.equipment_offset.saturating_add(9_999);
    app.standard_gump_ids = ids
        .iter()
        .copied()
        .filter(|gump_id| !is_paperdoll_sprite_id(*gump_id))
        .collect();
    app.paperdoll_gump_ids = ids
        .into_iter()
        .filter(|gump_id| (paperdoll_start..=paperdoll_end).contains(gump_id))
        .collect();
    app.gump_list_source = Some(app.selected_gump_source);
    app.gump_list_profile = app.selected_paperdoll_profile.clone();
    app.gump_list_status = format!(
        "{} standard gumps, {} paperdoll sprites",
        app.standard_gump_ids.len(),
        app.paperdoll_gump_ids.len(),
    );
}

fn collect_available_gump_ids(app: &UocfEditorApp, source: GumpSource) -> Vec<u32> {
    match classic_container(source) {
        Some(container) => app.cc_gumps.as_ref().map_or_else(Vec::new, |gumps| gumps.available_gump_ids_in(Some(container))),
        None => collect_raw_ec_gump_ids(app),
    }
}

fn collect_raw_ec_gump_ids(app: &UocfEditorApp) -> Vec<u32> {
    let mut ids = Vec::new();
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

        for gump_id in 0..=MAX_RAW_UOP_GUMP_ID {
            if ec_gump_candidates(gump_id)
                .iter()
                .any(|(path, _)| loaded.package.get_file_by_hash(hash_file_name_single(path)).is_some())
            {
                ids.push(gump_id);
            }
        }
    }

    ids
}

fn is_paperdoll_sprite_id(gump_id: u32) -> bool {
    (PAPERDOLL_SPRITE_ID_START..=PAPERDOLL_SPRITE_ID_END).contains(&gump_id)
}

fn paperdoll_profiles() -> &'static [PaperdollProfile] {
    static PROFILES: OnceLock<Vec<PaperdollProfile>> = OnceLock::new();
    PROFILES.get_or_init(|| {
        // The file is embedded and a test decodes it, so the fallback only guards a broken build.
        match decode_paperdoll_profiles(PAPERDOLL_PROFILE_KDL) {
            Ok(profiles) => profiles,
            Err(error) => {
                log::error!("{error}; using the built-in paperdoll profiles");
                default_profiles()
            }
        }
    })
}

/// The `profile` nodes and their properties, for error messages that name them.
const PAPERDOLL_PROFILES_SCHEMA: &[udd_kdl::strict::NodeSchema] = &[udd_kdl::strict::NodeSchema::leaf(
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

fn decode_paperdoll_profiles(text: &str) -> color_eyre::eyre::Result<Vec<PaperdollProfile>> {
    let decoded: PaperdollProfilesKdl =
        udd_kdl::strict::decode("PaperdollProfiles.kdl", text, PAPERDOLL_PROFILES_SCHEMA)?;
    let profiles: Vec<_> = decoded.profiles.into_iter().map(PaperdollProfile::from).collect();
    if profiles.is_empty() {
        color_eyre::eyre::bail!("PaperdollProfiles.kdl: contains no profile nodes");
    }
    Ok(profiles)
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
        GumpSource::Mul => format!("MUL source: gumpidx.mul/gumpart.mul entry {gump_id}"),
        GumpSource::CcUop => {
            let paths = cc_gump_paths(gump_id);
            format!("CC UOP source: gumpartLegacyMUL.uop candidates {} / {}", paths[0], paths[1])
        }
        GumpSource::EcUop => {
            let path = ec_gump_path(gump_id, "tga");
            let hash = hash_file_name_single(&path);
            format!("EC source: interface.uop {path} hash={hash:016X}")
        }
    }
}

pub(super) fn ui_hue_picker(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
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

fn select_or_load_ec_gump_entry(app: &mut UocfEditorApp, gump_id: u32) -> bool {
    for (path, _) in ec_gump_candidates(gump_id) {
        let hash = hash_file_name_single(&path);
        if app.select_raw_uop_entry("interface.uop", hash) {
            return true;
        }
    }

    let Some(ec_path) = app.shown_enhanced_folder().map(Path::to_path_buf) else {
        return false;
    };
    let Some(uop_path) = ["interface.uop", "Interface.uop"]
        .into_iter()
        .map(|name| ec_path.join(name))
        .find(|path| path.exists())
    else {
        return false;
    };

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

fn select_or_load_cc_gump_entry(app: &mut UocfEditorApp, gump_id: u32) -> bool {
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

/// The Derived column of an EC item row.
fn ec_equipment_detail(
    frame_sets: Option<&FrameSets>,
    tileart: Option<&[crate::app::TileArtFileEntry]>,
    tiledata: Option<&uocf::classic::tiledata::TileData>,
    body_id: u32,
    item_id: &str,
) -> String {
    if item_id.trim().is_empty() {
        return String::new();
    }
    let Ok(item_id) = parse_u32_field(item_id) else {
        return "invalid item id".to_string();
    };
    let Some(tileart) = tileart else {
        return "tileart.uop is not loaded".to_string();
    };
    ec_paperdoll::item_detail(frame_sets, Definitions(tileart), body_id, item_id, tiledata_layer(tiledata, item_id))
}

/// The layer of an item in tiledata, if it has one.
fn tiledata_layer(tiledata: Option<&uocf::classic::tiledata::TileData>, item_id: u32) -> Option<u8> {
    let item = tiledata?.item_tiles().get(usize::try_from(item_id).ok()?)?;
    (item.quality != 0).then_some(item.quality)
}

/// The `Paperdoll.uop` frame sets of the Enhanced-format client shown, if they have been read.
fn cached_ec_paperdoll_frame_sets(app: &UocfEditorApp, ctx: &egui::Context) -> Option<Arc<FrameSets>> {
    let ec_path = app.shown_enhanced_folder()?;
    ctx.data(|data| data.get_temp::<Arc<FrameSets>>(egui::Id::new((EC_PAPERDOLL_FRAME_SETS_ID, ec_path))))
}

/// The `Paperdoll.uop` frame sets of the Enhanced-format client shown, read on first use.
fn ec_paperdoll_frame_sets(app: &UocfEditorApp, ctx: &egui::Context) -> color_eyre::eyre::Result<Arc<FrameSets>> {
    if let Some(sets) = cached_ec_paperdoll_frame_sets(app, ctx) {
        return Ok(sets);
    }
    let ec_path = app
        .shown_enhanced_folder()
        .ok_or_else(|| color_eyre::eyre::eyre!("No Enhanced Client or Kingdom Reborn folder is set."))?;
    let path = crate::app::find_client_file_case_insensitive(ec_path, PAPERDOLL_UOP_NAME)
        .ok_or_else(|| color_eyre::eyre::eyre!("No {PAPERDOLL_UOP_NAME} in {}.", ec_path.display()))?;
    let sets = Arc::new(FrameSets::load(&path)?);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new((EC_PAPERDOLL_FRAME_SETS_ID, ec_path)), Arc::clone(&sets)));
    Ok(sets)
}

/// The tables of the given hues, from the cached `hues.uop` or the one of the Enhanced-format client shown.
fn ec_paperdoll_hue_tables(app: &UocfEditorApp, hues: Vec<u16>) -> color_eyre::eyre::Result<EcHueTables> {
    if hues.iter().all(|&hue| hue == 0) {
        return Ok(EcHueTables::default());
    }
    let cached = app.uop_cache.loaded_uops.iter().find(|loaded| {
        loaded
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(HUES_UOP_NAME))
    });
    if let Some(loaded) = cached {
        return ec_paperdoll::hue_tables(&loaded.package, hues);
    }
    let path = app
        .shown_enhanced_folder()
        .and_then(|ec_path| crate::app::find_client_file_case_insensitive(ec_path, HUES_UOP_NAME))
        .ok_or_else(|| color_eyre::eyre::eyre!("No {HUES_UOP_NAME} in the Enhanced-format client folder to hue with."))?;
    ec_paperdoll::hue_tables(&UopPackage::load_with_mode(&path, LoadMode::Lazy)?, hues)
}

/// The Enhanced Client's paperdoll: `Paperdoll.uop` frames found and ordered as the client does.
fn render_ec_paperdoll(
    app: &mut UocfEditorApp,
    ctx: &egui::Context,
) -> color_eyre::eyre::Result<(egui::TextureHandle, String)> {
    let body_id = parse_u32_field(&app.paperdoll_body_id)?;
    let tileart = app
        .ec_tileart_entries
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| color_eyre::eyre::eyre!("tileart.uop of the Enhanced Client is not loaded."))?;
    let sets = ec_paperdoll_frame_sets(app, ctx)?;
    let tiledata = app.cc_tiledata.as_ref().map(Arc::clone);
    let mut items = Vec::new();
    for row in &app.paperdoll_equipment {
        if row.item_id.trim().is_empty() {
            continue;
        }
        let item_id = parse_u32_field(&row.item_id)?;
        items.push(WornItem {
            item_id,
            hue: parse_optional_hue(&row.hue),
            tiledata_layer: tiledata_layer(tiledata.as_deref(), item_id),
        });
    }
    let body_hue = parse_optional_hue(&app.paperdoll_body_hue);
    let hues = ec_paperdoll_hue_tables(app, items.iter().map(|item| item.hue).chain([body_hue]).collect())?;
    let rules = WearableRules::embedded().map_err(|error| color_eyre::eyre::eyre!("{error}"))?;
    let paperdoll = ec_paperdoll::build(&sets, Definitions(&tileart), rules, &hues, body_id, body_hue, &items)?;
    let (width, height, pixels) = ec_paperdoll::compose(&paperdoll.frames);
    let handle = super::textures::load_rgba(
        ctx,
        format!("paperdoll_{:?}_{body_id}", app.selected_gump_source),
        [width as usize, height as usize],
        &pixels,
        Default::default(),
    )
    .map_err(|problem| color_eyre::eyre::eyre!("Cannot show the paperdoll: {problem}."))?;
    app.register_current_image_preview(
        EC_PAPERDOLL_PREVIEW_KEY | u64::from(body_id),
        format!("EC paperdoll {body_id}"),
        width,
        height,
        &pixels,
    );
    for note in &paperdoll.notes {
        log::info!("EC paperdoll {body_id}: {note}");
    }
    let status = if paperdoll.notes.is_empty() {
        format!("Rendered the EC paperdoll of body {body_id}.")
    } else {
        format!("Rendered the EC paperdoll of body {body_id}; {}.", paperdoll.notes.join("; "))
    };
    Ok((handle, status))
}

fn render_paperdoll(
    app: &mut UocfEditorApp,
    ctx: &egui::Context,
) -> color_eyre::eyre::Result<(egui::TextureHandle, String)> {
    if app.selected_gump_source == GumpSource::EcUop {
        return render_ec_paperdoll(app, ctx);
    }
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
    let handle = super::textures::load_rgba(
        ctx,
        format!("paperdoll_{:?}_{}", app.selected_gump_source, app.selected_paperdoll_profile),
        [width as usize, height as usize],
        &pixels,
        Default::default(),
    )
    .map_err(|problem| color_eyre::eyre::eyre!("Cannot show the paperdoll: {problem}."))?;
    let key = PAPERDOLL_PREVIEW_KEY | u64::from(body_id);
    app.register_current_image_preview(
        key,
        format!("paperdoll {} {}", app.selected_paperdoll_profile, body_id),
        width,
        height,
        &pixels,
    );
    Ok((handle, "Rendered paperdoll preview.".to_string()))
}

fn compose_paperdoll(
    app: &UocfEditorApp,
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
    app: &UocfEditorApp,
    source: GumpSource,
    gump_id: u32,
) -> color_eyre::eyre::Result<(u32, u32, Vec<u8>)> {
    match classic_container(source) {
        Some(container) => {
            let gumps = app
                .cc_gumps
                .as_ref()
                .ok_or_else(|| color_eyre::eyre::eyre!("CC gump source is not loaded."))?;
            let mut scratch = Vec::new();
            let (width, height, pixels) = gumps.decode_gump_in(Some(container), gump_id, &mut scratch)?;
            Ok((width as u32, height as u32, pixels))
        }
        None => decode_ec_gump_rgba(app, gump_id),
    }
}

fn decode_ec_gump_rgba(
    app: &UocfEditorApp,
    gump_id: u32,
) -> color_eyre::eyre::Result<(u32, u32, Vec<u8>)> {
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

        if let Some(image) = ec_gump_rgba_in(&loaded.package, gump_id)? {
            return Ok(image);
        }
    }

    color_eyre::eyre::bail!("EC gump {gump_id} was not found in interface.uop.");
}

/// The RGBA of EC gump `gump_id` in `package` (an interface.uop); `None` when it has none.
fn ec_gump_rgba_in(package: &UopPackage, gump_id: u32) -> color_eyre::eyre::Result<Option<(u32, u32, Vec<u8>)>> {
    for (path, format) in ec_gump_candidates(gump_id) {
        let hash = hash_file_name_single(&path);
        if package.get_file_by_hash(hash).is_some() {
            let data: Arc<[u8]> = package.unpack_file_by_hash(hash)?
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
            return Ok(Some((rgba.width(), rgba.height(), rgba.into_raw())));
        }
    }
    Ok(None)
}

fn apply_hue(
    app: &UocfEditorApp,
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

pub(super) fn alpha_blend_at(
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
    fn embedded_paperdoll_profiles_decode_strictly() {
        let profiles = decode_paperdoll_profiles(PAPERDOLL_PROFILE_KDL).unwrap_or_else(|error| panic!("{error}"));
        assert!(!profiles.is_empty());
    }

    #[test]
    fn paperdoll_profiles_reject_unknown_properties_and_missing_values() {
        let error = decode_paperdoll_profiles("profile \"human_male\" equipment_offset=50000 body_z=1\n")
            .err()
            .expect("unknown property")
            .to_string();
        assert!(error.starts_with("PaperdollProfiles.kdl: line 1, column 45: unknown property `body_z`"), "{error}");
        let error = decode_paperdoll_profiles("profile \"human_male\"\n").err().expect("missing").to_string();
        assert!(error.contains("equipment_offset"), "{error}");
    }

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
