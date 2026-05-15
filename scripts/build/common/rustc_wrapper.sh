#!/bin/bash
# scripts/build/common/rustc_wrapper.sh
#
# --- PURPOSE ---
# This script acts as a wrapper for 'rustc' (the Rust compiler). It is invoked by Cargo 
# because it is pointed to by the RUSTC_WRAPPER environment variable.
#
# --- WHY IS THIS NEEDED? ---
# 1. Environment Constraints: RUSTC_WRAPPER requires a single executable or script.
#    Since we often want to toggle between different tools (like sccache) or apply
#    custom logic per-crate, we use this script as a stable entry point.
# 2. Interception Logic: This script allows us to inspect the compiler arguments 
#    (like --crate-name) and modify them on the fly. For example, we can apply 
#    different optimization levels to dependencies vs. workspace crates.
# 3. Justfile Integration: The 'justfile' handles the high-level orchestration 
#    and OS detection, then points RUSTC_WRAPPER here when it wants this custom 
#    interception logic active.
#
# --- CURRENT STATE ---
# Currently, this script is a pass-through that executes the compiler directly.
# Future interception logic (e.g. for specific workspace crates) should be added below.

RUSTC=$1
shift

# Pass through version/query flags directly without interception to keep cargo probes clean
if [[ "$*" == *"-vV"* ]] || [[ "$*" == *"--version"* ]] || [[ "$#" -eq 0 ]]; then
    exec "$RUSTC" "$@"
fi

# Execute the actual compiler
exec "$RUSTC" "$@"
