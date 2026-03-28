#!/bin/bash
# scripts/build/common/rustc_wrapper.sh
# Intercepts rustc calls to apply specific flags only to dependencies.

RUSTC=$1
shift

CRATE_NAME=""
for i in "$@"; do
    if [[ $last_arg == "--crate-name" ]]; then
        CRATE_NAME=$i
    fi
    last_arg=$i
done

MY_CRATES=("dynamapper" "uocf")
IS_MY_CRATE=false
for my_crate in "${MY_CRATES[@]}"; do
    if [[ "$CRATE_NAME" == "$my_crate" ]]; then
        IS_MY_CRATE=true
        break
    fi
done

# Helper function to run rustc with optional sccache
run_rustc() {
    local extra_args=("$@")
    if command -v sccache >/dev/null 2>&1; then
        # Try sccache, fall back to direct rustc on failure
        if ! sccache "$RUSTC" "${extra_args[@]}" 2>/dev/null; then
            exec "$RUSTC" "${extra_args[@]}"
        fi
    else
        exec "$RUSTC" "${extra_args[@]}"
    fi
}

if [ "$IS_MY_CRATE" = true ]; then
    # Compile your code with whatever is in Cargo.toml (standard abort)
    run_rustc "$@"
else
    # Prune dependencies with no-fmt-debug.
    # We removed immediate-abort because it is binary-incompatible with standard abort.
    run_rustc "$@" -Zfmt-debug=none
fi
