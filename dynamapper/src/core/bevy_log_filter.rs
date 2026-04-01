use crate::console_logger::{self, LogAbout, LogSev};
use bevy::prelude::*;
use tracing::{Level, Subscriber};
use tracing_subscriber::{registry::LookupSpan, Layer};

fn strip_meta_prefix<'a>(value: &'a str) -> &'a str {
    let mut out = value;
    while let Some(rest) = out
        .strip_prefix("log::")
        .or_else(|| out.strip_prefix("tracing::"))
    {
        out = rest;
    }
    out
}

fn tracing_location_label(target: &str, module_path: &str) -> String {
    let sanitized_target = strip_meta_prefix(target);
    let sanitized_module = strip_meta_prefix(module_path);

    if !sanitized_module.is_empty() {
        sanitized_module.to_string()
    } else if !sanitized_target.is_empty() {
        sanitized_target.to_string()
    } else {
        "tracing".to_string()
    }
}

fn normalize_output_message(about: LogAbout, msg: &str) -> String {
    let text = if about == LogAbout::UoFiles {
        msg.trim_start()
            .strip_prefix("uocf:")
            .map(str::trim_start)
            .unwrap_or(msg)
    } else {
        msg
    };

    text.split(['\n', '\r'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_backend_message(target: &str, module_path: &str) -> bool {
    let target_lc = target.to_ascii_lowercase();
    let module_lc = module_path.to_ascii_lowercase();

    // Prefer metadata-based matching: tracing target and module path identify backend crates.
    target_lc.starts_with("wgpu")
        || target_lc.starts_with("ash")
        || target_lc.starts_with("naga")
        || module_lc.starts_with("wgpu")
        || module_lc.starts_with("ash")
        || module_lc.starts_with("naga")
}

/// Replaces Bevy's default fmt layer with a compact one that uses HH:MM:SS
/// timestamps instead of the verbose ISO-8601 default.
/// This is wired into LogPlugin::fmt_layer (not custom_layer), which means it
/// fully replaces the default formatter rather than being added on top of it.
/*
fn bevy_logging_fmt_layer(_app: &mut App) -> Option<bevy::log::BoxedFmtLayer> {
    Some(Box::new(
        fmt::layer()
            //.with_span_events(FmtSpan::NONE)
            .with_ansi(true)
            .with_level(true)
            .with_target(true)
            // Compact HH:MM:SS format — avoids the verbose 2026-03-18T09:50:34.068944Z default.
            .with_timer(fmt::time::ChronoLocal::new("%H:%M:%S".into()))
            .compact(),
    ))
}
*/

pub fn custom_bevy_log_config() -> bevy::log::LogPlugin {
    bevy::log::LogPlugin {
        // Suppress benign calloop warnings on Linux (e.g. "Received an event for non-existence source")
        filter: "calloop=error,bevy_framepace=warn".into(),
        // Return a no-op fmt layer that writes to /dev/null.
        // Returning None would make Bevy fall back to its default stderr formatter,
        // causing double logging alongside our InterceptLogLayer.
        fmt_layer: |_| {
            Some(Box::new(
                tracing_subscriber::fmt::Layer::default().with_writer(std::io::sink),
            ))
        },
        // Add the InterceptLogLayer on top (intercepts log events from Bevy or Uocf crates).
        custom_layer: bevy_logging_custom_layer,
        ..Default::default()
    }
}

fn classify_log_target(target: &str, module_path: &str, msg: &str) -> LogAbout {
    if target.starts_with("uocf") || module_path.starts_with("uocf") {
        return LogAbout::UoFiles;
    }
    if target.starts_with("bevy") || module_path.starts_with("bevy") {
        return LogAbout::Bevy;
    }

    let msg_lc = msg.to_ascii_lowercase();
    if msg_lc.starts_with("uocf:") {
        return LogAbout::UoFiles;
    }

    if is_backend_message(target, module_path) {
        return LogAbout::BevyBackends;
    }

    LogAbout::General
}

fn normalize_debug_string(s: String) -> String {
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        s[1..s.len() - 1].to_string()
    } else {
        s
    }
}

pub fn bevy_logging_custom_layer(_app: &mut App) -> Option<bevy::log::BoxedLayer> {
    struct EventFields {
        message: String,
        module_path: String,
    }

    struct MsgVisitor {
        fields: EventFields,
    }

    impl tracing::field::Visit for MsgVisitor {
        fn record_str(&mut self, f: &tracing::field::Field, v: &str) {
            match f.name() {
                "message" => self.fields.message = v.to_string(),
                "module_path" | "log.module_path" => self.fields.module_path = v.to_string(),
                _ => {}
            }
        }

        fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
            let value = normalize_debug_string(format!("{v:?}"));
            match f.name() {
                "message" => self.fields.message = value,
                "module_path" | "log.module_path" => self.fields.module_path = value,
                _ => {}
            }
        }
    }

    struct InterceptLogLayer;

    impl<S> Layer<S> for InterceptLogLayer
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let target = event.metadata().target();
            let sev = match *event.metadata().level() {
                Level::ERROR => LogSev::Error,
                Level::WARN => LogSev::Warn,
                Level::DEBUG => LogSev::Debug,
                Level::TRACE => LogSev::DebugVerbose,
                _ => LogSev::Info,
            };

            let mut vis = MsgVisitor {
                fields: EventFields {
                    message: String::new(),
                    module_path: String::new(),
                },
            };
            event.record(&mut vis);

            let location = tracing_location_label(target, &vis.fields.module_path);

            let about: LogAbout = classify_log_target(target, &vis.fields.module_path, &vis.fields.message);
            let backend_chatter = is_backend_message(target, &vis.fields.module_path);
            if about == LogAbout::General {
                let msg: String = if vis.fields.message.is_empty() {
                    "<no message field>".to_string()
                } else {
                    vis.fields.message
                };
                let msg = normalize_output_message(LogAbout::General, &msg);
                console_logger::one_with_location_override(
                    Some(&location),
                    LogSev::DebugVerbose,
                    LogAbout::General,
                    &msg,
                );
                return;
            }

            let msg = if vis.fields.message.is_empty() {
                "<no message field>".to_string()
            } else {
                vis.fields.message
            };
            let msg = normalize_output_message(about.clone(), &msg);
            let sev = if backend_chatter {
                LogSev::DebugVerbose
            } else {
                sev
            };
            console_logger::one_with_location_override(
                Some(&location),
                sev,
                about,
                &msg,
            );
        }
    }

    Some(InterceptLogLayer.boxed())
}
