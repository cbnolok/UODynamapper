use crate::app::{concrete_art_tile_source, ArtSource, EcHueingMode, HuesSource, UopInspectorApp};
use eframe::egui;
use super::{arrow_delta, list_sort_controls, move_selection, sorted_indices_by};
use uocf::classic::art::{static_art_id_for_source, STATIC_TILE_ID_BASE};
use uocf::enhanced::hues::{
    atlas_coord_for_hue, hue_bitmap_path, HUE_STRIP_WIDTH, HUES_ATLAS_HEIGHT, HUES_ATLAS_PATH,
    HUENAMES_PATH, FIXED_PALETTE_HASH, FIXED_PALETTE_NAME, MAX_EC_HUES,
};

const CC_HUES_LIST_PANEL_WIDTH: f32 = 300.0;
const EC_HUES_LIST_PANEL_WIDTH: f32 = 320.0;

pub fn ui_hues(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let has_cc_hues = app
        .client_data
        .as_ref()
        .and_then(|client| client.hues.as_ref())
        .is_some();
    let has_ec_hues = app.ec_hues.is_some();

    if app.hues_source == HuesSource::CcMul && !has_cc_hues && has_ec_hues {
        app.hues_source = HuesSource::EcUop;
    } else if app.hues_source == HuesSource::EcUop && !has_ec_hues && has_cc_hues {
        app.hues_source = HuesSource::CcMul;
    }

    egui::Panel::top("hues_file_tabs").show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            if has_cc_hues {
                ui.selectable_value(&mut app.hues_source, HuesSource::CcMul, "hues.mul");
            }
            if has_ec_hues {
                ui.selectable_value(&mut app.hues_source, HuesSource::EcUop, "hues.uop");
            }
        });
    });

    match app.hues_source {
        HuesSource::CcMul => ui_cc_hues(app, ctx, ui),
        HuesSource::EcUop => ui_ec_hues(app, ctx, ui),
    }
}

fn ui_cc_hues(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::Panel::left("cc_hues_list")
        .resizable(true)
        .default_size(CC_HUES_LIST_PANEL_WIDTH)
        .show_inside(ui, |ui| {
            ui.heading("hues.mul Entries");
            let sort = list_sort_controls(ui, "cc_hues", &["ID", "Name"], 0);
            ui.separator();

            if let Some(client) = &app.client_data {
                if let Some(hues) = &client.hues {
                    let hue_ids: Vec<u16> = (1..=hues.len() as u16).collect();
                    let sorted_indices = sorted_indices_by(&hue_ids, sort.ordering(), |left, right| {
                        match sort.option_index {
                            1 => {
                                let left_name = hues
                                    .get((*left as usize).saturating_sub(1))
                                    .map(|hue| String::from_utf8_lossy(&hue.name).trim_matches('\0').to_ascii_lowercase())
                                    .unwrap_or_default();
                                let right_name = hues
                                    .get((*right as usize).saturating_sub(1))
                                    .map(|hue| String::from_utf8_lossy(&hue.name).trim_matches('\0').to_ascii_lowercase())
                                    .unwrap_or_default();
                                left_name.cmp(&right_name)
                            }
                            _ => left.cmp(right),
                        }
                    });
                    let visible_hues: Vec<u16> = sorted_indices.iter().map(|index| hue_ids[*index]).collect();
                    let keyboard_moved = if let Some(delta) = arrow_delta(ui, false) {
                        if let Some(hue_id) =
                            move_selection(&visible_hues, Some(app.selected_hue_id), delta)
                        {
                            app.selected_hue_id = hue_id;
                        }
                        true
                    } else {
                        false
                    };

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for hue_id in visible_hues {
                            let Some(hue) = hues.get((hue_id as usize).saturating_sub(1)) else {
                                continue;
                            };
                            let name = String::from_utf8_lossy(&hue.name).trim_matches('\0').to_string();
                            let label = if name.is_empty() {
                                format!("Hue {}", hue_id)
                            } else {
                                format!("Hue {}: {}", hue_id, name)
                            };

                            let selected = app.selected_hue_id == hue_id;
                            let response = ui.selectable_label(selected, label);
                            if keyboard_moved && selected {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if response.clicked() {
                                app.selected_hue_id = hue_id;
                            }
                        }
                    });
                } else {
                    ui.label("hues.mul not loaded");
                }
            } else {
                ui.label("Load client data first");
            }
        });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        if let Some(client) = &app.client_data {
            if let Some(hues) = &client.hues {
                let idx = (app.selected_hue_id as usize).saturating_sub(1);
                if let Some(hue) = hues.get(idx) {
                    ui.heading(format!("Hue {}: {}", app.selected_hue_id, String::from_utf8_lossy(&hue.name).trim_matches('\0')));
                    ui.separator();

                    ui.label("Color Table (32 colors):");
                    egui::ScrollArea::horizontal()
                        .id_salt("cc_hue_color_table_strip")
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for &color in &hue.color_table {
                                    let r = (((color >> 10) & 0x1F) << 3) as u8;
                                    let g = (((color >> 5) & 0x1F) << 3) as u8;
                                    let b = ((color & 0x1F) << 3) as u8;

                                    let (rect, _response) = ui.allocate_at_least(
                                        egui::vec2(24.0, 24.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(
                                        rect,
                                        2.0,
                                        egui::Color32::from_rgb(r, g, b),
                                    );
                                }
                            });
                        });

                    ui.separator();
                    ui.label("Properties:");
                    ui.label(format!("Start: {}", hue.table_start));
                    ui.label(format!("End: {}", hue.table_end));

                    ui_cc_hue_item_preview(app, ctx, ui);
                }
            }
        }
    });
}

