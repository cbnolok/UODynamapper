//! Editing controls of the Def tab, through `uocf::classic::def_files`, `speech` and `skills`.
//!
//! A row of the tab's table is edited as text fields, and read back by the file's own rules before it is taken. Lines
//! and records that are not edited keep their bytes, comments and spacing too.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use color_eyre::eyre;
use eframe::egui;
use uocf::classic::def_files::{BodyConvRecord, DefDocument, DefRecord, EquipConvRecord, TargetsRecord};
use uocf::classic::skills::{Skill, SkillGroupName, SKILLS_INDEX_FILE};
use uocf::classic::speech::SpeechEntry;
use uocf::utils::path::find_file_ignoring_case;

use super::classic_defs::{read_file, read_table, table_of, DefFile, FileKind, Table};
use crate::app::{UopInspectorApp, ViewMode};
use crate::ui::editor_bar::{editor_bar, pick_output, report_saved, BarAction, MUL_FILTER};
use crate::ui::tiledata_editor::same_file;
use crate::ui::unsaved_changes::TabEditor;

const DEF_FILTER: (&str, &[&str]) = ("Def file", &["def"]);
const INDEX_EXTENSION: &str = "idx";
const NEW_SKILL_NAME: &str = "New skill";
const NEW_GROUP_NAME: &str = "New group";
const YES: &str = "yes";
const NO: &str = "no";
/// The anim file columns of a new `bodyconv.def` line in a file that has no line to copy the count from.
const DEFAULT_BODY_COLUMNS: usize = 4;
const NO_BODY: i32 = -1;
const FIELD_WIDTH: f32 = 360.0;
const EDIT_HOVER: &str = "Change, add or remove rows of the selected file and save a copy. Lines you do not edit keep their bytes.";
const LIST_HOVER: &str = "Numbers separated by commas or spaces.";

struct EditedDef {
    kind: FileKind,
    source: PathBuf,
    data: DefFile,
    /// The file as it was before the first change since loading or saving; `Some` is the one unsaved state.
    opened: Option<DefFile>,
}

#[derive(Default)]
pub struct DefsEditor {
    pub(super) enabled: bool,
    /// The folder the files were read from.
    root: Option<PathBuf>,
    files: Vec<EditedDef>,
    /// The table row being edited, with the text of its fields.
    row: Option<usize>,
    values: Vec<String>,
    /// Grows with every change, for the tab to rebuild its table.
    generation: u64,
    error: Option<String>,
}

fn number<T: FromStr>(name: &str, text: &str) -> Result<T, String> {
    text.trim().parse().map_err(|_| format!("{name}: {text:?} is not a number this field holds"))
}

fn hexadecimal(name: &str, text: &str) -> Result<u32, String> {
    u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).map_err(|_| format!("{name}: {text:?} is not a hexadecimal number"))
}

fn number_list(name: &str, text: &str) -> Result<Vec<i32>, String> {
    text.split([',', ' ', '\t']).filter(|part| !part.is_empty()).map(|part| number(name, part)).collect()
}

fn list_text(values: &[i32], separator: &str) -> String {
    values.iter().map(i32::to_string).collect::<Vec<_>>().join(separator)
}

