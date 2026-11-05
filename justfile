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
# 5. Linkers: On Linux, we automatically detect and use 'lld', 'mold', or 'wild' for
#    significantly faster link times.

set shell := ["bash", "-c"]
set windows-shell := ["powershell.exe", "-c"]

# --- Platform & Environment Detection ---
os := os()
is_windows := if os == "windows" { "true" } else { "false" }
is_linux := if os == "linux" { "true" } else { "false" }
is_macos := if os == "macos" { "true" } else { "false" }
is_ci := env_var_or_default("GITHUB_ACTIONS", "false")
RUSTFLAGS := env_var_or_default("RUSTFLAGS", "")
# The toolchain the nightly recipes use. CI pins a dated nightly (RUST_NIGHTLY_TOOLCHAIN in release.yml), so a broken
# nightly can't fail the release builds; locally it is the current nightly.
nightly_toolchain := env_var_or_default("RUST_NIGHTLY_TOOLCHAIN", "nightly")

# --- Features ---
# Enable sccache explicitly. If "true", it will bypass the wrapper and use sccache directly.
sccache := "false"

# --- Tool Detection ---
has_sccache := if is_windows == "true" {
    `powershell -NoProfile -Command "if (Get-Command sccache -ErrorAction SilentlyContinue) { (Get-Command sccache).Source }"`
} else {
    `command -v sccache || echo ""`
}
has_lld  := if is_linux == "true" { `command -v ld.lld || echo ""` } else { "" }
has_mold := if is_linux == "true" { `command -v mold || echo ""` } else { "" }
has_wild := if is_linux == "true" { `command -v wild || echo ""` } else { "" }
rust_llvm_major_stable := if is_linux == "true" {
    `rustc -vV 2>/dev/null |
 sed -n 's/^LLVM version: \([0-9][0-9]*\).*/\1/p' | head -n1 || true`
} else {
    ""
}
rust_llvm_major_nightly := if is_linux == "true" {
    `rustc +"${RUST_NIGHTLY_TOOLCHAIN:-nightly}" -vV 2>/dev/null | sed -n 's/^LLVM version: \([0-9][0-9]*\).*/\1/p' | head -n1 || true`
} else {
    ""
}
lld_llvm_major := if has_lld != "" {
    `ld.lld --version 2>/dev/null | sed -n 's/.*LLD \([0-9][0-9]*\).*/\1/p' | head -n1 || true`
} else {
    ""
}

# --- Build Configuration ---

# RUSTC_WRAPPER logic (needed for sccache)
wrapper_path := "scripts/build/common/rustc_wrapper.sh"

# Determine if sccache should be used: on CI by default. The Windows CI jobs pin sccache 0.16, since later releases
#   expand cargo's @argfile past Windows' command line limit (see release.yml).
sccache_requested := if sccache == "true" { "true" } else { is_ci }
sccache_available := if has_sccache != "" { "true" } else { "false" }
sccache_effective := if sccache_requested == "true" { sccache_available } else { "false" }

# Windows runs without a wrapper unless sccache is on: its pass-through wrapper goes through PowerShell's -File
#   argument binding, which splits `--remap-path-prefix=D:\…=.` at the drive colon, and starts pwsh for every rustc
#   call. Cargo treats an empty RUSTC_WRAPPER as no wrapper.
export RUSTC_WRAPPER := if sccache_effective == "true" { "sccache" } else if is_windows == "true" { "" } else { wrapper_path }

# Determine linker to be used and to be passed to RUSTFLAGS
# For now linux_debug_linker_base is unused, since we are relying on config.toml for rapid iteration debug
#   builds with stable rust toolchain
linux_debug_linker_base := \
if has_wild != "" {
    " -Clink-arg=-fuse-ld=wild"
} else if has_mold != "" {
    " -Clink-arg=-fuse-ld=mold"
} else if has_lld != "" {
    " -Clink-arg=-fuse-ld=lld"
} else {
    ""
}
# lld can only read Rust LLVM bitcode for linker-plugin LTO when both use the same LLVM major version.
lld_plugin_lto_stable := if lld_llvm_major == "" {
    ""
} else if rust_llvm_major_stable == "" {
    ""
} else if lld_llvm_major == rust_llvm_major_stable {
    " -Clinker-plugin-lto"
} else {
    ""
}
lld_plugin_lto_nightly := if lld_llvm_major == "" {
    ""
} else if rust_llvm_major_nightly == "" {
    ""
} else if lld_llvm_major == rust_llvm_major_nightly {
    " -Clinker-plugin-lto"
} else {
    ""
}

