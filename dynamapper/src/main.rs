use std::process::ExitCode;

pub mod settings;
pub mod console_logger;
pub mod content;
pub mod app;
pub mod diagnostics;
pub mod export;
pub mod input;
pub mod render;
pub mod ui;
pub mod world;
pub mod ingame_sysmessage_logger;
mod prelude;

#[macro_use]
pub mod util_lib;

//Use Mimalloc only when Bevy is compiled as a static library (not with bevy/dynamic_linking enabled).
//  When Bevy is compiled as a shared library, it gets its own copy of the Rust standard library
//  with the system allocator. The #[global_allocator] only applies to the main binary's allocation context.

#[cfg(not(feature = "bevy_development_opts"))]
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;
//static GLOBAL_ALLOCATOR: jemallocator::Jemalloc = jemallocator::Jemalloc;

fn main() -> ExitCode {
    color_eyre::install() // colored panic and backtrace
        .expect("Can't install color_eyre?");

    // Before any package decodes: udd's parallel decodes share the background pool's threads.
    app::background::install_udd_executor();
    console_logger::system("Starting Bevy app.");
    app::run_bevy_app()
}
