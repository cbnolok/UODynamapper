# UODynamapper workspace justfile
#
# This file serves as the unified entry point for building, testing, and packaging UODynamapper
# across Linux, macOS, and Windows. It replaces dozens of platform-specific shell/PowerShell scripts.
#
# --- Architecture Summary ---
# 1. Task Runner: 'just' handles OS detection, environment variable exports, and task orchestration.
# 2. Environment: All recipes export RUSTFLAGS to ensure consistent behavior.
# 3. Toolchains: We support both Stable and Nightly toolchains. Nightly is used for aggressive
#    optimizations like 'build-std' which recompiles the standard library for the target.
# 4. Linkers: On Linux, we automatically detect and use 'lld', 'mold', or 'wild' for
#    significantly faster link times.

set shell := ["bash", "-c"]
set windows-shell := ["powershell.exe", "-c"]

# --- Platform & Environment Detection ---
os := os()
is_windows := if os == "windows" { "true" } else { "false" }
is_linux := if os == "linux" { "true" } else { "false" }
is_macos := if os == "macos" { "true" } else { "false" }
# The toolchain the nightly recipes use. CI pins a dated nightly (RUST_NIGHTLY_TOOLCHAIN in release.yml), so a broken
# nightly can't fail the release builds; locally it is the current nightly.
nightly_toolchain := env_var_or_default("RUST_NIGHTLY_TOOLCHAIN", "nightly")

# --- Tool Detection ---
has_lld  := if is_linux == "true" { `command -v ld.lld || echo ""` } else { "" }
has_mold := if is_linux == "true" { `command -v mold || echo ""` } else { "" }
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
# Features for --no-default-features builds (release, profiling, run)
build_features := linux_features
# Avoids passing --features "" on platforms where build_features is empty (Windows PowerShell can't handle empty quoted args)
features_args  := if build_features != "" { "--features " + build_features } else { "" }
# -Clink-arg=-lgcc in the musl RUSTFLAGS satisfies compiler builtins like __popcountdi2 that the C dependencies may emit
linux_musl_rustflags := " -Ctarget-feature=+crt-static -Clink-self-contained=yes -Clink-arg=-lgcc"
rustflags_release_nightly_musl := ""

# Strip the local checkout path from optimized binaries. Skipped when the path contains spaces, since RUSTFLAGS
#   is split on whitespace.
workspace_dir        := justfile_directory()
remap_workspace_flag := if replace(workspace_dir, " ", "") == workspace_dir { " --remap-path-prefix=" + workspace_dir + "=." } else { "" }

# MSVC always writes separate .pdb files, and the windows-gnu targets reject split-debuginfo as unstable.
split_debuginfo_flag := if is_windows == "true" { "" } else { " -Csplit-debuginfo=packed" }
# Specialized RUSTFLAGS for different build types (exported to be accessible in shell commands)
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
app_assets_unread := "regression_fixtures" +\
    " cc_ec_convtables/TerrainTranscode.kdl cc_ec_convtables/EcTerrainOverrides.kdl" +\
    " cc_ec_convtables/EcSurfaceOverrides.kdl cc_ec_convtables/EcMobileAnimations.kdl" +\
    " cc_ec_convtables/MobileFrameFilters.kdl cc_ec_convtables/TilemetaOverrides.kdl"
app_assets_unread_ext := "md"
tool_docs := "README.md docs/USER_TROUBLESHOOTING.md tools/udd-conv-cli/UPSCALE_PROFILES.md"
# The end-user tools. uocf-devtools-cli and udd-devtools-cli stay out: opt-in research and diagnostics.
tool_bins := "udd-cli udd-conv-gui uddp-inspector-gui uo-client-patcher-gui uocf-cli uocf-editor-gui uop-dict-populator-gui uoppackage-cli webmap-export-cli"
# The data files the tools read, each copied into the tools package's shared/ folder (a folder keeps its name), where
# uocf::utils::bundled_files finds them from bin/. The CLI-only package ships them too: udd-cli reads the tables.
tool_shared := "lib/uocf/assets/Dictionary.dic lib/uocf/assets/kr/kr_delta.uddp dynamapper/assets/cc_ec_convtables"
# The static musl builds ship only these. A static musl binary can't load shared libraries (dlopen fails with "Dynamic
# loading not supported"), and dynamapper and the GUI tools load their windowing, GPU and audio libraries that way.
cli_bins := "udd-cli uocf-cli uoppackage-cli webmap-export-cli"
cli_packages := "-p udd-conv-cli -p uocf-cli -p uoppackage-cli -p webmap-export-cli"