linux_optimized_linker_base_stable := \
if has_lld != "" {
    " -Clink-arg=-fuse-ld=lld" + lld_plugin_lto_stable
} else if has_mold != "" {
    " -Clink-arg=-fuse-ld=mold"
} else {
    ""
}
linux_optimized_linker_base_nightly := \
if has_mold != "" {
    " -Clink-arg=-fuse-ld=mold"
} else if has_lld != "" {
    " -Clink-arg=-fuse-ld=lld" + lld_plugin_lto_nightly
} else {
    ""
}

# Linker flags per build
linker_debug_flags := if is_linux == "true" {
    linux_debug_linker_base
} else {
    ""
}
linker_optimized_flags_stable := if is_linux == "true" {
    linux_optimized_linker_base_stable +\
    " -Clink-arg=-Wl,--gc-sections -Clink-arg=-Wl,--no-allow-shlib-undefined"
} else if is_macos == "true" {
    " -Clink-arg=-Wl,-dead_strip"
} else {
  ""
}
linker_optimized_flags_nightly := if is_linux == "true" {
    linux_optimized_linker_base_nightly +\
    " -Clink-arg=-Wl,--gc-sections -Clink-arg=-Wl,--no-allow-shlib-undefined"
} else if is_macos == "true" {
    " -Clink-arg=-Wl,-dead_strip"
} else {
  ""
}

# Features to enable on Linux by default (ensures Wayland/X11 support when using --no-default-features)
linux_features := if is_linux == "true" { "linux_wayland,linux_x11" } else { "" }
# Set VENDORED_JXL=true to build libjxl from source (required on systems without libjxl-dev installed).
# Defaults to true on CI where libjxl-dev is not installed.
jxl_vendored      := env_var_or_default("VENDORED_JXL", is_ci)
jxl_vendored_feat := if jxl_vendored == "true" { "udd-container/vendored-jpegxl" } else { "" }
# Combined features for --no-default-features builds (release, profiling, run)
# NOTE: just (1.51) conditions support only a single comparison — no && / || chaining,
# even parenthesized — so this joins the two feature lists via nested ifs.
build_features := \
    if linux_features != "" { \
        if jxl_vendored_feat != "" { linux_features + "," + jxl_vendored_feat } \
        else { linux_features } \
    } else { jxl_vendored_feat }
# Musl targets always require vendored libjxl (no system package available)
musl_features := "udd-container/vendored-jpegxl"
# Avoids passing --features "" on platforms where build_features is empty (Windows PowerShell can't handle empty quoted args)
features_args  := if build_features != "" { "--features " + build_features } else { "" }
# Added -Clink-arg=-lgcc to musl RUSTFLAGS to satisfy compiler builtins like __popcountdi2 emitted by vendored libjxl C++ objects
linux_musl_rustflags := " -Ctarget-feature=+crt-static -Clink-self-contained=yes -Clink-arg=-lgcc"
rustflags_release_stable_musl := ""
rustflags_release_nightly_musl := ""
rustflags_profile_stable_musl := ""
rustflags_profile_nightly_musl := ""

# Strip the local checkout path from optimized binaries. Skipped when the path contains spaces, since RUSTFLAGS
#   is split on whitespace.
workspace_dir        := justfile_directory()
remap_workspace_flag := if replace(workspace_dir, " ", "") == workspace_dir { " --remap-path-prefix=" + workspace_dir + "=." } else { "" }

# MSVC always writes separate .pdb files, and the windows-gnu targets reject split-debuginfo as unstable.
split_debuginfo_flag := if is_windows == "true" { "" } else { " -Csplit-debuginfo=packed" }
# Specialized RUSTFLAGS for different build types (exported to be accessible in shell commands)
rustflags_debug_common              := linker_debug_flags + split_debuginfo_flag + " -Csymbol-mangling-version=v0 -Cembed-bitcode=no"   # llvm bitcode is unneeded since we are not using LTO in debug
rustflags_optimized_common_stable   := linker_optimized_flags_stable  + remap_workspace_flag + split_debuginfo_flag + " -Csymbol-mangling-version=v0"
rustflags_optimized_common_nightly  := linker_optimized_flags_nightly + remap_workspace_flag + split_debuginfo_flag + " -Csymbol-mangling-version=v0" +\
                                        " -Zshare-generics=y -Zlocation-detail=none"
