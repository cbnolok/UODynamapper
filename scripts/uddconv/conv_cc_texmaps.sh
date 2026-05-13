#!/bin/bash
set -euo pipefail

# This script converts classic UO texmaps.mul into UODynamapper's cc_texmaps.uddp atlas format.

CC_PATH=""
OUTPUT_DIR="."

while [[ $# -gt 0 ]]; do
    case "$1" in
        --ccdir)
            CC_PATH="$2"
            shift 2
            ;;
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        *)
            echo "Usage: $0 [--ccdir CC_DATA_DIR] [--output-dir OUTPUT_DIR]"
            exit 1
            ;;
    esac
done

if [ -z "$CC_PATH" ]; then
    echo "Usage: $0 [--ccdir CC_DATA_DIR] [--output-dir OUTPUT_DIR]"
    exit 1
fi

CC_PATH=$(realpath "$CC_PATH")
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR=$(realpath "$OUTPUT_DIR")

echo "--- Starting UODynamapper CC Texmaps Conversion ---"
echo "CC Source: $CC_PATH"
echo "Target: $OUTPUT_DIR/cc_texmaps.uddp"

# Run the pack-texmaps command. We use --bc7 by default for optimal VRAM usage.
cargo run --release --bin uddpack -- pack-texmaps --ccdir "$CC_PATH" --output "$OUTPUT_DIR/cc_texmaps.uddp" --bc7

echo "Conversion complete!"
