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

    /*  CRATE: intel_tex_2  */
    // Removed: intel_tex_2 is no longer a dependency of uocf.
    // The C++ linking logic for stdc++/c++ is no longer required.
    // TODO: move this to the uddconv build.rs if that crate is the only user of intel_tex_2.
}
