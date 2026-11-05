//! The animations list, the same for every source: bodies, each with its actions; an action entry holds the frames
//! of all its directions (one `anim*.mul` entry per direction, or one AnimationFrame UOP entry).

use crate::app::{AnimationFrameUopEntry, ArtSource, UopInspectorApp};
use crate::ui::{arrow_delta, list_sort_controls, move_selection};
use eframe::egui;
use std::collections::HashSet;
use std::sync::Arc;

const GOTO_FIELD_WIDTH: f32 = 90.0;
/// Indent of an action row under its body.
const ACTION_ROW_INDENT: &str = "    ";
const SCANNING_REPAINT: std::time::Duration = std::time::Duration::from_millis(100);
/// Index of the first `AnimationFrame*.uop` in the package names (group 0).
const FIRST_UOP_PACKAGE_NUMBER: u8 = 1;
const HEX_PREFIX: &str = "0x";
/// `anim{N}.mul` is MUL file index `N - 1` (index 0 is `anim.mul`).
const MUL_FILE_NUMBER_OFFSET: u16 = 1;

/// Where the frames of an action are stored.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum ActionSource {
    /// `anim*.mul` file `file_index` (0 = anim.mul): one entry per direction, the first at `first_index`.
    Mul { file_index: u8, first_index: u32 },
    /// One entry of `AnimationFrame{group + 1}.uop`, all directions.
    Uop { package_index: usize, file_hash: u64, group: u8 },
}

/// One list entry: an action of a body.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct ActionKey {
    pub body: u32,
    pub action: u16,
    pub source: ActionSource,
}

impl ActionKey {
    /// The MUL file or the UOP package group.
    pub(super) fn group(&self) -> u8 {
        match self.source {
            ActionSource::Mul { file_index, .. } => file_index,
            ActionSource::Uop { group, .. } => group,
        }
    }

    pub(super) fn file_hash(&self) -> Option<u64> {
        match self.source {
            ActionSource::Mul { .. } => None,
            ActionSource::Uop { file_hash, .. } => Some(file_hash),
        }
    }

    /// Where the frames come from, as the list shows it.
    pub(super) fn provenance(&self) -> String {
        match self.source {
            ActionSource::Mul { file_index, first_index } => format!("{} idx {first_index}", mul_file_name(file_index)),
            ActionSource::Uop { group, .. } => format!("AnimationFrame{}.uop", group + FIRST_UOP_PACKAGE_NUMBER),
        }
    }

    fn order(&self) -> (u32, u16, u8, usize, u64) {
        match self.source {
            ActionSource::Mul { file_index, first_index } => (self.body, self.action, file_index, 0, u64::from(first_index)),
            ActionSource::Uop { package_index, file_hash, group } => (self.body, self.action, group, package_index, file_hash),
        }
    }
}

/// `anim.mul`, `anim2.mul`, ... for MUL file `file_index`.
pub(super) fn mul_file_name(file_index: u8) -> String {
    match file_index {
        0 => "anim.mul".to_string(),
        index => format!("anim{}.mul", u16::from(index) + MUL_FILE_NUMBER_OFFSET),
    }
}

/// The selection as the app fields hold it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct ListSelection {
    pub body: u32,
    pub action: u16,
    /// MUL file or UOP group.
    pub group: u8,
    /// The UOP entry picked in the list; `None` when the body was typed.
    pub file_hash: Option<u64>,
}

/// A row of the list: a body header, or an action of an expanded body (an index into the keys).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ListRow {
    Body { body: u32, actions: usize },
    Action(usize),
}

#[derive(Default)]
pub(crate) struct AnimationListState {
    /// Bodies shown with their actions.
    expanded: HashSet<u32>,
    goto: String,
    /// The keys of a source, sorted by body, action, then source; the cache key is the source and the entries'
    /// identity (MUL: the file index and loaded file count).
    keys: Option<((u8, usize), Arc<Vec<ActionKey>>)>,
}

