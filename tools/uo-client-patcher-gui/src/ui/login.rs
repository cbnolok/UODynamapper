//! Section 5: the login server. Enhanced Client: the built-in public hosts and ports, a host per slot and a port
//! per immediate, edited as one more edit of the patch; Kingdom Reborn: its one built-in IPv4 address and port, the
//! same way; Classic: the login.cfg lists, saved as a copy apart from the patch.

use std::path::PathBuf;

use eframe::egui;
use uocf::client::login_servers::{LoginEntrySource, LoginFileRole};

use super::{error_label, info_grid, muted_label, warning_label, ITEM_SPACING, SUCCESS_COLOR};
use crate::app::Session;
use crate::logic::login::{
    built_in_servers, check_server_row, internal_address, kind_label, login_mode, login_site_rows, login_status,
    servers_text, stock_login, ClassicLogin, LoginEdit, LoginFolder, LoginMode, CFG_KINDS, KR_IPV4_ONLY,
};
use crate::logic::preview::login_warnings;

const HOST_EDIT_WIDTH: f32 = 180.0;
const PORT_EDIT_WIDTH: f32 = 64.0;
const SERVER_ROW_WIDTH: f32 = 240.0;
const SITE_GRID_SPACING: [f32; 2] = [14.0, 2.0];
const OVERWRITE_MODAL_ID: &str = "overwrite_login_cfg";

/// What the section asks the app to do.
pub enum LoginAction {
    /// Choose the path of the login.cfg copy.
    SaveCfgAs,
    /// Overwrite the confirmed existing file.
    Overwrite(PathBuf),
}

pub fn ui_login(ui: &mut egui::Ui, session: &mut Session) -> Option<LoginAction> {
    let family = session.opened.inspection.family;
    match login_mode(&session.opened.inspection) {
        LoginMode::PublicSites => {
            muted_label(
                ui,
                "The Enhanced Client connects to the address built into the executable: a host per public slot and a \
                 port per immediate. The values below are one more edit of the same patch as the facet and seam \
                 edits: the preview lists its bytes, \"Save as\" writes it and the manifest records it.",
            );
            ui_executable_sites(ui, session);
        }
        LoginMode::FixedAddress => {
            muted_label(
                ui,
                "Kingdom Reborn connects to the one address built into its code: an IPv4 number (MOV imm32) and a port \
                 (MOV imm16). The values below are one more edit of the same patch as the facet edits: the preview \
                 lists its bytes, \"Save as\" writes it and the manifest records it.",
            );
            ui_executable_sites(ui, session);
        }
        LoginMode::ConfigFile => return ui_classic(ui, session),
        LoginMode::NotLocated => {
            warning_label(
                ui,
                format!(
                    "The login address code of this {} build was not located: its built-in login server is not shown \
                     and cannot be changed.",
                    family.label()
                ),
            );
        }
    }
    None
}

/// The login sites of the executable (Enhanced Client, Kingdom Reborn) and their editor.
fn ui_executable_sites(ui: &mut egui::Ui, session: &mut Session) {
    let inspection = &session.opened.inspection;
    let Some(sites) = &inspection.login else {
        return;
    };
    let stock = stock_login(inspection, session.opened.current_manifest());
    let what = if sites.uses_fixed_address() { "Built-in address" } else { "Public servers" };
    let mut rows = vec![(what, servers_text(&built_in_servers(sites))), ("State", login_status(sites, stock.as_ref()))];
    if let Some(internal) = internal_address(inspection) {
        rows.push(("Internal test address", format!("{internal}; used only with the client's internal flag, never patched")));
    }
    info_grid(ui, "login_grid", &rows);
    ui.add_space(ITEM_SPACING);
    egui::Grid::new("login_sites_grid").striped(true).num_columns(7).spacing(SITE_GRID_SPACING).show(ui, |ui| {
        for title in ["Site", "Picked by", "File offset", "VA", "Size", "Value", "Stock"] {
            ui.label(egui::RichText::new(title).strong());
        }
        ui.end_row();
        for row in login_site_rows(inspection, stock.as_ref()) {
            ui.label(row.what);
            ui.label(row.drawn_by);
            ui.monospace(row.offset);
            ui.monospace(row.va);
            ui.label(row.size);
            ui.monospace(row.value);
            ui.label(row.stock);
            ui.end_row();
        }
    });
    if sites.hosts.len() > 1 || sites.ports.len() > 1 {
        muted_label(
            ui,
            "Each login draws rand() twice: the first picks the host, the second the port, independently. The same \
             host on every slot and the same port on every immediate make every draw give the one address.",
        );
    }

    ui.add_space(ITEM_SPACING);
    ui_login_fields(ui, &mut session.edits.login);
    let (edit, validation) = session.edits.login.check();
    for error in &validation.errors {
        error_label(ui, error.as_str());
    }
    let preview = session.preview.get(&session.opened, &session.edits);
    match preview.plan() {
        Some(plan) => {
            for warning in login_warnings(&plan.warnings) {
                warning_label(ui, warning.text.as_str());
            }
        }
        None => {
            for warning in &validation.warnings {
                warning_label(ui, warning.as_str());
            }
        }
    }
    match edit {
        Some(edit) => {
            ui.colored_label(SUCCESS_COLOR, format!("In the patch: {}.", edit.describe()));
        }
        None if session.edits.login.is_edited() && validation.is_ok() => {
            muted_label(ui, "The file already has these values: nothing to write.")
        }
        None => {}
    }
}

