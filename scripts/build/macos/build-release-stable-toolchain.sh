#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running macOS stable release build..."

# Use sccache directly when available to avoid wrapper-induced retries.
if command -v sccache >/dev/null 2>&1; then
    export RUSTC_WRAPPER="sccache"
else
    unset RUSTC_WRAPPER
fi

# Stable toolchain release flags.
# macOS uses -Wl,-dead_strip instead of --gc-sections
# macOS doesn't support --icf (Identical Code Folding)
export RUSTFLAGS=" \
-C link-arg=-Wl,-dead_strip \
${RUSTFLAGS:-}"

build_args=(
    build --release --locked --workspace --no-default-features
)

if [[ -n "${CARGO_FEATURES:-}" ]]; then
    build_args+=(--features "$CARGO_FEATURES")
fi

echo "Building workspace..."
cargo "${build_args[@]}" "$@"

echo "Build complete."