# Windows targets require unwind tables: rustc rejects -Cforce-unwind-tables=no there.
no_unwind_tables_flag               := if is_windows == "true" { "" } else { " -Cforce-unwind-tables=no" }
export RUSTFLAGS_RELEASE_STABLE     := rustflags_optimized_common_stable  + no_unwind_tables_flag
export RUSTFLAGS_RELEASE_NIGHTLY    := rustflags_optimized_common_nightly + no_unwind_tables_flag
export RUSTFLAGS_PROFILE_STABLE     := rustflags_optimized_common_stable  +\
                                        " -Cforce-frame-pointers=yes -Cembed-bitcode=no"
export RUSTFLAGS_PROFILE_NIGHTLY    := rustflags_optimized_common_nightly +\
                                        " -Cforce-frame-pointers=yes -Cembed-bitcode=no"
# Specialized cargo flags
# On Windows/PowerShell, commas are array separators, so -Zbuild-std=std,panic_abort must be quoted.
export CARGO_FLAGS_NIGHTLY := if is_windows == "true" {
    ' "-Zbuild-std=std,panic_abort" -Zbuild-std-features=optimize_for_size'
} else {
    " -Zbuild-std=std,panic_abort -Zbuild-std-features=optimize_for_size"
}
#   -Zfmt-debug=shallow
#   -Zembed-metadata=no --emit=metadata to produce the full metadata into a separate .rmeta file.

# Cross-platform Cargo runners to properly inject RUSTFLAGS in the shell
cargo_release_nightly   :=\
    if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_RELEASE_NIGHTLY; cargo +" + nightly_toolchain } else { "export RUSTFLAGS=\"$RUSTFLAGS_RELEASE_NIGHTLY\"; cargo +" + nightly_toolchain }
cargo_release_stable    :=\
    if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_RELEASE_STABLE; cargo" }           else { "export RUSTFLAGS=\"$RUSTFLAGS_RELEASE_STABLE\"; cargo" }
cargo_profile_nightly   :=\
    if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_PROFILE_NIGHTLY; cargo +" + nightly_toolchain } else { "export RUSTFLAGS=\"$RUSTFLAGS_PROFILE_NIGHTLY\"; cargo +" + nightly_toolchain }
cargo_profile_stable    :=\
    if is_windows == "true" { "$env:RUSTFLAGS=$env:RUSTFLAGS_PROFILE_STABLE; cargo" }           else { "export RUSTFLAGS=\"$RUSTFLAGS_PROFILE_STABLE\"; cargo" }

# --- Release Package Contents ---
# The app package: UODynamapper/ holds dynamapper, the assets/ tree it reads (settings, shaders, fonts, the KDL
# tables), README.md and docs/. The tools package: UODynamapper-tools/ holds bin/, shared/ and docs/.
app_docs := "docs/keybindings.md docs/USER_TROUBLESHOOTING.md"
# Left out of the app's assets/ copy, as nothing reads them at run time: these folders and files below assets/ (the
# conversion tables are read by the packers, which embed what the app needs in the .uddp files; the tools package
# ships them in shared/), and the notes (the files with this extension, beside the shaders). A folder they leave
# empty goes too.
app_assets_unread := "regression_fixtures shaders/postprocess_ec_doc shaders/postprocess_kr_doc" +\
    " cc_ec_convtables/TerrainTranscode.kdl cc_ec_convtables/EcTerrainOverrides.kdl" +\
    " cc_ec_convtables/EcSurfaceOverrides.kdl cc_ec_convtables/EcMobileAnimations.kdl"
app_assets_unread_ext := "md"
tool_docs := "README.md docs/USER_TROUBLESHOOTING.md tools/udd-conv-cli/UPSCALE_PROFILES.md"
# The end-user tools. uocf-devtools-cli stays out: research and diagnostic commands, built only with dev-tool-tests.
tool_bins := "udd-cli udd-conv-gui uddp-inspector-gui uo-client-patcher-gui uocf-cli uocf-editor-gui uop-dict-populator-gui uoppackage-cli webmap-export-cli"
tool_packages := "-p udd-conv-cli -p uocf-cli -p udd-conv-gui -p uddp-inspector-gui -p uo-client-patcher-gui -p uocf-editor-gui -p uop-dict-populator-gui -p uoppackage-cli -p webmap-export-cli"
# The data files the tools read, each copied into the tools package's shared/ folder (a folder keeps its name), where
# uocf::utils::bundled_files finds them from bin/. The CLI-only package ships them too: udd-cli reads the tables.
tool_shared := "tools/_shared_assets/Dictionary.dic dynamapper/assets/cc_ec_convtables"
# The static musl builds ship only these. A static musl binary can't load shared libraries (dlopen fails with "Dynamic
# loading not supported"), and dynamapper and the GUI tools load their windowing, GPU and audio libraries that way.
cli_bins := "udd-cli uocf-cli uoppackage-cli webmap-export-cli"
cli_packages := "-p udd-conv-cli -p uocf-cli -p uoppackage-cli -p webmap-export-cli"

