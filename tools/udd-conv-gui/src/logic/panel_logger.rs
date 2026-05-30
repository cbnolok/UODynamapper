use std::sync::{Arc, Mutex, Once};

use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::models::{LogLevel, LogMessage};

const PANEL_LOG_LEVEL: LevelFilter = LevelFilter::Info;

static INSTALL_LOGGER: Once = Once::new();

pub fn install_panel_logger(logs: Arc<Mutex<Vec<LogMessage>>>) {
    INSTALL_LOGGER.call_once(|| {
        let logger = PanelLogger { logs };
        match log::set_boxed_logger(Box::new(logger)) {
            Ok(()) => log::set_max_level(PANEL_LOG_LEVEL),
            Err(error) => eprintln!("Could not install GUI log panel logger: {error}"),
        }
    });
}

struct PanelLogger {
    logs: Arc<Mutex<Vec<LogMessage>>>,
}

impl Log for PanelLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= PANEL_LOG_LEVEL && is_project_target(metadata.target())
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let text = format!("[{}] {}", record.level(), record.args());
        eprintln!("{}", udd_logging::format_record(record));

        let level = match record.level() {
            Level::Error => LogLevel::Error,
            Level::Warn => LogLevel::Warning,
            Level::Info | Level::Debug | Level::Trace => LogLevel::Info,
        };

        if let Ok(mut logs) = self.logs.lock() {
            logs.push(LogMessage { text, level });
        }
    }

    fn flush(&self) {}
}

fn is_project_target(target: &str) -> bool {
    target.starts_with("udd_") || target.starts_with("uocf")
}

#[cfg(unix)]
pub fn capture_stdout_to_panel(logs: Arc<Mutex<Vec<LogMessage>>>) -> Option<StdoutPanelCapture> {
    match StdoutPanelCapture::start(logs.clone()) {
        Ok(capture) => Some(capture),
        Err(error) => {
            push_panel_log(
                &logs,
                format!("Could not mirror stdout to the log panel: {error}"),
                LogLevel::Warning,
            );
            None
        }
    }
}

#[cfg(not(unix))]
pub fn capture_stdout_to_panel(_logs: Arc<Mutex<Vec<LogMessage>>>) -> Option<StdoutPanelCapture> {
    None
}

#[cfg(unix)]
pub struct StdoutPanelCapture {
    saved_stdout_fd: libc::c_int,
    reader: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(unix))]
pub struct StdoutPanelCapture;

#[cfg(unix)]
impl StdoutPanelCapture {
    fn start(logs: Arc<Mutex<Vec<LogMessage>>>) -> std::io::Result<Self> {
        use std::fs::File;
        use std::io::{BufRead, BufReader, Write};
        use std::os::fd::FromRawFd;

        std::io::stdout().flush()?;

        let mut pipe_fds = [-1; 2];
        // SAFETY: pipe_fds points to two valid c_int slots owned by this stack frame.
        if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }

        // SAFETY: STDOUT_FILENO is a valid process fd while the GUI is running.
        let saved_stdout_fd = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if saved_stdout_fd < 0 {
            close_fd(pipe_fds[0]);
            close_fd(pipe_fds[1]);
            return Err(std::io::Error::last_os_error());
        }

        // SAFETY: duplicate the original stdout for the reader thread to mirror lines back.
        let mirror_stdout_fd = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if mirror_stdout_fd < 0 {
            close_fd(saved_stdout_fd);
            close_fd(pipe_fds[0]);
            close_fd(pipe_fds[1]);
            return Err(std::io::Error::last_os_error());
        }

        // SAFETY: dup2 atomically points stdout at the pipe write end.
        if unsafe { libc::dup2(pipe_fds[1], libc::STDOUT_FILENO) } < 0 {
            let error = std::io::Error::last_os_error();
            close_fd(mirror_stdout_fd);
            close_fd(saved_stdout_fd);
            close_fd(pipe_fds[0]);
            close_fd(pipe_fds[1]);
            return Err(error);
        }
        close_fd(pipe_fds[1]);

        let reader = std::thread::spawn(move || {
            // SAFETY: ownership of these fds is transferred exactly once into File.
            let pipe_reader = unsafe { File::from_raw_fd(pipe_fds[0]) };
            let mut mirror_stdout = unsafe { File::from_raw_fd(mirror_stdout_fd) };
            for line in BufReader::new(pipe_reader).lines() {
                let Ok(line) = line else {
                    break;
                };
                let _ = writeln!(mirror_stdout, "{line}");
                let _ = mirror_stdout.flush();
                push_panel_log(&logs, line, LogLevel::Info);
            }
        });

        Ok(Self {
            saved_stdout_fd,
            reader: Some(reader),
        })
    }
}

#[cfg(unix)]
impl Drop for StdoutPanelCapture {
    fn drop(&mut self) {
        use std::io::Write;

        let _ = std::io::stdout().flush();
        // SAFETY: saved_stdout_fd was created by dup and remains owned by this guard.
        let _ = unsafe { libc::dup2(self.saved_stdout_fd, libc::STDOUT_FILENO) };
        close_fd(self.saved_stdout_fd);

        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(unix)]
fn close_fd(fd: libc::c_int) {
    if fd >= 0 {
        // SAFETY: fd is either an owned descriptor or already invalid; close errors are ignored.
        let _ = unsafe { libc::close(fd) };
    }
}

fn push_panel_log(logs: &Arc<Mutex<Vec<LogMessage>>>, text: impl Into<String>, level: LogLevel) {
    if let Ok(mut logs) = logs.lock() {
        logs.push(LogMessage {
            text: text.into(),
            level,
        });
    }
}
