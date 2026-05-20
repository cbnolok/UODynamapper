# UODynamapper justfile
#
# This file serves as the unified entry point for building, testing, and packaging UODynamapper
# across Linux, macOS, and Windows. It replaces dozens of platform-specific shell/PowerShell scripts.
#
# --- Architecture Summary ---
# 1. Task Runner: 'just' handles OS detection, environment variable exports, and task orchestration.
# 2. Environment: All recipes export RUSTFLAGS and RUSTC_WRAPPER to ensure consistent behavior.
# 3. Wrapper Scripts (scripts/build/common/): These small scripts are necessary because
#    RUSTC_WRAPPER requires an executable/script to wrap rustc. They serve as a pass-through
#    when sccache is disabled, allowing for potential future interception logic (e.g. for specific crates).
# 4. Toolchains: We support both Stable and Nightly toolchains. Nightly is used for aggressive
#    optimizations like 'build-std' which recompiles the standard library for the target.
# 5. Linkers: On Linux, we automatically detect and use 'mold' or 'wild' for significantly
#    faster link times.

set shell := ["bash", "-c"]
set windows-shell := ["powershell.exe", "-c"]

# --- Platform & Environment Detection ---
os := os()
is_windows := if os == "windows" { "true" } else { "false" }
is_linux := if os == "linux" { "true" } else { "false" }
is_macos := if os == "macos" { "true" } else { "false" }
is_ci := env_var_or_default("GITHUB_ACTIONS", "false")

# --- Features ---
# Enable sccache explicitly. If "true", it will bypass the wrapper and use sccache directly.
sccache := "false"

# --- Tool Detection ---
has_sccache := `command -v sccache || echo ""`
has_mold := if is_linux == "true" { `command -v mold || echo ""` } else { "" }
has_wild := if is_linux == "true" { `command -v wild || echo ""` } else { "" }

# --- Build Configuration ---

# RUSTC_WRAPPER logic
wrapper_path := if is_windows == "true" {
    "scripts/build/common/rustc_wrapper.bat"
} else {
    "scripts/build/common/rustc_wrapper.sh"
}

# Determine if sccache should be used
sccache_requested := if sccache == "true" { "true" } else { is_ci }
sccache_available := if has_sccache != "" { "true" } else { "false" }
sccache_effective := if sccache_requested == "true" { sccache_available } else { "false" }

export RUSTC_WRAPPER := if sccache_effective == "true" { "sccache" } else { wrapper_path }

# Default RUSTFLAGS based on platform
linux_linker_base := if has_mold != "" {
    "-Clink-arg=-fuse-ld=mold"
} else if has_wild != "" {
    "-Clink-arg=-fuse-ld=wild"
} else {
    ""
}

# Specific GNU ld / ELF linker flags, not supported by windows cl.exe
linux_flags := linux_linker_base + " -Clink-arg=-Wl,--gc-sections -Clink-arg=-Wl,--no-allow-shlib-undefined"

# Features to enable on Linux by default (ensures Wayland/X11 support when using --no-default-features)
linux_features := if is_linux == "true" { "linux_wayland,linux_x11" } else { "" }

export RUSTFLAGS := if is_linux == "true" { linux_flags } else { "" }

# Specialized RUSTFLAGS for different build types (exported to be accessible in shell commands)
rustflags_optimized_common_nightly      := " -Zshare-generics=y -Zlocation-detail=none"
rustflags_optimized_common_stable       := ""
export RUSTFLAGS_RELEASE_NIGHTLY        := RUSTFLAGS + rustflags_optimized_common_nightly + " -Csymbol-mangling-version=v0 -Cforce-unwind-tables=no"
export RUSTFLAGS_RELEASE_STABLE         := RUSTFLAGS + rustflags_optimized_common_stable  + " -Csymbol-mangling-version=v0 -Cforce-unwind-tables=no"
export RUSTFLAGS_PROFILE_NIGHTLY        := RUSTFLAGS + rustflags_optimized_common_nightly + " -Cforce-frame-pointers=yes"
export RUSTFLAGS_PROFILE_STABLE         := RUSTFLAGS + rustflags_optimized_common_stable  + " -Cforce-frame-pointers=yes"
export CARGO_FLAGS_NIGHTLY              := " -Zbuild-std=std,panic_abort -Zbuild-std-features=optimize_for_size"

