#!/usr/bin/env bash
set -euo pipefail

KEY_DIR="/app/config/keys/envelope"
if [[ ! -d "$KEY_DIR" ]] || [[ -z "$(ls -A "$KEY_DIR" 2>/dev/null)" ]]; then
    bash /app/scripts/gen_envelope_keys.sh
    chmod 700 "$KEY_DIR"
    find "$KEY_DIR" -type f -exec chmod 600 {} +
fi

exec /app/upupt "$@"
