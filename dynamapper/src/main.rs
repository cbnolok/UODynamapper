use std::process::ExitCode;

pub mod core;
pub mod external_data;
pub mod logger;
mod prelude;

#[macro_use]
pub mod util_lib;

fn main() -> ExitCode {
    color_eyre::install() // colored panic and backtrace
        .expect("Can't install color_eyre?");

    println!("Starting Bevy app.");
    core::run_bevy_app()
}
