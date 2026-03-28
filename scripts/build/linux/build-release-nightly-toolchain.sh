#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running Linux nightly release build..."

# Set RUSTC_WRAPPER to the wrapper script
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

# Stable release flags plus nightly-only size/build-std flags.
#-C link-arg=-Wl,--no-hash-style   # unsupported by mold
#-C link-arg=-Wl,--icf=all         # might break something? is it better to use icf=safe? safe saves 1 mb
#-C link-arg=-Wl,--gc-sections
#-C link-arg=-Wl,--no-allow-shlib-undefined
#-C link-arg=-Wl,--strip-all
#-Zlocation-detail=none            # Doesn't help much with binary size actually
RUSTFLAGS=" \
-Clink-arg=-fuse-ld=mold \
-Clink-arg=-Wl,--gc-sections \
-Clink-arg=-Wl,--icf=safe \
-Clink-arg=-Wl,--no-allow-shlib-undefined \
-Cforce-unwind-tables=no -Csymbol-mangling-version=v0 \
-Zshare-generics=y -Zlocation-detail=none \
${RUSTFLAGS:-}" \
cargo +nightly build --release --locked --no-default-features \
    --bin dynamapper --package dynamapper \
    -Z build-std=std,panic_abort \
    -Z build-std-features=optimize_for_size \
    "$@"

echo "Build complete."
