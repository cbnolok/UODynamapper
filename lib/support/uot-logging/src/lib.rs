use std::fmt;
use std::io::{self, Write};

use log::{Level, Log, Metadata, Record, SetLoggerError};
use paris::formatter::colorize_string;
use tracing_indicatif::{style::ProgressStyle, IndicatifLayer};
use tracing_log::LogTracer;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt as tracing_fmt};

pub mod progress {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    pub use tracing_indicatif::style::ProgressStyle;
    use tracing_indicatif::span_ext::IndicatifSpanExt;

    const NO_LENGTH: u64 = u64::MAX;

    thread_local! {
        static PROGRESS_STACK: RefCell<Vec<tracing::Span>> = RefCell::new(Vec::new());
    }

    pub struct ProgressBar {
        span: Mutex<Option<tracing::Span>>,
        length: AtomicU64,
    }

    impl ProgressBar {
        pub fn new(length: u64) -> Self {
            let span = progress_span();
            span.pb_set_length(length);
            span.pb_start();
            push_progress_span(span.clone());
            Self {
                span: Mutex::new(Some(span)),
                length: AtomicU64::new(length),
            }
        }

        pub fn set_style(&self, style: ProgressStyle) {
            self.with_span(|span| span.pb_set_style(&style));
        }

        pub fn set_message(&self, message: impl Into<String>) {
            let message = message.into();
            self.with_span(|span| span.pb_set_message(&message));
        }

        pub fn inc(&self, delta: u64) {
            self.with_span(|span| span.pb_inc(delta));
        }

        pub fn finish_with_message(&self, message: impl Into<String>) {
            let message = message.into();
            let span = self.close_span();
            if let Some(span) = span {
                if let Some(length) = self.length() {
                    span.pb_set_position(length);
                }
                span.pb_set_finish_message(&message);
            }
        }

        pub fn finish_and_clear(&self) {
            let _ = self.close_span();
        }

        fn length(&self) -> Option<u64> {
            match self.length.load(Ordering::Relaxed) {
                NO_LENGTH => None,
                length => Some(length),
            }
        }

        fn with_span(&self, f: impl FnOnce(&tracing::Span)) {
            let span = self.span.lock().expect("progress span poisoned");
            if let Some(span) = span.as_ref() {
                f(span);
            }
        }

        fn close_span(&self) -> Option<tracing::Span> {
            let span = self.span.lock().expect("progress span poisoned").take();
            if let Some(span) = span.as_ref() {
                pop_progress_span(span);
            }
            span
        }
    }

    impl Drop for ProgressBar {
        fn drop(&mut self) {
            let span = self.span.get_mut().expect("progress span poisoned").take();
            if let Some(span) = span.as_ref() {
                pop_progress_span(span);
            }
        }
    }

    fn progress_span() -> tracing::Span {
        let parent = PROGRESS_STACK.with(|stack| stack.borrow().last().cloned());
        if let Some(parent) = parent {
            tracing::span!(parent: &parent, tracing::Level::INFO, "progress")
        } else {
            tracing::info_span!("progress")
        }
    }

    fn push_progress_span(span: tracing::Span) {
        PROGRESS_STACK.with(|stack| stack.borrow_mut().push(span));
    }

    fn pop_progress_span(span: &tracing::Span) {
        let span_id = span.id();
        PROGRESS_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.last().is_some_and(|last| last.id() == span_id) {
                stack.pop();
                return;
            }
            if let Some(span_id) = span_id.as_ref() {
                stack.retain(|candidate| candidate.id().as_ref() != Some(span_id));
            }
        });
    }
}

pub fn install_tracing_indicatif_logger(
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let indicatif_layer = IndicatifLayer::new()
        .with_span_child_prefix_indent("  ")
        .with_span_child_prefix_symbol("|- ")
        .with_progress_style(
            ProgressStyle::default_bar()
                .template("{span_child_prefix}{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
    let writer = indicatif_layer.get_stdout_writer();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    LogTracer::init()?;
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_fmt::layer()
                .with_writer(writer)
                .with_target(false)
                .with_level(true),
        )
        .with(indicatif_layer)
        .try_init()?;
    Ok(())
}

pub fn install_paris_logger() -> Result<(), SetLoggerError> {
    let mut filter_builder = env_filter::Builder::new();
    filter_builder.filter_level(log::LevelFilter::Info);
    if let Ok(rust_log) = std::env::var("RUST_LOG") {
        filter_builder.parse(&rust_log);
    }
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

        let _ = writeln!(io::stdout(), "{}", format_record(record));
    }

    fn flush(&self) {
        let _ = io::stdout().flush();
    }
}

pub fn format_record(record: &Record<'_>) -> String {
    colorize_string(record_markup(record))
}

pub fn format_message(level: Level, message: impl fmt::Display) -> String {
    colorize_string(message_markup(level, &message.to_string()))
}

fn message_markup(level: Level, message: &str) -> String {
    match level {
        Level::Error => format!("<red><cross> {message}</>"),
        Level::Warn => format!("<yellow><warn> {message}</>"),
        Level::Info => format!("<cyan><info> {message}</>"),
        Level::Debug => format!("<blue><bold>debug</> <blue>{message}</>"),
        Level::Trace => format!("<bright_black><bold>trace</> <bright_black>{message}</>"),
    }
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

    #[test]
    fn info_message_colors_message_text() {
        let formatted = format_message(Level::Info, "Loading CC assets... Success");

        assert!(formatted.contains("\x1B[36m"));
        assert!(formatted.contains("Loading CC assets... Success"));
    }
}
