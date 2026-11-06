use eframe::egui;
use crate::app::{classic_tile_flags_summary, concrete_art_tile_source, ArtSource, ArtViewerRow, UocfEditorApp, ViewMode};
use crate::logic::ClientData;
use super::{
    arrow_delta, enhanced_files_availability, folder_files_availability, go_to_field, list_sort_controls,
    move_selection, sorted_indices_by, source_choice, ClientSource, CLASSIC_FOLDER, ENHANCED_FOLDER,
};
use super::classic_image_import::{ui_classic_image_panel, PreferredContainer};
use uocf::classic::image_files::ClassicImageKind;
use std::sync::Arc;
use uocf::classic::art::{static_art_id_for_source, ArtMap, STATIC_TILE_ID_BASE};
use uocf::classic::art_write::{ArtColors, ArtKind};
use uocf::classic::hues::HueEntry;
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem as RawTextureItem};
use uocf::classic::texmap_write::{TexmapMulFile, TEXMAP_DATA_FILE, TEXMAP_INDEX_FILE};
use uocf::image_export::{art_file_stem, id_file_stem};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use super::image_export::{
    classic_hued, export_menu, hue_rgba, save_image, save_status, start_bulk, BulkEntry, BulkExport, ExportClicked,
    ExportImage, ExportMenu, NOT_HUED,
};

const ART_VIEWER_LIST_PANEL_WIDTH: f32 = 300.0;
const ART_MUL_FILES: &str = "art.mul and artidx.mul";
const ART_CC_UOP_FILE: &str = "artLegacyMUL.uop";
const ART_KR_UOP_FILE: &str = "Texture.uop";
/// KR UOP while no Kingdom Reborn folder is set.
const ART_CARRIED_KR_UOP_FILE: &str = "Texture.uop (the Kingdom Reborn art the Enhanced Client carries)";
const ART_EC_UOP_NAME: &str = "LegacyTexture.uop";
const ART_EC_UOP_FILE: &str = "LegacyTexture.uop (the classic-style art the Enhanced Client draws by default)";
const DDS_MAGIC: &[u8] = b"DDS ";
const NO_HUE_SELECTED: &str = "Hue 0: the view draws the colours as stored.";
/// Exported texmaps are named by texture id: `texmap_0x0003`.
const TEXMAP_FILE_PREFIX: &str = "texmap";

