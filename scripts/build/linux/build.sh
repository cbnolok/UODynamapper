#!/bin/bash
# scripts/build/linux/build.sh
# Nightly build with extreme size optimizations - Safe Tier

# Get the absolute path to the wrapper
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

echo "Running Linux Nightly Build (Safe Optimizations)..."

# RUSTFLAGS Breakdown:
# -Zshare-generics=y: Shares functions across crates
# -Zlocation-detail=none: Removes file/line strings from panics
# -Cforce-unwind-tables=no: Removes .eh_frame sections
# -Csymbol-mangling-version=v0: Compact symbols
# -Clink-arg=-Wl,--gc-sections: Removes unused code sections at link time

RUSTFLAGS="-Zshare-generics=y -Zlocation-detail=none -Cforce-unwind-tables=no -Csymbol-mangling-version=v0 -Clink-arg=-Wl,--gc-sections $RUSTFLAGS" \
cargo +nightly build --release --locked --no-default-features \
    -Z build-std=std,panic_abort \
    -Z build-std-features="optimize_for_size" \
    "$@"

echo "Build complete."
