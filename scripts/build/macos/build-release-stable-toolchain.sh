#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running macOS stable release build..."

# Set RUSTC_WRAPPER to the wrapper script
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

# Stable toolchain release flags.
# macOS uses -Wl,-dead_strip instead of --gc-sections
# macOS doesn't support --icf (Identical Code Folding)
RUSTFLAGS=" \
-C link-arg=-Wl,-dead_strip \
${RUSTFLAGS:-}" \
cargo build --release --locked --no-default-features \
    --bin dynamapper --package dynamapper \
    "$@"

echo "Build complete."
