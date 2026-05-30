use std::io::{self, Write};

use log::{Level, Log, Metadata, Record, SetLoggerError};
use paris::formatter::colorize_string;

pub fn install_paris_logger() -> Result<(), SetLoggerError> {
    let mut filter_builder = env_filter::Builder::from_env("RUST_LOG");
    let filter = filter_builder.build();
    let max_level = filter.filter();
    let logger = env_filter::FilteredLog::new(ParisLog, filter);

    log::set_boxed_logger(Box::new(logger))?;
    log::set_max_level(max_level);
    Ok(())
}

struct ParisLog;

impl Log for ParisLog {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let message = match record.level() {
            Level::Error => format!("<red><cross></> {}", record.args()),
            Level::Warn => format!("<yellow><warn></> {}", record.args()),
            Level::Info => format!("<cyan><info></> {}", record.args()),
            Level::Debug => format!(
                "<blue><bold>debug</> [{}] {}",
                record.target(),
                record.args()
            ),
            Level::Trace => format!(
                "<bright_black><bold>trace</> [{}] {}",
                record.target(),
                record.args()
            ),
        };

        let _ = writeln!(io::stderr(), "{}", colorize_string(message));
    }

    fn flush(&self) {
        let _ = io::stderr().flush();
    }
}
