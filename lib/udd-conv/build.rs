fn main() {
    // Force full location detail for workspace crates on nightly.
    // This overrides any global `-Zlocation-detail=none` set in wrapper scripts.
    // Only has effect on nightly; silently ignored on stable/beta.
    let is_nightly = std::env::var("CFG_RELEASE_CHANNEL")
        .map(|channel| channel == "nightly")
        .unwrap_or(false);

    if is_nightly {
        println!("cargo:rustc-flag=-Zlocation-detail=full");
    }

    // BC7-related native backends used by this crate can pull in C++ objects on some
    // platforms. Link the platform C++ runtime unconditionally for this crate so test
    // binaries and feature combinations resolve the same way as normal library builds.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    match (target_os.as_str(), target_env.as_str()) {
        ("linux", _) | ("windows", "gnu") => {
            println!("cargo:rustc-link-lib=stdc++");
        }
        ("macos", _) => {
            println!("cargo:rustc-link-lib=c++");
        }
        _ => {}
    }
}
