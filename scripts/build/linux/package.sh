#!/bin/bash
set -euo pipefail

# scripts/build/linux/package.sh <artifact_name> [<target_triple>]

ARTIFACT_NAME="$1"
TARGET_TRIPLE="${2:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

RELEASE_DIR="target/release"
if [[ -n "$TARGET_TRIPLE" ]]; then
    RELEASE_DIR="target/$TARGET_TRIPLE/release"
fi

DEST_DIR="artifact/$ARTIFACT_NAME"

# Subfolders
mkdir -p "$DEST_DIR/utils/cli"
mkdir -p "$DEST_DIR/utils/gui"
mkdir -p "$DEST_DIR/utils/debugging"

echo "Packaging $ARTIFACT_NAME from $RELEASE_DIR..."

# Main binary
cp "$RELEASE_DIR/dynamapper" "$DEST_DIR/"

# CLI Utilities
CLI_UTILS=(
    "udd-pack"
    "udd-tool"
    "uop-tool"
    "cc-uop-mul-converter"
    "uop-dict-populator-cli"
)

for util in "${CLI_UTILS[@]}"; do
    if [[ -f "$RELEASE_DIR/$util" ]]; then
        cp "$RELEASE_DIR/$util" "$DEST_DIR/utils/cli/"
    else
        echo "Warning: CLI Utility $util not found in $RELEASE_DIR"
    fi
done

# GUI Utilities
GUI_UTILS=(
    "udd-conv-gui"
    "uddp-inspector-gui"
    "uop-inspector-gui"
    "uop-dict-populator-gui"
)

for util in "${GUI_UTILS[@]}"; do
    if [[ -f "$RELEASE_DIR/$util" ]]; then
        cp "$RELEASE_DIR/$util" "$DEST_DIR/utils/gui/"
    else
        echo "Warning: GUI Utility $util not found in $RELEASE_DIR"
    fi
done

# Debugging Utilities
DEBUG_UTILS=(
    "texture-scanner"
)

for util in "${DEBUG_UTILS[@]}"; do
    if [[ -f "$RELEASE_DIR/$util" ]]; then
        cp "$RELEASE_DIR/$util" "$DEST_DIR/utils/debugging/"
    else
        echo "Warning: Debugging Utility $util not found in $RELEASE_DIR"
    fi
done

# Assets
cp -r assets "$DEST_DIR/"

# Documentation
cp README.md "$DEST_DIR/" 2>/dev/null || true

echo "Packaging complete: $DEST_DIR"
