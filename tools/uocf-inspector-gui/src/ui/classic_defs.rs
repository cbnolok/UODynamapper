//! View of the Classic def and reference data files, read strictly as the client reads them
//! (`uocf::classic::def_files`, `speech`, `skills`): the remap def files, `bodyconv.def`, `Equipconv.def`,
//! `speech.mul`, `skills.mul` and `skillgrp.mul`, as tables, with the line and column of a syntax error. `Edit` is in
//! [`super::classic_defs_editor`].

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::HashSet;
use std::path::Path;
use uocf::classic::def_files::{BodyConvDefFile, DefLine, EquipConvDefFile, TargetsDefFile};
use uocf::classic::skills::{SKILL_GROUPS_FILE, SKILLS_DATA_FILE, SkillGroupsFile, SkillsFile};
use uocf::classic::speech::{SPEECH_FILE, SpeechFile};
use uocf::utils::path::find_file_ignoring_case;

use super::classic_defs_editor::DefsEditor;
use crate::app::UopInspectorApp;

const TARGETS_FILES: [&str; 9] =
    ["gump.def", "anim1.def", "anim2.def", "anim3.def", "anim4.def", "art.def", "texterr.def", "body.def", "corpse.def"];
const BODYCONV_FILE: &str = "bodyconv.def";
const EQUIPCONV_FILE: &str = "equipconv.def";
const ROW_HEIGHT: f32 = 18.0;
const TOOLS_PANEL_WIDTH: f32 = 200.0;
const USED: &str = "yes";
const SHADOWED: &str = "no (an earlier line wins)";
const OVERRIDDEN: &str = "no (a later line wins)";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FileKind {
    Targets(&'static str),
    BodyConv,
    EquipConv,
    Speech,
    Skills,
    SkillGroups,
}

impl FileKind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Targets(name) => name,
            Self::BodyConv => BODYCONV_FILE,
            Self::EquipConv => EQUIPCONV_FILE,
            Self::Speech => SPEECH_FILE,
            Self::Skills => SKILLS_DATA_FILE,
            Self::SkillGroups => SKILL_GROUPS_FILE,
        }
    }

    /// What the file is for, from the client notes (`ghidra-classic-defs`, `ghidra-classic-frames`), the shipped
    /// files' own comments and `uocf::classic`; it says so where the use in the client was not traced.
    fn purpose(self) -> &'static str {
        match self {
            Self::Targets(name) => match name {
                "gump.def" => {
                    "A remap table with a hue. The shipped file's own comment says it is currently used only for the \
                     paperdoll, and names the columns <orig body> {<new body>} <new hue>. How the client uses it was \
                     not traced further."
                }
                "anim1.def" | "anim2.def" | "anim3.def" | "anim4.def" => {
                    "A remap table the client loads at start (anim1.def to anim4.def), in the same format as the other \
                     remap files; the value is 0 in the shipped anim1.def and anim2.def. What the client remaps with \
                     these tables is not known: it was not traced. The shipped client has only anim1.def and anim2.def."
                }
                "art.def" => {
                    "Supplies art for art ids that have no art record: the client keeps an entry only for an id without \
                     one. An entry names target art ids and a hue."
                }
                "texterr.def" => {
                    "A remap table with a hue that the client loads at start, in the same format as the other remap \
                     files. What the client remaps with it is not known: it was not traced."
                }
                "body.def" => {
                    "Remaps a mobile's body to other bodies and a hue (<orig body> {<new body>} <new hue>), when its \
                     animation is read from the MUL files; UOP animations do not use it. A body below 0x800 that \
                     bodyconv.def maps for an enabled expansion is not remapped."
                }
                "corpse.def" => {
                    "Remaps the body of a corpse to other bodies and a hue (<orig body> {<new body>} <new hue>), when its \
                     animation is read from the MUL files; UOP animations do not use it."
                }
                _ => "A remap table. Its purpose is not known: it was not traced.",
            },
            Self::BodyConv => {
                "Says which anim file holds a body: for a body, its index in anim2.mul, anim3.mul, anim4.mul and anim5.mul \
                 to use instead of anim.mul (-1: none), per the shipped file's comment. Only the MUL animation path uses it."
            }
            Self::EquipConv => {
                "Converts an equipment id to another for a given body type. It can also change the gump (0: equipment \
                 id + 50000, -1: converted id + 50000, else the gump id) and the hue, per the shipped file's comment."
            }
            Self::Speech => {
                "The keywords a server is told the player said: a keyword id with its phrasings, where * is a wildcard. \
                 Not a remap file."
            }
            Self::Skills => "The skill names, and whether each skill has a button (is an action). Its index is Skills.idx.",
            Self::SkillGroups => "The skill groups: their names and the group of each skill.",
        }
    }

    pub(super) fn all() -> impl Iterator<Item = Self> {
        TARGETS_FILES.into_iter().map(Self::Targets).chain([Self::BodyConv, Self::EquipConv, Self::Speech, Self::Skills, Self::SkillGroups])
    }
}

