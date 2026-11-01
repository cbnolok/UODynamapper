//! Read-only view of the Classic def and reference data files, read strictly as the client reads them
//! (`uocf::classic::def_files`, `speech`, `skills`): the remap def files, `bodyconv.def`, `Equipconv.def`,
//! `speech.mul`, `skills.mul` and `skillgrp.mul`, as tables, with the line and column of a syntax error.

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::HashSet;
use std::path::Path;
use uocf::classic::def_files::{BodyConvDefFile, DefLine, EquipConvDefFile, TargetsDefFile};
use uocf::classic::skills::{SKILL_GROUPS_FILE, SKILLS_DATA_FILE, SkillGroupsFile, SkillsFile};
use uocf::classic::speech::{SPEECH_FILE, SpeechFile};
use uocf::utils::path::find_file_ignoring_case;

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
enum FileKind {
    Targets(&'static str),
    BodyConv,
    EquipConv,
    Speech,
    Skills,
    SkillGroups,
}

impl FileKind {
    fn name(self) -> &'static str {
        match self {
            Self::Targets(name) => name,
            Self::BodyConv => BODYCONV_FILE,
            Self::EquipConv => EQUIPCONV_FILE,
            Self::Speech => SPEECH_FILE,
            Self::Skills => SKILLS_DATA_FILE,
            Self::SkillGroups => SKILL_GROUPS_FILE,
        }
    }

    fn all() -> impl Iterator<Item = Self> {
        TARGETS_FILES.into_iter().map(Self::Targets).chain([Self::BodyConv, Self::EquipConv, Self::Speech, Self::Skills, Self::SkillGroups])
    }
}

struct Table {
    headers: Vec<&'static str>,
    rows: Vec<Vec<String>>,
    summary: String,
}

#[derive(Default)]
pub struct ClassicDefsState {
    selected: Option<FileKind>,
    table: Option<Result<Table, String>>,
    filter: String,
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

fn read_table(dir: &Path, kind: FileKind) -> Result<Table, String> {
    let path = find_file_ignoring_case(dir, kind.name()).ok_or_else(|| format!("no {} in {}", kind.name(), dir.display()))?;
    let error = |error: color_eyre::eyre::Report| format!("{error:#}");
    match kind {
        FileKind::Targets(_) => {
            let file = TargetsDefFile::load(&path).map_err(error)?;
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
        FileKind::BodyConv => {
            let file = BodyConvDefFile::load(&path).map_err(error)?;
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
        FileKind::EquipConv => {
            let file = EquipConvDefFile::load(&path).map_err(error)?;
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
        FileKind::Speech => {
            let file = SpeechFile::load(&path).map_err(error)?;
            let ids: HashSet<u16> = file.entries.iter().map(|entry| entry.id).collect();
            let rows = file.entries.iter().enumerate().map(|(index, entry)| vec![index.to_string(), entry.id.to_string(), entry.text.clone()]).collect();
            Ok(Table { headers: vec!["Record", "Keyword id", "Text"], rows, summary: format!("{} records of {} keywords", file.entries.len(), ids.len()) })
        }
        FileKind::Skills => {
            let file = SkillsFile::load(dir).map_err(error)?;
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
        FileKind::SkillGroups => {
            let file = SkillGroupsFile::load(&path).map_err(error)?;
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
    let cc_path = app.settings.cc_path.clone();
    let state = &mut app.classic_file_views.defs;
    egui::Panel::left("classic_defs_files").resizable(true).default_size(TOOLS_PANEL_WIDTH).show(ui, |ui| {
        ui.heading("Def and data files");
        ui.separator();
        let Some(cc_path) = cc_path else {
            ui.label("Set the CC path (File > Search Paths).");
            return;
        };
        for kind in FileKind::all() {
            let present = find_file_ignoring_case(&cc_path, kind.name()).is_some();
            let response = ui.add_enabled(present, egui::Button::selectable(state.selected == Some(kind), kind.name()));
            if response.clicked() {
                state.selected = Some(kind);
                state.table = Some(read_table(&cc_path, kind));
            }
        }
    });

    egui::CentralPanel::default().show(ui, |ui| {
        let (Some(kind), Some(table)) = (state.selected, &state.table) else {
            ui.centered_and_justified(|ui| ui.label("Select a file."));
            return;
        };
        ui.heading(kind.name());
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
        let rows: Vec<&Vec<String>> =
            table.rows.iter().filter(|row| filter.is_empty() || row.iter().any(|cell| cell.to_lowercase().contains(&filter))).collect();
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
                        let cells = rows[row.index()];
                        for cell in cells {
                            row.col(|ui| {
                                ui.label(cell);
                            });
                        }
                    });
                });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";

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
