use crate::app::{ArtSource, MultiCollectionSource, MultisSource, UopInspectorApp};
use eframe::egui;
use uocf::classic::art::static_art_id_for_source;

#[derive(Clone)]
struct PreviewPart {
    item_id: u16,
    x: i16,
    y: i16,
    z: i16,
    original_z: i16,
    flags: String,
}

#[derive(Clone, Copy, Default)]
struct PartRenderInfo {
    is_floor: bool,
    width_delta: i32,
    height_delta: i32,
    offset_x: i32,
    offset_y: i32,
}

struct PreviewRenderPart {
    part: PreviewPart,
    info: PartRenderInfo,
    texture: Option<egui::TextureHandle>,
    size: egui::Vec2,
}

pub fn ui_multis(app: &mut UopInspectorApp, ctx: &egui::Context) {
    app.selected_legacy_source = concrete_art_tile_source(app.selected_legacy_source);

    let has_classic = app
        .client_data
        .as_ref()
        .and_then(|client| client.multis.as_ref())
        .is_some();
    let has_uop = app.multi_collection.is_some();
    let has_multimap = app.cc_multimap.is_some();

    if !source_available(app.multis_source, has_classic, has_uop, has_multimap) {
        app.multis_source = if has_classic {
            MultisSource::ClassicMul
        } else if has_uop {
            MultisSource::Uop
        } else {
            MultisSource::Multimap
        };
    }

    egui::TopBottomPanel::top("multis_source_tabs").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if has_classic {
                ui.selectable_value(&mut app.multis_source, MultisSource::ClassicMul, "multi.mul/.idx");
            }
            if has_uop {
                let label = format!("{} MultiCollection.uop", multi_collection_source_label(app));
                ui.selectable_value(&mut app.multis_source, MultisSource::Uop, label);
            }
            if has_multimap {
                ui.selectable_value(&mut app.multis_source, MultisSource::Multimap, "multimap.rle");
            }
            ui.separator();
            ui.label("Art:");
            let previous_source = app.selected_legacy_source;
            egui::ComboBox::from_id_salt("multis_art_source")
                .selected_text(art_source_label(app.selected_legacy_source))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.selected_legacy_source, ArtSource::Mul, "MUL");
                    ui.selectable_value(&mut app.selected_legacy_source, ArtSource::CcUop, "CC UOP");
                    ui.selectable_value(&mut app.selected_legacy_source, ArtSource::EcUopLegacy, "EC Legacy UOP");
                    ui.selectable_value(&mut app.selected_legacy_source, ArtSource::EcUopKr, "KR/New UOP");
                });
            if app.selected_legacy_source != previous_source {
                clear_multi_preview_failure_logs(app);
            }
        });
    });

    match app.multis_source {
        MultisSource::ClassicMul => ui_classic_multis(app, ctx),
        MultisSource::Uop => ui_uop_multis(app, ctx),
        MultisSource::Multimap => ui_multimap(app, ctx),
    }
}

fn source_available(source: MultisSource, has_classic: bool, has_uop: bool, has_multimap: bool) -> bool {
    match source {
        MultisSource::ClassicMul => has_classic,
        MultisSource::Uop => has_uop,
        MultisSource::Multimap => has_multimap,
    }
}

fn multi_collection_source_label(app: &UopInspectorApp) -> &'static str {
    match app.multi_collection_source {
        Some(MultiCollectionSource::ClassicClient) => "CC",
        Some(MultiCollectionSource::EnhancedClient) => "EC",
        None => "Loaded",
    }
}

fn art_source_label(source: ArtSource) -> &'static str {
    match source {
        ArtSource::Any => "CC UOP",
        ArtSource::Mul => "MUL",
        ArtSource::CcUop => "CC UOP",
        ArtSource::EcUop | ArtSource::EcUopLegacy => "EC Legacy UOP",
        ArtSource::EcUopKr => "KR/New UOP",
    }
}