/// A file of the tab, as read.
#[derive(Clone, PartialEq)]
pub(super) enum DefFile {
    Targets(TargetsDefFile),
    BodyConv(BodyConvDefFile),
    EquipConv(EquipConvDefFile),
    Speech(SpeechFile),
    Skills(SkillsFile),
    SkillGroups(SkillGroupsFile),
}

pub(super) struct Table {
    headers: Vec<&'static str>,
    rows: Vec<Vec<String>>,
    summary: String,
}

#[derive(Default)]
pub struct ClassicDefsState {
    pub(super) selected: Option<FileKind>,
    table: Option<Result<Table, String>>,
    /// The generation of the editor `table` was built at.
    table_generation: u64,
    filter: String,
    pub(super) editor: DefsEditor,
}

/// The 1-based line numbers of the records of `lines`, with whether the client uses each.
fn record_lines<R>(lines: &[DefLine<R>]) -> impl Iterator<Item = (usize, &R)> {
    lines.iter().enumerate().filter_map(|(index, line)| match line {
        DefLine::Record { record, .. } => Some((index + 1, record)),
        DefLine::Other(_) => None,
    })
}

fn used_label(used: bool, first_wins: bool) -> String {
    match (used, first_wins) {
        (true, _) => USED,
        (false, true) => SHADOWED,
        (false, false) => OVERRIDDEN,
    }
    .to_owned()
}

fn comment(comment: &Option<String>) -> String {
    comment.as_deref().map(str::trim).unwrap_or_default().to_owned()
}

pub(super) fn read_file(dir: &Path, kind: FileKind) -> Result<DefFile, String> {
    let path = find_file_ignoring_case(dir, kind.name()).ok_or_else(|| format!("no {} in {}", kind.name(), dir.display()))?;
    let error = |error: color_eyre::eyre::Report| format!("{error:#}");
    Ok(match kind {
        FileKind::Targets(_) => DefFile::Targets(TargetsDefFile::load(&path).map_err(error)?),
        FileKind::BodyConv => DefFile::BodyConv(BodyConvDefFile::load(&path).map_err(error)?),
        FileKind::EquipConv => DefFile::EquipConv(EquipConvDefFile::load(&path).map_err(error)?),
        FileKind::Speech => DefFile::Speech(SpeechFile::load(&path).map_err(error)?),
        FileKind::Skills => DefFile::Skills(SkillsFile::load(dir).map_err(error)?),
        FileKind::SkillGroups => DefFile::SkillGroups(SkillGroupsFile::load(&path).map_err(error)?),
    })
}

pub(super) fn read_table(dir: &Path, kind: FileKind) -> Result<Table, String> {
    table_of(&read_file(dir, kind)?)
}

