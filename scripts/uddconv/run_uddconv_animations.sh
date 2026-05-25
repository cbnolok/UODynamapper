#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec "$SCRIPT_DIR/uddconv_animations.sh" \
    --ccdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Classic fp/" \
    --ecdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Enhanced fp/" \
    --output-dir "/home/claudio/test/uddp/" \
    "$@"
