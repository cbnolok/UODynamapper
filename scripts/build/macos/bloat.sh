#!/bin/bash
# scripts/build/macos/bloat.sh
# Nightly build analysis - Safe Tier (macOS)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

ARGS="$@"
if [ -z "$ARGS" ]; then ARGS="--crates"; fi

echo "Running macOS Nightly Bloat Analysis..."

RUSTFLAGS="-Zshare-generics=y -Zlocation-detail=none -Cforce-unwind-tables=no -Csymbol-mangling-version=v0 -Clink-arg=-Wl,-dead_strip $RUSTFLAGS" \
cargo +nightly bloat --release --no-default-features \
    --config 'profile.release.strip=false' \
    -Z build-std=std,panic_abort \
    -Z build-std-features="optimize_for_size" \
    $ARGS

echo "Analysis finished."
