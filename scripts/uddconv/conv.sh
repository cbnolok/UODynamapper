#!/bin/bash
set -e

CC_PATH=""
EC_PATH=""
OUTPUT_DIR="."

while [[ $# -gt 0 ]]; do
    case "$1" in
        --ccdir)
            CC_PATH="$2"
            shift 2
            ;;
        --ecdir)
            EC_PATH="$2"
            shift 2
            ;;
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        *)
            echo "Usage: $0 [--ccdir CC_DATA_DIR] [--ecdir EC_DATA_DIR] [--output-dir OUTPUT_DIR]"
            exit 1
            ;;
    esac
done

if [ -z "$CC_PATH" ] && [ -z "$EC_PATH" ]; then
    echo "Usage: $0 [--ccdir CC_DATA_DIR] [--ecdir EC_DATA_DIR] [--output-dir OUTPUT_DIR]"
    exit 1
fi

SOURCE_ARGS=()
if [ -n "$CC_PATH" ]; then
    CC_PATH=$(realpath "$CC_PATH")
    SOURCE_ARGS+=(--ccdir "$CC_PATH")
fi
if [ -n "$EC_PATH" ]; then
    EC_PATH=$(realpath "$EC_PATH")
    SOURCE_ARGS+=(--ecdir "$EC_PATH")
fi
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR=$(realpath "$OUTPUT_DIR")

echo "--- Starting UODynamapper Asset Conversion ---"
if [ -n "$CC_PATH" ]; then
    echo "CC Source: $CC_PATH"
fi
if [ -n "$EC_PATH" ]; then
    echo "EC Source: $EC_PATH"
fi
echo "Target: $OUTPUT_DIR"

# Convert assets
cargo run --release --bin uddconv_cli -- pack-art "${SOURCE_ARGS[@]}" --output "$OUTPUT_DIR/cc_art.uddp" 
echo
cargo run --release --bin uddconv_cli -- pack-ec-art "${SOURCE_ARGS[@]}" --output "$OUTPUT_DIR/ec_art.uddp" 
echo
cargo run --release --bin uddconv_cli -- pack-ec-land "${SOURCE_ARGS[@]}" --output "$OUTPUT_DIR/ec_land.uddp" 

# Convert metadata
echo
cargo run --release --bin uddconv_cli -- pack-unified-tiledata "${SOURCE_ARGS[@]}" --output "$OUTPUT_DIR/tiledata.uddp"

echo "Conversion complete!"