/// The action entries of `anim*.mul` file `file_index`, from `(source index, body, action)` per direction entry.
pub(super) fn mul_action_keys(file_index: u8, entries: impl IntoIterator<Item = (u32, u16, u16)>) -> Vec<ActionKey> {
    let mut first_indices = std::collections::HashMap::<(u32, u16), u32>::new();
    for (index, body, action) in entries {
        let first = first_indices.entry((u32::from(body), action)).or_insert(index);
        *first = (*first).min(index);
    }
    let mut keys: Vec<ActionKey> = first_indices
        .into_iter()
        .map(|((body, action), first_index)| ActionKey { body, action, source: ActionSource::Mul { file_index, first_index } })
        .collect();
    sort_keys(&mut keys, false);
    keys
}

/// The action entries of the scanned AnimationFrame UOP entries: one per stored file (the Classic scan lists each
/// file once per direction).
pub(super) fn uop_action_keys(entries: &[AnimationFrameUopEntry]) -> Vec<ActionKey> {
    let mut seen = HashSet::new();
    let mut keys: Vec<ActionKey> = entries
        .iter()
        .filter(|entry| seen.insert((entry.package_index, entry.file_hash)))
        .filter_map(|entry| {
            Some(ActionKey {
                body: entry.body_id,
                action: entry.action_id?,
                source: ActionSource::Uop {
                    package_index: entry.package_index,
                    file_hash: entry.file_hash,
                    group: entry.group_id.unwrap_or_default(),
                },
            })
        })
        .collect();
    sort_keys(&mut keys, false);
    keys
}

/// Bodies ascending, or descending; the actions of a body always ascending.
pub(super) fn sort_keys(keys: &mut [ActionKey], descending: bool) {
    if descending {
        keys.sort_by_key(|key| (std::cmp::Reverse(key.body), key.order()));
    } else {
        keys.sort_by_key(ActionKey::order);
    }
}

fn filter_label(key: &ActionKey) -> String {
    format!("body {} action {} {}", key.body, key.action, key.provenance()).to_ascii_lowercase()
}

/// The rows of `keys` (sorted), a body's actions only when `expanded` says so.
pub(super) fn list_rows(keys: &[ActionKey], expanded: impl Fn(u32) -> bool) -> Vec<ListRow> {
    let mut rows = Vec::new();
    let mut start = 0;
    while start < keys.len() {
        let body = keys[start].body;
        let end = start + keys[start..].iter().take_while(|key| key.body == body).count();
        rows.push(ListRow::Body { body, actions: end - start });
        if expanded(body) {
            rows.extend((start..end).map(ListRow::Action));
        }
        start = end;
    }
    rows
}

/// The listed key the selection names: a UOP entry by its hash, else (`match_unhashed`, a typed body of a source
/// without redirects) by body, action and group.
pub(super) fn find_selected(keys: &[ActionKey], selection: ListSelection, match_unhashed: bool) -> Option<ActionKey> {
    keys.iter().copied().find(|key| {
        key.body == selection.body
            && key.action == selection.action
            && match (key.file_hash(), selection.file_hash) {
                (Some(hash), Some(selected)) => hash == selected,
                (None, _) => key.group() == selection.group,
                (Some(_), None) => match_unhashed && key.group() == selection.group,
            }
    })
}

