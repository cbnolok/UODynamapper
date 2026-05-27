#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec "$SCRIPT_DIR/uddconv_maps_statics.sh" \
    --ccdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Classic fp/" \
    --output-dir "/home/claudio/test/uddp/" \
    "$@"
