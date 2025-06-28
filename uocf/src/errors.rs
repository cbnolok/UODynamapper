#[macro_export]
macro_rules! eyre_imports {
    () => {
        use color_eyre::eyre::{
            self,    // for eyre::Result
            eyre,    // for eyre! macro
            WrapErr, // for wrap_err* methods
        };
    };
}