fn comment_of(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

/// The line of `record` must read back as the same record, or the file would not say what the row says.
fn reads_back<R: DefRecord>(record: &R) -> Result<(), String> {
    match R::parse(&record.format(), 1) {
        Ok(Some(read)) if read == *record => Ok(()),
        Ok(_) => Err(format!("the line \"{}\" would not read back the same", record.format())),
        Err(error) => Err(format!("the line \"{}\" would not read back: {}", record.format(), error.message)),
    }
}

fn replace_record<R: DefRecord>(document: &mut DefDocument<R>, row: usize, record: R) -> Result<(), String> {
    reads_back(&record)?;
    let slot = document.records_mut().nth(row).ok_or_else(|| format!("no row {row}"))?;
    *slot = record;
    Ok(())
}

fn remove_record<R: DefRecord>(document: &mut DefDocument<R>, row: usize) {
    let mut at = 0;
    document.remove_where(|_| {
        at += 1;
        at - 1 == row
    });
}

/// The id of the skill on table row `row`: the table lists the skills that exist.
fn skill_id(file: &uocf::classic::skills::SkillsFile, row: usize) -> Option<usize> {
    (0..file.len()).filter(|&id| matches!(file.skill(id), Ok(Some(_)))).nth(row)
}

/// The names and texts of the fields of table row `row`.
fn fields_of(file: &DefFile, row: usize) -> Option<Vec<(&'static str, String)>> {
    Some(match file {
        DefFile::Targets(document) => {
            let record = document.records().nth(row)?;
            vec![
                ("Id", record.id.to_string()),
                ("Targets", list_text(&record.targets, ", ")),
                ("Value (hue)", record.value.to_string()),
                ("Comment", record.comment.clone().unwrap_or_default()),
            ]
        }
        DefFile::BodyConv(document) => {
            let record = document.records().nth(row)?;
            vec![
                ("Body", record.body.to_string()),
                ("anim2, anim3, anim4, anim5, ... bodies (-1: none)", list_text(&record.anim_file_bodies, " ")),
                ("Comment", record.comment.clone().unwrap_or_default()),
            ]
        }
        DefFile::EquipConv(document) => {
            let record = document.records().nth(row)?;
            vec![
                ("Body", record.body.to_string()),
                ("Equipment", record.equipment.to_string()),
                ("Converted", record.converted.to_string()),
                ("Gump", record.gump.to_string()),
                ("Hue (hexadecimal)", format!("{:X}", record.hue)),
                ("Comment", record.comment.clone().unwrap_or_default()),
            ]
        }
        DefFile::Speech(file) => {
            let entry = file.entries.get(row)?;
            vec![("Keyword id", entry.id.to_string()), ("Text", entry.text.clone())]
        }
        DefFile::Skills(file) => {
            let skill = file.skill(skill_id(file, row)?).ok()??;
            vec![("Button (yes or no)", if skill.action { YES } else { NO }.to_owned()), ("Name", skill.name)]
        }
        DefFile::SkillGroups(file) => {
            let group = *file.skill_groups.get(row)?;
            vec![("Group", group.to_string()), ("Group name", file.group_name(group as usize).unwrap_or_default().to_owned())]
        }
    })
}

/// Takes the texts of `fields_of` back into row `row`.
fn apply_fields(file: &mut DefFile, row: usize, values: &[String]) -> Result<(), String> {
    let text = |index: usize| values.get(index).map_or("", String::as_str);
    match file {
        DefFile::Targets(document) => {
            let record = TargetsRecord {
                id: number("Id", text(0))?,
                targets: number_list("Targets", text(1))?,
                value: number("Value", text(2))?,
                comment: comment_of(text(3)),
            };
            replace_record(document, row, record)
        }
        DefFile::BodyConv(document) => {
            let record = BodyConvRecord {
                body: number("Body", text(0))?,
                anim_file_bodies: number_list("Bodies", text(1))?,
                comment: comment_of(text(2)),
            };
            replace_record(document, row, record)
        }
        DefFile::EquipConv(document) => {
            let record = EquipConvRecord {
                body: number("Body", text(0))?,
                equipment: number("Equipment", text(1))?,
                converted: number("Converted", text(2))?,
                gump: number("Gump", text(3))?,
                hue: hexadecimal("Hue", text(4))?,
                comment: comment_of(text(5)),
            };
            replace_record(document, row, record)
        }
        DefFile::Speech(file) => {
            let entry = file.entries.get_mut(row).ok_or_else(|| format!("no row {row}"))?;
            *entry = SpeechEntry { id: number("Keyword id", text(0))?, text: text(1).to_owned() };
            Ok(())
        }
        DefFile::Skills(file) => {
            let id = skill_id(file, row).ok_or_else(|| format!("no row {row}"))?;
            let action = match text(0).trim() {
                YES => true,
                NO => false,
                other => return Err(format!("Button: {other:?} is not {YES} or {NO}")),
            };
            file.set_skill(id, Some(&Skill { action, name: text(1).to_owned() })).map_err(|error| format!("{error:#}"))
        }
        DefFile::SkillGroups(file) => {
            let group: i32 = number("Group", text(0))?;
            let name = usize::try_from(group).ok().and_then(|group| group.checked_sub(1)).and_then(|index| file.groups.get_mut(index));
            match name {
                Some(entry) => {
                    if entry.name != text(1) {
                        entry.name = text(1).to_owned();
                    }
                }
                None if group == 0 && text(1).is_empty() => {}
                None if group == 0 => return Err("Group 0 has no name in the file.".to_owned()),
                None => return Err(format!("Group {group}: the file has groups 0 to {}. Add a group first.", file.groups.len())),
            }
            let slot = file.skill_groups.get_mut(row).ok_or_else(|| format!("no row {row}"))?;
            *slot = group;
            Ok(())
        }
    }
}

/// Adds a row at the end of the table.
fn add_row(file: &mut DefFile) -> Result<(), String> {
    match file {
        DefFile::Targets(document) => document.push(TargetsRecord { id: 0, targets: vec![0], value: 0, comment: None }),
        DefFile::BodyConv(document) => {
            let columns = document.records().next().map_or(DEFAULT_BODY_COLUMNS, |record| record.anim_file_bodies.len());
            document.push(BodyConvRecord { body: 0, anim_file_bodies: vec![NO_BODY; columns], comment: None });
        }
        DefFile::EquipConv(document) => document.push(EquipConvRecord { body: 0, equipment: 0, converted: 0, gump: 0, hue: 0, comment: None }),
        DefFile::Speech(file) => file.entries.push(SpeechEntry { id: 0, text: String::new() }),
        DefFile::Skills(file) => {
            let skill = Skill { action: false, name: NEW_SKILL_NAME.to_owned() };
            file.set_skill(file.len(), Some(&skill)).map_err(|error| format!("{error:#}"))?;
        }
        DefFile::SkillGroups(file) => file.skill_groups.push(0),
    }
    Ok(())
}

/// Removes table row `row`. A skill's group is by position, so `skillgrp.mul` has only its last row removed.
fn remove_row(file: &mut DefFile, row: usize) -> Result<(), String> {
    match file {
        DefFile::Targets(document) => remove_record(document, row),
        DefFile::BodyConv(document) => remove_record(document, row),
        DefFile::EquipConv(document) => remove_record(document, row),
        DefFile::Speech(file) => {
            file.entries.remove(row);
        }
        DefFile::Skills(file) => {
            let id = skill_id(file, row).ok_or_else(|| format!("no row {row}"))?;
            file.set_skill(id, None).map_err(|error| format!("{error:#}"))?;
        }
        DefFile::SkillGroups(file) => {
            if row + 1 != file.skill_groups.len() {
                return Err("A skill's group goes by its position: only the last row can be removed.".to_owned());
            }
            file.skill_groups.pop();
        }
    }
    Ok(())
}

/// The file must still write: a text too long for its record, or a group out of range, is told at once.
fn check_writes(file: &DefFile) -> Result<(), String> {
    let report = |error: eyre::Report| format!("{error:#}");
    match file {
        DefFile::Targets(_) | DefFile::BodyConv(_) | DefFile::EquipConv(_) => Ok(()),
        DefFile::Speech(file) => file.to_bytes().map(drop).map_err(report),
        DefFile::Skills(file) => file.to_bytes().map(drop).map_err(report),
        DefFile::SkillGroups(file) => file.to_bytes().map(drop).map_err(report),
    }
}

/// Writes `file` to `output`, and `skills.idx` beside `skills.mul`. A file it was read from is refused.
fn write_copy(file: &EditedDef, output: &Path) -> eyre::Result<()> {
    let refuse = |source: &Path, target: &Path| {
        if same_file(source, target) {
            eyre::bail!("the loaded {} is refused as the target: save a copy", file.kind.name());
        }
        Ok(())
    };
    refuse(&file.source, output)?;
    match &file.data {
        DefFile::Targets(document) => document.save(output),
        DefFile::BodyConv(document) => document.save(output),
        DefFile::EquipConv(document) => document.save(output),
        DefFile::Speech(data) => data.save(output),
        DefFile::Skills(data) => {
            let index = output.with_extension(INDEX_EXTENSION);
            let source_index = file.source.parent().and_then(|dir| find_file_ignoring_case(dir, SKILLS_INDEX_FILE));
            if let Some(source_index) = source_index {
                refuse(&source_index, &index)?;
            }
            data.save(&index, output)
        }
        DefFile::SkillGroups(data) => data.save(output),
    }
}

impl DefsEditor {
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn is_row(&self, row: usize) -> bool {
        self.row == Some(row)
    }

    fn file(&self, kind: FileKind) -> Option<&EditedDef> {
        self.files.iter().find(|file| file.kind == kind)
    }

    /// The table of `kind`: of the file being edited when it is open, else as read from `root`.
    pub(super) fn table(&self, root: &Path, kind: FileKind) -> Result<Table, String> {
        match self.file(kind) {
            Some(file) => table_of(&file.data),
            None => read_table(root, kind),
        }
    }

    fn is_dirty(&self) -> bool {
        self.files.iter().any(|file| file.opened.is_some())
    }

    fn revert(&mut self) {
        for file in &mut self.files {
            if let Some(opened) = file.opened.take() {
                file.data = opened;
            }
        }
        self.row = None;
        self.generation += 1;
    }

    /// Reads the file of `kind` when it is not open yet.
    fn open(&mut self, root: &Path, kind: FileKind) -> Result<(), String> {
        if self.root.as_deref() != Some(root) && !self.is_dirty() {
            self.files.clear();
            self.row = None;
            self.root = Some(root.to_path_buf());
            self.generation += 1;
        }
        if self.file(kind).is_none() {
            let source = find_file_ignoring_case(root, kind.name()).ok_or_else(|| format!("no {} in {}", kind.name(), root.display()))?;
            self.files.push(EditedDef { kind, source, data: read_file(root, kind)?, opened: None });
            self.generation += 1;
        }
        Ok(())
    }

    /// Starts editing row `row`, or stops when it is the row being edited.
    pub(super) fn select_row(&mut self, kind: FileKind, row: usize) {
        if self.is_row(row) {
            self.row = None;
            return;
        }
        let Some(fields) = self.file(kind).and_then(|file| fields_of(&file.data, row)) else {
            return;
        };
        self.values = fields.into_iter().map(|(_, value)| value).collect();
        self.row = Some(row);
    }

    /// The row is no longer the one on the table: the selected file changed.
    pub(super) fn clear_row(&mut self) {
        self.row = None;
    }

    /// Runs `change` on a copy of the file, and keeps the copy when it is good and differs.
    fn update(&mut self, kind: FileKind, change: impl FnOnce(&mut DefFile) -> Result<(), String>) -> bool {
        let Some(file) = self.files.iter_mut().find(|file| file.kind == kind) else {
            return false;
        };
        let mut changed = file.data.clone();
        if let Err(error) = change(&mut changed).and_then(|()| check_writes(&changed)) {
            self.error = Some(error);
            return false;
        }
        self.error = None;
        if changed == file.data {
            return false;
        }
        let old = std::mem::replace(&mut file.data, changed);
        file.opened.get_or_insert(old);
        self.generation += 1;
        true
    }

    /// The fields of the edited row, and adding and removing rows.
    pub(super) fn record_panel(&mut self, ui: &mut egui::Ui, kind: FileKind) {
        let fields = self.row.and_then(|row| self.file(kind).and_then(|file| fields_of(&file.data, row)));
        let mut apply = false;
        if let Some(fields) = &fields {
            egui::Grid::new("classic_defs_record_fields").num_columns(2).show(ui, |ui| {
                for ((name, _), value) in fields.iter().zip(&mut self.values) {
                    let label = ui.label(*name);
                    if name.ends_with("(-1: none)") || *name == "Targets" {
                        label.on_hover_text(LIST_HOVER);
                    }
                    let response = ui.add(egui::TextEdit::singleline(value).desired_width(FIELD_WIDTH));
                    apply |= response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    ui.end_row();
                }
            });
        } else {
            ui.weak("Click a row to edit it.");
        }
        let mut add = false;
        let mut add_group = false;
        let mut remove = false;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(fields.is_some(), |ui| {
                apply |= ui.button("Apply").clicked();
                remove = ui.button("Remove row").clicked();
            });
            add = ui.button("Add row").clicked();
            if kind == FileKind::SkillGroups {
                add_group = ui.button("Add group").on_hover_text("Adds a group at the end; a skill joins it through its Group field.").clicked();
            }
        });
        if apply {
            if let Some(row) = self.row {
                let values = self.values.clone();
                self.update(kind, |file| apply_fields(file, row, &values));
            }
        }
        if add {
            self.update(kind, add_row);
        }
        if add_group {
            self.update(kind, |file| {
                if let DefFile::SkillGroups(file) = file {
                    file.groups.push(SkillGroupName::new(NEW_GROUP_NAME));
                }
                Ok(())
            });
        }
        if remove {
            if let Some(row) = self.row {
                if self.update(kind, |file| remove_row(file, row)) {
                    self.row = None;
                }
            }
        }
    }
}

