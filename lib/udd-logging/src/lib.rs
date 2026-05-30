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

        let _ = writeln!(io::stderr(), "{}", format_record(record));
    }

    fn flush(&self) {
        let _ = io::stderr().flush();
    }
}

pub fn format_record(record: &Record<'_>) -> String {
    colorize_string(record_markup(record))
}

fn record_markup(record: &Record<'_>) -> String {
    match record.level() {
        Level::Error => format!("<red><cross> {}</>", record.args()),
        Level::Warn => format!("<yellow><warn> {}</>", record.args()),
        Level::Info => format!("<cyan><info> {}</>", record.args()),
        Level::Debug => format!(
            "<blue><bold>debug</> [{}] <blue>{}</>",
            record.target(),
            record.args()
        ),
        Level::Trace => format!(
            "<bright_black><bold>trace</> [{}] <bright_black>{}</>",
            record.target(),
            record.args()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_record_colors_message_text() {
        let record = Record::builder()
            .args(format_args!("uocf: string dictionary reached EOF"))
            .level(Level::Warn)
            .target("uocf::enhanced::string_dictionary")
            .build();

        let formatted = format_record(&record);

        assert!(formatted.contains("\x1B[33m"));
        assert!(formatted.contains("uocf: string dictionary reached EOF"));
    }
}
