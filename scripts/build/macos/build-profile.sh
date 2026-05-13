#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running macOS profile build (release optimizations + debug symbols, no LTO)..."

# Set RUSTC_WRAPPER to the wrapper script
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

# Profile build: release optimizations with debug symbols for profilers.
# - No LTO: makes profilers more precise
# - Debug symbols: included for profiling
# - No strip: keep symbols
# - force-frame-pointers: essential for profilers
export RUSTFLAGS=" \
-C force-frame-pointers=yes \
${RUSTFLAGS:-}"

cargo build --profile profiling --locked --workspace --no-default-features --features profiling \
    "$@"

echo "Profile build complete."
