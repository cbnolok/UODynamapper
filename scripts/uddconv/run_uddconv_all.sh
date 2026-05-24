#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec "$SCRIPT_DIR/uddconv_all.sh" \
    --ccdir "/mnt/dati/_proj_local/_uo_clients/Ultima Online Classic_7_0_20_0/" \
    --ecdir "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Enhanced/" \
    --outdir "/home/claudio/test/uddp/" \
    "$@"