/// One row per host site (with its slot's capacity) and per port site; an empty field keeps the file's value.
fn ui_login_fields(ui: &mut egui::Ui, login: &mut LoginEdit) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("New login server").strong());
        muted_label(ui, "(an empty field keeps the file's value)");
        if ui.add_enabled(login.is_edited(), egui::Button::new("Clear")).on_hover_text("Keep the built-in values").clicked() {
            login.clear();
        }
    });
    if login.has_several_sites() {
        let mut same = login.same_for_all;
        let toggle = ui
            .checkbox(&mut same, "Same for all")
            .on_hover_text("Typing a host or port fills every host or port site with it");
        if toggle.changed() {
            login.set_same_for_all(same);
        }
    }
    let host_hint = if login.mode == LoginMode::FixedAddress { "dotted IPv4" } else { "IPv4 or host name" };
    let mut spread = None;
    egui::Grid::new("login_edit_grid").num_columns(3).spacing(SITE_GRID_SPACING).show(ui, |ui| {
        for (port, fields) in [(false, &mut login.hosts), (true, &mut login.ports)] {
            for (index, field) in fields.iter_mut().enumerate() {
                ui.label(capitalized(&field.label));
                let (hint, width) = if port { (field.current.as_str(), PORT_EDIT_WIDTH) } else { (host_hint, HOST_EDIT_WIDTH) };
                let response = ui.add(egui::TextEdit::singleline(&mut field.text).hint_text(hint).desired_width(width));
                if response.changed() {
                    spread = Some((port, index));
                }
                let note = match field.max_host_length() {
                    Some(max) => format!("now {}; at most {max} characters ({}-byte slot)", field.current, field.capacity.unwrap_or_default()),
                    None => format!("now {}", field.current),
                };
                muted_label(ui, note);
                ui.end_row();
            }
        }
    });
    if let Some((port, index)) = spread {
        login.spread(port, index);
    }
    if login.mode == LoginMode::FixedAddress {
        muted_label(ui, format!("{KR_IPV4_ONLY}."));
    }
}

fn file_role(role: LoginFileRole) -> &'static str {
    match role {
        LoginFileRole::UoCfg => "uo.cfg",
        LoginFileRole::LoginCfg => "login file",
        LoginFileRole::Executable => "executable",
    }
}

