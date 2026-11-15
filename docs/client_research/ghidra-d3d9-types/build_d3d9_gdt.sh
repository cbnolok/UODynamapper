#!/bin/sh
# Build a Direct3D 9 / D3DX9 Ghidra data type archive from mingw-w64 headers.
# Usage: build_d3d9_gdt.sh GHIDRA_INSTALL_DIR MINGW_INCLUDE_DIR OUTPUT_GDT
set -eu

if [ "$#" -ne 3 ]; then
    echo "Usage: $0 GHIDRA_INSTALL_DIR MINGW_INCLUDE_DIR OUTPUT_GDT" >&2
    exit 2
fi
ghidra_dir=$1
mingw_include=$2
output=$3
tool_dir=$(cd "$(dirname "$0")" && pwd)
work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT

# Ghidra's preprocessor rejects the multi-line string literals in the *_TEMPLATES macros.
# They hold .x file templates, not types, so a patched copy drops them.
mkdir "$work_dir/patched" "$work_dir/project"
awk '/^#define [A-Z_]+_TEMPLATES \\$/ { skip = 1 }
     skip { if ($0 !~ /\\$/) skip = 0; next }
     { print }' "$mingw_include/d3dx9mesh.h" > "$work_dir/patched/d3dx9mesh.h"

# Headless scripts need a program; the prelude imported as raw bytes is never used.
"$ghidra_dir/support/analyzeHeadless" "$work_dir/project" d3d9 \
    -import "$tool_dir/headers/d3d9_prelude.h" -loader BinaryLoader \
    -processor x86:LE:32:default -noanalysis -deleteProject \
    -scriptPath "$tool_dir" \
    -preScript CreateD3D9Archive.java "$output" \
    "$work_dir/patched" "$tool_dir/headers" "$mingw_include"
