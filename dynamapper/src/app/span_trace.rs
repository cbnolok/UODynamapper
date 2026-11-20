//! A span trace for hunting hitches. With `DYNAMAPPER_SPAN_TRACE=<file>`, every tracing span lasting at least
//! [`MIN_SPAN`] is written to the file as `thread start_us duration_us label`, after a `T0_UNIX_US` line with the
//! wall-clock time of start 0. Every `update` span is written whatever its length, and flushes the file, so the frames
//! can be told apart.
//!
//! Without the variable no layer is installed and the app pays nothing. Bevy's per-system spans need the `span_trace`
//! cargo feature (`bevy/trace`), which costs every frame; without it the trace holds the app's own spans and the
//! backends'. `scripts/dev-helpers/renderer/span_trace/` runs and reads it.
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::field::{Field, Visit};
use tracing::{span, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

pub const SPAN_TRACE_ENV: &str = "DYNAMAPPER_SPAN_TRACE";
/// Shorter spans are left out: they are most of the spans and none of the hitches.
const MIN_SPAN: Duration = Duration::from_micros(50);
/// Bevy's span around one frame of the main app.
const FRAME_SPAN: &str = "update";
/// The span field that names a system or a schedule; the label is `span:name`.
const NAME_FIELD: &str = "name";

pub struct SpanTraceLayer {
    out: Mutex<BufWriter<File>>,
    t0: Instant,
}

struct Label(String);
struct Entered(Instant);
struct NameVisitor(Option<String>);

impl Visit for NameVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == NAME_FIELD {
            self.0 = Some(format!("{value:?}"));
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == NAME_FIELD {
            self.0 = Some(value.to_owned());
        }
    }
}

/// The layer, if [`SPAN_TRACE_ENV`] names a file that can be created.
pub fn layer() -> Option<SpanTraceLayer> {
    let path = std::env::var_os(SPAN_TRACE_ENV)?;
    let file = match File::create(&path) {
        Ok(file) => file,
        Err(error) => {
            // The logger is not up yet.
            eprintln!("{SPAN_TRACE_ENV}: cannot create {}: {error}", path.to_string_lossy());
            return None;
        }
    };
    let mut out = BufWriter::new(file);
    let t0 = Instant::now();
    let unix_us = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_micros());
    writeln!(out, "T0_UNIX_US {unix_us}").ok()?;
    Some(SpanTraceLayer { out: Mutex::new(out), t0 })
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SpanTraceLayer {
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let mut visitor = NameVisitor(None);
        attrs.record(&mut visitor);
        let name = attrs.metadata().name();
        let label = match visitor.0 {
            Some(field) => format!("{name}:{field}"),
            None => name.to_owned(),
        };
        if let Some(span) = ctx.span(id) {
            // The file's fields are space-separated.
            span.extensions_mut().insert(Label(label.replace(' ', "_")));
        }
    }

    fn on_enter(&self, id: &span::Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().replace(Entered(Instant::now()));
        }
    }

    fn on_exit(&self, id: &span::Id, ctx: Context<'_, S>) {
        let now = Instant::now();
        let Some(span) = ctx.span(id) else { return };
        let extensions = span.extensions();
        let (Some(Entered(start)), Some(Label(label))) = (extensions.get::<Entered>(), extensions.get::<Label>()) else {
            return;
        };
        let duration = now.saturating_duration_since(*start);
        let is_frame = label == FRAME_SPAN;
        if duration < MIN_SPAN && !is_frame {
            return;
        }
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("?").replace(' ', "_");
        let Ok(mut out) = self.out.lock() else { return };
        let _ = writeln!(out, "{thread} {} {} {label}", start.saturating_duration_since(self.t0).as_micros(),
            duration.as_micros());
        if is_frame {
            let _ = out.flush();
        }
    }
}
