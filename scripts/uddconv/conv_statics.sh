#!/bin/bash
set -euo pipefail

CC_PATH=""
OUTPUT_DIR="."
MAP_IDS=(0 1 2 3 4 5)

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
        --maps)
            IFS=',' read -ra MAP_IDS <<< "$2"
            shift 2
            ;;
        *)
            echo "Usage: $0 --ccdir CC_DATA_DIR [--output-dir OUTPUT_DIR] [--maps 0,1,2...]"
            exit 1
            ;;
    esac
done

if [ -z "$CC_PATH" ]; then
    echo "Error: --ccdir is required"
    echo "Usage: $0 --ccdir CC_DATA_DIR [--output-dir OUTPUT_DIR] [--maps 0,1,2...]"
    exit 1
fi

CC_PATH=$(realpath "$CC_PATH")
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR=$(realpath "$OUTPUT_DIR")

echo "--- Converting Statics MUL Files to UDDP ---"
echo "Source: $CC_PATH"
echo "Target: $OUTPUT_DIR"
echo "Maps: ${MAP_IDS[*]}"
echo

for MAP_ID in "${MAP_IDS[@]}"; do
    echo ">>> Packing statics${MAP_ID}.mul..."
    cargo run --release --bin uddpack -- pack-statics --ccdir "$CC_PATH" --map-id "$MAP_ID" --output "$OUTPUT_DIR/statics${MAP_ID}.uddp"
    echo
done

echo "Conversion complete!"
