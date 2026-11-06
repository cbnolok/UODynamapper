//! Whether the Classic client applies the mapdif / stadif patches, shown next to the dif options of the world tab
//! (information only: the options stay the user's choice). The check reads the client executable
//! ([`inspect_classic_difs`]) on a worker thread and caches the result per path and modification time.

use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use uocf::client::classic_difs::{inspect_classic_difs, ClassicDifUsage, DifKind, DifStatus};
use uocf::client::pe::PeImage;

/// The executable looked for in the Classic client folder (any case).
pub const CLASSIC_CLIENT_EXE_NAME: &str = "client.exe";
const EXE_EXTENSION: &str = "exe";

/// What a check reads: the Classic client folder and the executable the user picked instead of the automatic one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DifCheckRequest {
    pub cc_dir: Option<PathBuf>,
    pub chosen_exe: Option<PathBuf>,
}

/// The automatic pick among the executables of a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoClientExe {
    Found(PathBuf),
    Missing,
    /// More than one file named [`CLASSIC_CLIENT_EXE_NAME`] in different cases.
    Several(Vec<PathBuf>),
}

/// The executable a check read, or why there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedExe {
    NoFolder,
    FolderUnreadable(String),
    Chosen(PathBuf),
    Auto(AutoClientExe),
}

impl CheckedExe {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Chosen(path) | Self::Auto(AutoClientExe::Found(path)) => Some(path),
            _ => None,
        }
    }
}

/// The dif usage of one executable, or why it could not be read.
pub type DifCheckResult = Arc<Result<ClassicDifUsage, String>>;

#[derive(Debug, Clone)]
pub struct DifCheckReport {
    /// The executables of the Classic client folder, for the user to pick from.
    pub folder_exes: Vec<PathBuf>,
    pub exe: CheckedExe,
    /// `None` when there is no executable to check.
    pub result: Option<DifCheckResult>,
}

/// Results by executable path and modification time, shared with the worker threads.
#[derive(Debug, Clone, Default)]
pub struct DifCheckCache(Arc<Mutex<HashMap<(PathBuf, SystemTime), DifCheckResult>>>);

/// The files of `dir` with the `.exe` extension (any case), sorted.
pub fn find_folder_exes(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut exes = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let is_exe = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(EXE_EXTENSION));
        if is_exe && path.is_file() {
            exes.push(path);
        }
    }
    exes.sort();
    Ok(exes)
}

/// The executable named [`CLASSIC_CLIENT_EXE_NAME`] (any case) among `exes`.
pub fn auto_client_exe(exes: &[PathBuf]) -> AutoClientExe {
    let mut matches: Vec<PathBuf> = exes
        .iter()
        .filter(|path| {
            path.file_name().is_some_and(|name| name.eq_ignore_ascii_case(CLASSIC_CLIENT_EXE_NAME))
        })
        .cloned()
        .collect();
    match matches.len() {
        0 => AutoClientExe::Missing,
        1 => AutoClientExe::Found(matches.remove(0)),
        _ => AutoClientExe::Several(matches),
    }
}

/// Lists the folder, picks the executable (the chosen one, else the automatic one) and checks it, reusing the
/// cached result while its modification time is unchanged.
pub fn run_dif_check(request: &DifCheckRequest, cache: &DifCheckCache) -> DifCheckReport {
    let listing = request.cc_dir.as_deref().map(find_folder_exes);
    let folder_exes = match &listing {
        Some(Ok(exes)) => exes.clone(),
        _ => Vec::new(),
    };
    let exe = match (&request.chosen_exe, &listing) {
        (Some(chosen), _) => CheckedExe::Chosen(chosen.clone()),
        (None, None) => CheckedExe::NoFolder,
        (None, Some(Err(error))) => CheckedExe::FolderUnreadable(error.to_string()),
        (None, Some(Ok(exes))) => CheckedExe::Auto(auto_client_exe(exes)),
    };
    let result = exe.path().map(|path| check_exe(path, cache));
    DifCheckReport { folder_exes, exe, result }
}

fn check_exe(path: &Path, cache: &DifCheckCache) -> DifCheckResult {
    let modified = match std::fs::metadata(path).and_then(|metadata| metadata.modified()) {
        Ok(modified) => modified,
        Err(error) => return Arc::new(Err(error.to_string())),
    };
    let key = (path.to_path_buf(), modified);
    if let Some(cached) = cache.0.lock().ok().and_then(|cache| cache.get(&key).cloned()) {
        return cached;
    }
    let result = Arc::new(inspect_exe(path));
    if let Ok(mut cache) = cache.0.lock() {
        cache.insert(key, result.clone());
    }
    result
}

