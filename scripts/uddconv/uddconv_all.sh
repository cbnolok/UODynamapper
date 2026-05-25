#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec python "$ROOT_DIR/scripts/uddconv/uddconv.py" all \
    --ccdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Classic fp/" \
    --ecdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Enhanced fp/" \
    --output-dir "/home/claudio/test/uddp/" \
    --cc-art-format=raw \
    --cc-land-format=raw \
    --cc-anim-format=bc7 \
    --ec-anim-format=bc7 \
    --art-format=bc7 \
    --land-format=bc7 \
    "$@"