pub fn ui_art_viewer(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    app.selected_legacy_source = concrete_art_tile_source(app.selected_legacy_source);
    let mut shown_ids = Vec::new();

    egui::Panel::left("tex_art_cc_list")
        .resizable(true)
        .default_size(ART_VIEWER_LIST_PANEL_WIDTH)
        .show(ui, |ui| {
            ui.heading("Art & TileData");

            ui.horizontal(|ui| {
                ui.label("Source:");
                let current = app.selected_legacy_source;
                if let Some(source) = art_source_choice(app, ui, current) {
                    app.selected_legacy_source = source;
                }
            });

            let mut art_search_has_focus = false;
            ui.horizontal(|ui| {
                ui.label("Search:");
                let response = ui.text_edit_singleline(&mut app.searches.art);
                art_search_has_focus |= response.has_focus();
            });
            let go_to = go_to_field(ui, "art_viewer_rows");
            art_search_has_focus |= go_to.has_focus;

            ui.separator();
            ui.heading("Hue Options");
            ui.horizontal(|ui| {
                ui.label("Selected Hue:");
                ui.add(egui::DragValue::new(&mut app.selected_hue_id));
            });

            if let Some(client) = &app.client_data {
                if let Some(hues) = &client.hues {
                    if app.selected_hue_id > 0 && (app.selected_hue_id as usize) <= hues.len() {
                        let hue = &hues[app.selected_hue_id as usize - 1];
                        let name = std::str::from_utf8(&hue.name).unwrap_or("").trim_matches('\0');
                        ui.label(format!("Name: {}", name));
                    }
                }
            }
            ui.separator();

            ensure_art_viewer_rows(app);
            if let Some(rows) = app.art_viewer_rows.clone() {
                let query = app.searches.art.trim();
                let sort = list_sort_controls(ui, "art_viewer_rows", &["ID", "Name", "Group"], 0);
                let sorted_indices = sorted_indices_by(&rows, sort.ordering(), |left, right| {
                    match sort.option_index {
                        1 => left.label.to_ascii_lowercase().cmp(&right.label.to_ascii_lowercase()),
                        2 => left.group.cmp(right.group).then_with(|| left.art_id.cmp(&right.art_id)),
                        _ => left.art_id.cmp(&right.art_id),
                    }
                });
                let filtered_indices = filtered_metadata_indices(&rows, Some(&sorted_indices), query, |row| &row.search_text);
                let visible_ids = filtered_indices
                    .iter()
                    .map(|index| rows[*index].art_id)
                    .collect::<Vec<_>>();
                let keyboard_moved = if let Some(delta) = arrow_delta(ui, art_search_has_focus) {
                    app.selected_tex_art_cc_id = move_selection(&visible_ids, app.selected_tex_art_cc_id, delta);
                    if app.selected_tex_art_cc_id.is_some() {
                        app.view_mode = ViewMode::TexArtCc;
                    }
                    true
                } else if let Some(position) = go_to.position_in(visible_ids.iter().map(|&art_id| u64::from(art_id))) {
                    app.selected_tex_art_cc_id = Some(visible_ids[position]);
                    app.view_mode = ViewMode::TexArtCc;
                    true
                } else {
                    false
                };

                shown_ids = visible_ids;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui_art_group_rows(app, ui, &rows, &filtered_indices, "Land Tiles", keyboard_moved);
                    ui_art_group_rows(app, ui, &rows, &filtered_indices, "Static Tiles", keyboard_moved);
                });
            }
        });

    egui::CentralPanel::default().show(ui, |ui| {
        if app.view_mode == ViewMode::CcTileData {
            ui_cc_tiledata(app, ctx, ui);
        } else if let Some(id) = app.selected_tex_art_cc_id {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut app.view_mode, ViewMode::TexArtCc, "Specialized");
                if ui.button("Raw UOP").clicked() {
                    app.select_raw_art_entry(id, concrete_art_tile_source(app.selected_legacy_source));
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading(format!("Art ID: {}", id));
                    if let Some(handle) = app.get_tex_art_cc_texture(ctx, id) {
                        ui.horizontal(|ui| {
                            ui.label(format!("Dimensions: {}x{}", handle.size()[0], handle.size()[1]));
                            ui_art_export(app, ui, id, &shown_ids);
                        });
                        ui.image(&handle);
                    } else {
                        ui.colored_label(egui::Color32::RED, "Art data not found");
                    }
                    // The Classic sources: art ids as the Classic files number them.
                    let preferred = match concrete_art_tile_source(app.selected_legacy_source) {
                        ArtSource::Mul => Some(PreferredContainer::Mul),
                        ArtSource::CcUop => Some(PreferredContainer::Uop),
                        _ => None,
                    };
                    if let Some(preferred) = preferred {
                        ui.separator();
                        ui_classic_image_panel(app, ctx, ui, ClassicImageKind::Art, id, preferred);
                    }
                });

                ui.separator();

                ui.vertical(|ui| {
                    ui.heading("TileData Properties");
                    let mut texmap = None;
                    if let Some(client) = &app.client_data {
                        let source = concrete_art_tile_source(app.selected_legacy_source);
                        if id < STATIC_TILE_ID_BASE && !source.is_ec_uop() {
                            if let Some(tile) = client.tiledata.land_tiles().get(id as usize) {
                                egui::Grid::new("land_tile_grid").striped(true).show(ui, |ui| {
                                    ui.label("Name"); ui.label(tile.name_ascii()); ui.end_row();
                                    ui.label("Flags"); ui.label(format!("{:016X}", tile.flags.internal_flags)); ui.end_row();
                                    ui.label("Texture ID"); ui.label(tile.texture_id.to_string()); ui.end_row();
                                });
                                ui.separator();
                                ui.label("Flags Breakdown:");
                                ui.label(classic_tile_flags_summary(tile.flags));
                                texmap = Some((client.path.clone(), tile.texture_id, shown_land_textures(client, &shown_ids)));
                            }
                        } else {
                            let item_id = if source.is_ec_uop() {
                                id
                            } else {
                                id - STATIC_TILE_ID_BASE
                            };
                            if let Some(tile) = client.tiledata.item_tiles().get(item_id as usize) {
                                egui::Grid::new("item_tile_grid").striped(true).show(ui, |ui| {
                                    ui.label("Name"); ui.label(tile.name_ascii()); ui.end_row();
                                    ui.label("Flags"); ui.label(format!("{:016X}", tile.flags.internal_flags)); ui.end_row();
                                    ui.label("Weight"); ui.label(tile.weight.to_string()); ui.end_row();
                                    ui.label("Layer/LightID"); ui.label(tile.quality.to_string()); ui.end_row();
                                    ui.label("Count/Quantity"); ui.label(tile.quantity.to_string()); ui.end_row();
                                    ui.label("Anim ID"); ui.label(tile.anim_id.to_string()); ui.end_row();
                                    ui.label("Hue Extra"); ui.label(tile.hue_extra.to_string()); ui.end_row();
                                    ui.label("Height"); ui.label(tile.height_raw().to_string()); ui.end_row();
                                });
                                ui.separator();
                                ui.label("Flags Breakdown:");
                                ui.label(classic_tile_flags_summary(tile.flags));
                            }
                        }
                    }
                    if let Some((folder, texture_id, shown_textures)) = texmap {
                        ui.separator();
                        ui_texmap(app, ctx, ui, &folder, texture_id, shown_textures);
                    }
                });
            });
        } else {
            ui.centered_and_justified(|ui| { ui.label("Select a tile from the sidebar to inspect"); });
        }
    });
}

