#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running macOS nightly release build..."

# Set RUSTC_WRAPPER to the wrapper script
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

# Stable release flags plus nightly-only size/build-std flags.
# macOS uses -Wl,-dead_strip instead of --gc-sections
# macOS doesn't support --icf (Identical Code Folding)
RUSTFLAGS=" \
-C link-arg=-Wl,-dead_strip \
-C force-unwind-tables=no -C symbol-mangling-version=v0 \
-Z share-generics=y -Z location-detail=none \
${RUSTFLAGS:-}" \
cargo +nightly build --release --locked --no-default-features \
    --bin dynamapper --package dynamapper \
    -Z build-std=std,panic_abort \
    -Z build-std-features=optimize_for_size \
    "$@"

echo "Build complete."