fn ui_classic_multis(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::SidePanel::left("classic_multi_sidebar")
        .resizable(true)
        .default_width(250.0)
        .show(ctx, |ui| {
            ui.heading("multi.mul/.idx");
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("ID:");
                ui.add(egui::DragValue::new(&mut app.selected_multi_id));
            });

            if let Some(client) = &app.client_data {
                if let Some(multis) = &client.multis {
                    let max_id = multis.max_id();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for id in 0..max_id {
                            if ui
                                .selectable_label(app.selected_multi_id == id, format!("Multi {}", id))
                                .clicked()
                            {
                                select_multi(app, id, None);
                            }
                        }
                    });
                }
            }
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        let parts = app
            .client_data
            .as_ref()
            .and_then(|client| client.multis.as_ref())
            .and_then(|multis| multis.get_parts(app.selected_multi_id).ok())
            .unwrap_or_default();
        let preview_parts: Vec<_> = parts
            .iter()
            .map(|part| PreviewPart {
                item_id: part.item_id,
                x: part.x,
                y: part.y,
                z: part.z,
                original_z: part.z,
                flags: format!("0x{:08X}", part.flags),
            })
            .collect();
        draw_multi_details(app, ctx, ui, "Classic Multi", &preview_parts, None);
    });
}

fn ui_uop_multis(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(collection) = app.multi_collection.clone() else {
        return;
    };

    egui::SidePanel::left("uop_multi_sidebar")
        .resizable(true)
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.heading(format!(
                "{} MultiCollection.uop ({})",
                multi_collection_source_label(app),
                collection.items.len()
            ));
            if let Some(path) = &app.multi_collection_path {
                ui.monospace(path.display().to_string());
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut app.search_query);
            });
            ui.separator();

            let query = app.search_query.to_lowercase();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for item in &collection.items {
                    if !query.is_empty()
                        && !item.id.to_string().contains(&query)
                        && !item.path.to_lowercase().contains(&query)
                        && !format!("{:016X}", item.filename_hash).to_lowercase().contains(&query)
                    {
                        continue;
                    }

                    if ui
                        .selectable_label(
                            app.selected_multi_id == item.id,
                            format!("Multi {} ({:016X})", item.id, item.filename_hash),
                        )
                        .clicked()
                    {
                        select_multi(app, item.id, Some(item.filename_hash));
                    }
                }
            });
        });

    egui::TopBottomPanel::top("multi_uop_raw_tabs").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut app.multis_source, MultisSource::Uop, "Specialized");
            if ui.button("Raw selected").clicked() {
                app.select_raw_multi_collection_entry(app.selected_multi_id);
            }
            if collection.housing_hash.is_some() && ui.button("Raw housing").clicked() {
                app.select_raw_multi_collection_housing();
            }
        });
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(item) = collection.get(app.selected_multi_id) {
            let preview_parts: Vec<_> = item
                .parts
                .iter()
                .map(|part| PreviewPart {
                    item_id: part.item_id,
                    x: part.x,
                    y: part.y,
                    z: part.z,
                    original_z: part.z,
                    flags: format!(
                        "0x{:04X} / {} clilocs",
                        part.flags,
                        part.cliloc_offsets.len()
                    ),
                })
                .collect();
            let raw = Some(format!(
                "{} / {} / 0x{:016X}",
                multi_collection_source_label(app),
                item.path,
                item.filename_hash
            ));
            draw_multi_details(app, ctx, ui, "UOP Multi", &preview_parts, raw.as_deref());
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a MultiCollection.uop entry from the left panel.");
            });
        }
    });
}

fn select_multi(app: &mut UopInspectorApp, multi_id: u32, uop_hash: Option<u64>) {
    if app.selected_multi_id != multi_id || app.selected_multi_uop_hash != uop_hash {
        app.selected_multi_id = multi_id;
        app.selected_multi_uop_hash = uop_hash;
        clear_multi_preview_failure_logs(app);
    }
}