/// The texmaps (texmaps.mul) of the Classic folder of the art, read when a land tile first shows its texture.
#[derive(Default)]
pub struct ArtTexmaps {
    /// The folder read, with its texmaps or why they could not be read.
    file: Option<(PathBuf, Result<Arc<TexmapMulFile>, String>)>,
    /// The texture id shown, with its image and preview (`None` when texmaps.mul has no such texture) or why it has
    /// none.
    shown: Option<(u16, Result<Option<(ExportImage, egui::TextureHandle)>, String>)>,
}

impl ArtTexmaps {
    fn file(&mut self, folder: &Path) -> Result<Arc<TexmapMulFile>, String> {
        if self.file.as_ref().is_none_or(|(read, _)| read != folder) {
            let file = TexmapMulFile::load(folder).map(Arc::new).map_err(|error| format!("{error:#}"));
            if let Err(error) = &file {
                log::warn!("texmaps of {}: {error}", folder.display());
            }
            self.file = Some((folder.to_path_buf(), file));
            self.shown = None;
        }
        self.file.as_ref().expect("read above").1.clone()
    }

    fn shown(
        &mut self,
        ctx: &egui::Context,
        texmaps: &TexmapMulFile,
        texture_id: u16,
    ) -> &Result<Option<(ExportImage, egui::TextureHandle)>, String> {
        if self.shown.as_ref().is_none_or(|(shown, _)| *shown != texture_id) {
            let decoded = texmaps.texture(usize::from(texture_id)).map_err(|error| format!("{error:#}")).and_then(|record| {
                record
                    .map(|record| {
                        let image = ExportImage::texmap(&record);
                        let size = [image.width() as usize, image.height() as usize];
                        let name = format!("texmap_preview_{texture_id}");
                        super::textures::load_rgba(ctx, name, size, &image.to_rgba(), Default::default()).map(|handle| (image, handle))
                    })
                    .transpose()
            });
            self.shown = Some((texture_id, decoded));
        }
        &self.shown.as_ref().expect("decoded above").1
    }
}

/// The texture ids of the land tiles among the `shown` art ids, each once.
fn shown_land_textures(client: &ClientData, shown: &[u32]) -> Vec<u16> {
    let land = client.tiledata.land_tiles();
    let textures: BTreeSet<u16> = shown
        .iter()
        .filter(|&&id| id < STATIC_TILE_ID_BASE)
        .filter_map(|&id| land.get(id as usize).map(|tile| tile.texture_id))
        .collect();
    textures.into_iter().collect()
}

