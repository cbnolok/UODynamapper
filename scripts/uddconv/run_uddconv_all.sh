#!/bin/sh
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

exec "$SCRIPT_DIR/uddconv_all.sh" \
    --ccdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Classic/" \
    --ecdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Enhanced/" \
    --output-dir "/home/claudio/test/uddp/" \
    "$@"