fn inspect_exe(path: &Path) -> Result<ClassicDifUsage, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let pe = PeImage::parse(&bytes).map_err(|error| error.to_string())?;
    panic::catch_unwind(AssertUnwindSafe(|| inspect_classic_difs(&bytes, &pe)))
        .map_err(|_| "the dif check panicked".to_owned())
}

/// The name of the patches of `kind`, as the world tab's options call them, plural and singular.
fn dif_names(kind: DifKind) -> (&'static str, &'static str) {
    match kind {
        DifKind::Land => ("map difs", "map dif"),
        DifKind::Statics => ("static difs", "static dif"),
    }
}

/// One neutral line on what the client does with the patches of `kind`.
pub fn status_line(kind: DifKind, status: &DifStatus) -> String {
    let (plural, singular) = dif_names(kind);
    match status {
        DifStatus::Applied => format!("This client applies {plural}"),
        DifStatus::LoadedNotApplied => format!("This client loads but never applies {plural}"),
        DifStatus::Absent => format!("This client has no {singular} loader"),
        DifStatus::Unknown(_) => format!("Whether this client applies {plural}: {}", status.describe()),
    }
}

/// The checker's own wording of the status, for a hover text.
pub fn status_detail(kind: DifKind, status: &DifStatus) -> String {
    format!("{}: {}", kind.label(), status.describe())
}

/// One line on the executable the check read, or why there is none.
pub fn exe_line(exe: &CheckedExe, folder_exes: &[PathBuf]) -> String {
    match exe {
        CheckedExe::NoFolder => "Classic client folder not set: no executable to check.".to_owned(),
        CheckedExe::FolderUnreadable(error) => format!("Cannot list the Classic client folder: {error}"),
        CheckedExe::Chosen(path) => format!("Executable (chosen): {}", path.display()),
        CheckedExe::Auto(AutoClientExe::Found(path)) => format!("Executable: {}", path.display()),
        CheckedExe::Auto(AutoClientExe::Missing) if folder_exes.is_empty() => {
            format!("No {CLASSIC_CLIENT_EXE_NAME} or other executable in the Classic client folder.")
        }
        CheckedExe::Auto(AutoClientExe::Missing) => format!(
            "No {CLASSIC_CLIENT_EXE_NAME} in the Classic client folder; {} other executable(s) to pick from.",
            folder_exes.len()
        ),
        CheckedExe::Auto(AutoClientExe::Several(paths)) => format!(
            "{} files named {CLASSIC_CLIENT_EXE_NAME} in the Classic client folder; pick one.",
            paths.len()
        ),
    }
}

/// The latest report and the worker of the running check.
#[derive(Default)]
pub struct DifCheckState {
    cache: DifCheckCache,
    requested: Option<DifCheckRequest>,
    pending: Option<Receiver<DifCheckReport>>,
    pub report: Option<DifCheckReport>,
}

impl DifCheckState {
    /// Takes a finished report, and starts a check when `request` differs from the last one. `notify` runs on the
    /// worker once its report is ready (to repaint).
    pub fn update(&mut self, request: DifCheckRequest, notify: impl FnOnce() + Send + 'static) {
        self.poll();
        if self.requested.as_ref() != Some(&request) {
            self.report = None;
            self.start(request, notify);
        }
    }

    /// Checks the last request again (a changed executable is read again; an unchanged one comes from the cache).
    pub fn recheck(&mut self, notify: impl FnOnce() + Send + 'static) {
        if let Some(request) = self.requested.clone() {
            self.start(request, notify);
        }
    }

    pub fn is_running(&self) -> bool {
        self.pending.is_some()
    }

