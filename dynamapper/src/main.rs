use std::process::ExitCode;

mod core;
mod logger;

#[macro_use]
pub mod util_lib;

fn main() -> ExitCode {
    println!("Starting Bevy app.");
    core::run_bevy_app()
}