fn ui_classic(ui: &mut egui::Ui, session: &mut Session) -> Option<LoginAction> {
    muted_label(
        ui,
        "The Classic client reads its login server from files, not from the executable: uo.cfg, then the file its \
         LoginServerFile key names, or login.cfg.",
    );
    let mut reload = false;
    ui.horizontal(|ui| {
        ui.label("Client folder:");
        ui.label(egui::RichText::new(session.data_dir.to_string_lossy()).monospace());
        muted_label(ui, "(the Data check's folder)");
        reload = ui.button("Reload").on_hover_text("Rereads the files; the list edits are dropped").clicked();
    });
    if reload {
        session.reload_login_folder();
    }
    let LoginFolder::Classic(login) = &mut session.login_folder else {
        return None;
    };
    match &login.report {
        Err(error) => error_label(ui, format!("Cannot read the login files: {error}")),
        Ok(report) => {
            let rows: Vec<(&str, String)> = report
                .files
                .iter()
                .map(|file| {
                    let state = if file.found { "read by the client" } else { "not found" };
                    (file_role(file.role), format!("{}: {state}", file.path.display()))
                })
                .collect();
            info_grid(ui, "login_files_grid", &rows);
            for note in &report.notes {
                warning_label(ui, note.as_str());
            }
            for entry in &report.entries {
                let kind = match entry.source {
                    LoginEntrySource::Cfg(kind) => kind_label(kind),
                    _ => "other",
                };
                let address = match &entry.address {
                    Ok(address) => address.to_string(),
                    Err(error) => error.clone(),
                };
                let file = entry.file.file_name().unwrap_or_default().to_string_lossy();
                let line = entry.line.map(|line| format!(" line {line}")).unwrap_or_default();
                ui.label(format!("Current {kind}: {address} ({file}{line})"));
            }
        }
    }
    let others = login.other_entries();
    if !others.is_empty() {
        warning_label(
            ui,
            format!(
                "{} LoginServer line(s) outside {} (uo.cfg) are read by the client too; they are not edited here.",
                others.len(),
                login.file_name()
            ),
        );
    }
    ui.add_space(ITEM_SPACING);
    if let Some(error) = login.cfg_error() {
        error_label(ui, format!("{}: {error}", login.cfg_path.display()));
        return None;
    }
    if !login.cfg_found {
        muted_label(ui, format!("{} does not exist: the copy starts empty.", login.cfg_path.display()));
    }
    for kind in CFG_KINDS {
        ui_server_list(ui, login, kind);
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(login.is_edited(), egui::Button::new("Reset to the file")).clicked() {
            login.reset();
        }
    });

    ui.add_space(ITEM_SPACING);
    let edited = login.edited_cfg();
    let mut action = None;
    ui.horizontal(|ui| {
        let save = ui
            .add_enabled(edited.is_ok(), egui::Button::new(format!("💾 Save {} as…", login.file_name())))
            .on_hover_text("Writes a copy; the client's file is never changed")
            .clicked();
        if save {
            action = Some(LoginAction::SaveCfgAs);
        }
    });
    muted_label(
        ui,
        format!(
            "A copy of {} with these lists, keeping its comments and other lines, outside the client folder. This is \
             separate from the executable patch: \"Save as\" in Preview and write does not write it.",
            login.file_name()
        ),
    );
    if let Err(errors) = &edited {
        for error in errors {
            error_label(ui, error.as_str());
        }
    }
    match &login.last_save {
        Some(Ok(path)) => {
            ui.colored_label(SUCCESS_COLOR, format!("Wrote {}.", path.display()));
        }
        Some(Err(error)) => error_label(ui, format!("Not written: {error}")),
        None => {}
    }
    if let Some(path) = login.pending_overwrite.clone() {
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new(OVERWRITE_MODAL_ID)).show(ui.ctx(), |ui| {
            ui.label(format!("{} exists. Overwrite it?", path.display()));
            ui.add_space(ITEM_SPACING);
            ui.horizontal(|ui| {
                if ui.button("Overwrite").clicked() {
                    action = Some(LoginAction::Overwrite(path.clone()));
                }
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || modal.should_close() {
            login.pending_overwrite = None;
        }
    }
    action
}

/// The editor of one list: a `host:port` row per server, remove and add.
fn ui_server_list(ui: &mut egui::Ui, login: &mut ClassicLogin, kind: uocf::client::login_servers::CfgServerKind) {
    ui.label(egui::RichText::new(format!("{} servers ({}=host,port)", capitalized(kind_label(kind)), kind.key())).strong());
    let list = login.list_mut(kind);
    let mut remove = None;
    for (index, row) in list.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(row).hint_text("host:port").desired_width(SERVER_ROW_WIDTH));
            if ui.small_button("✖").on_hover_text("Remove").clicked() {
                remove = Some(index);
            }
            if let Err(error) = check_server_row(row) {
                error_label(ui, error);
            }
        });
    }
    if let Some(index) = remove {
        list.remove(index);
    }
    if list.is_empty() {
        muted_label(ui, "    (none)");
    }
    if ui.small_button(format!("+ Add a {} server", kind_label(kind))).clicked() {
        list.push(String::new());
    }
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}
