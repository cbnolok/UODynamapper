use std::process::ExitCode;

pub mod core;

fn main() -> ExitCode {
    core::run_bevy_app()
}
