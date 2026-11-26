#!/usr/bin/env bash
set -euo pipefail

mkdir -p "${X_UPT_DATA_DIR:-/data}"
if [[ $# -eq 0 ]]; then
    exec /app/x-upt serve
fi
exec /app/x-upt "$@"
