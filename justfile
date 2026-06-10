# UODynamapper workspace justfile
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
RUSTFLAGS := env_var_or_default("RUSTFLAGS", "")

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

# RUSTC_WRAPPER logic (needed for sccache)
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

# Determine linker to be used and to be passed to RUSTFLAGS
# For now linux_debug_linker_base is unused, since we are relying on config.toml for rapid iteration debug
#   builds with stable rust toolchain
linux_debug_linker_base := if has_wild != "" {
    "-Clink-arg=-fuse-ld=wild"
} else if has_mold != "" {
    "-Clink-arg=-fuse-ld=mold"
} else {
    ""
}
linux_optimized_linker_base := if has_mold != "" {
    "-Clink-arg=-fuse-ld=mold"
} else {
    ""
}

# Linker flags per build
linker_debug_flags := if is_linux == "true" {
    linux_debug_linker_base
} else {
    ""
}
linker_optimized_flags := if is_linux == "true" {
    linux_optimized_linker_base +
    "-Clink-arg=-Wl,--gc-sections -Clink-arg=-Wl,--no-allow-shlib-undefined"
} else if is_macos == "true" {
    "-Clink-arg=-Wl,-dead_strip   -Clink-arg=-Wl,--no-allow-shlib-undefined"
} else {
  ""
}

# Features to enable on Linux by default (ensures Wayland/X11 support when using --no-default-features)
linux_target_features := if is_linux == "true" { "linux_wayland,linux_x11" } else { "" }
linux_features := linux_target_features
# Added -Clink-arg=-lgcc to musl RUSTFLAGS to satisfy compiler builtins like __popcountdi2 emitted by vendored libjxl C++ objects
linux_musl_rustflags := " -C target-feature=+crt-static -C link-self-contained=yes -Clink-arg=-lgcc"

# Specialized RUSTFLAGS for different build types (exported to be accessible in shell commands)
rustflags_debug_common              := " -C embed-bitcode=no"   # llvm bitcode is unneeded since we are not using LTO in debug
rustflags_optimized_common_stable   := ""
rustflags_optimized_common_nightly  := rustflags_optimized_common_stable  +\
                                        " -Zshare-generics=y -Zlocation-detail=none"
export RUSTFLAGS_RELEASE_STABLE     := rustflags_optimized_common_stable  +\
                                        " -Csymbol-mangling-version=v0 -Cforce-unwind-tables=no"
export RUSTFLAGS_RELEASE_NIGHTLY    := rustflags_optimized_common_nightly +\
                                        " -Csymbol-mangling-version=v0 -Cforce-unwind-tables=no"
export RUSTFLAGS_PROFILE_STABLE     := rustflags_optimized_common_stable  +\
                                        " -Cforce-frame-pointers=yes"
export RUSTFLAGS_PROFILE_NIGHTLY    := rustflags_optimized_common_nightly +\
                                        " -Cforce-frame-pointers=yes"
# Specialized cargo flags
export CARGO_FLAGS_NIGHTLY          := " -Zbuild-std=std,panic_abort -Zbuild-std-features=optimize_for_size"
#   -Zfmt-debug=shallow
#   -Zembed-metadata=no --emit=metadata to produce the full metadata into a separate .rmeta file.

# Cross-platform Cargo runners to properly inject RUSTFLAGS in the shell
cargo_release_nightly   :=\
    if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_RELEASE_NIGHTLY; cargo +nightly" } else { "export RUSTFLAGS=\"$RUSTFLAGS_RELEASE_NIGHTLY\"; cargo +nightly" }
cargo_release_stable    :=\
    if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_RELEASE_STABLE; cargo" }           else { "export RUSTFLAGS=\"$RUSTFLAGS_RELEASE_STABLE\"; cargo" }
cargo_profile_nightly   :=\
    if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_PROFILE_NIGHTLY; cargo +nightly" } else { "export RUSTFLAGS=\"$RUSTFLAGS_PROFILE_NIGHTLY\"; cargo +nightly" }
cargo_profile_stable    :=\
    if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_PROFILE_STABLE; cargo" }           else { "export RUSTFLAGS=\"$RUSTFLAGS_PROFILE_STABLE\"; cargo" }

