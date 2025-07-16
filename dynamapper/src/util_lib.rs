pub mod array;
pub mod math;
pub mod image;
//pub mod rect;
pub mod uo_coords;

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
