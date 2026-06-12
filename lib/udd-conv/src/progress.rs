use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub(crate) use tracing_indicatif::style::ProgressStyle;
use tracing_indicatif::span_ext::IndicatifSpanExt;

const NO_LENGTH: u64 = u64::MAX;

thread_local! {
    static PROGRESS_STACK: RefCell<Vec<tracing::Span>> = RefCell::new(Vec::new());
}

pub(crate) struct ProgressBar {
    span: Mutex<Option<tracing::Span>>,
    length: AtomicU64,
}

impl ProgressBar {
    pub(crate) fn new(length: u64) -> Self {
        let span = progress_span();
        span.pb_set_length(length);
        span.pb_start();
        push_progress_span(span.clone());
        Self {
            span: Mutex::new(Some(span)),
            length: AtomicU64::new(length),
        }
    }

    pub(crate) fn set_style(&self, style: ProgressStyle) {
        self.with_span(|span| span.pb_set_style(&style));
    }

    pub(crate) fn set_message(&self, message: impl Into<String>) {
        let message = message.into();
        self.with_span(|span| span.pb_set_message(&message));
    }

    pub(crate) fn inc(&self, delta: u64) {
        self.with_span(|span| span.pb_inc(delta));
    }

    pub(crate) fn set_position(&self, position: u64) {
        self.with_span(|span| span.pb_set_position(position));
    }

    pub(crate) fn set_length(&self, length: u64) {
        self.length.store(length, Ordering::Relaxed);
        self.with_span(|span| span.pb_set_length(length));
    }

    pub(crate) fn length(&self) -> Option<u64> {
        match self.length.load(Ordering::Relaxed) {
            NO_LENGTH => None,
            length => Some(length),
        }
    }

    pub(crate) fn enable_steady_tick(&self, _interval: std::time::Duration) {
        self.with_span(|span| span.pb_tick());
    }

    pub(crate) fn disable_steady_tick(&self) {}

    pub(crate) fn finish_with_message(&self, message: impl Into<String>) {
        let message = message.into();
        let span = self.close_span();
        if let Some(span) = span {
            if let Some(length) = self.length() {
                span.pb_set_position(length);
            }
            span.pb_set_finish_message(&message);
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