# --- Release Package Contents ---
app_docs := "docs/keybindings.md docs/USER_TROUBLESHOOTING.md"
tool_docs := "README.md docs/USER_TROUBLESHOOTING.md"
tool_bins := "udd-conv-gui udd-pack udd-tool uddp-inspector-gui uocf-inspector-gui uop-tool cc-uop-mul-converter sound-tool multimap-tool uop-dict-populator-cli uop-dict-populator-gui"
tool_packages := "-p udd-conv-cli -p uocf-cli -p udd-conv-gui -p uddp-inspector-gui -p uocf-inspector-gui -p uop-dict-populator-gui"

# --- Recipes ---

# List all available tasks
default:
    @just --list

# Build the workspace locally in debug mode
# Purpose: Fast compilation for local development. Includes debug symbols.
build-local-workspace-debug *args:
    @echo "Running {{os}} debug build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    cargo build --workspace "{{args}}"

# Build only dynamapper locally in debug mode
build-local-dynamapper-debug *args:
    cargo build -p dynamapper --bin dynamapper "{{args}}"

# Build only the shipped tools locally in debug mode
build-local-tools-debug *args:
    cargo build {{tool_packages}} --bins {{args}}

# Build udd-pack locally in debug mode using the same package selection as the uddconv runtime path.
build-local-udd-pack-debug *args:
    @echo "Running {{os}} udd-pack debug build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    cargo build -p udd-conv-cli --bin udd-pack "{{args}}"

# Build the workspace locally in release mode (stable toolchain)
# Purpose: Production build using the stable toolchain. Includes LTO and basic stripping.
# Linker: Uses mold/wild on Linux for speed, default on other platforms.
build-local-workspace-release *args:
    @echo "Running {{os}} stable release build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    {{cargo_release_stable}} build --release --locked --workspace --no-default-features --features \
        "{{linux_features}}" "{{args}}"

# Build the workspace for a Linux musl target in release mode (stable toolchain)
build-linux-musl-release target="x86_64-unknown-linux-musl" *args:
    #!/usr/bin/env bash
    set -euo pipefail
    export RUSTFLAGS_RELEASE_STABLE="{{RUSTFLAGS_RELEASE_STABLE}}{{linux_musl_rustflags}}"
    echo "Running {{os}} stable musl release build for target {{target}}..."
    echo "Using RUSTFLAGS: $RUSTFLAGS_RELEASE_STABLE"
    {{cargo_release_stable}} build --release --locked --workspace --target "{{target}}" --no-default-features --features "{{linux_target_features}}" {{args}}

# Build the workspace for a Linux musl target in release mode (nightly toolchain)
build-linux-musl-release-nightly target="x86_64-unknown-linux-musl" *args:
    #!/usr/bin/env bash
    set -euo pipefail
    export RUSTFLAGS_RELEASE_NIGHTLY="{{RUSTFLAGS_RELEASE_NIGHTLY}}{{linux_musl_rustflags}}"
    echo "Running {{os}} nightly musl release build for target {{target}}..."
    echo "Using RUSTFLAGS: $RUSTFLAGS_RELEASE_NIGHTLY"
    echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_release_nightly}} build --release --locked --workspace --target "{{target}}" --no-default-features --features "{{linux_target_features}}" {{CARGO_FLAGS_NIGHTLY}} {{args}}

# Build the workspace exactly as CI does for release artifacts
build-ci-workspace *args:
    @echo "Running {{os}} CI workspace build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_release_nightly}} build --release --locked --workspace --no-default-features --features "{{linux_features}}" "{{CARGO_FLAGS_NIGHTLY}}" "{{args}}"

# Build only dynamapper with the same release settings used by CI
build-ci-dynamapper *args:
    @echo "Running {{os}} CI dynamapper build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_release_nightly}} build --release --locked --no-default-features --features "{{linux_features}}" "{{CARGO_FLAGS_NIGHTLY}}" \
        -p dynamapper --bin dynamapper "{{args}}"

# Build only the shipped tools with the same release settings used by CI
build-ci-tools *args:
    @echo "Running {{os}} CI tools build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_release_nightly}} build --release --locked --no-default-features --features "{{linux_features}}" "{{CARGO_FLAGS_NIGHTLY}}" \
        {{tool_packages}} --bins "{{args}}"

# Build the binaries that are included in release artifacts
build-ci-shipping *args:
    @just build-ci-dynamapper {{args}}
    @just build-ci-tools {{args}}

# Build the workspace locally in profiling mode (stable toolchain)
# Purpose: Release-level optimizations but with frame pointers and symbols kept for profilers.
build-local-workspace-profile *args:
    @echo "Running {{os}} stable profile build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    {{cargo_profile_stable}} build --profile profiling --locked --workspace --no-default-features --features "profiling,{{linux_features}}" "{{args}}"

