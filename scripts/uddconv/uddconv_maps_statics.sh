#!/bin/sh
set -eu

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
exec python3 "$ROOT_DIR/scripts/uddconv/uddconv.py" maps-statics \
    --ccdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Classic fp/" \
    --output-dir "/home/claudio/test/uddp/" \
    "$@"
