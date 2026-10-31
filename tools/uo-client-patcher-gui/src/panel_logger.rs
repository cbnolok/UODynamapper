//! The log panel: the app's messages and the `log` records of uocf, mirrored to stderr.

use std::sync::{Arc, Mutex, Once};

use log::{Level, LevelFilter, Log, Metadata, Record};

const PANEL_LOG_LEVEL: LevelFilter = LevelFilter::Info;
/// The log targets shown: the library and this app.
const PANEL_TARGETS: [&str; 2] = ["uocf", "uo_client_patcher_gui"];
/// The panel keeps at most this many messages.
const MAX_LOG_MESSAGES: usize = 2000;

static INSTALL_LOGGER: Once = Once::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct LogMessage {
    pub text: String,
    pub level: LogLevel,
}

/// The messages of the log panel, shared with the logger.
#[derive(Debug, Clone, Default)]
pub struct Logs(Arc<Mutex<Vec<LogMessage>>>);

impl Logs {
    /// Adds a message to the panel and prints it to stderr.
    pub fn push(&self, level: LogLevel, text: impl Into<String>) {
        let text = text.into();
        let log_level = match level {
            LogLevel::Info | LogLevel::Success => Level::Info,
            LogLevel::Warning => Level::Warn,
            LogLevel::Error => Level::Error,
        };
        eprintln!("{}", udd_logging::format_message(log_level, &text));
        self.append(LogMessage { text, level });
    }

    fn append(&self, message: LogMessage) {
        if let Ok(mut logs) = self.0.lock() {
            logs.push(message);
            let excess = logs.len().saturating_sub(MAX_LOG_MESSAGES);
            logs.drain(..excess);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut logs) = self.0.lock() {
            logs.clear();
        }
    }

    pub fn snapshot(&self) -> Vec<LogMessage> {
        self.0.lock().map(|logs| logs.clone()).unwrap_or_default()
    }
}

/// Routes the `log` records of [`PANEL_TARGETS`] to `logs` (once per process).
pub fn install_panel_logger(logs: Logs) {
    INSTALL_LOGGER.call_once(|| match log::set_boxed_logger(Box::new(PanelLogger { logs })) {
        Ok(()) => log::set_max_level(PANEL_LOG_LEVEL),
        Err(error) => eprintln!("Could not install the log panel logger: {error}"),
    });
}

struct PanelLogger {
    logs: Logs,
}

impl Log for PanelLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= PANEL_LOG_LEVEL && PANEL_TARGETS.iter().any(|target| metadata.target().starts_with(target))
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        eprintln!("{}", udd_logging::format_record(record));
        let level = match record.level() {
            Level::Error => LogLevel::Error,
            Level::Warn => LogLevel::Warning,
            Level::Info | Level::Debug | Level::Trace => LogLevel::Info,
        };
        self.logs.append(LogMessage { text: format!("[{}] {}", record.level(), record.args()), level });
    }

    fn flush(&self) {}
}