# --- Recipes ---

# Show all available recipes and their arguments.
default:
    @just --list

# Build the full workspace in debug mode for fast local iteration.
build-local-workspace-debug *args:
    @echo "Building workspace in debug mode on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    cargo build --workspace {{args}}

# Build only the Dynamapper application in debug mode.
build-local-dynamapper-debug *args:
    @echo "Building dynamapper debug binary on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    cargo build -p dynamapper --bin dynamapper {{args}}

# Build all shipped tool binaries in debug mode.
build-local-tools-debug *args:
    @echo "Building shipped tool binaries in debug mode on {{os}}..."
    @echo "Packages: {{tool_packages}}"
    cargo build {{tool_packages}} --bins {{args}}

# Build the udd-pack CLI binary in debug mode.
build-local-udd-pack-debug *args:
    @echo "Building udd-pack debug binary on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    cargo build -p udd-conv-cli --bin udd-pack {{args}}

# Build the full workspace in release mode with the stable toolchain.
build-local-workspace-release *args:
    @echo "Building workspace release with the stable toolchain on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_STABLE}}"
    @echo "Using features: {{build_features}}"
    {{cargo_release_stable}} build --release --locked --workspace --no-default-features {{features_args}} {{args}}

# Build the full workspace in release mode with nightly build-std optimizations.
build-local-workspace-release-nightly *args:
    @echo "Building workspace release with the nightly toolchain on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_NIGHTLY}}"
    @echo "Using features: {{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    {{cargo_release_nightly}} build --release --locked --workspace --no-default-features {{features_args}} {{CARGO_FLAGS_NIGHTLY}} {{args}}

# Build the CLI tools for a Linux musl release target with the stable toolchain (see cli_packages).
build-linux-musl-release target *args:
    @echo "Building CLI tools release for musl target {{target}} with the stable toolchain..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_STABLE}}{{linux_musl_rustflags}}{{rustflags_release_stable_musl}}"
    @echo "Using features: {{build_features}}"
    @echo "Adding musl RUSTFLAGS: {{linux_musl_rustflags}} {{rustflags_release_stable_musl}}"
    RUSTFLAGS="{{RUSTFLAGS_RELEASE_STABLE}}{{linux_musl_rustflags}}{{rustflags_release_stable_musl}}" \
        cargo build --target "{{target}}" --release --locked {{cli_packages}} --bins --no-default-features --features \
        "{{musl_features}}" {{args}}

# Build the CLI tools for a Linux musl release target with the nightly toolchain (see cli_packages).
build-linux-musl-release-nightly target *args:
    @echo "Building CLI tools release for musl target {{target}} with the nightly toolchain..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_NIGHTLY}}{{linux_musl_rustflags}}{{rustflags_release_nightly_musl}}"
    @echo "Using features: {{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    @echo "Adding musl RUSTFLAGS: {{linux_musl_rustflags}} {{rustflags_release_nightly_musl}}"
    RUSTFLAGS="{{RUSTFLAGS_RELEASE_NIGHTLY}}{{linux_musl_rustflags}}{{rustflags_release_nightly_musl}}" \
        cargo +{{nightly_toolchain}} build --target "{{target}}" --release --locked {{cli_packages}} --bins --no-default-features --features \
        "{{musl_features}}" {{CARGO_FLAGS_NIGHTLY}} {{args}}

# Build the full workspace with the same nightly release settings used by CI.
build-ci-workspace *args:
    @echo "Building CI workspace release on {{os}}..."
    @just build-local-workspace-release-nightly {{args}}

# Build only the Dynamapper application with CI nightly release settings.
build-ci-dynamapper *args:
    @echo "Building CI dynamapper release on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_NIGHTLY}}"
    @echo "Using features: {{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    {{cargo_release_nightly}} build --release --locked --no-default-features {{features_args}} {{CARGO_FLAGS_NIGHTLY}} \
        -p dynamapper --bin dynamapper {{args}}

# Build all shipped tools with CI nightly release settings.
build-ci-tools *args:
    @echo "Building CI tool releases on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_NIGHTLY}}"
    @echo "Using features: {{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    {{cargo_release_nightly}} build --release --locked --no-default-features {{features_args}} {{CARGO_FLAGS_NIGHTLY}} \
        {{tool_packages}} --bins {{args}}

