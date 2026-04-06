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
}
