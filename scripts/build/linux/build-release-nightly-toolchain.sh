#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running Linux nightly release build..."

# Set RUSTC_WRAPPER to the wrapper script
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

# Parse target from arguments
TARGET=""
for i in "$@"; do
    if [[ "$prev_arg" == "--target" ]]; then
        TARGET="$i"
    fi
    prev_arg="$i"
done

# Check if building for musl (statically linked)
if [[ "$TARGET" == *"musl"* ]]; then
    # Musl build: statically linked, no mold (use default linker)
    # musl doesn't support --icf or --no-allow-shlib-undefined
    RUSTFLAGS=" \
-C target-feature=+crt-static \
-C link-self-contained=yes \
-Clink-arg=-Wl,--gc-sections \
-Cforce-unwind-tables=no -Csymbol-mangling-version=v0 \
-Zshare-generics=y -Zlocation-detail=none \
${RUSTFLAGS:-}"
else
    # GNU libc build: use mold linker with full optimizations
    RUSTFLAGS=" \
-Clink-arg=-fuse-ld=mold \
-Clink-arg=-Wl,--gc-sections \
-Clink-arg=-Wl,--icf=safe \
-Clink-arg=-Wl,--no-allow-shlib-undefined \
-Cforce-unwind-tables=no -Csymbol-mangling-version=v0 \
-Zshare-generics=y -Zlocation-detail=none \
${RUSTFLAGS:-}"
fi

cargo +nightly build --release --locked --no-default-features \
    --bin dynamapper --package dynamapper \
    -Z build-std=std,panic_abort \
    -Z build-std-features=optimize_for_size \
    "$@"

echo "Build complete."