# Build the Dynamapper app and shipped tools used in release artifacts.
build-ci-shipping *args:
    @echo "Building CI shipping binaries: dynamapper plus shipped tools..."
    @just build-ci-dynamapper {{args}}
    @just build-ci-tools {{args}}

# Build the full workspace in profiling mode with the stable toolchain.
build-local-workspace-profile *args:
    @echo "Building workspace profiling profile with the stable toolchain on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_PROFILE_STABLE}}"
    @echo "Using features: profiling,{{build_features}}"
    {{cargo_profile_stable}} build --profile profiling --locked --workspace --no-default-features --features "profiling,{{build_features}}" {{args}}

# Build the CLI tools for a Linux musl profiling target with the stable toolchain (see cli_packages).
build-linux-musl-profile target *args:
    @echo "Building CLI tools profiling profile for musl target {{target}} with the stable toolchain..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_PROFILE_STABLE}}{{linux_musl_rustflags}}{{rustflags_profile_stable_musl}}"
    @echo "Using features: profiling,{{build_features}}"
    @echo "Adding musl RUSTFLAGS: {{linux_musl_rustflags}} {{rustflags_profile_stable_musl}}"
    RUSTFLAGS="{{RUSTFLAGS_PROFILE_STABLE}}{{linux_musl_rustflags}}{{rustflags_profile_stable_musl}}" \
        cargo build --target "{{target}}" --profile profiling --locked {{cli_packages}} --bins --no-default-features --features \
        "uocf/profiling,{{musl_features}}" {{args}}

# Build the full workspace in profiling mode with nightly build-std optimizations.
build-local-workspace-profile-nightly *args:
    @echo "Building workspace profiling profile with the nightly toolchain on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_PROFILE_NIGHTLY}}"
    @echo "Using features: profiling,{{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    {{cargo_profile_nightly}} build --profile profiling --locked --workspace --no-default-features --features "profiling,{{build_features}}" {{CARGO_FLAGS_NIGHTLY}} {{args}}

# Build the CLI tools for a Linux musl profiling target with the nightly toolchain (see cli_packages).
build-linux-musl-profile-nightly target *args:
    @echo "Building CLI tools profiling profile for musl target {{target}} with the nightly toolchain..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_PROFILE_NIGHTLY}}{{linux_musl_rustflags}}{{rustflags_profile_nightly_musl}}"
    @echo "Using features: profiling,{{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    @echo "Adding musl RUSTFLAGS: {{linux_musl_rustflags}} {{rustflags_profile_nightly_musl}}"
    RUSTFLAGS="{{RUSTFLAGS_PROFILE_NIGHTLY}}{{linux_musl_rustflags}}{{rustflags_profile_nightly_musl}}" \
        cargo +{{nightly_toolchain}} build --target "{{target}}" --profile profiling --locked {{cli_packages}} --bins --no-default-features --features \
        "uocf/profiling,{{musl_features}}" {{CARGO_FLAGS_NIGHTLY}} {{args}}

# Generate a Dynamapper SVG flamegraph with cargo-flamegraph and nightly profiling settings.
profile-dynamapper-flamegraph *args:
    @echo "Generating dynamapper flamegraph on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_PROFILE_NIGHTLY}}"
    @echo "Using features: profiling,{{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    {{cargo_profile_nightly}} flamegraph --profile profiling --no-default-features --features "profiling,{{build_features}}" {{CARGO_FLAGS_NIGHTLY}} \
        --bin dynamapper --package dynamapper {{args}}

# Analyze release binary size with cargo-bloat and stable release settings.
bloat-stable *args:
    @echo "Running cargo-bloat stable release analysis on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_STABLE}}"
    @echo "Using features: {{build_features}}"
    {{cargo_release_stable}} bloat --profile profiling --no-default-features --features "profiling,{{build_features}}" \
        --config 'profile.profiling.strip=false' --config 'profile.profiling.debug=true' \
        {{args}}

# Analyze release binary size with cargo-bloat and nightly release settings.
bloat-nightly *args:
    @echo "Running cargo-bloat nightly release analysis on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_NIGHTLY}}"
    @echo "Using features: {{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    {{cargo_release_nightly}} bloat --profile profiling --no-default-features --features "profiling,{{build_features}}" {{CARGO_FLAGS_NIGHTLY}} \
        --config 'profile.profiling.strip=false' --config 'profile.profiling.debug=true' \
        {{args}}

