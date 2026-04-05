#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running Linux profile build (release optimizations + debug symbols, no LTO)..."

# Set RUSTC_WRAPPER to the wrapper script
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

# Profile build: release optimizations with debug symbols for profilers.
# - No LTO: makes profilers more precise
# - Debug symbols: included for profiling
# - No strip: keep symbols
export RUSTFLAGS=" \
-Cforce-frame-pointers=yes \
-Clink-arg=-fuse-ld=mold \
-Clink-arg=-Wl,--gc-sections \
-Clink-arg=-Wl,--no-allow-shlib-undefined \
${RUSTFLAGS:-}"
# -Clink-arg=-Wl,--icf=safe # Identical Code Folding (ICF) is not yet stable on Github runners Linux targets (old "mold" versions?)

build_args=(
    build --release --locked --no-default-features
    --bin dynamapper --package dynamapper
)

if [[ -n "${CARGO_FEATURES:-}" ]]; then
    build_args+=(--features "$CARGO_FEATURES")
fi

build_args+=("$@")
cargo "${build_args[@]}"

echo "Profile build complete."
