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
    // Preface: we aren't using this crate anymore, but we might use in the future crates with similar issues.

    // `intel_tex_2` uses ISPC-compiled code (ASTC texture compression) that references
    // C++ exception-handling symbols (e.g. `__gxx_personality_v0`) from libstdc++.
    // The `mold` linker is stricter than GNU ld and won't resolve these automatically,
    // so we link libstdc++ explicitly here.
    //
    // Why this doesn't surface in `--release --no-default-features`:
    // With `dynamic_linking` disabled (the `bevy_development_opts` default feature),
    // all of Bevy's crates are linked as static `.rlib`s. Their own `build.rs` scripts
    // emit `cargo:rustc-link-lib=stdc++`, and those directives flow through to the final
    // binary's linker invocation, so `libstdc++` is already on the link line.
    //
    // With `dynamic_linking` enabled (debug / default builds), Bevy's crates are
    // pre-linked into `libbevy_dylib.so`. Their `build.rs` directives are consumed by
    // *that* shared-library link step and become a runtime `NEEDED` entry of the `.so` —
    // they do NOT propagate to `dynamapper`'s own link step. Meanwhile `intel_tex_2` is
    // a direct dep of `dynamapper` (not inside the dylib), so its ISPC object files land
    // in `dynamapper`'s link and `__gxx_personality_v0` goes unresolved. Emitting the
    // directive from `dynamapper`'s own `build.rs` makes the dep explicit and ensures
    // `libstdc++` is on the linker command line regardless of whether dynamic linking is
    // active.
    // Only emit the directive for toolchains that use libstdc++ for C++ runtime support.
    // - Linux (gnu): always uses libstdc++.
    // - Windows GNU (MinGW): also uses libstdc++; same issue applies.
    // - Windows MSVC: the MSVC linker automatically pulls in the Visual C++ runtime
    //   (msvcrt / vcruntime), which provides the equivalent symbols (__CxxFrameHandler3
    //   instead of __gxx_personality_v0). No explicit linkage needed.
    // - macOS (not a target here): uses libc++ instead of libstdc++; would need
    //   `cargo:rustc-link-lib=c++` instead.
    let target_os  = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    match (target_os.as_str(), target_env.as_str()) {
        ("linux", _) | ("windows", "gnu") => {
            println!("cargo:rustc-link-lib=stdc++");
        }
        _ => {}
    }
}
