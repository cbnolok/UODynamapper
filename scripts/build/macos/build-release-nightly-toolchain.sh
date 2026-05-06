#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running macOS nightly release build..."

# Use sccache directly when available to avoid wrapper-induced retries.
if command -v sccache >/dev/null 2>&1; then
    export RUSTC_WRAPPER="sccache"
else
    unset RUSTC_WRAPPER
fi

# Release flags plus nightly-only size/build-std flags.
# macOS uses -Wl,-dead_strip instead of --gc-sections
# macOS doesn't support --icf (Identical Code Folding)
export RUSTFLAGS=" \
-C link-arg=-Wl,-dead_strip \
-C force-unwind-tables=no -C symbol-mangling-version=v0 \
-Z share-generics=y -Z location-detail=none \
${RUSTFLAGS:-}"

build_args=(
    +nightly build --release --locked --no-default-features
    --bin dynamapper --package dynamapper
    -Z build-std=std,panic_abort
    -Z build-std-features=optimize_for_size
)

if [[ -n "${CARGO_FEATURES:-}" ]]; then
    build_args+=(--features "$CARGO_FEATURES")
fi

# Build main application
echo "Building dynamapper..."
cargo "${build_args[@]}" "$@"

# Build utilities
echo "Building utilities (uddp_inspector, uddconv_cli, uddconv_gui, uocf_cli)..."
cargo +nightly build --release --locked \
    --package uddp_inspector \
    --package uddconv_cli \
    --package uddconv_gui \
    --package uocf_cli \
    -Z build-std=std,panic_abort \
    -Z build-std-features=optimize_for_size \
    "$@"

echo "Build complete."
