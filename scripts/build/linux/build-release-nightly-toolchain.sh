#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "Running Linux nightly release build..."

# Use sccache directly when available to avoid wrapper-induced retries.
if command -v sccache >/dev/null 2>&1; then
    export RUSTC_WRAPPER="sccache"
else
    unset RUSTC_WRAPPER
fi

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

# Release builds: use mold with aggressive size/link-time flags.
export RUSTFLAGS=" \
-Clink-arg=-fuse-ld=mold \
-Clink-arg=-Wl,--gc-sections \
-Clink-arg=-Wl,--no-allow-shlib-undefined \
-Clink-arg=-Wl,--strip-all \
-Cforce-unwind-tables=no \
-Csymbol-mangling-version=v0 \
-Zshare-generics=y \
-Zlocation-detail=none \
${RUSTFLAGS:-}"
# -Clink-arg=-Wl,--icf=safe # Identical Code Folding (ICF) is not yet stable on Github runners Linux targets (old "mold" versions?)

build_args=(
    +nightly build --release --locked --workspace --no-default-features
    -Z build-std=std,panic_abort
    -Z build-std-features=optimize_for_size
)

if [[ -n "${CARGO_FEATURES:-}" ]]; then
    build_args+=(--features "$CARGO_FEATURES")
fi

echo "Building workspace..."
cargo "${build_args[@]}" "$@"

echo "Build complete."