# Cross-platform Cargo runners to properly inject RUSTFLAGS in the shell
cargo_release_nightly   := if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_RELEASE_NIGHTLY; cargo +nightly" } else { "export RUSTFLAGS=\"$RUSTFLAGS_RELEASE_NIGHTLY\"; cargo +nightly" }
cargo_release_stable    := if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_RELEASE_STABLE; cargo" } else { "export RUSTFLAGS=\"$RUSTFLAGS_RELEASE_STABLE\"; cargo" }
cargo_profile_nightly   := if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_PROFILE_NIGHTLY; cargo +nightly" } else { "export RUSTFLAGS=\"$RUSTFLAGS_PROFILE_NIGHTLY\"; cargo +nightly" }
cargo_profile_stable    := if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_PROFILE_STABLE; cargo" } else { "export RUSTFLAGS=\"$RUSTFLAGS_PROFILE_STABLE\"; cargo" }

# --- Recipes ---

# List all available tasks
default:
    @just --list

# Build the workspace in debug mode
# Purpose: Fast compilation for local development. Includes debug symbols.
build-debug *args:
    @echo "Running {{os}} debug build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    cargo build --workspace {{args}}

# Build the workspace in release mode (stable toolchain)
# Purpose: Production build using the stable toolchain. Includes LTO and basic stripping.
# Linker: Uses mold/wild on Linux for speed, default on other platforms.
build-release-stable *args:
    @echo "Running {{os}} stable release build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    {{cargo_release_stable}} build --release --locked --workspace --no-default-features --features "{{linux_features}}" {{args}}

# Build the workspace in release mode (nightly toolchain, most optimized)
# Purpose: Highly optimized production build using nightly features.
# Optimizations: build-std (recompiles std with optimizations), panic_abort, symbol stripping.
build-release-nightly *args:
    @echo "Running {{os}} nightly release build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_release_nightly}} build --release --locked --workspace --no-default-features --features "{{linux_features}}" {{CARGO_FLAGS_NIGHTLY}} {{args}}

# Alias for nightly release build (preferred for production)
build-release *args:
    @just build-release-nightly {{args}}

# Build the workspace in profiling mode (stable toolchain)
# Purpose: Release-level optimizations but with frame pointers and symbols kept for profilers.
build-profile-stable *args:
    @echo "Running {{os}} stable profile build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    {{cargo_profile_stable}} build --profile profiling --locked --workspace --no-default-features --features "profiling,{{linux_features}}" {{args}}

# Build the workspace in profiling mode (nightly toolchain)
# Purpose: Most accurate profiling with optimized standard library symbols.
build-profile-nightly *args:
    @echo "Running {{os}} nightly profile build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_profile_nightly}} build --profile profiling --locked --workspace --no-default-features --features "profiling,{{linux_features}}" {{CARGO_FLAGS_NIGHTLY}} {{args}}

# Alias for stable profile build
build-profile *args:
    @just build-profile-stable {{args}}

# Run flamegraph profiling (requires cargo-flamegraph)
# Purpose: Generates a SVG flamegraph for performance analysis.
build-flamegraph *args:
    @echo "Running {{os}} flamegraph build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_profile_nightly}} flamegraph --profile profiling --no-default-features --features "profiling,{{linux_features}}" {{CARGO_FLAGS_NIGHTLY}} \
        --bin dynamapper --package dynamapper {{args}}

# Run bloat analysis (requires cargo-bloat and nightly)
# Purpose: Identifies which crates/functions contribute most to binary size.
bloat *args:
    @echo "Running {{os}} bloat analysis..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_release_nightly}} bloat --release --no-default-features --features "{{linux_features}}" {{CARGO_FLAGS_NIGHTLY}} \
        --config 'profile.release.strip=false' \
        {{args}}

# Package the build artifacts (Linux/macOS)
[unix]
package name="dynamapper-pkg" target="":
    #!/usr/bin/env bash
    set -euo pipefail
    RELEASE_DIR="target/release"
    if [ -n "{{target}}" ]; then
        RELEASE_DIR="target/{{target}}/release"
    fi
    DEST_DIR="artifact/{{name}}"
    mkdir -p "$DEST_DIR/tools/cli" "$DEST_DIR/tools/gui" "$DEST_DIR/tools/dev-tools"
    echo "Packaging {{name}} from $RELEASE_DIR..."
    [ -f "$RELEASE_DIR/dynamapper" ] && cp "$RELEASE_DIR/dynamapper" "$DEST_DIR/"
    CLI_UTILS=("udd-pack" "udd-tool" "uop-tool" "cc-uop-mul-converter" "uop-dict-populator-cli")
    for util in "${CLI_UTILS[@]}"; do
        [ -f "$RELEASE_DIR/$util" ] && cp "$RELEASE_DIR/$util" "$DEST_DIR/tools/cli/"
    done
    GUI_UTILS=("udd-conv-gui" "uddp-inspector-gui" "uocf-inspector-gui" "uop-dict-populator-gui")
    for util in "${GUI_UTILS[@]}"; do
        [ -f "$RELEASE_DIR/$util" ] && cp "$RELEASE_DIR/$util" "$DEST_DIR/tools/gui/"
    done
    [ -f "$RELEASE_DIR/texture-scanner" ] && cp "$RELEASE_DIR/texture-scanner" "$DEST_DIR/tools/dev-tools/"
    cp -r assets "$DEST_DIR/"
    [ -f "README.md" ] && cp "README.md" "$DEST_DIR/"
    echo "Packaging complete: $DEST_DIR"

