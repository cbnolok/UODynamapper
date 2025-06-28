//! Boots Bevy, installs cache and chunk systems, sets light direction.
/*
#![allow(unused_parens)]
*/
pub mod texture_cache;
pub mod constants;
pub mod render;

use std::process::ExitCode;
use bevy::prelude::*;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn run_bevy_app() -> ExitCode {
    // Install the custom own log subscriber (must come BEFORE Bevy app launch!)
    //  to change the default Bevy log format.
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_ansi(true) // colored output like Bevy default
                .with_level(true)
                .with_target(true)
                // Use chrono for timestamp, format with NO milliseconds
                .with_timer(fmt::time::ChronoLocal::new("%Y-%m-%d %H:%M:%s".into()))
                .compact() // Looks a lot like Bevy default (use .pretty() for multiline pretty logs)
        )
        .with(EnvFilter::from_default_env())
        .init();

    let result = App::new()
        //.insert_resource(WindowDescriptor {
        //    title: "UODynamapper".to_string(),
        //    ..Default::default()
        //    })
        .add_plugins(DefaultPlugins.build()
            .disable::<bevy::log::LogPlugin>()
            .set(ImagePlugin::default_nearest()))
        .add_plugins((
            render::RenderPlugin                { registered_by: "CorePlugin" },
            texture_cache::TextureCachePlugin   { registered_by: "CorePlugin" },
        ))
        .run();

    match result {
        AppExit::Success => ExitCode::SUCCESS,
        AppExit::Error(value) => ExitCode::from(value.get()),
    }
}