# Build the workspace for a Linux musl target in profiling mode (stable toolchain)
build-linux-musl-profile target="x86_64-unknown-linux-musl" *args:
    #!/usr/bin/env bash
    set -euo pipefail
    export RUSTFLAGS_PROFILE_STABLE="{{RUSTFLAGS_PROFILE_STABLE}}{{linux_musl_rustflags}}"
    echo "Running {{os}} stable musl profile build for target {{target}}..."
    echo "Using RUSTFLAGS: $RUSTFLAGS_PROFILE_STABLE"
    {{cargo_profile_stable}} build --profile profiling --locked --workspace --target "{{target}}" --no-default-features --features "profiling,{{linux_target_features}}" {{args}}

# Build the workspace locally in profiling mode (nightly toolchain)
# Purpose: Most accurate profiling with optimized standard library symbols.
build-local-workspace-profile-nightly *args:
    @echo "Running {{os}} nightly profile build..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_profile_nightly}} build --profile profiling --locked --workspace --no-default-features --features "profiling,{{linux_features}}" "{{CARGO_FLAGS_NIGHTLY}}" "{{args}}"

# Build the workspace for a Linux musl target in profiling mode (nightly toolchain)
build-linux-musl-profile-nightly target="x86_64-unknown-linux-musl" *args:
    #!/usr/bin/env bash
    set -euo pipefail
    export RUSTFLAGS_PROFILE_NIGHTLY="{{RUSTFLAGS_PROFILE_NIGHTLY}}{{linux_musl_rustflags}}"
    echo "Running {{os}} nightly musl profile build for target {{target}}..."
    echo "Using RUSTFLAGS: $RUSTFLAGS_PROFILE_NIGHTLY"
    echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_profile_nightly}} build --profile profiling --locked --workspace --target "{{target}}" --no-default-features --features "profiling,{{linux_target_features}}" {{CARGO_FLAGS_NIGHTLY}} {{args}}

# Run flamegraph profiling (requires cargo-flamegraph)
# Purpose: Generates a SVG flamegraph for performance analysis.
profile-dynamapper-flamegraph *args:
    @echo "Running {{os}} dynamapper flamegraph profile..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_profile_nightly}} flamegraph --profile profiling --no-default-features --features "profiling,{{linux_features}}" "{{CARGO_FLAGS_NIGHTLY}}" \
        --bin dynamapper --package dynamapper "{{args}}"

# Run bloat analysis (requires cargo-bloat and nightly)
# Purpose: Identifies which crates/functions contribute most to binary size.
bloat *args:
    @echo "Running {{os}} bloat analysis..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_release_nightly}} bloat --release --no-default-features --features "{{linux_features}}" "{{CARGO_FLAGS_NIGHTLY}}" \
        --config 'profile.release.strip=false' \
        "{{args}}"

# Verify release package inputs before creating CI artifacts
[unix]
verify-package-inputs target="":
    #!/usr/bin/env bash
    set -euo pipefail
    RELEASE_DIR="target/release"
    if [ -n "{{target}}" ]; then
        RELEASE_DIR="target/{{target}}/release"
    fi
    missing=0
    require_file() {
        if [ ! -f "$1" ]; then
            echo "missing file: $1"
            missing=1
        fi
    }
    require_dir() {
        if [ ! -d "$1" ]; then
            echo "missing directory: $1"
            missing=1
        fi
    }

    require_file "$RELEASE_DIR/dynamapper"
    for util in {{tool_bins}}; do
        require_file "$RELEASE_DIR/$util"
    done
    require_dir "dynamapper/assets"
    require_file "tools/_shared_assets/Dictionary.dic"
    for doc in {{app_docs}} {{tool_docs}}; do
        require_file "$doc"
    done
    exit "$missing"

# Verify release package inputs before creating CI artifacts
[windows]
verify-package-inputs target="":
    @powershell -NoProfile -Command " \
    $releaseDir = if ('{{target}}' -ne '') { 'target/{{target}}/release' } else { 'target/release' }; \
    $missing = $false; \
    function Require-File([string]$path) { if (-not (Test-Path -Path $path -PathType Leaf)) { Write-Host \"missing file: $path\"; $script:missing = $true } }; \
    function Require-Dir([string]$path) { if (-not (Test-Path -Path $path -PathType Container)) { Write-Host \"missing directory: $path\"; $script:missing = $true } }; \
    Require-File \"$releaseDir/dynamapper.exe\"; \
    foreach ($util in '{{tool_bins}}'.Split(' ')) { Require-File \"$releaseDir/$util.exe\" }; \
    Require-Dir 'dynamapper/assets'; \
    Require-File 'tools/_shared_assets/Dictionary.dic'; \
    foreach ($doc in ('{{app_docs}} {{tool_docs}}'.Split(' '))) { Require-File $doc }; \
    if ($missing) { exit 1 }"