    fn poll(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        match pending.try_recv() {
            Ok(report) => {
                self.report = Some(report);
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.pending = None,
        }
    }

    fn start(&mut self, request: DifCheckRequest, notify: impl FnOnce() + Send + 'static) {
        let (sender, receiver) = mpsc::channel();
        let cache = self.cache.clone();
        let worker_request = request.clone();
        std::thread::spawn(move || {
            // A newer request drops the receiver: the report is discarded.
            if sender.send(run_dif_check(&worker_request, &cache)).is_ok() {
                notify();
            }
        });
        self.requested = Some(request);
        self.pending = Some(receiver);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_folder(name: &str, files: &[&str]) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system time is after unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("udd_conv_gui_dif_check_{name}_{nanos}"));
        std::fs::create_dir_all(&dir).expect("create temp folder");
        for file in files {
            std::fs::write(dir.join(file), b"not a PE image").expect("write temp file");
        }
        dir
    }

    fn file_names(paths: &[PathBuf]) -> Vec<String> {
        paths.iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn folder_exes_match_the_extension_in_any_case() {
        let dir = temp_folder("extension", &["CLIENT.EXE", "client_tc.Exe", "notes.txt", "exe"]);
        std::fs::create_dir(dir.join("folder.exe")).expect("create sub folder");

        let exes = find_folder_exes(&dir).expect("list folder");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(file_names(&exes), ["CLIENT.EXE", "client_tc.Exe"]);
        assert_eq!(auto_client_exe(&exes), AutoClientExe::Found(exes[0].clone()));
    }

    #[test]
    fn client_exe_missing_among_other_executables() {
        let dir = temp_folder("missing", &["client_tc.exe", "razor.exe"]);

        let exes = find_folder_exes(&dir).expect("list folder");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(auto_client_exe(&exes), AutoClientExe::Missing);
        let line = exe_line(&CheckedExe::Auto(AutoClientExe::Missing), &exes);
        assert!(line.contains("2 other executable"), "{line}");
        assert!(exe_line(&CheckedExe::Auto(AutoClientExe::Missing), &[]).contains("or other executable"));
    }

    #[test]
    fn client_exe_in_several_cases_is_ambiguous() {
        let dir = temp_folder("several", &["client.exe", "Client.exe", "client_tc.exe"]);

        let exes = find_folder_exes(&dir).expect("list folder");
        let _ = std::fs::remove_dir_all(&dir);

        let AutoClientExe::Several(matches) = auto_client_exe(&exes) else {
            panic!("expected several matches");
        };
        assert_eq!(file_names(&matches), ["Client.exe", "client.exe"]);
    }

    #[test]
    fn check_prefers_the_chosen_exe_and_caches_by_modification_time() {
        let dir = temp_folder("chosen", &["client.exe", "client_tc.exe"]);
        let chosen = dir.join("client_tc.exe");
        let cache = DifCheckCache::default();
        let request = DifCheckRequest { cc_dir: Some(dir.clone()), chosen_exe: Some(chosen.clone()) };

        let first = run_dif_check(&request, &cache);
        let second = run_dif_check(&request, &cache);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(first.exe, CheckedExe::Chosen(chosen));
        assert_eq!(first.folder_exes.len(), 2);
        let (first, second) = (first.result.expect("checked"), second.result.expect("checked"));
        assert!(first.is_err(), "not a PE image");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn check_without_folder_reads_nothing() {
        let report = run_dif_check(&DifCheckRequest { cc_dir: None, chosen_exe: None }, &DifCheckCache::default());

        assert_eq!(report.exe, CheckedExe::NoFolder);
        assert!(report.result.is_none());
        assert!(exe_line(&report.exe, &report.folder_exes).contains("not set"));
    }

    #[test]
    fn status_lines_name_the_patches_neutrally() {
        let cases = [
            (DifKind::Land, DifStatus::Applied, "This client applies map difs"),
            (DifKind::Land, DifStatus::LoadedNotApplied, "This client loads but never applies map difs"),
            (DifKind::Land, DifStatus::Absent, "This client has no map dif loader"),
            (DifKind::Statics, DifStatus::Applied, "This client applies static difs"),
            (DifKind::Statics, DifStatus::LoadedNotApplied, "This client loads but never applies static difs"),
            (DifKind::Statics, DifStatus::Absent, "This client has no static dif loader"),
            (
                DifKind::Statics,
                DifStatus::Unknown("odd shape".to_owned()),
                "Whether this client applies static difs: unknown: odd shape",
            ),
        ];
        for (kind, status, expected) in cases {
            assert_eq!(status_line(kind, &status), expected);
        }
        assert_eq!(status_detail(DifKind::Land, &DifStatus::Absent), "land (mapdif): absent: no dif loader");
    }
}
