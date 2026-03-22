#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running Linux profile build (release optimizations + debug symbols, no LTO)..."

# Profile build: release optimizations with debug symbols for profilers.
# - No LTO: makes profilers more precise
# - Debug symbols: included for profiling
# - No strip: keep symbols
RUSTFLAGS=" \
-Clink-arg=-fuse-ld=mold \
-Clink-arg=-Wl,--gc-sections \
-Clink-arg=-Wl,--no-allow-shlib-undefined \
-Clink-arg=-Wl,--icf=safe \
${RUSTFLAGS:-}" \
cargo build --release --locked --no-default-features \
    --bin dynamapper --package dynamapper \
    --config 'profile.release.lto=false' \
    --config 'profile.release.debug=true' \
    --config 'profile.release.strip=false' \
    "$@"

echo "Profile build complete."