# --- Recipes ---

# Show all available recipes and their arguments.
default:
    @just --list

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

# Build the CLI tools for a Linux musl release target with the nightly toolchain (see cli_packages).
build-linux-musl-release-nightly target *args:
    @echo "Building CLI tools release for musl target {{target}} with the nightly toolchain..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_RELEASE_NIGHTLY}}{{linux_musl_rustflags}}{{rustflags_release_nightly_musl}}"
    @echo "Using features: {{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    @echo "Adding musl RUSTFLAGS: {{linux_musl_rustflags}} {{rustflags_release_nightly_musl}}"
    RUSTFLAGS="{{RUSTFLAGS_RELEASE_NIGHTLY}}{{linux_musl_rustflags}}{{rustflags_release_nightly_musl}}" \
        cargo +{{nightly_toolchain}} build --target "{{target}}" --release --locked {{cli_packages}} --bins --no-default-features \
        {{CARGO_FLAGS_NIGHTLY}} {{args}}

# Build the full workspace with the same nightly release settings used by CI.
build-ci-workspace *args:
    @echo "Building CI workspace release on {{os}}..."
    @just build-local-workspace-release-nightly {{args}}

# Build the full workspace in profiling mode with the stable toolchain.
build-local-workspace-profile *args:
    @echo "Building workspace profiling profile with the stable toolchain on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_PROFILE_STABLE}}"
    @echo "Using features: profiling,{{build_features}}"
    {{cargo_profile_stable}} build --profile profiling --locked --workspace --no-default-features --features "profiling,{{build_features}}" {{args}}

# Build the full workspace in profiling mode with nightly build-std optimizations.
build-local-workspace-profile-nightly *args:
    @echo "Building workspace profiling profile with the nightly toolchain on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_PROFILE_NIGHTLY}}"
    @echo "Using features: profiling,{{build_features}}"
    @echo 'Adding cargo flags: {{CARGO_FLAGS_NIGHTLY}}'
    {{cargo_profile_nightly}} build --profile profiling --locked --workspace --no-default-features --features "profiling,{{build_features}}" {{CARGO_FLAGS_NIGHTLY}} {{args}}

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

# Run the Dynamapper application in profiling mode with the stable toolchain.
run-dynamapper-profile *args:
    @echo "Running dynamapper in profiling mode with the stable toolchain on {{os}}..."
    @echo "Using RUSTFLAGS: {{RUSTFLAGS_PROFILE_STABLE}}"
    @echo "Using features: profiling,{{build_features}}"
    {{cargo_profile_stable}} run --profile profiling --no-default-features --features "profiling,{{build_features}}" \
        --bin dynamapper {{args}}

# Run a named binary with the same flags/features as the workspace build recipes.
# Arguments after the binary name are passed to the program (no extra -- needed).
[positional-arguments]
run-bin-release binary *args:
    {{cargo_release_stable}} run --release --locked --no-default-features {{features_args}} --bin {{ if is_windows == "true" { '"' + binary + '"' } else { '"$1"' } }} -- {{ if is_windows == "true" { args } else { '"${@:2}"' } }}

# Measure the mobiles' walk and run strides by following the planted foot into the animation packages (`udd-cli tool
# strides measure`): `just measure-mobile-strides changed FILE...` measures only the clips whose frames changed, `full`
# all of them; `--body N` (repeatable), `--output DIR`, `--report DIR`, `--method flow` (RIFE on the GPU).
[positional-arguments]
measure-mobile-strides *args:
    just run-bin-release udd-cli tool strides measure {{ if is_windows == "true" { args } else { '"$@"' } }}

[positional-arguments]
run-bin-release-nightly binary *args:
    {{cargo_release_nightly}} run --release --locked --no-default-features {{features_args}} {{CARGO_FLAGS_NIGHTLY}} --bin {{ if is_windows == "true" { '"' + binary + '"' } else { '"$1"' } }} -- {{ if is_windows == "true" { args } else { '"${@:2}"' } }}

