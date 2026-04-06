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

    // `intel_tex_2` uses ISPC-compiled code that may require an explicit C++ runtime link.
    if std::env::var_os("CARGO_FEATURE_INTEL_TEX").is_some() {
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
}
