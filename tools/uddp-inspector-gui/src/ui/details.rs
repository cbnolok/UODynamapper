use eframe::egui;
use udd_container::{FileKey, xxh64_virtual_path};
use crate::app::{InspectorApp, MAX_INLINE_PREVIEW_WIDTH, MAX_INLINE_PREVIEW_HEIGHT};
use crate::models::{PreviewModeKind, ViewMode, VirtualEntryData};
use crate::utils::{atlas_page_paths, data_type_to_str, format_size};

impl InspectorApp {
    pub fn ui_details(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, idx: usize) {
        ui.horizontal(|ui| {
            ui.heading("Entry Details");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.view_mode == ViewMode::Package {
                    if ui.button("💾 Extract").clicked() {
                        self.extract_payload(idx);
                    }
                }
                if ui.button("✖").clicked() {
                    self.selected_idx = None;
                    self.selected_virtual_idx = None;
                }
            });
        });
        ui.add_space(5.0);

        // Render mode-specific metadata
        match self.view_mode {
            ViewMode::Package => self.ui_details_package(ctx, ui, idx),
            ViewMode::Virtual => self.ui_details_virtual(ctx, ui, idx),
            ViewMode::MobileAnimCc | ViewMode::MobileAnimEc => {}
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);
        ui.heading("Content Preview");

        // Image/Text preview section
        if let Some((texture, size, label)) = self.active_preview_image() {
            ui.label(label);
            if size[0] > MAX_INLINE_PREVIEW_WIDTH || size[1] > MAX_INLINE_PREVIEW_HEIGHT {
                ui.label(format!(
                    "Image is {}x{}, which is too large for the side panel. Open it in the separate window.",
                    size[0], size[1]
                ));
                if ui.button("Open Image Window").clicked() {
                    self.image_window_mode = self.preview_mode;
                    self.image_window_open = true;
                }
            } else {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.image(texture);
                });
            }
        } else if let Some(text) = &self.preview_text {
            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 20.0)
                .show(ui, |ui| {
                    let mut t = text.as_str();
                    ui.add(
                        egui::TextEdit::multiline(&mut t)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
        } else {
            ui.label("Loading preview...");
        }
    }

    fn ui_details_package(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, idx: usize) {
        let (data_type, key, codec, raw_size, stored_size) = {
            let entry = &self.entries[idx];
            (
                entry.data_type,
                entry.key,
                entry.codec,
                entry.raw_size,
                entry.stored_size,
            )
        };
        let is_texture_type = data_type == 1 || data_type == 9;

        // Header buttons for image-enabled entries
        if self.preview_texture.is_some() || self.atlas_texture.is_some() || is_texture_type {
            let linked_v_idx = self.find_linked_virtual_entry(idx);

            ui.horizontal(|ui| {
                if self.atlas_texture.is_some() {
                    ui.selectable_value(
                        &mut self.preview_mode,
                        PreviewModeKind::Entry,
                        "Entry View",
                    );
                    ui.selectable_value(
                        &mut self.preview_mode,
                        PreviewModeKind::Atlas,
                        "Full Atlas",
                    );
                }

                if ui
                    .button(egui::RichText::new("🖼 Open Image Window").strong())
                    .clicked()
                {
                    self.image_window_mode = self.preview_mode;
                    self.image_window_open = true;
                }

                if let Some(v_idx) = linked_v_idx {
                    if ui
                        .button(egui::RichText::new("🔍 Show Atlas").color(egui::Color32::LIGHT_BLUE))
                        .on_hover_text(
                            "This entry is linked to virtual assets. Click to view them in the atlas.",
                        )
                        .clicked()
                    {
                        self.view_mode = ViewMode::Virtual;
                        self.select_virtual_entry(ctx, v_idx);
                        self.preview_mode = PreviewModeKind::Atlas;
                    }
                }
            });
            ui.add_space(5.0);
        }

        // Package Entry ID / Hash
        ui.label(egui::RichText::new(format!("{:?}", key)).strong());
        ui.separator();
        egui::Grid::new("detail_grid").show(ui, |ui| {
            ui.label("Data Type:");
            ui.label(format!(
                "{:?} ({})",
                data_type_to_str(data_type),
                data_type
            ));
            ui.end_row();
            ui.label("Codec:");
            ui.label(format!("{:?}", codec));
            ui.end_row();
            ui.label("Raw Size:");
            ui.label(format_size(raw_size as u64));
            ui.end_row();
            ui.label("Stored Size:");
            ui.label(format_size(stored_size as u64));
            ui.end_row();
        });
    }

    /// Finds if a physical package entry is referenced by a virtual entry (e.g. CC Art -> Atlas Page)
    fn find_linked_virtual_entry(&self, entry_idx: usize) -> Option<usize> {
        match self.entries[entry_idx].key {
            FileKey::Id(id) => self.virtual_entries.iter().position(|v| v.id == id),
            FileKey::PathHash(h) => {
                for (&page_idx, _info) in &self.atlas_pages {
                    let has_match = atlas_page_paths(page_idx)
                        .iter()
                        .any(|path| xxh64_virtual_path(path) == h);

                    if has_match {
                        return self.virtual_entries.iter().position(|v| {
                            if let VirtualEntryData::AtlasRect { page_index, .. } = v.data {
                                page_index == page_idx
                            } else {
                                false
                            }
                        });
                    }
                }
                None
            }
        }
    }

    fn ui_details_virtual(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui, _idx: usize) {
        let Some(v_idx) = self.selected_virtual_idx else {
            return;
        };
        let ventry = &self.virtual_entries[v_idx];

        ui.horizontal(|ui| {
            if let VirtualEntryData::AtlasRect { .. } = ventry.data {
                ui.selectable_value(&mut self.preview_mode, PreviewModeKind::Entry, "Tile View");
                ui.selectable_value(&mut self.preview_mode, PreviewModeKind::Atlas, "Full Atlas");

                if ui
                    .button(egui::RichText::new("🖼 Open Atlas in Window").strong())
                    .clicked()
                {
                    self.image_window_mode = PreviewModeKind::Atlas;
                    self.image_window_open = true;
                }
            } else if self.preview_texture.is_some() {
                if ui
                    .button(egui::RichText::new("🖼 Open Image Window").strong())
                    .clicked()
                {
                    self.image_window_mode = PreviewModeKind::Entry;
                    self.image_window_open = true;
                }
            }
        });
        ui.add_space(5.0);

        // Virtual Asset ID and Kind
        ui.label(egui::RichText::new(format!("{} ID: {}", ventry.kind, ventry.id)).strong());
        ui.separator();
        egui::Grid::new("detail_grid_v").show(ui, |ui| {
            ui.label("Kind:");
            ui.label(&ventry.kind);
            ui.end_row();
            ui.label("Summary:");
            ui.label(&ventry.summary);
            ui.end_row();
            ui.label("Location:");
            ui.label(&ventry.location);
            ui.end_row();
            match &ventry.data {
                VirtualEntryData::AtlasRect {
                    page_index,
                    x,
                    y,
                    width,
                    height,
                    flags,
                } => {
                    ui.label("Page Index:");
                    ui.label(page_index.to_string());
                    ui.end_row();
                    ui.label("Rect:");
                    ui.label(format!("{},{} - {}x{}", x, y, width, height));
                    ui.end_row();
                    ui.label("Flags:");
                    ui.label(format!("0x{:04X}", flags));
                    ui.end_row();
                }
                VirtualEntryData::TileMetaLand(info) => {
                    ui.label("Texture:");
                    ui.label(info.texture_id.to_string());
                    ui.end_row();
                    ui.label("Tile Type:");
                    ui.label(info.tile_type.to_string());
                    ui.end_row();
                    ui.label("Name:");
                    ui.label(if info.name.is_empty() {
                        "<unnamed>"
                    } else {
                        &info.name
                    });
                    ui.end_row();
                }
                VirtualEntryData::TileMetaItem(info) => {
                    ui.label("EC Texture:");
                    ui.label(info.ec_texture_id.to_string());
                    ui.end_row();
                    ui.label("CC Texture:");
                    ui.label(info.cc_texture_id.to_string());
                    ui.end_row();
                    ui.label("Name:");
                    ui.label(if info.name.is_empty() {
                        "<unnamed>"
                    } else {
                        &info.name
                    });
                    ui.end_row();
                }
                VirtualEntryData::MapBlock { .. } | VirtualEntryData::StaticBlock { .. } => {
                    ui.label("Block ID:");
                    ui.label(ventry.id.to_string());
                    ui.end_row();
                }
                /*
                VirtualEntryData::DirectPayload { offset, size } => {
                    ui.label("Raw Offset:");
                    ui.label(format!("0x{:08X}", offset));
                    ui.end_row();
                    ui.label("Size:");
                    ui.label(format_size(*size as u64));
                    ui.end_row();
                }
                */
            }
        });
    }
}