# Verify Linux/macOS release package inputs before creating artifacts.
[unix]
verify-package-inputs target="":
    #!/usr/bin/env bash
    set -euo pipefail
    RELEASE_DIR="target/release"
    if [ -n "{{target}}" ]; then
        RELEASE_DIR="target/{{target}}/release"
    fi
    echo "Verifying release package inputs in $RELEASE_DIR..."
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
    require_path() {
        if [ ! -e "$1" ]; then
            echo "missing file or directory: $1"
            missing=1
        fi
    }

    require_file "$RELEASE_DIR/dynamapper"
    for util in {{tool_bins}}; do
        require_file "$RELEASE_DIR/$util"
    done
    require_dir "dynamapper/assets"
    for item in {{tool_shared}}; do
        require_path "$item"
    done
    for doc in {{app_docs}} {{tool_docs}}; do
        require_file "$doc"
    done
    exit "$missing"

# Verify Windows release package inputs before creating artifacts.
[windows]
[script("powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File")]
[extension(".ps1")]
verify-package-inputs target="":
    $releaseDir = if ('{{target}}' -ne '') { 'target/{{target}}/release' } else { 'target/release' }
    Write-Host "Verifying release package inputs in $releaseDir..."
    $missing = $false
    function Require-File([string]$path) { if (-not (Test-Path -Path $path -PathType Leaf)) { Write-Host "missing file: $path"; $script:missing = $true } }
    function Require-Dir([string]$path) { if (-not (Test-Path -Path $path -PathType Container)) { Write-Host "missing directory: $path"; $script:missing = $true } }
    function Require-Path([string]$path) { if (-not (Test-Path -Path $path)) { Write-Host "missing file or directory: $path"; $script:missing = $true } }
    Require-File "$releaseDir/dynamapper.exe"
    foreach ($util in '{{tool_bins}}'.Split(' ')) { Require-File "$releaseDir/$util.exe" }
    Require-Dir 'dynamapper/assets'
    foreach ($item in '{{tool_shared}}'.Split(' ')) { Require-Path $item }
    foreach ($doc in ('{{app_docs}} {{tool_docs}}'.Split(' '))) { Require-File $doc }
    if ($missing) { exit 1 }

# Validate and package the CLI tools artifact of a CLI-only build (the musl targets).
[unix]
package-release-cli name="dynamapper-pkg" target="":
    #!/usr/bin/env bash
    set -euo pipefail
    RELEASE_DIR="target/release"
    if [ -n "{{target}}" ]; then
        RELEASE_DIR="target/{{target}}/release"
    fi
    TOOLS_DIR="artifact/{{name}}-tools/UODynamapper-tools"
    missing=0
    for input in $(printf "$RELEASE_DIR/%s " {{cli_bins}}) {{tool_docs}}; do
        if [ ! -f "$input" ]; then
            echo "missing file: $input"
            missing=1
        fi
    done
    for input in {{tool_shared}}; do
        if [ ! -e "$input" ]; then
            echo "missing file or directory: $input"
            missing=1
        fi
    done
    [ "$missing" = 0 ] || exit 1
    rm -rf "artifact/{{name}}-tools"
    mkdir -p "$TOOLS_DIR/bin" "$TOOLS_DIR/shared" "$TOOLS_DIR/docs"
    for util in {{cli_bins}}; do
        cp "$RELEASE_DIR/$util" "$TOOLS_DIR/bin/"
    done
    for item in {{tool_shared}}; do
        cp -r "$item" "$TOOLS_DIR/shared/"
    done
    for doc in {{tool_docs}}; do
        cp "$doc" "$TOOLS_DIR/docs/"
    done
    echo "Packaging complete: $TOOLS_DIR"

# Validate release inputs, then package app and tool artifacts.
package-release name="dynamapper-pkg" target="":
    @echo "Creating checked release package '{{name}}' from target '{{target}}'..."
    @just verify-package-inputs {{target}}
    @just package {{name}} {{target}}

# Package Linux/macOS app and tool release artifacts.
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
    for unread in {{app_assets_unread}}; do
        rm -rf "$APP_DIR/assets/$unread"
    done
    find "$APP_DIR/assets" -type f -name '*.{{app_assets_unread_ext}}' -delete
    find "$APP_DIR/assets" -type d -empty -delete
    [ -f "README.md" ] && cp "README.md" "$APP_DIR/"
    mkdir -p "$APP_DIR/docs"
    for doc in {{app_docs}}; do
        [ -f "$doc" ] && cp "$doc" "$APP_DIR/docs/"
    done

    for util in {{tool_bins}}; do
        [ -f "$RELEASE_DIR/$util" ] && cp "$RELEASE_DIR/$util" "$TOOLS_DIR/bin/"
    done
    for item in {{tool_shared}}; do
        [ -e "$item" ] && cp -r "$item" "$TOOLS_DIR/shared/"
    done
    for doc in {{tool_docs}}; do
        [ -f "$doc" ] && cp "$doc" "$TOOLS_DIR/docs/"
    done
    echo "Packaging complete: $APP_DIR and $TOOLS_DIR"

