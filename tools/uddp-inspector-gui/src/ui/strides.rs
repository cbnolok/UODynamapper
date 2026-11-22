//! The measured strides of a mobile animation package (`udd_assets::animation::mobile_strides`): the stride of the
//! selected clip in the detail area, and a window with every clip's stride and state (unchanged, changed, new,
//! removed), measuring the changed clips or all of them again by the planted foot
//! (`udd_conv::animation::mobile_strides`) on a thread of its own, into the package or another file.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use udd_assets::animation::mobile_strides::{MobileAnimClips, MobileStrideRecord, StridePlan, STRIDE_FACINGS};
use udd_conv::animation::mobile_strides::{
    clip_stride_states, measure_package_strides, stride_status, ClipStrideState, MeasureOptions, PackageStrides,
    StrideJob,
};

use crate::app::InspectorApp;
use crate::ui::{metadata_grid, metadata_label};

const WINDOW_DEFAULT_SIZE: [f32; 2] = [620.0, 560.0];
const TABLE_ROW_HEIGHT: f32 = 18.0;
const TABLE_COLUMN_WIDTH: f32 = 72.0;
const PROGRESS_BAR_WIDTH: f32 = 360.0;
/// Facing labels of [`STRIDE_FACINGS`], the server directions 4, 5 and 6.
const FACING_LABELS: [&str; STRIDE_FACINGS.len()] = ["South", "South-west", "West"];

type ClipStates = BTreeMap<(u32, u16), (ClipStrideState, Option<MobileStrideRecord>)>;

/// The strides of the open mobile animation package.
pub struct StridesPanel {
    path: PathBuf,
    package: Arc<dyn MobileAnimClips + Send + Sync>,
    states: Option<ClipStates>,
    status: Option<StrideJob<StridePlan>>,
    measurement: Option<StrideJob<PackageStrides>>,
    pub window_open: bool,
    /// Which states the window lists.
    shown: [bool; ClipStrideState::ALL.len()],
    /// Where a measurement writes: `None` into the package.
    output: Option<PathBuf>,
    /// What the last measurement did, or why it failed.
    message: Option<String>,
}