/// The texmap of a land tile (its texture id in texmaps.mul, from `folder`): the image with its export menu (Classic
/// 16-bit colours, no transparency), and the bulk export of the `shown` texture ids.
fn ui_texmap(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui, folder: &Path, texture_id: u16, shown: Vec<u16>) {
    ui.heading("Texmap");
    let texmaps = match app.art_texmaps.file(folder) {
        Ok(texmaps) => texmaps,
        Err(error) => {
            ui.colored_label(ui.visuals().warn_fg_color, format!("No {TEXMAP_DATA_FILE} and {TEXMAP_INDEX_FILE}: {error}"));
            return;
        }
    };
    let (image, handle) = match app.art_texmaps.shown(ctx, &texmaps, texture_id) {
        Ok(Some((image, handle))) => (image, handle),
        Ok(None) => {
            ui.weak(format!("{TEXMAP_DATA_FILE} has no texture {texture_id}."));
            return;
        }
        Err(error) => {
            ui.colored_label(ui.visuals().warn_fg_color, format!("Texture {texture_id}: {error}"));
            return;
        }
    };
    let stem = id_file_stem(TEXMAP_FILE_PREFIX, u32::from(texture_id));
    let menu = ExportMenu {
        hue: Err(NOT_HUED),
        frame: false,
        bulk: vec![BulkEntry {
            label: format!("Export the texmaps of the shown land tiles ({})...", shown.len()),
            hover: format!(
                "The texmap of every land tile the list shows, each texture once as {stem}.<format> (by texture id), \
                 into a folder you pick (files of the same name are replaced)."
            ),
        }],
    };
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.label(format!("Texture {texture_id}: {}x{}", image.width(), image.height()));
        clicked = export_menu(ui, &mut app.image_export, &menu);
    });
    ui.image(handle);
    let format = app.image_export.format;
    match clicked {
        None => {}
        Some(ExportClicked::Shown) => {
            if let Some(status) = save_status(&format!("texmap {texture_id}"), save_image(&stem, image, format)) {
                app.status_message = status;
            }
        }
        Some(ExportClicked::Bulk(_)) => {
            start_bulk(app, |folder| {
                BulkExport::start(format!("Export {} texmaps", shown.len()), folder, format, shown, move |texture_id: u16| {
                    let record = texmaps.texture(usize::from(texture_id))?;
                    let stem = id_file_stem(TEXMAP_FILE_PREFIX, u32::from(texture_id));
                    Ok(record.map(|record| vec![(stem, ExportImage::texmap(&record))]).unwrap_or_default())
                })
            });
        }
    }
}

/// The export menu of art `id`, and the bulk export of the `shown` ids.
fn ui_art_export(app: &mut UocfEditorApp, ui: &mut egui::Ui, id: u32, shown: &[u32]) {
    let Some(client) = &app.client_data else {
        return;
    };
    let (art, hues) = (Arc::clone(&client.art), client.hues.clone());
    let source = concrete_art_tile_source(app.selected_legacy_source);
    let menu = ExportMenu {
        hue: if app.selected_hue_id > 0 { Ok(()) } else { Err(NO_HUE_SELECTED) },
        frame: false,
        bulk: vec![BulkEntry {
            label: format!("Export all shown ({})...", shown.len()),
            hover: format!(
                "Every art id the list shows, each as {}.<format> (land tiles as land_0x<land id>, items as \
                 item_0x<item id>), into a folder you pick (files of the same name are replaced).",
                art_file_stem(id)
            ),
        }],
    };
    let Some(clicked) = export_menu(ui, &mut app.image_export, &menu) else {
        return;
    };
    let settings = app.image_export;
    let hue = settings
        .takes_hue(&menu)
        .then(|| hues.and_then(|hues| hues.get(usize::from(app.selected_hue_id).checked_sub(1)?).copied()))
        .flatten();
    match clicked {
        ExportClicked::Shown => {
            let result = art_export_image(&art, id, source, hue.as_ref())
                .map_err(|error| format!("{error:#}"))
                .and_then(|image| image.ok_or_else(|| "the source has no such art".to_string()))
                .and_then(|image| save_image(&art_file_stem(id), &image, settings.format));
            if let Some(status) = save_status(&format!("art {id:#06x}"), result) {
                app.status_message = status;
            }
        }
        ExportClicked::Bulk(_) => {
            let ids = shown.to_vec();
            start_bulk(app, |folder| {
                BulkExport::start(format!("Export {} art", ids.len()), folder, settings.format, ids, move |id| {
                    let image = art_export_image(&art, id, source, hue.as_ref())?;
                    Ok(image.map(|image| vec![(art_file_stem(id), image)]).unwrap_or_default())
                })
            });
        }
    }
}