fn editor(app: &mut UopInspectorApp) -> &mut DefsEditor {
    &mut app.classic_file_views.defs.editor
}

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::ClassicDefs,
    label: "the def and data files",
    is_dirty: |app| app.classic_file_views.defs.editor.is_dirty(),
    save,
    discard: |app| editor(app).revert(),
};

/// Asks where to write each file with changes, one after the other, until none is left or the user cancels.
fn save(app: &mut UopInspectorApp) {
    while let Some(at) = editor(app).files.iter().position(|file| file.opened.is_some()) {
        let kind = editor(app).files[at].kind;
        let filter = if kind.name().ends_with("def") { DEF_FILTER } else { MUL_FILTER };
        let Some(output) = pick_output(kind.name(), filter) else {
            return;
        };
        if let Err(error) = write_copy(&editor(app).files[at], &output) {
            editor(app).error = Some(format!("Save failed: {error:#}"));
            return;
        }
        editor(app).files[at].opened = None;
        editor(app).error = None;
        report_saved(app, &output);
    }
}

/// `Edit` and the rest of the bar, and the selected file read for it.
pub(super) fn bar(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let selected = app.classic_file_views.defs.selected;
    let editor_state = editor(app);
    let dirty = editor_state.is_dirty();
    let action = editor_bar(ui, &mut editor_state.enabled, EDIT_HOVER, dirty, "Save def files...", &mut editor_state.error);
    match action {
        Some(BarAction::Revert) => editor(app).revert(),
        Some(BarAction::Save) => save(app),
        None => {}
    }
    if !editor(app).enabled {
        return;
    }
    let (Some(root), Some(kind)) = (app.settings.cc_path.clone(), selected) else {
        return;
    };
    if let Err(error) = editor(app).open(&root, kind) {
        editor(app).error = Some(error);
        editor(app).enabled = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;
    use crate::ui::test_support::{client_roots, output_path, CLASSIC_CLIENT_ROOTS_ENV};
    use uocf::classic::def_files::TargetsDefFile;
    use uocf::classic::speech::SpeechFile;

    const KIND: FileKind = FileKind::Targets("body.def");
    const SHIPPED_STYLE_LINES: &str = "# a comment\r\n400 {401, 402} 7\r\n  500   {501}   0 # keeps its spacing\r\n";
    const EDITED_ID: &str = "450";
    const EDITED_TEXT: &str = "edited";

    fn small_file() -> DefFile {
        DefFile::Targets(TargetsDefFile::from_bytes(SHIPPED_STYLE_LINES.as_bytes()).unwrap())
    }

    fn with_open_file(app: &mut UopInspectorApp) {
        let data = small_file();
        editor(app).files.push(EditedDef { kind: KIND, source: PathBuf::new(), data, opened: None });
    }

    fn document_bytes(file: &DefFile) -> Vec<u8> {
        let DefFile::Targets(document) = file else { panic!("not a remap file") };
        document.to_bytes()
    }

    #[test]
    fn an_edited_row_changes_only_its_line_and_discarding_takes_it_back() {
        let mut app = test_app();
        with_open_file(&mut app);
        editor(&mut app).select_row(KIND, 0);
        editor(&mut app).values[0] = EDITED_ID.to_owned();
        let values = editor(&mut app).values.clone();
        assert!(editor(&mut app).update(KIND, |file| apply_fields(file, 0, &values)));
        assert!((TAB_EDITOR.is_dirty)(&app));
        let written = String::from_utf8(document_bytes(&editor(&mut app).files[0].data)).unwrap();
        assert_eq!(written, format!("# a comment\r\n{EDITED_ID} {{401, 402}} 7\r\n  500   {{501}}   0 # keeps its spacing\r\n"));

        (TAB_EDITOR.discard)(&mut app);
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        assert_eq!(document_bytes(&editor(&mut app).files[0].data), SHIPPED_STYLE_LINES.as_bytes());
    }

    #[test]
    fn text_that_cannot_be_written_is_refused_and_changes_nothing() {
        let mut app = test_app();
        with_open_file(&mut app);
        let bad_id = vec!["not a number".to_owned(), "1".to_owned(), "0".to_owned(), String::new()];
        assert!(!editor(&mut app).update(KIND, |file| apply_fields(file, 0, &bad_id)));
        assert!(editor(&mut app).error.is_some());
        assert!(!(TAB_EDITOR.is_dirty)(&app));

        let no_targets = vec!["1".to_owned(), String::new(), "0".to_owned(), String::new()];
        assert!(!editor(&mut app).update(KIND, |file| apply_fields(file, 0, &no_targets)), "a remap line has targets");
        assert!(!(TAB_EDITOR.is_dirty)(&app));
    }

    #[test]
    fn a_row_can_be_added_and_removed() {
        let mut file = small_file();
        add_row(&mut file).unwrap();
        assert_eq!(fields_of(&file, 2).unwrap()[0].1, "0");
        remove_row(&mut file, 2).unwrap();
        assert!(fields_of(&file, 2).is_none());
        assert_eq!(document_bytes(&file), SHIPPED_STYLE_LINES.as_bytes());
    }

    #[test]
    fn only_the_last_skill_group_row_can_be_removed() {
        let mut groups = uocf::classic::skills::SkillGroupsFile::from_bytes(&1i32.to_le_bytes()).unwrap();
        groups.skill_groups = vec![0, 0];
        let mut file = DefFile::SkillGroups(groups);
        assert!(remove_row(&mut file, 0).is_err());
        assert!(remove_row(&mut file, 1).is_ok());
    }

    /// Changes a text in a field every shipped record can take.
    fn edit(kind: FileKind, values: &mut [String]) {
        let field = match kind {
            FileKind::Targets(_) => 3,
            FileKind::BodyConv => 2,
            FileKind::EquipConv => 5,
            FileKind::Speech | FileKind::Skills | FileKind::SkillGroups => 1,
        };
        values[field] = EDITED_TEXT.to_owned();
    }

    /// A row whose fields `edit` can change: a named group in `skillgrp.mul`, else the first.
    fn row_to_edit(file: &DefFile) -> usize {
        match file {
            DefFile::SkillGroups(file) => file.skill_groups.iter().position(|&group| group > 0).unwrap(),
            _ => 0,
        }
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn shipped_files_save_the_same_bytes_and_an_edit_comes_back_after_reopening() {
        for root in client_roots(CLASSIC_CLIENT_ROOTS_ENV) {
            let out_dir = output_path("def-files");
            std::fs::create_dir_all(&out_dir).unwrap();
            let mut checked = 0;
            for kind in FileKind::all().filter(|kind| find_file_ignoring_case(&root, kind.name()).is_some()) {
                let source = find_file_ignoring_case(&root, kind.name()).unwrap();
                let mut file = EditedDef { kind, source: source.clone(), data: read_file(&root, kind).unwrap(), opened: None };
                let output = out_dir.join(kind.name());
                write_copy(&file, &output).unwrap();
                assert!(std::fs::read(&output).unwrap() == std::fs::read(&source).unwrap(), "{} {}: differs after a save without changes", root.display(), kind.name());
                if kind == FileKind::Skills {
                    let index = find_file_ignoring_case(&root, SKILLS_INDEX_FILE).unwrap();
                    assert!(std::fs::read(output.with_extension(INDEX_EXTENSION)).unwrap() == std::fs::read(index).unwrap(), "{}: the index differs", root.display());
                }

                checked += 1;
                let row = row_to_edit(&file.data);
                let Some(fields) = fields_of(&file.data, row) else {
                    println!("{} {}: no row to edit", root.display(), kind.name());
                    continue;
                };
                let mut values: Vec<String> = fields.into_iter().map(|(_, value)| value).collect();
                edit(kind, &mut values);
                apply_fields(&mut file.data, row, &values).unwrap();
                check_writes(&file.data).unwrap();
                write_copy(&file, &output).unwrap();
                let reopened = read_file(&out_dir, kind).unwrap();
                let read: Vec<String> = fields_of(&reopened, row).unwrap().into_iter().map(|(_, value)| value).collect();
                assert_eq!(read, values, "{} {}", root.display(), kind.name());
            }
            std::fs::remove_dir_all(&out_dir).unwrap();
            println!("{}: {checked} files save the same bytes; an edit comes back", root.display());
        }
    }

    #[test]
    fn the_loaded_file_is_refused_as_the_target() {
        let source = output_path("refused-speech.mul");
        std::fs::write(&source, []).unwrap();
        let file = EditedDef { kind: FileKind::Speech, source: source.clone(), data: DefFile::Speech(SpeechFile::default()), opened: None };
        assert!(write_copy(&file, &source).is_err());
        std::fs::remove_file(&source).unwrap();
    }
}
