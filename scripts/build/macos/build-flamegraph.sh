#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running macOS flamegraph build (release optimizations + debug symbols, frame pointers)..."

# Set RUSTC_WRAPPER to the wrapper script
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

# Flamegraph build: release optimizations with debug symbols and frame pointers.
# - No LTO & No Strip: essential for stack walking
# - force-frame-pointers: essential for profilers
RUSTFLAGS=" \
-C force-frame-pointers=yes \
${RUSTFLAGS:-}" \
cargo flamegraph --profile profiling --no-default-features --features profiling \
    --bin dynamapper --package dynamapper \
    "$@"

echo "Flamegraph complete."