/// Art `art_id` of `source` to export: Classic art as stored (16-bit), Enhanced art decoded to RGBA; `hue` where the
/// view hues it (statics, and Enhanced art). `None` when the source has no such art.
fn art_export_image(art: &ArtMap, art_id: u32, source: ArtSource, hue: Option<&HueEntry>) -> color_eyre::eyre::Result<Option<ExportImage>> {
    if !art.has_id_from_source(art_id, source) {
        return Ok(None);
    }
    let mut raw = Vec::new();
    art.get_raw_art_data_from_source(art_id, source, &mut raw)?;
    let hue = hue.filter(|_| crate::app::cc_hue_should_apply_to_art(art_id, source));
    if source.is_ec_uop() || raw.starts_with(DDS_MAGIC) {
        let format = if raw.starts_with(DDS_MAGIC) { ECImageFormat::DDS } else { ECImageFormat::TGA };
        let texture = TextureFile {
            metadata: RawTextureItem::absent(),
            is_ec: source.is_ec_uop(),
            format,
            props: None,
            raw_data: Arc::from(raw),
            image_data_offset: 0,
        };
        let rgba = texture.decode_to_rgba()?.to_rgba8();
        let (width, height) = rgba.dimensions();
        let mut rgba = rgba.into_raw();
        if let Some(hue) = hue {
            hue_rgba(hue, &mut rgba);
        }
        return Ok(Some(ExportImage::Rgba { width, height, rgba }));
    }
    let mut image = ExportImage::art(&ArtColors::decode(ArtKind::of_id(art_id), &raw)?);
    if let Some(hue) = hue {
        image.hue_classic_colors(|color| classic_hued(hue, color));
    }
    Ok(Some(image))
}

fn art_row_should_show(source: ArtSource, has_art: bool, has_metadata: bool) -> bool {
    match source {
        ArtSource::CcUop | ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => has_art,
        ArtSource::Mul | ArtSource::Any => has_art || has_metadata,
    }
}

fn ensure_art_viewer_rows(app: &mut UocfEditorApp) {
    let source = concrete_art_tile_source(app.selected_legacy_source);
    if app.art_viewer_rows_source == Some(source) && app.art_viewer_rows.is_some() {
        return;
    }

    let rows = app
        .client_data
        .as_ref()
        .map(|client| Arc::new(collect_art_viewer_rows(client, source)));
    app.art_viewer_rows = rows;
    app.art_viewer_rows_source = Some(source);
}

fn collect_art_viewer_rows(client: &ClientData, source: ArtSource) -> Vec<ArtViewerRow> {
    let mut rows = Vec::new();

    if !source.is_ec_uop() {
        for id in 0..STATIC_TILE_ID_BASE {
            let has_art = client.art.has_id_from_source(id, source);
            let has_metadata = client.tiledata.land_tiles().get(id as usize).is_some();
            if !art_row_should_show(source, has_art, has_metadata) {
                continue;
            }

            let tile_name = client
                .tiledata
                .land_tiles()
                .get(id as usize)
                .map(|tile| tile.name_ascii())
                .unwrap_or_default();
            rows.push(art_viewer_row(id, "Land Tiles", tile_name));
        }
    }

    let source_max_id = client.art.max_id_for_source(source);
    let max_item_id = if source.is_ec_uop() {
        source_max_id.max(32768)
    } else {
        source_max_id
            .max(STATIC_TILE_ID_BASE + 32768)
            .saturating_sub(STATIC_TILE_ID_BASE)
    };
    for item_id in 0..max_item_id {
        let id = static_art_id_for_source(item_id, source);
        let has_art = client.art.has_id_from_source(id, source);
        let has_metadata = client.tiledata.item_tiles().get(item_id as usize).is_some();
        if !art_row_should_show(source, has_art, has_metadata) {
            continue;
        }

        let tile_name = client
            .tiledata
            .item_tiles()
            .get(item_id as usize)
            .map(|tile| tile.name_ascii())
            .unwrap_or_default();
        rows.push(art_viewer_row(id, "Static Tiles", tile_name));
    }

    rows
}

fn art_viewer_row(art_id: u32, group: &'static str, tile_name: &str) -> ArtViewerRow {
    let label = format!("{}: {}", art_id, tile_name);
    ArtViewerRow {
        art_id,
        label,
        group,
        search_text: format!("{} {} {}", art_id, group.to_lowercase(), tile_name.to_lowercase()),
    }
}

fn ui_art_group_rows(
    app: &mut UocfEditorApp,
    ui: &mut egui::Ui,
    rows: &[ArtViewerRow],
    filtered_indices: &[usize],
    group: &'static str,
    keyboard_moved: bool,
) {
    let has_rows = filtered_indices.iter().any(|index| rows[*index].group == group);
    if !has_rows {
        return;
    }

    ui.collapsing(group, |ui| {
        for index in filtered_indices.iter().copied().filter(|index| rows[*index].group == group) {
            ui_art_row(app, ui, &rows[index], keyboard_moved);
        }
    });
}