# Strict release packaging: validates required inputs, then creates app and tools artifacts
package-release name="dynamapper-pkg" target="":
    @just verify-package-inputs {{target}}
    @just package {{name}} {{target}}

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
    for doc in {{app_docs}}; do
        [ -f "$doc" ] && cp "$doc" "$APP_DIR/docs/"
    done

    for util in {{tool_bins}}; do
        [ -f "$RELEASE_DIR/$util" ] && cp "$RELEASE_DIR/$util" "$TOOLS_DIR/bin/"
    done
    [ -f "tools/_shared_assets/Dictionary.dic" ] && cp "tools/_shared_assets/Dictionary.dic" "$TOOLS_DIR/shared/"
    for doc in {{tool_docs}}; do
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
    foreach ($doc in '{{app_docs}}'.Split(' ')) { if (Test-Path $doc) { Copy-Item $doc \"$appDir/docs/\" } }; \
    foreach ($util in '{{tool_bins}}'.Split(' ')) { if (Test-Path \"$releaseDir/$util.exe\") { Copy-Item \"$releaseDir/$util.exe\" \"$toolsDir/bin/\" } }; \
    if (Test-Path 'tools/_shared_assets/Dictionary.dic') { Copy-Item 'tools/_shared_assets/Dictionary.dic' \"$toolsDir/shared/\" }; \
    foreach ($doc in '{{tool_docs}}'.Split(' ')) { if (Test-Path $doc) { Copy-Item $doc \"$toolsDir/docs/\" } }; \
    Write-Host \"Packaging complete: $appDir and $toolsDir\""


# --- Development Run Recipes ---
# TODO: incomplete

# Alias for run-dynamapper-debug
run-dynamapper *args:
    @just run-dynamapper-debug "{{args}}"

# Run the main application in debug mode
run-dynamapper-debug *args:
    @echo "Running dynamapper in debug mode..."
    cargo run --bin dynamapper "{{args}}"

# Run the main application in release mode (nightly)
run-dynamapper-release *args:
    @echo "Running dynamapper in release mode (nightly Rust toolchain)..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    @echo "Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}"
    {{cargo_release_nightly}} run --release --no-default-features --features "{{linux_features}}" "{{CARGO_FLAGS_NIGHTLY}}" \
        --bin dynamapper "{{args}}"

# Run the main application in profiling mode (stable, better symbol resolution by perf)
run-dynamapper-profile *args:
    @echo "Running dynamapper in profiling mode (nightly Rust toolchain)..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    {{cargo_profile_stable}} run --profile profiling --no-default-features --features "profiling,{{linux_features}}" \
        --bin dynamapper "{{args}}"

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
    {{cargo_release_nightly}} run --release --no-default-features --features "{{linux_features}}" "{{CARGO_FLAGS_NIGHTLY}}" \
        --bin {{tool}} -- "{{args}}"

uddconv-all ccdir="" ecdir="" output_dir="target/uddp" maps="0,1,2,3,4,5":
    python scripts/uddconv/uddconv.py all --ccdir "{{ccdir}}" --ecdir "{{ecdir}}" --output-dir "{{output_dir}}" --maps "{{maps}}"

# Convert only mobile animation packages
uddconv-animations ccdir="" ecdir="" output_dir="target/uddp":
    python scripts/uddconv/uddconv.py animations --ccdir "{{ccdir}}" --ecdir "{{ecdir}}" --output-dir "{{output_dir}}"

# Convert only Classic gump packages
uddconv-gumps ccdir="" output_dir="target/uddp":
    python scripts/uddconv/uddconv.py gumps --ccdir "{{ccdir}}" --output-dir "{{output_dir}}"

# Convert only Enhanced Client gump packages
uddconv-ec-gumps ecdir="" output_dir="target/uddp":
    python scripts/uddconv/uddconv.py ec-gumps --ecdir "{{ecdir}}" --output-dir "{{output_dir}}"


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