# Package the build artifacts (Windows)
[windows]
package name="dynamapper-pkg" target="":
    @powershell -NoProfile -Command " \
    $releaseDir = if ('{{target}}' -ne '') { 'target/{{target}}/release' } else { 'target/release' }; \
    $destDir = 'artifact/{{name}}'; \
    New-Item -ItemType Directory -Force -Path \"$destDir/tools/cli\", \"$destDir/tools/gui\", \"$destDir/tools/dev-tools\" | Out-Null; \
    Write-Host \"Packaging {{name}} from $releaseDir...\"; \
    if (Test-Path \"$releaseDir/dynamapper.exe\") { Copy-Item \"$releaseDir/dynamapper.exe\" \"$destDir/\" }; \
    $cliUtils = @('udd-pack.exe', 'udd-tool.exe', 'uop-tool.exe', 'cc-uop-mul-converter.exe', 'uop-dict-populator-cli.exe'); \
    foreach ($util in $cliUtils) { if (Test-Path \"$releaseDir/$util\") { Copy-Item \"$releaseDir/$util\" \"$destDir/tools/cli/\" } }; \
    $guiUtils = @('udd-conv-gui.exe', 'uddp-inspector-gui.exe', 'uocf-inspector-gui.exe', 'uop-dict-populator-gui.exe'); \
    foreach ($util in $guiUtils) { if (Test-Path \"$releaseDir/$util\") { Copy-Item \"$releaseDir/$util\" \"$destDir/tools/gui/\" } }; \
    if (Test-Path \"$releaseDir/texture-scanner.exe\") { Copy-Item \"$releaseDir/texture-scanner.exe\" \"$destDir/tools/dev-tools/\" }; \
    Copy-Item -Recurse assets \"$destDir/\"; \
    if (Test-Path 'README.md') { Copy-Item 'README.md' \"$destDir/\" }; \
    Write-Host \"Packaging complete: $destDir\""

# --- Development Run Recipes ---

# Alias for run-dynamapper-debug
run-dynamapper *args:
    @just run-dynamapper-debug {{args}}

# Run the main application in debug mode
run-dynamapper-debug *args:
    @echo "Running dynamapper in debug mode..."
    cargo run --bin dynamapper {{args}}

# Run the main application in release mode (nightly)
run-dynamapper-release *args:
    @echo "Running dynamapper in release mode (nightly Rust toolchain)..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_release_nightly}} run --release --no-default-features --features "{{linux_features}}" {{CARGO_FLAGS_NIGHTLY}} \
        --bin dynamapper {{args}}

# Run the main application in profiling mode (stable, better symbol resolution by perf)
run-dynamapper-profile *args:
    @echo "Running dynamapper in profiling mode (nightly Rust toolchain)..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    {{cargo_profile_stable}} run --profile profiling --no-default-features --features "profiling,{{linux_features}}" \
        --bin dynamapper {{args}}

# Alias for run-tool-debug
run-tool tool *args:
    @just run-tool-debug {{tool}} {{args}}

# Run any tool from the workspace in debug mode
run-tool-debug tool *args:
    @echo "Running tool {{tool}} in debug mode via cargo..."
    cargo run --bin {{tool}} -- {{args}}

# Run any tool from the workspace in release mode (nightly)
run-tool-release tool *args:
    @echo "Running tool {{tool}} in release mode via cargo..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_release_nightly}} run --release --no-default-features --features "{{linux_features}}" {{CARGO_FLAGS_NIGHTLY}} \
        --bin {{tool}} -- {{args}}


# --- Maintenance ---

# Run tests across the whole workspace
test:
    cargo test --workspace

# Clean build artifacts
clean:
    cargo clean
    rm -rf artifact/

# Format all code
fmt:
    cargo fmt --all

# Run clippy for the whole workspace
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Lint the Bevy project using bevy_cli
bevy-lint:
    bevy lint