fn ui_art_row(
    app: &mut UocfEditorApp,
    ui: &mut egui::Ui,
    row: &ArtViewerRow,
    keyboard_moved: bool,
) {
    let selected = app.selected_tex_art_cc_id == Some(row.art_id);
    let response = ui.selectable_label(selected, &row.label);
    if response.clicked() {
        app.selected_tex_art_cc_id = Some(row.art_id);
        app.view_mode = ViewMode::TexArtCc;
    }
    if keyboard_moved && selected {
        response.scroll_to_me(Some(egui::Align::Center));
    }
}

/// The selector source of an art source. The art of `Texture.uop` is the Kingdom Reborn art the Enhanced Client
/// carries: KR UOP. The classic-style art of `LegacyTexture.uop`, which the Enhanced Client draws by default: EC UOP.
pub(super) fn art_client_source(source: ArtSource) -> ClientSource {
    match source {
        ArtSource::Mul => ClientSource::Mul,
        ArtSource::CcUop | ArtSource::Any => ClientSource::CcUop,
        ArtSource::EcUopKr => ClientSource::KrUop,
        ArtSource::EcUop | ArtSource::EcUopLegacy => ClientSource::EcUop,
    }
}

fn art_source_of(source: ClientSource) -> ArtSource {
    match source {
        ClientSource::Mul => ArtSource::Mul,
        ClientSource::CcUop => ArtSource::CcUop,
        ClientSource::KrUop => ArtSource::EcUopKr,
        ClientSource::EcUop => ArtSource::EcUopLegacy,
    }
}

/// The art source buttons, `current` pressed; returns the art source clicked, after showing its client's files when it
/// is the Enhanced-format client not shown ([`UocfEditorApp::show_enhanced_client`]).
pub(super) fn art_source_choice(app: &mut UocfEditorApp, ui: &mut egui::Ui, current: ArtSource) -> Option<ArtSource> {
    let art = app.client_data.as_ref().map(|client| Arc::clone(&client.art));
    let has = |source: ClientSource| art.as_ref().is_some_and(|art| art.max_id_for_source(art_source_of(source)) > 0);
    let cc_path = app.settings.cc_path.as_deref();
    // While no Kingdom Reborn folder is set, KR UOP is the Kingdom Reborn art the Enhanced Client carries.
    let carried_kr_art = app.enhanced_folders.kr.is_none() && app.shown_enhanced == Some(ClientSource::EcUop);
    let clicked = source_choice(ui, Some(art_client_source(current)), |source| match source {
        ClientSource::Mul => folder_files_availability(cc_path, CLASSIC_FOLDER, ART_MUL_FILES, has(source)),
        ClientSource::CcUop => folder_files_availability(cc_path, CLASSIC_FOLDER, ART_CC_UOP_FILE, has(source)),
        ClientSource::KrUop if carried_kr_art => {
            folder_files_availability(app.shown_enhanced_folder(), ENHANCED_FOLDER, ART_CARRIED_KR_UOP_FILE, has(source))
        }
        ClientSource::KrUop => enhanced_files_availability(app, source, ART_KR_UOP_FILE, &[ART_KR_UOP_FILE], has(source)),
        ClientSource::EcUop => enhanced_files_availability(app, source, ART_EC_UOP_FILE, &[ART_EC_UOP_NAME], has(source)),
    });
    if let Some(source) = clicked {
        app.show_enhanced_client(source);
    }
    clicked.map(art_source_of)
}

fn ui_cc_tiledata(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    super::tile_metadata::contents(app, ctx, ui);
}

fn filtered_metadata_indices<T>(
    rows: &[T],
    sorted_indices: Option<&[usize]>,
    query: &str,
    search_text: impl Fn(&T) -> &str,
) -> Vec<usize> {
    let source_indices: Vec<usize> = sorted_indices
        .map(|indices| indices.to_vec())
        .unwrap_or_else(|| (0..rows.len()).collect());
    if query.is_empty() {
        return source_indices;
    }

    let query = query.to_lowercase();
    source_indices
        .into_iter()
        .filter(|index| search_text(&rows[*index]).contains(&query))
        .collect()
}