fn ui_multimap(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(multimap) = app.cc_multimap.clone() else {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("multimap.rle is not loaded.");
            });
        });
        return;
    };

    let black_pixels = multimap
        .pixels
        .iter()
        .filter(|&&pixel| pixel == uocf::classic::multimap_rle::BLACK_PIXEL)
        .count();
    let total_pixels = multimap.pixels.len();
    let white_pixels = total_pixels.saturating_sub(black_pixels);

    egui::SidePanel::left("multimap_sidebar")
        .resizable(true)
        .default_width(260.0)
        .show(ctx, |ui| {
            ui.heading("multimap.rle");
            ui.separator();
            egui::Grid::new("multimap_details").striped(true).show(ui, |ui| {
                ui.label("Dimensions");
                ui.label(format!("{} x {}", multimap.width, multimap.height));
                ui.end_row();

                ui.label("Pixels");
                ui.label(total_pixels.to_string());
                ui.end_row();

                ui.label("Black");
                ui.label(black_pixels.to_string());
                ui.end_row();

                ui.label("White");
                ui.label(white_pixels.to_string());
                ui.end_row();

                ui.label("Source");
                if let Some(path) = &app.cc_multimap_path {
                    ui.label(path.display().to_string());
                } else {
                    ui.label("Loaded from Classic Client path");
                }
                ui.end_row();
            });
            ui.separator();
            ui.add(
                egui::Slider::new(&mut app.multimap_zoom, 0.05..=1.0)
                    .logarithmic(true)
                    .text("Zoom"),
            );
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(handle) = app.get_multimap_texture(ctx) {
            let size = egui::vec2(
                multimap.width as f32 * app.multimap_zoom,
                multimap.height as f32 * app.multimap_zoom,
            );
            egui::ScrollArea::both().show(ui, |ui| {
                ui.add(egui::Image::new(&handle).fit_to_exact_size(size));
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Failed to build multimap texture.");
            });
        }
    });
}

fn draw_multi_details(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    title: &str,
    parts: &[PreviewPart],
    raw_label: Option<&str>,
) {
    if parts.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(format!("Multi {} is empty or missing", app.selected_multi_id));
        });
        return;
    }

    ui.heading(format!("{} {}: {} parts", title, app.selected_multi_id, parts.len()));
    if let Some(raw_label) = raw_label {
        ui.monospace(raw_label);
    }
    ui.separator();

    let available = ui.available_size();
    ui.horizontal(|ui| {
        let component_width = if available.x < 700.0 {
            available.x.min(320.0)
        } else {
            (available.x * 0.32).clamp(320.0, 480.0)
        };
        ui.allocate_ui_with_layout(
            egui::vec2(component_width, available.y),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.heading("Components");
                egui::ScrollArea::vertical()
                    .id_salt(("multi_parts_scroll", app.multis_source as u8, app.selected_multi_id))
                    .auto_shrink([false, false])
                    .max_height(ui.available_height())
                    .show(ui, |ui| {
                        egui::Grid::new("multi_parts_grid").striped(true).show(ui, |ui| {
                            ui.label("Item ID");
                            ui.label("X");
                            ui.label("Y");
                            ui.label("Z");
                            ui.label("Flags");
                            ui.end_row();
                            for part in parts {
                                ui.label(part.item_id.to_string());
                                ui.label(part.x.to_string());
                                ui.label(part.y.to_string());
                                ui.label(part.z.to_string());
                                ui.label(&part.flags);
                                ui.end_row();
                            }
                        });
                    });
            },
        );

        ui.separator();

        let preview_size = ui.available_size();
        ui.allocate_ui_with_layout(
            preview_size,
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.heading("2D Preview");
                let preview_available = ui.available_size();
                ui.allocate_ui_with_layout(
                    preview_available,
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        draw_preview(app, ctx, ui, parts);
                    },
                );
            },
        );
    });
}