fn ui_hue_preview_item_picker(app: &mut UopInspectorApp, ui: &mut egui::Ui) -> u32 {
    app.selected_legacy_source = concrete_art_tile_source(app.selected_legacy_source);

    ui.horizontal(|ui| {
        ui.label("Item source:");
        egui::ComboBox::from_id_salt("hue_preview_item_source")
            .selected_text(match app.selected_legacy_source {
                ArtSource::Mul => "CC MUL",
                ArtSource::CcUop => "CC UOP",
                ArtSource::EcUop | ArtSource::EcUopLegacy => "EC Legacy UOP",
                ArtSource::EcUopKr => "KR/New UOP",
                ArtSource::Any => "CC UOP",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut app.selected_legacy_source, ArtSource::Mul, "CC MUL");
                ui.selectable_value(&mut app.selected_legacy_source, ArtSource::CcUop, "CC UOP");
                ui.selectable_value(&mut app.selected_legacy_source, ArtSource::EcUopLegacy, "EC Legacy UOP");
                ui.selectable_value(&mut app.selected_legacy_source, ArtSource::EcUopKr, "KR/New UOP");
            });
    });

    let selected_art_id = app
        .selected_tex_art_cc_id
        .unwrap_or_else(|| static_art_id_for_source(0, app.selected_legacy_source));
    let mut item_id = if app.selected_legacy_source.is_ec_uop() {
        selected_art_id
    } else {
        selected_art_id.saturating_sub(STATIC_TILE_ID_BASE)
    };
    ui.horizontal(|ui| {
        ui.label("Item ID:");
        if ui
            .add(egui::DragValue::new(&mut item_id).range(0..=0xFFFF).speed(1))
            .changed()
        {
            app.selected_tex_art_cc_id = Some(static_art_id_for_source(item_id, app.selected_legacy_source));
        }
    });
    let art_id = static_art_id_for_source(item_id, app.selected_legacy_source);
    app.selected_tex_art_cc_id = Some(art_id);
    art_id
}