[positional-arguments]
run-bin-profile binary *args:
    {{cargo_profile_stable}} run --profile profiling --locked --no-default-features --features "profiling,{{build_features}}" --bin {{ if is_windows == "true" { '"' + binary + '"' } else { '"$1"' } }} -- {{ if is_windows == "true" { args } else { '"${@:2}"' } }}

[positional-arguments]
run-bin-profile-nightly binary *args:
    {{cargo_profile_nightly}} run --profile profiling --locked --no-default-features --features "profiling,{{build_features}}" {{CARGO_FLAGS_NIGHTLY}} --bin {{ if is_windows == "true" { '"' + binary + '"' } else { '"$1"' } }} -- {{ if is_windows == "true" { args } else { '"${@:2}"' } }}

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

# Validate the release inputs, then package the Linux/macOS app and tool artifacts.
[unix]
package-release name="dynamapper-pkg" target="":
    #!/usr/bin/env bash
    set -euo pipefail
    RELEASE_DIR="target/release"
    if [ -n "{{target}}" ]; then
        RELEASE_DIR="target/{{target}}/release"
    fi
    echo "Verifying release package inputs in $RELEASE_DIR..."
    missing=0
    for input in "$RELEASE_DIR/dynamapper" $(printf "$RELEASE_DIR/%s " {{tool_bins}}) {{app_docs}} {{tool_docs}}; do
        if [ ! -f "$input" ]; then
            echo "missing file: $input"
            missing=1
        fi
    done
    for input in dynamapper/assets {{tool_shared}}; do
        if [ ! -e "$input" ]; then
            echo "missing file or directory: $input"
            missing=1
        fi
    done
    [ "$missing" = 0 ] || exit 1
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

# Validate the release inputs, then package the Windows app and tool artifacts.
[windows]
[script("powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File")]
[extension(".ps1")]
package-release name="dynamapper-pkg" target="":
    $ErrorActionPreference = 'Stop'
    $releaseDir = if ('{{target}}' -ne '') { 'target/{{target}}/release' } else { 'target/release' }
    Write-Host "Verifying release package inputs in $releaseDir..."
    $inputs = @("$releaseDir/dynamapper.exe") + ('{{tool_bins}}'.Split(' ') | ForEach-Object { "$releaseDir/$_.exe" }) + '{{app_docs}} {{tool_docs}}'.Split(' ') + 'dynamapper/assets' + '{{tool_shared}}'.Split(' ')
    $missing = @($inputs | Where-Object { -not (Test-Path -Path $_) })
    if ($missing.Count -gt 0) { $missing | ForEach-Object { Write-Host "missing: $_" }; exit 1 }
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

# --- Maintenance ---

# Run all Rust tests across the workspace.
test:
    @echo "Running all workspace tests..."
    cargo test --workspace

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

# Lint the Bevy app with bevy_cli. The workspace has several binaries; dynamapper is the one Bevy crate.
# Needs `bevy lint install main`: the latest release (v0.6.0) targets Bevy 0.18, and this app is on 0.20.
bevy-lint:
    @echo "Running Bevy lints..."
    bevy lint --package dynamapper

# Check that the standalone shell scripts are portable POSIX sh (ENGINEERING.md, "Shell scripts").
check-shell:
    git ls-files -z '*.sh' | xargs -0 shellcheck -s sh

# Check that the workspace's dependencies, direct and transitive, resolve to the versions bevy and bevy_egui use
# (e.g. egui pulled by eframe), so no crate is compiled twice. Fails on a mismatch; CI runs it.
check-dep-alignment *args:
    @echo "Checking workspace dependency alignment against bevy and bevy_egui..."
    cargo metadata --format-version 1 | python3 scripts/dev-helpers/check_dep_alignment.py {{args}}

# Check workspace direct dependency versions against a selected reference crate.
check-dep-alignment-ref ref *args:
    @echo "Checking workspace dependency alignment against {{ref}}..."
    cargo metadata --format-version 1 | python3 scripts/dev-helpers/check_dep_alignment.py --reference {{ref}} {{args}}
