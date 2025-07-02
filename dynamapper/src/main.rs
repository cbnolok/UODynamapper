use std::process::ExitCode;

pub mod core;
pub mod logger;
mod prelude;

#[macro_use]
pub mod util_lib;

fn main() -> ExitCode {
    println!("Starting Bevy app.");
    core::run_bevy_app()
}