fn parse_number(text: &str) -> Option<u32> {
    match text.strip_prefix(HEX_PREFIX) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// "Go to": `body [action]`, decimal or `0x` hex. The body's entry of that action (else its first), or when the
/// body isn't listed, the first body after it.
pub(super) fn goto_key(keys: &[ActionKey], text: &str) -> Option<ActionKey> {
    let mut numbers = text.split(|c: char| c.is_whitespace() || c == ',' || c == '/').filter(|part| !part.is_empty());
    let body = parse_number(&numbers.next()?.to_ascii_lowercase())?;
    let action = match numbers.next() {
        Some(part) => Some(parse_number(&part.to_ascii_lowercase())?),
        None => None,
    };
    let of_body = || keys.iter().copied().filter(move |key| key.body == body);
    action
        .and_then(|action| of_body().find(|key| u32::from(key.action) == action))
        .or_else(|| of_body().next())
        .or_else(|| {
            let after = keys.iter().filter(|key| key.body > body).map(|key| key.body).min()?;
            keys.iter().copied().find(|key| key.body == after)
        })
}

pub(super) fn selection_of(app: &UopInspectorApp) -> ListSelection {
    ListSelection {
        body: app.selected_anim_id,
        action: app.selected_action_id,
        group: app.selected_anim_file_idx,
        file_hash: app.selected_animationframe_file_hash,
    }
}

pub(super) fn select_key(app: &mut UopInspectorApp, ctx: &egui::Context, key: ActionKey) {
    app.selected_anim_id = key.body;
    app.selected_action_id = key.action;
    app.selected_anim_file_idx = key.group();
    app.selected_animationframe_file_hash = key.file_hash();
    app.current_frame_idx = 0;
    app.last_frame_time = ctx.input(|input| input.time);
}

/// The sorted action entries of `source`; `None` while the UOP scan runs.
pub(super) fn action_keys(app: &mut UopInspectorApp, source: ArtSource) -> Option<Arc<Vec<ActionKey>>> {
    let identity = match source {
        ArtSource::Mul => {
            let anim_map = app.client_data.as_ref().and_then(|client| client.anim_map.clone())?;
            let file_index = app.selected_anim_file_idx;
            let cache_key = (source as u8, usize::from(file_index));
            if let Some((cached, keys)) = &app.animations_view.list.keys {
                if *cached == cache_key {
                    return Some(keys.clone());
                }
            }
            let count = anim_map.source_index_count(file_index).unwrap_or_default();
            let entries = (0..count as u32).filter(|&index| anim_map.has_anim(file_index, index)).filter_map(|index| {
                let identity = uocf::classic::anim::classic_animation_identity_from_source_index(file_index, index);
                (identity.flags == 0).then_some((index, identity.body_id, identity.action_id))
            });
            let keys = Arc::new(mul_action_keys(file_index, entries));
            app.animations_view.list.keys = Some((cache_key, keys.clone()));
            return Some(keys);
        }
        ArtSource::CcUop | ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => {
            super::animations::animationframe_uop_entries(app, source)?
        }
        ArtSource::Any => return Some(Arc::default()),
    };
    let cache_key = (source as u8, Arc::as_ptr(&identity) as usize);
    if let Some((cached, keys)) = &app.animations_view.list.keys {
        if *cached == cache_key {
            return Some(keys.clone());
        }
    }
    let keys = Arc::new(uop_action_keys(&identity));
    app.animations_view.list.keys = Some((cache_key, keys));
    app.animations_view.list.keys.as_ref().map(|(_, keys)| keys.clone())
}

/// A selectable row of the list, its text on the left (a sized button centres it).
fn list_row(ui: &mut egui::Ui, size: egui::Vec2, selected: bool, text: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() {
        ui.painter().rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
    }
    let position = rect.left_center() + egui::vec2(ui.spacing().button_padding.x, 0.0);
    let font = egui::TextStyle::Button.resolve(ui.style());
    ui.painter().text(position, egui::Align2::LEFT_CENTER, text, font, visuals.text_color());
    response
}

/// Filter, go to, sort, and the list itself; arrows move the selection when no field has the keyboard.
pub(super) fn show_action_list(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, match_unhashed: bool) {
    let source = app.selected_legacy_source;
    let mut goto_requested = false;
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.add(egui::TextEdit::singleline(&mut app.searches.animations).desired_width(GOTO_FIELD_WIDTH))
            .on_hover_text("Matches \"body N action M\" and the file the entry comes from.");
        ui.label("Go to:");
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.animations_view.list.goto)
                .desired_width(GOTO_FIELD_WIDTH)
                .hint_text("body [action]"),
        );
        goto_requested = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        goto_requested |= ui.button("Go").clicked();
    });
    let sort = list_sort_controls(ui, "animation_tree_entries", &["Body"], 0);
    ui.horizontal(|ui| {
        if ui.button("Collapse All").clicked() {
            app.animations_view.list.expanded.clear();
        }
        if super::animations::is_ec_source(source) && ui.button("Rescan Animations").clicked() {
            let key = (source as u8, app.uop_cache.loaded_uops.len());
            app.animationframe_uop_entries.remove(&key);
            app.animationframe_uop_worker_rx = None;
            app.animationframe_uop_worker_key = None;
        }
    });

    let Some(keys) = action_keys(app, source) else {
        ui.label("Scanning AnimationFrame UOP metadata...");
        ctx.request_repaint_after(SCANNING_REPAINT);
        return;
    };
    let query = app.searches.animations.trim().to_ascii_lowercase();
    let mut visible: Vec<ActionKey> =
        keys.iter().copied().filter(|key| query.is_empty() || filter_label(key).contains(&query)).collect();
    if sort.ordering() == Some(true) {
        sort_keys(&mut visible, true);
    }

    let selected = find_selected(&visible, selection_of(app), match_unhashed);
    let mut scroll_to = None;
    if let Some(delta) = arrow_delta(ui, ctx.egui_wants_keyboard_input()) {
        scroll_to = move_selection(&visible, selected, delta);
    }
    if goto_requested {
        scroll_to = goto_key(&visible, &app.animations_view.list.goto).or(scroll_to);
    }
    if let Some(key) = scroll_to {
        select_key(app, ctx, key);
        app.animations_view.list.expanded.insert(key.body);
    }
    let selected = scroll_to.or(selected);

    let expanded = &app.animations_view.list.expanded;
    let rows = list_rows(&visible, |body| !query.is_empty() || expanded.contains(&body));
    if rows.is_empty() {
        ui.label("No animations match the current source/filter.");
        return;
    }
    let row_height = ui.spacing().interact_size.y;
    let row_step = row_height + ui.spacing().item_spacing.y;
    let viewport_height = ui.available_height();
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("animation_action_list", source as u8))
        .auto_shrink([false, false])
        .max_height(viewport_height);
    if let Some(row) = scroll_to.and_then(|key| rows.iter().position(|row| matches!(row, ListRow::Action(index) if visible[*index] == key))) {
        area = area.vertical_scroll_offset((row as f32 * row_step - (viewport_height - row_height) / 2.0).max(0.0));
    }
    let mut clicked = None;
    let mut toggled = None;
    area.show_rows(ui, row_height, rows.len(), |ui, range| {
        let width = ui.available_width();
        for row in &rows[range] {
            match *row {
                ListRow::Body { body, actions } => {
                    let open = !query.is_empty() || app.animations_view.list.expanded.contains(&body);
                    let marker = if open { "⏷" } else { "⏵" };
                    let has_selection = selected.is_some_and(|key| key.body == body);
                    let text = format!("{marker} Body {body} ({actions} action{})", if actions == 1 { "" } else { "s" });
                    if list_row(ui, egui::vec2(width, row_height), has_selection && !open, &text).clicked() {
                        toggled = Some(body);
                    }
                }
                ListRow::Action(index) => {
                    let key = visible[index];
                    let text = format!("{ACTION_ROW_INDENT}Action {} · {}", key.action, key.provenance());
                    if list_row(ui, egui::vec2(width, row_height), selected == Some(key), &text).clicked() {
                        clicked = Some(key);
                    }
                }
            }
        }
    });
    if let Some(body) = toggled {
        let expanded = &mut app.animations_view.list.expanded;
        if !expanded.remove(&body) {
            expanded.insert(body);
        }
    }
    if let Some(key) = clicked {
        select_key(app, ctx, key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uop_entry(body_id: u32, action: u16, direction: Option<u8>, file_hash: u64) -> AnimationFrameUopEntry {
        AnimationFrameUopEntry {
            package_index: 0,
            file_hash,
            body_id,
            action_id: Some(action),
            direction,
            group_id: Some(1),
            source_index: u32::from(action),
        }
    }

    fn body_actions(keys: &[ActionKey]) -> Vec<(u32, u16)> {
        keys.iter().map(|key| (key.body, key.action)).collect()
    }

    #[test]
    fn classic_and_enhanced_list_the_same_body_action_entries_in_the_same_order() {
        // The Classic scan lists each file once per direction; the Enhanced scan once.
        let classic: Vec<_> = [(2, 1, 0xA), (1, 3, 0xB), (1, 0, 0xC)]
            .into_iter()
            .flat_map(|(body, action, hash)| (0..5).map(move |direction| uop_entry(body, action, Some(direction), hash)))
            .collect();
        let enhanced: Vec<_> =
            [(2, 1, 0xA), (1, 3, 0xB), (1, 0, 0xC)].into_iter().map(|(body, action, hash)| uop_entry(body, action, None, hash)).collect();
        let mul = mul_action_keys(0, (0..5).map(|direction| (100 + direction, 2, 1)).chain((0..5).map(|d| (40 + d, 1, 3))).chain((0..5).map(|d| (30 + d, 1, 0))));
        let expected = vec![(1, 0), (1, 3), (2, 1)];
        assert_eq!(body_actions(&uop_action_keys(&classic)), expected);
        assert_eq!(body_actions(&uop_action_keys(&enhanced)), expected);
        assert_eq!(body_actions(&mul), expected);
        assert_eq!(mul[0].source, ActionSource::Mul { file_index: 0, first_index: 30 });
    }

    #[test]
    fn descending_reverses_the_bodies_and_keeps_their_actions_ascending() {
        let mut keys = uop_action_keys(&[uop_entry(1, 0, None, 1), uop_entry(1, 3, None, 2), uop_entry(2, 1, None, 3)]);
        sort_keys(&mut keys, true);
        assert_eq!(body_actions(&keys), vec![(2, 1), (1, 0), (1, 3)]);
    }

    #[test]
    fn arrows_walk_the_entries_across_bodies_and_the_rows_open_the_body_of_the_selection() {
        let keys = uop_action_keys(&[uop_entry(1, 0, None, 1), uop_entry(1, 3, None, 2), uop_entry(2, 1, None, 3)]);
        let selection = ListSelection { body: 1, action: 3, group: 1, file_hash: Some(2) };
        let selected = find_selected(&keys, selection, false);
        assert_eq!(selected, Some(keys[1]));
        let next = move_selection(&keys, selected, 1);
        assert_eq!(next, Some(keys[2]), "down from the last action of body 1 is the first of body 2");
        assert_eq!(move_selection(&keys, None, 1), Some(keys[0]));
        let rows = list_rows(&keys, |body| body == 2);
        assert_eq!(rows, vec![ListRow::Body { body: 1, actions: 2 }, ListRow::Body { body: 2, actions: 1 }, ListRow::Action(2)]);
    }

    #[test]
    fn a_typed_body_selects_its_entry_only_where_no_redirect_applies() {
        let keys = uop_action_keys(&[uop_entry(5, 2, None, 9)]);
        let typed = ListSelection { body: 5, action: 2, group: 1, file_hash: None };
        assert_eq!(find_selected(&keys, typed, true), Some(keys[0]));
        assert_eq!(find_selected(&keys, typed, false), None);
        let mul = mul_action_keys(0, [(7, 5, 2)]);
        assert_eq!(find_selected(&mul, typed, false), None, "another MUL file");
        assert_eq!(find_selected(&mul, ListSelection { group: 0, ..typed }, false), Some(mul[0]));
    }

    #[test]
    fn go_to_takes_a_body_and_an_optional_action_in_decimal_or_hex() {
        let keys = uop_action_keys(&[uop_entry(16, 0, None, 1), uop_entry(16, 4, None, 2), uop_entry(30, 1, None, 3)]);
        assert_eq!(goto_key(&keys, "16"), Some(keys[0]));
        assert_eq!(goto_key(&keys, "0x10 4"), Some(keys[1]));
        assert_eq!(goto_key(&keys, "16/9"), Some(keys[0]), "a missing action goes to the body");
        assert_eq!(goto_key(&keys, "20"), Some(keys[2]), "a missing body goes to the next one");
        assert_eq!(goto_key(&keys, "31"), None);
        assert_eq!(goto_key(&keys, "x"), None);
    }
}
