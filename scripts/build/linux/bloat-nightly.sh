#!/bin/bash
# scripts/build/linux/bloat.sh
# Nightly build analysis using cargo-bloat - Safe Tier

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export RUSTC_WRAPPER="$SCRIPT_DIR/../common/rustc_wrapper.sh"

ARGS="$@"
if [ -z "$ARGS" ]; then ARGS="--crates"; fi

echo "Running Linux Nightly Bloat Analysis..."

RUSTFLAGS="\
-Clink-arg=-fuse-ld=mold \
-Clink-arg=-Wl,--gc-sections \
-Clink-arg=-Wl,--icf=safe \
-Clink-arg=-Wl,--no-allow-shlib-undefined \
-Cforce-unwind-tables=no \
-Csymbol-mangling-version=v0 \
-Zshare-generics=y \
-Zlocation-detail=none \


$RUSTFLAGS" \
cargo +nightly bloat --release --no-default-features \
    --config 'profile.release.strip=false' \
    -Z build-std=std,panic_abort \
    -Z build-std-features="optimize_for_size" \
    $ARGS

echo "Analysis finished."