fn ui_cc_hue_item_preview(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.separator();
    ui.label("Preview item:");
    let art_id = ui_hue_preview_item_picker(app, ui);
    if let Some(handle) = app.get_tex_art_texture_from_source(ctx, art_id, app.selected_legacy_source) {
        ui.image(&handle);
    } else {
        ui.label("Item art not found for selected source.");
    }
}

fn ui_ec_hue_item_preview(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    hue_id: u16,
) {
    ui.separator();
    ui.label("Preview item:");
    ui.horizontal(|ui| {
        ui.label("Hueing mode:");
        ui.selectable_value(&mut app.selected_ec_hueing_mode, EcHueingMode::Cc, "CC");
        ui.selectable_value(&mut app.selected_ec_hueing_mode, EcHueingMode::Ec, "EC");
    });
    let art_id = ui_hue_preview_item_picker(app, ui);
    if let Some(handle) =
        app.get_tex_art_texture_with_ec_hue_from_source(ctx, art_id, app.selected_legacy_source, hue_id)
    {
        let response = ui_nearest_clickable_texture(ui, &handle, handle.size_vec2());
        if response.clicked() {
            app.select_raw_art_entry(art_id, app.selected_legacy_source);
        }
    } else {
        ui.label("Item art or EC hue bitmap not found for selected source.");
    }
}

fn ui_nearest_clickable_texture(
    ui: &mut egui::Ui,
    handle: &egui::TextureHandle,
    size: egui::Vec2,
) -> egui::Response {
    ui.add(
        egui::Image::new(handle)
            .fit_to_exact_size(size)
            .texture_options(egui::TextureOptions::NEAREST)
            .sense(egui::Sense::click()),
    )
}

fn fit_texture_to_width(handle: &egui::TextureHandle, width: f32) -> egui::Vec2 {
    let [texture_width, texture_height] = handle.size();
    if texture_width == 0 || texture_height == 0 {
        return egui::vec2(width.max(1.0), 1.0);
    }

    let width = width.max(1.0);
    let height = width * texture_height as f32 / texture_width as f32;
    egui::vec2(width, height.max(1.0))
}

fn ec_hue_from_atlas_response(
    response: &egui::Response,
    texture_size: [usize; 2],
) -> Option<u16> {
    let pos = response.interact_pointer_pos()?;
    if texture_size[0] == 0 || texture_size[1] == 0 || response.rect.width() <= 0.0 {
        return None;
    }
    if response.rect.height() <= 0.0 {
        return None;
    }

    let uv_x = ((pos.x - response.rect.min.x) / response.rect.width()).clamp(0.0, 0.999_999);
    let uv_y = ((pos.y - response.rect.min.y) / response.rect.height()).clamp(0.0, 0.999_999);
    let tex_x = (uv_x * texture_size[0] as f32).floor() as u32;
    let tex_y = (uv_y * texture_size[1] as f32).floor() as u32;
    let column = tex_x / HUE_STRIP_WIDTH;
    let row = tex_y.min(HUES_ATLAS_HEIGHT - 1);
    let hue_id = if column == 0 {
        row
    } else {
        HUES_ATLAS_HEIGHT + (column - 1) * HUES_ATLAS_HEIGHT + row
    };

    if (1..=MAX_EC_HUES as u32).contains(&hue_id) {
        Some(hue_id as u16)
    } else {
        None
    }
}