fn draw_preview(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, parts: &[PreviewPart]) {
    let (min_x, max_x, min_y, max_y) = parts.iter().fold(
        (0i16, 0i16, 0i16, 0i16),
        |(min_x, max_x, min_y, max_y), p| {
            (min_x.min(p.x), max_x.max(p.x), min_y.min(p.y), max_y.max(p.y))
        },
    );
    let (min_z, max_z) = parts.iter().fold((0i16, 0i16), |(min_z, max_z), p| {
        (min_z.min(p.z), max_z.max(p.z))
    });
    let span_x = ((max_x as i32) - (min_x as i32) + 1).max(1);
    let span_y = ((max_y as i32) - (min_y as i32) + 1).max(1);
    let span_z = ((max_z as i32) - (min_z as i32)).max(0);
    let tile_w = 22.0f32;
    let tile_h = 22.0f32;
    let viewport_size = ui.available_size();
    let mut sorted_parts: Vec<_> = parts
        .iter()
        .map(|part| {
            let info = part_render_info(app, part.item_id);
            let texture = get_multi_part_texture(app, ctx, part.item_id);
            let size = texture
                .as_ref()
                .map(|handle| handle.size_vec2())
                .unwrap_or_else(|| egui::vec2(14.0, 14.0));
            PreviewRenderPart {
                part: part.clone(),
                info,
                texture,
                size,
            }
        })
        .collect();
    sorted_parts.sort_by(|a, b| {
        a.part.z.cmp(&b.part.z)
            .then(a.part.y.cmp(&b.part.y))
            .then(a.part.x.cmp(&b.part.x))
            .then((!a.info.is_floor).cmp(&(!b.info.is_floor)))
    });

    let padding = 96.0f32;
    let mut content_min = egui::pos2(f32::INFINITY, f32::INFINITY);
    let mut content_max = egui::pos2(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for render_part in &sorted_parts {
        let pos = preview_part_position(render_part, min_x, min_y, min_z, tile_w, tile_h);
        content_min.x = content_min.x.min(pos.x);
        content_min.y = content_min.y.min(pos.y);
        content_max.x = content_max.x.max(pos.x + render_part.size.x);
        content_max.y = content_max.y.max(pos.y + render_part.size.y);
    }
    if !content_min.x.is_finite() {
        content_min = egui::pos2(0.0, 0.0);
        content_max = egui::pos2(
            (span_x as f32 + span_y as f32) * tile_w,
            (span_x as f32 + span_y as f32) * tile_h + span_z as f32 * 4.0,
        );
    }

    let content_w = (content_max.x - content_min.x + padding * 2.0).max(1.0);
    let content_h = (content_max.y - content_min.y + padding * 2.0).max(1.0);
    let canvas_w = content_w.max(viewport_size.x.max(1.0));
    let canvas_h = content_h.max(viewport_size.y.max(1.0));
    let canvas_size = egui::vec2(canvas_w, canvas_h);

    egui::ScrollArea::both()
        .id_salt(("multi_preview_scroll", app.multis_source as u8, app.selected_multi_id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
        let (rect, _response) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(24));

        let origin = rect.min
            + egui::vec2(
                (canvas_w - content_w).max(0.0) * 0.5 + padding - content_min.x,
                (canvas_h - content_h).max(0.0) * 0.5 + padding - content_min.y,
            );

        for render_part in sorted_parts {
            let part_pos = preview_part_position(&render_part, min_x, min_y, min_z, tile_w, tile_h);
            if let Some(handle) = render_part.texture {
                let draw_pos = rect.min + egui::vec2(
                    origin.x - rect.min.x + part_pos.x,
                    origin.y - rect.min.y + part_pos.y,
                );
                let part_rect = egui::Rect::from_min_size(draw_pos, render_part.size);
                painter.image(
                    handle.id(),
                    part_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                let fallback_pos = origin + part_pos.to_vec2();
                let part_rect = egui::Rect::from_center_size(
                    fallback_pos,
                    render_part.size,
                );
                painter.rect_filled(part_rect.shrink(1.0), 2.0, egui::Color32::BLUE);
            }
        }
    });
}

fn preview_part_position(
    render_part: &PreviewRenderPart,
    min_x: i16,
    min_y: i16,
    min_z: i16,
    tile_w: f32,
    tile_h: f32,
) -> egui::Pos2 {
    let part = &render_part.part;
    let info = render_part.info;
    let x = ((part.x as i32) - (min_x as i32)) as f32;
    let y = ((part.y as i32) - (min_y as i32)) as f32;
    let base_x = (x - y) * tile_w + info.height_delta as f32;
    let base_y = (x + y) * tile_h
        + info.width_delta as f32
        + info.height_delta as f32
        - (part.original_z as f32 * 4.0);

    egui::pos2(
        base_x + info.offset_x as f32,
        base_y + info.offset_y as f32 + (min_z as f32 * 4.0),
    )
}

fn get_multi_part_texture(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    item_id: u16,
) -> Option<egui::TextureHandle> {
    let source = concrete_art_tile_source(app.selected_legacy_source);
    let art_id = static_art_id_for_source(item_id as u32, source);
    let selected_hue_id = app.selected_hue_id;
    app.selected_hue_id = 0;
    let texture = app.get_tex_art_texture_from_source(ctx, art_id, source);
    app.selected_hue_id = selected_hue_id;
    if texture.is_none() {
        log_missing_multi_part_texture(app, item_id, art_id, source);
    }
    texture
}

fn log_missing_multi_part_texture(
    app: &mut UopInspectorApp,
    item_id: u16,
    art_id: u32,
    source: ArtSource,
) {
    let has_art = app
        .client_data
        .as_ref()
        .map(|client| client.art.has_id_from_source(art_id, source))
        .unwrap_or(false);
    let message = if app.client_data.is_none() {
        format!(
            "Multi preview cannot render item 0x{item_id:04X}: no CC art map is loaded for {:?}.",
            source
        )
    } else if has_art {
        format!(
            "Multi preview failed to decode item 0x{item_id:04X} as art 0x{art_id:04X} from {:?}.",
            source
        )
    } else {
        format!(
            "Multi preview missing item 0x{item_id:04X}: art 0x{art_id:04X} is not present in {:?}.",
            source
        )
    };
    if !app.logs.iter().any(|log| log == &message) {
        app.log(message);
    }
}

fn concrete_art_tile_source(source: ArtSource) -> ArtSource {
    match source {
        ArtSource::Any => ArtSource::CcUop,
        ArtSource::EcUop => ArtSource::EcUopLegacy,
        source => source,
    }
}

fn clear_multi_preview_failure_logs(app: &mut UopInspectorApp) {
    app.logs
        .retain(|log| !log.starts_with("Multi preview "));
}

fn part_render_info(app: &UopInspectorApp, item_id: u16) -> PartRenderInfo {
    let mut info = PartRenderInfo::default();

    if let Some(client) = &app.client_data {
        if let Some(tile) = client.tiledata.item_tiles().get(item_id as usize) {
            info.is_floor = tile.flags.surface() && !tile.flags.bridge();
        }
    }

    if let Some(entries) = &app.ec_tileart_entries {
        if let Some(file) = entries.iter().find(|file| file.entry.tile_id == item_id as u32) {
            info.width_delta = file.entry.cc_img_offset.y_start - file.entry.cc_img_offset.y_end;
            info.height_delta = file.entry.cc_img_offset.x_start;
            info.offset_x = file.entry.cc_img_offset.x_off;
            info.offset_y = file.entry.cc_img_offset.y_off;
        }
    }

    info
}