# Package Windows app and tool release artifacts.
[windows]
[script("powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File")]
[extension(".ps1")]
package name="dynamapper-pkg" target="":
    $ErrorActionPreference = 'Stop'
    $releaseDir = if ('{{target}}' -ne '') { 'target/{{target}}/release' } else { 'target/release' }
    $appDir = 'artifact/{{name}}/UODynamapper'
    $toolsDir = 'artifact/{{name}}-tools/UODynamapper-tools'
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue 'artifact/{{name}}', 'artifact/{{name}}-tools'
    New-Item -ItemType Directory -Force -Path "$appDir", "$appDir/docs", "$toolsDir/bin", "$toolsDir/shared", "$toolsDir/docs" | Out-Null
    Write-Host "Packaging {{name}} from $releaseDir..."
    if (Test-Path "$releaseDir/dynamapper.exe") { Copy-Item "$releaseDir/dynamapper.exe" "$appDir/" }
    Copy-Item -Recurse 'dynamapper/assets' "$appDir/assets"
    foreach ($unread in '{{app_assets_unread}}'.Split(' ')) { if (Test-Path "$appDir/assets/$unread") { Remove-Item -Recurse -Force "$appDir/assets/$unread" } }
    Get-ChildItem -Path "$appDir/assets" -Recurse -File -Filter '*.{{app_assets_unread_ext}}' | Remove-Item -Force
    # Deepest first, so a folder holding only empty folders goes too.
    Get-ChildItem -Path "$appDir/assets" -Recurse -Directory | Sort-Object { $_.FullName.Length } -Descending | Where-Object { -not (Get-ChildItem -LiteralPath $_.FullName -Force) } | Remove-Item -Force
    if (Test-Path 'README.md') { Copy-Item 'README.md' "$appDir/" }
    foreach ($doc in '{{app_docs}}'.Split(' ')) { if (Test-Path $doc) { Copy-Item $doc "$appDir/docs/" } }
    foreach ($util in '{{tool_bins}}'.Split(' ')) { if (Test-Path "$releaseDir/$util.exe") { Copy-Item "$releaseDir/$util.exe" "$toolsDir/bin/" } }
    foreach ($item in '{{tool_shared}}'.Split(' ')) { if (Test-Path $item) { Copy-Item -Recurse $item "$toolsDir/shared/" } }
    foreach ($doc in '{{tool_docs}}'.Split(' ')) { if (Test-Path $doc) { Copy-Item $doc "$toolsDir/docs/" } }
    Write-Host "Packaging complete: $appDir and $toolsDir"


# --- Development Run Recipes ---

# Alias for running Dynamapper in debug mode.
run-dynamapper *args:
    @echo "Delegating to run-dynamapper-debug..."
    @just run-dynamapper-debug {{args}}

# Run the Dynamapper application in debug mode.
run-dynamapper-debug *args:
    @echo "Running dynamapper in debug mode on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    cargo run --bin dynamapper {{args}}

# Run the Dynamapper application in release mode with nightly build-std settings.
run-dynamapper-release *args:
    @echo "Running dynamapper in release mode with the nightly toolchain on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_NIGHTLY}}"
    @echo "Using features: {{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    {{cargo_release_nightly}} run --release --no-default-features --features "{{build_features}}" {{CARGO_FLAGS_NIGHTLY}} \
        --bin dynamapper {{args}}

# Run the Dynamapper application in profiling mode with the stable toolchain.
run-dynamapper-profile *args:
    @echo "Running dynamapper in profiling mode with the stable toolchain on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_PROFILE_STABLE}}"
    @echo "Using features: profiling,{{build_features}}"
    {{cargo_profile_stable}} run --profile profiling --no-default-features --features "profiling,{{build_features}}" \
        --bin dynamapper {{args}}

# Alias for running any workspace tool in debug mode.
run-tool tool *args:
    @echo "Delegating to run-tool-debug for {{tool}}..."
    @just run-tool-debug {{tool}} {{args}}

# Run any workspace tool in debug mode.
run-tool-debug tool *args:
    @echo "Running tool {{tool}} in debug mode on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS}}"
    cargo run --bin {{tool}} -- {{args}}

