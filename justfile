# UODynamapper justfile

# --- General ---

# List available recipes
default:
    @just --list

# --- Linux / macOS ---

# Build in debug mode
build-debug:
    @./scripts/build/linux/build-debug.sh

# Build in release mode (nightly toolchain)
build-release:
    @./scripts/build/linux/build-release-nightly-toolchain.sh

# Build in profiling mode
build-profile:
    @./scripts/build/linux/build-profile-stable.sh

# Package the build
package name="dynamapper-linux-x86_64" target="":
    @./scripts/build/linux/package.sh {{name}} {{target}}

# --- Windows ---

# Build in debug mode (Windows)
build-debug-win:
    @pwsh ./scripts/build/windows/build-debug.ps1

# Build in release mode (Windows)
build-release-win:
    @pwsh ./scripts/build/windows/build-release-nightly-toolchain.ps1

# Package the build (Windows)
package-win name="dynamapper-windows-x86_64" target="":
    @pwsh ./scripts/build/windows/package.ps1 -ArtifactName {{name}} -TargetTriple {{target}}

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

# Lint the Bevy project using bevy_cli (requires: cargo install bevy_cli)
bevy-lint:
    bevy lint
