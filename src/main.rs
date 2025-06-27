use std::process::ExitCode;

pub mod core;
pub mod logger;

fn main() -> ExitCode {
    core::run_bevy_app()
}
