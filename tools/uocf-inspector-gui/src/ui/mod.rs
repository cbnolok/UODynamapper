use eframe::egui;
use crate::app::UopInspectorApp;
use std::cmp::Ordering;

pub mod animations;
pub mod animdata;
pub mod art_viewer;
pub mod uop_browser;
pub mod multis;
pub mod multimap;
pub mod hues;
pub mod image_export;
pub mod gumps;
pub mod clilocs;
pub mod terrain_definition;
pub mod string_dictionary;
pub mod sounds;
pub mod upscale_preview;

pub(super) fn arrow_delta(ui: &egui::Ui, disabled: bool) -> Option<isize> {
    if disabled {
        return None;
    }

    if ui.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
        Some(1)
    } else if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
        Some(-1)
    } else {
        None
    }
}

pub(super) fn move_selection<T: Copy + Eq>(
    visible: &[T],
    selected: Option<T>,
    delta: isize,
) -> Option<T> {
    if visible.is_empty() {
        return selected;
    }

    let current = selected
        .and_then(|selected| visible.iter().position(|value| *value == selected));
    let next = match (current, delta) {
        (Some(index), delta) if delta < 0 => index.saturating_sub(1),
        (Some(index), delta) if delta > 0 => (index + 1).min(visible.len() - 1),
        (Some(index), _) => index,
        (None, delta) if delta < 0 => visible.len() - 1,
        (None, _) => 0,
    };

    Some(visible[next])
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ListSortState {
    pub option_index: usize,
    pub ordering: ListSortOrdering,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ListSortOrdering {
    None,
    Ascending,
    Descending,
}

impl ListSortState {
    pub fn ordering(self) -> Option<bool> {
        match self.ordering {
            ListSortOrdering::None => None,
            ListSortOrdering::Ascending => Some(false),
            ListSortOrdering::Descending => Some(true),
        }
    }
}

pub(super) fn list_sort_controls(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    options: &[&'static str],
    default_option_index: usize,
) -> ListSortState {
    let id = ui.make_persistent_id(("uocf_list_sort", id_salt));
    let mut state = ui
        .data_mut(|data| data.get_temp::<ListSortState>(id))
        .unwrap_or(ListSortState {
            option_index: default_option_index.min(options.len().saturating_sub(1)),
            ordering: ListSortOrdering::None,
        });
    if state.option_index >= options.len() {
        state.option_index = default_option_index.min(options.len().saturating_sub(1));
    }

    ui.horizontal(|ui| {
        ui.label("Sort:");
        egui::ComboBox::from_id_salt((id, "option"))
            .selected_text(options.get(state.option_index).copied().unwrap_or("Source"))
            .show_ui(ui, |ui| {
                for (index, option) in options.iter().enumerate() {
                    ui.selectable_value(&mut state.option_index, index, *option);
                }
            });
        egui::ComboBox::from_id_salt((id, "ordering"))
            .selected_text(match state.ordering {
                ListSortOrdering::None => "No ordering",
                ListSortOrdering::Ascending => "Ascending",
                ListSortOrdering::Descending => "Descending",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.ordering, ListSortOrdering::None, "No ordering");
                ui.selectable_value(&mut state.ordering, ListSortOrdering::Ascending, "Ascending");
                ui.selectable_value(&mut state.ordering, ListSortOrdering::Descending, "Descending");
            });
    });

    ui.data_mut(|data| data.insert_temp(id, state));
    state
}

pub(super) fn sorted_indices_by<T>(
    items: &[T],
    ordering: Option<bool>,
    compare: impl Fn(&T, &T) -> Ordering,
) -> Vec<usize> {
    let Some(descending) = ordering else {
        return (0..items.len()).collect();
    };

    let mut indices: Vec<usize> = (0..items.len()).collect();
    indices.sort_by(|left, right| {
        let ordering = compare(&items[*left], &items[*right]);
        let ordering = if descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| left.cmp(right))
    });
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_indices_by_preserves_source_order_for_equal_keys() {
        let values = ["b", "a", "b"];
        let indices = sorted_indices_by(&values, Some(false), |left, right| left.cmp(right));
        assert_eq!(indices, vec![1, 0, 2]);
    }

    #[test]
    fn sorted_indices_by_reverses_requested_order() {
        let values = [2, 1, 3];
        let indices = sorted_indices_by(&values, Some(true), |left, right| left.cmp(right));
        assert_eq!(indices, vec![2, 0, 1]);
    }

    #[test]
    fn sorted_indices_by_can_leave_source_order_unchanged() {
        let values = [2, 1, 3];
        let indices = sorted_indices_by(&values, None, |left, right| left.cmp(right));
        assert_eq!(indices, vec![0, 1, 2]);
    }
}

pub fn draw_ui(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::Panel::top("top_panel").show_inside(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open UOP...").clicked() {
                    app.open_uop();
                    ui.close_kind(egui::UiKind::Menu);
                }
                if ui.button("Search Paths...").clicked() {
                    app.show_search_paths = true;
                    ui.close_kind(egui::UiKind::Menu);
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            
            ui.separator();
            if ui.button("Upscale Preview").clicked() {
                app.show_upscale_preview = true;
            }

            ui.separator();
            
            ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Home, "Home");
            if uop_browser::has_visible_uop_explorer_package(app) {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::UopExplorer, "UOP Explorer");
            }
            if app.client_data.is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::TexArtCc, "Art");
            }
            if app.client_data.as_ref().and_then(|client| client.multis.as_ref()).is_some()
                || app.multi_collection.is_some()
                || app.cc_multimap.is_some()
            {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Multis, "Multis");
            }
            ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Multimap, "Multimap");
            if app.client_data.is_some() || app.ec_hues.is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Hues, "Hues");
            }
            if app.cliloc.is_some() || app.localized_strings.is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Clilocs, "CliLocs");
            }
            if app.cc_tiledata.is_some() || app.ec_tileart_entries.is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::TileMetadata, "Tile Metadata");
            }
            if app.cc_gumps_package.is_some()
                || app.ec_gumps_package.is_some()
                || app.cc_gumps.is_some()
                || app.uop_cache.loaded_uops.iter().any(|loaded| {
                    loaded
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.eq_ignore_ascii_case("interface.uop"))
                        .unwrap_or(false)
                })
            {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Gumps, "Gumps");
            }
            ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Animations, "Animations");
            if app.client_data.as_ref().and_then(|client| client.animdata.as_ref()).is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::AnimData, "AnimData");
            }
            if app.terrain_def_package.is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::TerrainDefinition, "Terrain Def");
            }
            if app.uo_string_dictionary.is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::StringDictionary, "String Dict");
            }
            if app.cc_sounds.is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Sounds, "Sounds");
            }
        });
    });

    egui::Panel::bottom("status_bar").show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(&app.status_message);
            ui.separator();
            match &app.settings.dict_path {
                Some(path) => {
                    ui.label("DIC:");
                    ui.label(path.display().to_string());
                }
                None => {
                    ui.weak("No .dic loaded");
                }
            }
        });
    });

    let mut show_search_paths = app.show_search_paths;
    if show_search_paths {
        egui::Window::new("Search Paths")
            .open(&mut show_search_paths)
            .show(ctx, |ui| {
                egui::Grid::new("paths_grid").show(ui, |ui| {
                    ui.label("CC Path:");
                    ui.horizontal(|ui| {
                        if let Some(path) = &app.settings.cc_path {
                            ui.label(path.display().to_string());
                        } else {
                            ui.label("None");
                        }
                        if ui.button("Select...").clicked() {
                            if let Some(path) = crate::dialog::pick_folder() {
                                app.settings.cc_path = Some(path);
                                app.trigger_reload();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("EC Path:");
                    ui.horizontal(|ui| {
                        if let Some(path) = &app.settings.ec_path {
                            ui.label(path.display().to_string());
                        } else {
                            ui.label("None");
                        }
                        if ui.button("Select...").clicked() {
                            if let Some(path) = crate::dialog::pick_folder() {
                                app.settings.ec_path = Some(path);
                                app.trigger_reload();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("Dictionary (.dic):");
                    ui.horizontal(|ui| {
                        if let Some(path) = &app.settings.dict_path {
                            ui.label(path.display().to_string());
                        } else {
                            ui.label("None");
                        }
                        if ui.button("Select...").clicked() {
                            if let Some(path) = crate::dialog::file_dialog()
                                .add_filter("DIC Dictionary", &["dic"])
                                .pick_file()
                            {
                                app.settings.dict_path = Some(path);
                                app.trigger_reload();
                            }
                        }
                    });
                    ui.end_row();
                });
            });
        app.show_search_paths = show_search_paths;
    }

    if app.view_mode == crate::app::ViewMode::UopExplorer
        && app.selected_uop_idx.is_none()
        && !uop_browser::has_visible_uop_explorer_package(app)
    {
        app.view_mode = crate::app::ViewMode::Home;
    }

    match app.view_mode {
        crate::app::ViewMode::Home => {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.heading("Logs");
                ui.separator();
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for log in &app.logs {
                            ui.label(log);
                        }
                    });
            });
        }
        crate::app::ViewMode::UopExplorer => {
            uop_browser::ui_uop_browser(app, ctx, ui);
        }
        crate::app::ViewMode::TexArtCc | crate::app::ViewMode::CcTileData => {
            art_viewer::ui_art_viewer(app, ctx, ui);
        }
        crate::app::ViewMode::TileMetadata => {
            art_viewer::ui_tile_metadata(app, ctx, ui);
        }
        crate::app::ViewMode::Animations => {
            animations::ui_animations(app, ctx, ui);
        }
        crate::app::ViewMode::Gumps => {
            gumps::ui_gumps(app, ctx, ui);
        }
        crate::app::ViewMode::AnimData => {
            animdata::ui_animdata(app, ctx, ui);
        }
        crate::app::ViewMode::Multis => {
            multis::ui_multis(app, ctx, ui);
        }
        crate::app::ViewMode::Multimap => {
            multimap::ui_multimap(app, ctx, ui);
        }
        crate::app::ViewMode::Hues => {
            hues::ui_hues(app, ctx, ui);
        }
        crate::app::ViewMode::Clilocs => {
            clilocs::ui_clilocs(app, ctx, ui);
        }
        crate::app::ViewMode::TerrainDefinition => {
            terrain_definition::ui_terrain_definition(app, ctx, ui);
        }
        crate::app::ViewMode::StringDictionary => {
            string_dictionary::ui_string_dictionary(app, ctx, ui);
        }
        crate::app::ViewMode::Sounds => {
            sounds::ui_sounds(app, ctx, ui);
        }
    }

    upscale_preview::ui_upscale_preview_window(app, ctx, ui);
}
