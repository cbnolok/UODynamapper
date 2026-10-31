//! The app state: the opened executable and its edits (a [`Session`]), the actions of the sections (open, data check,
//! write and reopen) and the settings kept between runs.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;
use serde::{Deserialize, Serialize};
use uocf::client_exe::patch::{manifest_path, ExePatch};

use crate::logic::edits::EditState;
use crate::logic::opened::{ManifestState, OpenedExe};
use crate::logic::preview::{DataCheck, PreviewCache};
use crate::panel_logger::{install_panel_logger, LogLevel, Logs};

pub const SETTINGS_KEY: &str = "uo_client_patcher_settings";

/// UI conveniences kept between runs (eframe storage).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub last_exe_dir: Option<PathBuf>,
    pub last_output_dir: Option<PathBuf>,
    pub last_data_dir: Option<PathBuf>,
    pub hide_file_ids: bool,
}

/// The sections of the window, in workflow order. A new kind of edit (the login server) is one more section whose
/// edits [`EditState::build_patch`] collects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Executable,
    Facets,
    Seam,
    DataCheck,
    PreviewWrite,
    LoginServer,
}

impl Section {
    pub const ALL: [Self; 6] =
        [Self::Executable, Self::Facets, Self::Seam, Self::DataCheck, Self::PreviewWrite, Self::LoginServer];

    pub fn title(self) -> &'static str {
        match self {
            Self::Executable => "1. Executable",
            Self::Facets => "2. Facets",
            Self::Seam => "3. Split / seam",
            Self::DataCheck => "4. Data check",
            Self::PreviewWrite => "5. Preview and write",
            Self::LoginServer => "6. Login server — coming next",
        }
    }

    /// Open when the window starts.
    pub fn default_open(self) -> bool {
        self != Self::LoginServer
    }
}

/// The outcome of the last write.
#[derive(Debug, Clone)]
pub enum WriteOutcome {
    Written { copy: PathBuf, manifest: PathBuf },
    Failed(String),
}

/// An opened executable and everything edited and checked for it.
pub struct Session {
    pub opened: OpenedExe,
    pub edits: EditState,
    pub preview: PreviewCache,
    /// The folder the data check reads (the executable's by default).
    pub data_dir: PathBuf,
    pub data_check: Option<DataCheck>,
    pub running_check: Option<Receiver<DataCheck>>,
    pub last_write: Option<WriteOutcome>,
}

impl Session {
    fn new(opened: OpenedExe, data_dir: Option<PathBuf>) -> Self {
        let edits = EditState::with_manifest(&opened.inspection, opened.current_manifest());
        let data_dir = data_dir.unwrap_or_else(|| opened.folder());
        Self {
            opened,
            edits,
            preview: PreviewCache::default(),
            data_dir,
            data_check: None,
            running_check: None,
            last_write: None,
        }
    }

