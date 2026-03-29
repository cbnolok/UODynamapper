#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running Linux stable release build..."

# Set RUSTC_WRAPPER to the wrapper script
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

# Stable toolchain release flags.
RUSTFLAGS=" \
-Clink-arg=-fuse-ld=mold \
-Clink-arg=-Wl,--gc-sections \
-Clink-arg=-Wl,--no-allow-shlib-undefined \
-Clink-arg=-Wl,--icf=all \
-Clink-arg=-Wl,--strip-all \
${RUSTFLAGS:-}" \
cargo build --release --locked --no-default-features \
    --bin dynamapper --package dynamapper \
    "$@"

echo "Build complete."
