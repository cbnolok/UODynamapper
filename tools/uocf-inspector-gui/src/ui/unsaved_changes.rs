//! The status bar's save button, and the question asked when a tab with unsaved changes is left.
//!
//! A tab joins by defining a [`TabEditor`] and listing it in [`TAB_EDITORS`]; nothing else changes in the tab. Tab
//! switches are noticed in one place, whatever made them (the top bar or a jump from a tab's own button).

use eframe::egui;

use crate::app::{UopInspectorApp, ViewMode};

const SAVE_BUTTON_TEXT: &str = "Save active file changes";
const NOTHING_TO_SAVE_HOVER: &str = "This tab has no unsaved changes.";
const SAVE_COPY_NOTE: &str = "The tool asks where to write; a loaded client file is refused as the target.";
const DISCARD_HOVER: &str = "Back to the data as opened. Copies you already saved are not touched.";

/// The editor of a file on a tab. One tab can have several (the UOP Explorer does).
pub(crate) struct TabEditor {
    /// The tab the editor is on.
    pub tab: ViewMode,
    /// What it edits, for the messages.
    pub label: &'static str,
    pub is_dirty: fn(&UopInspectorApp) -> bool,
    /// Asks where to write, and writes. It stays dirty when the user cancels, or when the save fails (the tab shows
    /// why), and the tab is then not left.
    pub save: fn(&mut UopInspectorApp),
    /// Back to the data as opened, with nothing left to save.
    pub discard: fn(&mut UopInspectorApp),
}

const TAB_EDITORS: &[TabEditor] = &[
    super::housing::TAB_EDITOR,
    super::animation_definitions::TAB_EDITOR,
    super::effects::TAB_EDITOR,
    super::multi_editor::TAB_EDITOR,
    super::tileart_editor::TAB_EDITOR,
    super::clilocs::TAB_EDITOR,
    super::legacy_terrain::TAB_EDITOR,
    super::terrain_definition_editor::TAB_EDITOR,
    super::uop_browser::WAYPOINTS_TAB_EDITOR,
    super::uop_browser::TEXT_ENTRIES_TAB_EDITOR,
];

/// A tab switch held back by the question.
#[derive(Default)]
pub(crate) struct UnsavedChanges {
    /// The tab on screen: an `app.view_mode` that differs from it is a switch.
    shown_tab: Option<ViewMode>,
    /// The tab the user asked for, while the question is open.
    pending_tab: Option<ViewMode>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Save,
    Discard,
    Stay,
}

fn dirty_editors(app: &UopInspectorApp, tab: ViewMode) -> Vec<&'static TabEditor> {
    TAB_EDITORS.iter().filter(|editor| editor.tab == tab && (editor.is_dirty)(app)).collect()
}

/// Notices a tab switch, and keeps the tab on screen while it has unsaved changes.
pub(super) fn hold_tab_switch(app: &mut UopInspectorApp) {
    let shown = *app.unsaved_changes.shown_tab.get_or_insert(app.view_mode);
    if app.view_mode == shown {
        return;
    }
    if dirty_editors(app, shown).is_empty() {
        app.unsaved_changes.shown_tab = Some(app.view_mode);
    } else {
        app.unsaved_changes.pending_tab = Some(app.view_mode);
        app.view_mode = shown;
    }
}

/// Goes to `tab` from a tab that can no longer be shown, without asking: its editors keep their changes for when it
/// is back.
pub(super) fn leave_a_tab_that_is_gone(app: &mut UopInspectorApp, tab: ViewMode) {
    app.view_mode = tab;
    app.unsaved_changes.shown_tab = Some(tab);
    app.unsaved_changes.pending_tab = None;
}

/// The status bar's button: saves the unsaved changes of the tab on screen.
pub(super) fn save_button(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let editors = dirty_editors(app, app.view_mode);
    let labels: Vec<&str> = editors.iter().map(|editor| editor.label).collect();
    let clicked = ui
        .add_enabled(!editors.is_empty(), egui::Button::new(SAVE_BUTTON_TEXT))
        .on_hover_text(format!("Saves: {}.\n{SAVE_COPY_NOTE}", labels.join(", ")))
        .on_disabled_hover_text(NOTHING_TO_SAVE_HOVER)
        .clicked();
    if clicked {
        for editor in editors {
            (editor.save)(app);
        }
    }
}

/// Asks what to do with the unsaved changes of the tab being left.
pub(super) fn leave_tab_window(app: &mut UopInspectorApp, ctx: &egui::Context) {
    if app.unsaved_changes.pending_tab.is_none() {
        return;
    }
    let editors = dirty_editors(app, app.view_mode);
    let mut answer = None;
    egui::Window::new("Unsaved changes")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("This tab has unsaved changes:");
            for editor in &editors {
                ui.label(editor.label);
            }
            ui.horizontal(|ui| {
                if ui.button("Save...").on_hover_text(SAVE_COPY_NOTE).clicked() {
                    answer = Some(Answer::Save);
                }
                if ui.button("Discard").on_hover_text(DISCARD_HOVER).clicked() {
                    answer = Some(Answer::Discard);
                }
                if ui.button("Stay on this tab").clicked() {
                    answer = Some(Answer::Stay);
                }
            });
        });
    if let Some(answer) = answer {
        answer_question(app, answer);
    }
    finish_switch_when_clean(app);
}

