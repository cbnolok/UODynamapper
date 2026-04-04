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
prev_arg=""
for i in "$@"; do
    if [[ "$prev_arg" == "--target" ]]; then
        TARGET="$i"
    fi
    prev_arg="$i"
done


# -Zfmt-debug=none \

# GNU libc build: use mold linker with full optimizations
RUSTFLAGS=" \
-Clink-arg=-fuse-ld=mold \
-Clink-arg=-Wl,--gc-sections \
-Clink-arg=-Wl,--icf=safe \
-Clink-arg=-Wl,--no-allow-shlib-undefined \
-Clink-arg=-Wl,--strip-all \
-Cforce-unwind-tables=no \
-Csymbol-mangling-version=v0 \
-Zshare-generics=y \
-Zlocation-detail=none \
${RUSTFLAGS:-}"

build_args=(
    +nightly build --release --locked --no-default-features
    --bin dynamapper --package dynamapper
    -Z build-std=std,panic_abort
    -Z build-std-features=optimize_for_size
)

if [[ -n "${CARGO_FEATURES:-}" ]]; then
    build_args+=(--features "$CARGO_FEATURES")
fi

build_args+=("$@")
cargo "${build_args[@]}"

echo "Build complete."