# Run any workspace tool in release mode with nightly build-std settings.
run-tool-release tool *args:
    @echo "Running tool {{tool}} in release mode with the nightly toolchain on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_NIGHTLY}}"
    @echo "Using features: {{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    {{cargo_release_nightly}} run --release --no-default-features --features "{{linux_features}}" {{CARGO_FLAGS_NIGHTLY}} \
        --bin {{tool}} -- {{args}}

# Convert all supported Classic and Enhanced Client assets into UDDP packages.
uddconv-all ccdir="" ecdir="" output_dir="target/uddp" maps="0,1,2,3,4,5":
    @echo "Converting all UO assets to UDDP packages in {{output_dir}}..."
    @echo "Classic Client: {{ccdir}}"
    @echo "Enhanced Client: {{ecdir}}"
    @echo "Maps: {{maps}}"
    python scripts/uddconv/uddconv.py all --ccdir "{{ccdir}}" --ecdir "{{ecdir}}" --output-dir "{{output_dir}}" --maps "{{maps}}"

# Convert only mobile animation packages into UDDP.
uddconv-animations ccdir="" ecdir="" output_dir="target/uddp":
    @echo "Converting mobile animations to UDDP packages in {{output_dir}}..."
    @echo "Classic Client: {{ccdir}}"
    @echo "Enhanced Client: {{ecdir}}"
    python scripts/uddconv/uddconv.py animations --ccdir "{{ccdir}}" --ecdir "{{ecdir}}" --output-dir "{{output_dir}}"

# Convert only Classic Client gumps into UDDP.
uddconv-gumps ccdir="" output_dir="target/uddp":
    @echo "Converting Classic Client gumps to UDDP packages in {{output_dir}}..."
    @echo "Classic Client: {{ccdir}}"
    python scripts/uddconv/uddconv.py gumps --ccdir "{{ccdir}}" --output-dir "{{output_dir}}"

# Convert only Enhanced Client gumps into UDDP.
uddconv-ec-gumps ecdir="" output_dir="target/uddp":
    @echo "Converting Enhanced Client gumps to UDDP packages in {{output_dir}}..."
    @echo "Enhanced Client: {{ecdir}}"
    python scripts/uddconv/uddconv.py ec-gumps --ecdir "{{ecdir}}" --output-dir "{{output_dir}}"


# --- Maintenance ---

# Run all Rust tests across the workspace.
test:
    @echo "Running all workspace tests..."
    cargo test --workspace

# Rebuild the debug workspace, tests and dynamapper, then delete the debug artifacts none of them uses.
sweep-debug *args:
    python3 scripts/build/sweep_debug_target.py {{args}}

# Remove Cargo and release packaging build artifacts.
clean:
    @echo "Removing Cargo build artifacts and artifact/ packages..."
    cargo clean
    rm -rf artifact/

# Format all Rust code with cargo fmt.
fmt:
    @echo "Formatting Rust code across the workspace..."
    cargo fmt --all

# Run Clippy across all workspace targets with warnings denied.
clippy:
    @echo "Running Clippy across all workspace targets..."
    cargo clippy --workspace --all-targets -- -D warnings

# Lint the Bevy app with bevy_cli.
bevy-lint:
    @echo "Running Bevy lints..."
    bevy lint

# Check that the standalone shell scripts are portable POSIX sh (ENGINEERING.md, "Shell scripts").
check-shell:
    git ls-files -z '*.sh' | xargs -0 shellcheck -s sh

# Check that the workspace's dependencies, direct and transitive, resolve to the versions bevy and bevy_egui use
# (e.g. egui pulled by eframe), so no crate is compiled twice. Fails on a mismatch; CI runs it.
# Both recipes run cargo without RUSTC_WRAPPER: they compile nothing, and on CI the wrapper is sccache, whose server
# would start inside the pipe, inherit it and keep it open, so python3 would wait for the end of input forever.
check-dep-alignment *args:
    @echo "Checking workspace dependency alignment against bevy and bevy_egui..."
    RUSTC_WRAPPER= cargo metadata --format-version 1 | RUSTC_WRAPPER= python3 scripts/dev-helpers/check_dep_alignment.py {{args}}

# Check workspace direct dependency versions against a selected reference crate.
check-dep-alignment-ref ref *args:
    @echo "Checking workspace dependency alignment against {{ref}}..."
    RUSTC_WRAPPER= cargo metadata --format-version 1 | RUSTC_WRAPPER= python3 scripts/dev-helpers/check_dep_alignment.py --reference {{ref}} {{args}}