fn answer_question(app: &mut UopInspectorApp, answer: Answer) {
    let editors = dirty_editors(app, app.view_mode);
    match answer {
        Answer::Save => {
            for editor in editors {
                (editor.save)(app);
            }
        }
        Answer::Discard => {
            for editor in editors {
                (editor.discard)(app);
            }
        }
        Answer::Stay => app.unsaved_changes.pending_tab = None,
    }
}

/// Goes to the asked tab once nothing is left to save (it can also have been saved by the tab's own button).
fn finish_switch_when_clean(app: &mut UopInspectorApp) {
    let Some(pending) = app.unsaved_changes.pending_tab else {
        return;
    };
    if dirty_editors(app, app.view_mode).is_empty() {
        app.unsaved_changes.pending_tab = None;
        app.unsaved_changes.shown_tab = Some(pending);
        app.view_mode = pending;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;
    use std::path::Path;

    const ENTRY_HASH: u64 = 1;
    const FRAMES: usize = 2;

    /// An app on the UOP Explorer tab, with an edited text entry there.
    fn app_with_a_text_edit() -> UopInspectorApp {
        let mut app = test_app();
        app.view_mode = ViewMode::UopExplorer;
        hold_tab_switch(&mut app);
        let edit = app.entry_text_edits.open(Path::new("/packages/GameData.uop"), ENTRY_HASH, b"text\n").unwrap();
        edit.buffer.push('!');
        app
    }

    #[test]
    fn a_tab_without_changes_is_left_at_once() {
        let mut app = test_app();
        app.view_mode = ViewMode::Effects;
        hold_tab_switch(&mut app);
        app.view_mode = ViewMode::Home;
        hold_tab_switch(&mut app);
        assert_eq!(app.view_mode, ViewMode::Home);
        assert_eq!(app.unsaved_changes.pending_tab, None);
    }

    #[test]
    fn leaving_a_tab_with_changes_asks_first() {
        let mut app = app_with_a_text_edit();
        app.view_mode = ViewMode::Home;
        hold_tab_switch(&mut app);
        assert_eq!(app.view_mode, ViewMode::UopExplorer, "the switch is held");
        assert_eq!(app.unsaved_changes.pending_tab, Some(ViewMode::Home));

        answer_question(&mut app, Answer::Stay);
        finish_switch_when_clean(&mut app);
        assert_eq!(app.view_mode, ViewMode::UopExplorer);
        assert_eq!(app.unsaved_changes.pending_tab, None);
        assert!(!dirty_editors(&app, ViewMode::UopExplorer).is_empty(), "staying keeps the changes");
    }

    #[test]
    fn discarding_drops_the_changes_and_goes_on() {
        let mut app = app_with_a_text_edit();
        app.view_mode = ViewMode::Home;
        hold_tab_switch(&mut app);
        answer_question(&mut app, Answer::Discard);
        finish_switch_when_clean(&mut app);
        assert_eq!(app.view_mode, ViewMode::Home);
        assert!(dirty_editors(&app, ViewMode::UopExplorer).is_empty());
        hold_tab_switch(&mut app);
        assert_eq!(app.view_mode, ViewMode::Home, "the tab on screen followed the switch");
    }

    #[test]
    fn a_saved_entry_has_nothing_left_to_ask_about() {
        let mut app = app_with_a_text_edit();
        let edit = app.entry_text_edits.open(Path::new("/packages/GameData.uop"), ENTRY_HASH, &[]).unwrap();
        edit.mark_saved();
        assert!(!edit.has_unsaved_changes() && edit.modified(), "a saved copy leaves the loaded package as it was");
        app.view_mode = ViewMode::Home;
        hold_tab_switch(&mut app);
        assert_eq!(app.view_mode, ViewMode::Home);
    }

    #[test]
    fn every_editor_copes_with_an_app_with_nothing_open() {
        let mut app = test_app();
        for editor in TAB_EDITORS {
            assert!(!(editor.is_dirty)(&app), "{}", editor.label);
            (editor.discard)(&mut app);
            assert!(!(editor.is_dirty)(&app), "{}", editor.label);
        }
    }

    fn run_frames(app: &mut UopInspectorApp) {
        let ctx = egui::Context::default();
        for _ in 0..FRAMES {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let ctx = ui.ctx().clone();
                crate::ui::draw_ui(app, &ctx, ui);
            });
        }
    }

    #[test]
    fn the_status_bar_and_the_question_draw_without_panicking() {
        let mut app = test_app();
        app.view_mode = ViewMode::Housing;
        hold_tab_switch(&mut app);
        app.unsaved_changes.pending_tab = Some(ViewMode::Home);
        run_frames(&mut app);
        assert_eq!(app.view_mode, ViewMode::Home, "nothing was left to save, so the switch went on");
        assert_eq!(app.unsaved_changes.pending_tab, None);
    }

    #[test]
    fn a_tab_that_cannot_be_shown_is_left_without_asking() {
        let mut app = app_with_a_text_edit();
        run_frames(&mut app);
        assert_eq!(app.view_mode, ViewMode::Home, "no package is loaded, so the explorer has nothing to show");
        assert_eq!(app.unsaved_changes.pending_tab, None);
        assert!(app.entry_text_edits.any_unsaved(), "the changes wait for the package");
    }
}