impl StridesPanel {
    pub fn new(path: PathBuf, package: Arc<dyn MobileAnimClips + Send + Sync>) -> Self {
        Self {
            path, package, states: None, status: None, measurement: None, window_open: false,
            shown: [false, true, true, true], output: None, message: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.measurement.is_some()
    }

    /// Hashes the clips on a thread of their own the first time the states are asked for.
    fn ensure_status(&mut self, ctx: &egui::Context) {
        if self.states.is_some() || self.status.is_some() {
            return;
        }
        let package = self.package.clone();
        let repaint = ctx.clone();
        self.status = Some(StrideJob::spawn(move |_, progress| stride_status(&*package, None, progress).map(Some),
            move || repaint.request_repaint()));
    }

    /// Takes the results of the work that ended. Gives the package to reopen after a measurement wrote it.
    fn poll(&mut self) -> Option<PathBuf> {
        if let Some(job) = &mut self.status && let Some(result) = job.finished() {
            self.status = None;
            match result {
                Ok(Some(plan)) => self.states = Some(clip_stride_states(&plan, self.package.stride_records())),
                Ok(None) => {}
                Err(error) => self.message = Some(format!("The clips did not hash: {error:#}")),
            }
        }
        let job = self.measurement.as_mut()?;
        let result = job.finished()?;
        self.measurement = None;
        match result {
            Ok(Some(done)) => {
                let target = self.output.clone().unwrap_or_else(|| self.path.clone());
                let written = match done.write_secs {
                    Some(secs) => format!("wrote {} in {secs:.1} s", target.display()),
                    None => "the strides are unchanged; nothing written".to_string(),
                };
                self.message = Some(format!("{}\n{}\n{written}", done.strides.summary(),
                    done.strides.timing(done.load_secs)));
                log::info!("{}", done.strides.summary());
                (done.write_secs.is_some() && target == self.path).then_some(target)
            }
            Ok(None) => {
                self.message = Some("Measurement cancelled; nothing written.".to_string());
                None
            }
            Err(error) => {
                self.message = Some(format!("The measurement failed: {error:#}"));
                log::error!("The stride measurement failed: {error:#}");
                None
            }
        }
    }

    fn start_measurement(&mut self, ctx: &egui::Context, everything: bool) {
        let path = self.path.clone();
        let output = self.output.clone().unwrap_or_else(|| self.path.clone());
        let options = MeasureOptions { everything, bodies: None, ..Default::default() };
        let repaint = ctx.clone();
        self.message = None;
        self.measurement = Some(StrideJob::spawn(move |cancel, progress| {
            measure_package_strides(&path, &output, &options, None, &mut None, cancel, progress)
        }, move || repaint.request_repaint()));
    }
}

/// The stride of the selected clip, in the detail area.
pub fn ui_selected_clip_strides(app: &mut InspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, body: u32,
    action: u16) {
    let Some(panel) = app.strides_panel.as_mut() else { return };
    panel.ensure_status(ctx);
    ui.horizontal(|ui| {
        ui.strong("Stride");
        if ui.button("Strides…").on_hover_text("Every clip's stride, and measuring them").clicked() {
            panel.window_open = true;
        }
    });
    let stored = panel.package.stride_record(body, action);
    let state = panel.states.as_ref().map(|states| states.get(&(body, action)).map(|(state, _)| *state));
    metadata_grid("mobile_anim_selected_stride").show(ui, |ui| {
        metadata_label(ui, "State");
        ui.label(match state {
            None => "hashing the clips…",
            Some(None) => "not measured (no walk or run, or no frames)",
            Some(Some(state)) => state.label(),
        });
        ui.end_row();
        metadata_label(ui, "Tiles per cycle");
        ui.label(stored.map_or("-".to_string(), |record| tiles(record.stride_tiles)));
        ui.end_row();
        for (index, label) in FACING_LABELS.iter().enumerate() {
            metadata_label(ui, *label);
            ui.label(stored.map_or("-".to_string(), |record| tiles(record.facing_tiles[index])));
            ui.end_row();
        }
    });
}

/// The strides window, inside the main viewport.
pub fn ui_strides_window(app: &mut InspectorApp, ctx: &egui::Context) {
    let reopen = app.strides_panel.as_mut().and_then(StridesPanel::poll);
    if let Some(path) = reopen {
        let selected = app.selected_mobile_anim_index;
        let message = app.strides_panel.as_mut().and_then(|panel| panel.message.take());
        app.open_package(path);
        app.selected_mobile_anim_index = selected;
        if let Some(panel) = app.strides_panel.as_mut() {
            panel.window_open = true;
            panel.message = message;
        }
    }
    let Some(panel) = app.strides_panel.as_mut() else { return };
    if !panel.window_open {
        return;
    }
    panel.ensure_status(ctx);
    let mut open = panel.window_open;
    egui::Window::new("Mobile strides")
        .open(&mut open)
        .default_size(WINDOW_DEFAULT_SIZE)
        .resizable(true)
        .show(ctx, |ui| ui_window_contents(panel, ctx, ui));
    panel.window_open = open;
}

fn ui_window_contents(panel: &mut StridesPanel, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.label(format!("{} ({})", panel.path.display(), panel.package.stride_source().label()));
    match &panel.states {
        Some(states) => {
            let mut counts = [0usize; ClipStrideState::ALL.len()];
            for (state, _) in states.values() {
                counts[*state as usize] += 1;
            }
            ui.horizontal(|ui| {
                for (index, state) in ClipStrideState::ALL.into_iter().enumerate() {
                    ui.checkbox(&mut panel.shown[index], format!("{} {}", counts[index], state.label()));
                }
            });
        }
        None => {
            if let Some(job) = &panel.status {
                let progress = job.progress();
                ui.add(egui::ProgressBar::new(progress.fraction).desired_width(PROGRESS_BAR_WIDTH)
                    .text(progress.activity));
            }
        }
    }
    ui.separator();

    ui.add_enabled_ui(!panel.busy(), |ui| {
        ui.horizontal(|ui| {
            ui.label("Write to:");
            if ui.radio(panel.output.is_none(), "the package").clicked() {
                panel.output = None;
            }
            let other = panel.output.as_ref().map_or("another file…".to_string(), |path| path.display().to_string());
            if ui.radio(panel.output.is_some(), other).clicked() {
                let name = panel.path.file_name().map_or(String::new(), |name| name.to_string_lossy().into_owned());
                if let Some(path) = uot_gui::save_file_dialog(&name) {
                    panel.output = (path != panel.path).then_some(path);
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Measure changed").on_hover_text("Measure the new and changed clips, keep the others' \
                strides and drop those of clips gone").clicked() {
                panel.start_measurement(ctx, false);
            }
            if ui.button("Measure all").on_hover_text("Measure every clip again (tens of minutes)").clicked() {
                panel.start_measurement(ctx, true);
            }
        });
    });
    if let Some(job) = &panel.measurement {
        let progress = job.progress();
        ui.horizontal(|ui| {
            let activity =
                if progress.activity.is_empty() { "loading the package".to_string() } else { progress.activity };
            ui.add(egui::ProgressBar::new(progress.fraction).desired_width(PROGRESS_BAR_WIDTH).show_percentage()
                .text(activity));
            let label = if job.cancelling() { "Cancelling…" } else { "Cancel" };
            if ui.add_enabled(!job.cancelling(), egui::Button::new(label)).clicked() {
                job.cancel();
            }
        });
    }
    if let Some(message) = &panel.message {
        ui.label(message);
    }
    ui.separator();

    let Some(states) = &panel.states else { return };
    let rows: Vec<(&(u32, u16), &(ClipStrideState, Option<MobileStrideRecord>))> = states.iter()
        .filter(|(_, (state, _))| panel.shown[*state as usize]).collect();
    egui::ScrollArea::both().auto_shrink([false, false]).show_rows(ui, TABLE_ROW_HEIGHT, rows.len() + 1,
        |ui, range| {
            egui::Grid::new("mobile_strides_table").striped(true).min_col_width(TABLE_COLUMN_WIDTH)
                .start_row(range.start).show(ui, |ui| {
                for row in range {
                    if row == 0 {
                        for heading in ["Body", "Action", "State", "Stride"].into_iter().chain(FACING_LABELS) {
                            ui.strong(heading);
                        }
                        ui.end_row();
                        continue;
                    }
                    let ((body, action), (state, record)) = rows[row - 1];
                    ui.label(body.to_string());
                    ui.label(action.to_string());
                    ui.label(state.label());
                    ui.label(record.map_or("-".to_string(), |record| tiles(record.stride_tiles)));
                    for index in 0..STRIDE_FACINGS.len() {
                        ui.label(record.map_or("-".to_string(), |record| tiles(record.facing_tiles[index])));
                    }
                    ui.end_row();
                }
            });
        });
}

/// A stride in tiles per cycle; "none" where it was measured without a usable value.
fn tiles(value: Option<f32>) -> String {
    value.map_or("none".to_string(), |value| format!("{value:.2}"))
}
