#!/bin/bash
# scripts/build/common/rustc_wrapper.sh
# Intercepts rustc calls to apply specific flags only to dependencies.

RUSTC=$1
shift

# Pass through version/query flags directly without interception
if [[ "$*" == *"-vV"* ]] || [[ "$*" == *"--version"* ]] || [[ "$#" -eq 0 ]]; then
    exec "$RUSTC" "$@"
fi

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
        if sccache "$RUSTC" "${extra_args[@]}"; then
            return 0
        else
            # sccache failed (or compilation error), fall back to direct rustc
            # to be safe and ensure output is correctly handled.
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
    run_rustc "$@"
fi
