#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running Linux stable release build..."

# Use sccache directly when available to avoid wrapper-induced retries.
if command -v sccache >/dev/null 2>&1; then
    export RUSTC_WRAPPER="sccache"
else
    unset RUSTC_WRAPPER
fi

# Stable toolchain release flags.
export RUSTFLAGS=" \
-Clink-arg=-fuse-ld=mold \
-Clink-arg=-Wl,--gc-sections \
-Clink-arg=-Wl,--no-allow-shlib-undefined \
-Clink-arg=-Wl,--icf=all \
-Clink-arg=-Wl,--strip-all \
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
