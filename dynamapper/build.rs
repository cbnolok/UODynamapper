fn main() {
    // Scope this build script's inputs explicitly. Without any `rerun-if-*`
    // directive, Cargo re-runs the script whenever ANY file in the package
    // directory changes — including `assets/settings/*.toml`, which the app
    // rewrites on every run. That made each `cargo run` re-run build.rs and
    // relink the (large) binary even with no source change. Watch only this
    // script and the env var it actually reads.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CFG_RELEASE_CHANNEL");

    // Force full location detail for workspace crates on nightly.
    // This overrides any global `-Zlocation-detail=none` set in wrapper scripts.
    // Only has effect on nightly; silently ignored on stable/beta.
    let is_nightly = std::env::var("CFG_RELEASE_CHANNEL")
        .map(|channel| channel == "nightly")
        .unwrap_or(false);

    if is_nightly {
        println!("cargo:rustc-flag=-Zlocation-detail=full");
    }
}