    /// Runs the data check of the edited sizes on a worker thread; it only reads.
    pub fn start_data_check(&mut self, ctx: &egui::Context, logs: &Logs) {
        let dimensions = match self.edits.edited_dimensions() {
            Ok(dimensions) => dimensions,
            Err(error) => {
                logs.push(LogLevel::Error, format!("Data check: {error}"));
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let (inspection, dir, ctx) = (self.opened.inspection.clone(), self.data_dir.clone(), ctx.clone());
        logs.push(LogLevel::Info, format!("Checking the facet data in {} against the edited sizes...", dir.display()));
        std::thread::spawn(move || {
            let _ = sender.send(DataCheck::run(&inspection, &dimensions, &dir));
            ctx.request_repaint();
        });
        self.running_check = Some(receiver);
    }

    /// Takes the result of a finished data check.
    fn poll_data_check(&mut self, logs: &Logs) {
        let Some(receiver) = &self.running_check else {
            return;
        };
        match receiver.try_recv() {
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                logs.push(LogLevel::Error, "Data check: the worker stopped without a result");
                self.running_check = None;
            }
            Ok(check) => {
                match &check.reports {
                    Ok(_) => {
                        let counts = check.counts();
                        let level = if counts.mismatch == 0 { LogLevel::Success } else { LogLevel::Warning };
                        logs.push(
                            level,
                            format!(
                                "Data check: {} match, {} mismatch, {} absent",
                                counts.ok, counts.mismatch, counts.absent
                            ),
                        );
                    }
                    Err(error) => logs.push(LogLevel::Error, format!("Data check failed: {error}")),
                }
                self.data_check = Some(check);
                self.running_check = None;
            }
        }
    }
}

pub struct PatcherApp {
    pub settings: AppSettings,
    pub logs: Logs,
    pub session: Option<Session>,
    pub open_error: Option<String>,
}

impl PatcherApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings: AppSettings = cc.storage.and_then(|storage| eframe::get_value(storage, SETTINGS_KEY)).unwrap_or_default();
        let logs = Logs::default();
        install_panel_logger(logs.clone());
        logs.push(LogLevel::Info, "Open a client executable (UOSA*.exe, UOKR.exe, client.exe) or drop one on the window.");
        Self { settings, logs, session: None, open_error: None }
    }

    /// Opens `path` as the new session; the previous one is dropped. `data_dir` keeps a chosen data folder (after a
    /// write, the copy's folder holds no data).
    pub fn open(&mut self, path: &Path, data_dir: Option<PathBuf>) {
        match OpenedExe::open(path) {
            Ok(opened) => {
                let inspection = &opened.inspection;
                let build = inspection.known.map(|build| format!(", known build {}", build.name)).unwrap_or_default();
                self.logs.push(LogLevel::Info, format!("Opened {}: {}{build}", path.display(), inspection.family.label()));
                match &opened.manifest {
                    ManifestState::Current(manifest) => self.logs.push(
                        LogLevel::Info,
                        format!("Manifest {}: {} steps", manifest_path(path).display(), manifest.steps.len()),
                    ),
                    ManifestState::Stale(_) => {
                        self.logs.push(LogLevel::Warning, "The manifest next to the file records another file: ignored")
                    }
                    ManifestState::Unreadable(error) => {
                        self.logs.push(LogLevel::Error, format!("The manifest cannot be read: {error}"))
                    }
                    ManifestState::None => {}
                }
                for blocker in opened.blockers() {
                    self.logs.push(LogLevel::Warning, format!("Cannot be patched: {blocker}"));
                }
                self.settings.last_exe_dir = Some(opened.folder());
                self.session = Some(Session::new(opened, data_dir));
                self.open_error = None;
            }
            Err(error) => {
                let error = format!("{error:#}");
                self.logs.push(LogLevel::Error, format!("Cannot open {}: {error}", path.display()));
                self.open_error = Some(error);
            }
        }
    }

    pub fn open_dialog(&mut self) {
        let initial = self.settings.last_exe_dir.clone();
        if let Some(path) = gui_shared::open_filtered_file_dialog(initial.as_deref(), "Client executables", &["exe", "EXE"])
        {
            self.open(&path, None);
        }
    }

    /// Asks for the copy's path, writes it with `patch` and reopens it so the next patch chains.
    pub fn save_as(&mut self, patch: &ExePatch) {
        let Some(session) = &self.session else {
            return;
        };
        let mut dialog = gui_shared::file_dialog().set_file_name(session.opened.file_name());
        if let Some(dir) = self.settings.last_output_dir.as_deref().filter(|dir| dir.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(output) = dialog.save_file() else {
            return;
        };
        let data_dir = session.data_dir.clone();
        match session.opened.write_copy(&output, patch) {
            Ok(written) => {
                for warning in &written.warnings {
                    self.logs.push(LogLevel::Warning, warning.text.clone());
                }
                let manifest = manifest_path(&output);
                self.logs.push(
                    LogLevel::Success,
                    format!("Wrote {} ({} byte changes) and {}", output.display(), written.changes.len(), manifest.display()),
                );
                self.settings.last_output_dir = output.parent().map(Path::to_path_buf);
                let previous_check = self.session.as_mut().and_then(|session| session.data_check.take());
                self.open(&output, Some(data_dir));
                if let Some(session) = &mut self.session {
                    session.data_check = previous_check;
                    session.last_write = Some(WriteOutcome::Written { copy: output, manifest });
                }
            }
            Err(error) => {
                let error = format!("{error:#}");
                self.logs.push(LogLevel::Error, format!("Not written: {error}"));
                if let Some(session) = &mut self.session {
                    session.last_write = Some(WriteOutcome::Failed(error));
                }
            }
        }
    }

    /// Per-frame bookkeeping: dropped files and finished data checks.
    pub fn update(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.first().map(|file| file.path().to_path_buf()));
        if let Some(path) = dropped {
            self.open(&path, None);
        }
        if let Some(session) = &mut self.session {
            session.poll_data_check(&self.logs);
        }
    }
}
