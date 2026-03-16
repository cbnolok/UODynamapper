#!/bin/bash
# scripts/build/macos/build.sh
# Nightly build with extreme size optimizations - Safe Tier (macOS)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

echo "Running macOS Nightly Build..."

# macOS uses -Wl,-dead_strip instead of --gc-sections
RUSTFLAGS="-Zshare-generics=y -Zlocation-detail=none -Cforce-unwind-tables=no -Csymbol-mangling-version=v0 -Clink-arg=-Wl,-dead_strip $RUSTFLAGS" \
cargo +nightly build --release --locked --no-default-features \
    -Z build-std=std,panic_abort \
    -Z build-std-features="optimize_for_size" \
    "$@"

echo "Build complete."
