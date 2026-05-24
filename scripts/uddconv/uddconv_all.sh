#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

usage() {
    echo "usage: ./uddconv_all.sh --ccdir <path> [--ecdir <path>] [--outdir <path>] [--maps <ids>]"
}

CCDIR=""
ECDIR=""
OUTDIR="target/uddp"
MAPS="0,1,2,3,4,5"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --ccdir)
            if [ "$#" -lt 2 ]; then
                usage
                exit 2
            fi
            CCDIR="${2:-}"
            shift 2
            ;;
        --ecdir)
            if [ "$#" -lt 2 ]; then
                usage
                exit 2
            fi
            ECDIR="${2:-}"
            shift 2
            ;;
        --outdir)
            if [ "$#" -lt 2 ]; then
                usage
                exit 2
            fi
            OUTDIR="${2:-}"
            shift 2
            ;;
        --maps)
            if [ "$#" -lt 2 ]; then
                usage
                exit 2
            fi
            MAPS="${2:-}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage
            exit 2
            ;;
    esac
done

if [ -z "$CCDIR" ]; then
    usage
    exit 2
fi

exec just --justfile "$ROOT_DIR/justfile" --working-directory "$ROOT_DIR" uddconv-all "$CCDIR" "$ECDIR" "$OUTDIR" "$MAPS"