pub(super) fn table_of(file: &DefFile) -> Result<Table, String> {
    let error = |error: color_eyre::eyre::Report| format!("{error:#}");
    match file {
        DefFile::Targets(file) => {
            let mut seen = HashSet::new();
            let rows: Vec<Vec<String>> = record_lines(&file.lines)
                .map(|(line, record)| {
                    let written: Vec<String> = record.targets.iter().map(i32::to_string).collect();
                    let read: Vec<String> = record.client_targets().iter().map(u16::to_string).collect();
                    vec![
                        line.to_string(),
                        record.id.to_string(),
                        written.join(", "),
                        read.join(", "),
                        record.value.to_string(),
                        comment(&record.comment),
                        used_label(seen.insert(record.id), true),
                    ]
                })
                .collect();
            let summary = format!("{} remaps of {} ids; a remap draws one of its targets at random", rows.len(), seen.len());
            Ok(Table { headers: vec!["Line", "Id", "Targets", "Read as", "Value (hue)", "Comment", "Used"], rows, summary })
        }
        DefFile::BodyConv(file) => {
            let records: Vec<_> = record_lines(&file.lines).collect();
            let mut seen = HashSet::new();
            let mut rows: Vec<Vec<String>> = records
                .iter()
                .rev()
                .map(|(line, record)| {
                    let mut row = vec![line.to_string(), record.body.to_string()];
                    let bodies: Vec<String> = record.anim_file_bodies.iter().map(i32::to_string).collect();
                    row.push(bodies.join("  "));
                    row.extend([comment(&record.comment), used_label(seen.insert(record.body), false)]);
                    row
                })
                .collect();
            rows.reverse();
            let summary = format!("{} lines of {} bodies; the 2024 client reads 5 numbers, UO Gold 4", rows.len(), seen.len());
            Ok(Table { headers: vec!["Line", "Body", "anim2, anim3, anim4, anim5, ... bodies (-1: none)", "Comment", "Used"], rows, summary })
        }
        DefFile::EquipConv(file) => {
            let records: Vec<_> = record_lines(&file.lines).collect();
            let mut seen = HashSet::new();
            let mut rows: Vec<Vec<String>> = records
                .iter()
                .rev()
                .map(|(line, record)| {
                    vec![
                        line.to_string(),
                        record.body.to_string(),
                        record.equipment.to_string(),
                        record.converted.to_string(),
                        record.gump.to_string(),
                        format!("0x{:X}", record.hue),
                        comment(&record.comment),
                        used_label(seen.insert((record.body, record.equipment)), false),
                    ]
                })
                .collect();
            rows.reverse();
            let summary = format!("{} conversions; the hue is read as hexadecimal", rows.len());
            Ok(Table { headers: vec!["Line", "Body", "Equipment", "Converted", "Gump", "Hue", "Comment", "Used"], rows, summary })
        }
        DefFile::Speech(file) => {
            let ids: HashSet<u16> = file.entries.iter().map(|entry| entry.id).collect();
            let rows = file.entries.iter().enumerate().map(|(index, entry)| vec![index.to_string(), entry.id.to_string(), entry.text.clone()]).collect();
            Ok(Table { headers: vec!["Record", "Keyword id", "Text"], rows, summary: format!("{} records of {} keywords", file.entries.len(), ids.len()) })
        }
        DefFile::Skills(file) => {
            let mut rows = Vec::new();
            for id in 0..file.len() {
                if let Some(skill) = file.skill(id).map_err(error)? {
                    let extra = file.container.entries[id].extra().unwrap_or_default();
                    rows.push(vec![id.to_string(), if skill.action { USED.to_owned() } else { String::new() }, skill.name, extra.to_string()]);
                }
            }
            let summary = format!("{} skills in {} index records", rows.len(), file.len());
            Ok(Table { headers: vec!["Id", "Button", "Name", "Index extra"], rows, summary })
        }
        DefFile::SkillGroups(file) => {
            let rows = file
                .skill_groups
                .iter()
                .enumerate()
                .map(|(skill, &group)| vec![skill.to_string(), group.to_string(), file.group_name(group as usize).unwrap_or("(group 0)").to_owned()])
                .collect();
            let summary = format!("{} groups, {} skills, {:?} names", file.groups.len() + 1, file.skill_groups.len(), file.encoding);
            Ok(Table { headers: vec!["Skill", "Group", "Group name"], rows, summary })
        }
    }
}

