pub mod math;
pub mod image;
// Temporary: hot files still import the old path (docs/reviews/2026-11-18-dynamapper-module-map.md, Order 3).
pub use crate::world::coords as uo_coords;
#[cfg(test)]
pub mod wesl_source;

#[macro_use]
pub mod tracked_plugin;

/// Get the current function name.
#[macro_export]
macro_rules! fname {
    () => {{
        fn f_() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let name = type_name_of(f_);
        name.strip_suffix("::f_").unwrap()
    }}
}

/// Enter a Tracy span only when the crate-level `trace_tracy` feature is enabled.
///
/// The macro returns a guard that keeps the span open for the current scope and
/// becomes a zero-cost no-op when profiling is disabled.
#[macro_export]
macro_rules! tracy_span {
    ($($arg:tt)*) => {{
        #[cfg(feature = "trace_tracy")]
        let _entered = tracing::info_span!($($arg)*).entered();
        #[cfg(not(feature = "trace_tracy"))]
        #[allow(clippy::let_unit_value)]
        let _entered = ();
        _entered
    }};
}
