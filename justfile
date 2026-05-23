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
has_sccache := if is_windows == "true" {
    `powershell -NoProfile -Command "if (Get-Command sccache -ErrorAction SilentlyContinue) { (Get-Command sccache).Source }"`
} else {
    `command -v sccache || echo ""`
}
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
    APP_DIR="artifact/{{name}}/UODynamapper"
    TOOLS_DIR="artifact/{{name}}-tools/UODynamapper-tools"
    rm -rf "artifact/{{name}}" "artifact/{{name}}-tools"
    mkdir -p "$APP_DIR" "$TOOLS_DIR/bin" "$TOOLS_DIR/shared" "$TOOLS_DIR/docs"
    echo "Packaging {{name}} from $RELEASE_DIR..."
    [ -f "$RELEASE_DIR/dynamapper" ] && cp "$RELEASE_DIR/dynamapper" "$APP_DIR/"
    cp -r dynamapper/assets "$APP_DIR/"
    [ -f "README.md" ] && cp "README.md" "$APP_DIR/"
    mkdir -p "$APP_DIR/docs"
    for doc in docs/keybindings.md docs/USER_TROUBLESHOOTING.md; do
        [ -f "$doc" ] && cp "$doc" "$APP_DIR/docs/"
    done

    TOOL_BINS=(
        "udd-conv-gui" "udd-pack" "udd-tool" "uddp-inspector-gui"
        "uocf-inspector-gui" "uop-tool" "cc-uop-mul-converter"
        "texture-scanner" "sound-tool" "multimap-tool" "facet-evidence-tool"
        "kr-ec-terrain-diff-tool" "uop-dict-populator-cli" "uop-dict-populator-gui"
    )
    for util in "${TOOL_BINS[@]}"; do
        [ -f "$RELEASE_DIR/$util" ] && cp "$RELEASE_DIR/$util" "$TOOLS_DIR/bin/"
    done
    [ -f "tools/_shared_assets/Dictionary.dic" ] && cp "tools/_shared_assets/Dictionary.dic" "$TOOLS_DIR/shared/"
    for doc in docs/ASSET_PIPELINE.md docs/WORKSPACE_COMPONENTS.md docs/UDDP_FORMATS.md README.md; do
        [ -f "$doc" ] && cp "$doc" "$TOOLS_DIR/docs/"
    done
    echo "Packaging complete: $APP_DIR and $TOOLS_DIR"

# Package the build artifacts (Windows)
[windows]
package name="dynamapper-pkg" target="":
    @powershell -NoProfile -Command " \
    $releaseDir = if ('{{target}}' -ne '') { 'target/{{target}}/release' } else { 'target/release' }; \
    $appDir = 'artifact/{{name}}/UODynamapper'; \
    $toolsDir = 'artifact/{{name}}-tools/UODynamapper-tools'; \
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue 'artifact/{{name}}', 'artifact/{{name}}-tools'; \
    New-Item -ItemType Directory -Force -Path \"$appDir\", \"$appDir/docs\", \"$toolsDir/bin\", \"$toolsDir/shared\", \"$toolsDir/docs\" | Out-Null; \
    Write-Host \"Packaging {{name}} from $releaseDir...\"; \
    if (Test-Path \"$releaseDir/dynamapper.exe\") { Copy-Item \"$releaseDir/dynamapper.exe\" \"$appDir/\" }; \
    Copy-Item -Recurse 'dynamapper/assets' \"$appDir/assets\"; \
    if (Test-Path 'README.md') { Copy-Item 'README.md' \"$appDir/\" }; \
    foreach ($doc in @('docs/keybindings.md', 'docs/USER_TROUBLESHOOTING.md')) { if (Test-Path $doc) { Copy-Item $doc \"$appDir/docs/\" } }; \
    $toolBins = @('udd-conv-gui.exe', 'udd-pack.exe', 'udd-tool.exe', 'uddp-inspector-gui.exe', 'uocf-inspector-gui.exe', 'uop-tool.exe', 'cc-uop-mul-converter.exe', 'texture-scanner.exe', 'sound-tool.exe', 'multimap-tool.exe', 'facet-evidence-tool.exe', 'kr-ec-terrain-diff-tool.exe', 'uop-dict-populator-cli.exe', 'uop-dict-populator-gui.exe'); \
    foreach ($util in $toolBins) { if (Test-Path \"$releaseDir/$util\") { Copy-Item \"$releaseDir/$util\" \"$toolsDir/bin/\" } }; \
    if (Test-Path 'tools/_shared_assets/Dictionary.dic') { Copy-Item 'tools/_shared_assets/Dictionary.dic' \"$toolsDir/shared/\" }; \
    foreach ($doc in @('docs/ASSET_PIPELINE.md', 'docs/WORKSPACE_COMPONENTS.md', 'docs/UDDP_FORMATS.md', 'README.md')) { if (Test-Path $doc) { Copy-Item $doc \"$toolsDir/docs/\" } }; \
    Write-Host \"Packaging complete: $appDir and $toolsDir\""

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