pub fn ui_classic_defs(app: &mut UopInspectorApp, _ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::Panel::top("classic_defs_editor_bar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| super::classic_defs_editor::bar(app, ui));
    });
    let cc_path = app.settings.cc_path.clone();
    let state = &mut app.classic_file_views.defs;
    egui::Panel::left("classic_defs_files").resizable(true).default_size(TOOLS_PANEL_WIDTH).show(ui, |ui| {
        ui.heading("Def and data files");
        ui.separator();
        let Some(cc_path) = &cc_path else {
            ui.label("Set the CC path (File > Search Paths).");
            return;
        };
        for kind in FileKind::all() {
            let present = find_file_ignoring_case(cc_path, kind.name()).is_some();
            let response = ui.add_enabled(present, egui::Button::selectable(state.selected == Some(kind), kind.name()));
            if response.clicked() {
                state.selected = Some(kind);
                state.table = Some(state.editor.table(cc_path, kind));
                state.editor.clear_row();
            }
        }
    });

    if let (true, Some(kind)) = (state.editor.enabled, state.selected) {
        egui::Panel::bottom("classic_defs_record").show(ui, |ui| state.editor.record_panel(ui, kind));
    }
    if let (Some(kind), Some(cc_path)) = (state.selected, &cc_path) {
        if state.table_generation != state.editor.generation() {
            state.table = Some(state.editor.table(cc_path, kind));
            state.table_generation = state.editor.generation();
        }
    }

    egui::CentralPanel::default().show(ui, |ui| {
        let (Some(kind), Some(table)) = (state.selected, &state.table) else {
            ui.centered_and_justified(|ui| ui.label("Select a file."));
            return;
        };
        ui.horizontal(|ui| {
            ui.heading(kind.name());
            crate::ui::help_mark(ui, kind.purpose());
        });
        let table = match table {
            Ok(table) => table,
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
                return;
            }
        };
        ui.label(&table.summary);
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(&mut state.filter);
        });
        let filter = state.filter.to_lowercase();
        let rows: Vec<(usize, &Vec<String>)> = table
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| filter.is_empty() || row.iter().any(|cell| cell.to_lowercase().contains(&filter)))
            .collect();
        let editing = state.editor.enabled;
        let mut clicked_row = None;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            let mut builder = TableBuilder::new(ui).striped(true).resizable(true).cell_layout(egui::Layout::left_to_right(egui::Align::Center));
            for _ in 1..table.headers.len() {
                builder = builder.column(Column::auto());
            }
            builder
                .column(Column::remainder())
                .header(ROW_HEIGHT, |mut header| {
                    for name in &table.headers {
                        header.col(|ui| {
                            ui.strong(*name);
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW_HEIGHT, rows.len(), |mut row| {
                        let (row_index, cells) = rows[row.index()];
                        let selected = state.editor.is_row(row_index);
                        for cell in cells {
                            row.col(|ui| {
                                if !editing {
                                    ui.label(cell);
                                } else if ui.selectable_label(selected, cell).clicked() {
                                    clicked_row = Some(row_index);
                                }
                            });
                        }
                    });
                });
        });
        if let Some(row) = clicked_row {
            state.editor.select_row(kind, row);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";

    #[test]
    fn every_file_says_what_it_is_for() {
        for kind in FileKind::all() {
            assert!(!kind.purpose().is_empty(), "{}", kind.name());
            if let FileKind::Targets(name) = kind {
                assert!(!kind.purpose().starts_with("A remap table. Its purpose"), "{name} has no text of its own");
            }
        }
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
    fn every_shipped_file_reads_into_a_table() {
        let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
        for root in std::env::split_paths(&roots) {
            for kind in FileKind::all().filter(|kind| find_file_ignoring_case(&root, kind.name()).is_some()) {
                let table = read_table(&root, kind).unwrap_or_else(|error| panic!("{} {}: {error}", root.display(), kind.name()));
                assert!(table.rows.iter().all(|row| row.len() == table.headers.len()), "{} {}", root.display(), kind.name());
                println!("{}: {} {}", root.display(), kind.name(), table.summary);
            }
        }
    }
}
