#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec python "$ROOT_DIR/scripts/uddconv/uddconv.py" all \
    --ccdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Classic fp/" \
    --ecdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Enhanced fp/" \
    --output-dir "/home/claudio/test/uddp/" \
    --cc-art-raw \
    --cc-land-raw \
    --cc-anim-bc7 \
    --ec-anim-bc7 \
    --art-bc7 \
    --land-bc7 \
    "$@"