fn ui_ec_hues(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(ec_hues_arc) = app.ec_hues.clone() else {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Select an Enhanced Client path containing hues.uop.");
            });
        });
        return;
    };
    let ec_hues = ec_hues_arc.as_ref();

    egui::Panel::left("ec_hues_list")
        .resizable(true)
        .default_size(EC_HUES_LIST_PANEL_WIDTH)
        .show_inside(ui, |ui| {
            ui.heading(format!("hues.uop Entries ({})", ec_hues.bitmaps.len()));
            ui.separator();

            let mut text_has_focus = false;
            ui.horizontal(|ui| {
                ui.label("Search:");
                let res = ui.text_edit_singleline(&mut app.search_query);
                text_has_focus |= res.has_focus();
            });
            let sort = list_sort_controls(ui, "ec_hues", &["ID", "Name", "Path", "Hash"], 0);
            ui.separator();

            let query = app.search_query.to_lowercase();
            let hue_ids: Vec<u16> = (1..=MAX_EC_HUES).collect();
            let sorted_indices = sorted_indices_by(&hue_ids, sort.ordering(), |left, right| {
                match sort.option_index {
                    1 => ec_hues
                        .hue_name(*left)
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .cmp(&ec_hues.hue_name(*right).unwrap_or("").to_ascii_lowercase()),
                    2 => hue_bitmap_path(*left).cmp(&hue_bitmap_path(*right)),
                    3 => ec_hues
                        .bitmap_for_hue(*left)
                        .map(|entry| entry.filename_hash)
                        .unwrap_or(u64::MAX)
                        .cmp(
                            &ec_hues
                                .bitmap_for_hue(*right)
                                .map(|entry| entry.filename_hash)
                                .unwrap_or(u64::MAX),
                        ),
                    _ => left.cmp(right),
                }
            });
            let visible_hues: Vec<u16> = sorted_indices
                .iter()
                .map(|index| hue_ids[*index])
                .filter(|hue_id| {
                    let name = ec_hues.hue_name(*hue_id).unwrap_or("");
                    let path = hue_bitmap_path(*hue_id);
                    query.is_empty()
                        || hue_id.to_string().contains(&query)
                        || name.to_lowercase().contains(&query)
                        || path.to_lowercase().contains(&query)
                })
                .collect();
            let keyboard_moved = if let Some(delta) = arrow_delta(ui, text_has_focus) {
                if let Some(hue_id) = move_selection(&visible_hues, Some(app.selected_ec_hue_id), delta) {
                    app.selected_ec_hue_id = hue_id;
                    app.selected_ec_hue_hash =
                        ec_hues.bitmap_for_hue(hue_id).map(|entry| entry.filename_hash);
                }
                true
            } else {
                false
            };

            egui::ScrollArea::vertical().show(ui, |ui| {
                for hue_id in visible_hues {
                    let name = ec_hues.hue_name(hue_id).unwrap_or("");

                    let label = if name.is_empty() {
                        format!("Hue {}", hue_id)
                    } else {
                        format!("Hue {}: {}", hue_id, name)
                    };

                    let selected = app.selected_ec_hue_id == hue_id;
                    let response = ui.selectable_label(selected, label);
                    if keyboard_moved && selected {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        app.selected_ec_hue_id = hue_id;
                        app.selected_ec_hue_hash =
                            ec_hues.bitmap_for_hue(hue_id).map(|entry| entry.filename_hash);
                    }
                }
            });
        });

    egui::Panel::top("ec_hues_raw_tabs").show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut app.hues_source, HuesSource::EcUop, "Specialized");
            if ui.button("Raw selected BMP").clicked() {
                app.select_raw_ec_hue_bitmap(app.selected_ec_hue_id);
            }
            if ui.button("Raw atlas").clicked() {
                app.select_raw_ec_hues_atlas();
            }
            if ui.button("Raw names").clicked() {
                app.select_raw_ec_huenames();
            }
            if ui.button("Raw palette").clicked() {
                app.select_raw_ec_hue_palette();
            }
        });
    });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("EC Hues");
                ui.separator();
                ui.label(format!("{} bitmap entries", ec_hues.bitmaps.len()));
                ui.label(format!("{} names", ec_hues.names.len()));
            });
            ui.separator();

            let hue_id = app.selected_ec_hue_id.clamp(1, MAX_EC_HUES);
            if app.selected_ec_hue_id != hue_id {
                app.selected_ec_hue_id = hue_id;
            }

            ui.heading(format!(
                "Hue {}{}",
                hue_id,
                ec_hues
                    .hue_name(hue_id)
                    .filter(|name| !name.is_empty())
                    .map(|name| format!(": {}", name))
                    .unwrap_or_default()
            ));

            egui::Grid::new("ec_hue_details").striped(true).show(ui, |ui| {
                ui.label("BMP path");
                ui.monospace(hue_bitmap_path(hue_id));
                ui.end_row();

                if let Some(entry) = ec_hues.bitmap_for_hue(hue_id) {
                    ui.label("BMP hash");
                    ui.monospace(format!("0x{:016X}", entry.filename_hash));
                    ui.end_row();
                    ui.label("BMP bytes");
                    ui.label(entry.byte_len.to_string());
                    ui.end_row();
                }

                if let Some(coord) = atlas_coord_for_hue(hue_id) {
                    ui.label("Atlas strip");
                    ui.label(format!(
                        "column {}, row {}, x {}, y {}",
                        coord.column, coord.row, coord.x, coord.y
                    ));
                    ui.end_row();
                }

                if let Some(hash) = ec_hues.atlas_hash {
                    ui.label("Atlas hash");
                    ui.monospace(format!("0x{:016X}", hash));
                    ui.end_row();
                }

                if let Some(hash) = ec_hues.huenames_hash {
                    ui.label("Names hash");
                    ui.monospace(format!("0x{:016X}", hash));
                    ui.end_row();
                }

                ui.label("Fixed palette hash");
                ui.monospace(format!("0x{:016X}", FIXED_PALETTE_HASH));
                ui.end_row();
            });

            ui.separator();
            ui.label("Selected hue BMP:");
            if let Some(entry) = ec_hues.bitmap_for_hue(hue_id) {
                if let Some(handle) = app.get_hues_uop_texture(ctx, entry.filename_hash, &entry.path) {
                    let size = fit_texture_to_width(&handle, ui.available_width());
                    let response = ui_nearest_clickable_texture(ui, &handle, size);
                    if response.clicked() {
                        app.select_raw_ec_hue_bitmap(hue_id);
                    }
                } else {
                    ui.label("Preview unavailable");
                }
            } else {
                ui.label("BMP entry not present in hues.uop");
            }

            ui_ec_hue_item_preview(app, ctx, ui, hue_id);

            ui.separator();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(HUES_ATLAS_PATH);
                    if let Some(hash) = ec_hues.atlas_hash {
                        if let Some(handle) = app.get_hues_uop_texture(ctx, hash, HUES_ATLAS_PATH) {
                            let response = ui_nearest_clickable_texture(ui, &handle, egui::vec2(256.0, 256.0));
                            if response.clicked() {
                                if let Some(clicked_hue_id) =
                                    ec_hue_from_atlas_response(&response, handle.size())
                                {
                                    app.selected_ec_hue_id = clicked_hue_id;
                                    app.selected_ec_hue_hash = ec_hues
                                        .bitmap_for_hue(clicked_hue_id)
                                        .map(|entry| entry.filename_hash);
                                }
                                app.select_raw_ec_hues_atlas();
                            }
                        } else {
                            ui.label("Preview unavailable");
                        }
                    } else {
                        ui.label("Not present");
                    }
                });

                ui.separator();

                ui.vertical(|ui| {
                    ui.label(FIXED_PALETTE_NAME);
                    if let Some(hash) = ec_hues.fixed_palette_hash {
                        if let Some(handle) = app.get_hues_uop_texture(ctx, hash, FIXED_PALETTE_NAME) {
                            let response = ui_nearest_clickable_texture(ui, &handle, egui::vec2(256.0, 256.0));
                            if response.clicked() {
                                app.select_raw_ec_hue_palette();
                            }
                        } else {
                            ui.label("Preview unavailable");
                        }
                    } else {
                        ui.label("Not present");
                    }
                });
            });

            ui.separator();
            ui.label(HUENAMES_PATH);
            ui.label(ec_hues
                    .huenames_byte_len
                    .map(|len| format!("{len} bytes"))
                    .unwrap_or_else(|| "Not present".to_string()).to_string());
        });
    });
}
