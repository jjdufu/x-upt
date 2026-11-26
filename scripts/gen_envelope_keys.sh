#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ "$(basename "$SCRIPT_DIR")" = "scripts" ]]; then
    ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
else
    ROOT="$SCRIPT_DIR"
fi

OUT="${ROOT}/config/keys/envelope"
mkdir -p "$OUT"

rand_bytes() {
    local path="$1"
    local size="$2"
    openssl rand -out "$path" "$size"
}

rand_line() {
    local path="$1"
    openssl rand -base64 24 | tr -d '\n' > "$path"
}

rand_bytes "${OUT}/aes.key" 16
rand_bytes "${OUT}/sm4.key" 16
rand_bytes "${OUT}/des.key" 8
rand_bytes "${OUT}/hmac.key" 32
rand_bytes "${OUT}/hmac_sm3.key" 32
rand_bytes "${OUT}/md5.key" 16
rand_line "${OUT}/issue_username"
rand_line "${OUT}/issue_password"
rand_line "${OUT}/issue_id"
rand_line "${OUT}/issue_secret"
rand_line "${OUT}/company_id"
rand_line "${OUT}/appcode"
rand_line "${OUT}/cred_username"

echo "wrote ${OUT}"
