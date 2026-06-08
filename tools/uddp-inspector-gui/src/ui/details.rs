use eframe::egui;
use udd_assets::hues::HUES_TEXTURE_ENTRY_PATH;
use udd_container::{FileKey, xxh64_virtual_path};
use crate::app::{InspectorApp, MAX_INLINE_PREVIEW_WIDTH, MAX_INLINE_PREVIEW_HEIGHT};
use crate::logic::discovery::upscale_algorithm_name;
use crate::models::{PreviewModeKind, ViewMode, VirtualEntryData};
use crate::ui::{metadata_grid, metadata_label};
use crate::utils::{
    atlas_page_paths,
    atlas_pixel_format_to_str,
    compression_to_str,
    data_type_to_str,
    format_size,
};

const DETAILS_SECTION_SPACING_SMALL: f32 = 5.0;
const DETAILS_SECTION_SPACING_LARGE: f32 = 10.0;
const DETAILS_TEXT_PREVIEW_BOTTOM_PADDING: f32 = 20.0;

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
        ui.add_space(DETAILS_SECTION_SPACING_SMALL);

        // Render mode-specific metadata
        match self.view_mode {
            ViewMode::Package => self.ui_details_package(ctx, ui, idx),
            ViewMode::Virtual => self.ui_details_virtual(ctx, ui, idx),
            ViewMode::MobileAnimCc | ViewMode::MobileAnimEc => {}
        }

        ui.add_space(DETAILS_SECTION_SPACING_LARGE);
        ui.separator();
        ui.add_space(DETAILS_SECTION_SPACING_LARGE);
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
                    ui.add(
                        egui::Image::new(texture)
                            .texture_options(egui::TextureOptions::NEAREST),
                    );
                });
            }
        } else if let Some(text) = &self.preview_text {
            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - DETAILS_TEXT_PREVIEW_BOTTOM_PADDING)
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
        let (data_type, key, codec, raw_size, stored_size, format_label) = {
            let entry = &self.entries[idx];
            (
                entry.data_type,
                entry.key,
                entry.codec,
                entry.raw_size,
                entry.stored_size,
                self.package_entry_format_label(entry.key),
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
            ui.add_space(DETAILS_SECTION_SPACING_SMALL);
        }

        // Package Entry ID / Hash
        ui.label(egui::RichText::new(format!("{:?}", key)).strong());
        ui.separator();
        metadata_grid("detail_grid").show(ui, |ui| {
            metadata_label(ui, "Data Type:");
            ui.label(format!(
                "{:?} ({})",
                data_type_to_str(data_type),
                data_type
            ));
            ui.end_row();
            metadata_label(ui, "Compression:");
            ui.label(compression_to_str(codec));
            ui.end_row();
            if let Some(format_label) = format_label {
                metadata_label(ui, "Format:");
                ui.label(format_label);
                ui.end_row();
            }
            metadata_label(ui, "Raw Size:");
            ui.label(format_size(raw_size as u64));
            ui.end_row();
            metadata_label(ui, "Stored Size:");
            ui.label(format_size(stored_size as u64));
            ui.end_row();
        });
    }

    fn package_entry_format_label(&self, key: FileKey) -> Option<&'static str> {
        let FileKey::PathHash(hash) = key else {
            return None;
        };

        if hash == xxh64_virtual_path(HUES_TEXTURE_ENTRY_PATH) {
            return Some("rgba8888");
        }

        self.atlas_pages
            .iter()
            .find_map(|(&page_idx, page_info)| {
                atlas_page_paths(page_idx)
                    .iter()
                    .any(|path| xxh64_virtual_path(path) == hash)
                    .then_some(atlas_pixel_format_to_str(page_info.pixel_format))
            })
    }

    /// Finds if a physical package entry is referenced by a virtual entry (e.g. CC Art -> Atlas Page)
    fn find_linked_virtual_entry(&self, entry_idx: usize) -> Option<usize> {
        match self.entries[entry_idx].key {
            FileKey::Id(id) => self.virtual_entries.iter().position(|v| v.id == id),
            FileKey::PathHash(h) => {
                for &page_idx in self.atlas_pages.keys() {
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
        let ventry = self.active_virtual_entries()[v_idx].clone();

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
        ui.add_space(DETAILS_SECTION_SPACING_SMALL);

        // Virtual Asset ID and Kind
        ui.label(egui::RichText::new(format!("{} ID: {}", ventry.kind, ventry.id)).strong());
        ui.separator();
        metadata_grid("detail_grid_v").show(ui, |ui| {
            metadata_label(ui, "Kind:");
            ui.label(&ventry.kind);
            ui.end_row();
            metadata_label(ui, "Summary:");
            ui.label(&ventry.summary);
            ui.end_row();
            metadata_label(ui, "Location:");
            ui.label(&ventry.location);
            ui.end_row();
            match &ventry.data {
                VirtualEntryData::AtlasRect {
                    page_index,
                    page_tile_idx,
                    x,
                    y,
                    width,
                    height,
                    flags,
                    upscale_factor,
                    upscale_algorithm,
                } => {
                    metadata_label(ui, "Page Index:");
                    ui.label(page_index.to_string());
                    ui.end_row();
                    metadata_label(ui, "Page Tile:");
                    ui.label(page_tile_idx.to_string());
                    ui.end_row();
                    metadata_label(ui, "Rect:");
                    ui.label(format!("{},{} - {}x{}", x, y, width, height));
                    ui.end_row();
                    metadata_label(ui, "Logical Size:");
                    ui.label(format!(
                        "{:.1}x{:.1}",
                        *width as f32 / f32::from((*upscale_factor).max(1)),
                        *height as f32 / f32::from((*upscale_factor).max(1))
                    ));
                    ui.end_row();
                    metadata_label(ui, "Upscale:");
                    ui.label(format!(
                        "{} {}x (code {})",
                        upscale_algorithm_name(*upscale_algorithm),
                        upscale_factor,
                        upscale_algorithm
                    ));
                    ui.end_row();
                    metadata_label(ui, "Flags:");
                    ui.label(format!("0x{:04X}", flags));
                    ui.end_row();
                }
                VirtualEntryData::EcLandMaterial(info) => {
                    metadata_label(ui, "Material ID:");
                    ui.label(info.material_id.to_string());
                    ui.end_row();
                    metadata_label(ui, "Material Name ID:");
                    ui.label(info.material_name_id.to_string());
                    ui.end_row();
                    metadata_label(ui, "TerrainDefinition Path:");
                    ui.label(&info.terrain_definition_path);
                    ui.end_row();
                    metadata_label(ui, "TerrainDefinition Hash:");
                    ui.label(format!("0x{:016X}", info.terrain_definition_hash64));
                    ui.end_row();
                    metadata_label(ui, "Primary Texture:");
                    ui.label(
                        info.primary_texture_id
                            .map(|texture_id| texture_id.to_string())
                            .unwrap_or_else(|| "<missing>".to_string()),
                    );
                    ui.end_row();
                    metadata_label(ui, "Aliases:");
                    ui.label(info.alias_count.to_string());
                    ui.end_row();
                    metadata_label(ui, "Selected Textures:");
                    ui.label(info.selected_texture_count.to_string());
                    ui.end_row();
                    if !info.selected_textures.is_empty() {
                        metadata_label(ui, "Material Layers:");
                        ui.label(format_material_layers(&info.selected_textures));
                        ui.end_row();
                    }
                    if !info.override_textures.is_empty() {
                        metadata_label(ui, "Linked Textures:");
                        ui.label(format_override_textures(&info.override_textures));
                        ui.end_row();
                    }
                    if let Some(preview) = info.preview.as_ref() {
                        metadata_label(ui, "Preview Slot:");
                        ui.label(preview.slot_id.to_string());
                        ui.end_row();
                        metadata_label(ui, "Preview Rect:");
                        ui.label(format!(
                            "page {} {},{} - {}x{}",
                            preview.page_index,
                            preview.x,
                            preview.y,
                            preview.width,
                            preview.height
                        ));
                        ui.end_row();
                    }
                }
                VirtualEntryData::TileMetaLand(info) => {
                    metadata_label(ui, "Texture:");
                    ui.label(info.texture_id.to_string());
                    ui.end_row();
                    metadata_label(ui, "Tile Type:");
                    ui.label(info.tile_type.to_string());
                    ui.end_row();
                    metadata_label(ui, "Name:");
                    ui.label(if info.name.is_empty() {
                        "<unnamed>"
                    } else {
                        &info.name
                    });
                    ui.end_row();
                }
                VirtualEntryData::TileMetaItem(info) => {
                    metadata_label(ui, "EC Texture:");
                    ui.label(info.ec_texture_id.to_string());
                    ui.end_row();
                    metadata_label(ui, "CC Texture:");
                    ui.label(info.cc_texture_id.to_string());
                    ui.end_row();
                    metadata_label(ui, "Name:");
                    ui.label(if info.name.is_empty() {
                        "<unnamed>"
                    } else {
                        &info.name
                    });
                    ui.end_row();
                }
                VirtualEntryData::WorldLight(info) => {
                    metadata_label(ui, "Light ID:");
                    ui.label(ventry.id.to_string());
                    ui.end_row();
                    metadata_label(ui, "Size:");
                    ui.label(format!("{}x{}", info.width, info.height));
                    ui.end_row();
                    metadata_label(ui, "Format:");
                    ui.label("rgba8888");
                    ui.end_row();
                    metadata_label(ui, "Flags:");
                    ui.label(format!("0x{:04X}", info.flags));
                    ui.end_row();
                    metadata_label(ui, "Path:");
                    ui.label(&info.path);
                    ui.end_row();
                }
                VirtualEntryData::MapBlock { .. } | VirtualEntryData::StaticBlock { .. } => {
                    metadata_label(ui, "Block ID:");
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

fn format_material_layers(textures: &[crate::models::EcLandMaterialTextureInfo]) -> String {
    textures
        .iter()
        .map(|texture| {
            let slot = texture
                .runtime_slot_id
                .map(|slot_id| slot_id.to_string())
                .unwrap_or_else(|| "missing".to_string());
            let primary = if texture.is_primary { " primary" } else { "" };
            format!(
                "{}: texture {} -> slot {} rep {:.3}{}",
                material_layer_name(texture.layer_index),
                texture.texture_id,
                slot,
                texture.repetition,
                primary
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_override_textures(textures: &[crate::models::EcLandMaterialOverrideTextureInfo]) -> String {
    textures
        .iter()
        .map(|texture| {
            let slot = texture
                .runtime_slot_id
                .map(|slot_id| slot_id.to_string())
                .unwrap_or_else(|| "missing".to_string());
            let layer = texture
                .layer_index
                .map(|layer_index| format!(" {}", material_layer_name(layer_index)))
                .unwrap_or_default();
            let repetition = texture
                .repetition
                .map(|value| format!(" rep {value:.3}"))
                .unwrap_or_default();
            format!(
                "{}{}: texture {} -> slot {}{}",
                texture.role,
                layer,
                texture.texture_id,
                slot,
                repetition
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn material_layer_name(layer_index: u32) -> String {
    match layer_index {
        0 => "layer 0 base/diffuse".to_string(),
        1 => "layer 1 detail".to_string(),
        2 => "layer 2 mask/alpha".to_string(),
        3 => "layer 3 normal".to_string(),
        u32::MAX => "unassigned layer".to_string(),
        _ => format!("layer {layer_index}"),
    }
}
