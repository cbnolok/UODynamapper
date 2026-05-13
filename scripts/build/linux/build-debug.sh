#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running Linux debug build..."

# Set RUSTC_WRAPPER to the wrapper script
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

# Ensure wild is visible as ld.wild for clang
if command -v wild >/dev/null 2>&1; then
    WILD_PATH=$(which wild)
    # Use -fuse-ld with absolute path for robustness if clang >= 13
    export RUSTFLAGS="-Clink-arg=-fuse-ld=$WILD_PATH ${RUSTFLAGS:-}"
    echo "Using wild linker: $WILD_PATH"
else
    echo "Warning: wild linker not found, falling back to default."
fi

cargo build --workspace "$@"

echo "Build complete."
